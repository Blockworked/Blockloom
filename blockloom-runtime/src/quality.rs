//! One quality controller for both the editor view and shipped players.
use crate::engine::Engine;
use crate::world::WorldCamera;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::sync_world::RenderEntity;
use bevy::render::view::{ExtractedView, Msaa};
use bevy::render::{Extract, ExtractSchedule, RenderApp};
pub use blockloom_core::quality::DrawCost;
use blockloom_core::quality::{Budget, Controller, GeometryController, Settings, Upscaler};

#[derive(Resource)]
pub struct Scaling {
    pub settings: Settings,
    pub controller: Controller,
    pub geometry: GeometryController,
    overrides: Vec<(blockloom_core::quality::Setting, String)>,
    dropped: bool,
    pub draw_calls: u32,
    pub triangles: u64,
    pub costs: [DrawCost; 6],
}
pub const SYSTEMS: [&str; 6] = ["terrain", "vegetation", "props", "vfx", "debris", "water"];

impl Default for Scaling {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            controller: Controller::new(&settings),
            geometry: GeometryController::default(),
            settings,
            overrides: Vec::new(),
            dropped: false,
            draw_calls: 0,
            triangles: 0,
            costs: [DrawCost::default(); 6],
        }
    }
}
impl Scaling {
    pub fn sample(&self) -> blockloom_core::quality::Sample {
        blockloom_core::quality::Sample {
            frame_ms: self.controller.frame_ms as f64,
            draw_calls: self.draw_calls,
            quality: self.controller.quality,
            dlss_available: false,
        }
    }
    pub fn particle_budget(&self, authored: u32) -> u32 {
        (authored.min(self.budget().particles) as f32 * self.geometry.factors[3]) as u32
    }
    pub fn shard_budget(&self, authored: usize) -> usize {
        (authored.min(self.budget().shards) as f32 * self.geometry.factors[4]) as usize
    }
    pub fn budget(&self) -> Budget {
        self.controller.quality.budget()
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<Scaling>()
        .add_systems(PreUpdate, feedback)
        .add_systems(
            Update,
            reset
                .after(crate::world::pump_editor)
                .before(crate::world::rebuild_world),
        )
        .add_systems(
            FixedUpdate,
            (
                fire_drop.before(crate::world::step_vm),
                apply_effects
                    .after(crate::world::apply_common)
                    .before(crate::world::clear_effects),
            )
                .in_set(crate::world::SimulationSet),
        )
        .add_systems(
            PostUpdate,
            configure_cameras.after(crate::environment::apply_environment),
        )
        .add_systems(Last, measure_draws);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(
            ExtractSchedule,
            scale_views.after(bevy::render::camera::extract_cameras),
        );
    }
}

fn feedback(
    engine: NonSend<Engine>,
    diagnostics: Option<Res<DiagnosticsStore>>,
    pace: Option<Res<crate::performance::LoopPace>>,
    warmup: Option<Res<crate::streaming::Warmup>>,
    mut scaling: ResMut<Scaling>,
    lod: Option<ResMut<crate::culling::LodPolicy>>,
    batches: Option<ResMut<crate::batching::BatchPolicy>>,
) {
    let mut settings = engine.project.world.quality.clone();
    if engine.rebuild {
        scaling.overrides.clear();
        scaling.dropped = false;
    }
    for (key, value) in &scaling.overrides {
        let _ = settings.set(*key, value);
    }
    settings.normalize();
    if settings != scaling.settings || engine.rebuild {
        scaling.controller = Controller::new(&settings);
        scaling.geometry = GeometryController::default();
        scaling.settings = settings.clone();
    }
    if !engine.rebuild && !engine.paused && !warmup.is_some_and(|w| w.active()) {
        let gpu = diagnostics
            .as_ref()
            .and_then(|d| d.get(&crate::gpu::FRAME_GPU))
            .and_then(|d| d.value())
            .unwrap_or(0.0);
        // Work time excludes the embedded view's presentation wait.
        let cpu = pace.map_or(0.0, |p| p.update_ms.max(p.main_ms));
        let limits = scaling.budget();
        let costs = scaling.costs;
        scaling.geometry.sample(&settings, &limits, &costs);
        let geometry_over = scaling.geometry.needs_preset_drop(&limits, &costs);
        let dropped =
            scaling
                .controller
                .sample_with_cost(&settings, gpu.max(cpu) as f32, geometry_over);
        scaling.dropped |= dropped && engine.running;
    }
    let budget = scaling.budget();
    if let Some(mut lod) = lod {
        lod.bias = budget.distance;
    }
    if let Some(mut batches) = batches {
        batches.instance_threshold = 2;
    }
}

fn configure_cameras(
    mut commands: Commands,
    scaling: Res<Scaling>,
    environment: Res<crate::environment::Environment>,
    cameras: Query<
        (Entity, Option<&TemporalAntiAliasing>, &mut Msaa),
        (With<WorldCamera>, With<Camera3d>),
    >,
) {
    let temporal = scaling.settings.upscaler != Upscaler::Spatial;
    for (entity, taa, mut msaa) in cameras {
        if temporal {
            *msaa = Msaa::Off;
            if taa.is_none() {
                commands
                    .entity(entity)
                    .insert(TemporalAntiAliasing::default());
            }
        } else if taa.is_some() {
            *msaa = if environment.wants_msaa_off() {
                Msaa::Off
            } else {
                Msaa::default()
            };
            commands.entity(entity).remove::<TemporalAntiAliasing>();
            commands
                .entity(entity)
                .remove::<bevy::render::camera::TemporalJitter>();
        }
    }
}

/// Scale working targets, leaving the output attachment and input coordinates
/// native. Bevy's final spatial blit fills that attachment in both dimensions.
fn scale_views(
    scaling: Extract<Res<Scaling>>,
    worlds: Extract<Query<RenderEntity, With<WorldCamera>>>,
    mut views: Query<(&mut ExtractedCamera, &mut ExtractedView)>,
) {
    let scale = scaling.controller.scale;
    if scale >= 1.0 {
        return;
    }
    for entity in &worlds {
        let Ok((mut camera, mut view)) = views.get_mut(entity) else {
            continue;
        };
        // Sub-viewports need an output rectangle separate from their work size.
        if camera.viewport.is_some() {
            continue;
        }
        let Some(size) = camera.physical_target_size else {
            continue;
        };
        let size = (size.as_vec2() * scale).round().as_uvec2().max(UVec2::ONE);
        camera.physical_target_size = Some(size);
        camera.physical_viewport_size = Some(size);
        view.viewport.z = size.x;
        view.viewport.w = size.y;
    }
}

fn apply_effects(mut scaling: ResMut<Scaling>, effects: Res<crate::engine::PendingEffects>) {
    for effect in &effects.0 {
        if let blockloom_core::vm::Effect::SetRenderSetting { setting, value } = effect {
            let mut settings = scaling.settings.clone();
            if let Err(error) = settings.set(*setting, value) {
                tracing::warn!("{error}");
                continue;
            }
            scaling.overrides.retain(|(key, _)| key != setting);
            scaling.overrides.push((*setting, value.clone()));
            scaling.controller = Controller::new(&settings);
            scaling.geometry = GeometryController::default();
            scaling.settings = settings;
        }
    }
}
fn fire_drop(mut engine: NonSendMut<Engine>, mut scaling: ResMut<Scaling>) {
    if engine.running && !engine.paused && scaling.dropped {
        scaling.dropped = false;
        engine.fire(blockloom_core::vm::Event::QualityDropped);
    }
}

/// Visible mesh/material groups, excluding shadow and post passes. Transparent
/// and custom materials count separately because their ordering can split draws.
pub(crate) fn measure_draws(
    mut every: Local<u32>,
    mut triangles_by_mesh: Local<std::collections::HashMap<AssetId<Mesh>, u64>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut scaling: ResMut<Scaling>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
    query: Query<(
        Entity,
        &Mesh3d,
        &ViewVisibility,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<crate::batching::InstancedMaterial>>,
        Has<crate::terrain::TerrainChunk>,
        Has<crate::terrain::vegetation::GrassCell>,
        Has<crate::terrain::vegetation::ScatterInstance>,
        Has<MeshMaterial3d<crate::vfx::ParticleMaterial>>,
        Has<crate::destruction::Shard>,
        Has<crate::water::WaterSurface>,
    )>,
    parents: Query<&ChildOf>,
    scatter_roots: Query<(), With<crate::terrain::vegetation::ScatterInstance>>,
    shard_roots: Query<(), With<crate::destruction::Shard>>,
) {
    // Capture counts before extraction moves render-only vertex data to the GPU.
    for event in mesh_events.read() {
        match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id } => {
                if let Some(count) = meshes.get(*id).and_then(triangle_count) {
                    triangles_by_mesh.insert(*id, count);
                }
            }
            AssetEvent::Removed { id } => {
                triangles_by_mesh.remove(id);
            }
            AssetEvent::Unused { .. } => {}
        }
    }
    *every = (*every + 1) % 30;
    if *every != 1 {
        return;
    }
    let mut groups = std::collections::HashSet::new();
    let mut triangles = 0;
    let mut costs = [DrawCost::default(); 6];
    for (entity, mesh, visible, standard, instanced, terrain, grass, scatter, vfx, debris, water) in
        &query
    {
        if !visible.get() {
            continue;
        }
        let Some(geometry) = meshes.get(&mesh.0) else {
            continue;
        };
        let mut ancestor = entity;
        let mut vegetation = grass || scatter;
        let mut debris = debris;
        while !vegetation
            && !debris
            && let Ok(parent) = parents.get(ancestor)
        {
            ancestor = parent.parent();
            vegetation |= scatter_roots.contains(ancestor);
            debris |= shard_roots.contains(ancestor);
        }
        let system = if terrain {
            0
        } else if vegetation {
            1
        } else if vfx {
            3
        } else if debris {
            4
        } else if water {
            5
        } else {
            2
        };
        let mesh_triangles = triangle_count(geometry)
            .or_else(|| triangles_by_mesh.get(&mesh.0.id()).copied())
            .unwrap_or(0);
        triangles += mesh_triangles;
        costs[system].triangles += mesh_triangles;
        let key = if let Some(material) = instanced {
            (mesh.0.id(), Some(material.0.id().untyped()), None)
        } else if let Some(material) = standard.filter(|m| {
            materials
                .get(&m.0)
                .is_some_and(|m| matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)))
        }) {
            (mesh.0.id(), Some(material.0.id().untyped()), None)
        } else {
            (mesh.0.id(), None, Some(entity))
        };
        if groups.insert((system, key)) {
            costs[system].draws += 1;
        }
    }
    scaling.draw_calls = groups.len() as u32;
    scaling.triangles = triangles;
    scaling.costs = costs;
}

fn triangle_count(mesh: &Mesh) -> Option<u64> {
    let vertices = match mesh.try_indices_option().ok()? {
        Some(indices) => indices.len(),
        None => mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()?.len(),
    };
    Some(vertices as u64 / 3)
}

fn reset(engine: NonSend<Engine>, mut scaling: ResMut<Scaling>) {
    if engine.rebuild {
        let mut settings = engine.project.world.quality.clone();
        settings.normalize();
        scaling.controller = Controller::new(&settings);
        scaling.geometry = GeometryController::default();
        scaling.settings = settings;
        scaling.overrides.clear();
        scaling.dropped = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::Indices;
    use bevy::render::render_resource::PrimitiveTopology;
    use blockloom_core::quality::{Quality, Setting};
    use blockloom_core::vm::Effect;

    #[test]
    fn local_feedback_waits_while_paused_and_preserves_pixel_rate() {
        let (_, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, blockloom_core::scene::Mode::ThreeD);
        engine.rebuild = false;
        engine.paused = true;
        engine.project.world.quality.auto_drop = true;
        engine.project.world.quality.dynamic_resolution = true;
        engine.project.world.quality.over_budget_frames = 15;
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<Scaling>()
            .init_resource::<crate::performance::LoopPace>()
            .add_systems(Update, feedback);
        app.world_mut()
            .resource_mut::<crate::performance::LoopPace>()
            .update_ms = 5.0;
        app.world_mut().resource_mut::<Scaling>().costs[1].draws = 10_000;
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(app.world().resource::<Scaling>().geometry.factors, [1.0; 6]);
        app.world_mut().non_send_mut::<Engine>().paused = false;
        for _ in 0..15 {
            app.update();
        }
        let scaling = app.world().resource::<Scaling>();
        assert_eq!(scaling.geometry.factors, [1.0, 0.9, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(scaling.controller.quality, Quality::High);
        assert_eq!(scaling.controller.scale, 1.0);
    }

    #[test]
    fn transient_limits_combine_authored_caps_presets_and_local_density() {
        let mut scaling = Scaling::default();
        scaling.geometry.factors[3] = 0.5;
        assert_eq!(scaling.particle_budget(100), 50);
        assert_eq!(scaling.particle_budget(u32::MAX), 100_000);
        assert_eq!(scaling.shard_budget(256), 256);
        scaling.geometry.factors[4] = 0.25;
        assert_eq!(scaling.shard_budget(100), 25);
        assert_eq!(scaling.shard_budget(usize::MAX), 64);
        scaling.controller.quality = Quality::Low;
        assert_eq!(scaling.particle_budget(u32::MAX), 12_500);
        assert_eq!(scaling.shard_budget(usize::MAX), 16);
        assert_eq!(scaling.particle_budget(0), 0);
        assert_eq!(scaling.shard_budget(0), 0);
    }

    #[test]
    fn explicit_quality_restores_an_automatically_lowered_preset() {
        let mut app = App::new();
        app.init_resource::<Scaling>()
            .insert_resource(crate::engine::PendingEffects(vec![
                Effect::SetRenderSetting {
                    setting: Setting::Quality,
                    value: "High".into(),
                },
            ]))
            .add_systems(Update, apply_effects);
        app.world_mut().resource_mut::<Scaling>().controller.quality = Quality::Low;
        app.world_mut().resource_mut::<Scaling>().geometry.factors = [0.25; 6];
        app.update();
        assert_eq!(
            app.world().resource::<Scaling>().controller.quality,
            Quality::High
        );
        assert_eq!(app.world().resource::<Scaling>().geometry.factors, [1.0; 6]);
    }

    #[test]
    fn rebuild_discards_run_overrides() {
        let (_, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, blockloom_core::scene::Mode::ThreeD);
        engine.rebuild = true;
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<Scaling>()
            .add_systems(Update, reset);
        {
            let mut scaling = app.world_mut().resource_mut::<Scaling>();
            scaling.overrides.push((Setting::Quality, "Low".into()));
            scaling.controller.quality = Quality::Low;
            scaling.geometry.factors = [0.25; 6];
            scaling.dropped = true;
        }
        app.update();
        let scaling = app.world().resource::<Scaling>();
        assert!(scaling.overrides.is_empty());
        assert!(!scaling.dropped);
        assert_eq!(scaling.controller.quality, Quality::High);
        assert_eq!(scaling.geometry.factors, [1.0; 6]);
    }

    #[test]
    fn scattered_model_parts_keep_vegetation_costs_after_upload() {
        let mut app = App::new();
        app.init_resource::<Scaling>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_message::<AssetEvent<Mesh>>()
            .add_systems(Update, measure_draws);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 6]),
        );
        let root = app.world_mut().spawn_empty().id();
        let holder = app
            .world_mut()
            .spawn((
                crate::terrain::vegetation::ScatterInstance {
                    levels: Vec::new(),
                    parts: Vec::new(),
                },
                ChildOf(root),
            ))
            .id();
        let nested = app.world_mut().spawn(ChildOf(holder)).id();
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            ViewVisibility::VISIBLE,
            ChildOf(nested),
        ));
        app.world_mut()
            .spawn((Mesh3d(mesh.clone()), ViewVisibility::VISIBLE, ChildOf(root)));
        app.world_mut()
            .write_message(AssetEvent::<Mesh>::Added { id: mesh.id() });
        app.update();
        let scaling = app.world().resource::<Scaling>();
        assert_eq!(scaling.costs[1].draws, 1);
        assert_eq!(scaling.costs[1].triangles, 2);
        assert_eq!(scaling.costs[2].draws, 1);
        assert_eq!(scaling.costs[2].triangles, 2);

        let _gpu_mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .get_mut(&mesh)
            .unwrap()
            .take_gpu_data()
            .unwrap();
        app.world_mut()
            .write_message(AssetEvent::<Mesh>::Modified { id: mesh.id() });
        for _ in 0..30 {
            app.update();
        }
        let scaling = app.world().resource::<Scaling>();
        assert_eq!(scaling.costs[1].triangles, 2);
        assert_eq!(scaling.costs[2].triangles, 2);
    }

    #[test]
    fn render_only_meshes_do_not_panic_the_draw_sampler() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0]; 4])
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 2, 3, 0]));
        assert_eq!(triangle_count(&mesh), Some(2));

        let _gpu_mesh = mesh.take_gpu_data().unwrap();
        assert_eq!(triangle_count(&mesh), None);
    }
}
