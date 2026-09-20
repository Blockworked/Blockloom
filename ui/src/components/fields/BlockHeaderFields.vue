<script setup lang="ts">
// The hat row of a custom block's own body strand: its prototype, with one
// draggable oval per declared input. Dragging an oval reuses the same palette
// machinery a `Var:` block uses, with kind `Param:<name>`; the drag kind packs
// the block's id in too (`Param:<blockId>:<name>`) so a parameter parked on open
// canvas can still be traced back to the block that declared it.
import { computed } from 'vue';
import { Blocks } from 'lucide-vue-next';
import { PaletteValueBlock } from 'blockstitch';
import { openActor } from '../../store';
import { findBlockDef } from '../../types';
import type { InstrPath, InstructionDto } from '../../types';

const props = defineProps<{ strandId: string; path: InstrPath; instruction: InstructionDto }>();

const def = computed(() => findBlockDef(openActor.value, props.instruction.block_id));

function dragKind(name: string): string {
  return `Param:${String(props.instruction.block_id)}:${name}`;
}
</script>

<template>
  <Blocks class="custom-block-header-icon" />
  <span v-if="!def" class="instruction-label">(deleted block)</span>
  <template v-else v-for="(piece, i) in def.pieces" :key="i">
    <span v-if="piece.kind === 'Label'" class="instruction-label block-header-label">{{ piece.text }}</span>
    <PaletteValueBlock
      v-else
      class="blockloom-custom-value-block"
      :style="{ '--blockloom-custom-block-color': def.color }"
      :kind="`Param:${piece.name}`"
      :drag-kind="dragKind(piece.name)"
      :bool-override="piece.value_type === 'Bool'"
    />
  </template>
</template>
