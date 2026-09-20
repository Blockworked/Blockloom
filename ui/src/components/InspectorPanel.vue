<script setup lang="ts">
// The right-hand panel: the selected actor's look, place and body, then the
// world's own settings. Every field writes straight through to the backend, and
// a running world picks the change up as soon as it stops.
import { computed } from 'vue';
import { AppDropdown, SwitchControl } from 'blockstitch';
import { mode, openActor, state } from '../store';
import {
  renameActor,
  setActorPhysics,
  setActorPlacement,
  setActorVisible,
  setActorVisual,
  setBackground,
  setCamera,
  setGravity,
} from '../tauri';
import { BODY_OPTIONS } from '../constants';
import { shapesFor, type CameraDto, type PhysicsDto, type PlacementDto, type VisualDto } from '../types';

const actor = computed(() => openActor.value);
const world = computed(() => state.project?.world ?? null);
/** Where the actor is right now, while a run is going - the project's own
 * numbers are where it will start from again. */
const live = computed(() => state.status?.actors.find(a => a.id === actor.value?.id) ?? null);

const shapeOptions = computed(() => shapesFor(mode.value).map(shape => ({ value: shape, label: shape })));
const followOptions = computed(() => [
  { value: '', label: 'nothing' },
  ...(state.project?.actors ?? []).map(a => ({ value: a.id, label: a.name })),
]);

function num(e: Event, fallback: number): number {
  const parsed = Number((e.target as HTMLInputElement).value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function writePlacement(next: Partial<PlacementDto>) {
  if (!actor.value) return;
  void setActorPlacement(actor.value.id, { ...actor.value.placement, ...next });
}

function writePosition(index: 0 | 1 | 2, value: number) {
  if (!actor.value) return;
  const position: [number, number, number] = [...actor.value.placement.position];
  position[index] = value;
  writePlacement({ position });
}

function writeRotation(index: 0 | 1 | 2, value: number) {
  if (!actor.value) return;
  const rotation: [number, number, number] = [...actor.value.placement.rotation];
  rotation[index] = value;
  writePlacement({ rotation });
}

function writePhysics(next: Partial<PhysicsDto>) {
  if (!actor.value) return;
  void setActorPhysics(actor.value.id, { ...actor.value.physics, ...next });
}

/** Swapping a shape keeps what carries over (its color) and takes sensible
 * defaults for the rest, since a sphere has no width and a box has no radius. */
function writeShape(shape: string) {
  if (!actor.value) return;
  const color = 'color' in actor.value.visual ? actor.value.visual.color : '#4C97FF';
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
  if (next) void setActorVisual(actor.value.id, next);
}

function writeVisual(next: Partial<VisualDto>) {
  if (!actor.value) return;
  void setActorVisual(actor.value.id, { ...actor.value.visual, ...next } as VisualDto);
}

function writeCamera(next: Partial<CameraDto>) {
  if (!world.value) return;
  void setCamera({ ...world.value.camera, ...next });
}

function writeGravity(index: 0 | 1 | 2, value: number) {
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
        <input type="text" :value="actor.name" @change="e => renameActor(actor!.id, (e.target as HTMLInputElement).value)">
      </div>
      <div class="panel-row">
        <label>Shape</label>
        <AppDropdown :options="shapeOptions" :model-value="actor.visual.shape" @update:model-value="writeShape" />
      </div>
      <div class="panel-row" v-if="'color' in actor.visual">
        <label>Color</label>
        <input type="color" :value="actor.visual.color" @change="e => writeVisual({ color: (e.target as HTMLInputElement).value.toUpperCase() } as Partial<VisualDto>)">
      </div>
      <div class="panel-row" v-if="actor.visual.shape === 'Image'">
        <label>Image</label>
        <input
          type="text"
          :value="actor.visual.path"
          placeholder="assets/player.png"
          @change="e => writeVisual({ path: (e.target as HTMLInputElement).value } as Partial<VisualDto>)"
        >
      </div>
      <div class="panel-row" v-if="'radius' in actor.visual">
        <label>Radius</label>
        <input type="number" step="any" :value="actor.visual.radius" @change="e => writeVisual({ radius: num(e, 1) } as Partial<VisualDto>)">
      </div>
      <div class="panel-row" v-if="actor.visual.shape === 'Capsule'">
        <label>Height</label>
        <input type="number" step="any" :value="actor.visual.height" @change="e => writeVisual({ height: num(e, 1) } as Partial<VisualDto>)">
      </div>
      <div class="panel-row triple" v-if="'size' in actor.visual">
        <label>Size</label>
        <input
          v-for="(dimension, i) in actor.visual.size"
          :key="i"
          type="number"
          step="any"
          :value="dimension"
          @change="e => { const size = [...(actor!.visual as { size: number[] }).size]; size[i] = num(e, dimension); writeVisual({ size } as unknown as Partial<VisualDto>); }"
        >
      </div>

      <div class="panel-heading"><span>Place</span></div>
      <div class="panel-row triple">
        <label>Position</label>
        <input type="number" step="any" :value="actor.placement.position[0]" @change="e => writePosition(0, num(e, 0))">
        <input type="number" step="any" :value="actor.placement.position[1]" @change="e => writePosition(1, num(e, 0))">
        <input v-if="mode === 'ThreeD'" type="number" step="any" :value="actor.placement.position[2]" @change="e => writePosition(2, num(e, 0))">
      </div>
      <div class="panel-row triple">
        <label>Rotation</label>
        <input v-if="mode === 'ThreeD'" type="number" step="any" :value="actor.placement.rotation[0]" @change="e => writeRotation(0, num(e, 0))">
        <input v-if="mode === 'ThreeD'" type="number" step="any" :value="actor.placement.rotation[1]" @change="e => writeRotation(1, num(e, 0))">
        <input type="number" step="any" :value="actor.placement.rotation[2]" @change="e => writeRotation(2, num(e, 0))">
      </div>
      <div class="panel-row">
        <label>Size</label>
        <input type="number" step="any" :value="actor.placement.scale" @change="e => writePlacement({ scale: num(e, 1) })">
      </div>
      <div class="panel-row">
        <label>Visible</label>
        <SwitchControl :model-value="actor.visible" @update:model-value="v => setActorVisible(actor!.id, v)" />
      </div>
      <p class="panel-note" v-if="live">
        Now at {{ live.position.map(n => n.toFixed(1)).join(', ') }}
      </p>

      <div class="panel-heading"><span>Body</span></div>
      <div class="panel-row">
        <label>Kind</label>
        <AppDropdown :options="BODY_OPTIONS" :model-value="actor.physics.body" @update:model-value="body => writePhysics({ body: body as PhysicsDto['body'] })" />
      </div>
      <template v-if="actor.physics.body !== 'None'">
        <div class="panel-row">
          <label>Gravity ×</label>
          <input type="number" step="any" :value="actor.physics.gravity_scale" @change="e => writePhysics({ gravity_scale: num(e, 1) })">
        </div>
        <div class="panel-row">
          <label>Bounce</label>
          <input type="number" step="any" :value="actor.physics.restitution" @change="e => writePhysics({ restitution: num(e, 0) })">
        </div>
        <div class="panel-row">
          <label>Friction</label>
          <input type="number" step="any" :value="actor.physics.friction" @change="e => writePhysics({ friction: num(e, 0.5) })">
        </div>
        <div class="panel-row">
          <label>Upright</label>
          <SwitchControl :model-value="actor.physics.lock_rotation" @update:model-value="v => writePhysics({ lock_rotation: v })" />
        </div>
      </template>
    </template>

    <template v-if="world">
      <div class="panel-heading"><span>World</span></div>
      <div class="panel-row">
        <label>Background</label>
        <input type="color" :value="world.background" @change="e => setBackground((e.target as HTMLInputElement).value.toUpperCase())">
      </div>
      <div class="panel-row triple">
        <label>Gravity</label>
        <input type="number" step="any" :value="world.gravity[0]" @change="e => writeGravity(0, num(e, 0))">
        <input type="number" step="any" :value="world.gravity[1]" @change="e => writeGravity(1, num(e, 0))">
        <input v-if="mode === 'ThreeD'" type="number" step="any" :value="world.gravity[2]" @change="e => writeGravity(2, num(e, 0))">
      </div>
      <div class="panel-row">
        <label>Camera on</label>
        <AppDropdown
          :options="followOptions"
          :model-value="world.camera.follow ?? ''"
          @update:model-value="id => writeCamera({ follow: id === '' ? null : id })"
        />
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
        A 2D unit is a pixel and a 3D unit is a metre, which is why the numbers
        jump when you switch dimensions.
      </p>
    </template>
  </aside>
</template>
