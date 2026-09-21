<script setup lang="ts">
// Building the project into a game somebody else can run: which platform, and
// where to put it. The platform list comes from the backend with a reason
// attached to each one that isn't available, since "why can't I pick macOS"
// is the whole question this dialog has to answer.
import { computed, onMounted, ref, watch } from 'vue';
import { FolderOpen } from 'lucide-vue-next';
import { state } from '../store';
import { buildGame, listBuildTargets, pickFolder } from '../tauri';
import type { BuildTarget } from '../types';

const emit = defineEmits<{ close: [] }>();

const targets = ref<BuildTarget[]>([]);
const triple = ref('');
const location = ref(state.default_project_location);
const error = ref('');
const built = ref('');
const busy = ref(false);
const fast = ref(false);

const chosen = computed(() => targets.value.find(target => target.triple === triple.value));
watch(chosen, target => {
  fast.value = Boolean(target?.fast_ready);
});

onMounted(async () => {
  try {
    targets.value = await listBuildTargets();
    // This machine comes first and can always build, so it is the default.
    triple.value = (targets.value.find(target => target.ready) ?? targets.value[0])?.triple ?? '';
  } catch (e) {
    error.value = String(e);
  }
});

async function browse() {
  const picked = await pickFolder('Where to put the built game', location.value);
  if (picked) location.value = picked;
}

async function submit() {
  if (busy.value || !chosen.value?.ready) return;
  busy.value = true;
  error.value = '';
  built.value = '';
  try {
    built.value = await buildGame(location.value.trim(), triple.value, fast.value);
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
      <h2>Build a game</h2>
      <p v-if="error" class="dialog-error">{{ error }}</p>

      <label class="dialog-label" for="build-target">Platform</label>
      <div class="dialog-row">
        <select id="build-target" v-model="triple">
          <option v-for="target in targets" :key="target.triple" :value="target.triple">
            {{ target.label }}{{ target.host ? ' (this machine)' : '' }}{{ target.ready ? '' : ' - unavailable' }}
          </option>
        </select>
      </div>
      <p v-if="chosen" class="panel-note">{{ chosen.note }}</p>

      <label class="build-fast-option">
        <input v-model="fast" type="checkbox" :disabled="!chosen?.fast_ready">
        <span>Compile blocks for maximum speed</span>
      </label>
      <p v-if="chosen" class="panel-note">{{ chosen.fast_note }}</p>

      <label class="dialog-label" for="build-location">Where to put it</label>
      <div class="dialog-row">
        <input id="build-location" type="text" v-model="location" @keydown.enter="submit">
        <button class="btn" title="Browse for a folder" @click="browse"><FolderOpen /></button>
      </div>
      <p class="panel-note">
        The game gets a folder of its own, named for the project and the platform, with the
        player and the project's assets inside it. Anyone on that platform can run it without
        Blockloom.
      </p>

      <p v-if="built" class="panel-note path-preview">Built: <code>{{ built }}</code></p>

      <div class="dialog-actions">
        <button class="btn" @click="emit('close')">{{ built ? 'Done' : 'Cancel' }}</button>
        <button class="btn primary" :disabled="busy || !chosen?.ready" @click="submit">
          {{ busy ? 'Building...' : 'Build' }}
        </button>
      </div>
    </div>
  </div>
</template>
