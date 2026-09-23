// The sidebar's prefabs: one live instruction per block type and one live value
// per palette entry, editable in place and cloned onto the canvas when dragged
// out. Editing a prefab never touches the backend - there's no strand behind a
// palette entry - which is why this state lives here rather than in the store.
import { reactive } from 'vue';
import { isOneBasedListIndexArg } from 'blockstitch';
import { INSTRUCTION_TYPES, newId, numberValue, parseParamKind, textValue } from './types';
import type { InstructionDto, InstructionType, ValueDto, ValueKind } from './types';
import { OPERATOR_KINDS, specForKind } from './valueOps';

const blankBool = (): ValueDto => ({ kind: 'Bool' });

/** A fresh block of each type, with the defaults that make it useful straight
 * away (`move 10 steps`, not `move 0 steps`). */
function defaults(type: InstructionType): Record<string, unknown> {
  switch (type) {
    case 'WhenKeyPressed':
      return { key: 'space' };
    case 'WhenCollision':
      return { with: '' };
    case 'WhenMessage':
    case 'Broadcast':
      return { name: 'message1' };
    case 'BlockHeader':
      return { block_id: '' };
    case 'Move':
      return { steps: numberValue(10) };
    case 'GoTo':
      return { x: numberValue(0), y: numberValue(0), z: numberValue(0) };
    case 'NavigateTo':
      return { x: numberValue(0), y: numberValue(0), z: numberValue(0), speed: numberValue(4) };
    case 'ChangePosition':
      return { axis: 'X', by: numberValue(10) };
    case 'Glide':
      return { seconds: numberValue(1), x: numberValue(0), y: numberValue(0), z: numberValue(0) };
    case 'Turn':
      return { axis: 'Z', degrees: numberValue(15) };
    case 'SetRotation':
      return { axis: 'Z', degrees: numberValue(0) };
    case 'PointTowards':
      return { target: 'mouse' };
    case 'SetScale':
      return { factor: numberValue(1) };
    case 'SetBody':
      return { body: 'Dynamic' };
    case 'ApplyImpulse':
      return { x: numberValue(0), y: numberValue(5), z: numberValue(0) };
    case 'SetVelocity':
      return { x: numberValue(0), y: numberValue(0), z: numberValue(0) };
    case 'SetGravity':
      return { x: numberValue(0), y: numberValue(-9.81), z: numberValue(0) };
    case 'SetDensity':
      return { density: numberValue(1) };
    case 'SetMass':
      return { mass: numberValue(1) };
    case 'SetTrigger':
      return { trigger: false };
    case 'SetCollisionLayer':
      return { layer: numberValue(1) };
    case 'SetCollisionMask':
      return { mask: numberValue(255) };
    case 'Say':
      return { text: textValue('Hello!') };
    case 'SetVisible':
      return { visible: true };
    case 'SetColor':
      return { color: textValue('#FFAB19') };
    case 'PlaySound':
      return {
        sound: textValue('assets/sounds/sound.wav'),
        volume: numberValue(100),
        pitch: numberValue(1),
        loop: false,
        bus: 'Sfx',
      };
    case 'PlaySoundAt':
      return {
        sound: textValue('assets/sounds/sound.wav'),
        volume: numberValue(100),
        pitch: numberValue(1),
        loop: false,
        bus: 'Sfx',
        target: textValue(''),
      };
    case 'StopSound':
      return { sound: textValue('') };
    case 'SetSoundVolume':
      return { sound: textValue(''), volume: numberValue(100) };
    case 'SetSoundPitch':
      return { sound: textValue(''), pitch: numberValue(1) };
    case 'SetBusVolume':
      return { bus: 'Sfx', volume: numberValue(100) };
    case 'SetComponentField':
      return { component: '', field: '', value: numberValue(0) };
    case 'SetCameraView':
      return { view: 'ThirdPerson' };
    case 'SetCameraPitch':
      return { degrees: numberValue(0) };
    case 'SetCameraFov':
      return { fov: numberValue(75) };
    case 'AttachComponent':
    case 'DetachComponent':
      return { component: '' };
    case 'SetParent':
      return { parent: textValue('') };
    case 'CreateClone':
      return { of: '' };
    case 'CreateActor':
      return { name: textValue('Actor'), x: numberValue(0), y: numberValue(0), z: numberValue(0) };
    case 'DeleteActor':
      return { target: textValue('myself') };
    case 'Wait':
      return { duration: numberValue(1) };
    case 'WaitUntil':
      return { condition: blankBool() };
    case 'If':
      return { condition: blankBool(), body: [] };
    case 'IfElse':
      return { condition: blankBool(), then_body: [], else_body: [] };
    case 'Repeat':
      return { count: numberValue(10), body: [] };
    case 'Forever':
      return { body: [] };
    case 'While':
      return { condition: blankBool(), body: [] };
    case 'SetVariable':
      return { name: '', value: numberValue(0) };
    case 'ChangeVariable':
      return { name: '', value: numberValue(1) };
    case 'CallBlock':
      return { block_id: '', args: [] };
    case 'Return':
      return { value: numberValue(0) };
    case 'SetMouseLocked':
      return { locked: true };
    case 'WhenUiClicked':
    case 'WhenUiChanged':
      return { element: 'my-button' };
    // Every `show` block wears the same placement tail, so they share one
    // set of defaults for it and add only their own caption on top.
    case 'ShowPanel':
      return { ...uiPlacement('Center'), element: textValue('menu'), title: textValue('Menu'), modal: true };
    case 'ShowLabel':
      return { ...uiPlacement('TopLeft'), element: textValue('score'), text: textValue('Score: 0') };
    case 'ShowButton':
      return { ...uiPlacement('Center'), element: textValue('resume'), label: textValue('Resume') };
    case 'ShowImage':
      return { ...uiPlacement('TopLeft'), element: textValue('logo'), asset: textValue('') };
    case 'ShowInput':
      return { ...uiPlacement('Center'), element: textValue('name'), placeholder: textValue('your name') };
    case 'ShowSlider':
      return {
        ...uiPlacement('Center'),
        element: textValue('volume'),
        min: numberValue(0),
        max: numberValue(100),
        value: numberValue(50),
      };
    case 'ShowToggle':
      return { ...uiPlacement('Center'), element: textValue('shadows'), label: textValue('Shadows'), on: true };
    case 'ShowList':
      return { ...uiPlacement('Center'), element: textValue('items'), width: numberValue(280), height: numberValue(240) };
    case 'SetUiTheme':
      return { theme: 'Dark' };
    case 'SetUiProp':
      return { prop: 'Text', element: textValue('score'), value: textValue('') };
    case 'SaveVariable':
    case 'ClearSavedVariable':
      return { name: '' };
    case 'AddToList':
      return { name: '', value: numberValue(0) };
    case 'DeleteOfList':
      return { name: '', index: numberValue(1) };
    case 'DeleteAllOfList':
      return { name: '' };
    case 'ShiftList':
      return { name: '', amount: numberValue(1) };
    case 'InsertIntoList':
      return { name: '', value: numberValue(0), index: numberValue(1) };
    case 'ReplaceItemOfList':
      return { name: '', index: numberValue(1), value: numberValue(0) };
    case 'ReverseList':
      return { name: '' };
    case 'LoadJsonIntoList':
      return { name: '', json: textValue('') };
    case 'SetDictValue':
      return { name: '', key: textValue(''), value: numberValue(0) };
    case 'DeleteDictKey':
      return { name: '', key: textValue('') };
    case 'DeleteAllOfDict':
      return { name: '' };
    case 'LoadJsonIntoDict':
      return { name: '', json: textValue('') };
    case 'HideElement':
    case 'DeleteElement':
      return { element: textValue('menu') };
    case 'FocusElement':
      return { element: textValue('name') };
    // WhenStarted, WhenClicked, WhenCloned, EscapeLoop, ContinueLoop,
    // StopAll: no fields.
    default:
      return {};
  }
}

/** The placement tail every `show` block starts with: hung off `anchor`,
 * no offset, sized to its content, and at screen level. */
function uiPlacement(anchor: string): Record<string, unknown> {
  return {
    anchor,
    x: numberValue(0),
    y: numberValue(0),
    width: numberValue(0),
    height: numberValue(0),
    parent: textValue(''),
  };
}

export function defaultInstruction(type: InstructionType): InstructionDto {
  return { id: newId(), type, ...defaults(type) };
}

const LIST_COMMAND_TYPES: InstructionType[] = [
  'AddToList',
  'DeleteOfList',
  'DeleteAllOfList',
  'ShiftList',
  'InsertIntoList',
  'ReplaceItemOfList',
  'ReverseList',
  'LoadJsonIntoList',
];

const DICT_COMMAND_TYPES: InstructionType[] = [
  'SetDictValue',
  'DeleteDictKey',
  'DeleteAllOfDict',
  'LoadJsonIntoDict',
];

const LIST_OPERATOR_KINDS = [
  'ListItem',
  'ListItemNumber',
  'ListAmount',
  'ListLength',
  'ListContains',
  'ListItemExists',
  'ListIsEmpty',
  'ListAsJson',
];

const DICT_OPERATOR_KINDS = [
  'DictValue',
  'DictHasKey',
  'DictSize',
  'DictKeys',
  'DictAsJson',
  'DictIsEmpty',
];

/** Keeps sidebar prefabs useful as soon as names arrive from the backend.
// Only unselected or no-longer-valid palette targets are changed; blocks
// already dropped onto the canvas are persisted separately and untouched. */
export function syncPaletteListDefaults(names: string[]): void {
  syncPaletteNameDefaults(names, LIST_COMMAND_TYPES, LIST_OPERATOR_KINDS);
}

/** Dict-name counterpart to [`syncPaletteListDefaults`]. */
export function syncPaletteDictDefaults(names: string[]): void {
  syncPaletteNameDefaults(names, DICT_COMMAND_TYPES, DICT_OPERATOR_KINDS);
}

function syncPaletteNameDefaults(
  names: string[],
  commandTypes: InstructionType[],
  operatorKinds: string[],
): void {
  const first = names[0];
  if (!first) return;
  for (const type of commandTypes) {
    const instruction = paletteInstructions[type];
    if (typeof instruction.name !== 'string' || !names.includes(instruction.name)) {
      instruction.name = first;
    }
  }
  for (const kind of operatorKinds) {
    const spec = specForKind(kind);
    const index = spec?.enumArg?.index;
    if (index === undefined) continue;
    const held = paletteValues[kind];
    if (held?.kind !== 'Op') continue;
    const current = held.args[index];
    if (current?.kind !== 'Text' || !names.includes(current.value)) {
      held.args[index] = textValue(first);
    }
  }
}

export const paletteInstructions: Record<InstructionType, InstructionDto> = reactive(
  Object.fromEntries(INSTRUCTION_TYPES.map(type => [type, defaultInstruction(type)])) as Record<
    InstructionType,
    InstructionDto
  >,
);

/** The instruction a palette drag drops onto the canvas: the prefab as it
 * currently reads, with fresh ids. */
export function clonePaletteInstruction(type: InstructionType): InstructionDto {
  return { ...cloneDto(paletteInstructions[type]), id: newId() };
}

// ─── Value prefabs ─────────────────────────────────────────────────────────

/** A fresh argument for one operator slot: its dropdown's first option, a blank
 * hexagon for a boolean, empty text, or zero. Mirrors the defaults
 * `blockstitch-core`'s operator table hands out on the Rust side. */
function defaultArg(kind: string, index: number): ValueDto {
  const spec = specForKind(kind);
  if (!spec) return numberValue(0);
  if (spec.enumArg?.index === index) return textValue(spec.enumArg.options[0].value);
  // List item slots are one-based, matching the command-block defaults and
  // the backend's operator construction.
  if (isOneBasedListIndexArg(kind, index)) return numberValue(1);
  if (spec.argTypes[index] === 'bool') return blankBool();
  return spec.argTypes[index] === 'text' ? textValue('') : numberValue(0);
}

function operatorValue(kind: string): ValueDto {
  const spec = specForKind(kind);
  if (!spec) return numberValue(0);
  return {
    kind: 'Op',
    op: spec.op,
    args: Array.from({ length: spec.arity }, (_, i) => defaultArg(kind, i)),
    saved: numberValue(0),
  };
}

/** Every operator prefab's live args, so a number typed into a sidebar block
 * is still there when it's dragged out. */
export const paletteValues: Record<string, ValueDto> = reactive({
  Number: numberValue(0),
  Text: textValue(''),
  ...Object.fromEntries(OPERATOR_KINDS.map(spec => [spec.kind, operatorValue(spec.kind)])),
});

/** The value a palette entry currently represents - what lands on the canvas. */
export function paletteValueFor(kind: ValueKind): ValueDto {
  if (kind.startsWith('Var:')) return { kind: 'Var', name: kind.slice('Var:'.length) };
  if (kind.startsWith('Param:')) return { kind: 'Param', name: parseParamKind(kind).name };
  const held = paletteValues[kind];
  return held ? cloneDto(held) : numberValue(0);
}

/** Palette prefabs are Vue proxies. JSON is also their wire format, so this
 * makes a plain deep copy without asking structuredClone to clone a proxy. */
function cloneDto<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

/** Writes an edit blockstitch's generic palette block made back onto the
 * prefab - the inverse of `paletteValueFor`. */
export function applyPaletteValueEdit(kind: ValueKind, next: ValueDto): void {
  if (paletteValues[kind]) paletteValues[kind] = next;
}
