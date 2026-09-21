<script setup lang="ts">
// Naming a new project, choosing where it goes and which dimension it is.
// The folder name is not a field: the app names the folder after the project.
import { computed, onMounted, ref } from 'vue';
import { FolderOpen } from 'lucide-vue-next';
import { state } from '../store';
import { createProject, pickFolder } from '../tauri';
import type { Mode } from '../types';

const emit = defineEmits<{ close: [] }>();

const name = ref('My Game');
const location = ref(state.default_project_location);
const mode = ref<Mode>('TwoD');
const error = ref('');
const busy = ref(false);
const input = ref<HTMLInputElement | null>(null);

onMounted(() => {
  input.value?.focus();
  input.value?.select();
});

/** What the project's own folder will be called - the app owns this, so
 * anything a path can't hold is shown as the `-` it becomes. */
const folder = computed(() => {
  const cleaned = name.value
    .replace(/[/\\:*?"<>|]/g, '-')
    .trim()
    .replace(/^\.+|\.+$/g, '')
    .trim();
  return cleaned === '' ? 'Project' : cleaned;
});

const separator = computed(() => (location.value.includes('\\') ? '\\' : '/'));
const fullPath = computed(() => `${location.value.replace(/[/\\]+$/, '')}${separator.value}${folder.value}`);

async function browse() {
  const picked = await pickFolder('Where to keep the project', location.value);
  if (picked) location.value = picked;
}

async function submit() {
  if (busy.value) return;
  if (name.value.trim() === '') {
    error.value = 'Give the project a name';
    return;
  }
  busy.value = true;
  try {
    await createProject(name.value.trim(), location.value.trim(), mode.value);
    emit('close');
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')">
    <div class="dialog">
      <h2>New project</h2>
      <p v-if="error" class="dialog-error">{{ error }}</p>

      <label class="dialog-label" for="new-project-name">Name</label>
      <div class="dialog-row">
        <input
          id="new-project-name"
          ref="input"
          type="text"
          v-model="name"
          placeholder="My Game"
          @keydown.enter="submit"
        >
      </div>

      <label class="dialog-label" for="new-project-location">Where to keep it</label>
      <div class="dialog-row">
        <input id="new-project-location" type="text" v-model="location" @keydown.enter="submit">
        <button class="btn" title="Browse for a folder" @click="browse"><FolderOpen /></button>
      </div>
      <p class="panel-note path-preview">The project gets a folder of its own: <code>{{ fullPath }}</code></p>

      <label class="dialog-label">Dimension</label>
      <div class="mode-switch dialog-mode">
        <button :class="{ active: mode === 'TwoD' }" @click="mode = 'TwoD'">2D</button>
        <button :class="{ active: mode === 'ThreeD' }" @click="mode = 'ThreeD'">3D</button>
      </div>
      <p class="panel-note">
        {{ mode === 'TwoD'
          ? 'Sprites and flat physics, measured in pixels.'
          : 'Meshes and 3D physics, measured in metres.' }}
        A project can switch later.
      </p>

      <div class="dialog-actions">
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn primary" :disabled="busy" @click="submit">Create</button>
      </div>
    </div>
  </div>
</template>
