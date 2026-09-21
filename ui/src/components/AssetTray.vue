<script setup lang="ts">
// The asset tray: a small file manager over the project folder, along the
// bottom of the editor.
//
// A project is a folder, so this is that folder - `project.blockloom` beside
// the `assets/` everything else lives in. You can walk into subfolders, make
// folders and files, import from anywhere on the machine, rename, move and
// delete. Dragging a file out of here onto an input is what puts it in the
// document: see `AssetDrop.vue`, and the Image row in the inspector.
//
// The listing isn't part of the app snapshot - a folder on disk changes for
// reasons the editor never hears about - so the tray asks for it after every
// change of its own, and has a refresh button for everyone else's.
import { computed, ref, watch, type Component } from 'vue';
import { ContextMenuPanel, type ContextMenuItem } from 'blockstitch';
import {
  Box,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Download,
  File as FileIcon,
  FileCode,
  FilePlus,
  FileText,
  FileType,
  Folder,
  FolderPlus,
  House,
  Image as ImageIcon,
  Music,
  Pencil,
  RefreshCw,
  Trash2,
  Upload,
} from 'lucide-vue-next';
import { state } from '../store';
import {
  createAsset,
  createAssetFolder,
  deleteAsset,
  importAssets,
  listAssets,
  moveAsset,
  pickFiles,
  readAsset,
  renameAsset,
} from '../tauri';
import {
  crumbs,
  dragged,
  droppedAsset,
  endAssetDrag,
  fileSize,
  parentOf,
  setTrayHeight,
  setTrayOpen,
  startAssetDrag,
  tray,
} from '../assets';
import type { AssetEntry, AssetKind } from '../types';

const entries = ref<AssetEntry[]>([]);
const error = ref('');
const busy = ref(false);
/** Image previews by `path@modified`, so a replaced file gets a fresh one. */
const thumbnails = ref<Record<string, string>>({});

/** The tile being named: a new folder, a new file, or one being renamed. */
const drafting = ref<{ mode: 'folder' | 'file' | 'rename'; path: string; name: string } | null>(null);
const draftInput = ref<HTMLInputElement | null>(null);

const menu = ref<{ x: number; y: number; entry: AssetEntry } | null>(null);
/** The folder a dragged asset is hovering over, for the drop highlight. */
const dropTarget = ref<string | null>(null);

const ICONS: Record<AssetKind, Component> = {
  folder: Folder,
  image: ImageIcon,
  audio: Music,
  font: FileType,
  model: Box,
  script: FileCode,
  text: FileText,
  other: FileIcon,
};

const trail = computed(() => crumbs(tray.path));

async function refresh() {
  if (!state.project_path) {
    entries.value = [];
    return;
  }
  busy.value = true;
  try {
    entries.value = await listAssets(tray.path);
    error.value = '';
    void loadThumbnails();
  } catch (e) {
    entries.value = [];
    // A folder deleted from under us drops the tray back to the top rather
    // than leaving it stuck somewhere that no longer exists.
    if (tray.path) tray.path = '';
    else error.value = String(e);
  } finally {
    busy.value = false;
  }
}

/** Reads every image in the listing back as a data URL - the page has no
 * other way to see a file on disk. */
async function loadThumbnails() {
  for (const entry of entries.value) {
    if (entry.kind !== 'image') continue;
    const key = `${entry.path}@${entry.modified}`;
    if (thumbnails.value[key]) continue;
    try {
      thumbnails.value = { ...thumbnails.value, [key]: await readAsset(entry.path) };
    } catch {
      // Too big to preview, or gone again. The icon stands in.
    }
  }
}

function thumbnail(entry: AssetEntry): string | null {
  return thumbnails.value[`${entry.path}@${entry.modified}`] ?? null;
}

// A different project is a different folder tree, so nothing carries over.
watch(
  () => state.project_path,
  () => {
    tray.path = '';
    tray.selected = '';
    thumbnails.value = {};
  },
);

watch(
  () => [state.project_path, tray.path, tray.open] as const,
  () => {
    if (tray.open) void refresh();
  },
  { immediate: true },
);

async function run(action: () => Promise<unknown>) {
  error.value = '';
  try {
    await action();
  } catch (e) {
    error.value = String(e);
  }
  await refresh();
}

// ─── Getting around ────────────────────────────────────────────────────────

function goTo(path: string) {
  tray.path = path;
  tray.selected = '';
}

function open(entry: AssetEntry) {
  if (entry.kind === 'folder') goTo(entry.path);
}

// ─── Making, naming and removing ───────────────────────────────────────────

/** One ref for whichever input is being typed into - only ever one at a
 * time, which is why the tile in a `v-for` can share it. */
function setDraftInput(el: unknown) {
  draftInput.value = (el as HTMLInputElement | null) ?? null;
}

function isRenaming(entry: AssetEntry): boolean {
  return drafting.value?.mode === 'rename' && drafting.value.path === entry.path;
}

function startDraft(mode: 'folder' | 'file') {
  closeMenu();
  drafting.value = { mode, path: '', name: mode === 'folder' ? 'New folder' : 'notes.txt' };
  focusDraft();
}

function startRename(entry: AssetEntry) {
  if (entry.protected) return;
  drafting.value = { mode: 'rename', path: entry.path, name: entry.name };
  focusDraft();
}

/** Selects the stem, not the extension: renaming a file rarely means changing
 * what it is. */
function focusDraft() {
  requestAnimationFrame(() => {
    const name = drafting.value?.name ?? '';
    const dot = name.lastIndexOf('.');
    draftInput.value?.focus();
    draftInput.value?.setSelectionRange(0, dot > 0 ? dot : name.length);
  });
}

async function commitDraft() {
  const draft = drafting.value;
  drafting.value = null;
  if (!draft || !draft.name.trim()) return;
  await run(() => {
    if (draft.mode === 'folder') return createAssetFolder(tray.path, draft.name);
    if (draft.mode === 'file') return createAsset(tray.path, draft.name);
    return renameAsset(draft.path, draft.name);
  });
}

async function importHere() {
  const paths = await pickFiles('Import into the project');
  if (!paths?.length) return;
  await run(() => importAssets(tray.path, paths));
}

function remove(entry: AssetEntry) {
  const what = entry.kind === 'folder' ? `"${entry.name}" and everything in it` : `"${entry.name}"`;
  if (!window.confirm(`Delete ${what}?\n\nThis cannot be undone.`)) return;
  void run(() => deleteAsset(entry.path));
}

function moveUp(entry: AssetEntry) {
  void run(() => moveAsset(entry.path, parentOf(parentOf(entry.path))));
}

// ─── Dragging ──────────────────────────────────────────────────────────────

function onDragStart(e: DragEvent, entry: AssetEntry) {
  tray.selected = entry.path;
  startAssetDrag(e, entry);
}

/** Whether the asset in flight could land in the folder at `path`. */
function canDropIn(path: string): boolean {
  const moving = dragged.value;
  return (
    !!moving &&
    !moving.protected &&
    moving.path !== path &&
    parentOf(moving.path) !== path &&
    !path.startsWith(`${moving.path}/`)
  );
}

function onDragOverFolder(e: DragEvent, path: string) {
  if (!canDropIn(path)) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  dropTarget.value = path;
}

function onDropOn(e: DragEvent, parent: string) {
  e.preventDefault();
  e.stopPropagation();
  dropTarget.value = null;
  const entry = droppedAsset(e);
  endAssetDrag();
  if (!entry || entry.protected || parentOf(entry.path) === parent) return;
  void run(() => moveAsset(entry.path, parent));
}

// ─── The right-click menu ──────────────────────────────────────────────────

function openMenu(e: MouseEvent, entry: AssetEntry) {
  tray.selected = entry.path;
  menu.value = { x: e.clientX, y: e.clientY, entry };
}

function closeMenu() {
  menu.value = null;
}

/** The project document is listed but can't be moved, renamed or deleted -
 * the app finds a project by that exact name - so it gets no menu at all.
 * The panel doesn't close itself on a choice, so each item does. */
const menuItems = computed<ContextMenuItem[]>(() => {
  const entry = menu.value?.entry;
  if (!entry || entry.protected) return [];
  const choose = (action: () => void) => () => {
    closeMenu();
    action();
  };
  const items: ContextMenuItem[] = [];
  if (entry.kind === 'folder') {
    items.push({ key: 'open', label: 'Open', icon: Folder, onSelect: choose(() => open(entry)) });
  }
  items.push(
    { key: 'rename', label: 'Rename', icon: Pencil, onSelect: choose(() => startRename(entry)) },
    {
      key: 'up',
      label: 'Move up one folder',
      icon: Upload,
      disabled: !tray.path,
      onSelect: choose(() => moveUp(entry)),
    },
    {
      key: 'delete',
      label: `Delete "${entry.name}"`,
      icon: Trash2,
      danger: true,
      onSelect: choose(() => remove(entry)),
    },
  );
  return items;
});

// ─── Resizing ──────────────────────────────────────────────────────────────

function startResize(e: MouseEvent) {
  e.preventDefault();
  const startY = e.clientY;
  const startHeight = tray.height;
  const onMove = (move: MouseEvent) => setTrayHeight(startHeight + (startY - move.clientY));
  const onUp = () => {
    document.removeEventListener('mousemove', onMove);
    document.removeEventListener('mouseup', onUp);
  };
  document.addEventListener('mousemove', onMove);
  document.addEventListener('mouseup', onUp);
}
</script>

<template>
  <section class="asset-tray" :class="{ open: tray.open }">
    <div v-if="tray.open" class="asset-grip" title="Drag to resize" @mousedown="startResize" />

    <div class="asset-head">
      <button
        class="asset-toggle"
        :title="tray.open ? 'Hide the assets' : 'Show the assets'"
        @click="setTrayOpen(!tray.open)"
      >
        <ChevronDown v-if="tray.open" :size="14" />
        <ChevronUp v-else :size="14" />
        <span>Assets</span>
      </button>

      <nav v-if="tray.open" class="asset-crumbs">
        <button
          class="crumb"
          :class="{ target: dropTarget === '' }"
          title="The project folder"
          @click="goTo('')"
          @dragover="onDragOverFolder($event, '')"
          @dragleave="dropTarget = null"
          @drop="onDropOn($event, '')"
        >
          <House :size="12" />
          <span>{{ state.project?.name ?? 'Project' }}</span>
        </button>
        <template v-for="crumb in trail" :key="crumb.path">
          <ChevronRight :size="12" class="crumb-arrow" />
          <button
            class="crumb"
            :class="{ target: dropTarget === crumb.path }"
            @click="goTo(crumb.path)"
            @dragover="onDragOverFolder($event, crumb.path)"
            @dragleave="dropTarget = null"
            @drop="onDropOn($event, crumb.path)"
          >
            {{ crumb.name }}
          </button>
        </template>
      </nav>

      <span class="spacer" />

      <template v-if="tray.open">
        <button class="btn-small" title="New folder" @click="startDraft('folder')">
          <FolderPlus :size="13" />
        </button>
        <button
          class="btn-small"
          title="New file - the extension says what it is, and a .rs gets the script template"
          @click="startDraft('file')"
        >
          <FilePlus :size="13" />
        </button>
        <button class="btn-small" title="Import files into this folder" @click="importHere">
          <Download :size="13" />
        </button>
        <button class="btn-small" title="Re-read this folder" :disabled="busy" @click="refresh">
          <RefreshCw :size="13" />
        </button>
      </template>
    </div>

    <div
      v-if="tray.open"
      class="asset-body"
      :style="{ height: `${tray.height}px` }"
      @click="tray.selected = ''"
      @dragover="onDragOverFolder($event, tray.path)"
      @drop="onDropOn($event, tray.path)"
    >
      <p v-if="error" class="dialog-error asset-error">{{ error }}</p>

      <div class="asset-tiles">
        <!-- Up one, as a tile: it is also somewhere to drop a file. -->
        <div
          v-if="tray.path"
          class="asset-tile up"
          :class="{ target: dropTarget === parentOf(tray.path) }"
          title="Up one folder"
          @click.stop="goTo(parentOf(tray.path))"
          @dragover="onDragOverFolder($event, parentOf(tray.path))"
          @dragleave="dropTarget = null"
          @drop="onDropOn($event, parentOf(tray.path))"
        >
          <span class="asset-icon"><ChevronUp :size="22" /></span>
          <span class="asset-name">..</span>
        </div>

        <!-- A new folder or file, being named before it exists. -->
        <div v-if="drafting && drafting.mode !== 'rename'" class="asset-tile drafting">
          <span class="asset-icon">
            <Folder v-if="drafting.mode === 'folder'" :size="22" />
            <FileIcon v-else :size="22" />
          </span>
          <input
            :ref="setDraftInput"
            class="asset-rename"
            v-model="drafting.name"
            @keydown.enter="commitDraft"
            @keydown.esc="drafting = null"
            @blur="commitDraft"
            @click.stop
          >
        </div>

        <div
          v-for="entry in entries"
          :key="entry.path"
          class="asset-tile"
          :class="{
            selected: tray.selected === entry.path,
            target: dropTarget === entry.path,
            folder: entry.kind === 'folder',
          }"
          :draggable="!isRenaming(entry)"
          :title="`${entry.path}${entry.kind === 'folder' ? '' : ` · ${fileSize(entry.size)}`}`"
          @click.stop="tray.selected = entry.path"
          @dblclick="open(entry)"
          @contextmenu.prevent.stop="openMenu($event, entry)"
          @dragstart="onDragStart($event, entry)"
          @dragend="endAssetDrag"
          @dragover="entry.kind === 'folder' && onDragOverFolder($event, entry.path)"
          @dragleave="dropTarget = null"
          @drop="entry.kind === 'folder' && onDropOn($event, entry.path)"
        >
          <span class="asset-icon">
            <img v-if="thumbnail(entry)" :src="thumbnail(entry)!" alt="" class="asset-thumb">
            <component :is="ICONS[entry.kind]" v-else :size="22" />
          </span>
          <input
            v-if="isRenaming(entry)"
            :ref="setDraftInput"
            class="asset-rename"
            v-model="drafting!.name"
            @keydown.enter="commitDraft"
            @keydown.esc="drafting = null"
            @blur="commitDraft"
            @click.stop
          >
          <span v-else class="asset-name">{{ entry.name }}</span>
        </div>
      </div>

      <p v-if="!entries.length && !error" class="panel-note asset-empty">
        Nothing here yet. Import a sprite or a sound, or make a folder to put
        one in - then drag it onto a component that takes it.
      </p>
    </div>

    <ContextMenuPanel
      :open="!!menu && menuItems.length > 0"
      :x="menu?.x ?? 0"
      :y="menu?.y ?? 0"
      :items="menuItems"
      @close="closeMenu"
    />
  </section>
</template>
