<script setup lang="ts">
// The embedded preview: the runtime's MJPEG sidecar as an <img>, with run
// controls, a resolution switch, headless mode, single-step and input
// forwarding. Windowed mode keeps the OS game window up beside the viewport;
// headless hides it while the hidden window keeps rendering the stream.
import { computed, onUnmounted, ref } from 'vue';
import { Pause, Play, Square, StepForward } from 'lucide-vue-next';
import { state } from '../store';
import {
  pauseProject,
  previewInput,
  pushLog,
  runProject,
  setPreviewEnabled,
  setPreviewHeadless,
  setPreviewSize,
  stepProject,
  stopProject,
} from '../tauri';

const streamUrl = computed(() =>
  state.preview_port != null ? `http://127.0.0.1:${state.preview_port}/preview.mjpg` : null,
);

const resolutions = [
  { label: '270p', width: 480, height: 270 },
  { label: '360p', width: 640, height: 360 },
  { label: '540p', width: 960, height: 540 },
];
const resolutionKey = computed(() => `${state.preview_width}x${state.preview_height}`);

async function report(action: () => Promise<void>) {
  try {
    await action();
  } catch (e) {
    await pushLog('error', String(e));
  }
}

function onResolution(e: Event) {
  const found = resolutions.find(
    candidate => `${candidate.width}x${candidate.height}` === (e.target as HTMLSelectElement).value,
  );
  if (found) void report(() => setPreviewSize(found.width, found.height));
}

// ─── Input forwarding ───────────────────────────────────────────────────────
// Coordinates go over in viewport pixels with the viewport's size, so the
// runtime can scale onto its own window. Keys go over as `KeyboardEvent.code`.
const heldButtons = ref(new Set<number>());

function frameBox(e: MouseEvent): { x: number; y: number; w: number; h: number } | null {
  const box = (e.currentTarget as Element).getBoundingClientRect();
  if (box.width <= 0 || box.height <= 0) return null;
  return {
    x: e.clientX - box.left,
    y: e.clientY - box.top,
    w: box.width,
    h: box.height,
  };
}

function onMouseMove(e: MouseEvent) {
  const box = frameBox(e);
  if (box) void previewInput({ kind: 'mouse_move', ...box }).catch(() => {});
}

function onMouseDown(e: MouseEvent) {
  if (e.button !== 0 && e.button !== 1 && e.button !== 2) return;
  const box = frameBox(e);
  if (!box) return;
  heldButtons.value.add(e.button);
  e.preventDefault();
  void previewInput({ kind: 'mouse_button', button: e.button, down: true, ...box }).catch(() => {});
}

function releaseButton(button: number, e: MouseEvent | null) {
  if (!heldButtons.value.has(button)) return;
  heldButtons.value.delete(button);
  const box = e ? frameBox(e) : null;
  void previewInput({
    kind: 'mouse_button',
    button,
    down: false,
    x: box?.x ?? 0,
    y: box?.y ?? 0,
    w: box?.w ?? state.preview_width,
    h: box?.h ?? state.preview_height,
  }).catch(() => {});
}

function onMouseUp(e: MouseEvent) {
  releaseButton(e.button, e);
}

function onMouseLeave(e: MouseEvent) {
  for (const button of [...heldButtons.value]) releaseButton(button, e);
}

function onContextMenu(e: Event) {
  e.preventDefault();
}

const GAME_KEYS = new Set([
  'Space', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Enter', 'Tab', 'Escape', 'Backspace',
]);

function onKeyDown(e: KeyboardEvent) {
  if (e.code === 'Tab') e.preventDefault();
  if (GAME_KEYS.has(e.code) || e.code.startsWith('Key') || e.code.startsWith('Digit')) {
    e.preventDefault();
  }
  void previewInput({ kind: 'key', code: e.code, down: true }).catch(() => {});
  // Printable characters also travel as text, so a focused in-game input
  // receives them the way a physical keystroke delivers them.
  if (e.key.length === 1 && !e.ctrlKey && !e.metaKey) {
    void previewInput({ kind: 'text', text: e.key }).catch(() => {});
  }
}

function onKeyUp(e: KeyboardEvent) {
  void previewInput({ kind: 'key', code: e.code, down: false }).catch(() => {});
}

function releaseAll() {
  for (const button of [...heldButtons.value]) {
    heldButtons.value.delete(button);
    void previewInput({
      kind: 'mouse_button',
      button,
      down: false,
      x: 0,
      y: 0,
      w: state.preview_width,
      h: state.preview_height,
    }).catch(() => {});
  }
}

window.addEventListener('mouseup', releaseAll);
onUnmounted(() => window.removeEventListener('mouseup', releaseAll));
</script>

<template>
  <section class="preview-panel">
    <div class="preview-toolbar">
      <label class="preview-toggle" title="Stream the running game into the editor">
        <input
          type="checkbox"
          :checked="state.preview_enabled"
          @change="report(() => setPreviewEnabled(!state.preview_enabled))"
        >
        Preview
      </label>
      <label
        class="preview-toggle"
        title="Hide the OS game window while the stream runs. The hidden window keeps rendering."
      >
        <input
          type="checkbox"
          :checked="state.preview_headless"
          :disabled="!state.preview_enabled"
          @change="report(() => setPreviewHeadless(!state.preview_headless))"
        >
        Headless
      </label>
      <select
        class="preview-resolution"
        title="Stream resolution"
        :value="resolutionKey"
        @change="onResolution"
      >
        <option
          v-for="option in resolutions"
          :key="option.label"
          :value="`${option.width}x${option.height}`"
        >
          {{ option.label }}
        </option>
      </select>
      <span class="spacer" />
      <span v-if="state.running && state.status" class="preview-fps">
        {{ Math.round(state.status.fps) }} fps
      </span>
      <button
        class="icon-button"
        :title="state.paused ? 'Resume' : 'Pause'"
        :disabled="!state.running"
        @click="report(() => pauseProject(!state.paused))"
      >
        <Play v-if="state.paused" />
        <Pause v-else />
      </button>
      <button
        class="icon-button"
        title="Advance one tick while paused"
        :disabled="!state.running || !state.paused"
        @click="report(stepProject)"
      >
        <StepForward />
      </button>
      <button
        class="run-button"
        :class="{ stop: state.running }"
        :disabled="!state.project"
        @click="report(() => (state.running ? stopProject() : runProject()))"
      >
        <Square v-if="state.running" />
        <Play v-else />
        {{ state.running ? 'Stop' : 'Play' }}
      </button>
    </div>
    <div
      v-if="state.preview_enabled"
      class="preview-viewport"
      tabindex="0"
      @keydown="onKeyDown"
      @keyup="onKeyUp"
    >
      <img
        v-if="streamUrl"
        :key="streamUrl"
        class="preview-frame"
        :src="streamUrl"
        alt="Game preview"
        draggable="false"
        @mousemove="onMouseMove"
        @mousedown="onMouseDown"
        @mouseup="onMouseUp"
        @mouseleave="onMouseLeave"
        @contextmenu="onContextMenu"
      >
      <p v-else class="preview-hint">
        {{ state.runtime_open ? 'Starting the stream…' : 'Press Play to start the stream.' }}
      </p>
    </div>
  </section>
</template>

<style scoped>
.preview-panel {
  border-bottom: 1px solid var(--border-color, #2a2f3a);
}
.preview-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px;
}
.preview-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  user-select: none;
}
.preview-resolution {
  font-size: 12px;
}
.preview-fps {
  font-size: 12px;
  opacity: 0.7;
}
.preview-viewport {
  display: flex;
  justify-content: center;
  padding: 4px 8px 8px;
  outline: none;
}
.preview-viewport:focus .preview-frame {
  outline: 1px solid var(--accent-color, #4c97ff);
}
.preview-frame {
  max-width: 100%;
  max-height: 320px;
  user-select: none;
}
.preview-hint {
  font-size: 12px;
  opacity: 0.7;
  margin: 8px 0;
}
</style>
