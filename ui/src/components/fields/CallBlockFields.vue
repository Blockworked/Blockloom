<script setup lang="ts">
// A `CallBlock` row, built from the `BlockDef` it names: labels as plain text,
// one value slot per declared input. There's no fixed list of pieces for this
// one - it depends on whichever block the call points at.
import { computed } from 'vue';
import { ValueBlock } from 'blockstitch';
import { openActor } from '../../store';
import { fieldLocation, findBlockDef, numberValue } from '../../types';
import type { InstrPath, InstructionDto, ValueDto } from '../../types';

const props = defineProps<{ strandId: string; path: InstrPath; instruction: InstructionDto }>();

const def = computed(() => findBlockDef(openActor.value, props.instruction.block_id));
const args = computed(() => (Array.isArray(props.instruction.args) ? (props.instruction.args as ValueDto[]) : []));

// Pairs each prototype piece with the argument index it addresses (labels get
// none), plus the blank that index falls back to if `args` came up short - a
// boolean input blanks to an empty hexagon, not a zero.
const pieces = computed(() => {
  let argIndex = 0;
  return (def.value?.pieces ?? []).map(piece =>
    piece.kind === 'Input'
      ? {
          piece,
          argIndex: argIndex++,
          fallback: (piece.value_type === 'Bool' ? { kind: 'Bool' } : numberValue(0)) as ValueDto,
        }
      : { piece, argIndex: -1, fallback: numberValue(0) },
  );
});
</script>

<template>
  <span v-if="!def" class="instruction-label">(deleted block)</span>
  <template v-else v-for="(item, i) in pieces" :key="i">
    <span v-if="item.piece.kind === 'Label'" class="instruction-label">{{ item.piece.text }}</span>
    <ValueBlock
      v-else
      :location="fieldLocation(strandId, path, `CallArg:${item.argIndex}`)"
      :value="args[item.argIndex] ?? item.fallback"
    />
  </template>
</template>
