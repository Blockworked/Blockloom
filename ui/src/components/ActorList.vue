<script setup lang="ts">
// The actors in the world. Selecting one swaps the canvas to its own blocks,
// the way switching sprites does in Scratch.
import { computed } from 'vue';
import { AppDropdown } from 'blockstitch';
import { Copy, Trash2 } from 'lucide-vue-next';
import { mode, state } from '../store';
import { addActor, duplicateActor, removeActor, selectActor } from '../tauri';
import { SHAPES_2D, SHAPES_3D, shapesFor, type ActorDto } from '../types';

const SHAPE_LABELS: Record<string, string> = {
  Rect: 'Square',
  Circle: 'Ball',
  Image: 'Image',
  Cuboid: 'Box',
  Sphere: 'Sphere',
  Capsule: 'Capsule',
  Plane: 'Ground plane',
};

const addOptions = computed(() =>
  shapesFor(mode.value).map(shape => ({ value: shape, label: SHAPE_LABELS[shape] ?? shape })),
);

const actors = computed(() => state.project?.actors ?? []);

/** An actor whose shape belongs to the other dimension can't be drawn - the
 * list says so rather than leaving an invisible actor to puzzle over. */
function wrongDimension(actor: ActorDto): boolean {
  const list = mode.value === 'ThreeD' ? SHAPES_3D : SHAPES_2D;
  return !list.includes(actor.visual.shape);
}

function swatch(actor: ActorDto): Record<string, string> {
  const color = 'color' in actor.visual ? actor.visual.color : '#8e8e93';
  return { background: color };
}

function isRound(actor: ActorDto): boolean {
  return actor.visual.shape === 'Circle' || actor.visual.shape === 'Sphere';
}

function onRemove(actorId: string) {
  const actor = actors.value.find(candidate => candidate.id === actorId);
  if (actor && window.confirm(`Delete actor "${actor.name}"? This cannot be undone.`)) void removeActor(actorId);
}
</script>

<template>
  <aside class="side-panel">
    <div class="panel-heading">
      <span>Actors</span>
      <AppDropdown
        :options="addOptions"
        model-value=""
        placeholder="Add"
        class-name="dd-compact"
        :reset-after-select="true"
        @update:model-value="shape => addActor(shape)"
      />
    </div>
    <button
      v-for="actor in actors"
      :key="actor.id"
      class="actor-row"
      :class="{ selected: actor.id === state.selected_actor, mismatch: wrongDimension(actor) }"
      @click="selectActor(actor.id)"
    >
      <span class="actor-swatch" :class="{ round: isRound(actor) }" :style="swatch(actor)" />
      <span class="actor-name">{{ actor.name }}</span>
      <span v-if="wrongDimension(actor)" class="actor-badge" :title="`A ${actor.visual.shape} can't be drawn in this dimension`">!</span>
      <span v-else-if="actor.physics.body !== 'None'" class="actor-badge">{{ actor.physics.body[0] }}</span>
    </button>
    <div class="panel-row" v-if="state.selected_actor">
      <button class="btn-small" title="Duplicate this actor" @click="duplicateActor(state.selected_actor)">
        <Copy />
      </button>
      <button
        class="btn-small"
        title="Delete this actor"
        @click="onRemove(state.selected_actor)"
      >
        <Trash2 />
      </button>
    </div>
  </aside>
</template>
