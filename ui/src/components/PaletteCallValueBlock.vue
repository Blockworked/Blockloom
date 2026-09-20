<script setup lang="ts">
// Sidebar prefab for one reporter-shaped custom block - the value-block
// counterpart of PaletteCallBlock.vue.
import { computed } from 'vue';
import { PaletteNumberField, beginValuePaletteDrag, paletteEvalPreview } from 'blockstitch';
import { paletteCallArgs } from '../blockDefs';
import { blockInputNames, type BlockDefDto } from '../types';
import { openMyBlockMenu } from '../contextMenu';

const props = defineProps<{ def: BlockDefDto }>();

const inputNames = computed(() => blockInputNames(props.def));
const kind = computed(() => `Call:${props.def.id}`);
const preview = computed(() => (paletteEvalPreview.value?.kind === kind.value ? paletteEvalPreview.value : null));

function onPointerDown(e: PointerEvent) {
  if ((e.target as Element | null)?.closest('input')) return;
  beginValuePaletteDrag(e, kind.value, e.currentTarget as HTMLElement);
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault();
  openMyBlockMenu(e, props.def.id);
}
</script>

<template>
  <span
    class="value-block palette-prefab blockloom-custom-value-block blockloom-custom-operator"
    :class="def.shape === 'ReturnsBool' ? 'value-card-shape-bool' : 'value-card-shape'"
    :style="{ '--blockloom-custom-block-color': def.color }"
    @pointerdown="onPointerDown"
    @contextmenu="onContextMenu"
  >
    <template v-for="(piece, i) in def.pieces" :key="i">
      <span v-if="piece.kind === 'Label'" class="value-op">{{ piece.text }}</span>
      <span v-else-if="piece.value_type === 'Bool'" class="value-block value-hex-blank">
        <span class="value-op value-hex-blank-spacer">&nbsp;</span>
      </span>
      <PaletteNumberField
        v-else
        :model-value="paletteCallArgs[def.id]?.[inputNames.indexOf(piece.name)] ?? { kind: 'Number', value: 0 }"
        @update:model-value="v => { const args = paletteCallArgs[def.id]; if (args) args[inputNames.indexOf(piece.name)] = v; }"
      />
    </template>
    <span v-if="preview" class="value-eval-tooltip" :class="{ 'value-eval-tooltip-error': preview.error }">
      {{ preview.text }}
    </span>
  </span>
</template>
