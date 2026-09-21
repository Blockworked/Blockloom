// The editor's vertical panels - the block sidebar, the actor list and the
// components inspector - collapse to a thin rail, and the two app-owned panels
// also drag to resize. Only per-eye settings live here, like `assets.ts`;
// nothing the project owns. The block sidebar's width is blockstitch's own
// `sidebarWidth`, so only its collapse state is kept here.
import { reactive } from 'vue';

export type PanelSide = 'left' | 'right';

const OPEN_BLOCKS = 'blockloom.panels.blocks.open';
const OPEN_LEFT = 'blockloom.panels.left.open';
const OPEN_RIGHT = 'blockloom.panels.right.open';
const WIDTH_LEFT = 'blockloom.panels.left.width';
const WIDTH_RIGHT = 'blockloom.panels.right.width';

/** How wide a collapsed side panel stays, so it can be clicked back open. */
export const COLLAPSED_PANEL_WIDTH = 34;
export const MIN_PANEL_WIDTH = 180;
export const MAX_PANEL_WIDTH = 420;

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

function readOpen(key: string): boolean {
  return remembered(key) !== 'false';
}

function readWidth(key: string, fallback: number): number {
  const stored = Number(remembered(key));
  if (!Number.isFinite(stored) || stored <= 0) return fallback;
  return Math.min(MAX_PANEL_WIDTH, Math.max(MIN_PANEL_WIDTH, stored));
}

export const panels = reactive({
  blocks: { open: readOpen(OPEN_BLOCKS) },
  left: { open: readOpen(OPEN_LEFT), width: readWidth(WIDTH_LEFT, 220) },
  right: { open: readOpen(OPEN_RIGHT), width: readWidth(WIDTH_RIGHT, 268) },
});

const openKey = (side: PanelSide): string => (side === 'left' ? OPEN_LEFT : OPEN_RIGHT);
const widthKey = (side: PanelSide): string => (side === 'left' ? WIDTH_LEFT : WIDTH_RIGHT);

export function setBlocksOpen(open: boolean): void {
  panels.blocks.open = open;
  remember(OPEN_BLOCKS, String(open));
}

export function setPanelOpen(side: PanelSide, open: boolean): void {
  panels[side].open = open;
  remember(openKey(side), String(open));
}

export function setPanelWidth(side: PanelSide, width: number): void {
  const clamped = Math.min(MAX_PANEL_WIDTH, Math.max(MIN_PANEL_WIDTH, Math.round(width)));
  panels[side].width = clamped;
  remember(widthKey(side), String(clamped));
}

// A pointer-drag on a panel's inner edge resizes it, using the same
// attach-listeners-for-the-duration shape as blockstitch's sidebar resize.
const drag = reactive({ side: null as PanelSide | null, pointer: -1, startX: 0, startWidth: 0 });

function onPointerMove(e: PointerEvent) {
  if (!drag.side || e.pointerId !== drag.pointer) return;
  // A right panel's handle is on its left edge, where the right edge of the
  // panel stays put - so the drag maps backwards there.
  const delta = e.clientX - drag.startX;
  setPanelWidth(drag.side, drag.side === 'right' ? drag.startWidth - delta : drag.startWidth + delta);
}

function endDrag(e: PointerEvent) {
  if (!drag.side || e.pointerId !== drag.pointer) return;
  drag.side = null;
  document.removeEventListener('pointermove', onPointerMove);
  document.removeEventListener('pointerup', endDrag);
  document.removeEventListener('pointercancel', endDrag);
  document.body.classList.remove('panel-resizing');
}

export function beginPanelResize(side: PanelSide, e: PointerEvent) {
  if (e.button !== undefined && e.button !== 0) return;
  e.preventDefault();
  drag.side = side;
  drag.pointer = e.pointerId;
  drag.startX = e.clientX;
  drag.startWidth = panels[side].width;
  document.body.classList.add('panel-resizing');
  document.addEventListener('pointermove', onPointerMove);
  document.addEventListener('pointerup', endDrag);
  document.addEventListener('pointercancel', endDrag);
}