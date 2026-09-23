// The one startup wiring point between Blockloom and blockstitch: it registers
// Blockloom's block vocabulary (shapes, rows, icons, operators) and builds the
// canvas host - the backend blockstitch drives and the document it edits.
// Called once, before the app mounts (see main.ts).
import {
  configureCanvas,
  locationsEqual,
  registerBlockField,
  registerBlockShape,
  registerIcons,
  registerOperators,
  registerPaletteBlockField,
  type CanvasBackend,
  type CanvasHost,
} from 'blockstitch';
import { BLOCK_SPECS, bodyKeysFor, defineBlockFields, definePaletteBlockFields } from './blockFields';
import { openActor, state } from './store';
import * as backendCalls from './tauri';
import { BLOCK_ICONS, ICONS } from './icons';
import { OPERATOR_KINDS } from './valueOps';
import { clonePaletteInstruction, paletteValueFor } from './paletteState';
import { paletteCallInstructionFor, paletteCallValueFor } from './blockDefs';
import {
  INSTRUCTION_TYPES,
  asBody,
  blockInputPieces,
  findBlockDef,
  parseParamKind,
  type InstructionDto,
  type InstructionType,
  type ValueDto,
} from './types';
import {
  openBlockMenu,
  openCanvasMenu,
  openPaletteInstructionMenu,
  openPaletteValueMenu,
  openValueMenu,
  openVariableMenu,
} from './contextMenu';
import BlockHeaderFields from './components/fields/BlockHeaderFields.vue';
import CallBlockFields from './components/fields/CallBlockFields.vue';

/** Blocks whose row depends on a `BlockDef` rather than their type, so they
 * can't come from the declarative table. */
const DOCUMENT_SHAPED: InstructionType[] = ['BlockHeader', 'CallBlock'];

const HEADER_TYPES: InstructionType[] = [
  'WhenStarted',
  'WhenKeyPressed',
  'WhenActionPressed',
  'WhenTouched',
  'WhenClicked',
  'WhenCollision',
  'WhenMessage',
  'WhenCloned',
  'BlockHeader',
];
const CAP_TYPES: InstructionType[] = ['Return', 'EscapeLoop', 'ContinueLoop', 'StopAll'];
const WRAP_TYPES: InstructionType[] = ['If', 'IfElse', 'Repeat', 'Forever', 'While'];

let didSetup = false;

export function setupBlockstitch(): void {
  if (didSetup) return;
  didSetup = true;

  registerIcons(ICONS);
  registerOperators(OPERATOR_KINDS);
  registerShapes();
  registerRows();
  configureCanvas(buildCanvasHost());
}

function iconFor(type: InstructionType) {
  return ICONS[BLOCK_ICONS[type]];
}

function registerShapes() {
  for (const type of INSTRUCTION_TYPES) {
    const icon = iconFor(type);
    if (WRAP_TYPES.includes(type)) {
      // Which nested lists a C-block owns comes from the same spec that draws
      // its mouths, so a shape can't drift from its row.
      const keys = bodyKeysFor(type);
      registerBlockShape<InstructionDto>(type, {
        kind: 'wrap',
        icon,
        getSlots: node => keys.map(key => asBody(node[key])),
        mapSlots: (node, fn) => ({
          ...node,
          ...Object.fromEntries(keys.map((key, slot) => [key, fn(asBody(node[key]), slot)])),
        }),
      });
    } else if (HEADER_TYPES.includes(type)) {
      registerBlockShape(type, { kind: 'header', icon, isEntryTrigger: type !== 'BlockHeader' });
    } else if (CAP_TYPES.includes(type)) {
      registerBlockShape(type, { kind: 'cap', icon });
    } else if (type === 'CallBlock') {
      // Every custom block shares this one type, so "ends the stack" can't be a
      // static registration - it depends on the block being called.
      registerBlockShape<InstructionDto>('CallBlock', {
        kind: 'stack',
        icon,
        isCap: node => findBlockDef(openActor.value, node.block_id)?.shape === 'Ending',
      });
    } else {
      registerBlockShape(type, { kind: 'stack', icon });
    }
  }
}

function registerRows() {
  for (const type of INSTRUCTION_TYPES) {
    if (DOCUMENT_SHAPED.includes(type)) continue;
    const spec = BLOCK_SPECS[type];
    registerBlockField(type, defineBlockFields(spec));
    registerPaletteBlockField(type, definePaletteBlockFields(spec));
  }
  registerBlockField('BlockHeader', BlockHeaderFields);
  registerBlockField('CallBlock', CallBlockFields);
}

function buildCanvasHost(): CanvasHost<InstructionDto> {
  const backend: CanvasBackend<InstructionDto> = {
    addInstruction: backendCalls.addInstruction,
    addStrand: backendCalls.addStrand,
    removeStrand: backendCalls.removeStrand,
    moveStrand: backendCalls.moveStrand,
    splitStrand: backendCalls.splitStrand,
    mergeStrand: backendCalls.mergeStrand,
    deleteBlockDef: backendCalls.deleteBlock,
    editValueField: backendCalls.editValueField,
    takeValue: backendCalls.takeValue,
    putValue: backendCalls.putValue,
    previewValue: backendCalls.previewValue,
    // `sourceKind` is only set for a fresh sidebar or header drag: recover the
    // block id a `Param:` oval packed into its drag kind, so a parameter parked
    // on open canvas still knows which block declared it.
    createFloatingValue: (x, y, value, sourceKind) =>
      backendCalls.createFloatingValue(
        x,
        y,
        value as ValueDto,
        sourceKind?.startsWith('Param:') ? parseParamKind(sourceKind).blockId : null,
      ),
    moveFloatingValue: backendCalls.moveFloatingValue,
    removeFloatingValue: backendCalls.removeFloatingValue,
    moveComment: backendCalls.moveComment,
    removeComment: backendCalls.removeComment,
    editCommentText: backendCalls.editCommentText,
    setCommentCollapsed: backendCalls.setCommentCollapsed,
  };

  return {
    // Keyed by actor id, so switching actors re-centers the canvas the way
    // opening a different document would.
    getDocument: () => {
      const actor = openActor.value;
      if (!actor) return null;
      return {
        id: actor.id,
        strands: actor.strands,
        floating_values: actor.floating_values,
        comments: actor.comments,
      };
    },
    backend,
    resolveFreshValue: kind =>
      kind.startsWith('Call:') ? paletteCallValueFor(kind.slice('Call:'.length)) : paletteValueFor(kind),
    clonePaletteInstruction: (type, variantId) =>
      type === 'CallBlock' && variantId
        ? paletteCallInstructionFor(variantId)
        : clonePaletteInstruction(type as InstructionType),
    onCanvasContextMenu: e => openCanvasMenu(e),
    onBlockContextMenu: (e, strandId, path) => openBlockMenu(e, strandId, path),
    onVariableContextMenu: (e, name) => openVariableMenu(e, name),
    onPaletteInstructionContextMenu: (e, type, variantId) => openPaletteInstructionMenu(e, type, variantId),
    onPaletteValueContextMenu: (e, kind) => openPaletteValueMenu(e, kind),
    onValueContextMenu: (e, _location, value) => openValueMenu(e, value as ValueDto),
    resolveCallPieces: blockId =>
      findBlockDef(openActor.value, blockId)?.pieces.map(piece =>
        piece.kind === 'Label' ? { kind: 'Label', text: piece.text } : { kind: 'Input' },
      ),
    floatingValueColor: floating =>
      floating.value.kind === 'Call'
        ? findBlockDef(openActor.value, floating.value.block_id)?.color
        : undefined,
    paramIsBool: (location, name) => {
      const isBool = (blockId: string) => {
        const def = findBlockDef(openActor.value, blockId);
        return def ? blockInputPieces(def).some(input => input.name === name && input.value_type === 'Bool') : false;
      };
      // A parameter sitting in a field lives in a strand, and a block's body
      // strand always starts with that block's own header - so walk to it.
      if (location.kind === 'Field') {
        const strand = openActor.value?.strands.find(candidate => candidate.id === location.strand_id);
        const header = strand?.instructions[0];
        if (header?.type === 'BlockHeader' && typeof header.block_id === 'string') {
          return isBool(header.block_id);
        }
      }
      // A parked one has no strand, but `createFloatingValue` recorded where it
      // came from.
      if (location.kind === 'Floating') {
        const floating = openActor.value?.floating_values.find(f => f.id === location.floating_id);
        if (floating?.origin_block_id) return isBool(floating.origin_block_id);
      }
      // Nothing recorded: fall back to "every block with an input of this name
      // agrees it's boolean", which is exact in the usual one-block case.
      const matches = (openActor.value?.block_defs ?? []).flatMap(def =>
        blockInputPieces(def).filter(input => input.name === name),
      );
      return matches.length > 0 && matches.every(input => input.value_type === 'Bool');
    },
    callIsBool: blockId => findBlockDef(openActor.value, blockId)?.shape === 'ReturnsBool',
    getInvalidText: location => {
      const entry = state.invalid_field_buffers.find(buffer => locationsEqual(buffer.location, location));
      if (!entry) return null;
      const trimmed = entry.text.trim();
      let invalid = true;
      if (trimmed !== '') {
        const parsed = Number(trimmed);
        if (!isNaN(parsed)) {
          // Only a loop count is whole-numbers-only - see
          // `blockloom_core::fields::FieldId::requires_integer`.
          const field = location.kind === 'Field' ? location.field_id : null;
          invalid = field === 'RepeatCount' ? !Number.isInteger(parsed) : false;
        }
      }
      return { text: entry.text, invalid };
    },
  };
}
