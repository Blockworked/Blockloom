//! Ray-traced lighting through Bevy Solari, 3D only.
//!
//! Realtime tracing puts `SolariLighting` on the world camera: the raster
//! passes still draw the G-buffer, and Solari lights it with traced direct
//! light, shadows, bounced light and reflections in place of the deferred
//! lighting pass. Opaque materials therefore go deferred while it is on
//! (`DefaultOpaqueRendererMethod`, every material re-prepared on a switch)
//! and forward again when it is off, so the raster rig is untouched.
//!
//! Solari only traces `StandardMaterial` meshes in its own vertex layout, so
//! the traced world is a copy: one invisible proxy per drawn mesh (no `Mesh3d`,
//! so the raster passes never see it) and one emissive stand-in per traced
//! light, since Solari's only lights are the sun and glowing surfaces. Both
//! are plain top-level entities that follow what they copy after transform
//! propagation.
//!
//! The reference path tracer (`Pathtracer`) is the scene view's, not the
//! game's: it traces the same copy, starts over when anything in it moves,
//! and counts samples towards the view's budget.

// Without the feature only the state and the reports are left.
#![cfg_attr(not(feature = "ray_tracing"), allow(unused_imports, dead_code))]

use crate::batching::{InstanceSlot, InstanceTable, InstancedMaterial, MergedBatch};
use crate::bridge;
use crate::edit::SceneEditor;
use crate::engine::{Dimension, Engine};
use crate::environment::Environment;
use crate::lights::{ActorLight, Lit};
use crate::materials::{BoxMaterial, GraphMaterial3d};
use crate::world::{WorldCamera, WorldLight, parse_color};
use bevy::asset::RenderAssetUsages;
use bevy::camera::{CameraMainTextureUsages, Hdr};
use bevy::core_pipeline::prepass::{
    DeferredPrepass, DeferredPrepassDoubleBuffer, DepthPrepassDoubleBuffer, MotionVectorPrepass,
};
use bevy::light::EnvironmentMapLight;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::DefaultOpaqueRendererMethod;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::renderer::RenderDevice;
use bevy::render::view::Msaa;
use blockloom_core::components::{LightKind, LightSpec};
use blockloom_core::scene::{Mode, RayTracingSettings, TracingMode};
use blockloom_core::vm::Effect;
use blockloom_protocol::{RayTracingStatus, RuntimeMessage};
use std::collections::{HashMap, HashSet};

#[cfg(feature = "ray_tracing")]
use crate::traced::{TracedDenoiser, TracedPaths};
#[cfg(feature = "ray_tracing")]
use bevy::solari::{
    pathtracer::{Pathtracer, PathtracingPlugin},
    prelude::{RaytracingMesh3d, SolariLighting, SolariPlugins},
};

/// Smallest emitter a light stands in as, in metres, so a zero-radius point
/// light isn't infinitely bright.
const MIN_EMITTER_RADIUS: f32 = 0.05;
/// How far a spot's hood reaches down its beam, in emitter radii.
const HOOD_DEPTH: f32 = 4.0;
/// Degrees. A spot wider than this gets no hood, which would barely shade it.
const HOOD_WIDEST: f32 = 75.0;
/// Seconds between path tracer progress reports. Each re-sends the editor's
/// whole state, so not often.
const PROGRESS_EVERY: f32 = 0.5;

pub fn register(app: &mut App, mode: Mode) {
    if mode != Mode::ThreeD {
        app.insert_resource(RayTracingState {
            probed: true,
            reason: "ray tracing is for 3D worlds".into(),
            ..default()
        });
        return;
    }
    app.init_resource::<RayTracingState>();
    app.init_resource::<TracedScene>()
        .init_resource::<PathTraceProgress>()
        .init_resource::<TracedAmbient>()
        .add_systems(
            Update,
            update_traced_ambient.after(crate::environment::blend_environment),
        );
    #[cfg(feature = "ray_tracing")]
    {
        app.add_plugins((
            SolariPlugins,
            PathtracingPlugin,
            crate::traced::TracedPlugin,
        ));
        crate::solari_patch::register(app);
        // Solari asks for deferred everywhere; only a traced camera needs it.
        app.insert_resource(DefaultOpaqueRendererMethod::forward());
    }
}

/// What ray tracing can do here and is doing now. Read by the atmosphere
/// sample, so `is ray tracing on?` answers on the fixed tick.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct RayTracingState {
    probed: bool,
    pub available: bool,
    pub reason: String,
    /// Realtime tracing is on the world camera.
    pub active: bool,
    /// The path tracer is drawing the view.
    pub path_tracing: bool,
}

/// The path tracer's progress since its image last started over.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PathTraceProgress {
    pub samples: u32,
    pub seconds: f32,
    pub converged: bool,
}

/// The traced copy of the world.
#[derive(Resource, Default)]
pub struct TracedScene {
    /// Source mesh entity -> its proxy.
    proxies: HashMap<Entity, Entity>,
    /// Light child entity -> its emitter.
    emitters: HashMap<Entity, Entity>,
    /// Source mesh -> its traceable copy, or `None` for one that can't be.
    meshes: HashMap<AssetId<Mesh>, Option<Handle<Mesh>>>,
    /// Proxy materials for surfaces that aren't a plain standard material.
    materials: HashMap<SurfaceKey, Handle<StandardMaterial>>,
    shapes: Option<EmitterShapes>,
    /// Light child entity -> its spot's hood.
    hoods: HashMap<Entity, Entity>,
    /// Hood meshes by cone angle in degrees.
    hood_meshes: HashMap<u16, Handle<Mesh>>,
    hood_material: Option<Handle<StandardMaterial>>,
    /// Anything in the copy moved, came or went this frame.
    pub changed: bool,
}

/// Unit meshes the emitters are scaled from.
struct EmitterShapes {
    sphere: Handle<Mesh>,
    disk: Handle<Mesh>,
    rect: Handle<Mesh>,
}

/// What a proxy's material is made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SurfaceKey {
    Instanced(AssetId<InstancedMaterial>, [u32; 12]),
    Boxed(AssetId<BoxMaterial>),
    Graph(AssetId<GraphMaterial3d>),
    Plain,
}

/// On a proxy: the mesh entity it copies.
#[derive(Component)]
pub struct TracedProxy {
    mesh: AssetId<Mesh>,
    material: AssetId<StandardMaterial>,
}

/// On a light's emissive stand-in: what it was built from.
#[derive(Component)]
pub struct TracedEmitter(EmitterKey);

/// On a spot emitter's black hood: its cone in degrees. A lambertian disk
/// lights everything in front of it, so the hood keeps it to the beam.
#[derive(Component)]
pub struct TracedHood(u16);

#[derive(Clone, Copy, Debug, PartialEq)]
struct EmitterKey {
    kind: LightKind,
    /// Radius, or a rect's width and height, in metres.
    size: Vec2,
    /// Linear radiance in nits.
    radiance: Vec3,
    /// A spot's hooded cone in degrees, or 0 for no hood.
    cone: u16,
}

/// What the project and this run ask for.
fn wanted(engine: &Engine) -> RayTracingSettings {
    let mut settings = engine
        .project
        .world
        .lighting
        .ray_tracing
        .clone()
        .sanitized();
    if let Some(enabled) = engine.ray_tracing {
        settings.enabled = enabled;
    }
    if let Some(bounces) = engine.gi_bounces {
        settings.bounces = bounces;
    }
    if let Some(samples) = engine.gi_samples {
        settings.samples = samples;
    }
    settings.sanitized()
}

/// `enable ray tracing`, `set GI bounces` and `set GI samples` for the rest
/// of the run. Cleared with everything else live on a rebuild.
pub fn apply_ray_tracing_effects(
    effects: Res<crate::engine::PendingEffects>,
    mut engine: NonSendMut<Engine>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::SetRayTracing { enabled } => engine.ray_tracing = Some(*enabled),
            Effect::SetGiBounces { bounces } if bounces.is_finite() => {
                engine.gi_bounces = Some(bounces.round().clamp(1.0, 64.0) as u32);
            }
            Effect::SetGiSamples { samples } if samples.is_finite() => {
                engine.gi_samples = Some(samples.round().clamp(1.0, 64.0) as u32);
            }
            _ => {}
        }
    }
}

/// Why this GPU and build can't trace, or `None` when they can.
fn unavailable(device: Option<&RenderDevice>, mode: Mode) -> Option<String> {
    if mode != Mode::ThreeD {
        return Some("ray tracing is for 3D worlds".into());
    }
    if !cfg!(feature = "ray_tracing") {
        return Some("this build of Blockloom leaves ray tracing out".into());
    }
    let device = device?;
    #[cfg(feature = "ray_tracing")]
    {
        let missing = SolariPlugins::required_wgpu_features().difference(device.features());
        if !missing.is_empty() {
            return Some(format!(
                "the GPU or its driver can't trace rays ({missing:?})"
            ));
        }
    }
    let _ = device;
    None
}

/// Probes the GPU once, then puts tracing on the world camera or takes it off
/// to match what the project, the run and the scene view ask for.
#[allow(clippy::type_complexity)]
pub fn apply_ray_tracing(
    mut commands: Commands,
    engine: NonSend<Engine>,
    editor: Option<Res<SceneEditor>>,
    dimension: Res<Dimension>,
    device: Option<Res<RenderDevice>>,
    environment: Res<Environment>,
    mut state: ResMut<RayTracingState>,
    (mut method, mut deferred): (ResMut<DefaultOpaqueRendererMethod>, Local<bool>),
    (mut standard, mut instanced): (
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<InstancedMaterial>>,
    ),
    mut warned: Local<bool>,
    #[cfg(feature = "ray_tracing")] cameras: Query<
        (
            Entity,
            (
                Option<&SolariLighting>,
                Option<&TracedPaths>,
                Has<TracedDenoiser>,
                Has<Pathtracer>,
            ),
            &Msaa,
            Has<Hdr>,
            Option<&CameraMainTextureUsages>,
        ),
        With<WorldCamera>,
    >,
    mut suns: Query<&mut DirectionalLight, With<WorldLight>>,
) {
    if !state.probed {
        if device.is_none() && dimension.0 == Mode::ThreeD {
            return;
        }
        state.probed = true;
        let why = unavailable(device.as_deref(), dimension.0);
        state.available = why.is_none();
        state.reason = why.unwrap_or_default();
    }
    let settings = wanted(&engine);
    let path = editor
        .as_ref()
        .is_some_and(|editor| editor.view.path_tracer.enabled);
    if (settings.enabled || path) && !state.available && !*warned {
        *warned = true;
        let text = format!(
            "Ray tracing is off: {}. The raster lighting carries on.",
            state.reason
        );
        warn!("{text}");
        bridge::send(&RuntimeMessage::Say {
            actor: "Blockloom".into(),
            text,
        });
    }
    let path_tracing = path && state.available;
    let active = settings.enabled && state.available && !path_tracing;
    state.set_if_neq(RayTracingState {
        active,
        path_tracing,
        ..state.clone()
    });
    if dimension.0 != Mode::ThreeD {
        return;
    }

    // The G-buffer is what Solari lights and screen-space reflections read,
    // so opaque surfaces go deferred while either is on. Touching every
    // material re-prepares it in the new way.
    let wants_deferred = active || environment.post.ssr.enabled;
    if *deferred != wants_deferred {
        *deferred = wants_deferred;
        *method = if wants_deferred {
            DefaultOpaqueRendererMethod::deferred()
        } else {
            DefaultOpaqueRendererMethod::forward()
        };
        for _ in standard.iter_mut() {}
        for _ in instanced.iter_mut() {}
    }

    // Solari replaces shadow maps with traced shadows.
    for mut sun in &mut suns {
        let maps = !(active || path_tracing);
        if sun.shadow_maps_enabled != maps {
            sun.shadow_maps_enabled = maps;
        }
    }

    #[cfg(feature = "ray_tracing")]
    for (entity, (solari, paths, has_denoiser, has_pathtracer), msaa, has_hdr, usages) in &cameras {
        let mut camera = commands.entity(entity);
        let hybrid = active && settings.mode == TracingMode::Hybrid;
        let path_traced = active && settings.mode == TracingMode::PathTraced;
        let filtered = active && settings.denoiser.filters();
        if active || path_tracing {
            if *msaa != Msaa::Off {
                camera.insert(Msaa::Off);
            }
            if !has_hdr {
                camera.insert(Hdr);
            }
            let usages = usages.copied().unwrap_or_default();
            if !usages.0.contains(TextureUsages::STORAGE_BINDING) {
                camera.insert(usages.with(TextureUsages::STORAGE_BINDING));
            }
        }
        if hybrid {
            let lighting = solari_lighting(&settings, solari);
            if solari.is_none_or(|live| !same_dials(live, &lighting)) {
                camera.insert(lighting);
            }
        } else if solari.is_some() {
            camera.remove::<(
                SolariLighting,
                DeferredPrepassDoubleBuffer,
                DepthPrepassDoubleBuffer,
            )>();
        }
        if path_traced {
            let wanted = TracedPaths {
                paths: settings.paths,
                bounces: settings.bounces,
            };
            if paths != Some(&wanted) {
                camera.insert(wanted);
            }
        } else if paths.is_some() {
            camera.remove::<TracedPaths>();
        }
        if filtered && !has_denoiser {
            camera.insert(TracedDenoiser::default());
        } else if !filtered && has_denoiser {
            camera.remove::<TracedDenoiser>();
        }
        let traced_before = solari.is_some() || paths.is_some() || has_denoiser;
        if !active && traced_before {
            // The depth prepass stays: every 3D world camera has one, for
            // occlusion culling.
            camera.remove::<(
                DeferredPrepass,
                MotionVectorPrepass,
                DeferredPrepassDoubleBuffer,
                DepthPrepassDoubleBuffer,
            )>();
        }
        if path_tracing && !has_pathtracer {
            camera.insert(Pathtracer { reset: true });
        } else if !path_tracing && has_pathtracer {
            camera.remove::<Pathtracer>();
        }
        if !active && !path_tracing && (traced_before || has_pathtracer) {
            // What `apply_environment` would have left. A multisampled
            // target can't be a storage texture, so that usage goes first.
            camera.insert((
                CameraMainTextureUsages::default(),
                if environment.wants_msaa_off() {
                    Msaa::Off
                } else {
                    Msaa::default()
                },
            ));
        }
    }
    #[cfg(not(feature = "ray_tracing"))]
    let _ = (&mut commands, &environment);
}

/// The flat ambient as a one-texel cube, for traced rays that escape a world
/// with no sky: `apply_sky` hangs it on the world camera when the sky has no
/// light of its own, and Solari reads the camera's environment light.
#[derive(Resource, Default)]
pub struct TracedAmbient {
    pub light: Option<EnvironmentMapLight>,
    cube: Option<(Handle<Image>, [u16; 3])>,
}

pub fn update_traced_ambient(
    state: Res<RayTracingState>,
    environment: Res<Environment>,
    mut ambient: ResMut<TracedAmbient>,
    mut images: ResMut<Assets<Image>>,
) {
    if !(state.active || state.path_tracing) {
        if ambient.light.is_some() {
            ambient.light = None;
        }
        return;
    }
    let color = LinearRgba::from(environment.ambient_color);
    let texel =
        [color.red, color.green, color.blue].map(|c| half::f16::from_f32(c.max(0.0)).to_bits());
    let image = || {
        let data = [texel[0], texel[1], texel[2], half::f16::ONE.to_bits()]
            .repeat(6)
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        crate::sky::cube_image(1, 1, data, TextureFormat::Rgba16Float)
    };
    let cube = match &ambient.cube {
        Some((cube, written)) if *written == texel => cube.clone(),
        Some((cube, _)) => {
            let _ = images.insert(cube.id(), image());
            cube.clone()
        }
        None => images.add(image()),
    };
    ambient.cube = Some((cube.clone(), texel));
    let wanted = EnvironmentMapLight {
        diffuse_map: cube.clone(),
        specular_map: cube,
        intensity: environment.ambient_brightness,
        ..default()
    };
    let same = ambient.light.as_ref().is_some_and(|light| {
        light.specular_map == wanted.specular_map && light.intensity == wanted.intensity
    });
    if !same {
        ambient.light = Some(wanted);
    }
}

/// The camera's Solari settings for these project settings, keeping the
/// live one's reset flag so a rewrite doesn't throw its history away.
#[cfg(feature = "ray_tracing")]
fn solari_lighting(settings: &RayTracingSettings, live: Option<&SolariLighting>) -> SolariLighting {
    let base = live.cloned().unwrap_or_default();
    SolariLighting {
        restir: settings.denoiser.reuses(),
        max_bounces: settings.bounces,
        primary_di_samples: settings.samples,
        secondary_di_samples: (settings.samples / 2).max(1),
        world_cache_max_gi_ray_distance: settings.gi_distance,
        ..base
    }
}

/// Whether two Solari settings differ in anything this module sets.
#[cfg(feature = "ray_tracing")]
fn same_dials(a: &SolariLighting, b: &SolariLighting) -> bool {
    a.restir == b.restir
        && a.max_bounces == b.max_bounces
        && a.primary_di_samples == b.primary_di_samples
        && a.secondary_di_samples == b.secondary_di_samples
        && a.world_cache_max_gi_ray_distance == b.world_cache_max_gi_ray_distance
}

/// Keeps the traced copy of the world in step with what the raster passes
/// draw: proxies for meshes, emitters for lights. Runs after transform
/// propagation, so a copy stands exactly where its source is drawn.
#[cfg(feature = "ray_tracing")]
#[allow(clippy::type_complexity)]
pub fn sync_traced_scene(
    mut commands: Commands,
    state: Res<RayTracingState>,
    mut scene: ResMut<TracedScene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    (instanced, boxed, graphs, table): (
        Res<Assets<InstancedMaterial>>,
        Res<Assets<BoxMaterial>>,
        Res<Assets<GraphMaterial3d>>,
        Res<InstanceTable>,
    ),
    (mut mesh_events, mut instanced_events, mut boxed_events): (
        MessageReader<AssetEvent<Mesh>>,
        MessageReader<AssetEvent<InstancedMaterial>>,
        MessageReader<AssetEvent<BoxMaterial>>,
    ),
    sources: Query<
        (
            Entity,
            &Mesh3d,
            &GlobalTransform,
            &InheritedVisibility,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<(&MeshMaterial3d<InstancedMaterial>, &InstanceSlot)>,
            Option<&MeshMaterial3d<BoxMaterial>>,
            Option<&MeshMaterial3d<GraphMaterial3d>>,
        ),
        (
            Without<TracedProxy>,
            Without<TracedEmitter>,
            Without<TracedHood>,
            Without<MergedBatch>,
            Without<crate::streaming::Placeholder>,
            Without<crate::fx::Particle>,
            Without<crate::fx::Ghost>,
            Without<crate::beams::Additive>,
            Without<crate::water::WaterSurface>,
        ),
    >,
    mut proxies: Query<
        (&mut TracedProxy, &mut Transform, &mut GlobalTransform),
        Without<TracedEmitter>,
    >,
    lights: Query<
        (Entity, &GlobalTransform, &InheritedVisibility, &ChildOf),
        (
            With<ActorLight>,
            Without<TracedProxy>,
            Without<TracedEmitter>,
            Without<TracedHood>,
        ),
    >,
    lit: Query<&Lit>,
    mut emitters: Query<
        (&mut TracedEmitter, &mut Transform, &mut GlobalTransform),
        (Without<TracedProxy>, Without<TracedHood>),
    >,
    mut hoods: Query<
        (&mut TracedHood, &mut Transform, &mut GlobalTransform),
        (Without<TracedProxy>, Without<TracedEmitter>),
    >,
) {
    scene.changed = false;
    let tracing = state.active || state.path_tracing;
    if !tracing {
        if !scene.proxies.is_empty() || !scene.emitters.is_empty() {
            let scene = &mut *scene;
            let copies: Vec<Entity> = scene
                .proxies
                .drain()
                .chain(scene.emitters.drain())
                .chain(scene.hoods.drain())
                .map(|(_, copy)| copy)
                .collect();
            for copy in copies {
                commands.entity(copy).try_despawn();
            }
            scene.meshes.clear();
            scene.materials.clear();
            scene.changed = true;
        }
        mesh_events.clear();
        instanced_events.clear();
        boxed_events.clear();
        return;
    }

    for event in mesh_events.read() {
        if let AssetEvent::Modified { id } | AssetEvent::Removed { id } = event {
            scene.meshes.remove(id);
        }
    }
    for event in instanced_events.read() {
        if let AssetEvent::Modified { id } | AssetEvent::Removed { id } = event {
            scene
                .materials
                .retain(|key, _| !matches!(key, SurfaceKey::Instanced(source, _) if source == id));
        }
    }
    for event in boxed_events.read() {
        if let AssetEvent::Modified { id } | AssetEvent::Removed { id } = event {
            scene.materials.remove(&SurfaceKey::Boxed(*id));
        }
    }

    let mut seen = HashSet::new();
    let mut used_meshes = HashSet::new();
    let mut used_materials = HashSet::new();
    for (entity, mesh, transform, visible, plain, instance, projected, graph) in &sources {
        if !visible.get() {
            continue;
        }
        let Some(traced) = traced_mesh(&mut scene, &mut meshes, mesh.id()) else {
            continue;
        };
        used_meshes.insert(mesh.id());
        let material = if let Some(plain) = plain {
            plain.0.clone()
        } else {
            let key = match (instance, projected, graph) {
                (Some((material, slot)), _, _) => {
                    let record = table
                        .record(slot.0)
                        .unwrap_or(crate::batching::InstanceRecord::IDENTITY);
                    let bits = [record.tint, record.uv, record.options]
                        .map(|v| v.to_array().map(f32::to_bits));
                    SurfaceKey::Instanced(material.id(), bytemuck::cast(bits))
                }
                (_, Some(material), _) => SurfaceKey::Boxed(material.id()),
                (_, _, Some(material)) => SurfaceKey::Graph(material.id()),
                _ => SurfaceKey::Plain,
            };
            used_materials.insert(key);
            match scene.materials.get(&key) {
                Some(handle) => handle.clone(),
                None => {
                    let made = proxy_material(key, &instanced, &boxed, &graphs, &table, instance);
                    let handle = standard.add(made);
                    scene.materials.insert(key, handle.clone());
                    handle
                }
            }
        };
        seen.insert(entity);
        let pose = transform.compute_transform();
        match scene.proxies.get(&entity).copied() {
            Some(proxy) => {
                let Ok((mut record, mut local, mut global)) = proxies.get_mut(proxy) else {
                    continue;
                };
                if record.mesh != traced.id() || record.material != material.id() {
                    record.mesh = traced.id();
                    record.material = material.id();
                    commands
                        .entity(proxy)
                        .insert((RaytracingMesh3d(traced), MeshMaterial3d(material)));
                    scene.changed = true;
                }
                if *global != *transform {
                    *local = pose;
                    *global = *transform;
                    scene.changed = true;
                }
            }
            None => {
                let proxy = commands
                    .spawn((
                        TracedProxy {
                            mesh: traced.id(),
                            material: material.id(),
                        },
                        RaytracingMesh3d(traced),
                        MeshMaterial3d(material),
                        pose,
                        *transform,
                        Name::new("traced proxy"),
                    ))
                    .id();
                scene.proxies.insert(entity, proxy);
                scene.changed = true;
            }
        }
    }
    let gone: Vec<Entity> = scene
        .proxies
        .keys()
        .filter(|source| !seen.contains(*source))
        .copied()
        .collect();
    for source in gone {
        if let Some(proxy) = scene.proxies.remove(&source) {
            commands.entity(proxy).try_despawn();
            scene.changed = true;
        }
    }
    scene.meshes.retain(|id, _| used_meshes.contains(id));
    scene
        .materials
        .retain(|key, _| used_materials.contains(key));

    // Lights, as glowing surfaces of the same power.
    let shapes = scene
        .shapes
        .take()
        .unwrap_or_else(|| EmitterShapes::new(&mut meshes));
    let mut lit_now = HashSet::new();
    for (light, transform, visible, parent) in &lights {
        let Ok(Lit { spec, .. }) = lit.get(parent.parent()) else {
            continue;
        };
        if !visible.get() || !spec.ray_traced {
            continue;
        }
        let key = EmitterKey::of(spec);
        let pose = emitter_pose(transform, &key);
        lit_now.insert(light);
        match scene.emitters.get(&light).copied() {
            Some(emitter) => {
                let Ok((mut record, mut local, mut global)) = emitters.get_mut(emitter) else {
                    continue;
                };
                if record.0 != key {
                    record.0 = key;
                    commands.entity(emitter).insert((
                        RaytracingMesh3d(shapes.of(key.kind)),
                        MeshMaterial3d(standard.add(emitter_material(&key))),
                    ));
                    scene.changed = true;
                }
                let placed = GlobalTransform::from(pose);
                if *global != placed {
                    *local = pose;
                    *global = placed;
                    scene.changed = true;
                }
            }
            None => {
                let emitter = commands
                    .spawn((
                        TracedEmitter(key),
                        RaytracingMesh3d(shapes.of(key.kind)),
                        MeshMaterial3d(standard.add(emitter_material(&key))),
                        pose,
                        GlobalTransform::from(pose),
                        Name::new("traced light"),
                    ))
                    .id();
                scene.emitters.insert(light, emitter);
                scene.changed = true;
            }
        }
        sync_hood(
            &mut commands,
            &mut scene,
            &mut meshes,
            &mut standard,
            &mut hoods,
            light,
            &key,
            pose,
        );
    }
    let dark: Vec<Entity> = scene
        .emitters
        .keys()
        .filter(|light| !lit_now.contains(*light))
        .copied()
        .collect();
    for light in dark {
        if let Some(emitter) = scene.emitters.remove(&light) {
            commands.entity(emitter).try_despawn();
            scene.changed = true;
        }
        if let Some(hood) = scene.hoods.remove(&light) {
            commands.entity(hood).try_despawn();
        }
    }
    scene.shapes = Some(shapes);
}

/// Keeps a spot's hood on its emitter, or takes it off a light that no
/// longer wants one.
#[cfg(feature = "ray_tracing")]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_hood(
    commands: &mut Commands,
    scene: &mut TracedScene,
    meshes: &mut Assets<Mesh>,
    standard: &mut Assets<StandardMaterial>,
    hoods: &mut Query<
        (&mut TracedHood, &mut Transform, &mut GlobalTransform),
        (Without<TracedProxy>, Without<TracedEmitter>),
    >,
    light: Entity,
    key: &EmitterKey,
    pose: Transform,
) {
    if key.cone == 0 {
        if let Some(hood) = scene.hoods.remove(&light) {
            commands.entity(hood).try_despawn();
            scene.changed = true;
        }
        return;
    }
    let pose = Transform {
        scale: Vec3::splat(key.size.x),
        ..pose
    };
    let mesh = scene
        .hood_meshes
        .entry(key.cone)
        .or_insert_with(|| meshes.add(hood_mesh(key.cone)))
        .clone();
    match scene.hoods.get(&light).copied() {
        Some(hood) => {
            let Ok((mut record, mut local, mut global)) = hoods.get_mut(hood) else {
                return;
            };
            if record.0 != key.cone {
                record.0 = key.cone;
                commands.entity(hood).insert(RaytracingMesh3d(mesh));
                scene.changed = true;
            }
            let placed = GlobalTransform::from(pose);
            if *global != placed {
                *local = pose;
                *global = placed;
                scene.changed = true;
            }
        }
        None => {
            let material = scene
                .hood_material
                .get_or_insert_with(|| {
                    standard.add(StandardMaterial {
                        base_color: Color::BLACK,
                        perceptual_roughness: 1.0,
                        reflectance: 0.0,
                        ..default()
                    })
                })
                .clone();
            let hood = commands
                .spawn((
                    TracedHood(key.cone),
                    RaytracingMesh3d(mesh),
                    MeshMaterial3d(material),
                    pose,
                    GlobalTransform::from(pose),
                    Name::new("traced spot hood"),
                ))
                .id();
            scene.hoods.insert(light, hood);
            scene.changed = true;
        }
    }
}

/// An open, flared tube around a unit disk facing +Z, reaching `HOOD_DEPTH`
/// down the beam and opening at the cone's angle, black inside and out.
fn hood_mesh(cone: u16) -> Mesh {
    const SEGMENTS: u32 = 24;
    let near = 1.02;
    let slope = (cone as f32).to_radians().tan();
    let far = near + HOOD_DEPTH * slope;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for i in 0..=SEGMENTS {
        let u = i as f32 / SEGMENTS as f32;
        let (sin, cos) = (u * std::f32::consts::TAU).sin_cos();
        let inward = Vec3::new(-cos, -sin, slope).normalize();
        positions.push([near * cos, near * sin, 0.0]);
        positions.push([far * cos, far * sin, HOOD_DEPTH]);
        normals.extend([inward.to_array(); 2]);
        uvs.extend([[u, 0.0], [u, 1.0]]);
    }
    let mut indices = Vec::new();
    for i in 0..SEGMENTS {
        let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
        indices.extend([a, b, c, b, d, c]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    traceable(&mesh).unwrap_or(mesh)
}

/// The traceable copy of a mesh, made on first use. `None` while the source
/// is still loading, or for one Solari can't trace.
fn traced_mesh(
    scene: &mut TracedScene,
    meshes: &mut Assets<Mesh>,
    id: AssetId<Mesh>,
) -> Option<Handle<Mesh>> {
    if let Some(known) = scene.meshes.get(&id) {
        return known.clone();
    }
    let source = meshes.get(id)?;
    let traced = traceable(source).map(|mesh| meshes.add(mesh));
    scene.meshes.insert(id, traced.clone());
    traced
}

/// A mesh in the one layout Solari traces: triangles with positions,
/// normals, UVs and tangents, and 32-bit indices. A skinned or morphed mesh
/// comes out in its bind pose.
pub fn traceable(source: &Mesh) -> Option<Mesh> {
    if source.primitive_topology() != PrimitiveTopology::TriangleList {
        return None;
    }
    let positions = match source.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(positions) => positions.clone(),
        _ => return None,
    };
    if positions.is_empty() {
        return None;
    }
    let count = positions.len();
    let indices: Vec<u32> = match source.indices() {
        Some(indices) => indices.iter().map(|i| i as u32).collect(),
        None => (0..count as u32).collect(),
    };
    if indices.len() < 3 || indices.iter().any(|&i| i as usize >= count) {
        return None;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    match source.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(normals)) if normals.len() == count => {
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone());
        }
        _ => mesh.compute_normals(),
    }
    let uvs = match source.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uvs)) if uvs.len() == count => uvs.clone(),
        _ => vec![[0.0, 0.0]; count],
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    match source.attribute(Mesh::ATTRIBUTE_TANGENT) {
        Some(VertexAttributeValues::Float32x4(tangents)) if tangents.len() == count => {
            mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents.clone());
        }
        _ => {
            if mesh.generate_tangents().is_err() {
                mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0, 0.0, 0.0, 1.0]; count]);
            }
        }
    }
    mesh.enable_raytracing = true;
    Some(mesh)
}

/// A standard material that traces like the surface it stands in for.
fn proxy_material(
    key: SurfaceKey,
    instanced: &Assets<InstancedMaterial>,
    boxed: &Assets<BoxMaterial>,
    graphs: &Assets<GraphMaterial3d>,
    table: &InstanceTable,
    instance: Option<(&MeshMaterial3d<InstancedMaterial>, &InstanceSlot)>,
) -> StandardMaterial {
    match key {
        SurfaceKey::Instanced(id, _) => {
            let Some(material) = instanced.get(id) else {
                return StandardMaterial::default();
            };
            let record = instance
                .and_then(|(_, slot)| table.record(slot.0))
                .unwrap_or(crate::batching::InstanceRecord::IDENTITY);
            let mut base = material.base.clone();
            let tint = base.base_color.to_linear().to_vec4() * record.tint;
            base.base_color = LinearRgba::from_vec4(tint).into();
            base.uv_transform = record.uv_transform();
            base
        }
        SurfaceKey::Boxed(id) => boxed
            .get(id)
            .map(|material| material.base.clone())
            .unwrap_or_default(),
        // An effect is its own light, which tracing has no way to run; its
        // tint is the nearest plain surface.
        SurfaceKey::Graph(id) => StandardMaterial {
            base_color: graphs
                .get(id)
                .map(|material| Color::from(LinearRgba::from_vec4(material.tint)))
                .unwrap_or(Color::WHITE),
            perceptual_roughness: 0.6,
            ..default()
        },
        SurfaceKey::Plain => StandardMaterial {
            perceptual_roughness: 0.6,
            ..default()
        },
    }
}

impl EmitterShapes {
    fn new(meshes: &mut Assets<Mesh>) -> Self {
        let mut shape = |mesh: Mesh| meshes.add(traceable(&mesh).unwrap_or(mesh));
        Self {
            sphere: shape(
                Sphere::new(1.0)
                    .mesh()
                    .ico(2)
                    .unwrap_or_else(|_| Sphere::new(1.0).mesh().uv(16, 8)),
            ),
            disk: shape(Circle::new(1.0).mesh().resolution(24).build()),
            rect: shape(Rectangle::new(1.0, 1.0).mesh().build()),
        }
    }

    fn of(&self, kind: LightKind) -> Handle<Mesh> {
        match kind {
            LightKind::Point => self.sphere.clone(),
            LightKind::Spot | LightKind::Disk => self.disk.clone(),
            LightKind::Rect => self.rect.clone(),
        }
    }
}

impl EmitterKey {
    /// A lambertian emitter giving off the light's power: a sphere for a
    /// point light, and for a spot a disk facing down its beam that is as
    /// bright on its axis as Bevy's spot (whose power isn't gathered into
    /// the cone). Rect and disk lights emit from their front face.
    fn of(spec: &LightSpec) -> Self {
        use std::f32::consts::PI;
        let lumens = spec.lumens().max(0.0);
        let radius = spec.radius.max(MIN_EMITTER_RADIUS);
        let (size, area) = match spec.kind {
            LightKind::Point => (Vec2::splat(radius), 4.0 * PI * radius * radius * PI),
            LightKind::Spot => (Vec2::splat(radius), 4.0 * PI * PI * radius * radius),
            LightKind::Rect => {
                let (width, height) = spec.area_size();
                let size = Vec2::new(width, height).max(Vec2::splat(MIN_EMITTER_RADIUS));
                (size, PI * size.x * size.y)
            }
            LightKind::Disk => {
                let (width, _) = spec.area_size();
                let radius = (0.5 * width).max(MIN_EMITTER_RADIUS);
                (Vec2::splat(radius), PI * PI * radius * radius)
            }
        };
        let color = LinearRgba::from(parse_color(&spec.color));
        let nits = lumens / area;
        let outer = spec.cone().1.to_degrees();
        let cone = if spec.kind == LightKind::Spot && outer < HOOD_WIDEST {
            outer.round().max(1.0) as u16
        } else {
            0
        };
        Self {
            kind: spec.kind,
            size,
            radiance: Vec3::new(color.red, color.green, color.blue) * nits,
            cone,
        }
    }
}

/// Where an emitter stands: at its light, turned with it but not scaled by
/// the actor, and sized by the light itself. Disks and rects face -Z, the
/// way Bevy's spot and rect lights shine.
fn emitter_pose(light: &GlobalTransform, key: &EmitterKey) -> Transform {
    let (_, rotation, translation) = light.to_scale_rotation_translation();
    let (turn, scale) = match key.kind {
        LightKind::Point => (Quat::IDENTITY, Vec3::splat(key.size.x)),
        LightKind::Spot | LightKind::Disk => (
            Quat::from_rotation_y(std::f32::consts::PI),
            Vec3::new(key.size.x, key.size.x, 1.0),
        ),
        LightKind::Rect => (
            Quat::from_rotation_y(std::f32::consts::PI),
            Vec3::new(key.size.x, key.size.y, 1.0),
        ),
    };
    Transform {
        translation,
        rotation: rotation * turn,
        scale,
    }
}

fn emitter_material(key: &EmitterKey) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(key.radiance.x, key.radiance.y, key.radiance.z),
        perceptual_roughness: 1.0,
        ..default()
    }
}

/// Gives an EXR capture camera the world camera's tracing, so the file shows
/// what the view does.
pub fn trace_like(world: &mut World, from: Entity, to: Entity) {
    #[cfg(feature = "ray_tracing")]
    {
        let solari = world.get::<SolariLighting>(from).cloned();
        let path = world.get::<Pathtracer>(from).is_some();
        let Ok(mut capture) = world.get_entity_mut(to) else {
            return;
        };
        if solari.is_some() || path {
            capture.insert((
                Msaa::Off,
                CameraMainTextureUsages::default().with(TextureUsages::STORAGE_BINDING),
            ));
        }
        if let Some(solari) = solari {
            capture.insert(SolariLighting {
                reset: true,
                ..solari
            });
        }
        if path {
            // Its first frame starts over anyway, being new.
            capture.insert(Pathtracer { reset: false });
        }
    }
    #[cfg(not(feature = "ray_tracing"))]
    let _ = (world, from, to);
}

/// Counts the path tracer's samples and starts its image over whenever the
/// traced world or the camera moves.
#[cfg(feature = "ray_tracing")]
pub fn drive_path_tracer(
    state: Res<RayTracingState>,
    scene: Res<TracedScene>,
    editor: Option<Res<SceneEditor>>,
    time: Res<Time<Real>>,
    mut progress: ResMut<PathTraceProgress>,
    mut cameras: Query<(&mut Pathtracer, Ref<GlobalTransform>), With<WorldCamera>>,
) {
    if !state.path_tracing {
        if *progress != PathTraceProgress::default() {
            *progress = PathTraceProgress::default();
        }
        return;
    }
    let Some(budget) = editor.map(|editor| editor.view.path_tracer) else {
        return;
    };
    let mut reset = scene.changed;
    for (pathtracer, transform) in &mut cameras {
        reset |= pathtracer.is_added() || transform.is_changed();
    }
    for (mut pathtracer, _) in &mut cameras {
        if pathtracer.reset != reset {
            pathtracer.reset = reset;
        }
    }
    if reset {
        progress.samples = 0;
        progress.seconds = 0.0;
    }
    progress.samples += 1;
    progress.seconds += time.delta_secs();
    progress.converged = progress.samples >= budget.samples.max(1)
        || (budget.seconds > 0.0 && progress.seconds >= budget.seconds);
}

/// Tells the editor what tracing can do and is doing: on any change, and a
/// few times a second while the path tracer converges.
pub fn report_ray_tracing(
    state: Res<RayTracingState>,
    progress: Option<Res<PathTraceProgress>>,
    time: Res<Time<Real>>,
    mut last: Local<Option<RayTracingStatus>>,
    mut since: Local<f32>,
) {
    if !state.probed {
        return;
    }
    let progress = progress.map(|p| p.clone()).unwrap_or_default();
    let status = RayTracingStatus {
        available: state.available,
        reason: state.reason.clone(),
        active: state.active,
        path_tracing: state.path_tracing,
        samples: progress.samples,
        seconds: progress.seconds,
        converged: progress.converged,
    };
    *since += time.delta_secs();
    let Some(previous) = last.as_ref() else {
        bridge::send(&RuntimeMessage::RayTracing(status.clone()));
        *last = Some(status);
        *since = 0.0;
        return;
    };
    let moved = RayTracingStatus {
        samples: previous.samples,
        seconds: previous.seconds,
        ..status.clone()
    } != *previous;
    let ticking = status.samples != previous.samples && *since >= PROGRESS_EVERY;
    if moved || ticking {
        bridge::send(&RuntimeMessage::RayTracing(status.clone()));
        *last = Some(status);
        *since = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::MeshBuilder;

    #[test]
    fn a_mesh_comes_out_in_the_layout_solari_traces() {
        // Bevy's cube: 16-bit indices, and a UV set too many.
        let mut cube = Cuboid::default().mesh().build();
        cube.insert_attribute(
            Mesh::ATTRIBUTE_UV_1,
            vec![[0.0, 0.0]; cube.count_vertices()],
        );
        let traced = traceable(&cube).unwrap();
        assert!(matches!(traced.indices(), Some(Indices::U32(_))));
        let mut attributes: Vec<_> = traced.attributes().map(|(a, _)| a.name).collect();
        attributes.sort();
        let mut wanted = vec![
            Mesh::ATTRIBUTE_POSITION.name,
            Mesh::ATTRIBUTE_NORMAL.name,
            Mesh::ATTRIBUTE_UV_0.name,
            Mesh::ATTRIBUTE_TANGENT.name,
        ];
        wanted.sort();
        assert_eq!(attributes, wanted);
        assert!(traced.enable_raytracing);
    }

    #[test]
    fn a_mesh_without_normals_or_uvs_still_traces() {
        let mut bare = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        bare.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        let traced = traceable(&bare).unwrap();
        assert_eq!(traced.indices().unwrap().len(), 3);
        assert!(traced.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
        assert!(traced.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
    }

    #[test]
    fn lines_and_empty_meshes_are_left_out() {
        let lines = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
        assert!(traceable(&lines).is_none());
        let empty = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        assert!(traceable(&empty).is_none());
    }

    #[test]
    fn a_point_emitter_gives_off_its_lights_power() {
        use std::f32::consts::PI;
        let spec = LightSpec {
            intensity: 800.0,
            radius: 0.1,
            ..LightSpec::default()
        };
        let key = EmitterKey::of(&spec);
        // A lambertian sphere emits pi * L * area.
        let power = PI * key.radiance.x * 4.0 * PI * key.size.x * key.size.x;
        assert!((power - 800.0).abs() < 0.01, "{power}");
    }

    #[test]
    fn a_narrow_spot_is_hooded_to_its_cone() {
        let spot = LightSpec {
            kind: LightKind::Spot,
            outer_angle: 30.0,
            ..LightSpec::default()
        };
        assert_eq!(EmitterKey::of(&spot).cone, 30);
        let wide = LightSpec {
            outer_angle: 85.0,
            ..spot.clone()
        };
        assert_eq!(EmitterKey::of(&wide).cone, 0);
        assert_eq!(EmitterKey::of(&LightSpec::default()).cone, 0);

        let mesh = hood_mesh(30);
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        // The far rim opens at the cone's angle from the disk's rim.
        let (near, far) = (Vec3::from(positions[0]), Vec3::from(positions[1]));
        let flare = (far.truncate().length() - near.truncate().length()).atan2(far.z - near.z);
        assert!((flare.to_degrees() - 30.0).abs() < 0.01, "{flare}");
    }

    #[test]
    fn a_zero_radius_light_still_has_a_size() {
        let key = EmitterKey::of(&LightSpec::default());
        assert_eq!(key.size.x, MIN_EMITTER_RADIUS);
        assert!(key.radiance.x.is_finite());
    }

    #[test]
    fn the_blocks_override_the_project_for_the_run() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.running = true;
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<crate::engine::PendingEffects>()
            .add_systems(Update, apply_ray_tracing_effects);
        app.world_mut()
            .resource_mut::<crate::engine::PendingEffects>()
            .0
            .extend([
                Effect::SetRayTracing { enabled: true },
                Effect::SetGiBounces { bounces: 99.0 },
                Effect::SetGiSamples { samples: 2.4 },
            ]);
        app.update();
        let engine = app.world().non_send::<Engine>();
        let settings = wanted(engine);
        assert!(settings.enabled);
        assert_eq!(settings.bounces, RayTracingSettings::MAX_BOUNCES);
        assert_eq!(settings.samples, 2);
    }
}
