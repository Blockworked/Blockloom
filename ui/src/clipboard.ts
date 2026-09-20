// The editor's own copy/paste buffer for blocks - not the OS clipboard.
import { resolveInstructionAt, resolveInstructionList } from 'blockstitch';
import type { InstrPath, InstructionDto, StrandDto } from './types';

let copied: InstructionDto[] | null = null;

/** Copies one block on its own, without whatever is stacked below it. */
export function copyBlock(strand: StrandDto, path: InstrPath): void {
  const instruction = resolveInstructionAt(strand, path);
  if (instruction) copied = [instruction as InstructionDto];
}

/** Copies a block and everything below it in the same list. */
export function copyStack(strand: StrandDto, path: InstrPath): void {
  if (path.length === 0) return;
  const list = resolveInstructionList(strand, path.slice(0, -1));
  copied = list.slice(path[path.length - 1].index) as InstructionDto[];
}

export function hasClipboard(): boolean {
  return copied !== null && copied.length > 0;
}

export function clipboardContents(): InstructionDto[] | null {
  return copied;
}
