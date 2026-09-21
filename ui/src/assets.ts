// The asset tray's own state, and the one drag that leaves it.
//
// A project is a folder, so an asset is a file in it and its path is relative
// to that folder with forward slashes (`assets/player.png`) - the same
// spelling a `Script` component uses, so what the tray hands an input is what
// the document stores.
//
// Dragging an asset onto an input is a plain HTML5 drag. The payload rides on
// the dataTransfer for anything outside the app, but a drop target also needs
// to know *during* the drag whether it wants what is coming - and dataTransfer
// won't say until the drop - so the entry being dragged is mirrored here.
import { reactive, ref } from 'vue';
import type { AssetEntry, AssetKind } from './types';

/** The drag payload's own type, so an asset dropped anywhere else is ignored. */
export const ASSET_MIME = 'application/x-blockloom-asset';

/** The asset under the cursor right now, while one is being dragged. */
export const dragged = ref<AssetEntry | null>(null);

/** Whether the tray is open and how tall it is, remembered between sessions.
 * Only per-eye settings live here - nothing the project owns. */
const OPEN_KEY = 'blockloom.assets.open';
const HEIGHT_KEY = 'blockloom.assets.height';

export const MIN_TRAY_HEIGHT = 120;
export const MAX_TRAY_HEIGHT = 520;

function remembered(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    // A page with storage blocked still works; it just forgets.
    return null;
  }
}

function remember(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // As above.
  }
}

function readHeight(): number {
  const stored = Number(remembered(HEIGHT_KEY));
  if (!Number.isFinite(stored) || stored <= 0) return 200;
  return Math.min(MAX_TRAY_HEIGHT, Math.max(MIN_TRAY_HEIGHT, stored));
}

/** What the tray is showing, kept out of the component so collapsing and
 * reopening it doesn't lose your place. */
export const tray = reactive({
  open: remembered(OPEN_KEY) !== 'false',
  height: readHeight(),
  /** The folder being listed, relative to the project folder. */
  path: '',
  selected: '',
});

export function setTrayOpen(open: boolean): void {
  tray.open = open;
  remember(OPEN_KEY, String(open));
}

export function setTrayHeight(height: number): void {
  tray.height = Math.min(MAX_TRAY_HEIGHT, Math.max(MIN_TRAY_HEIGHT, Math.round(height)));
  remember(HEIGHT_KEY, String(tray.height));
}

export function startAssetDrag(e: DragEvent, entry: AssetEntry): void {
  dragged.value = entry;
  if (!e.dataTransfer) return;
  e.dataTransfer.effectAllowed = 'copyMove';
  e.dataTransfer.setData(ASSET_MIME, JSON.stringify(entry));
  // Plain text too, so dropping one in a text editor pastes its path.
  e.dataTransfer.setData('text/plain', entry.path);
}

export function endAssetDrag(): void {
  dragged.value = null;
}

/** The asset a drop is carrying, or null if it isn't carrying one. */
export function droppedAsset(e: DragEvent): AssetEntry | null {
  const raw = e.dataTransfer?.getData(ASSET_MIME);
  if (raw) {
    try {
      return JSON.parse(raw) as AssetEntry;
    } catch {
      return null;
    }
  }
  // CEF hides custom types from a same-page drop often enough that the
  // mirrored entry is the reliable answer; the dataTransfer is the fallback.
  return dragged.value;
}

/** Whether a target taking `accept` kinds wants `entry`. No list means any
 * file - but never a folder, which no input can hold. */
export function accepts(accept: AssetKind[] | undefined, entry: AssetEntry | null): boolean {
  if (!entry || entry.kind === 'folder') return false;
  return !accept || accept.includes(entry.kind);
}

/** "12 KB" - the one number a file listing owes you. */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** The parent of a project-relative path - `''` at the project folder. */
export function parentOf(path: string): string {
  const cut = path.lastIndexOf('/');
  return cut === -1 ? '' : path.slice(0, cut);
}

/** Each folder on the way down to `path`, for the breadcrumb. */
export function crumbs(path: string): { name: string; path: string }[] {
  const parts = path.split('/').filter(Boolean);
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join('/') }));
}
