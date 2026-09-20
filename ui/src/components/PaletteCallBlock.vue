<script setup lang="ts">
// Sidebar prefab for one command-shaped custom block. Same job as blockstitch's
// PaletteInstructionBlock, but keyed by a `BlockDef` instead of a block type -
// a custom block's row is per-project, so there's no static entry to look up.
import { computed } from 'vue';
import { Blocks } from 'lucide-vue-next';
import { PaletteNumberField, beginPaletteDrag } from 'blockstitch';
import { paletteCallArgs } from '../blockDefs';
import { blockInputNames, type BlockDefDto } from '../types';
import { openMyBlockMenu } from '../contextMenu';

const props = defineProps<{ def: BlockDefDto }>();

const inputNames = computed(() => blockInputNames(props.def));

function onPointerDown(e: PointerEvent) {
  if ((e.target as Element | null)?.closest('input, select, textarea, button')) return;
  const el = e.currentTarget as HTMLElement;
  beginPaletteDrag(e, 'CallBlock', el.cloneNode(true) as HTMLElement, props.def.id);
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault();
  openMyBlockMenu(e, props.def.id);
}
</script>

<template>
  <div
    class="instruction-row palette-prefab blockloom-custom-block"
    :class="{ 'instruction-row-cap': def.shape === 'Ending' }"
    :style="{ '--blockloom-custom-block-color': def.color }"
    @pointerdown="onPointerDown"
    @contextmenu="onContextMenu"
  >
    <div class="instruction-shape">
      <Blocks class="instruction-type-icon" />
      <div class="instruction-content">
        <template v-for="(piece, i) in def.pieces" :key="i">
          <span v-if="piece.kind === 'Label'" class="instruction-label">{{ piece.text }}</span>
          <!-- A boolean input has no editable leaf in the sidebar, same as a
               built-in operator's boolean slot. -->
          <span v-else-if="piece.value_type === 'Bool'" class="value-block value-hex-blank">
            <span class="value-op value-hex-blank-spacer">&nbsp;</span>
          </span>
          <PaletteNumberField
            v-else
            :model-value="paletteCallArgs[def.id]?.[inputNames.indexOf(piece.name)] ?? { kind: 'Number', value: 0 }"
            @update:model-value="v => { const args = paletteCallArgs[def.id]; if (args) args[inputNames.indexOf(piece.name)] = v; }"
          />
        </template>
      </div>
    </div>
  </div>
</template>
