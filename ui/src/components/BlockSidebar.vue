<script setup lang="ts">
// The palette: every block, grouped, plus the value blocks, the variables and
// "My Blocks". Dragging anything out of here drops a copy of it onto the canvas.
import { computed } from 'vue';
import { Trash2 } from 'lucide-vue-next';
import {
  PaletteInstructionBlock,
  PaletteValueBlock,
  beginSidebarResize,
  sidebarWidth,
  type ValueNode,
} from 'blockstitch';
import { mode, openActor, state } from '../store';
import { paletteInstructions, paletteValueFor, applyPaletteValueEdit } from '../paletteState';
import { OPERATOR_GROUPS, specForKind } from '../valueOps';
import { blockShapeReturnsValue, variableNames, type InstructionType } from '../types';
import { openCreateVariableDialog, closeVariableDialog, variableDialog } from '../variableDialogs';
import { blockDialog, closeBlockDialog, openCreateBlockDialog } from '../blockDialogs';
import MakeVariableDialog from './MakeVariableDialog.vue';
import MakeBlockDialog from './MakeBlockDialog.vue';
import PaletteCallBlock from './PaletteCallBlock.vue';
import PaletteCallValueBlock from './PaletteCallValueBlock.vue';

/** The palette's groups. `BlockHeader` and `CallBlock` never appear as fixed
 * prefabs - a custom block gets its own entry under "My Blocks" instead. */
const BLOCK_GROUPS: { label: string; types: InstructionType[] }[] = [
  {
    label: 'Events',
    types: ['WhenStarted', 'WhenKeyPressed', 'WhenClicked', 'WhenCollision', 'WhenMessage', 'Broadcast'],
  },
  {
    label: 'Motion',
    types: ['Move', 'GoTo', 'ChangePosition', 'Glide', 'Turn', 'SetRotation', 'PointTowards', 'SetScale'],
  },
  { label: 'Physics', types: ['SetBody', 'ApplyImpulse', 'SetVelocity', 'SetGravity'] },
  { label: 'Looks', types: ['Say', 'SetVisible', 'SetColor'] },
  {
    label: 'Control',
    types: ['Wait', 'WaitUntil', 'If', 'IfElse', 'Repeat', 'Forever', 'While', 'EscapeLoop', 'ContinueLoop', 'StopAll'],
  },
];

const commandBlocks = computed(() =>
  (openActor.value?.block_defs ?? []).filter(def => !blockShapeReturnsValue(def.shape)),
);
const reporterBlocks = computed(() =>
  (openActor.value?.block_defs ?? []).filter(def => blockShapeReturnsValue(def.shape)),
);

const actorVariables = computed(() => (openActor.value?.variables ?? []).map(v => v.name).sort());
const globalVariables = computed(() =>
  (state.project?.globals ?? [])
    .map(v => v.name)
    .filter(name => !actorVariables.value.includes(name))
    .sort(),
);
const hasVariables = computed(() => variableNames(state.project, openActor.value).length > 0);

// The palette derives a value block's hexagon-or-capsule shape from Blockloom's
// own operator table, so it can never disagree with what the canvas draws once
// the block is dropped.
function isBool(kind: string): boolean {
  return specForKind(kind)?.resultType === 'bool';
}

function onValueEdit(kind: string, next: ValueNode) {
  applyPaletteValueEdit(kind, next);
}

/** Blocks that would do nothing in this dimension are still offered, but the
 * palette says which they are rather than hiding them. */
function dimensionNote(type: InstructionType): string | undefined {
  if (mode.value === 'ThreeD') return undefined;
  return type === 'SetRotation' || type === 'Turn' ? 'In 2D this turns within the plane' : undefined;
}

// Empty palette space has no action of its own, so a browser menu there is just
// noise. Text inputs keep their own menu.
function onContextMenu(event: MouseEvent) {
  if ((event.target as Element | null)?.closest('input, textarea')) return;
  event.preventDefault();
}
</script>

<template>
  <div class="instruction-sidebar" id="instruction-sidebar" :style="{ width: sidebarWidth + 'px' }" @contextmenu="onContextMenu">
    <div class="sidebar-trash-hint">
      <Trash2 />
      <span>Drag a block here to delete it</span>
    </div>
    <div class="sidebar-scroll">
      <template v-for="group in BLOCK_GROUPS" :key="group.label">
        <div class="sidebar-section-label">{{ group.label }}</div>
        <div class="sidebar-palette">
          <PaletteInstructionBlock
            v-for="type in group.types"
            :key="type"
            :type="type"
            :instruction="paletteInstructions[type]"
            :title="dimensionNote(type)"
          />
        </div>
      </template>

      <template v-for="group in OPERATOR_GROUPS" :key="group.label">
        <div class="sidebar-section-label">{{ group.label }}</div>
        <div class="sidebar-palette sidebar-palette-values">
          <PaletteValueBlock
            v-for="kind in group.kinds"
            :key="kind"
            :kind="kind"
            :value="paletteValueFor(kind)"
            :bool-override="isBool(kind)"
            @update:value="v => onValueEdit(kind, v)"
          />
        </div>
      </template>

      <div class="sidebar-section-label">Values</div>
      <div class="sidebar-palette sidebar-palette-values">
        <PaletteValueBlock
          v-for="kind in ['Number', 'Text']"
          :key="kind"
          :kind="kind"
          :value="paletteValueFor(kind)"
          @update:value="v => onValueEdit(kind, v)"
        />
      </div>

      <div class="panel-heading">
        <span>This actor's variables</span>
        <button type="button" class="btn-small" @click="openCreateVariableDialog('actor')">New</button>
      </div>
      <div class="sidebar-palette sidebar-palette-values" v-if="actorVariables.length">
        <PaletteValueBlock v-for="name in actorVariables" :key="name" :kind="`Var:${name}`" />
      </div>
      <div class="panel-heading">
        <span>Shared variables</span>
        <button type="button" class="btn-small" @click="openCreateVariableDialog('global')">New</button>
      </div>
      <div class="sidebar-palette sidebar-palette-values" v-if="globalVariables.length">
        <PaletteValueBlock v-for="name in globalVariables" :key="name" :kind="`Var:${name}`" />
      </div>
      <div class="sidebar-palette" v-if="hasVariables">
        <PaletteInstructionBlock type="SetVariable" :instruction="paletteInstructions.SetVariable" />
        <PaletteInstructionBlock type="ChangeVariable" :instruction="paletteInstructions.ChangeVariable" />
      </div>
      <p v-else class="panel-note">A variable remembers a number or some text - a score, a level, a name.</p>

      <div class="panel-heading">
        <span>My Blocks</span>
        <button type="button" class="btn-small" @click="openCreateBlockDialog()">Make a Block</button>
      </div>
      <div class="sidebar-palette sidebar-palette-values" v-if="reporterBlocks.length">
        <PaletteCallValueBlock v-for="def in reporterBlocks" :key="def.id" :def="def" />
      </div>
      <div class="sidebar-palette">
        <PaletteCallBlock v-for="def in commandBlocks" :key="def.id" :def="def" />
        <PaletteInstructionBlock type="Return" :instruction="paletteInstructions.Return" />
      </div>
    </div>
    <div class="sidebar-resize-handle" @pointerdown="beginSidebarResize" />
  </div>
  <MakeVariableDialog
    v-if="variableDialog.mode"
    :rename-target="variableDialog.mode === 'rename' ? variableDialog.renameTarget : null"
    @close="closeVariableDialog"
  />
  <MakeBlockDialog
    v-if="blockDialog.mode"
    :edit-target="blockDialog.mode === 'edit' ? blockDialog.editTarget : null"
    @close="closeBlockDialog"
  />
</template>
