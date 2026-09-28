//! GPU-side indirect-draw counters.
//!
//! Bevy bins every 3D mesh into per-view render phases and draws each bin
//! with one direct draw or, where the adapter supports it, one indirect or
//! multi-draw call. Those bins are what the GPU actually executes, so they
//! are counted here in the render world, after batching, and the totals are
//! handed to the main world over a shared snapshot (the same pattern as
//! [`crate::gpu`]). The profiler shows them as `indirect/*`, next to the
//! CPU's `quality/estimated_mesh_draws` estimate, so the two can be held
//! against each other.
//!
//! What each number means:
//! - `api_draws`: draw calls issued for the world cameras (one multi-draw
//!   counts once, like the API counts it).
//! - `multidraw_sets`: how many of those are multidraw sets. Each set holds
//!   at least one indirect command; the exact per-bin command count expands
//!   on the GPU and is not CPU-visible, so `commands` counts one per set as
//!   a lower bound.
//! - `commands`: direct draws plus multidraw sets plus transparent items.
//! - `instances`: meshes absorbed into direct draws. Multidraw instances
//!   live in GPU buffers only, so this is a floor while sets are in use.
//! - `transparent_draws`: sorted transparent items, one draw each.
//! - `gpu`: true once the renderer has counted at least one frame. Before
//!   that (or with no renderer, as in unit tests) the counters mirror the
//!   main world's visible-mesh grouping instead, which is also what keeps
//!   `draws_per_system` filled: bins don't know which system a mesh belongs
//!   to, so per-system membership always comes from the CPU mirror.
//!
//! 2D is left out on purpose: Bevy's sprite batcher already covers it, and
//! the CPU estimate stands there. Shadow, prepass and probe views are left
//! out too: only the world cameras' main phases are counted.

use bevy::core_pipeline::core_3d::{AlphaMask3d, Opaque3d, Transparent3d};
use bevy::core_pipeline::deferred::{AlphaMask3dDeferred, Opaque3dDeferred};
use bevy::prelude::*;
use bevy::render::render_phase::{BinnedPhaseItem, ViewBinnedRenderPhases, ViewSortedRenderPhases};
use bevy::render::sync_world::MainEntity;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::world::{WorldCamera, pump_editor, rebuild_world};

/// What the GPU was asked to draw for the world cameras, main world copy.
/// Totals come from the render world's binned phases once `gpu` is set;
/// `draws_per_system` always comes from the CPU mirror in
/// [`crate::quality::measure_draws`].
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct IndirectDrawCounters {
    pub api_draws: u32,
    pub commands: u32,
    pub instances: u32,
    pub multidraw_sets: u32,
    pub transparent_draws: u32,
    /// One draw per shared mesh-plus-material group, by quality system
    /// (terrain, vegetation, props, vfx, debris, water). Filled from the
    /// CPU mirror every time it samples, even under GPU totals.
    pub draws_per_system: [u32; 6],
    /// True once the renderer has counted at least one frame.
    pub gpu: bool,
}

impl IndirectDrawCounters {
    /// Fold the main world's visible-mesh grouping into the counters. The
    /// per-system membership always lands; the totals only land while the
    /// renderer hasn't taken over yet.
    pub fn fill_membership<K>(&mut self, groups: &HashMap<(usize, K), u32>) {
        let mut api = 0;
        let mut instances = 0;
        let mut per_system = [0; 6];
        for ((system, _), count) in groups {
            api += 1;
            instances += count;
            if let Some(slot) = per_system.get_mut(*system) {
                *slot += 1;
            }
        }
        self.draws_per_system = per_system;
        if !self.gpu {
            self.api_draws = api;
            // The mirror has one command per group: what a direct draw per
            // group would issue, and a lower bound once multidraw is on.
            self.commands = api;
            self.instances = instances;
            self.multidraw_sets = 0;
            self.transparent_draws = 0;
        }
    }
}

/// One render frame's counts, handed from the render world to the main
/// world. `frames` tells a first real count (possibly all zeros for an
/// empty scene) apart from no renderer having run yet.
#[derive(Clone, Copy, Default, Debug)]
struct IndirectSnapshot {
    frames: u64,
    api_draws: u32,
    commands: u32,
    instances: u32,
    multidraw_sets: u32,
    transparent_draws: u32,
}

/// Handed between the render world, which counts, and the main world.
#[derive(Resource, Clone, Default)]
struct IndirectShared(Arc<Mutex<IndirectSnapshot>>);

/// Main-world cameras, as the render world knows them. Binned phases are
/// keyed by [`bevy::render::view::RetainedViewEntity`], whose `main_entity`
/// is matched against this set so shadow, probe and prepass views never
/// inflate the counts.
#[derive(Resource, Default)]
struct RenderWorldCameras(Vec<MainEntity>);

pub fn register(app: &mut App) {
    let shared = IndirectShared::default();
    app.insert_resource(shared.clone())
        .init_resource::<IndirectDrawCounters>()
        .add_systems(PreUpdate, sync_indirect)
        .add_systems(
            Update,
            reset_indirect.after(pump_editor).before(rebuild_world),
        );
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .insert_resource(shared)
            .init_resource::<RenderWorldCameras>()
            .add_systems(ExtractSchedule, extract_world_cameras)
            .add_systems(Render, count_render_indirect.in_set(RenderSystems::Cleanup));
    }
}

/// Copy the render world's latest counts into the main resource. Runs every
/// frame; takes over (setting `gpu`) on the first real count.
fn sync_indirect(shared: Res<IndirectShared>, mut counters: ResMut<IndirectDrawCounters>) {
    let snapshot = *shared.0.lock().unwrap();
    if snapshot.frames == 0 {
        return;
    }
    counters.api_draws = snapshot.api_draws;
    counters.commands = snapshot.commands;
    counters.instances = snapshot.instances;
    counters.multidraw_sets = snapshot.multidraw_sets;
    counters.transparent_draws = snapshot.transparent_draws;
    counters.gpu = true;
}

/// A rebuild starts a new authored world. Old GPU counts belong to the old
/// one, so drop them; the renderer and the CPU mirror refill from zero.
fn reset_indirect(
    engine: NonSend<crate::engine::Engine>,
    mut counters: ResMut<IndirectDrawCounters>,
) {
    if engine.rebuild {
        *counters = IndirectDrawCounters::default();
    }
}

/// Remember which main-world entities are world cameras, so the render
/// counter can skip every other view.
fn extract_world_cameras(
    cameras: Extract<Query<Entity, With<WorldCamera>>>,
    mut views: ResMut<RenderWorldCameras>,
) {
    views.0 = cameras.iter().map(MainEntity::from).collect();
}

/// Count one binned phase's bins for the world cameras. Each multidrawable
/// batch set is one multidraw API call; each batchable bin and each
/// unbatchable entity is one direct draw.
fn count_binned<BPI: BinnedPhaseItem>(
    phases: &ViewBinnedRenderPhases<BPI>,
    allowed: Option<&HashSet<MainEntity>>,
    snapshot: &mut IndirectSnapshot,
) {
    for (retained, phase) in phases.0.iter() {
        if allowed.is_some_and(|set| !set.contains(&retained.main_entity)) {
            continue;
        }
        let sets = phase.multidrawable_meshes.len() as u32;
        snapshot.multidraw_sets += sets;
        snapshot.api_draws += sets;
        // One command per set is a lower bound: the set's bins expand on
        // the GPU, where their count is not CPU-visible.
        snapshot.commands += sets;
        for (_, bin) in phase.batchable_meshes.iter() {
            let members = bin.entities().len() as u32;
            snapshot.api_draws += 1;
            snapshot.commands += 1;
            snapshot.instances += members;
        }
        for (_, unbatchable) in phase.unbatchable_meshes.iter() {
            let members = unbatchable.entities.len() as u32;
            snapshot.api_draws += members;
            snapshot.commands += members;
            snapshot.instances += members;
        }
        for (_, items) in phase.non_mesh_items.iter() {
            let members = items.entities.len() as u32;
            snapshot.api_draws += members;
            snapshot.commands += members;
        }
    }
}

/// Count the sorted transparent phase: one draw per item.
fn count_transparent(
    phases: &ViewSortedRenderPhases<Transparent3d>,
    allowed: Option<&HashSet<MainEntity>>,
    snapshot: &mut IndirectSnapshot,
) {
    for (retained, phase) in phases.0.iter() {
        if allowed.is_some_and(|set| !set.contains(&retained.main_entity)) {
            continue;
        }
        let items = (phase.items.len() + phase.transient_items.len()) as u32;
        snapshot.transparent_draws += items;
        snapshot.api_draws += items;
        snapshot.commands += items;
        snapshot.instances += items;
    }
}

/// Count what the GPU was asked to draw this frame, after batching. Runs at
/// the end of the render schedule, so the bins still hold this frame's
/// queues; the next frame's extract hands the snapshot to the main world.
#[allow(clippy::too_many_arguments)]
fn count_render_indirect(
    opaque: Option<Res<ViewBinnedRenderPhases<Opaque3d>>>,
    mask: Option<Res<ViewBinnedRenderPhases<AlphaMask3d>>>,
    opaque_deferred: Option<Res<ViewBinnedRenderPhases<Opaque3dDeferred>>>,
    mask_deferred: Option<Res<ViewBinnedRenderPhases<AlphaMask3dDeferred>>>,
    transparent: Option<Res<ViewSortedRenderPhases<Transparent3d>>>,
    cameras: Option<Res<RenderWorldCameras>>,
    shared: Res<IndirectShared>,
) {
    let mut snapshot = IndirectSnapshot::default();
    let allowed = cameras.map(|cameras| cameras.0.iter().copied().collect::<HashSet<_>>());
    if let Some(phases) = opaque.as_deref() {
        count_binned(phases, allowed.as_ref(), &mut snapshot);
    }
    if let Some(phases) = mask.as_deref() {
        count_binned(phases, allowed.as_ref(), &mut snapshot);
    }
    // Under ray tracing the surfaces draw deferred instead of forward; the
    // forward phases are then empty, so summing both never double counts.
    if let Some(phases) = opaque_deferred.as_deref() {
        count_binned(phases, allowed.as_ref(), &mut snapshot);
    }
    if let Some(phases) = mask_deferred.as_deref() {
        count_binned(phases, allowed.as_ref(), &mut snapshot);
    }
    if let Some(phases) = transparent.as_deref() {
        count_transparent(phases, allowed.as_ref(), &mut snapshot);
    }
    snapshot.frames = shared.0.lock().unwrap().frames + 1;
    *shared.0.lock().unwrap() = snapshot;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(pairs: &[((usize, &'static str), u32)]) -> HashMap<(usize, &'static str), u32> {
        pairs.iter().cloned().collect()
    }

    #[test]
    fn fallback_groups_become_one_draw_each() {
        let mut counters = IndirectDrawCounters::default();
        counters.fill_membership(&groups(&[
            ((2, "crate"), 10),
            ((2, "barrel"), 4),
            ((1, "grass"), 64),
        ]));
        assert!(!counters.gpu);
        assert_eq!(counters.api_draws, 3);
        assert_eq!(counters.commands, 3);
        assert_eq!(counters.instances, 78);
        assert_eq!(counters.draws_per_system[2], 2);
        assert_eq!(counters.draws_per_system[1], 1);
        assert_eq!(counters.draws_per_system[0], 0);
    }

    #[test]
    fn membership_keeps_arriving_after_the_gpu_takes_the_totals() {
        let mut counters = IndirectDrawCounters::default();
        counters.fill_membership(&groups(&[((2, "crate"), 10)]));
        // The renderer's first count takes over the totals.
        counters.api_draws = 2;
        counters.commands = 5;
        counters.instances = 40;
        counters.multidraw_sets = 2;
        counters.gpu = true;
        // The next CPU sample refreshes membership but keeps GPU totals.
        counters.fill_membership(&groups(&[((2, "crate"), 10), ((5, "lake"), 1)]));
        assert_eq!((counters.api_draws, counters.commands), (2, 5));
        assert_eq!(counters.instances, 40);
        assert_eq!(counters.multidraw_sets, 2);
        assert_eq!(counters.draws_per_system[2], 1);
        assert_eq!(counters.draws_per_system[5], 1);
    }

    #[test]
    fn empty_groups_clear_the_mirror_but_keep_gpu_totals() {
        let mut counters = IndirectDrawCounters::default();
        counters.fill_membership(&groups(&[((0, "chunk"), 3)]));
        counters.gpu = true;
        counters.api_draws = 7;
        counters.fill_membership(&HashMap::<(usize, &'static str), u32>::new());
        assert_eq!(counters.api_draws, 7);
        assert_eq!(counters.draws_per_system, [0; 6]);
    }

    #[test]
    fn measure_draws_fills_the_cpu_mirror() {
        use bevy::asset::RenderAssetUsages;
        use bevy::camera::visibility::VisibleEntities;
        use bevy::render::render_resource::PrimitiveTopology;
        let mut app = App::new();
        app.init_resource::<crate::quality::Scaling>()
            .init_resource::<IndirectDrawCounters>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_message::<AssetEvent<Mesh>>()
            .add_systems(Update, crate::quality::measure_draws);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 6]),
        );
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let first = app
            .world_mut()
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d::<StandardMaterial>(material.clone()),
            ))
            .id();
        let second = app
            .world_mut()
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d::<StandardMaterial>(material.clone()),
            ))
            .id();
        let mut visible = VisibleEntities::default();
        visible
            .get_mut(std::any::TypeId::of::<Mesh3d>())
            .extend([first, second]);
        app.world_mut()
            .spawn((WorldCamera, Camera::default(), visible));
        app.update();
        let counters = app.world().resource::<IndirectDrawCounters>();
        // One shared mesh-plus-material group: one draw for two instances,
        // filed under props (system 2), with no renderer in sight.
        assert!(!counters.gpu);
        assert_eq!(counters.api_draws, 1);
        assert_eq!(counters.commands, 1);
        assert_eq!(counters.instances, 2);
        assert_eq!(counters.draws_per_system[2], 1);
    }

    #[test]
    fn sync_takes_the_first_render_count_and_ignores_an_empty_renderer() {
        let mut app = App::new();
        let shared = IndirectShared::default();
        app.insert_resource(shared.clone())
            .init_resource::<IndirectDrawCounters>()
            .add_systems(Update, sync_indirect);
        app.update();
        let counters = app.world().resource::<IndirectDrawCounters>();
        assert!(!counters.gpu);
        assert_eq!(counters.api_draws, 0);
        *shared.0.lock().unwrap() = IndirectSnapshot {
            frames: 1,
            api_draws: 4,
            commands: 9,
            instances: 120,
            multidraw_sets: 2,
            transparent_draws: 1,
        };
        app.update();
        let counters = app.world().resource::<IndirectDrawCounters>();
        assert!(counters.gpu);
        assert_eq!(
            (
                counters.api_draws,
                counters.commands,
                counters.instances,
                counters.multidraw_sets,
                counters.transparent_draws,
            ),
            (4, 9, 120, 2, 1)
        );
    }

    #[test]
    fn rebuild_discards_gpu_counts_for_the_new_world() {
        use blockloom_core::scene::Mode;
        let (_, incoming) = std::sync::mpsc::channel();
        let mut engine = crate::engine::Engine::new(incoming, Mode::ThreeD);
        engine.rebuild = true;
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<IndirectDrawCounters>()
            .add_systems(Update, reset_indirect);
        {
            let mut counters = app.world_mut().resource_mut::<IndirectDrawCounters>();
            counters.api_draws = 50;
            counters.instances = 500;
            counters.draws_per_system[2] = 3;
            counters.gpu = true;
        }
        app.update();
        let counters = app.world().resource::<IndirectDrawCounters>();
        assert_eq!(*counters, IndirectDrawCounters::default());
    }
}
