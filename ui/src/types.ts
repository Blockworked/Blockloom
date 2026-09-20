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
  'Say',
  'SetVisible',
  'SetColor',
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
}

export interface CameraDto {
  position: [number, number, number];
  look_at: [number, number, number];
  zoom: number;
  follow: string | null;
}

export interface WorldDto {
  mode: Mode;
  background: string;
  gravity: [number, number, number];
  camera: CameraDto;
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
  visual: VisualDto;
  placement: PlacementDto;
  physics: PhysicsDto;
  visible: boolean;
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

export interface StateDto {
  project_names: string[];
  selected: number | null;
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
    project_names: [],
    selected: null,
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
