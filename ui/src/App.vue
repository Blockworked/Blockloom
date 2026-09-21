<script setup lang="ts">
// Two pages: the Dashboard the app starts on, and the editor a project opens
// into - a top bar, the actor list, the block palette and canvas, the
// inspector, the asset tray and the run log.
import { onMounted, onUnmounted } from 'vue';
import { Canvas } from 'blockstitch';
import { initState, state } from './store';
import { redo, resetZoom, undo } from './tauri';
import Dashboard from './components/Dashboard.vue';
import TopBar from './components/TopBar.vue';
import ActorList from './components/ActorList.vue';
import BlockSidebar from './components/BlockSidebar.vue';
import InspectorPanel from './components/InspectorPanel.vue';
import AssetTray from './components/AssetTray.vue';
import RunLog from './components/RunLog.vue';
import ContextMenu from './components/ContextMenu.vue';

onMounted(() => {
  void initState();
  document.addEventListener('keydown', onKeydown);
});
onUnmounted(() => document.removeEventListener('keydown', onKeydown));

async function onKeydown(e: KeyboardEvent) {
  const typing = (e.target as Element | null)?.closest?.('input, textarea, [contenteditable="true"]');
  const chord = e.ctrlKey || e.metaKey;
  if (!chord) return;
  // Page zoom: handled here rather than by the browser's own Ctrl+0
  // accelerator, which this CEF runtime can't be relied on to deliver.
  if (!e.repeat && !e.shiftKey && !e.altKey && (e.code === 'Digit0' || e.code === 'Numpad0')) {
    e.preventDefault();
    await resetZoom();
    return;
  }
  if (typing) return;
  if (e.code === 'KeyZ' && !e.shiftKey) {
    e.preventDefault();
    await undo();
  } else if ((e.code === 'KeyZ' && e.shiftKey) || e.code === 'KeyY') {
    e.preventDefault();
    await redo();
  }
}
</script>

<template>
  <Dashboard v-if="!state.project" />
  <template v-else>
    <TopBar />
    <div v-if="!state.runtime_available" class="warning-banner">
      The game runtime is missing, so Play has nothing to open. Build the whole
      workspace (<code>just build</code>), not only the editor.
    </div>
    <div class="editor-body">
      <ActorList />
      <div class="editor-middle">
        <div class="editor-content-area">
          <BlockSidebar />
          <Canvas>
            <template #context-menu>
              <ContextMenu />
            </template>
          </Canvas>
        </div>
      </div>
      <InspectorPanel />
    </div>
    <AssetTray />
    <RunLog />
  </template>
</template>
