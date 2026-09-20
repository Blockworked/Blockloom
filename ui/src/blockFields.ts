// What every block row looks like, as data.
//
// blockstitch asks for one Vue component per block type (twice for a C-block:
// once for its head line, once for its mouth). Written by hand that would be
// seventy near-identical files, so each block is described once here as a list
// of pieces - a label, a value slot, a dropdown, a free-text field, a nested
// body - and two factories turn that into the canvas component and the lighter
// sidebar-prefab component. Adding a block means adding a row to `BLOCK_SPECS`.
//
// The two blocks whose shape depends on the document rather than their type
// (`BlockHeader` and `CallBlock`, which render a custom block's prototype) are
// still hand-written components - see `components/fields/`.

import { defineComponent, h, type Component, type PropType, type VNode } from 'vue';
import {
  AppDropdown,
  AutosizeInput,
  InstructionList,
  PaletteNumberField,
  ValueBlock,
  bodyBasePath,
  isCapType,
} from 'blockstitch';
import { editInstruction } from './tauri';
import { mode, openActor, state } from './store';
import { AXIS_OPTIONS, BODY_OPTIONS, KEY_OPTIONS, MOUSE_TARGET, VISIBLE_OPTIONS } from './constants';
import {
  asBody,
  asString,
  asValue,
  fieldLocation,
  variableNames,
  type InstrPath,
  type InstructionDto,
  type InstructionType,
  type Mode,
} from './types';

type Option = { value: string; label: string };
type Options = Option[] | (() => Option[]);

type Piece =
  | { kind: 'label'; text: string; mode?: Mode }
  /** A draggable value slot. `field` is the backend's field id, `key` the
   * instruction property holding it - they differ in wording only. */
  | { kind: 'value'; field: string; key: string; bool?: boolean; mode?: Mode }
  | {
      kind: 'dropdown';
      key: string;
      options: Options;
      placeholder?: string;
      mode?: Mode;
      /** For a non-string slot: how the chosen option maps onto it, and back. */
      encode?: (chosen: string) => unknown;
      decode?: (stored: unknown) => string;
    }
  | { kind: 'text'; key: string; placeholder?: string; mode?: Mode }
  /** A nested instruction list - a C-block's mouth. */
  | { kind: 'mouth'; slot: number; key: string }
  /** The bar between two mouths, as in if/else. `notchFrom` is the slot whose
   * last row sits against it, which decides whether its notch is drawn. */
  | { kind: 'midBar'; text: string; notchFrom: string };

interface BlockSpec {
  /** The block's one line, or a C-block's head line. */
  head: Piece[];
  /** A C-block's mouth (and any bar between two of them). */
  body?: Piece[];
}

const label = (text: string, options: { mode?: Mode } = {}): Piece => ({ kind: 'label', text, ...options });
const value = (field: string, key: string, options: { bool?: boolean; mode?: Mode } = {}): Piece => ({
  kind: 'value',
  field,
  key,
  ...options,
});
const mouth = (slot: number, key: string): Piece => ({ kind: 'mouth', slot, key });

/** Actor names, plus "mouse" where that's a sensible target. */
function actorOptions(withMouse: boolean): Option[] {
  const actors = (state.project?.actors ?? []).map(actor => ({ value: actor.name, label: actor.name }));
  return withMouse ? [...actors, { value: MOUSE_TARGET, label: 'the mouse' }] : actors;
}

/** Actor names, plus "anything" - an empty target means any collision at all. */
function collisionOptions(): Option[] {
  return [{ value: '', label: 'anything' }, ...actorOptions(false)];
}

function variableOptions(): Option[] {
  return variableNames(state.project, openActor.value).map(name => ({ value: name, label: name }));
}

/** Axes that mean something for a position in this dimension. */
function positionAxes(): Option[] {
  return mode.value === 'ThreeD' ? AXIS_OPTIONS : AXIS_OPTIONS.filter(axis => axis.value !== 'Z');
}

/** Axes that mean something for a rotation: in 2D, only turning within the
 * plane, which is Z. */
function rotationAxes(): Option[] {
  return mode.value === 'ThreeD' ? AXIS_OPTIONS : AXIS_OPTIONS.filter(axis => axis.value === 'Z');
}

/** The x/y/z row shared by every vector block, with z hidden in 2D. */
function vector(prefix: string, fields: [string, string, string], keys: [string, string, string]): Piece[] {
  return [
    label(`${prefix} x:`),
    value(fields[0], keys[0]),
    label('y:'),
    value(fields[1], keys[1]),
    label('z:', { mode: 'ThreeD' }),
    value(fields[2], keys[2], { mode: 'ThreeD' }),
  ];
}

export const BLOCK_SPECS: Record<InstructionType, BlockSpec> = {
  // ── Events ───────────────────────────────────────────────────────────────
  WhenStarted: { head: [label('when the project starts')] },
  WhenKeyPressed: {
    head: [label('when'), { kind: 'dropdown', key: 'key', options: KEY_OPTIONS }, label('pressed')],
  },
  WhenClicked: { head: [label('when I am clicked')] },
  WhenCollision: {
    head: [label('when I touch'), { kind: 'dropdown', key: 'with', options: collisionOptions, placeholder: 'anything' }],
  },
  WhenMessage: {
    head: [label('when I get'), { kind: 'text', key: 'name', placeholder: 'message' }],
  },
  Broadcast: { head: [label('broadcast'), { kind: 'text', key: 'name', placeholder: 'message' }] },
  BlockHeader: { head: [] },

  // ── Motion ───────────────────────────────────────────────────────────────
  Move: { head: [label('move'), value('MoveSteps', 'steps'), label('steps')] },
  GoTo: { head: vector('go to', ['GoToX', 'GoToY', 'GoToZ'], ['x', 'y', 'z']) },
  ChangePosition: {
    head: [
      label('change'),
      { kind: 'dropdown', key: 'axis', options: positionAxes },
      label('by'),
      value('ChangeByAmount', 'by'),
    ],
  },
  Glide: {
    head: [
      label('glide'),
      value('GlideSeconds', 'seconds'),
      label('secs to x:'),
      value('GlideX', 'x'),
      label('y:'),
      value('GlideY', 'y'),
      label('z:', { mode: 'ThreeD' }),
      value('GlideZ', 'z', { mode: 'ThreeD' }),
    ],
  },
  Turn: {
    head: [
      label('turn'),
      { kind: 'dropdown', key: 'axis', options: rotationAxes, mode: 'ThreeD' },
      label('by'),
      value('TurnDegrees', 'degrees'),
      label('degrees'),
    ],
  },
  SetRotation: {
    head: [
      label('point'),
      { kind: 'dropdown', key: 'axis', options: rotationAxes, mode: 'ThreeD' },
      label('in direction'),
      value('RotationDegrees', 'degrees'),
    ],
  },
  PointTowards: {
    head: [label('point towards'), { kind: 'dropdown', key: 'target', options: () => actorOptions(true) }],
  },
  SetScale: { head: [label('set size to'), value('ScaleFactor', 'factor')] },

  // ── Physics ──────────────────────────────────────────────────────────────
  SetBody: { head: [label('set body to'), { kind: 'dropdown', key: 'body', options: BODY_OPTIONS }] },
  ApplyImpulse: { head: vector('push', ['ImpulseX', 'ImpulseY', 'ImpulseZ'], ['x', 'y', 'z']) },
  SetVelocity: { head: vector('set velocity', ['VelocityX', 'VelocityY', 'VelocityZ'], ['x', 'y', 'z']) },
  SetGravity: { head: vector('set world gravity', ['GravityX', 'GravityY', 'GravityZ'], ['x', 'y', 'z']) },

  // ── Looks ────────────────────────────────────────────────────────────────
  Say: { head: [label('say'), value('SayText', 'text')] },
  SetVisible: {
    head: [
      {
        kind: 'dropdown',
        key: 'visible',
        options: VISIBLE_OPTIONS,
        encode: chosen => chosen === 'true',
        decode: stored => (stored === false ? 'false' : 'true'),
      },
      label('myself'),
    ],
  },
  SetColor: { head: [label('set color to'), value('ColorText', 'color')] },

  // ── Control ──────────────────────────────────────────────────────────────
  Wait: { head: [label('wait'), value('WaitDuration', 'duration'), label('seconds')] },
  WaitUntil: { head: [label('wait until'), value('WaitUntilCondition', 'condition', { bool: true })] },
  If: {
    head: [label('if'), value('Condition', 'condition', { bool: true }), label('then')],
    body: [mouth(0, 'body')],
  },
  IfElse: {
    head: [label('if'), value('Condition', 'condition', { bool: true }), label('then')],
    body: [
      mouth(0, 'then_body'),
      { kind: 'midBar', text: 'else', notchFrom: 'then_body' },
      mouth(1, 'else_body'),
    ],
  },
  Repeat: { head: [label('repeat'), value('RepeatCount', 'count')], body: [mouth(0, 'body')] },
  Forever: { head: [label('forever')], body: [mouth(0, 'body')] },
  While: {
    head: [label('while'), value('Condition', 'condition', { bool: true })],
    body: [mouth(0, 'body')],
  },
  EscapeLoop: { head: [label('break out of the loop')] },
  ContinueLoop: { head: [label('next loop iteration')] },
  StopAll: { head: [label('stop everything')] },

  // ── Variables ────────────────────────────────────────────────────────────
  SetVariable: {
    head: [
      label('set'),
      { kind: 'dropdown', key: 'name', options: variableOptions, placeholder: 'variable' },
      label('to'),
      value('SetVariableValue', 'value'),
    ],
  },
  ChangeVariable: {
    head: [
      label('change'),
      { kind: 'dropdown', key: 'name', options: variableOptions, placeholder: 'variable' },
      label('by'),
      value('ChangeVariableValue', 'value'),
    ],
  },

  // ── Custom blocks ────────────────────────────────────────────────────────
  // Their rows come from a `BlockDef`, not from a fixed list of pieces - see
  // `components/fields/CallBlockFields.vue` and `BlockHeaderFields.vue`.
  CallBlock: { head: [] },
  Return: { head: [label('return'), value('ReturnValue', 'value')] },
};

function resolve(options: Options): Option[] {
  return typeof options === 'function' ? options() : options;
}

/** A piece marked `mode` only shows in that dimension. */
function shows(piece: Piece): boolean {
  return !('mode' in piece) || piece.mode === undefined || piece.mode === mode.value;
}

function labelNode(text: string): VNode {
  return h('span', { class: 'instruction-label' }, text);
}

/** The blank hexagon a boolean slot shows in the sidebar, where nothing is
 * draggable yet. */
function blankHexagon(): VNode {
  return h('span', { class: 'value-block value-hex-blank' }, [
    h('span', { class: 'value-op value-hex-blank-spacer' }, ' '),
  ]);
}

/** The canvas component for one block type: real value slots, real dropdowns,
 * real nested lists, every edit going straight to the backend. */
export function defineBlockFields(spec: BlockSpec): Component {
  return defineComponent({
    name: 'BlockFields',
    props: {
      strandId: { type: String, required: true },
      path: { type: Array as PropType<InstrPath>, required: true },
      instruction: { type: Object as PropType<InstructionDto>, required: true },
      part: { type: String as PropType<'head' | 'body'>, default: undefined },
    },
    setup(props) {
      const write = (key: string, next: unknown) =>
        void editInstruction(props.strandId, props.path, { ...props.instruction, [key]: next }).catch(
          (e: unknown) => console.error(e),
        );

      const render = (piece: Piece): VNode | null => {
        switch (piece.kind) {
          case 'label':
            return labelNode(piece.text);
          case 'value':
            return h(ValueBlock, {
              location: fieldLocation(props.strandId, props.path, piece.field),
              value: asValue(props.instruction[piece.key]),
            });
          case 'dropdown': {
            const stored = props.instruction[piece.key];
            return h(AppDropdown, {
              options: resolve(piece.options),
              modelValue: piece.decode ? piece.decode(stored) : asString(stored),
              placeholder: piece.placeholder,
              className: 'dd-compact',
              'onUpdate:modelValue': (chosen: string) =>
                write(piece.key, piece.encode ? piece.encode(chosen) : chosen),
            });
          }
          case 'text':
            return h(AutosizeInput, {
              modelValue: asString(props.instruction[piece.key]),
              placeholder: piece.placeholder,
              minChars: 6,
              'onUpdate:modelValue': (next: string) => write(piece.key, next),
            });
          case 'mouth':
            return h('div', { class: 'wrap-mouth' }, [
              h(InstructionList, {
                strandId: props.strandId,
                basePath: bodyBasePath(props.path, piece.slot),
                instructions: asBody(props.instruction[piece.key]),
              }),
            ]);
          case 'midBar': {
            // A cap row against the bar has no bump for its notch to receive,
            // which would leave an unfilled hole - see blockstitch's
            // `.wrap-bar-flat-notch`.
            const rows = asBody(props.instruction[piece.notchFrom]);
            const last = rows[rows.length - 1];
            const flat = !!last && isCapType(last.type, last);
            return h('div', { class: ['wrap-mid-bar', { 'wrap-bar-flat-notch': flat }] }, [
              labelNode(piece.text),
            ]);
          }
        }
      };

      return () => {
        const pieces = props.part === 'body' ? (spec.body ?? []) : spec.head;
        return pieces.filter(shows).map(render);
      };
    },
  });
}

/** The sidebar-prefab component: the same row, editable in place but wired to
 * nothing - a palette entry has no strand behind it, and the sidebar refuses
 * value drops, so slots are plain leaves and mouths are empty. */
export function definePaletteBlockFields(spec: BlockSpec): Component {
  return defineComponent({
    name: 'PaletteBlockFields',
    props: {
      instruction: { type: Object as PropType<InstructionDto>, required: true },
      part: { type: String as PropType<'head' | 'body'>, default: undefined },
    },
    setup(props) {
      const render = (piece: Piece): VNode | null => {
        switch (piece.kind) {
          case 'label':
            return labelNode(piece.text);
          case 'value':
            return piece.bool
              ? blankHexagon()
              : h(PaletteNumberField, {
                  modelValue: asValue(props.instruction[piece.key]),
                  // The prefab object is the host's own reactive state, so
                  // editing it here is the whole of the write.
                  'onUpdate:modelValue': (next: unknown) => {
                    props.instruction[piece.key] = next;
                  },
                });
          case 'dropdown': {
            const stored = props.instruction[piece.key];
            return h(AppDropdown, {
              options: resolve(piece.options),
              modelValue: piece.decode ? piece.decode(stored) : asString(stored),
              placeholder: piece.placeholder,
              className: 'dd-compact',
              'onUpdate:modelValue': (chosen: string) => {
                props.instruction[piece.key] = piece.encode ? piece.encode(chosen) : chosen;
              },
            });
          }
          case 'text':
            return h(AutosizeInput, {
              modelValue: asString(props.instruction[piece.key]),
              placeholder: piece.placeholder,
              minChars: 6,
              'onUpdate:modelValue': (next: string) => {
                props.instruction[piece.key] = next;
              },
            });
          case 'mouth':
            return h('div', { class: 'wrap-mouth' });
          case 'midBar':
            return h('div', { class: 'wrap-mid-bar' }, [labelNode(piece.text)]);
        }
      };

      return () => {
        const pieces = props.part === 'body' ? (spec.body ?? []) : spec.head;
        return pieces.filter(shows).map(render);
      };
    },
  });
}

/** Which types are C-blocks, and which nested list each of their slots holds -
 * derived from the same specs, so a shape can't drift from its row. */
export function bodyKeysFor(type: InstructionType): string[] {
  return (BLOCK_SPECS[type].body ?? [])
    .filter((piece): piece is Extract<Piece, { kind: 'mouth' }> => piece.kind === 'mouth')
    .sort((a, b) => a.slot - b.slot)
    .map(piece => piece.key);
}
