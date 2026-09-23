<script setup lang="ts">
// The palette: every block, grouped, plus the value blocks, the variables and
// "My Blocks". Dragging anything out of here drops a copy of it onto the canvas.
import { computed, watchEffect } from 'vue';
import { ChevronLeft, ChevronRight, Trash2 } from 'lucide-vue-next';
import {
  PaletteInstructionBlock,
  PaletteValueBlock,
  beginSidebarResize,
  sidebarWidth,
  type ValueNode,
} from 'blockstitch';
import { DictPanel, ListPanel, MakeDictDialog, MakeListDialog } from 'blockstitch';
import { COLLAPSED_PANEL_WIDTH, panels, setBlocksOpen } from '../panels';
import { mode, openActor, state } from '../store';
import { paletteInstructions, paletteValueFor, applyPaletteValueEdit, syncPaletteDictDefaults, syncPaletteListDefaults } from '../paletteState';
import { OPERATOR_GROUPS, setDictNameOptions, setListNameOptions, specForKind } from '../valueOps';
import { blockShapeReturnsValue, dictNames, listNames, variableNames, type InstructionType } from '../types';
import { openCreateVariableDialog, closeVariableDialog, variableDialog } from '../variableDialogs';
import { closeListDialog, listDialog, openCreateListDialog } from '../listDialogs';
import { closeDictDialog, dictDialog, openCreateDictDialog } from '../dictDialogs';
import { openDictMenu, openListMenu } from '../contextMenu';
import { createDict, createList, renameDict, renameList } from '../tauri';
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
    types: [
      'WhenStarted',
      'WhenKeyPressed',
      'WhenClicked',
      'WhenCollision',
      'WhenMessage',
      'WhenCloned',
      'WhenUiClicked',
      'WhenUiChanged',
      'Broadcast',
    ],
  },
  {
    label: 'Motion',
    types: ['Move', 'GoTo', 'NavigateTo', 'ChangePosition', 'Glide', 'Turn', 'SetRotation', 'PointTowards', 'SetScale'],
  },
  { label: 'Physics', types: ['SetBody', 'ApplyImpulse', 'SetVelocity', 'SetGravity', 'SetDensity', 'SetMass'] },
  { label: 'Looks', types: ['Say', 'SetVisible', 'SetColor'] },
  {
    label: 'Sound',
    types: ['PlaySound', 'PlaySoundAt', 'StopSound', 'SetSoundVolume', 'SetSoundPitch', 'SetBusVolume'],
  },
  {
    label: 'Components',
    types: ['SetComponentField', 'SetCameraView', 'SetCameraPitch', 'SetCameraFov', 'AttachComponent', 'DetachComponent', 'SetParent'],
  },
  { label: 'Actors', types: ['CreateClone', 'CreateActor', 'DeleteActor'] },
  {
    label: 'Interface',
    types: [
      'ShowPanel',
      'ShowLabel',
      'ShowButton',
      'ShowImage',
      'ShowInput',
      'ShowSlider',
      'ShowToggle',
      'ShowList',
      'SetUiTheme',
      'SetUiProp',
      'HideElement',
      'HideAllUi',
      'DeleteElement',
      'FocusElement',
      'ClearFocus',
      'PauseGame',
      'ResumeGame',
    ],
  },
  {
    label: 'Control',
    types: ['Wait', 'WaitUntil', 'If', 'IfElse', 'Repeat', 'Forever', 'While', 'EscapeLoop', 'ContinueLoop', 'StopAll', 'SetMouseLocked'],
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

const actorLists = computed(() => [...(openActor.value?.lists ?? [])].sort((a, b) => a.name.localeCompare(b.name)));
const sharedLists = computed(() =>
  [...(state.project?.global_lists ?? [])]
    .filter(list => !actorLists.value.some(own => own.name === list.name))
    .sort((a, b) => a.name.localeCompare(b.name)),
);
const hasLists = computed(() => listNames(state.project, openActor.value).length > 0);

// The list reporters' name dropdowns read live choices, so keep them pointed
// at this actor's lists as they are created, renamed, or deleted.
watchEffect(() => {
  const names = listNames(state.project, openActor.value);
  setListNameOptions(names);
  syncPaletteListDefaults(names);
});

async function submitListDialog(name: string, renameTarget: string | null | undefined): Promise<void> {
  if (renameTarget) await renameList(renameTarget, name);
  else await createList(name, listDialog.scope);
}

const actorDicts = computed(() => [...(openActor.value?.dicts ?? [])].sort((a, b) => a.name.localeCompare(b.name)));
const sharedDicts = computed(() =>
  [...(state.project?.global_dicts ?? [])]
    .filter(dict => !actorDicts.value.some(own => own.name === dict.name))
    .sort((a, b) => a.name.localeCompare(b.name)),
);
const hasDicts = computed(() => dictNames(state.project, openActor.value).length > 0);

// The dict reporters' name dropdowns read live choices, so keep them pointed
// at this actor's dicts as they are created, renamed, or deleted.
watchEffect(() => {
  const names = dictNames(state.project, openActor.value);
  setDictNameOptions(names);
  syncPaletteDictDefaults(names);
});

async function submitDictDialog(name: string, renameTarget: string | null | undefined): Promise<void> {
  if (renameTarget) await renameDict(renameTarget, name);
  else await createDict(name, dictDialog.scope);
}

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
  <div
    class="instruction-sidebar"
    id="instruction-sidebar"
    :style="{ width: (panels.blocks.open ? sidebarWidth : COLLAPSED_PANEL_WIDTH) + 'px' }"
    @contextmenu="onContextMenu"
  >
    <template v-if="panels.blocks.open">
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
          <PaletteInstructionBlock type="SaveVariable" :instruction="paletteInstructions.SaveVariable" />
          <PaletteInstructionBlock type="ClearSavedVariable" :instruction="paletteInstructions.ClearSavedVariable" />
        </div>
        <p v-else class="panel-note">A variable remembers a number or some text - a score, a level, a name.</p>

        <div class="panel-heading">
          <span>This actor's lists</span>
          <button type="button" class="btn-small" @click="openCreateListDialog('actor')">New</button>
        </div>
        <ListPanel :lists="actorLists" @menu="(name, event) => openListMenu(event, name)" />
        <div class="panel-heading">
          <span>Shared lists</span>
          <button type="button" class="btn-small" @click="openCreateListDialog('global')">New</button>
        </div>
        <ListPanel :lists="sharedLists" @menu="(name, event) => openListMenu(event, name)" />
        <div class="sidebar-palette" v-if="hasLists">
          <PaletteInstructionBlock type="AddToList" :instruction="paletteInstructions.AddToList" />
          <PaletteInstructionBlock type="DeleteOfList" :instruction="paletteInstructions.DeleteOfList" />
          <PaletteInstructionBlock type="DeleteAllOfList" :instruction="paletteInstructions.DeleteAllOfList" />
          <PaletteInstructionBlock type="ShiftList" :instruction="paletteInstructions.ShiftList" />
          <PaletteInstructionBlock type="InsertIntoList" :instruction="paletteInstructions.InsertIntoList" />
          <PaletteInstructionBlock type="ReplaceItemOfList" :instruction="paletteInstructions.ReplaceItemOfList" />
          <PaletteInstructionBlock type="ReverseList" :instruction="paletteInstructions.ReverseList" />
        </div>
        <p v-else class="panel-note">A list holds numbers or text in order - a queue, a hand of cards, a high-score table.</p>

        <div class="panel-heading">
          <span>This actor's dicts</span>
          <button type="button" class="btn-small" @click="openCreateDictDialog('actor')">New</button>
        </div>
        <DictPanel :dicts="actorDicts" @menu="(name, event) => openDictMenu(event, name)" />
        <div class="panel-heading">
          <span>Shared dicts</span>
          <button type="button" class="btn-small" @click="openCreateDictDialog('global')">New</button>
        </div>
        <DictPanel :dicts="sharedDicts" @menu="(name, event) => openDictMenu(event, name)" />
        <div class="sidebar-palette" v-if="hasDicts">
          <PaletteInstructionBlock type="SetDictValue" :instruction="paletteInstructions.SetDictValue" />
          <PaletteInstructionBlock type="DeleteDictKey" :instruction="paletteInstructions.DeleteDictKey" />
          <PaletteInstructionBlock type="DeleteAllOfDict" :instruction="paletteInstructions.DeleteAllOfDict" />
          <PaletteInstructionBlock type="LoadJsonIntoDict" :instruction="paletteInstructions.LoadJsonIntoDict" />
          <PaletteInstructionBlock type="LoadJsonIntoList" :instruction="paletteInstructions.LoadJsonIntoList" />
        </div>
        <p v-else class="panel-note">A dict holds numbers or text by key - a save slot, an inventory, a settings table.</p>

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
      <button class="sidebar-collapse" title="Hide the blocks" @click="setBlocksOpen(false)">
        <ChevronLeft :size="14" />
      </button>
      <div class="sidebar-resize-handle" @pointerdown="beginSidebarResize" />
    </template>
    <template v-else>
      <button class="sidebar-rail" title="Show the blocks" @click="setBlocksOpen(true)">
        <ChevronRight :size="16" />
      </button>
    </template>
  </div>
  <MakeVariableDialog
    v-if="variableDialog.mode"
    :rename-target="variableDialog.mode === 'rename' ? variableDialog.renameTarget : null"
    @close="closeVariableDialog"
  />
  <MakeListDialog
    v-if="listDialog.mode"
    :rename-target="listDialog.mode === 'rename' ? listDialog.renameTarget : null"
    :on-submit="submitListDialog"
    @close="closeListDialog"
  />
  <MakeDictDialog
    v-if="dictDialog.mode"
    :rename-target="dictDialog.mode === 'rename' ? dictDialog.renameTarget : null"
    :on-submit="submitDictDialog"
    @close="closeDictDialog"
  />
  <MakeBlockDialog
    v-if="blockDialog.mode"
    :edit-target="blockDialog.mode === 'edit' ? blockDialog.editTarget : null"
    @close="closeBlockDialog"
  />
</template>
