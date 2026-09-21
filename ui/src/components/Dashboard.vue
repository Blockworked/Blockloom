<script setup lang="ts">
// Where the app starts: every project Blockloom knows about, and the ways to
// get another one. Opening a project swaps this page for the editor.
import { ref } from 'vue';
import { useTheme } from 'blockstitch';
import { Box, FolderOpen, Moon, Plus, Square, Sun, Trash2, Upload, X } from 'lucide-vue-next';
import { appVersion, state } from '../store';
import {
  deleteProject,
  forgetProject,
  importProject,
  openProject,
  openProjectFolder,
} from '../tauri';
import type { ProjectEntryDto } from '../types';
import NewProjectDialog from './NewProjectDialog.vue';

const { currentTheme, toggleTheme } = useTheme();

const creating = ref(false);
const error = ref('');

async function report(action: () => Promise<void>) {
  error.value = '';
  try {
    await action();
  } catch (e) {
    error.value = String(e);
  }
}

/** The tail of a path, since the front of a long one says the least. The
 * card's tooltip still has all of it. */
function where(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts.length <= 3 ? path : `…/${parts.slice(-3).join('/')}`;
}

/** "today", "3 days ago" - enough to sort the pile by eye. */
function opened(entry: ProjectEntryDto): string {
  if (!entry.opened_at) return 'never opened';
  const days = Math.floor((Date.now() / 1000 - entry.opened_at) / 86400);
  if (days <= 0) return 'opened today';
  if (days === 1) return 'opened yesterday';
  if (days < 30) return `opened ${days} days ago`;
  return `opened ${new Date(entry.opened_at * 1000).toLocaleDateString()}`;
}

function onForget(entry: ProjectEntryDto) {
  void report(() => forgetProject(entry.path));
}

function onDelete(entry: ProjectEntryDto) {
  const message = `Delete "${entry.name}" and everything in its folder?\n\n${entry.path}\n\nThis cannot be undone.`;
  if (window.confirm(message)) void report(() => deleteProject(entry.path));
}
</script>

<template>
  <div class="dashboard">
    <header class="dashboard-head">
      <div>
        <h1>Blockloom</h1>
        <p class="dashboard-tagline">Build a game out of blocks.</p>
      </div>
      <span class="spacer" />
      <span class="dashboard-version">{{ appVersion }}</span>
      <button
        class="icon-button"
        :title="currentTheme === 'dark' ? 'Light theme' : 'Dark theme'"
        @click="toggleTheme()"
      >
        <Sun v-if="currentTheme === 'dark'" />
        <Moon v-else />
      </button>
    </header>

    <div v-if="!state.runtime_available" class="warning-banner">
      The game runtime is missing, so Play will have nothing to open. Build the
      whole workspace (<code>just build</code>), not only the editor.
    </div>

    <div class="dashboard-actions">
      <button class="btn primary" @click="creating = true"><Plus /> Create project</button>
      <button class="btn" @click="report(() => openProjectFolder(state.default_project_location))">
        <FolderOpen /> Open a folder
      </button>
      <button class="btn" @click="report(importProject)"><Upload /> Import a file</button>
    </div>

    <p v-if="error" class="dialog-error dashboard-error">{{ error }}</p>

    <div v-if="state.library.length === 0" class="empty-state dashboard-empty">
      <p>No projects yet.</p>
      <p>Create one and it gets a folder of its own, assets and all.</p>
    </div>

    <ul v-else class="project-grid">
      <li v-for="entry in state.library" :key="entry.path">
        <button class="project-card" @click="report(() => openProject(entry.path))">
          <span class="project-card-top">
            <span class="project-card-name">{{ entry.name }}</span>
            <span class="project-card-mode" :title="entry.mode === 'ThreeD' ? '3D project' : '2D project'">
              <Box v-if="entry.mode === 'ThreeD'" />
              <Square v-else />
              {{ entry.mode === 'ThreeD' ? '3D' : '2D' }}
            </span>
          </span>
          <span class="project-card-path" :title="entry.path">{{ where(entry.path) }}</span>
          <span class="project-card-when">{{ opened(entry) }}</span>
        </button>
        <div class="project-card-tools">
          <button class="icon-button" title="Remove from this list" @click="onForget(entry)"><X /></button>
          <button class="icon-button danger" title="Delete the project folder" @click="onDelete(entry)">
            <Trash2 />
          </button>
        </div>
      </li>
    </ul>

    <NewProjectDialog v-if="creating" @close="creating = false" />
  </div>
</template>
