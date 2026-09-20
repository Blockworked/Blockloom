// Right-click menu state. What each variant offers lives in
// `components/ContextMenu.vue`; this is only where the menu is and what it was
// opened on.
import { reactive } from 'vue';
import { clientToCanvas, registerOpen, unregisterOpen } from 'blockstitch';
import type { InstrPath, ValueDto } from './types';

export type ContextMenuType =
  | 'block'
  | 'canvas'
  | 'variable'
  | 'myBlock'
  | 'paletteInstruction'
  | 'paletteValue'
  | 'value';

export const contextMenu = reactive({
  open: false,
  x: 0,
  y: 0,
  type: 'block' as ContextMenuType,
  strandId: '',
  path: [] as InstrPath,
  canvasX: 0,
  canvasY: 0,
  variableName: '',
  blockId: '',
  paletteInstructionType: '',
  paletteVariantId: undefined as string | undefined,
  paletteValueKind: '',
  valueNode: null as ValueDto | null,
});

function openAt(e: MouseEvent) {
  registerOpen(closeContextMenu);
  contextMenu.open = true;
  contextMenu.x = e.clientX;
  contextMenu.y = e.clientY;
}

export function openBlockMenu(e: MouseEvent, strandId: string, path: InstrPath): void {
  openAt(e);
  contextMenu.type = 'block';
  contextMenu.strandId = strandId;
  contextMenu.path = path;
}

export function openCanvasMenu(e: MouseEvent): void {
  openAt(e);
  contextMenu.type = 'canvas';
  const [x, y] = clientToCanvas(e.clientX, e.clientY);
  contextMenu.canvasX = x;
  contextMenu.canvasY = y;
}

export function openVariableMenu(e: MouseEvent, name: string): void {
  openAt(e);
  contextMenu.type = 'variable';
  contextMenu.variableName = name;
}

export function openMyBlockMenu(e: MouseEvent, blockId: string): void {
  openAt(e);
  contextMenu.type = 'myBlock';
  contextMenu.blockId = blockId;
}

export function openPaletteInstructionMenu(e: MouseEvent, type: string, variantId?: string): void {
  openAt(e);
  contextMenu.type = 'paletteInstruction';
  contextMenu.paletteInstructionType = type;
  contextMenu.paletteVariantId = variantId;
}

export function openPaletteValueMenu(e: MouseEvent, kind: string): void {
  openAt(e);
  contextMenu.type = 'paletteValue';
  contextMenu.paletteValueKind = kind;
}

export function openValueMenu(e: MouseEvent, value: ValueDto): void {
  // A variable or parameter reporter has nothing to offer here.
  if (value.kind !== 'Op' && value.kind !== 'Call') return;
  openAt(e);
  contextMenu.type = 'value';
  contextMenu.valueNode = value;
}

export function closeContextMenu(): void {
  if (!contextMenu.open) return;
  contextMenu.open = false;
  unregisterOpen(closeContextMenu);
}
