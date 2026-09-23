<script setup lang="ts">
// Two pages: the Dashboard the app starts on, and the editor a project opens
// into - a top bar, the actor list, the block palette and canvas, the
// inspector, the asset tray and the run log.
import { computed, onMounted, onUnmounted, watch } from 'vue';
import { Canvas, DictEditorOverlay, ListEditorOverlay, activateDictEditors, activateListEditors, isDictEditorOpen, isListEditorOpen, setDictEditorOpen, setListEditorOpen } from 'blockstitch';
import { initState, openActor, state } from './store';
import { redo, renameDict, renameList, resetZoom, setDictEntries, setListItems, undo } from './tauri';
import Dashboard from './components/Dashboard.vue';
import TopBar from './components/TopBar.vue';
import ActorList from './components/ActorList.vue';
import BlockSidebar from './components/BlockSidebar.vue';
import InspectorPanel from './components/InspectorPanel.vue';
import AssetTray from './components/AssetTray.vue';
import RunLog from './components/RunLog.vue';
import ContextMenu from './components/ContextMenu.vue';

// A list's canvas monitor, for the open actor's own lists and the project's
// shared ones. An actor's own list shadows a shared one of the same name,
// the same rule blocks read by.
const overlayLists = computed(() => {
  const own = openActor.value?.lists ?? [];
  const shared = (state.project?.global_lists ?? []).filter(
    list => !own.some(candidate => candidate.name === list.name),
  );
  return [...own, ...shared].filter(list => isListEditorOpen(list.name));
});

// A dict's canvas monitor, for the open actor's own dicts and the project's
// shared ones, with the same shadowing rule.
const overlayDicts = computed(() => {
  const own = openActor.value?.dicts ?? [];
  const shared = (state.project?.global_dicts ?? []).filter(
    dict => !own.some(candidate => candidate.name === dict.name),
  );
  return [...own, ...shared].filter(dict => isDictEditorOpen(dict.name));
});

watch(
  () => openActor.value?.id ?? null,
  id => {
    activateListEditors(id, [
      ...(openActor.value?.lists ?? []),
      ...(state.project?.global_lists ?? []),
    ]);
    activateDictEditors(id, [
      ...(openActor.value?.dicts ?? []),
      ...(state.project?.global_dicts ?? []),
    ]);
  },
  { immediate: true },
);

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
            <template #overlay>
              <ListEditorOverlay
                v-for="list in overlayLists"
                :key="list.name"
                :list="list"
                :on-save-items="(name, items) => setListItems(name, items)"
                :on-rename="(oldName, newName) => renameList(oldName, newName)"
                :on-hide="name => setListEditorOpen(name, false)"
              />
              <DictEditorOverlay
                v-for="dict in overlayDicts"
                :key="dict.name"
                :dict="dict"
                :on-save-entries="(name, entries) => setDictEntries(name, entries)"
                :on-rename="(oldName, newName) => renameDict(oldName, newName)"
                :on-hide="name => setDictEditorOpen(name, false)"
              />
            </template>
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
