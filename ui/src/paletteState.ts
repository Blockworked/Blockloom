// The sidebar's prefabs: one live instruction per block type and one live value
// per palette entry, editable in place and cloned onto the canvas when dragged
// out. Editing a prefab never touches the backend - there's no strand behind a
// palette entry - which is why this state lives here rather than in the store.
import { reactive } from 'vue';
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
    case 'Say':
      return { text: textValue('Hello!') };
    case 'SetVisible':
      return { visible: true };
    case 'SetColor':
      return { color: textValue('#FFAB19') };
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
