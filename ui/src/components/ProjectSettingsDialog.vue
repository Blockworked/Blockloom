<script setup lang="ts">
// Everything that belongs to the whole project rather than one actor: which
// dimension it is, and the world's background, gravity and camera. Each row
// writes straight through to the backend, like the rest of the editor.
//
// Swapping dimensions converts the scene and restarts a running game, so it
// asks before throwing a project at the other world.
import { onMounted, onUnmounted, reactive, ref, watch } from 'vue';
import { Box, FolderOpen, ImageIcon, Square, X } from 'lucide-vue-next';
import AssetDrop from './AssetDrop.vue';
import { mode, state } from '../store';
import {
  addInputBinding,
  createInputAction,
  deleteInputAction,
  importAssets,
  pickFiles,
  readAsset,
  removeInputBinding,
  setBackground,
  setCamera,
  setFixedRate,
  setGravity,
  setLighting,
  setMode,
  setPostProcess,
  setProjectIcon,
  setSoundMixer,
} from '../tauri';
import type { CameraDto, InputActionDto, InputBindingDto, LightingDto, Mode, PostProcessDto, SoundMixerDto } from '../types';

const emit = defineEmits<{ close: [] }>();
const iconPreview = ref('');

watch(
  () => state.project?.icon,
  async path => {
    if (!path) {
      iconPreview.value = '';
      return;
    }
    try {
      iconPreview.value = await readAsset(path);
    } catch {
      iconPreview.value = '';
    }
  },
  { immediate: true },
);

async function chooseIcon() {
  try {
    const files = await pickFiles('Choose a game icon');
    if (!files?.length) return;
    const imported = await importAssets('assets', [files[0]]);
    if (imported[0]) await setProjectIcon(imported[0]);
  } catch (err) {
    console.error(err);
  }
}

function applyIcon(path: string) {
  void setProjectIcon(path).catch((err: unknown) => console.error(err));
}

function writeIcon(e: Event) {
  applyIcon((e.target as HTMLInputElement).value);
}

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

function writeLighting(next: Partial<LightingDto>) {
  if (!state.project) return;
  void setLighting({ ...state.project.world.lighting, ...next }).catch((err: unknown) => console.error(err));
}

/** The camera's post, with neutral defaults for older documents. */
function postOf(): PostProcessDto {
  return {
    exposure_ev: 9.7,
    tonemapping: 'TonyMcMapface',
    bloom_enabled: false,
    bloom_threshold: 1,
    bloom_intensity: 0.15,
    vignette_strength: 0,
    ...state.project?.world.post,
  };
}

function writePost(next: Partial<PostProcessDto>) {
  if (!state.project) return;
  void setPostProcess({ ...postOf(), ...next }).catch((err: unknown) => console.error(err));
}

const TONEMAP_OPTIONS: PostProcessDto['tonemapping'][] = [
  'TonyMcMapface',
  'None',
  'Reinhard',
  'ReinhardLuminance',
  'AcesFitted',
  'Filmic',
];

/** The saved mix is stored linear (1 is unity); the dialog speaks percent. */
function mixerPct(value: number): number {
  return Math.round((value ?? 1) * 100);
}

function writeMixer(next: Partial<SoundMixerDto>) {
  if (!state.project) return;
  const sound = state.project.world.sound ?? { master_volume: 1, music_volume: 1, sfx_volume: 1 };
  void setSoundMixer({ ...sound, ...next }).catch((err: unknown) => console.error(err));
}

function writeBusVolume(key: 'master_volume' | 'music_volume' | 'sfx_volume', e: Event) {
  const pct = Math.min(Math.max(num(e, 100), 0), 200);
  writeMixer({ [key]: pct / 100 });
}

function setLightColor(e: Event) {
  writeLighting({ light_color: (e.target as HTMLInputElement).value.toUpperCase() });
}

function setAmbientColor(e: Event) {
  writeLighting({ ambient_color: (e.target as HTMLInputElement).value.toUpperCase() });
}

function writeFixedRate(e: Event) {
  if (!state.project) return;
  const rate = num(e, 60);
  void setFixedRate(Math.min(Math.max(rate, 1), 1000)).catch((err: unknown) => console.error(err));
}

// ─── Input actions ──────────────────────────────────────────────────────────
// Named actions group keys, mouse buttons and gamepad inputs under one name,
// so one strand answers a jump however the player says it.

const newActionName = ref('');
const draftBinding: Record<string, string> = reactive({});

function actionList(): InputActionDto[] {
  return state.project?.world.input?.actions ?? [];
}

/** Human spelling of a binding, the same text the `bind` block parses. */
function bindingText(binding: InputBindingDto): string {
  switch (binding.binding) {
    case 'Key':
      return binding.key ?? '';
    case 'Mouse':
      return `mouse:${(binding.button ?? 'left').toLowerCase()}`;
    case 'GamepadButton':
      return `gamepad:${(binding.button ?? '').toLowerCase()}`;
    case 'GamepadAxis': {
      const axis = (binding.axis ?? '').toLowerCase();
      const suffix = binding.direction === -1 ? '-' : binding.direction === 1 ? '+' : '';
      return `gamepad:${axis}${suffix}`;
    }
    default:
      return '';
  }
}

function addAction() {
  const name = newActionName.value.trim();
  if (!name) return;
  newActionName.value = '';
  void createInputAction(name).catch((err: unknown) => console.error(err));
}

function removeAction(name: string) {
  if (!window.confirm(`Delete the "${name}" input action?\n\nBlocks naming it will read as unheld.`)) return;
  void deleteInputAction(name).catch((err: unknown) => console.error(err));
}

function addBinding(name: string) {
  const binding = (draftBinding[name] ?? '').trim();
  if (!binding) return;
  draftBinding[name] = '';
  void addInputBinding(name, binding).catch((err: unknown) => console.error(err));
}

function removeBinding(name: string, binding: InputBindingDto) {
  void removeInputBinding(name, bindingText(binding)).catch((err: unknown) => console.error(err));
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
        <label class="dialog-label">Game icon</label>
        <div class="project-icon-row">
          <div class="project-icon-preview">
            <img v-if="iconPreview" :src="iconPreview" alt="Game icon">
            <ImageIcon v-else :size="28" />
          </div>
          <AssetDrop class="project-icon-input" :accept="['image']" @asset="applyIcon">
            <input
              type="text"
              :value="state.project?.icon ?? ''"
              placeholder="Blockloom default"
              @change="writeIcon"
            >
          </AssetDrop>
          <button class="btn" title="Choose an image" @click="chooseIcon"><FolderOpen :size="14" /></button>
          <button
            class="btn"
            title="Use the Blockloom default"
            :disabled="!state.project?.icon"
            @click="applyIcon('')"
          ><X :size="14" /></button>
        </div>
        <p class="settings-note">
          Used for the packaged executable or platform launcher. Square PNG images work best.
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
        <div class="settings-row">
          <label>Tick rate</label>
          <input type="number" min="1" max="1000" step="any" :value="state.project.world.fixed_rate" @change="writeFixedRate">
        </div>
        <p class="settings-note">
          How many times a second the world's blocks and physics advance, whatever
          the display rate is. Higher is smoother but heavier. Applies on the
          next run of the game.
        </p>
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

      <section v-if="state.project && mode === 'ThreeD'" class="settings-section">
        <h3 class="settings-section-title">Lighting</h3>
        <div class="settings-row triple">
          <label>Light direction</label>
          <input
            v-for="(coordinate, i) in state.project.world.lighting.light_direction"
            :key="i"
            type="number"
            step="any"
            :value="coordinate"
            @change="e => { const light_direction = [...state.project!.world.lighting.light_direction] as [number, number, number]; light_direction[i] = num(e, coordinate); writeLighting({ light_direction }); }"
          >
        </div>
        <div class="settings-row">
          <label>Light color</label>
          <input type="color" :value="state.project.world.lighting.light_color" @change="setLightColor">
        </div>
        <div class="settings-row">
          <label>Brightness</label>
          <input type="number" min="0" max="200000" step="any" :value="state.project.world.lighting.illuminance" @change="e => writeLighting({ illuminance: Math.min(Math.max(num(e, 10000), 0), 200000) })">
        </div>
        <div class="settings-row">
          <label>Ambient color</label>
          <input type="color" :value="state.project.world.lighting.ambient_color" @change="setAmbientColor">
        </div>
        <div class="settings-row">
          <label>Ambient</label>
          <input type="number" min="0" max="1000" step="any" :value="state.project.world.lighting.ambient_brightness" @change="e => writeLighting({ ambient_brightness: Math.min(Math.max(num(e, 80), 0), 1000) })">
        </div>
        <div class="settings-row">
          <label>Ambient occlusion</label>
          <input type="checkbox" :checked="state.project.world.lighting.ao_enabled" @change="e => writeLighting({ ao_enabled: (e.target as HTMLInputElement).checked })">
        </div>
        <div class="settings-row">
          <label>Shadow detail</label>
          <input type="number" min="512" max="8192" step="1" :value="state.project.world.lighting.shadow_map_size ?? 2048" @change="e => writeLighting({ shadow_map_size: Math.min(Math.max(Math.round(num(e, 2048)), 512), 8192) })">
        </div>
        <div class="settings-row">
          <label>Shadow bias</label>
          <input type="number" min="0" max="0.5" step="any" :value="state.project.world.lighting.shadow_bias ?? 0.02" @change="e => writeLighting({ shadow_bias: Math.min(Math.max(num(e, 0.02), 0), 0.5) })">
        </div>
        <p class="settings-note">
          Where the 3D sun shines from (aimed at the origin), and how the scene's
          ambient light looks. Occlusion darkens creases where objects meet but
          costs GPU time. Shadow detail snaps to a power of two; raise the bias
          if striped acne appears on lit faces. Applies on the next run of the game.
        </p>
      </section>

      <section v-if="state.project" class="settings-section">
        <h3 class="settings-section-title">Post-process</h3>
        <div class="settings-row">
          <label>Exposure</label>
          <input type="number" min="0" max="20" step="any" :value="postOf().exposure_ev" @change="e => writePost({ exposure_ev: Math.min(Math.max(num(e, 9.7), 0), 20) })">
        </div>
        <div class="settings-row">
          <label>Tonemap</label>
          <select :value="postOf().tonemapping" @change="e => writePost({ tonemapping: (e.target as HTMLSelectElement).value as PostProcessDto['tonemapping'] })">
            <option v-for="name in TONEMAP_OPTIONS" :key="name" :value="name">{{ name }}</option>
          </select>
        </div>
        <div class="settings-row">
          <label>Glow</label>
          <input type="checkbox" :checked="postOf().bloom_enabled" @change="e => writePost({ bloom_enabled: (e.target as HTMLInputElement).checked })">
        </div>
        <div class="settings-row" v-if="postOf().bloom_enabled">
          <label>Glow limit</label>
          <input type="number" min="0" step="any" :value="postOf().bloom_threshold" @change="e => writePost({ bloom_threshold: Math.max(num(e, 1), 0) })">
        </div>
        <div class="settings-row" v-if="postOf().bloom_enabled">
          <label>Glow amount</label>
          <input
            type="range" min="0" max="100" step="1"
            :value="Math.round(postOf().bloom_intensity * 100)"
            @change="e => writePost({ bloom_intensity: Math.min(Math.max(num(e, 15), 0), 100) / 100 })"
          >
          <span class="settings-value">{{ Math.round(postOf().bloom_intensity * 100) }}%</span>
        </div>
        <div class="settings-row">
          <label>Corners</label>
          <input
            type="range" min="0" max="100" step="1"
            :value="Math.round(postOf().vignette_strength * 100)"
            @change="e => writePost({ vignette_strength: Math.min(Math.max(num(e, 0), 0), 100) / 100 })"
          >
          <span class="settings-value">{{ Math.round(postOf().vignette_strength * 100) }}%</span>
        </div>
        <p class="settings-note">
          The camera's finish, in both dimensions. Lower exposure brightens;
          glow makes emissive surfaces bloom; corners darkens the frame edges.
          Applies on the next run of the game.
        </p>
      </section>

      <section v-if="state.project" class="settings-section">
        <h3 class="settings-section-title">Sound</h3>
        <div class="settings-row">
          <label>Master</label>
          <input
            type="range" min="0" max="200" step="1"
            :value="mixerPct(state.project.world.sound?.master_volume ?? 1)"
            @change="e => writeBusVolume('master_volume', e)"
          >
          <span class="settings-value">{{ mixerPct(state.project.world.sound?.master_volume ?? 1) }}%</span>
        </div>
        <div class="settings-row">
          <label>Music</label>
          <input
            type="range" min="0" max="200" step="1"
            :value="mixerPct(state.project.world.sound?.music_volume ?? 1)"
            @change="e => writeBusVolume('music_volume', e)"
          >
          <span class="settings-value">{{ mixerPct(state.project.world.sound?.music_volume ?? 1) }}%</span>
        </div>
        <div class="settings-row">
          <label>Effects</label>
          <input
            type="range" min="0" max="200" step="1"
            :value="mixerPct(state.project.world.sound?.sfx_volume ?? 1)"
            @change="e => writeBusVolume('sfx_volume', e)"
          >
          <span class="settings-value">{{ mixerPct(state.project.world.sound?.sfx_volume ?? 1) }}%</span>
        </div>
        <p class="settings-note">
          The saved mix every voice plays through: master scales everything,
          music and effects scale their own bus on top of it. A
          `set bus volume` block moves the live mix without changing this.
          Applies on the next run of the game.
        </p>
      </section>

      <section v-if="state.project" class="settings-section">
        <h3 class="settings-section-title">Input actions</h3>
        <div v-for="action in actionList()" :key="action.name" class="settings-action">
          <div class="settings-row">
            <label>{{ action.name }}</label>
            <button class="btn" title="Delete this action" @click="removeAction(action.name)"><X :size="14" /></button>
          </div>
          <div class="settings-bindings">
            <span v-for="binding in action.bindings" :key="bindingText(binding)" class="settings-binding">
              {{ bindingText(binding) }}
              <button class="btn" title="Remove this binding" @click="removeBinding(action.name, binding)"><X :size="12" /></button>
            </span>
            <span v-if="action.bindings.length === 0" class="settings-note">No bindings - blocks naming it read as unheld.</span>
          </div>
          <div class="settings-row">
            <input
              type="text"
              :value="draftBinding[action.name] ?? ''"
              placeholder="space, mouse:left, gamepad:south"
              @input="e => { draftBinding[action.name] = (e.target as HTMLInputElement).value; }"
              @keydown.enter="addBinding(action.name)"
            >
            <button class="btn" @click="addBinding(action.name)">Add binding</button>
          </div>
        </div>
        <div class="settings-row">
          <input
            type="text"
            v-model="newActionName"
            placeholder="New action name"
            @keydown.enter="addAction"
          >
          <button class="btn" @click="addAction">Add action</button>
        </div>
        <p class="settings-note">
          One name for every way to say it: keys, mouse buttons, gamepad buttons
          and sticks. Blocks read an action held, pressed, released or as an
          analog value, and a `bind` block remaps one for the rest of the run.
          Applies on the next run of the game.
        </p>
      </section>

      <div class="dialog-actions">
        <button class="btn primary" @click="emit('close')">Done</button>
      </div>
    </div>
  </div>
</template>
