// The shapes the backend sends and takes back, plus the small helpers the rest
// of the frontend builds on.
//
// An instruction arrives flat (`{ id, type, ...fields }`) - `blockloom-core`'s
// `wire` module folds blockstitch's `{ id, kind }` nesting away on the wire, so
// what lands here is already a blockstitch `BlockNode`. Fields are typed
// `unknown` rather than spelled out per block: `blockFields.ts` builds every
// block's row from one declarative table keyed by field name, so a precise
// per-block union would be read exactly nowhere.

import type { ValueLocation, ValueNode } from 'blockstitch';
import { fieldLocation as blockstitchFieldLocation, newId as blockstitchNewId } from 'blockstitch';

export type InstrPath = { index: number; slot?: number }[];

export const INSTRUCTION_TYPES = [
  'WhenStarted',
  'WhenKeyPressed',
  'WhenClicked',
  'WhenCollision',
  'WhenMessage',
  'BlockHeader',
  'Move',
  'GoTo',
  'ChangePosition',
  'Glide',
  'Turn',
  'SetRotation',
  'PointTowards',
  'SetScale',
  'SetBody',
  'ApplyImpulse',
  'SetVelocity',
  'SetGravity',
  'SetDensity',
  'SetMass',
  'Say',
  'SetVisible',
  'SetColor',
  'SetComponentField',
  'SetCameraView',
  'AttachComponent',
  'DetachComponent',
  'Wait',
  'WaitUntil',
  'If',
  'IfElse',
  'Repeat',
  'Forever',
  'While',
  'EscapeLoop',
  'ContinueLoop',
  'Broadcast',
  'StopAll',
  'SetVariable',
  'ChangeVariable',
  'CallBlock',
  'Return',
] as const;

export type InstructionType = (typeof INSTRUCTION_TYPES)[number];

export interface InstructionDto {
  id: string;
  type: InstructionType;
  [field: string]: unknown;
}

// ─── Values ────────────────────────────────────────────────────────────────

/** The value-expression tree, straight from blockstitch: a literal, a blank
 * boolean slot, an operator over nested args, a variable or parameter read, or
 * a call to a reporter-shaped custom block. Aliased rather than redeclared so
 * every value handed to a blockstitch component type-checks as-is. */
export type ValueDto = ValueNode;

/** A palette entry for a value block: a literal, an operator, a variable read
 * (`Var:<name>`), a block parameter (`Param:<name>`) or a reporter call
 * (`Call:<blockId>`). */
export type ValueKind = string;

export function numberValue(value: number): ValueDto {
  return { kind: 'Number', value };
}

export function textValue(value: string): ValueDto {
  return { kind: 'Text', value };
}

export function asValue(field: unknown): ValueDto {
  return (field as ValueDto | undefined) ?? numberValue(0);
}

export function asString(field: unknown): string {
  return typeof field === 'string' ? field : '';
}

export function asBody(field: unknown): InstructionDto[] {
  return Array.isArray(field) ? (field as InstructionDto[]) : [];
}

// ─── The scene ─────────────────────────────────────────────────────────────

export type Mode = 'TwoD' | 'ThreeD';
export type Axis = 'X' | 'Y' | 'Z';
export type BodyKind = 'None' | 'Static' | 'Dynamic' | 'Kinematic';

export type VisualDto =
  | { shape: 'Rect'; color: string; size: [number, number] }
  | { shape: 'Circle'; color: string; radius: number }
  | { shape: 'Image'; path: string; size: [number, number] }
  | { shape: 'Cuboid'; color: string; size: [number, number, number] }
  | { shape: 'Sphere'; color: string; radius: number }
  | { shape: 'Capsule'; color: string; radius: number; height: number }
  | { shape: 'Plane'; color: string; size: [number, number] };

export type VisualShape = VisualDto['shape'];

/** Which shapes belong to which dimension - the editor greys out the others
 * rather than hiding them, so switching modes never silently drops an actor. */
export const SHAPES_2D: VisualShape[] = ['Rect', 'Circle', 'Image'];
export const SHAPES_3D: VisualShape[] = ['Cuboid', 'Sphere', 'Capsule', 'Plane'];

export function shapesFor(mode: Mode): VisualShape[] {
  return mode === 'ThreeD' ? SHAPES_3D : SHAPES_2D;
}

export interface PlacementDto {
  position: [number, number, number];
  rotation: [number, number, number];
  scale: number;
}

export interface PhysicsDto {
  body: BodyKind;
  gravity_scale: number;
  lock_rotation: boolean;
  restitution: number;
  friction: number;
  /** Mass per unit of collider area/volume - how heavy the actor is for its size. */
  density: number;
  /** An explicit body mass; null lets the shape and `density` decide. */
  mass: number | null;
}

/** Where the camera stands when no actor is holding it. An actor's `Camera`
 * component takes it over from there. */
export interface CameraDto {
  position: [number, number, number];
  look_at: [number, number, number];
  zoom: number;
}

// ─── Components ────────────────────────────────────────────────────────────

export type CameraView = 'Follow' | 'FirstPerson' | 'ThirdPerson';

export interface CameraAttachDto {
  view: CameraView;
  offset: [number, number, number];
  distance: number;
  pitch: number;
}

/** An already-evaluated value, as a variable or a component field holds it. */
export type EvaluatedDto =
  | { kind: 'Number'; value: number }
  | { kind: 'Text'; value: string }
  | { kind: 'Bool'; value: boolean };

export interface ComponentFieldDto {
  name: string;
  value: EvaluatedDto;
}

/** One component on an actor. The built-in five are what the engine reads;
 * `Custom` is a bag of values the project invented, which blocks read and
 * write by name. */
export type ActorComponentDto =
  | { component: 'Place'; placement: PlacementDto }
  | { component: 'Look'; visual: VisualDto }
  | { component: 'Render'; visible: boolean }
  | { component: 'Body'; physics: PhysicsDto }
  | { component: 'Camera'; camera: CameraAttachDto }
  /** A Rust file under the project's assets/scripts, compiled on Play. */
  | { component: 'Script'; path: string }
  | { component: 'Custom'; name: string; fields: ComponentFieldDto[] };

export type ComponentName = ActorComponentDto['component'];

/** Components an actor can be given, in the order the "add" menu offers them.
 * `Place` isn't here: every actor has one and it can't be removed. */
export const ADDABLE_COMPONENTS: ComponentName[] = [
  'Look',
  'Render',
  'Body',
  'Camera',
  'Script',
  'Custom',
];

/** What the editor calls a component - matches `ActorComponent::name()`. */
export function componentName(component: ActorComponentDto): string {
  return component.component === 'Custom' ? component.name : component.component;
}

/** Saved presentation settings for runtime speech bubbles. `font_asset` names
 * a font in the project folder, the way the asset tray spells one; null uses
 * the runtime's built-in sans-serif font. */
export interface SpeechBubbleStyleDto {
  background: string;
  border: string;
  text: string;
  font_size: number;
  max_width: number;
  padding: [number, number];
  offset: [number, number];
  font_asset: string | null;
}

export interface WorldDto {
  mode: Mode;
  background: string;
  gravity: [number, number, number];
  /** How many simulation steps a second the world advances at, whatever the
   * display rate. The runtime interpolates between them. */
  fixed_rate: number;
  camera: CameraDto;
  speech_bubble: SpeechBubbleStyleDto;
}

export interface StrandDto {
  id: string;
  x: number;
  y: number;
  instructions: InstructionDto[];
}

export interface FloatingValueDto {
  id: string;
  x: number;
  y: number;
  value: ValueDto;
  origin_block_id: string | null;
}

export interface CommentDto {
  id: string;
  x: number;
  y: number;
  text: string;
  collapsed: boolean;
  attached_to: string | null;
}

export interface VariableDto {
  name: string;
  value: { kind: 'Number' | 'Text' | 'Bool'; value?: number | string | boolean };
}

export type BlockPieceDto =
  | { kind: 'Label'; id: string; text: string }
  | { kind: 'Input'; id: string; name: string; value_type: 'Any' | 'Bool' };

export type BlockShapeDto = 'Normal' | 'Ending' | 'ReturnsValue' | 'ReturnsBool';

export interface BlockDefDto {
  id: string;
  pieces: BlockPieceDto[];
  shape: BlockShapeDto;
  color: string;
}

export interface ActorDto {
  id: string;
  name: string;
  /** What the actor is made of - see `blockloom_core::components`. */
  components: ActorComponentDto[];
  // Flattened `BlockGraph` - one canvas per actor.
  strands: StrandDto[];
  floating_values: FloatingValueDto[];
  comments: CommentDto[];
  variables: VariableDto[];
  block_defs: BlockDefDto[];
}

export interface ProjectDto {
  id: string;
  name: string;
  world: WorldDto;
  actors: ActorDto[];
  globals: VariableDto[];
}

// ─── Assets ────────────────────────────────────────────────────────────────

/** What a file is, from its extension - what the tray draws and what an input
 * checks a dropped asset against. Matches `blockloom_core::assets::AssetKind`. */
export type AssetKind =
  | 'folder'
  | 'image'
  | 'audio'
  | 'font'
  | 'model'
  | 'script'
  | 'text'
  | 'other';

/** One row in the asset tray. `path` is relative to the project folder. */
export interface AssetEntry {
  name: string;
  path: string;
  kind: AssetKind;
  size: number;
  /** Unix seconds, 0 if the filesystem wouldn't say. */
  modified: number;
  /** The project document itself, which can't be renamed, moved or deleted. */
  protected: boolean;
}

// ─── Run state ─────────────────────────────────────────────────────────────

export interface ActorStatusDto {
  id: string;
  position: [number, number, number];
  rotation: [number, number, number];
  visible: boolean;
}

export interface StatusDto {
  running: boolean;
  paused: boolean;
  time: number;
  fps: number;
  actors: ActorStatusDto[];
  globals: VariableDto[];
}

export interface LogLineDto {
  kind: string;
  actor: string;
  text: string;
}

export interface InvalidFieldDto {
  location: ValueLocation;
  text: string;
}

/** One card on the Dashboard: a project folder Blockloom remembers. */
export interface ProjectEntryDto {
  path: string;
  name: string;
  mode: Mode;
  /** Unix seconds, 0 if it has never been opened. */
  opened_at: number;
}

/** One platform the Build dialog offers. */
export interface BuildTarget {
  /** The rustc target triple, which is also what `build_game` is given. */
  triple: string;
  label: string;
  /** Whether this is the machine Blockloom is running on. */
  host: boolean;
  /** Whether a build for it would get anywhere right now. */
  ready: boolean;
  /** What it would build with, or what is missing. */
  note: string;
  /** Whether this project's blocks can be compiled for this platform. */
  fast_ready: boolean;
  /** Why native blocks are or are not available. */
  fast_note: string;
}

export interface StateDto {
  library: ProjectEntryDto[];
  /** Where the New Project dialog points unless the user picks elsewhere. */
  default_project_location: string;
  /** The open project's folder. */
  project_path: string | null;
  project: ProjectDto | null;
  selected_actor: string | null;
  can_undo: boolean;
  can_redo: boolean;
  invalid_field_buffers: InvalidFieldDto[];
  running: boolean;
  paused: boolean;
  status: StatusDto | null;
  log: LogLineDto[];
  runtime_available: boolean;
  runtime_open: boolean;
}

export function emptyState(): StateDto {
  return {
    library: [],
    default_project_location: '',
    project_path: null,
    project: null,
    selected_actor: null,
    can_undo: false,
    can_redo: false,
    invalid_field_buffers: [],
    running: false,
    paused: false,
    status: null,
    log: [],
    runtime_available: true,
    runtime_open: false,
  };
}

// ─── Lookups ───────────────────────────────────────────────────────────────

export const newId = blockstitchNewId;
export const fieldLocation = blockstitchFieldLocation;
export type { ValueLocation };

export function findActor(project: ProjectDto | null, actorId: string | null): ActorDto | null {
  if (!project || !actorId) return null;
  return project.actors.find(actor => actor.id === actorId) ?? null;
}

export function findComponent(actor: ActorDto | null, name: string): ActorComponentDto | null {
  return actor?.components.find(component => componentName(component) === name) ?? null;
}

/** What an actor looks like, or null when it has no `Look` - a positioned
 * actor its blocks can still drive, with nothing to draw. */
export function actorVisual(actor: ActorDto | null): VisualDto | null {
  const look = findComponent(actor, 'Look');
  return look?.component === 'Look' ? look.visual : null;
}

export function actorPlacement(actor: ActorDto | null): PlacementDto {
  const place = findComponent(actor, 'Place');
  if (place?.component === 'Place') return place.placement;
  return { position: [0, 0, 0], rotation: [0, 0, 0], scale: 1 };
}

/** The actor's physics, or the inert default when it has no `Body`. */
export function actorPhysics(actor: ActorDto | null): PhysicsDto {
  const body = findComponent(actor, 'Body');
  if (body?.component === 'Body') return body.physics;
  return { body: 'None', gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5, density: 1, mass: null };
}

/** The script file an actor runs, if it has a Script component. */
export function actorScript(actor: ActorDto | null): string | null {
  const script = findComponent(actor, 'Script');
  return script?.component === 'Script' ? script.path : null;
}

/** Every component an actor is carrying, by name - what an attach or detach
 * block can pick from, alongside the built-ins it hasn't got yet. */
export function componentNames(actor: ActorDto | null): string[] {
  return (actor?.components ?? []).map(componentName);
}

/** The custom components an actor carries, which is what the component blocks
 * can name. */
export function customComponents(actor: ActorDto | null): Extract<ActorComponentDto, { component: 'Custom' }>[] {
  return (actor?.components ?? []).filter(
    (component): component is Extract<ActorComponentDto, { component: 'Custom' }> =>
      component.component === 'Custom',
  );
}

export function findBlockDef(actor: ActorDto | null, blockId: unknown): BlockDefDto | null {
  if (!actor || typeof blockId !== 'string') return null;
  return actor.block_defs.find(def => def.id === blockId) ?? null;
}

export function blockInputPieces(def: BlockDefDto): Extract<BlockPieceDto, { kind: 'Input' }>[] {
  return def.pieces.filter((p): p is Extract<BlockPieceDto, { kind: 'Input' }> => p.kind === 'Input');
}

export function blockInputNames(def: BlockDefDto): string[] {
  return blockInputPieces(def).map(p => p.name);
}

export function blockShapeReturnsValue(shape: BlockShapeDto): boolean {
  return shape === 'ReturnsValue' || shape === 'ReturnsBool';
}

/** `Param:<blockId>:<name>` - the drag kind a block header's parameter oval
 * carries, so a value parked on open canvas still knows which block it came
 * from. See `blockstitchSetup.ts`'s `createFloatingValue`. */
export function parseParamKind(kind: string): { blockId: string | null; name: string } {
  const rest = kind.slice('Param:'.length);
  const split = rest.indexOf(':');
  if (split === -1) return { blockId: null, name: rest };
  return { blockId: rest.slice(0, split), name: rest.slice(split + 1) };
}

/** Variable names the open actor can read: its own, then the project's, each
 * group alphabetical. A name in both is the actor's - it shadows the global. */
export function variableNames(project: ProjectDto | null, actor: ActorDto | null): string[] {
  const own = (actor?.variables ?? []).map(v => v.name).sort((a, b) => a.localeCompare(b));
  const globals = (project?.globals ?? [])
    .map(v => v.name)
    .filter(name => !own.includes(name))
    .sort((a, b) => a.localeCompare(b));
  return [...own, ...globals];
}
