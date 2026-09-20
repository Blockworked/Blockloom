<script setup lang="ts">
// What the running world has said, plus the shared variables' live values.
import { computed } from 'vue';
import { Trash2 } from 'lucide-vue-next';
import { state } from '../store';
import { clearLog } from '../tauri';

const globals = computed(() => state.status?.globals ?? []);

function shown(value: { kind: string; value?: unknown }): string {
  return value.value === undefined ? String(value.kind) : String(value.value);
}
</script>

<template>
  <section class="run-log">
    <div class="run-log-head">
      <span v-if="state.running">Running{{ state.paused ? ' (paused)' : '' }} · {{ (state.status?.time ?? 0).toFixed(1) }}s</span>
      <span v-else>Not running</span>
      <span v-for="variable in globals" :key="variable.name">{{ variable.name }}: {{ shown(variable.value) }}</span>
      <span class="spacer" style="flex: 1 1 auto" />
      <button class="btn-small" title="Clear the log" @click="clearLog()"><Trash2 /></button>
    </div>
    <div class="run-log-lines">
      <div v-for="(line, i) in state.log" :key="i" class="run-log-line" :class="{ error: line.kind === 'error' }">
        <span class="who">{{ line.actor }}</span>
        <span class="what"> {{ line.text }}</span>
      </div>
    </div>
  </section>
</template>
