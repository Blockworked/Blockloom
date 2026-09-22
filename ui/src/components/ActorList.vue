<script setup lang="ts">
// The actors in the world, as a parent/child tree. Selecting one swaps the
// canvas to its own blocks, the way switching sprites does in Scratch.
// Dragging a row onto another reparents it under that actor; dropping it on
// the list background (or the top-level strip shown mid-drag) unparents it.
// A drop that would make a loop is refused, the same way the backend's
// `check_parent` refuses it.
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
import {
  addActor,
  addActorComponent,
  duplicateActor,
  removeActor,
  removeActorComponent,
  selectActor,
  setActorComponent,
} from '../tauri';
import {
  SHAPES_2D,
  SHAPES_3D,
  actorParent,
  actorPhysics,
  actorVisual,
  findComponent,
  shapesFor,
  type ActorComponentDto,
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

/** The row (by actor id) a valid drop is hovering over, or `''` for the
 * top-level strip. Null means no valid target is hovered. */
const dropTarget = ref<string | null>(null);

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

// ─── Reparenting ────────────────────────────────────────────────────────────
// The Parent component is written back the way the inspector writes it: the
// existing offset stays, so the actor doesn't jump the moment it starts
// being placed by its new parent.

/** Hangs `childId` off `parentId`, keeping its authored offset. A no-op when
 * it already hangs there; failures (like a loop the hover check missed) are
 * the backend's error to report. */
function reparent(childId: string, parentId: string) {
  const list = actors.value;
  const child = list.find(candidate => candidate.id === childId);
  endActorDrag();
  dropTarget.value = null;
  if (!child || parentId === childId || actorParent(child) === parentId) return;
  if (wouldCycle(list, childId, parentId)) return;
  const existing = findComponent(child, 'Parent');
  const offset = existing?.component === 'Parent' ? existing.offset : null;
  const component: ActorComponentDto = { component: 'Parent', parent: parentId, offset };
  expandParent(parentId);
  const call = existing
    ? setActorComponent(childId, 'Parent', component)
    : addActorComponent(childId, component);
  void call.catch((e: unknown) => console.error(e));
}

/** Takes `childId` off whatever it hangs off, leaving it where it is. */
function unparent(childId: string) {
  const child = actors.value.find(candidate => candidate.id === childId);
  endActorDrag();
  dropTarget.value = null;
  if (!child || !actorParent(child)) return;
  void removeActorComponent(childId, 'Parent').catch((e: unknown) => console.error(e));
}

// ─── Drag and drop ──────────────────────────────────────────────────────────

function onDragStart(e: DragEvent, actorId: string) {
  dropTarget.value = null;
  startActorDrag(e, actorId);
}

function onDragEnd() {
  dropTarget.value = null;
  endActorDrag();
}

function onDragOverRow(e: DragEvent, target: ActorDto) {
  const dragged = droppedActor(e);
  if (!dragged || dragged === target.id) return;
  // A loop is refused without ever becoming a drop: no highlight, and the
  // event stays unclaimed so the background won't read it as an unparent.
  if (wouldCycle(actors.value, dragged, target.id)) {
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'none';
    return;
  }
  e.preventDefault();
  e.stopPropagation();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  dropTarget.value = target.id;
}

function onDropOnRow(e: DragEvent, target: ActorDto) {
  e.preventDefault();
  e.stopPropagation();
  const dragged = droppedActor(e);
  dropTarget.value = null;
  if (!dragged) {
    endActorDrag();
    return;
  }
  reparent(dragged, target.id);
}

/** The panel background is the way out: a drop landing on no row unparents.
 * Row hovers never reach here (valid ones stop propagation, invalid ones sit
 * on a row), so anything arriving here really is the background. */
function onDragOverRoot(e: DragEvent) {
  if ((e.target as HTMLElement | null)?.closest?.('.actor-row')) return;
  const dragged = droppedActor(e);
  if (!dragged) return;
  const child = actors.value.find(candidate => candidate.id === dragged);
  if (!child || !actorParent(child)) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  dropTarget.value = '';
}

function onDropOnRoot(e: DragEvent) {
  if ((e.target as HTMLElement | null)?.closest?.('.actor-row')) return;
  e.preventDefault();
  const dragged = droppedActor(e);
  dropTarget.value = null;
  if (!dragged) {
    endActorDrag();
    return;
  }
  unparent(dragged);
}

function rowTitle(row: Row): string {
  const dragged = draggedActor.value;
  if (dragged && dragged !== row.actor.id && wouldCycle(actors.value, dragged, row.actor.id))
    return `${row.actor.name} already hangs off the dragged actor`;
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
        @dragover="onDragOverRoot"
        @drop="onDropOnRoot"
        @dragleave="dropTarget === '' && (dropTarget = null)"
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
            'drop-target': dropTarget === row.actor.id,
            'drop-denied':
              !!draggedActor && draggedActor !== row.actor.id && wouldCycle(actors, draggedActor, row.actor.id),
          }"
          role="button"
          tabindex="0"
          @click="selectActor(row.actor.id)"
          @keydown.enter="selectActor(row.actor.id)"
          @dragstart="onDragStart($event, row.actor.id)"
          @dragend="onDragEnd"
          @dragover="onDragOverRow($event, row.actor)"
          @drop="onDropOnRow($event, row.actor)"
          @dragleave="dropTarget === row.actor.id && (dropTarget = null)"
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
        <div
          v-if="draggedActor"
          class="actor-unparent-zone"
          data-actor-drop="top-level"
          :class="{ 'drop-target': dropTarget === '' }"
          title="Drop here to move the actor to the top level"
        >
          Drop here for top level
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
