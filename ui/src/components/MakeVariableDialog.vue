<script setup lang="ts">
// Naming a variable, new or renamed.
import { onMounted, ref } from 'vue';
import { createVariable, renameVariable } from '../tauri';
import { variableDialog } from '../variableDialogs';

const props = defineProps<{ renameTarget: string | null }>();
const emit = defineEmits<{ close: [] }>();

const name = ref(props.renameTarget ?? '');
const error = ref('');
const input = ref<HTMLInputElement | null>(null);

onMounted(() => input.value?.focus());

async function submit() {
  const trimmed = name.value.trim();
  if (trimmed === '') {
    error.value = 'Give the variable a name';
    return;
  }
  try {
    if (props.renameTarget) await renameVariable(props.renameTarget, trimmed);
    else await createVariable(trimmed, variableDialog.scope);
    emit('close');
  } catch (e) {
    error.value = String(e);
  }
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')">
    <div class="dialog">
      <h2>{{ renameTarget ? `Rename "${renameTarget}"` : variableDialog.scope === 'global' ? 'New shared variable' : "New variable for this actor" }}</h2>
      <p v-if="error" class="dialog-error">{{ error }}</p>
      <div class="dialog-row">
        <input ref="input" type="text" v-model="name" placeholder="score" @keydown.enter="submit">
      </div>
      <p class="panel-note" v-if="!renameTarget">
        {{ variableDialog.scope === 'global'
          ? 'Every actor can read and write a shared variable.'
          : 'Only this actor can see its own variables.' }}
      </p>
      <div class="dialog-actions">
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn primary" @click="submit">{{ renameTarget ? 'Rename' : 'Create' }}</button>
      </div>
    </div>
  </div>
</template>
