<script setup lang="ts">
// "Make a Block": a name, some inputs, a shape and a color. The pieces list it
// builds is exactly what the backend stores as the block's prototype - labels
// spell out the name, inputs are the slots a call site fills in.
import { computed, ref } from 'vue';
import { Plus, Trash2 } from 'lucide-vue-next';
import { AppDropdown } from 'blockstitch';
import { createBlock, editBlock } from '../tauri';
import { newId, type BlockDefDto, type BlockPieceDto, type BlockShapeDto } from '../types';

const props = defineProps<{ editTarget: BlockDefDto | null }>();
const emit = defineEmits<{ close: [] }>();

const SHAPE_OPTIONS = [
  { value: 'Normal', label: 'a command' },
  { value: 'Ending', label: 'a command that ends the stack' },
  { value: 'ReturnsValue', label: 'a reporter (gives a value)' },
  { value: 'ReturnsBool', label: 'a reporter (gives yes or no)' },
];

const COLORS = ['#4C97FF', '#FFAB19', '#40BF4A', '#FF6680', '#9966FF', '#FF8C1A'];

/** A prototype is a run of labels and inputs; the dialog keeps the name as one
 * label at the front and every input after it, which covers everything the
 * editor needs to express without a drag-and-drop prototype builder. */
const name = ref(
  props.editTarget?.pieces
    .filter((piece): piece is Extract<BlockPieceDto, { kind: 'Label' }> => piece.kind === 'Label')
    .map(piece => piece.text)
    .join(' ') ?? '',
);
const inputs = ref(
  (props.editTarget?.pieces.filter(piece => piece.kind === 'Input') ?? []).map(piece => ({
    id: piece.id,
    name: (piece as Extract<BlockPieceDto, { kind: 'Input' }>).name,
    bool: (piece as Extract<BlockPieceDto, { kind: 'Input' }>).value_type === 'Bool',
  })),
);
const shape = ref<BlockShapeDto>(props.editTarget?.shape ?? 'Normal');
const color = ref(props.editTarget?.color ?? COLORS[0]);
const error = ref('');

const pieces = computed<BlockPieceDto[]>(() => [
  { kind: 'Label', id: props.editTarget?.pieces.find(p => p.kind === 'Label')?.id ?? newId(), text: name.value.trim() },
  ...inputs.value.map(input => ({
    kind: 'Input' as const,
    id: input.id,
    name: input.name.trim(),
    value_type: input.bool ? ('Bool' as const) : ('Any' as const),
  })),
]);

function addInput() {
  inputs.value.push({ id: newId(), name: `input${inputs.value.length + 1}`, bool: false });
}

async function submit() {
  if (name.value.trim() === '') {
    error.value = 'Give the block a name';
    return;
  }
  try {
    if (props.editTarget) await editBlock(props.editTarget.id, pieces.value, shape.value, color.value);
    else await createBlock(pieces.value, shape.value, color.value);
    emit('close');
  } catch (e) {
    error.value = String(e);
  }
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')">
    <div class="dialog">
      <h2>{{ editTarget ? 'Edit block' : 'Make a block' }}</h2>
      <p v-if="error" class="dialog-error">{{ error }}</p>
      <div class="dialog-row">
        <input type="text" v-model="name" placeholder="jump" @keydown.enter="submit">
      </div>
      <div class="dialog-row">
        <AppDropdown :options="SHAPE_OPTIONS" :model-value="shape" @update:model-value="v => (shape = v as BlockShapeDto)" />
      </div>
      <div class="dialog-row">
        <button
          v-for="swatch in COLORS"
          :key="swatch"
          class="actor-swatch"
          :style="{ background: swatch, outline: swatch === color ? '2px solid var(--blockstitch-accent)' : 'none' }"
          :title="swatch"
          @click="color = swatch"
        />
      </div>
      <div class="panel-heading">
        <span>Inputs</span>
        <button class="btn-small" @click="addInput"><Plus /></button>
      </div>
      <div class="dialog-row" v-for="(input, i) in inputs" :key="input.id">
        <input type="text" v-model="input.name" placeholder="height">
        <button class="btn-small" :class="{ primary: input.bool }" :title="input.bool ? 'Yes/no input' : 'Number or text input'" @click="input.bool = !input.bool">
          {{ input.bool ? 'yes/no' : 'value' }}
        </button>
        <button class="btn-small" title="Remove this input" @click="inputs.splice(i, 1)"><Trash2 /></button>
      </div>
      <p class="panel-note">
        The block's body goes on the canvas, under its own hat. Drag an input's
        oval out of that hat to use it inside the body.
      </p>
      <div class="dialog-actions">
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn primary" @click="submit">{{ editTarget ? 'Save' : 'Create' }}</button>
      </div>
    </div>
  </div>
</template>
