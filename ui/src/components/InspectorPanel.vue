<script setup lang="ts">
// The right-hand panel: the selected actor as a list of components. Each
// component is a card with a way to remove it, and "Add component" gives the
// actor one it hasn't got - including a custom one, a named bag of values the
// blocks read and write.
//
// Every field writes straight through to the backend, and a running world
// picks the change up as soon as it stops. The `*Of` helpers narrow the
// component union in one place, so the template stays free of casts.
//
// The rows naming a file - an image, a script - are wrapped in `AssetDrop`,
// so a file dragged out of the asset tray lands on them.
import { computed, ref, watch } from 'vue';
import { AppDropdown, SwitchControl } from 'blockstitch';
import { ChevronLeft, ChevronRight, Lock, LockOpen, Plus, X } from 'lucide-vue-next';
import { COLLAPSED_PANEL_WIDTH, beginPanelResize, panels, setPanelOpen } from '../panels';
import { mode, openActor, state } from '../store';
import {
  addActorComponent,
  checkScript,
  createScript,
  readAsset,
  removeActorComponent,
  renameActor,
  setActorComponent,
} from '../tauri';
import AssetDrop from './AssetDrop.vue';
import ScriptDialog from './ScriptDialog.vue';
import { BODY_OPTIONS, CAMERA_VIEW_OPTIONS, LAYER_OPTIONS } from '../constants';
import {
  ADDABLE_COMPONENTS,
  actorParent,
  actorPhysics,
  actorPlacement,
  actorVisual,
  componentName,
  shapesFor,
  withPhysicsDefaults,
  type ActorComponentDto,
  type CameraAttachDto,
  type ComponentFieldDto,
  type ComponentName,
  type EvaluatedDto,
  type GraphEffectDto,
  type ParticleSpecDto,
  type PhysicsDto,
  type PlacementDto,
  type SurfaceMaterialDto,
  type TilemapDto,
  type TrailSpecDto,
  type VisualDto,
} from '../types';

const actor = computed(() => openActor.value);
/** Where the actor is right now, while a run is going - the project's own
 * numbers are where it will start from again. */
const live = computed(() => state.status?.actors.find(a => a.id === actor.value?.id) ?? null);

const panelStyle = computed(() => ({
  width: `${panels.right.open ? panels.right.width : COLLAPSED_PANEL_WIDTH}px`,
  flexBasis: `${panels.right.open ? panels.right.width : COLLAPSED_PANEL_WIDTH}px`,
}));

const shapeOptions = computed(() => shapesFor(mode.value).map(shape => ({ value: shape, label: shape })));

/** Components this actor hasn't got yet. "Custom" is always on offer: an
 * actor can carry as many of those as it likes. */
const addableComponents = computed(() => {
  const held = new Set((actor.value?.components ?? []).map(componentName));
  return ADDABLE_COMPONENTS.filter(name => name === 'Custom' || !held.has(name)).map(name => ({
    value: name,
    label: name === 'Custom' ? 'Custom…' : name,
  }));
});

const addOpen = ref(false);
/** The script the editor dialog is open on, if any. */
const editingScript = ref<{ actorId: string; actorName: string; path: string } | null>(null);

/** The open actor's image, read back as a data URL - the page can't see a
 * file on disk any other way. Empty when it has none, or when the path names
 * something that isn't there. */
const preview = ref('');
const imagePath = computed(() => {
  const visual = actorVisual(actor.value);
  return visual?.shape === 'Image' ? visual.path : '';
});

watch(
  imagePath,
  async path => {
    preview.value = '';
    if (!path) return;
    try {
      preview.value = await readAsset(path);
    } catch {
      // A path naming nothing shows as the empty row it is.
    }
  },
  { immediate: true },
);

/** Whether the Image size inputs keep the file's own aspect. UI-only. */
const aspectLocked = ref(true);
/** The loaded image's own pixels, from the preview above. */
const naturalSize = ref<{ w: number; h: number } | null>(null);

watch(preview, url => {
  naturalSize.value = null;
  if (!url) return;
  const img = new Image();
  img.onload = () => {
    if (img.naturalWidth > 0 && img.naturalHeight > 0) {
      naturalSize.value = { w: img.naturalWidth, h: img.naturalHeight };
    }
  };
  img.src = url;
});

function imageAspect(): number | null {
  if (naturalSize.value && naturalSize.value.h > 0) {
    return naturalSize.value.w / naturalSize.value.h;
  }
  return null;
}

function num(e: Event, fallback: number): number {
  const parsed = Number((e.target as HTMLInputElement).value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

/** An empty mass input means "no explicit mass" - let density decide. */
function numOrNull(e: Event): number | null {
  const raw = (e.target as HTMLInputElement).value;
  return raw.trim() === '' ? null : num(e, 1);
}

function text(e: Event): string {
  return (e.target as HTMLInputElement).value;
}

// ─── Reading one component ─────────────────────────────────────────────────

function placementOf(component: ActorComponentDto): PlacementDto {
  return component.component === 'Place' ? component.placement : actorPlacement(actor.value);
}

function visualOf(component: ActorComponentDto): VisualDto {
  return component.component === 'Look'
    ? component.visual
    : { shape: 'Rect', color: '#4C97FF', size: [60, 60] };
}

function physicsOf(component: ActorComponentDto): PhysicsDto {
  return component.component === 'Body'
    ? withPhysicsDefaults(component.physics)
    : actorPhysics(actor.value);
}

/** Toggles one layer bit in the actor's collision mask. */
function toggleMaskBit(component: ActorComponentDto, layer: number) {
  const bit = 1 << (layer - 1);
  const mask = physicsOf(component).collision_mask ^ bit;
  writePhysics(component, { collision_mask: mask });
}

function cameraOf(component: ActorComponentDto): CameraAttachDto {
  return component.component === 'Camera'
    ? { ...component.camera, fov: component.camera.fov ?? 75 }
    : { view: 'Follow', offset: [0, 0.6, 0], distance: 6, pitch: 15, fov: 75 };
}

function fieldsOf(component: ActorComponentDto): ComponentFieldDto[] {
  return component.component === 'Custom' ? component.fields : [];
}

function scriptPathOf(component: ActorComponentDto): string {
  return component.component === 'Script' ? component.path : '';
}

function visibleOf(component: ActorComponentDto): boolean {
  return component.component === 'Render' ? component.visible : true;
}

function parentOf(component: ActorComponentDto): string {
  return component.component === 'Parent' ? component.parent : '';
}

function offsetOf(component: ActorComponentDto): [number, number, number] | null {
  return component.component === 'Parent' ? (component.offset ?? null) : null;
}

/** Writes the whole component back: an id, an offset, or both. */
function writeParent(component: ActorComponentDto, next: Partial<{ parent: string; offset: [number, number, number] | null }>) {
  write('Parent', {
    component: 'Parent',
    parent: next.parent ?? parentOf(component),
    offset: 'offset' in next ? (next.offset ?? null) : offsetOf(component),
  });
}

function writeOffset(component: ActorComponentDto, axis: number, value: number) {
  const offset: [number, number, number] = [...(offsetOf(component) ?? [0, 0, 0])];
  offset[axis] = value;
  writeParent(component, { offset });
}

/** Turning this on measures the offset from where the two actors stand now,
 * so the actor doesn't move the moment it starts being placed by its parent. */
function toggleOffset(component: ActorComponentDto, on: boolean) {
  if (!on) {
    writeParent(component, { offset: null });
    return;
  }
  const parent = state.project?.actors.find(candidate => candidate.id === parentOf(component));
  const mine = actorPlacement(actor.value)?.position ?? [0, 0, 0];
  const theirs = actorPlacement(parent ?? null)?.position ?? [0, 0, 0];
  writeParent(component, {
    offset: [mine[0] - theirs[0], mine[1] - theirs[1], mine[2] - theirs[2]],
  });
}

/** Every other actor, as a parent to hang this one off. Stored by id so a
 * rename doesn't break the link; "nothing" clears it. */
const parentOptions = computed(() => [
  { value: '', label: 'nothing' },
  ...(state.project?.actors ?? [])
    .filter(candidate => candidate.id !== actor.value?.id && !hangsOffMe(candidate.id))
    .map(candidate => ({ value: candidate.id, label: candidate.name })),
]);

/** Whether `id` already hangs off the open actor, directly or further down -
 * offering it as a parent would make a loop. */
function hangsOffMe(id: string): boolean {
  const me = actor.value?.id;
  if (!me) return false;
  const seen = new Set<string>();
  let at: string | null = id;
  while (at && !seen.has(at)) {
    seen.add(at);
    at = actorParent(state.project?.actors.find(candidate => candidate.id === at) ?? null);
    if (at === me) return true;
  }
  return false;
}

function sizeOf(component: ActorComponentDto): number[] {
  const visual = visualOf(component);
  return 'size' in visual ? visual.size : [];
}

function colorOf(component: ActorComponentDto): string {
  const visual = visualOf(component);
  return 'color' in visual ? visual.color : '#4C97FF';
}

function radiusOf(component: ActorComponentDto): number {
  const visual = visualOf(component);
  return 'radius' in visual ? visual.radius : 0;
}

function heightOf(component: ActorComponentDto): number {
  const visual = visualOf(component);
  return visual.shape === 'Capsule' ? visual.height : 0;
}

function imagePathOf(component: ActorComponentDto): string {
  const visual = visualOf(component);
  return visual.shape === 'Image' ? visual.path : '';
}

function modelPathOf(component: ActorComponentDto): string {
  const visual = visualOf(component);
  return visual.shape === 'Model' ? visual.path : '';
}

function modelTintOf(component: ActorComponentDto): string {
  const visual = visualOf(component);
  return visual.shape === 'Model' ? visual.tint : '#4C97FF';
}

function modelScaleOf(component: ActorComponentDto): [number, number, number] {
  const visual = visualOf(component);
  return visual.shape === 'Model' ? visual.scale : [1, 1, 1];
}

function writeModelScale(component: ActorComponentDto, index: number, value: number) {
  const scale: [number, number, number] = [...modelScaleOf(component)];
  scale[index] = value;
  writeVisual(component, { scale } as Partial<VisualDto>);
}

function tilemapOf(component: ActorComponentDto): TilemapDto {
  const current =
    component.component === 'Look' && component.visual.shape === 'Tilemap'
      ? component.visual.tilemap
      : null;
  return {
    tileset: current?.tileset ?? '',
    tile_size: current?.tile_size ?? [32, 32],
    width: current?.width ?? 8,
    height: current?.height ?? 8,
    sheet_columns: current?.sheet_columns ?? 4,
    sheet_rows: current?.sheet_rows ?? 4,
    tiles: current?.tiles ?? [],
    solid: current?.solid ?? false,
  };
}

function writeTilemap(component: ActorComponentDto, next: Partial<TilemapDto>) {
  const tilemap = { ...tilemapOf(component), ...next };
  // The grid is always exactly width × height; resizing keeps what overlaps.
  const width = Math.min(256, Math.max(1, Math.round(tilemap.width)));
  const height = Math.min(256, Math.max(1, Math.round(tilemap.height)));
  const tiles: number[] = [];
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      tiles.push(y < tilemapOf(component).height && x < tilemapOf(component).width
        ? (tilemapOf(component).tiles[y * tilemapOf(component).width + x] ?? -1)
        : -1);
    }
  }
  writeVisual(component, { tilemap: { ...tilemap, width, height, tiles } } as Partial<VisualDto>);
}

function writeTileSize(component: ActorComponentDto, index: number, value: number) {
  const tile_size: [number, number] = [...tilemapOf(component).tile_size];
  tile_size[index] = Math.max(1, value);
  writeTilemap(component, { tile_size });
}

function writeTilemapSize(component: ActorComponentDto, index: number, value: number) {
  writeTilemap(component, index === 0 ? { width: value } : { height: value });
}

/** Which tile a grid click paints, shared across the panel. -1 erases. */
const paintTile = ref(0);

function paintTileAt(component: ActorComponentDto, index: number) {
  const current = tilemapOf(component);
  if (index < 0 || index >= current.tiles.length) return;
  const tiles = [...current.tiles];
  const cells = Math.max(1, current.sheet_columns * current.sheet_rows);
  const tile = Math.round(paintTile.value);
  tiles[index] = tile >= 0 && tile < cells ? tile : -1;
  writeVisual(component, { tilemap: { ...current, tiles } } as Partial<VisualDto>);
}

function layerOf(component: ActorComponentDto): number {
  return component.component === 'Render' ? (component.layer ?? 0) : 0;
}

function writeRender(component: ActorComponentDto, next: { visible?: boolean; layer?: number }) {
  write('Render', {
    component: 'Render',
    visible: next.visible ?? visibleOf(component),
    layer: next.layer ?? layerOf(component),
  });
}

function materialOf(component: ActorComponentDto): SurfaceMaterialDto {
  return component.component === 'Material'
    ? component.material
    : { metallic: 0, roughness: 0.6, emissive: '#000000', emissive_energy: 0, albedo_texture: '', double_sided: false, shader: null };
}

function writeMaterial(component: ActorComponentDto, next: Partial<SurfaceMaterialDto>) {
  write('Material', { component: 'Material', material: { ...materialOf(component), ...next } });
}

function writeShader(component: ActorComponentDto, next: Partial<GraphEffectDto>) {
  const current = materialOf(component).shader ?? { mode: 'Solid', speed: 1, strength: 0.5, color: '#FFFFFF' };
  writeMaterial(component, { shader: { ...current, ...next } });
}

/** Switching the effect on starts from Solid white; switching it off drops
 * the whole custom path and the actor renders PBR again. */
function toggleShader(component: ActorComponentDto, on: boolean) {
  writeMaterial(component, {
    shader: on ? { mode: 'Solid', speed: 1, strength: 0.5, color: '#FFFFFF' } : null,
  });
}

function emitterOf(component: ActorComponentDto): ParticleSpecDto {
  return component.component === 'Emitter'
    ? component.emitter
    : { rate: 24, lifetime: 0.8, speed: 120, spread: 60, gravity_scale: 0.5, size_start: 6, size_end: 1, color_start: '#FFFFFF', color_end: '#FFAB19', max: 128 };
}

function writeEmitter(component: ActorComponentDto, next: Partial<ParticleSpecDto>) {
  write('Emitter', { component: 'Emitter', emitter: { ...emitterOf(component), ...next } });
}

function trailOf(component: ActorComponentDto): TrailSpecDto {
  return component.component === 'Trail'
    ? component.trail
    : { interval: 0.05, life: 0.4, color: '#FFFFFF' };
}

function writeTrail(component: ActorComponentDto, next: Partial<TrailSpecDto>) {
  write('Trail', { component: 'Trail', trail: { ...trailOf(component), ...next } });
}

/** The custom motions the effect dropdown offers. */
const EFFECT_OPTIONS = (['Solid', 'Wave', 'Plasma', 'Pulse', 'Dissolve'] as const).map(mode => ({
  value: mode,
  label: mode,
}));

// ─── Writing one component ─────────────────────────────────────────────────

/** Writes a component back under the name it currently has, so renaming a
 * custom one lands on the same slot. */
function write(name: string, component: ActorComponentDto) {
  if (!actor.value) return;
  void setActorComponent(actor.value.id, name, component).catch((e: unknown) => console.error(e));
}

function writePlacement(component: ActorComponentDto, next: Partial<PlacementDto>) {
  write('Place', { component: 'Place', placement: { ...placementOf(component), ...next } });
}

function writeVector(component: ActorComponentDto, key: 'position' | 'rotation', index: number, value: number) {
  const next: [number, number, number] = [...placementOf(component)[key]];
  next[index] = value;
  writePlacement(component, key === 'position' ? { position: next } : { rotation: next });
}

function writeVisual(component: ActorComponentDto, next: Partial<VisualDto>) {
  write('Look', { component: 'Look', visual: { ...visualOf(component), ...next } as VisualDto });
}

function writeSize(component: ActorComponentDto, index: number, value: number) {
  const size = [...sizeOf(component)];
  size[index] = value;
  writeVisual(component, { size } as unknown as Partial<VisualDto>);
}

/** Writes one Image dimension, carrying the other along when the aspect is
 * locked to the file's own proportions. */
function writeSizeAspect(component: ActorComponentDto, index: number, value: number) {
  const aspect = aspectLocked.value ? imageAspect() : null;
  const size = [...sizeOf(component)];
  if (size.length < 2 || !aspect || !(value > 0)) {
    writeSize(component, index, value);
    return;
  }
  const next = index === 0 ? [value, value / aspect] : [value * aspect, value];
  writeVisual(component, { size: next } as unknown as Partial<VisualDto>);
}

/** Flips the aspect lock. Turning it on snaps the height to the width, so
 * what is on screen is already at the file's proportions. */
function toggleAspectLock(component: ActorComponentDto) {
  aspectLocked.value = !aspectLocked.value;
  if (!aspectLocked.value) return;
  const aspect = imageAspect();
  const size = sizeOf(component);
  if (aspect && size.length === 2 && size[0] > 0) {
    writeVisual(component, { size: [size[0], size[0] / aspect] } as unknown as Partial<VisualDto>);
  }
}

/** Swapping a shape keeps what carries over (its color) and takes sensible
 * defaults for the rest, since a sphere has no width and a box has no radius. */
function writeShape(component: ActorComponentDto, shape: string) {
  const color = colorOf(component);
  const visuals: Record<string, VisualDto> = {
    Rect: { shape: 'Rect', color, size: [60, 60] },
    Circle: { shape: 'Circle', color, radius: 30 },
    Image: { shape: 'Image', path: '', size: [80, 80] },
    Cuboid: { shape: 'Cuboid', color, size: [1, 1, 1] },
    Sphere: { shape: 'Sphere', color, radius: 0.5 },
    Capsule: { shape: 'Capsule', color, radius: 0.4, height: 1 },
    Plane: { shape: 'Plane', color, size: [20, 20] },
    Model: { shape: 'Model', path: '', tint: color, scale: [1, 1, 1] },
    Tilemap: {
      shape: 'Tilemap',
      tilemap: {
        tileset: '',
        tile_size: [32, 32],
        width: 8,
        height: 8,
        sheet_columns: 4,
        sheet_rows: 4,
        tiles: Array(64).fill(-1),
        solid: false,
      },
    },
  };
  const next = visuals[shape];
  if (next) write('Look', { component: 'Look', visual: next });
}

function writePhysics(component: ActorComponentDto, next: Partial<PhysicsDto>) {
  write('Body', { component: 'Body', physics: { ...physicsOf(component), ...next } });
}

function writeCameraAttach(component: ActorComponentDto, next: Partial<CameraAttachDto>) {
  write('Camera', { component: 'Camera', camera: { ...cameraOf(component), ...next } });
}

function writeCameraOffset(component: ActorComponentDto, index: number, value: number) {
  const offset: [number, number, number] = [...cameraOf(component).offset];
  offset[index] = value;
  writeCameraAttach(component, { offset });
}

// ─── Custom components ─────────────────────────────────────────────────────

function writeCustom(component: ActorComponentDto, next: { name?: string; fields?: ComponentFieldDto[] }) {
  if (component.component !== 'Custom') return;
  write(component.name, {
    component: 'Custom',
    name: next.name ?? component.name,
    fields: next.fields ?? component.fields,
  });
}

function writeField(component: ActorComponentDto, index: number, next: Partial<ComponentFieldDto>) {
  const fields = fieldsOf(component).map((field, i) => (i === index ? { ...field, ...next } : field));
  writeCustom(component, { fields });
}

/** Keeps a field's type as the user typed it: a number that parses stays a
 * number, so arithmetic on it works; anything else is text. */
function parsedValue(raw: string): EvaluatedDto {
  const trimmed = raw.trim();
  if (trimmed !== '' && Number.isFinite(Number(trimmed))) return { kind: 'Number', value: Number(trimmed) };
  return { kind: 'Text', value: raw };
}

function fieldText(value: EvaluatedDto): string {
  return String(value.value);
}

function addField(component: ActorComponentDto) {
  const fields = fieldsOf(component);
  const taken = new Set(fields.map(field => field.name));
  let name = 'value';
  for (let n = 2; taken.has(name); n += 1) name = `value ${n}`;
  writeCustom(component, { fields: [...fields, { name, value: { kind: 'Number', value: 0 } }] });
}

function removeField(component: ActorComponentDto, index: number) {
  writeCustom(component, { fields: fieldsOf(component).filter((_, i) => i !== index) });
}

// ─── Scripts ───────────────────────────────────────────────────────────────

function openScript(component: ActorComponentDto) {
  if (!actor.value) return;
  editingScript.value = {
    actorId: actor.value.id,
    actorName: actor.value.name,
    path: scriptPathOf(component),
  };
}

/** Attaching a Script makes the file too, so the button that adds it is the
 * same one that writes the starter template. */
function newScript() {
  if (!actor.value) return;
  void createScript(actor.value.id).catch((e: unknown) => console.error(e));
}

function check() {
  if (!actor.value) return;
  void checkScript(actor.value.id).catch((e: unknown) => console.error(e));
}

// ─── Adding and removing whole components ──────────────────────────────────

/** A fresh component of each kind, with defaults that do something useful the
 * moment it lands. */
function blankComponent(name: ComponentName): ActorComponentDto | null {
  switch (name) {
    case 'Look':
      return {
        component: 'Look',
        visual:
          mode.value === 'ThreeD'
            ? { shape: 'Cuboid', color: '#4C97FF', size: [1, 1, 1] }
            : { shape: 'Rect', color: '#4C97FF', size: [60, 60] },
      };
    case 'Render':
      return { component: 'Render', visible: true, layer: 0 };
    case 'Body':
      return {
        component: 'Body',
        physics: { body: 'Dynamic', gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5, density: 1, mass: null, trigger: false, collision_layer: 1, collision_mask: 255 },
      };
    case 'Camera':
      return {
        component: 'Camera',
        camera: { view: 'ThirdPerson', offset: [0, 0.6, 0], distance: 6, pitch: 15, fov: 75 },
      };
    case 'Parent':
      return { component: 'Parent', parent: '', offset: null };
    case 'Material':
      return {
        component: 'Material',
        material: {
          metallic: 0,
          roughness: 0.6,
          emissive: '#000000',
          emissive_energy: 0,
          albedo_texture: '',
          double_sided: false,
          shader: null,
        },
      };
    case 'Emitter':
      return {
        component: 'Emitter',
        emitter: {
          rate: 24,
          lifetime: 0.8,
          speed: 120,
          spread: 60,
          gravity_scale: 0.5,
          size_start: 6,
          size_end: 1,
          color_start: '#FFFFFF',
          color_end: '#FFAB19',
          max: 128,
        },
      };
    case 'Trail':
      return {
        component: 'Trail',
        trail: { interval: 0.05, life: 0.4, color: '#FFFFFF' },
      };
    case 'Custom':
      return {
        component: 'Custom',
        name: 'Component',
        fields: [{ name: 'value', value: { kind: 'Number', value: 0 } }],
      };
    // A script needs a file on disk, so the backend makes both at once.
    case 'Script':
      return null;
    default:
      return null;
  }
}

function add(name: string) {
  addOpen.value = false;
  if (name === 'Script') {
    newScript();
    return;
  }
  const component = blankComponent(name as ComponentName);
  if (!component || !actor.value) return;
  void addActorComponent(actor.value.id, component).catch((e: unknown) => console.error(e));
}

function remove(name: string) {
  if (!actor.value) return;
  void removeActorComponent(actor.value.id, name).catch((e: unknown) => console.error(e));
}
</script>

<template>
  <aside class="side-panel right" :style="panelStyle" :class="{ collapsed: !panels.right.open }">
    <template v-if="actor">
      <div class="panel-heading">
        <span>{{ actor.name }}</span>
        <div class="panel-heading-actions">
          <button class="panel-collapse" title="Hide the components" @click="setPanelOpen('right', false)">
            <ChevronRight :size="13" />
          </button>
        </div>
      </div>
      <div class="panel-row">
        <label>Name</label>
        <input type="text" :value="actor.name" @change="e => renameActor(actor!.id, text(e))">
      </div>

      <template v-for="component in actor.components" :key="componentName(component)">
        <div class="panel-heading component-heading">
          <span>{{ componentName(component) }}</span>
          <button
            v-if="component.component !== 'Place'"
            class="component-remove"
            :title="`Remove the ${componentName(component)} component`"
            @click="remove(componentName(component))"
          >
            <X :size="13" />
          </button>
        </div>

        <!-- Place: every actor has one, and it can't be taken away. -->
        <template v-if="component.component === 'Place'">
          <div class="panel-row triple">
            <label>Position</label>
            <input type="number" step="any" :value="placementOf(component).position[0]" @change="e => writeVector(component, 'position', 0, num(e, 0))">
            <input type="number" step="any" :value="placementOf(component).position[1]" @change="e => writeVector(component, 'position', 1, num(e, 0))">
            <input v-if="mode === 'ThreeD'" type="number" step="any" :value="placementOf(component).position[2]" @change="e => writeVector(component, 'position', 2, num(e, 0))">
          </div>
          <div class="panel-row triple">
            <label>Rotation</label>
            <input v-if="mode === 'ThreeD'" type="number" step="any" :value="placementOf(component).rotation[0]" @change="e => writeVector(component, 'rotation', 0, num(e, 0))">
            <input v-if="mode === 'ThreeD'" type="number" step="any" :value="placementOf(component).rotation[1]" @change="e => writeVector(component, 'rotation', 1, num(e, 0))">
            <input type="number" step="any" :value="placementOf(component).rotation[2]" @change="e => writeVector(component, 'rotation', 2, num(e, 0))">
          </div>
          <div class="panel-row">
            <label>Size</label>
            <input type="number" step="any" :value="placementOf(component).scale" @change="e => writePlacement(component, { scale: num(e, 1) })">
          </div>
          <p class="panel-note" v-if="live">
            Now at {{ live.position.map(n => n.toFixed(1)).join(', ') }}
          </p>
        </template>

        <!-- Look: the shape, which is also the collider a body gets. -->
        <template v-else-if="component.component === 'Look'">
          <div class="panel-row">
            <label>Shape</label>
            <AppDropdown
              :options="shapeOptions"
              :model-value="visualOf(component).shape"
              @update:model-value="shape => writeShape(component, shape)"
            />
          </div>
          <div class="panel-row" v-if="'color' in visualOf(component)">
            <label>Color</label>
            <input type="color" :value="colorOf(component)" @change="e => writeVisual(component, { color: text(e).toUpperCase() } as Partial<VisualDto>)">
          </div>
          <div class="panel-row" v-if="visualOf(component).shape === 'Image'">
            <label>Image</label>
            <AssetDrop :accept="['image']" @asset="path => writeVisual(component, { path } as Partial<VisualDto>)">
              <input
                type="text"
                :value="imagePathOf(component)"
                placeholder="Drag an image here"
                @change="e => writeVisual(component, { path: text(e) } as Partial<VisualDto>)"
              >
            </AssetDrop>
          </div>
          <div class="panel-row image-preview" v-if="visualOf(component).shape === 'Image' && preview">
            <label />
            <img :src="preview" :alt="imagePathOf(component)">
          </div>
          <div class="panel-row" v-if="visualOf(component).shape === 'Model'">
            <label>Model</label>
            <AssetDrop :accept="['model']" @asset="path => writeVisual(component, { path } as Partial<VisualDto>)">
              <input
                type="text"
                :value="modelPathOf(component)"
                placeholder="Drag a model here"
                @change="e => writeVisual(component, { path: text(e) } as Partial<VisualDto>)"
              >
            </AssetDrop>
          </div>
          <div class="panel-row" v-if="visualOf(component).shape === 'Model'">
            <label>Tint</label>
            <input type="color" :value="modelTintOf(component)" @change="e => writeVisual(component, { tint: text(e).toUpperCase() } as Partial<VisualDto>)">
          </div>
          <div class="panel-row triple" v-if="visualOf(component).shape === 'Model'">
            <label>Scale</label>
            <input
              v-for="(dimension, i) in modelScaleOf(component)"
              :key="i"
              type="number"
              step="any"
              :value="dimension"
              @change="e => writeModelScale(component, i, num(e, dimension))"
            >
          </div>
          <p class="panel-note" v-if="visualOf(component).shape === 'Model' && !modelPathOf(component)">
            No file yet: the actor renders nothing until one is picked. glTF
            plays back rigs; OBJ and FBX import as static meshes.
          </p>
          <template v-if="visualOf(component).shape === 'Tilemap'">
            <div class="panel-row">
              <label>Tileset</label>
              <AssetDrop :accept="['image']" @asset="path => writeTilemap(component, { tileset: path })">
                <input
                  type="text"
                  :value="tilemapOf(component).tileset"
                  placeholder="Drag a tileset here"
                  @change="e => writeTilemap(component, { tileset: text(e) })"
                >
              </AssetDrop>
            </div>
            <div class="panel-row triple">
              <label>Tile px</label>
              <input type="number" step="any" :value="tilemapOf(component).tile_size[0]" @change="e => writeTileSize(component, 0, num(e, 32))">
              <input type="number" step="any" :value="tilemapOf(component).tile_size[1]" @change="e => writeTileSize(component, 1, num(e, 32))">
            </div>
            <div class="panel-row triple">
              <label>Map</label>
              <input type="number" step="1" min="1" max="256" :value="tilemapOf(component).width" @change="e => writeTilemapSize(component, 0, num(e, 8))">
              <input type="number" step="1" min="1" max="256" :value="tilemapOf(component).height" @change="e => writeTilemapSize(component, 1, num(e, 8))">
            </div>
            <div class="panel-row triple">
              <label>Sheet</label>
              <input type="number" step="1" min="1" :value="tilemapOf(component).sheet_columns" @change="e => writeTilemap(component, { sheet_columns: Math.max(1, Math.round(num(e, 4))) })">
              <input type="number" step="1" min="1" :value="tilemapOf(component).sheet_rows" @change="e => writeTilemap(component, { sheet_rows: Math.max(1, Math.round(num(e, 4))) })">
            </div>
            <div class="panel-row">
              <label>Solid</label>
              <SwitchControl
                :model-value="tilemapOf(component).solid"
                @update:model-value="v => writeTilemap(component, { solid: v })"
              />
            </div>
            <div class="panel-row">
              <label>Paint</label>
              <input type="number" step="1" min="-1" :value="paintTile" @change="e => paintTile = Math.round(num(e, 0))">
              <span class="panel-note">-1 erases</span>
            </div>
            <div class="panel-row">
              <label>Grid</label>
              <div class="tile-grid" :style="{ gridTemplateColumns: `repeat(${tilemapOf(component).width}, 18px)` }">
                <button
                  v-for="(tile, i) in tilemapOf(component).tiles"
                  :key="i"
                  class="tile-cell"
                  :class="{ filled: tile >= 0 }"
                  :title="`(${(i % tilemapOf(component).width)}, ${Math.floor(i / tilemapOf(component).width)}): ${tile}`"
                  @click="paintTileAt(component, i)"
                >
                  {{ tile >= 0 ? tile : '' }}
                </button>
              </div>
            </div>
          </template>
          <div class="panel-row" v-if="'radius' in visualOf(component)">
            <label>Radius</label>
            <input type="number" step="any" :value="radiusOf(component)" @change="e => writeVisual(component, { radius: num(e, 1) } as Partial<VisualDto>)">
          </div>
          <div class="panel-row" v-if="visualOf(component).shape === 'Capsule'">
            <label>Height</label>
            <input type="number" step="any" :value="heightOf(component)" @change="e => writeVisual(component, { height: num(e, 1) } as Partial<VisualDto>)">
          </div>
          <div class="panel-row triple" v-if="visualOf(component).shape === 'Image' && sizeOf(component).length === 2">
            <label>Size</label>
            <input
              type="number"
              step="any"
              :value="sizeOf(component)[0]"
              @change="e => writeSizeAspect(component, 0, num(e, sizeOf(component)[0]))"
            >
            <input
              type="number"
              step="any"
              :value="sizeOf(component)[1]"
              @change="e => writeSizeAspect(component, 1, num(e, sizeOf(component)[1]))"
            >
            <button
              class="component-remove aspect-lock"
              :class="{ on: aspectLocked }"
              :title="naturalSize ? (aspectLocked ? `Locked to ${naturalSize.w}×${naturalSize.h}` : `Lock to ${naturalSize.w}×${naturalSize.h}`) : 'Load an image to lock its aspect'"
              :disabled="!naturalSize"
              @click="toggleAspectLock(component)"
            >
              <Lock v-if="aspectLocked" :size="13" />
              <LockOpen v-else :size="13" />
            </button>
          </div>
          <div class="panel-row triple" v-else-if="sizeOf(component).length">
            <label>Size</label>
            <input
              v-for="(dimension, i) in sizeOf(component)"
              :key="i"
              type="number"
              step="any"
              :value="dimension"
              @change="e => writeSize(component, i, num(e, dimension))"
            >
          </div>
        </template>

        <template v-else-if="component.component === 'Parent'">
          <div class="panel-row">
            <label>Hangs off</label>
            <AppDropdown
              :options="parentOptions"
              :model-value="parentOf(component)"
              placeholder="nothing"
              @update:model-value="id => writeParent(component, { parent: id })"
            />
          </div>
          <div class="panel-row" v-if="parentOf(component)">
            <label>Placed by it</label>
            <SwitchControl
              :model-value="offsetOf(component) !== null"
              @update:model-value="on => toggleOffset(component, on)"
            />
          </div>
          <div class="panel-row triple" v-if="offsetOf(component)">
            <label>Offset</label>
            <input type="number" step="any" :value="offsetOf(component)![0]" @change="e => writeOffset(component, 0, num(e, 0))">
            <input type="number" step="any" :value="offsetOf(component)![1]" @change="e => writeOffset(component, 1, num(e, 0))">
            <input v-if="mode === 'ThreeD'" type="number" step="any" :value="offsetOf(component)![2]" @change="e => writeOffset(component, 2, num(e, 0))">
          </div>
          <p class="panel-note">
            <template v-if="offsetOf(component)">
              This actor starts that far from its parent, in the parent's own frame, and every move
              the parent makes is made to it too.
            </template>
            <template v-else>
              This actor keeps its own place, and every move its parent makes is made to it too.
            </template>
          </p>
        </template>

        <template v-else-if="component.component === 'Render'">
          <div class="panel-row">
            <label>Visible</label>
            <SwitchControl
              :model-value="visibleOf(component)"
              @update:model-value="v => writeRender(component, { visible: v })"
            />
          </div>
          <div class="panel-row" v-if="mode === 'TwoD'">
            <label>Layer</label>
            <input type="number" step="1" :value="layerOf(component)" @change="e => writeRender(component, { layer: Math.round(num(e, 0)) })">
          </div>
          <p class="panel-note" v-if="mode === 'TwoD'">
            Higher layers draw on top, without touching the actor's depth.
          </p>
        </template>

        <template v-else-if="component.component === 'Body'">
          <div class="panel-row">
            <label>Kind</label>
            <AppDropdown
              :options="BODY_OPTIONS"
              :model-value="physicsOf(component).body"
              @update:model-value="body => writePhysics(component, { body: body as PhysicsDto['body'] })"
            />
          </div>
          <template v-if="physicsOf(component).body !== 'None'">
            <div class="panel-row">
              <label>Gravity ×</label>
              <input type="number" step="any" :value="physicsOf(component).gravity_scale" @change="e => writePhysics(component, { gravity_scale: num(e, 1) })">
            </div>
            <div class="panel-row">
              <label>Bounce</label>
              <input type="number" step="any" :value="physicsOf(component).restitution" @change="e => writePhysics(component, { restitution: num(e, 0) })">
            </div>
            <div class="panel-row">
              <label>Friction</label>
              <input type="number" step="any" :value="physicsOf(component).friction" @change="e => writePhysics(component, { friction: num(e, 0.5) })">
            </div>
            <div class="panel-row">
              <label>Density</label>
              <input type="number" step="any" :value="physicsOf(component).density" @change="e => writePhysics(component, { density: num(e, 1) })">
            </div>
            <div class="panel-row">
              <label>Mass (optional)</label>
              <input type="number" step="any" :value="physicsOf(component).mass ?? ''" placeholder="density decides" @change="e => writePhysics(component, { mass: numOrNull(e) })">
            </div>
            <div class="panel-row">
              <label>Upright</label>
              <SwitchControl
                :model-value="physicsOf(component).lock_rotation"
                @update:model-value="v => writePhysics(component, { lock_rotation: v })"
              />
            </div>
            <div class="panel-row">
              <label>Trigger</label>
              <SwitchControl
                :model-value="physicsOf(component).trigger"
                @update:model-value="v => writePhysics(component, { trigger: v })"
              />
            </div>
            <div class="panel-row">
              <label>Layer</label>
              <AppDropdown
                :options="LAYER_OPTIONS"
                :model-value="String(physicsOf(component).collision_layer)"
                @update:model-value="layer => writePhysics(component, { collision_layer: Number(layer) })"
              />
            </div>
            <div class="panel-row">
              <label>Hits</label>
              <div class="mask-grid">
                <button
                  v-for="layer in 8"
                  :key="layer"
                  class="mask-bit"
                  :class="{ on: (physicsOf(component).collision_mask & (1 << (layer - 1))) !== 0 }"
                  :title="`layer ${layer}`"
                  @click="toggleMaskBit(component, layer)"
                >
                  {{ layer }}
                </button>
              </div>
            </div>
          </template>
        </template>

        <template v-else-if="component.component === 'Camera'">
          <div class="panel-row">
            <label>View</label>
            <AppDropdown
              :options="CAMERA_VIEW_OPTIONS"
              :model-value="cameraOf(component).view"
              @update:model-value="view => writeCameraAttach(component, { view: view as CameraAttachDto['view'] })"
            />
          </div>
          <div class="panel-row triple">
            <label>Eye at</label>
            <input
              v-for="(coordinate, i) in cameraOf(component).offset"
              :key="i"
              type="number"
              step="any"
              :value="coordinate"
              @change="e => writeCameraOffset(component, i, num(e, coordinate))"
            >
          </div>
          <template v-if="cameraOf(component).view === 'ThirdPerson' && mode === 'ThreeD'">
            <div class="panel-row">
              <label>Distance</label>
              <input type="number" step="any" :value="cameraOf(component).distance" @change="e => writeCameraAttach(component, { distance: num(e, 6) })">
            </div>
            <div class="panel-row">
              <label>Pitch</label>
              <input type="number" step="any" :value="cameraOf(component).pitch" @change="e => writeCameraAttach(component, { pitch: num(e, 15) })">
            </div>
          </template>
          <div class="panel-row" v-if="mode === 'ThreeD'">
            <label>FOV</label>
            <input type="number" step="any" min="30" max="110" :value="cameraOf(component).fov ?? 75" @change="e => writeCameraAttach(component, { fov: num(e, 75) })">
          </div>
          <p class="panel-note" v-if="mode === 'TwoD'">
            A 2D world has no depth to stand in, so every view here just keeps
            this actor centered.
          </p>
        </template>

        <template v-else-if="component.component === 'Script'">
          <div class="panel-row">
            <AssetDrop :accept="['script']" @asset="path => write('Script', { component: 'Script', path })">
              <span class="script-path" :title="scriptPathOf(component)">{{ scriptPathOf(component) }}</span>
            </AssetDrop>
          </div>
          <div class="panel-row">
            <button class="btn-small" @click="openScript(component)">Edit</button>
            <button class="btn-small" @click="check">Check</button>
          </div>
          <p class="panel-note">
            Real Rust, compiled when you press Play. It runs alongside this
            actor's blocks, not instead of them.
          </p>
        </template>

        <!-- A custom component: a name, and the values the blocks read. -->
        <template v-else-if="component.component === 'Custom'">
          <div class="panel-row">
            <label>Name</label>
            <input type="text" :value="componentName(component)" @change="e => writeCustom(component, { name: text(e) })">
          </div>
          <div class="panel-row field-row" v-for="(field, i) in fieldsOf(component)" :key="i">
            <input class="field-name" type="text" :value="field.name" @change="e => writeField(component, i, { name: text(e) })">
            <input type="text" :value="fieldText(field.value)" @change="e => writeField(component, i, { value: parsedValue(text(e)) })">
            <button class="component-remove" title="Remove this field" @click="removeField(component, i)">
              <X :size="13" />
            </button>
          </div>
          <div class="panel-row">
            <button class="component-add-field" @click="addField(component)">
              <Plus :size="13" /> Add field
            </button>
          </div>
        </template>

        <template v-else-if="component.component === 'Material'">
          <div class="panel-row">
            <label>Metallic</label>
            <input type="number" step="any" min="0" max="1" :value="materialOf(component).metallic" @change="e => writeMaterial(component, { metallic: num(e, 0) })">
          </div>
          <div class="panel-row">
            <label>Rough</label>
            <input type="number" step="any" min="0" max="1" :value="materialOf(component).roughness" @change="e => writeMaterial(component, { roughness: num(e, 0.6) })">
          </div>
          <div class="panel-row">
            <label>Glow</label>
            <input type="color" :value="materialOf(component).emissive" @change="e => writeMaterial(component, { emissive: text(e).toUpperCase() })">
            <input type="number" step="any" min="0" :value="materialOf(component).emissive_energy" title="Glow strength" @change="e => writeMaterial(component, { emissive_energy: num(e, 0) })">
          </div>
          <div class="panel-row">
            <label>Texture</label>
            <AssetDrop :accept="['image']" @asset="path => writeMaterial(component, { albedo_texture: path })">
              <input
                type="text"
                :value="materialOf(component).albedo_texture"
                placeholder="Optional albedo"
                @change="e => writeMaterial(component, { albedo_texture: text(e) })"
              >
            </AssetDrop>
          </div>
          <div class="panel-row">
            <label>Two-sided</label>
            <SwitchControl
              :model-value="materialOf(component).double_sided"
              @update:model-value="v => writeMaterial(component, { double_sided: v })"
            />
          </div>
          <div class="panel-row">
            <label>Effect</label>
            <SwitchControl
              :model-value="materialOf(component).shader !== null"
              @update:model-value="v => toggleShader(component, v)"
            />
          </div>
          <template v-if="materialOf(component).shader">
            <div class="panel-row">
              <label>Motion</label>
              <AppDropdown
                :options="EFFECT_OPTIONS"
                :model-value="materialOf(component).shader!.mode"
                @update:model-value="mode => writeShader(component, { mode: mode as GraphEffectDto['mode'] })"
              />
            </div>
            <div class="panel-row">
              <label>Speed</label>
              <input type="number" step="any" min="0" :value="materialOf(component).shader!.speed" @change="e => writeShader(component, { speed: num(e, 1) })">
            </div>
            <div class="panel-row">
              <label>Strength</label>
              <input type="number" step="any" min="0" max="1" :value="materialOf(component).shader!.strength" @change="e => writeShader(component, { strength: num(e, 0.5) })">
            </div>
            <div class="panel-row">
              <label>Color</label>
              <input type="color" :value="materialOf(component).shader!.color" @change="e => writeShader(component, { color: text(e).toUpperCase() })">
            </div>
          </template>
          <p class="panel-note" v-if="mode === 'TwoD' && !materialOf(component).shader">
            Metallic, roughness and glow need 3D lighting; in 2D they rest
            until a custom effect is switched on.
          </p>
        </template>

        <template v-else-if="component.component === 'Emitter'">
          <div class="panel-row">
            <label>Rate /s</label>
            <input type="number" step="any" min="0" max="240" :value="emitterOf(component).rate" @change="e => writeEmitter(component, { rate: num(e, 24) })">
          </div>
          <div class="panel-row">
            <label>Life s</label>
            <input type="number" step="any" min="0.05" max="10" :value="emitterOf(component).lifetime" @change="e => writeEmitter(component, { lifetime: num(e, 0.8) })">
          </div>
          <div class="panel-row">
            <label>Speed</label>
            <input type="number" step="any" min="0" :value="emitterOf(component).speed" @change="e => writeEmitter(component, { speed: num(e, 120) })">
          </div>
          <div class="panel-row">
            <label>Spread</label>
            <input type="number" step="any" min="0" max="360" :value="emitterOf(component).spread" @change="e => writeEmitter(component, { spread: num(e, 60) })">
          </div>
          <div class="panel-row">
            <label>Gravity ×</label>
            <input type="number" step="any" min="0" max="4" :value="emitterOf(component).gravity_scale" @change="e => writeEmitter(component, { gravity_scale: num(e, 0.5) })">
          </div>
          <div class="panel-row">
            <label>Size</label>
            <input type="number" step="any" :value="emitterOf(component).size_start" title="At birth" @change="e => writeEmitter(component, { size_start: num(e, 6) })">
            <input type="number" step="any" :value="emitterOf(component).size_end" title="At death" @change="e => writeEmitter(component, { size_end: num(e, 1) })">
          </div>
          <div class="panel-row">
            <label>Color</label>
            <input type="color" :value="emitterOf(component).color_start" title="At birth" @change="e => writeEmitter(component, { color_start: text(e).toUpperCase() })">
            <input type="color" :value="emitterOf(component).color_end" title="At death" @change="e => writeEmitter(component, { color_end: text(e).toUpperCase() })">
          </div>
          <div class="panel-row">
            <label>Max</label>
            <input type="number" step="1" min="1" max="512" :value="emitterOf(component).max" @change="e => writeEmitter(component, { max: Math.round(num(e, 128)) })">
          </div>
          <p class="panel-note">
            Runs while attached - detaching the emitter stops the spray, and
            what is already flying fades out on its own.
          </p>
        </template>

        <template v-else-if="component.component === 'Trail'">
          <div class="panel-row">
            <label>Every s</label>
            <input type="number" step="any" min="0.016" max="1" :value="trailOf(component).interval" @change="e => writeTrail(component, { interval: num(e, 0.05) })">
          </div>
          <div class="panel-row">
            <label>Lasts s</label>
            <input type="number" step="any" min="0.05" max="5" :value="trailOf(component).life" @change="e => writeTrail(component, { life: num(e, 0.4) })">
          </div>
          <div class="panel-row">
            <label>Color</label>
            <input type="color" :value="trailOf(component).color" @change="e => writeTrail(component, { color: text(e).toUpperCase() })">
          </div>
        </template>
      </template>

      <div class="panel-row component-add">
        <AppDropdown
          v-if="addOpen"
          :options="addableComponents"
          model-value=""
          placeholder="Pick a component"
          @update:model-value="add"
        />
        <button v-else class="component-add-button" @click="addOpen = true">
          <Plus :size="13" /> Add component
        </button>
      </div>
    </template>

    <div class="panel-resize-handle" @pointerdown="e => beginPanelResize('right', e)" />

    <ScriptDialog
      v-if="editingScript"
      :actor-id="editingScript.actorId"
      :actor-name="editingScript.actorName"
      :path="editingScript.path"
      @close="editingScript = null"
    />

    <button v-if="!panels.right.open" class="panel-rail" title="Show the components" @click="setPanelOpen('right', true)">
      <ChevronLeft :size="16" />
    </button>
  </aside>
</template>
