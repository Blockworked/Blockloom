<script setup lang="ts">
// The actors in the world, as a parent/child tree. Selecting one swaps the
// canvas to its own blocks, the way switching sprites does in Scratch.
// Dragging a row reorders it: hovering the top or bottom edge of another row
// drops next to it (before or after, at that row's level), hovering its
// middle drops inside it (as its last child), and dropping on the list
// background moves to the end of the top level. A drop that would make a
// loop is refused, the same way the backend refuses it.
import { computed, ref } from 'vue';
import { AppDropdown } from 'blockstitch';
import { ChevronDown, ChevronLeft, ChevronRight, Copy, Trash2 } from 'lucide-vue-next';
import {
  childrenOf,
  collapsedParents,
  draggedActor,
  droppedActor,
  endActorDrag,
  expandParent,
  isCollapsed,
  rootActors,
  startActorDrag,
  toggleCollapsed,
  wouldCycle,
} from '../actors';
import { COLLAPSED_PANEL_WIDTH, beginPanelResize, panels, setPanelOpen } from '../panels';
import { mode, state } from '../store';
import { addActor, duplicateActor, moveActor, removeActor, selectActor } from '../tauri';
import {
  SHAPES_2D,
  SHAPES_3D,
  actorParent,
  actorPhysics,
  actorVisual,
  shapesFor,
  type ActorDto,
} from '../types';

const SHAPE_LABELS: Record<string, string> = {
  Rect: 'Square',
  Circle: 'Ball',
  Image: 'Image',
  Cuboid: 'Box',
  Sphere: 'Sphere',
  Capsule: 'Capsule',
  Plane: 'Ground plane',
};

const addOptions = computed(() =>
  shapesFor(mode.value).map(shape => ({ value: shape, label: SHAPE_LABELS[shape] ?? shape })),
);

const actors = computed(() => state.project?.actors ?? []);

/** One visible row: the actor plus how deep in the tree it sits. A collapsed
 * parent hides its whole subtree. */
interface Row {
  actor: ActorDto;
  depth: number;
}

const rows = computed<Row[]>(() => {
  const list = actors.value;
  const out: Row[] = [];
  const walk = (actor: ActorDto, depth: number) => {
    out.push({ actor, depth });
    if (collapsedParents.has(actor.id)) return;
    for (const child of childrenOf(list, actor.id)) walk(child, depth + 1);
  };
  for (const root of rootActors(list)) walk(root, 0);
  return out;
});

function childCount(actorId: string): number {
  return childrenOf(actors.value, actorId).length;
}

/** The level a row sits in: its parent's children, or the roots. */
function levelOf(actorId: string): ActorDto[] {
  const parent = actorParent(actors.value.find(candidate => candidate.id === actorId) ?? null);
  return parent ? childrenOf(actors.value, parent) : rootActors(actors.value);
}

type DropPos = 'before' | 'after' | 'in';

/** The drop currently hovered: which row and whether the actor would land
 * next to it or inside it. */
const dropHint = ref<{ id: string; pos: DropPos } | null>(null);
/** Whether a drop is hovering the list background (the way to the top level). */
const rootHover = ref(false);

const panelStyle = computed(() => ({
  width: `${panels.left.open ? panels.left.width : COLLAPSED_PANEL_WIDTH}px`,
  flexBasis: `${panels.left.open ? panels.left.width : COLLAPSED_PANEL_WIDTH}px`,
}));

/** An actor whose shape belongs to the other dimension can't be drawn - the
 * list says so rather than leaving an invisible actor to puzzle over. An
 * actor with no Look at all is deliberate, not a mistake, so it says nothing. */
function wrongDimension(actor: ActorDto): boolean {
  const visual = actorVisual(actor);
  const list = mode.value === 'ThreeD' ? SHAPES_3D : SHAPES_2D;
  return !!visual && !list.includes(visual.shape);
}

function shapeName(actor: ActorDto): string {
  return actorVisual(actor)?.shape ?? 'shape';
}

function swatch(actor: ActorDto): Record<string, string> {
  const visual = actorVisual(actor);
  return { background: visual && 'color' in visual ? visual.color : '#8e8e93' };
}

function isRound(actor: ActorDto): boolean {
  const shape = actorVisual(actor)?.shape;
  return shape === 'Circle' || shape === 'Sphere';
}

function bodyBadge(actor: ActorDto): string | null {
  const body = actorPhysics(actor).body;
  return body === 'None' ? null : body[0];
}

function onRemove(actorId: string) {
  const actor = actors.value.find(candidate => candidate.id === actorId);
  if (actor && window.confirm(`Delete actor "${actor.name}"? This cannot be undone.`)) void removeActor(actorId);
}

// ─── Moving ─────────────────────────────────────────────────────────────────
// Every drop goes through one backend call: the new parent plus the
// level-mate to land in front of. The offset already authored stays, so the
// actor doesn't jump the moment a new parent places it.

interface Move {
  parent: string;
  before: string;
}

/** Where a `pos` drop on `targetId` goes: the new parent plus the level-mate
 * to land in front of. */
function moveFor(targetId: string, pos: DropPos): Move {
  if (pos === 'in') return { parent: targetId, before: '' };
  const target = actors.value.find(candidate => candidate.id === targetId);
  const parent = actorParent(target ?? null) ?? '';
  if (pos === 'before') return { parent, before: targetId };
  const level = parent ? childrenOf(actors.value, parent) : rootActors(actors.value);
  const next = level[level.findIndex(candidate => candidate.id === targetId) + 1];
  return { parent, before: next?.id ?? '' };
}

/** Whether a `pos` drop of the dragged actor on `targetId` would make a
 * loop: inside itself or a descendant, or next to a row whose level lives
 * inside it. The backend refuses the same drops; this is the list's upfront
 * answer. */
function dropInvalid(draggedId: string, targetId: string, pos: DropPos): boolean {
  if (draggedId === targetId) return true;
  const { parent } = moveFor(targetId, pos);
  return !!parent && wouldCycle(actors.value, draggedId, parent);
}

/** Whether the drop would leave the actor exactly where it is: next to the
 * row it already neighbours, or inside the parent whose last child it
 * already is. */
function dropNoop(draggedId: string, targetId: string, pos: DropPos): boolean {
  if (pos === 'in') {
    const kids = childrenOf(actors.value, targetId);
    return kids[kids.length - 1]?.id === draggedId;
  }
  const level = levelOf(targetId);
  const at = level.findIndex(candidate => candidate.id === targetId);
  const sameLevel = level.some(candidate => candidate.id === draggedId);
  if (!sameLevel) return false;
  if (pos === 'before') return at > 0 && level[at - 1]?.id === draggedId;
  return level[at + 1]?.id === draggedId;
}

function applyMove(draggedId: string, move: Move) {
  endActorDrag();
  dropHint.value = null;
  rootHover.value = false;
  if (move.parent) expandParent(move.parent);
  void moveActor(draggedId, move.parent, move.before).catch((e: unknown) => console.error(e));
}

// ─── Drag and drop ──────────────────────────────────────────────────────────
// The pointer's height on the row decides: edges mean next to, the middle
// means inside.

function posOnRow(e: DragEvent, el: HTMLElement): DropPos {
  const rect = el.getBoundingClientRect();
  const ratio = rect.height > 0 ? (e.clientY - rect.top) / rect.height : 0.5;
  if (ratio < 0.25) return 'before';
  if (ratio > 0.75) return 'after';
  return 'in';
}

function onDragStart(e: DragEvent, actorId: string) {
  dropHint.value = null;
  rootHover.value = false;
  startActorDrag(e, actorId);
}

function onDragEnd() {
  dropHint.value = null;
  rootHover.value = false;
  endActorDrag();
}

function onDragOverRow(e: DragEvent, target: ActorDto) {
  const dragged = draggedActor.value;
  if (!dragged || dragged === target.id) return;
  const el = e.currentTarget as HTMLElement | null;
  const pos = el ? posOnRow(e, el) : 'in';
  // A loop is refused without ever becoming a drop: no highlight, and the
  // event stays unclaimed so the background won't read it as a top-level
  // move either.
  if (dropInvalid(dragged, target.id, pos)) {
    if (dropHint.value?.id === target.id) dropHint.value = null;
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'none';
    return;
  }
  e.preventDefault();
  e.stopPropagation();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  dropHint.value = { id: target.id, pos };
  rootHover.value = false;
}

function onDropOnRow(e: DragEvent, target: ActorDto) {
  e.preventDefault();
  e.stopPropagation();
  const dragged = droppedActor(e);
  const el = e.currentTarget as HTMLElement | null;
  // The hover already worked out the position in this runtime; the CEF
  // fallback's re-dispatched drop carries the release point instead.
  const hint = dropHint.value?.id === target.id ? dropHint.value : null;
  const pos = hint?.pos ?? (el ? posOnRow(e, el) : 'in');
  dropHint.value = null;
  rootHover.value = false;
  if (!dragged || dropInvalid(dragged, target.id, pos)) {
    endActorDrag();
    return;
  }
  if (dropNoop(dragged, target.id, pos)) {
    endActorDrag();
    return;
  }
  applyMove(dragged, moveFor(target.id, pos));
}

/** The panel background is the way out: a drop landing on no row moves to
 * the end of the top level. Row hovers never reach here (valid ones stop
 * propagation, invalid ones sit on a row), so anything arriving here really
 * is the background. */
function onDragOverRoot(e: DragEvent) {
  if ((e.target as HTMLElement | null)?.closest?.('.actor-row')) return;
  const dragged = draggedActor.value;
  if (!dragged) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  dropHint.value = null;
  rootHover.value = true;
}

function onDropOnRoot(e: DragEvent) {
  if ((e.target as HTMLElement | null)?.closest?.('.actor-row')) return;
  e.preventDefault();
  const dragged = droppedActor(e);
  dropHint.value = null;
  rootHover.value = false;
  if (!dragged) {
    endActorDrag();
    return;
  }
  const level = rootActors(actors.value);
  if (!actorParent(actors.value.find(candidate => candidate.id === dragged) ?? null) && level[level.length - 1]?.id === dragged) {
    endActorDrag();
    return;
  }
  applyMove(dragged, { parent: '', before: '' });
}

function rowTitle(row: Row): string {
  const dragged = draggedActor.value;
  if (dragged && dragged !== row.actor.id) {
    if (dropInvalid(dragged, row.actor.id, 'in') && dropInvalid(dragged, row.actor.id, 'before'))
      return `${row.actor.name} hangs off the dragged actor, so nothing can land here`;
  }
  const kids = childCount(row.actor.id);
  return kids ? `${row.actor.name} (${kids} nested)` : row.actor.name;
}
</script>

<template>
  <aside class="side-panel" :style="panelStyle">
    <template v-if="panels.left.open">
      <div class="panel-heading">
        <span>Actors</span>
        <div class="panel-heading-actions">
          <AppDropdown
            :options="addOptions"
            model-value=""
            placeholder="Add"
            class-name="dd-compact"
            :reset-after-select="true"
            @update:model-value="shape => addActor(shape)"
          />
          <button class="panel-collapse" title="Hide the actor list" @click="setPanelOpen('left', false)">
            <ChevronLeft :size="13" />
          </button>
        </div>
      </div>
      <div
        class="actor-list"
        :class="{ 'root-hover': rootHover }"
        data-actor-drop="top-level"
        @dragover="onDragOverRoot"
        @drop="onDropOnRoot"
        @dragleave="rootHover && (rootHover = false)"
      >
        <div
          v-for="row in rows"
          :key="row.actor.id"
          class="actor-row"
          :data-actor-drop="row.actor.id"
          :draggable="true"
          :title="rowTitle(row)"
          :style="{ paddingLeft: `${12 + row.depth * 16}px` }"
          :class="{
            selected: row.actor.id === state.selected_actor,
            mismatch: wrongDimension(row.actor),
            'drop-in': dropHint?.id === row.actor.id && dropHint.pos === 'in',
            'drop-before': dropHint?.id === row.actor.id && dropHint.pos === 'before',
            'drop-after': dropHint?.id === row.actor.id && dropHint.pos === 'after',
            'drop-denied':
              !!draggedActor &&
              draggedActor !== row.actor.id &&
              dropInvalid(draggedActor, row.actor.id, 'in') &&
              dropInvalid(draggedActor, row.actor.id, 'before') &&
              dropInvalid(draggedActor, row.actor.id, 'after'),
          }"
          role="button"
          tabindex="0"
          @click="selectActor(row.actor.id)"
          @keydown.enter="selectActor(row.actor.id)"
          @dragstart="onDragStart($event, row.actor.id)"
          @dragend="onDragEnd"
          @dragover="onDragOverRow($event, row.actor)"
          @drop="onDropOnRow($event, row.actor)"
          @dragleave="dropHint?.id === row.actor.id && (dropHint = null)"
        >
          <button
            v-if="childCount(row.actor.id)"
            class="actor-toggle"
            :title="isCollapsed(row.actor.id) ? `Show actors under ${row.actor.name}` : `Hide actors under ${row.actor.name}`"
            @click.stop="toggleCollapsed(row.actor.id)"
          >
            <ChevronDown
              :size="13"
              :style="{ transform: isCollapsed(row.actor.id) ? 'rotate(-90deg)' : 'none' }"
            />
          </button>
          <span v-else class="actor-toggle-spacer" />
          <span class="actor-swatch" :class="{ round: isRound(row.actor) }" :style="swatch(row.actor)" />
          <span class="actor-name">{{ row.actor.name }}</span>
          <span v-if="wrongDimension(row.actor)" class="actor-badge" :title="`A ${shapeName(row.actor)} can't be drawn in this dimension`">!</span>
          <span v-else-if="bodyBadge(row.actor)" class="actor-badge">{{ bodyBadge(row.actor) }}</span>
        </div>
      </div>
      <div class="panel-row" v-if="state.selected_actor">
        <button class="btn-small" title="Duplicate this actor" @click="duplicateActor(state.selected_actor)">
          <Copy />
        </button>
        <button
          class="btn-small"
          title="Delete this actor"
          @click="onRemove(state.selected_actor)"
        >
          <Trash2 />
        </button>
      </div>
      <div class="panel-resize-handle" @pointerdown="e => beginPanelResize('left', e)" />
    </template>
    <button v-else class="panel-rail" title="Show the actors" @click="setPanelOpen('left', true)">
      <ChevronRight :size="16" />
    </button>
  </aside>
</template>
