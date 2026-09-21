<script setup lang="ts">
// Wraps an input so an asset dragged out of the tray can be dropped on it.
// The wrapper takes the drag events rather than the input itself, so a text
// box keeps its own drag behaviour (selecting, dropping text) and the asset
// still lands wherever inside it you let go.
//
// While a compatible asset is in flight every target that would take it shows
// its outline, so you can see where a file is allowed to go before you get
// there.
import { computed, ref } from 'vue';
import { accepts, dragged, droppedAsset, endAssetDrag } from '../assets';
import type { AssetEntry, AssetKind } from '../types';

const props = defineProps<{ accept?: AssetKind[] }>();
const emit = defineEmits<{ asset: [path: string, entry: AssetEntry] }>();

const over = ref(false);

/** An asset this target would take is being dragged somewhere on the page. */
const wanted = computed(() => accepts(props.accept, dragged.value));

function onOver(e: DragEvent) {
  if (!wanted.value) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  over.value = true;
}

function onDrop(e: DragEvent) {
  over.value = false;
  const entry = droppedAsset(e);
  if (!accepts(props.accept, entry) || !entry) return;
  e.preventDefault();
  e.stopPropagation();
  endAssetDrag();
  emit('asset', entry.path, entry);
}
</script>

<template>
  <div
    class="asset-drop"
    :class="{ ready: wanted, over: over && wanted }"
    @dragover="onOver"
    @dragenter="onOver"
    @dragleave="over = false"
    @drop="onDrop"
  >
    <slot />
  </div>
</template>
