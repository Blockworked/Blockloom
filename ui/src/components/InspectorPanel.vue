<script setup lang="ts">
// The right-hand panel: the selected actor as a list of components, then the
// world's own settings. Each component is a card with a way to remove it, and
// "Add component" gives the actor one it hasn't got - including a custom one,
// a named bag of values the blocks read and write.
//
// Every field writes straight through to the backend, and a running world
// picks the change up as soon as it stops. The `*Of` helpers narrow the
// component union in one place, so the template stays free of casts.
//
// The rows naming a file - an image, a script - are wrapped in `AssetDrop`,
// so a file dragged out of the asset tray lands on them.
import { computed, ref, watch } from 'vue';
import { AppDropdown, SwitchControl } from 'blockstitch';
import { Lock, LockOpen, Plus, X } from 'lucide-vue-next';
import { mode, openActor, state } from '../store';
import {
  addActorComponent,
  checkScript,
  createScript,
  readAsset,
  removeActorComponent,
  renameActor,
  setActorComponent,
  setBackground,
  setCamera,
  setGravity,
} from '../tauri';
import AssetDrop from './AssetDrop.vue';
import ScriptDialog from './ScriptDialog.vue';
import { BODY_OPTIONS, CAMERA_VIEW_OPTIONS } from '../constants';
import {
  ADDABLE_COMPONENTS,
  actorPhysics,
  actorPlacement,
  actorVisual,
  componentName,
  shapesFor,
  type ActorComponentDto,
  type CameraAttachDto,
  type CameraDto,
  type ComponentFieldDto,
  type ComponentName,
  type EvaluatedDto,
  type PhysicsDto,
  type PlacementDto,
  type VisualDto,
} from '../types';

const actor = computed(() => openActor.value);
const world = computed(() => state.project?.world ?? null);
/** Where the actor is right now, while a run is going - the project's own
 * numbers are where it will start from again. */
const live = computed(() => state.status?.actors.find(a => a.id === actor.value?.id) ?? null);

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
  return component.component === 'Body' ? component.physics : actorPhysics(actor.value);
}

function cameraOf(component: ActorComponentDto): CameraAttachDto {
  return component.component === 'Camera'
    ? component.camera
    : { view: 'Follow', offset: [0, 0.6, 0], distance: 6, pitch: 15 };
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
      return { component: 'Render', visible: true };
    case 'Body':
      return {
        component: 'Body',
        physics: { body: 'Dynamic', gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5 },
      };
    case 'Camera':
      return {
        component: 'Camera',
        camera: { view: 'ThirdPerson', offset: [0, 0.6, 0], distance: 6, pitch: 15 },
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

// ─── The world ─────────────────────────────────────────────────────────────

function writeCamera(next: Partial<CameraDto>) {
  if (!world.value) return;
  void setCamera({ ...world.value.camera, ...next });
}

function writeGravity(index: number, value: number) {
  if (!world.value) return;
  const gravity: [number, number, number] = [...world.value.gravity];
  gravity[index] = value;
  void setGravity(gravity);
}
</script>

<template>
  <aside class="side-panel right">
    <template v-if="actor">
      <div class="panel-heading"><span>{{ actor.name }}</span></div>
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

        <template v-else-if="component.component === 'Render'">
          <div class="panel-row">
            <label>Visible</label>
            <SwitchControl
              :model-value="visibleOf(component)"
              @update:model-value="v => write('Render', { component: 'Render', visible: v })"
            />
          </div>
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
              <label>Upright</label>
              <SwitchControl
                :model-value="physicsOf(component).lock_rotation"
                @update:model-value="v => writePhysics(component, { lock_rotation: v })"
              />
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

    <template v-if="world">
      <div class="panel-heading"><span>World</span></div>
      <div class="panel-row">
        <label>Background</label>
        <input type="color" :value="world.background" @change="e => setBackground(text(e).toUpperCase())">
      </div>
      <div class="panel-row triple">
        <label>Gravity</label>
        <input type="number" step="any" :value="world.gravity[0]" @change="e => writeGravity(0, num(e, 0))">
        <input type="number" step="any" :value="world.gravity[1]" @change="e => writeGravity(1, num(e, 0))">
        <input v-if="mode === 'ThreeD'" type="number" step="any" :value="world.gravity[2]" @change="e => writeGravity(2, num(e, 0))">
      </div>
      <div class="panel-row" v-if="mode === 'TwoD'">
        <label>Zoom</label>
        <input type="number" step="any" :value="world.camera.zoom" @change="e => writeCamera({ zoom: num(e, 1) })">
      </div>
      <div class="panel-row triple" v-else>
        <label>Camera at</label>
        <input
          v-for="(coordinate, i) in world.camera.position"
          :key="i"
          type="number"
          step="any"
          :value="coordinate"
          @change="e => { const position = [...world!.camera.position] as [number, number, number]; position[i] = num(e, coordinate); writeCamera({ position }); }"
        >
      </div>
      <p class="panel-note">
        Where the camera stands when no actor has a Camera component. A 2D unit
        is a pixel and a 3D unit is a metre, which is why the numbers jump when
        you switch dimensions.
      </p>
    </template>

    <ScriptDialog
      v-if="editingScript"
      :actor-id="editingScript.actorId"
      :actor-name="editingScript.actorName"
      :path="editingScript.path"
      @close="editingScript = null"
    />
  </aside>
</template>
