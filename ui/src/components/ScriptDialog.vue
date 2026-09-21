<script setup lang="ts">
// Editing an actor's Rust script without leaving Blockloom. The file on disk
// is the document here - it isn't part of the project JSON - so this reads it
// on open and writes it on save, and "Check" hands it to rustc and puts the
// result in the run log.
import { onMounted, ref } from 'vue';
import { checkScript, readScript, writeScript } from '../tauri';

const props = defineProps<{ actorId: string; actorName: string; path: string }>();
const emit = defineEmits<{ close: [] }>();

const source = ref('');
const error = ref('');
const busy = ref(true);
const area = ref<HTMLTextAreaElement | null>(null);

onMounted(async () => {
  try {
    source.value = await readScript(props.actorId);
  } catch (e) {
    error.value = String(e);
  }
  busy.value = false;
  area.value?.focus();
});

async function save(): Promise<boolean> {
  error.value = '';
  try {
    await writeScript(props.actorId, source.value);
    return true;
  } catch (e) {
    error.value = String(e);
    return false;
  }
}

/** Saving first, so rustc is told about what's on screen rather than what was
 * last written. */
async function check() {
  busy.value = true;
  if (await save()) {
    try {
      await checkScript(props.actorId);
    } catch (e) {
      error.value = String(e);
    }
  }
  busy.value = false;
}

async function saveAndClose() {
  if (await save()) emit('close');
}

/** A code editor that swallows Tab is worse than one that doesn't indent, so
 * Tab inserts two spaces and stays in the box. */
function onTab(e: KeyboardEvent) {
  e.preventDefault();
  const box = e.target as HTMLTextAreaElement;
  const { selectionStart: from, selectionEnd: to } = box;
  source.value = `${source.value.slice(0, from)}  ${source.value.slice(to)}`;
  requestAnimationFrame(() => box.setSelectionRange(from + 2, from + 2));
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')">
    <div class="dialog script-dialog">
      <h2>{{ actorName }} &middot; {{ path }}</h2>
      <p v-if="error" class="dialog-error">{{ error }}</p>
      <textarea
        ref="area"
        class="script-source"
        spellcheck="false"
        v-model="source"
        @keydown.tab="onTab"
      />
      <p class="panel-note">
        Real Rust, compiled with rustc when you press Play. `std` is there;
        other crates aren't. Errors land in the run log.
      </p>
      <div class="dialog-actions">
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn" :disabled="busy" @click="check">Check</button>
        <button class="btn primary" :disabled="busy" @click="saveAndClose">Save</button>
      </div>
    </div>
  </div>
</template>
