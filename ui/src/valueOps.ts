// Blockloom's operator table: blockstitch's built-in arithmetic/text/logic
// operators plus the sensing reporters `blockloom-core`'s `value` module
// registers on the Rust side, plus blockstitch's list reporters. The lists
// must agree on `op` (the wire name) and on argument count; everything else
// here is presentation.
import type { OperatorKindSpec } from 'blockstitch';
import {
  LIST_EMPTY_OPTIONS,
  LIST_NAME_OPTIONS,
  listReporterSpecs,
} from 'blockstitch';
import { KEY_OPTIONS } from './constants';

export { LIST_EMPTY_OPTIONS, LIST_NAME_OPTIONS };
export { setListNameOptions } from 'blockstitch';

const AXIS_OPTIONS = [
  { value: 'X', label: 'x' },
  { value: 'Y', label: 'y' },
  { value: 'Z', label: 'z' },
];

const MATH_OPTIONS = [
  { value: 'Abs', label: 'abs' },
  { value: 'Floor', label: 'floor' },
  { value: 'Ceiling', label: 'ceiling' },
  { value: 'Sign', label: 'sign' },
  { value: 'Sqrt', label: 'sqrt' },
  { value: 'Sin', label: 'sin' },
  { value: 'Cos', label: 'cos' },
  { value: 'Tan', label: 'tan' },
  { value: 'Asin', label: 'asin' },
  { value: 'Acos', label: 'acos' },
  { value: 'Atan', label: 'atan' },
  { value: 'Ln', label: 'ln' },
  { value: 'Log', label: 'log' },
  { value: 'Log2', label: 'log2' },
  { value: 'EPower', label: 'e ^' },
  { value: 'TenPower', label: '10 ^' },
];

const CASE_OPTIONS = [
  { value: 'Upper', label: 'uppercase' },
  { value: 'Lower', label: 'lowercase' },
];

const CURRENT_TIME_OPTIONS = [
  { value: 'Year', label: 'year' },
  { value: 'Month', label: 'month' },
  { value: 'Date', label: 'date' },
  { value: 'DayOfWeek', label: 'day of week' },
  { value: 'Hour', label: 'hour' },
  { value: 'Minute', label: 'minute' },
  { value: 'Second', label: 'second' },
];

/** Operators grouped the way the sidebar shows them. */
export const OPERATOR_GROUPS: { label: string; kinds: string[] }[] = [
  {
    label: 'Sensing',
    kinds: [
      'KeyDown',
      'MouseDown',
      'MouseX',
      'MouseY',
      'MouseDeltaX',
      'MouseDeltaY',
      'MouseLocked',
      'Timer',
      'MyPosition',
      'MyRotation',
      'Touching',
      'DistanceTo',
      'ActorPosition',
      'ComponentField',
    ],
  },
  {
    label: 'Interface',
    kinds: ['UiValue', 'UiText', 'UiShown', 'UiExists', 'UiFocus', 'GamePaused'],
  },
  { label: 'Actors', kinds: ['IsClone', 'MyParent', 'NewActor', 'ActorCount'] },
  { label: 'Maths', kinds: ['Add', 'Sub', 'Mul', 'Div', 'Mod', 'Round', 'Math', 'Random'] },
  { label: 'Comparing', kinds: ['Eq', 'Neq', 'Gt', 'Lt', 'Gte', 'Lte', 'And', 'Or', 'Not', 'True', 'False'] },
  {
    label: 'Text',
    kinds: ['Join', 'Join3', 'Length', 'LetterOf', 'IndexOf', 'LastIndexOf', 'Case', 'NewLine', 'Tab', 'CurrentTime'],
  },
  {
    label: 'Lists',
    kinds: ['ListItem', 'ListItemNumber', 'ListAmount', 'ListLength', 'ListContains', 'ListItemExists', 'ListIsEmpty'],
  },
];

export const OPERATOR_KINDS: OperatorKindSpec[] = [
  // ── Sensing: Blockloom's own, registered in blockloom-core/src/value.rs ──
  {
    kind: 'KeyDown',
    op: 'KeyDown',
    arity: 1,
    argTypes: ['text'],
    resultType: 'bool',
    prefix: 'key',
    suffix: 'down?',
    enumArg: { index: 0, options: KEY_OPTIONS },
  },
  { kind: 'MouseDown', op: 'MouseDown', arity: 0, argTypes: [], resultType: 'bool', prefix: 'mouse down?' },
  { kind: 'MouseX', op: 'MouseX', arity: 0, argTypes: [], resultType: 'number', prefix: 'mouse x' },
  { kind: 'MouseY', op: 'MouseY', arity: 0, argTypes: [], resultType: 'number', prefix: 'mouse y' },
  { kind: 'MouseDeltaX', op: 'MouseDeltaX', arity: 0, argTypes: [], resultType: 'number', prefix: 'mouse delta x' },
  { kind: 'MouseDeltaY', op: 'MouseDeltaY', arity: 0, argTypes: [], resultType: 'number', prefix: 'mouse delta y' },
  { kind: 'MouseLocked', op: 'MouseLocked', arity: 0, argTypes: [], resultType: 'bool', prefix: 'mouse locked?' },
  { kind: 'Timer', op: 'Timer', arity: 0, argTypes: [], resultType: 'number', prefix: 'timer' },
  {
    kind: 'UiValue',
    op: 'UiValue',
    arity: 1,
    argTypes: ['text'],
    resultType: 'number',
    prefix: 'value of',
  },
  {
    kind: 'UiText',
    op: 'UiText',
    arity: 1,
    argTypes: ['text'],
    resultType: 'text',
    prefix: 'text of',
  },
  {
    kind: 'UiShown',
    op: 'UiShown',
    arity: 1,
    argTypes: ['text'],
    resultType: 'bool',
    prefix: 'is',
    suffix: 'shown?',
  },
  {
    kind: 'UiExists',
    op: 'UiExists',
    arity: 1,
    argTypes: ['text'],
    resultType: 'bool',
    prefix: 'does',
    suffix: 'exist?',
  },
  { kind: 'UiFocus', op: 'UiFocus', arity: 0, argTypes: [], resultType: 'text', prefix: 'focused element' },
  { kind: 'GamePaused', op: 'GamePaused', arity: 0, argTypes: [], resultType: 'bool', prefix: 'game paused?' },
  {
    kind: 'MyPosition',
    op: 'MyPosition',
    arity: 1,
    argTypes: ['text'],
    resultType: 'number',
    prefix: 'my',
    suffix: 'position',
    enumArg: { index: 0, options: AXIS_OPTIONS },
  },
  {
    kind: 'MyRotation',
    op: 'MyRotation',
    arity: 1,
    argTypes: ['text'],
    resultType: 'number',
    prefix: 'my',
    suffix: 'rotation',
    enumArg: { index: 0, options: AXIS_OPTIONS },
  },
  { kind: 'Touching', op: 'Touching', arity: 1, argTypes: ['text'], resultType: 'bool', prefix: 'touching' },
  {
    kind: 'DistanceTo',
    op: 'DistanceTo',
    arity: 1,
    argTypes: ['text'],
    resultType: 'number',
    prefix: 'distance to',
  },
  {
    kind: 'ComponentField',
    op: 'ComponentField',
    arity: 2,
    argTypes: ['text', 'text'],
    resultType: 'number',
    prefix: 'my',
    infix: 'field',
  },
  {
    kind: 'IsClone',
    op: 'IsClone',
    arity: 0,
    argTypes: [],
    resultType: 'bool',
    prefix: 'am I a clone?',
  },
  {
    kind: 'MyParent',
    op: 'MyParent',
    arity: 0,
    argTypes: [],
    resultType: 'text',
    prefix: 'the actor I hang off',
  },
  {
    kind: 'NewActor',
    op: 'NewActor',
    arity: 0,
    argTypes: [],
    resultType: 'text',
    prefix: 'the actor I made',
  },
  {
    kind: 'ActorCount',
    op: 'ActorCount',
    arity: 1,
    argTypes: ['text'],
    resultType: 'number',
    prefix: 'how many',
    suffix: 'there are',
  },
  {
    kind: 'ActorPosition',
    op: 'ActorPosition',
    arity: 2,
    argTypes: ['text', 'text'],
    resultType: 'number',
    infix: "'s",
    suffix: 'position',
    enumArg: { index: 1, options: AXIS_OPTIONS },
  },

  // ── blockstitch's built-ins ─────────────────────────────────────────────
  { kind: 'Add', op: 'Add', arity: 2, argTypes: ['number', 'number'], resultType: 'number', infix: '+' },
  { kind: 'Sub', op: 'Sub', arity: 2, argTypes: ['number', 'number'], resultType: 'number', infix: '−' },
  { kind: 'Mul', op: 'Mul', arity: 2, argTypes: ['number', 'number'], resultType: 'number', infix: '×' },
  { kind: 'Div', op: 'Div', arity: 2, argTypes: ['number', 'number'], resultType: 'number', infix: '/' },
  { kind: 'Mod', op: 'Mod', arity: 2, argTypes: ['number', 'number'], resultType: 'number', infix: 'mod' },
  { kind: 'Round', op: 'Round', arity: 1, argTypes: ['number'], resultType: 'number', prefix: 'round' },
  {
    kind: 'Math',
    op: 'Math',
    arity: 2,
    argTypes: ['text', 'number'],
    resultType: 'number',
    infix: 'of',
    enumArg: { index: 0, options: MATH_OPTIONS },
  },
  {
    kind: 'Random',
    op: 'Random',
    arity: 2,
    argTypes: ['number', 'number'],
    resultType: 'number',
    prefix: 'pick random',
    infix: 'to',
  },
  { kind: 'Join', op: 'Join', arity: 2, argTypes: ['text', 'text'], resultType: 'text', prefix: 'join' },
  { kind: 'Join3', op: 'Join', arity: 3, argTypes: ['text', 'text', 'text'], resultType: 'text', prefix: 'join' },
  { kind: 'NewLine', op: 'NewLine', arity: 0, argTypes: [], resultType: 'text', prefix: 'new line' },
  { kind: 'Tab', op: 'Tab', arity: 0, argTypes: [], resultType: 'text', prefix: 'tab' },
  {
    kind: 'IndexOf',
    op: 'IndexOf',
    arity: 2,
    argTypes: ['text', 'text'],
    resultType: 'number',
    prefix: 'index of',
    infix: 'in',
  },
  {
    kind: 'LastIndexOf',
    op: 'LastIndexOf',
    arity: 2,
    argTypes: ['text', 'text'],
    resultType: 'number',
    prefix: 'last index of',
    infix: 'in',
  },
  {
    kind: 'LetterOf',
    op: 'LetterOf',
    arity: 2,
    argTypes: ['number', 'text'],
    resultType: 'text',
    prefix: 'letter',
    infix: 'of',
  },
  { kind: 'Length', op: 'Length', arity: 1, argTypes: ['text'], resultType: 'number', prefix: 'length of' },
  {
    kind: 'Case',
    op: 'Case',
    arity: 2,
    argTypes: ['text', 'text'],
    resultType: 'text',
    infix: 'to',
    enumArg: { index: 1, options: CASE_OPTIONS },
  },
  { kind: 'Eq', op: 'Eq', arity: 2, argTypes: ['number', 'number'], resultType: 'bool', infix: '=' },
  { kind: 'Neq', op: 'Neq', arity: 2, argTypes: ['number', 'number'], resultType: 'bool', infix: '≠' },
  { kind: 'Gt', op: 'Gt', arity: 2, argTypes: ['number', 'number'], resultType: 'bool', infix: '>' },
  { kind: 'Lt', op: 'Lt', arity: 2, argTypes: ['number', 'number'], resultType: 'bool', infix: '<' },
  { kind: 'Gte', op: 'Gte', arity: 2, argTypes: ['number', 'number'], resultType: 'bool', infix: '≥' },
  { kind: 'Lte', op: 'Lte', arity: 2, argTypes: ['number', 'number'], resultType: 'bool', infix: '≤' },
  { kind: 'And', op: 'And', arity: 2, argTypes: ['bool', 'bool'], resultType: 'bool', infix: 'and' },
  { kind: 'Or', op: 'Or', arity: 2, argTypes: ['bool', 'bool'], resultType: 'bool', infix: 'or' },
  { kind: 'Not', op: 'Not', arity: 1, argTypes: ['bool'], resultType: 'bool', prefix: 'not' },
  { kind: 'True', op: 'True', arity: 0, argTypes: [], resultType: 'bool', prefix: 'true' },
  { kind: 'False', op: 'False', arity: 0, argTypes: [], resultType: 'bool', prefix: 'false' },
  {
    kind: 'CurrentTime',
    op: 'CurrentTime',
    arity: 1,
    argTypes: ['text'],
    resultType: 'number',
    prefix: 'current',
    enumArg: { index: 0, options: CURRENT_TIME_OPTIONS },
  },
  // List reporters live in blockstitch (`listReporterSpecs`) so every project
  // reuses the same blocks, dropdowns, and name-arg positions.
  ...(listReporterSpecs(LIST_NAME_OPTIONS, LIST_EMPTY_OPTIONS) as OperatorKindSpec[]),
];

export function specForKind(kind: string): OperatorKindSpec | undefined {
  return OPERATOR_KINDS.find(spec => spec.kind === kind);
}
