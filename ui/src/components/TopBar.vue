<script setup lang="ts">
// Project selection, the 2D/3D switch, and the run controls.
import { computed, ref } from 'vue';
import { AppDropdown } from 'blockstitch';
import { useTheme } from 'blockstitch';
import { Download, Moon, MonitorX, Pause, Play, Plus, Redo2, Save, Square, Sun, Trash2, Undo2, Upload } from 'lucide-vue-next';
import { mode, state } from '../store';
import {
  closeRuntime,
  exportProject,
  importProject,
  pushLog,
  newProject,
  pauseProject,
  redo,
  removeProject,
  runProject,
  saveProject,
  selectProject,
  setMode,
  setProjectName,
  stopProject,
  undo,
} from '../tauri';

const { currentTheme, toggleTheme } = useTheme();

const projectOptions = computed(() =>
  state.project_names.map((name, index) => ({ value: String(index), label: name })),
);
const selected = computed(() => (state.selected === null ? '' : String(state.selected)));
const fps = computed(() => (state.status ? Math.round(state.status.fps) : 0));

function onSelect(value: string) {
  void selectProject(Number(value));
}

function onName(e: Event) {
  void setProjectName((e.target as HTMLInputElement).value);
}

// An import or export that fails has nowhere else to say so - the run log is
// where the user is already looking for what went wrong.
async function report(action: () => Promise<void>) {
  try {
    await action();
  } catch (e) {
    await pushLog('error', String(e));
  }
}

// Deleting is two clicks rather than a dialog: the button arms itself, and
// disarms again if it isn't confirmed.
const armed = ref(false);
let disarm: number | undefined;

function onRemove() {
  if (!armed.value) {
    armed.value = true;
    disarm = window.setTimeout(() => (armed.value = false), 3000);
    return;
  }
  window.clearTimeout(disarm);
  armed.value = false;
  void removeProject();
}
</script>

<template>
  <header class="top-bar">
    <AppDropdown
      v-if="projectOptions.length"
      :options="projectOptions"
      :model-value="selected"
      placeholder="Project"
      @update:model-value="onSelect"
    />
    <input
      v-if="state.project"
      class="project-name"
      type="text"
      :value="state.project.name"
      placeholder="Project name"
      @change="onName"
    >
    <button class="icon-button" title="New project" @click="newProject('Untitled', mode)"><Plus /></button>
    <button
      class="icon-button"
      :class="{ danger: armed }"
      :title="armed ? 'Click again to delete this project' : 'Delete this project'"
      :disabled="!state.project"
      @click="onRemove"
    >
      <Trash2 />
    </button>
    <button class="icon-button" title="Save now" :disabled="!state.project" @click="saveProject()"><Save /></button>
    <button class="icon-button" title="Import a project" @click="report(importProject)"><Upload /></button>
    <button class="icon-button" title="Export this project" :disabled="!state.project" @click="report(exportProject)">
      <Download />
    </button>

    <div class="mode-switch" v-if="state.project" title="A project is either 2D or 3D; the blocks are the same either way">
      <button :class="{ active: mode === 'TwoD' }" @click="setMode('TwoD')">2D</button>
      <button :class="{ active: mode === 'ThreeD' }" @click="setMode('ThreeD')">3D</button>
    </div>

    <span class="spacer" />

    <button class="icon-button" title="Undo" :disabled="!state.can_undo" @click="undo()"><Undo2 /></button>
    <button class="icon-button" title="Redo" :disabled="!state.can_redo" @click="redo()"><Redo2 /></button>
    <button class="icon-button" :title="currentTheme === 'dark' ? 'Light theme' : 'Dark theme'" @click="toggleTheme()">
      <Sun v-if="currentTheme === 'dark'" />
      <Moon v-else />
    </button>

    <span v-if="state.running" class="actor-badge">{{ fps }} fps</span>
    <button
      v-if="state.runtime_open"
      class="icon-button"
      title="Close the game window"
      @click="closeRuntime()"
    >
      <MonitorX />
    </button>
    <button
      v-if="state.running"
      class="icon-button"
      :title="state.paused ? 'Resume' : 'Pause'"
      @click="pauseProject(!state.paused)"
    >
      <Play v-if="state.paused" />
      <Pause v-else />
    </button>
    <button
      class="run-button"
      :class="{ stop: state.running }"
      :disabled="!state.project"
      @click="state.running ? stopProject() : runProject()"
    >
      <Square v-if="state.running" />
      <Play v-else />
      {{ state.running ? 'Stop' : 'Play' }}
    </button>
  </header>
</template>
