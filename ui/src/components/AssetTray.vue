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
import { computed, onMounted, onUnmounted, ref, watch, type Component } from 'vue';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { ContextMenuPanel, type ContextMenuItem } from 'blockstitch';
import {
  Box,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Download,
  ExternalLink,
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
  Pause,
  Pencil,
  Play,
  RefreshCw,
  Sparkles,
  Trash2,
  Upload,
} from 'lucide-vue-next';
import { state } from '../store';
import { isTauri } from '../bridge';
import {
  createAsset,
  createAssetFolder,
  deleteAsset,
  importAssets,
  listAssets,
  moveAsset,
  openAssetLocation,
  pickFiles,
  pipelineStatus,
  readAsset,
  reimportAssets,
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
import type { AssetEntry, AssetKind, PipelineReportDto } from '../types';

const entries = ref<AssetEntry[]>([]);
const error = ref('');
const busy = ref(false);
/** What the pipeline makes of each asset, by path. Best-effort: a project
 * with no pipeline state yet simply shows no badges. */
const reports = ref<Record<string, PipelineReportDto>>({});
/** The audio file previewing right now, by path. One at a time: starting
 * another stops the first. */
const previewing = ref<string | null>(null);
let previewAudio: HTMLAudioElement | null = null;

function stopPreview() {
  previewAudio?.pause();
  previewAudio = null;
  previewing.value = null;
}

/** Clicking a sound's icon plays it in place, through the browser's own
 * audio - the game runtime isn't involved. Click again to stop. */
async function togglePreview(entry: AssetEntry) {
  if (previewing.value === entry.path) {
    stopPreview();
    return;
  }
  stopPreview();
  try {
    const url = await readAsset(entry.path);
    const audio = new Audio(url);
    previewAudio = audio;
    previewing.value = entry.path;
    audio.onended = () => {
      if (previewing.value === entry.path) stopPreview();
    };
    await audio.play();
  } catch (err) {
    console.error(err);
    stopPreview();
  }
}

onUnmounted(stopPreview);
/** Image previews by `path@modified`, so a replaced file gets a fresh one. */
const thumbnails = ref<Record<string, string>>({});

/** The tile being named: a new folder, a new file, or one being renamed. */
const drafting = ref<{ mode: 'folder' | 'file' | 'rename'; path: string; name: string } | null>(null);
const draftInput = ref<HTMLInputElement | null>(null);

const menu = ref<{ x: number; y: number; entry: AssetEntry | null } | null>(null);
/** The folder a dragged asset is hovering over, for the drop highlight. */
const dropTarget = ref<string | null>(null);

/** A native OS drag over the window. The CEF runtime keeps the dragged paths
 * for itself, so the DOM only sees an opaque `Files` drag until the tauri
 * drag-drop event arrives with them. */
const nativeDragActive = ref(false);
/** The folder under a native drop, recorded by the DOM drop just before the
 * tauri drag-drop event arrives with the actual paths. */
const pendingDropParent = ref<string | null>(null);
let offDragDrop: (() => void) | undefined;

const ICONS: Record<AssetKind, Component> = {
  folder: Folder,
  image: ImageIcon,
  audio: Music,
  font: FileType,
  model: Box,
  script: FileCode,
  shader: Sparkles,
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
    void loadReports();
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

/** The pipeline's take on every asset, for the tray badges. A scan that
 * fails - a huge project, a backend that predates the pipeline - leaves the
 * old badges up rather than clearing them. */
async function loadReports() {
  try {
    const statuses = await pipelineStatus();
    const next: Record<string, PipelineReportDto> = {};
    for (const report of statuses) next[report.path] = report;
    reports.value = next;
  } catch {
    // No badges is a working tray; a broken one isn't.
  }
}

function report(entry: AssetEntry): PipelineReportDto | null {
  return reports.value[entry.path] ?? null;
}

/** A tile's tooltip: where it is, how big, and what the pipeline says. */
function tileTitle(entry: AssetEntry): string {
  const head = `${entry.path}${entry.kind === 'folder' ? '' : ` · ${fileSize(entry.size)}`}`;
  const summary = report(entry)?.summary;
  return summary ? `${head}\n${summary}` : head;
}

/** Re-inspects one file and refreshes its fingerprint. */
async function reimport(entry: AssetEntry) {
  await run(() => reimportAssets([entry.path]).then(() => undefined));
}

/** Re-inspects everything the pipeline flags as changed. */
async function reimportChanged() {
  const dirty = Object.values(reports.value)
    .filter(r => r.dirty)
    .map(r => r.path);
  if (!dirty.length) return;
  await run(() => reimportAssets(dirty).then(() => undefined));
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

/** Opens the native file manager without refreshing the listing - nothing in
 * the tray changed. `path` is relative to the project folder. */
async function openLocation(path: string) {
  error.value = '';
  try {
    await openAssetLocation(path);
  } catch (e) {
    error.value = String(e);
  }
}

// ─── Dragging ──────────────────────────────────────────────────────────────

function onDragStart(e: DragEvent, entry: AssetEntry) {
  tray.selected = entry.path;
  startAssetDrag(e, entry);
}

/** Whether the drag comes from outside the app - a native OS drag. The tray's
 * own asset drag only carries its MIME type and a `text/plain` path, so never
 * collides. The `Files` dataTransfer type covers the drag before the tauri
 * events arrive and `nativeDragActive` reliably from then on. */
function isNativeDrag(e: DragEvent): boolean {
  return (e.dataTransfer?.types.includes('Files') ?? false) || nativeDragActive.value;
}

/** Whether the thing in flight could land in the folder at `path`. Native
 * drops are always welcome; a tray asset must be movable there. */
function canDropIn(e: DragEvent, path: string): boolean {
  if (isNativeDrag(e)) return true;
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
  if (!canDropIn(e, path)) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.dataTransfer) e.dataTransfer.dropEffect = isNativeDrag(e) ? 'copy' : 'move';
  dropTarget.value = path;
}

function onDropOn(e: DragEvent, parent: string) {
  e.preventDefault();
  e.stopPropagation();
  dropTarget.value = null;
  if (isNativeDrag(e)) {
    // The dropped paths come through the tauri drag-drop event, which lands
    // on the webview a beat after the DOM drop. Remember the folder so it
    // imports there.
    pendingDropParent.value = parent;
    return;
  }
  const entry = droppedAsset(e);
  endAssetDrag();
  if (!entry || entry.protected || parentOf(entry.path) === parent) return;
  void run(() => moveAsset(entry.path, parent));
}

// ─── Importing a native drag ───────────────────────────────────────────────

/** Files and folders dropped from the OS file manager, imported into `parent`
 * whole - the same import behind the toolbar's button. A dropped folder is
 * one such path, which the backend copies as a tree, so structure lands
 * intact. */
async function importNativePaths(paths: string[], parent: string) {
  if (!paths.length) return;
  await run(() => importAssets(parent, paths));
}

/** The folder to import into when no DOM drop handler recorded one. The tauri
 * events know nothing of the tray's layout, so the drop's position hit-tests
 * the page: anything landing inside `.asset-tray` goes into the folder being
 * listed, anything outside it is the window ignoring the drop. */
function parentAt(position: { x: number; y: number }): string | null {
  const el = document.elementFromPoint(
    position.x / window.devicePixelRatio,
    position.y / window.devicePixelRatio,
  );
  return el?.closest('.asset-tray') ? tray.path : null;
}

// The CEF runtime intercepts native drags before the page sees them and
// re-sends them as `tauri://drag-*` events carrying real paths, so the DOM
// drop is only a where signal and this listener does the importing. Only the
// Tauri window has them - the dev-bridge browser tab has no native drops.
onMounted(async () => {
  if (!isTauri) return;
  offDragDrop = await getCurrentWebview().onDragDropEvent(({ payload }) => {
    switch (payload.type) {
      case 'enter':
        nativeDragActive.value = true;
        break;
      case 'over':
        nativeDragActive.value = true;
        break;
      case 'leave':
        nativeDragActive.value = false;
        pendingDropParent.value = null;
        dropTarget.value = null;
        break;
      case 'drop':
        nativeDragActive.value = false;
        dropTarget.value = null;
        // The DOM drop recorded the hovered folder first. If none did, the
        // drop landed on a part of the tray with no handler of its own, so
        // aim at the point it landed; outside the tray, import nothing.
        const parent = pendingDropParent.value ?? parentAt(payload.position);
        pendingDropParent.value = null;
        if (parent !== null) void importNativePaths(payload.paths, parent);
        break;
    }
  });
});

onUnmounted(() => offDragDrop?.());

// ─── The right-click menu ──────────────────────────────────────────────────

function openMenu(e: MouseEvent, entry: AssetEntry) {
  tray.selected = entry.path;
  menu.value = { x: e.clientX, y: e.clientY, entry };
}

/** Right-click on the empty tray: same folder actions, aimed at the folder
 * being listed rather than one item. */
function openBackgroundMenu(e: MouseEvent) {
  tray.selected = '';
  menu.value = { x: e.clientX, y: e.clientY, entry: null };
}

function closeMenu() {
  menu.value = null;
}

/** The project document is listed but can't be renamed, moved or deleted,
 * and right-clicking it gets no menu at all - the app finds a project by
 * that exact name. The panel doesn't close itself on a choice, so each item
 * does. */
const menuItems = computed<ContextMenuItem[]>(() => {
  const entry = menu.value?.entry ?? null;
  const choose = (action: () => void) => () => {
    closeMenu();
    action();
  };
  // No entry is the background: actions for the folder being listed.
  if (!entry) {
    const background: ContextMenuItem[] = [
      {
        key: 'location',
        label: 'Open File Location',
        icon: ExternalLink,
        onSelect: choose(() => void openLocation(tray.path)),
      },
      { key: 'new-folder', label: 'New folder', icon: FolderPlus, onSelect: choose(() => startDraft('folder')) },
      { key: 'new-file', label: 'New file', icon: FilePlus, onSelect: choose(() => startDraft('file')) },
      { key: 'import', label: 'Import files here', icon: Download, onSelect: choose(() => void importHere()) },
      { key: 'refresh', label: 'Re-read this folder', icon: RefreshCw, onSelect: choose(() => void refresh()) },
    ];
    if (Object.values(reports.value).some(r => r.dirty)) {
      background.push({
        key: 'reimport',
        label: 'Reimport changed files',
        icon: Sparkles,
        onSelect: choose(() => void reimportChanged()),
      });
    }
    return background;
  }
  if (entry.protected) return [];
  const items: ContextMenuItem[] = [
    {
      key: 'location',
      label: 'Open File Location',
      icon: ExternalLink,
      onSelect: choose(() => void openLocation(entry.path)),
    },
  ];
  if (report(entry)?.dirty) {
    items.push({
      key: 'reimport',
      label: 'Reimport this file',
      icon: Sparkles,
      onSelect: choose(() => void reimport(entry)),
    });
  }
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

/** Whether the resizer is being dragged, so it can keep its highlight beyond
 * the pointer leaving the handle itself. */
const resizing = ref(false);

function startResize(e: PointerEvent) {
  if (e.button !== undefined && e.button !== 0) return;
  e.preventDefault();
  const startY = e.clientY;
  const startHeight = tray.height;
  resizing.value = true;
  document.body.classList.add('asset-resizing');
  const onMove = (move: PointerEvent) => setTrayHeight(startHeight + (startY - move.clientY));
  const onUp = () => {
    resizing.value = false;
    document.removeEventListener('pointermove', onMove);
    document.removeEventListener('pointerup', onUp);
    document.removeEventListener('pointercancel', onUp);
    document.body.classList.remove('asset-resizing');
  };
  document.addEventListener('pointermove', onMove);
  document.addEventListener('pointerup', onUp);
  document.addEventListener('pointercancel', onUp);
}
</script>

<template>
  <section class="asset-tray" :class="{ open: tray.open }" @dragover.prevent @drop="onDropOn($event, tray.path)">
    <div
      v-if="tray.open"
      class="asset-resizer"
      :class="{ active: resizing }"
      title="Drag to resize"
      @pointerdown="startResize"
    >
      <span class="asset-resizer-grip" />
    </div>

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
      @contextmenu.prevent.stop="openBackgroundMenu($event)"
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
          :title="tileTitle(entry)"
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
            <span
              v-if="report(entry)?.dirty"
              class="asset-dirty"
              :title="`Changed since import - ${report(entry)?.summary ?? ''}`"
            />
            <button
              v-if="entry.kind === 'audio'"
              class="asset-preview"
              :title="previewing === entry.path ? 'Stop preview' : 'Preview this sound'"
              @click.stop="togglePreview(entry)"
            >
              <component :is="previewing === entry.path ? Pause : Play" :size="12" />
            </button>
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
