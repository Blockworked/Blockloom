<script setup lang="ts">
// Everything that belongs to the whole project rather than one actor: which
// dimension it is, and the world's background, gravity and camera. Each row
// writes straight through to the backend, like the rest of the editor.
//
// Swapping dimensions converts the scene and restarts a running game, so it
// asks before throwing a project at the other world.
import { onMounted, onUnmounted } from 'vue';
import { Box, Square } from 'lucide-vue-next';
import { mode, state } from '../store';
import { setBackground, setCamera, setGravity, setMode } from '../tauri';
import type { CameraDto, Mode } from '../types';

const emit = defineEmits<{ close: [] }>();

function applyMode(target: Mode) {
  if (target === mode.value) return;
  const label = target === 'ThreeD' ? '3D' : '2D';
  if (!window.confirm(`Switch this project to ${label}?\n\nContent is converted and a running game restarts.`)) return;
  void setMode(target).catch((e: unknown) => console.error(e));
}

function num(e: Event, fallback: number): number {
  const parsed = Number((e.target as HTMLInputElement).value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function setColor(e: Event) {
  if (!state.project) return;
  void setBackground((e.target as HTMLInputElement).value.toUpperCase()).catch((err: unknown) => console.error(err));
}

function writeGravity(index: number, value: number) {
  if (!state.project) return;
  const gravity: [number, number, number] = [...state.project.world.gravity];
  gravity[index] = value;
  void setGravity(gravity).catch((err: unknown) => console.error(err));
}

function writeCamera(next: Partial<CameraDto>) {
  if (!state.project) return;
  void setCamera({ ...state.project.world.camera, ...next }).catch((err: unknown) => console.error(err));
}

// Esc closes the dialog like any other modal.
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close');
}
onMounted(() => document.addEventListener('keydown', onKeydown));
onUnmounted(() => document.removeEventListener('keydown', onKeydown));
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')">
    <div class="dialog settings-dialog">
      <h2>Project settings</h2>

      <section class="settings-section">
        <h3 class="settings-section-title">Project</h3>
        <label class="dialog-label">Type</label>
        <div class="mode-switch settings-type" title="A project is either 2D or 3D; the blocks are the same either way">
          <button :class="{ active: mode === 'TwoD' }" @click="applyMode('TwoD')">
            <Square :size="13" />
            2D
          </button>
          <button :class="{ active: mode === 'ThreeD' }" @click="applyMode('ThreeD')">
            <Box :size="13" />
            3D
          </button>
        </div>
        <p class="settings-note">
          {{ mode === 'TwoD'
            ? 'Sprites and flat physics, measured in pixels.'
            : 'Meshes and 3D physics, measured in metres.' }}
          Switching converts the scene and restarts a running game.
        </p>
      </section>

      <section v-if="state.project" class="settings-section">
        <h3 class="settings-section-title">World</h3>
        <div class="settings-row">
          <label>Background</label>
          <input type="color" :value="state.project.world.background" @change="setColor">
        </div>
        <div class="settings-row triple">
          <label>Gravity</label>
          <input type="number" step="any" :value="state.project.world.gravity[0]" @change="e => writeGravity(0, num(e, 0))">
          <input type="number" step="any" :value="state.project.world.gravity[1]" @change="e => writeGravity(1, num(e, 0))">
          <input v-if="mode === 'ThreeD'" type="number" step="any" :value="state.project.world.gravity[2]" @change="e => writeGravity(2, num(e, 0))">
        </div>
        <div class="settings-row" v-if="mode === 'TwoD'">
          <label>Zoom</label>
          <input type="number" step="any" :value="state.project.world.camera.zoom" @change="e => writeCamera({ zoom: num(e, 1) })">
        </div>
        <div class="settings-row triple" v-else>
          <label>Camera at</label>
          <input
            v-for="(coordinate, i) in state.project.world.camera.position"
            :key="i"
            type="number"
            step="any"
            :value="coordinate"
            @change="e => { const position = [...state.project!.world.camera.position] as [number, number, number]; position[i] = num(e, coordinate); writeCamera({ position }); }"
          >
        </div>
        <p class="settings-note">
          Where the camera stands when no actor has a Camera component. A 2D unit
          is a pixel and a 3D unit is a metre.
        </p>
      </section>

      <div class="dialog-actions">
        <button class="btn primary" @click="emit('close')">Done</button>
      </div>
    </div>
  </div>
</template>