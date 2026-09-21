<script setup lang="ts">
// The open project's name, the way back to the Dashboard, the 2D/3D switch,
// and the run controls.
import { computed, onMounted, onUnmounted } from 'vue';
import { useTheme } from 'blockstitch';
import { Download, LayoutGrid, Moon, MonitorX, Pause, Play, Redo2, Save, Square, Sun, Undo2, Upload } from 'lucide-vue-next';
import { mode, state } from '../store';
import {
  closeProject,
  closeRuntime,
  exportProject,
  getState,
  importProject,
  pushLog,
  pauseProject,
  redo,
  runProject,
  saveProject,
  setMode,
  setProjectName,
  stopProject,
  undo,
} from '../tauri';

const { currentTheme, toggleTheme } = useTheme();

const fps = computed(() => (state.status ? Math.round(state.status.fps) : 0));

// `state-updated` events can be delayed, so without this the fps badge sits
// still until the next command answers (e.g. clicking pause). Refresh twice
// a second while a run is going instead.
let pollTimer: number | undefined;
onMounted(() => {
  pollTimer = window.setInterval(() => {
    if (!state.running) return;
    void getState()
      .then(next => Object.assign(state, next))
      .catch(() => {});
  }, 500);
});
onUnmounted(() => {
  if (pollTimer !== undefined) window.clearInterval(pollTimer);
});

function onName(e: Event) {
  void report(() => setProjectName((e.target as HTMLInputElement).value));
}

// A command that fails has nowhere else to say so - the run log is where the
// user is already looking for what went wrong.
async function report(action: () => Promise<void>) {
  try {
    await action();
  } catch (e) {
    await pushLog('error', String(e));
  }
}
</script>

<template>
  <header class="top-bar">
    <button class="icon-button" title="All projects" @click="report(closeProject)"><LayoutGrid /></button>
    <input
      v-if="state.project"
      class="project-name"
      type="text"
      :value="state.project.name"
      :title="state.project_path ?? ''"
      placeholder="Project name"
      @change="onName"
    >
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
