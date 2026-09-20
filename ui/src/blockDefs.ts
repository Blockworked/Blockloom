// Per-actor palette state for "My Blocks": the live argument list a custom
// block's sidebar prefab carries. Unlike the fixed operator table, custom
// blocks come and go with the open actor, so this stays synced with its
// `block_defs` rather than being a fixed record.
import { reactive, watch } from 'vue';
import { openActor } from './store';
import { blockInputPieces, newId, numberValue } from './types';
import type { BlockPieceDto, InstructionDto, ValueDto } from './types';

export const paletteCallArgs = reactive<Record<string, ValueDto[]>>({});

/** A blank argument matching an input's declared type - a boolean input gets an
 * empty hexagon, everything else a zero. */
function blankArg(piece: Extract<BlockPieceDto, { kind: 'Input' }> | undefined): ValueDto {
  return piece?.value_type === 'Bool' ? { kind: 'Bool' } : numberValue(0);
}

watch(
  () => openActor.value?.block_defs,
  defs => {
    const list = defs ?? [];
    const live = new Set(list.map(def => def.id));
    for (const id of Object.keys(paletteCallArgs)) {
      if (!live.has(id)) delete paletteCallArgs[id];
    }
    for (const def of list) {
      const inputs = blockInputPieces(def);
      const existing = paletteCallArgs[def.id] ?? [];
      paletteCallArgs[def.id] = inputs.map((piece, i) => existing[i] ?? blankArg(piece));
    }
  },
  { immediate: true, deep: true },
);

function currentArgs(blockId: string): ValueDto[] {
  const def = openActor.value?.block_defs.find(candidate => candidate.id === blockId);
  const inputs = def ? blockInputPieces(def) : null;
  const count = inputs ? inputs.length : (paletteCallArgs[blockId]?.length ?? 0);
  const existing = paletteCallArgs[blockId] ?? [];
  return Array.from({ length: count }, (_, i) => existing[i] ?? blankArg(inputs?.[i]));
}

/** The value a reporter block's prefab represents. */
export function paletteCallValueFor(blockId: string): ValueDto {
  return { kind: 'Call', block_id: blockId, args: currentArgs(blockId), branches: [], saved: numberValue(0) };
}

/** The instruction a command block's prefab represents. */
export function paletteCallInstructionFor(blockId: string): InstructionDto {
  return { id: newId(), type: 'CallBlock', block_id: blockId, args: currentArgs(blockId) };
}
