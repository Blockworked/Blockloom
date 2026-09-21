<script setup lang="ts">
// The open project's name, the way back to the Dashboard, the door to the
// project settings, and the run controls.
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { useTheme } from 'blockstitch';
import { Download, LayoutGrid, Moon, MonitorX, Package, Pause, Play, Redo2, Save, Settings, Square, Sun, Undo2, Upload } from 'lucide-vue-next';
import { state } from '../store';
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
  setProjectName,
  stopProject,
  undo,
} from '../tauri';
import BuildDialog from './BuildDialog.vue';
import ProjectSettingsDialog from './ProjectSettingsDialog.vue';

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

const settingsOpen = ref(false);
const buildOpen = ref(false);

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
    <button
      class="icon-button"
      title="Build a standalone game"
      :disabled="!state.project"
      @click="buildOpen = true"
    >
      <Package />
    </button>
    <button class="icon-button" title="Project settings" :disabled="!state.project" @click="settingsOpen = true">
      <Settings />
    </button>

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

    <BuildDialog v-if="buildOpen" @close="buildOpen = false" />
    <ProjectSettingsDialog v-if="settingsOpen" @close="settingsOpen = false" />
  </header>
</template>
