//! The VFX graph in the world (`blockloom_core::vfx` is the model).
//!
//! Every actor with an `Emitter` gets a draw: a particle buffer, a mesh
//! whose vertices only name slots, and a `ParticleMaterial` that reads the
//! buffer. What fills the buffer is the GPU sim (`shaders/vfx_sim.wesl`, 3D
//! with compute shaders) or the CPU pool (`vfx::Pool`: 2D, headless and
//! low-end devices, `cpu_only` projects and an actor's own ribbon). Either
//! way `step_emitters` decides what spawns each frame, and the particles'
//! `spawn`/`die`/`collide` counts come back as `when my particles` events:
//! straight away from the pool, a frame or two late from the GPU's readback.
//!
//! In the scene view, the selected actor's emitter plays on a loop of its
//! `duration`, so an edit shows without pressing Play.

mod gpu;
mod render;

pub use gpu::VfxView;
pub use render::ParticleMaterial;

use crate::engine::{ActorId, Dimension, Engine};
use crate::wind::WindField;
use crate::world::WorldCamera;
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::ShaderType;
use bevy::render::renderer::RenderDevice;
use bevy::render::storage::ShaderBuffer;
use bevy::sprite_render::{Material2dPlugin, MeshMaterial2d};
use blockloom_core::material::ParticleSpec;
use blockloom_core::scene::Mode;
use blockloom_core::sense;
use blockloom_core::vfx::{
    self as model, CPU_MAX, GPU_MAX, LaunchDirection, MAX_MODULES, Particle, ParticleBlend,
    ParticleEvent, Pool, RibbonSource, SimMode, SpawnShape, StepEvents, Surface, Triangle,
    UpdateModule,
};
use blockloom_core::vm::Event;
use gpu::{GpuStep, Upload, VfxFrame};
use render::{ParticleLook, billboard_mesh, ribbon_mesh, ribbon_slots};
use std::collections::HashMap;
use std::sync::Arc;

pub fn register(app: &mut App, mode: Mode) {
    let _ = mode;
    app.init_resource::<Draws>()
        .init_resource::<OverdrawMeter>()
        .init_resource::<ParticleSenses>()
        .init_resource::<VfxStats>()
        .init_resource::<GpuCounts>();
    // A bare test world has no render assets to draw with.
    if !app.world().contains_resource::<Assets<ShaderBuffer>>() {
        return;
    }
    bevy::asset::embedded_asset!(app, "shaders/vfx_particles.wesl");
    bevy::asset::embedded_asset!(app, "shaders/vfx_particles_2d.wesl");
    bevy::asset::embedded_asset!(app, "shaders/vfx_sim.wesl");
    gpu::register(app);
    // Both dimensions' pipelines live side by side for live cross-dimension
    // switches; each finds only its own entities in the wrong dimension.
    app.add_plugins(Material2dPlugin::<ParticleMaterial>::default());
    app.add_plugins(MaterialPlugin::<ParticleMaterial>::default());
    let shader = app
        .world()
        .resource::<AssetServer>()
        .load(sim_shader_path());
    gpu::register_sim(app, shader);
    app.add_systems(
        Update,
        (step_emitters, mark_views)
            .chain()
            .after(crate::world::interpolate_poses)
            .before(crate::world::drive_camera),
    );
}

fn embedded(path: std::path::PathBuf) -> bevy::asset::AssetPath<'static> {
    bevy::asset::AssetPath::from_path_buf(path).with_source("embedded")
}

pub(crate) fn particles_3d() -> bevy::asset::AssetPath<'static> {
    embedded(bevy::asset::embedded_path!("shaders/vfx_particles.wesl"))
}

pub(crate) fn particles_2d() -> bevy::asset::AssetPath<'static> {
    embedded(bevy::asset::embedded_path!("shaders/vfx_particles_2d.wesl"))
}

fn sim_shader_path() -> bevy::asset::AssetPath<'static> {
    embedded(bevy::asset::embedded_path!("shaders/vfx_sim.wesl"))
}

/// `SimParams` in `vfx_sim.wesl`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct SimParams {
    pub emitter: Mat4,
    pub clip_from_world: Mat4,
    pub world_from_clip: Mat4,
    pub gravity: Vec4,
    pub wind: Vec4,
    pub launch: Vec4,
    pub shape: Vec4,
    pub look: Vec4,
    pub screen: Vec4,
    pub size: Vec4,
    pub counts: UVec4,
    pub ribbon: Vec4,
    pub size_lut: [Vec4; 8],
    pub modules: [Vec4; 24],
    pub colliders: UVec4,
}

const SIM_FLAT: u32 = 1;
const SIM_OUTWARD: u32 = 2;
const SIM_RIBBONS: u32 = 4;
/// World units behind a surface a particle may be and still hit it.
const COLLIDE_THICKNESS: f32 = 1.0;
/// Bytes the GPU state takes: `EmitterState` in `vfx_sim.wesl`.
const STATE_BYTES: u64 = 80;
/// Frames the step sequence counts before wrapping, exactly as an f32.
const SEQUENCE_WRAP: u32 = 1 << 23;

/// Live emission on an actor carrying an emitter, changed by blocks.
#[derive(Component)]
pub struct EmitterState {
    /// The authored emitter with this run's dials laid on, once a block
    /// has turned one.
    pub(crate) spec: Option<ParticleSpec>,
    /// Particles a `burst` block asked for, spawned next frame.
    pub(crate) burst: u32,
    pub(crate) playing: bool,
    acc: f32,
    /// The emitter's own clock, which bursts are timed on.
    clock: f64,
}

impl EmitterState {
    pub fn fresh() -> Self {
        Self {
            spec: None,
            burst: 0,
            playing: true,
            acc: 0.0,
            // Just before zero, so a burst at 0 fires on the first frame.
            clock: -1e-4,
        }
    }

    /// Starting again restarts the burst clock.
    pub(crate) fn set_playing(&mut self, playing: bool) {
        if playing && !self.playing {
            self.clock = -1e-4;
            self.acc = 0.0;
        }
        self.playing = playing;
    }
}

/// What the world's particles cost, for the profiler.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct VfxStats {
    pub emitters: usize,
    pub gpu_emitters: usize,
    pub particles: u32,
    /// Screens' worth of particle area: 1.0 is every pixel drawn once.
    pub overdraw: f32,
    pub budget: u32,
    pub allocated: u32,
    pub stolen: u64,
}

impl VfxStats {
    pub fn metrics(&self) -> [(&'static str, f64, &'static str); 7] {
        [
            ("vfx/emitters", self.emitters as f64, "count"),
            ("vfx/gpu_emitters", self.gpu_emitters as f64, "count"),
            ("vfx/particles", self.particles as f64, "count"),
            ("vfx/budget", self.budget as f64, "count"),
            ("vfx/allocated_slots", self.allocated as f64, "count"),
            ("vfx/stolen_pools", self.stolen as f64, "count"),
            ("vfx/overdraw", self.overdraw as f64, "screens"),
        ]
    }
}

/// The last GPU step's counts per emitter, as the readback brought them.
#[derive(Resource, Default)]
struct GpuCounts(HashMap<String, (Entity, GpuState)>);

impl GpuCounts {
    fn state_for(&self, id: &str, reader: Entity) -> Option<&GpuState> {
        self.0
            .get(id)
            .filter(|(source, _)| *source == reader)
            .map(|(_, state)| state)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GpuState {
    alive: u32,
    spawned: u32,
    died: u32,
    collided: u32,
    sequence: u32,
    spawn_at: Option<[f32; 3]>,
    die_at: Option<[f32; 3]>,
    collide_at: Option<[f32; 3]>,
}

impl GpuState {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let word = |i: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                bytes.get(i * 4..i * 4 + 4)?.try_into().ok()?,
            ))
        };
        let float = |i: usize| word(i).map(f32::from_bits);
        // A lane's w says whether anything has landed there yet.
        let at = |i: usize| -> Option<Option<[f32; 3]>> {
            let point = [float(i)?, float(i + 1)?, float(i + 2)?];
            Some((float(i + 3)? > 0.5).then_some(point))
        };
        Some(GpuState {
            alive: word(1)?,
            spawned: word(2)?,
            died: word(3)?,
            collided: word(4)?,
            sequence: word(6)?,
            spawn_at: at(8)?,
            die_at: at(12)?,
            collide_at: at(16)?,
        })
    }

    fn events(&self) -> StepEvents {
        StepEvents {
            spawned: self.spawned,
            died: self.died,
            collided: self.collided,
            spawn_at: self.spawn_at,
            die_at: self.die_at,
            collide_at: self.collide_at,
        }
    }
}

/// Which emitter a readback belongs to.
#[derive(Component)]
struct StateReader(String);

fn read_state(
    event: On<ReadbackComplete>,
    readers: Query<&StateReader>,
    mut counts: ResMut<GpuCounts>,
) {
    let Ok(reader) = readers.get(event.entity) else {
        return;
    };
    if let Some(state) = GpuState::parse(&event.data) {
        counts.0.insert(reader.0.clone(), (event.entity, state));
    }
}

/// What a draw was built for: a change to any of it builds a new one.
#[derive(Clone, Debug, PartialEq)]
struct DrawKey {
    gpu: bool,
    capacity: u32,
    points: u32,
    ribbons: bool,
    heads: bool,
    tessellation: u32,
    image: String,
    mesh_surface: bool,
}

enum Sim {
    Cpu(Box<Pool>),
    Gpu {
        state: Handle<ShaderBuffer>,
        surface: Handle<ShaderBuffer>,
        reader: Entity,
        /// Steps dispatched, and the last one whose counts were used.
        sequence: u32,
        seen: u32,
    },
}

/// One emitter's buffers, entities and sim.
struct Draw {
    key: DrawKey,
    sim: Sim,
    particles: Handle<ShaderBuffer>,
    trail: Handle<ShaderBuffer>,
    entities: Vec<Entity>,
    materials: Vec<Handle<ParticleMaterial>>,
    spec: Option<ParticleSpec>,
    surface: Option<Surface>,
    /// The scene view's preview keeps its own clock.
    preview: EmitterState,
    alive: u32,
    touched: bool,
    last_used: f32,
}

/// Every emitter's draw, by actor id.
#[derive(Resource, Default)]
pub struct Draws(HashMap<String, Draw>);

impl Draws {
    fn drop_draw(commands: &mut Commands, draw: Draw) {
        for entity in draw.entities {
            commands.entity(entity).despawn();
        }
        if let Sim::Gpu { reader, .. } = draw.sim {
            commands.entity(reader).despawn();
        }
    }

    /// Forget every draw: the world was rebuilt.
    pub fn clear(&mut self, commands: &mut Commands) {
        for (_, draw) in self.0.drain() {
            Self::drop_draw(commands, draw);
        }
    }
}

/// Where the GPU sim can run.
fn gpu_ready(device: Option<&RenderDevice>) -> bool {
    device.is_some_and(|device| {
        let limits = device.limits();
        limits.max_compute_workgroup_size_x >= 64
            && limits.max_storage_buffers_per_shader_stage >= 5
    })
}

#[derive(SystemParam)]
struct DrawAssets<'w> {
    buffers: ResMut<'w, Assets<ShaderBuffer>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<ParticleMaterial>>,
    server: Res<'w, AssetServer>,
    device: Option<Res<'w, RenderDevice>>,
}

#[derive(SystemParam)]
struct Surroundings<'w, 's> {
    wind: Option<Res<'w, WindField>>,
    scaling: Option<Res<'w, crate::quality::Scaling>>,
    editor: Option<Res<'w, crate::edit::SceneEditor>>,
    cameras: Query<'w, 's, (&'static GlobalTransform, &'static Camera), With<WorldCamera>>,
    shapes: Query<'w, 's, (&'static Mesh3d, &'static GlobalTransform)>,
    children: Query<'w, 's, &'static Children>,
    poses: Query<'w, 's, &'static Transform, With<ActorId>>,
    visibility: Query<'w, 's, &'static ViewVisibility>,
}

fn step_emitters(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    time: Res<Time>,
    mut emitters: Query<(Entity, &ActorId, &Transform, Option<&mut EmitterState>)>,
    mut draws: ResMut<Draws>,
    mut assets: DrawAssets,
    around: Surroundings,
    mut frame: ResMut<VfxFrame>,
    mut counts: ResMut<GpuCounts>,
    mut stats: ResMut<VfxStats>,
    mut meter: ResMut<OverdrawMeter>,
    mut senses: ResMut<ParticleSenses>,
) {
    let mode = dimension.0;
    let dt = time.delta_secs().min(0.1);
    let running = engine.running && !engine.starting;
    let frozen = running && engine.paused;
    let previewing = (!engine.running)
        .then(|| {
            around
                .editor
                .as_ref()
                .and_then(|editor| editor.selected.clone())
        })
        .flatten();
    let gpu_ok =
        mode.is_3d() && gpu_ready(assets.device.as_deref()) && !engine.project.world.vfx.cpu_only;
    let authored_budget = engine.project.world.vfx.budget;
    let budget = around
        .scaling
        .as_ref()
        .map_or(authored_budget, |s| s.particle_budget(authored_budget));
    let camera = around.cameras.iter().next();
    let eye = camera.map_or(Vec3::ZERO, |(transform, _)| transform.translation());
    let pixels = camera
        .and_then(|(_, camera)| camera.physical_viewport_size())
        .map_or(0, |size| size.x * size.y);
    frame.uploads.clear();
    frame.steps.clear();
    let (shapes, shape_ids) = collider_shapes(eye);
    frame.shapes = meter.shapes(&mut assets.buffers, &shapes, &mut frame.uploads);
    frame
        .uploads
        .push(meter.restart(&mut commands, &mut assets.buffers));
    for draw in draws.0.values_mut() {
        draw.touched = false;
        if draw
            .entities
            .iter()
            .any(|e| around.visibility.get(*e).is_ok_and(|v| v.get()))
        {
            draw.last_used = time.elapsed_secs();
        }
    }
    let mut room = budget.saturating_sub(stats.particles);
    let mut next = VfxStats {
        budget,
        stolen: stats.stolen,
        // Last frame's fragments over the screen's pixels.
        overdraw: if pixels > 0 {
            meter.fragments as f32 / pixels as f32
        } else {
            0.0
        },
        ..default()
    };
    let mut sensed = HashMap::new();
    let mut fired = Vec::new();
    let gravity = Vec3::from_array(engine.project.world.gravity);
    let mut order: Vec<_> = emitters
        .iter()
        .map(|(entity, id, pose, _)| {
            let used = draws.0.get(&id.0).map_or(0.0, |d| d.last_used);
            (
                entity,
                used,
                pose.translation.distance_squared(eye),
                id.0.clone(),
            )
        })
        .collect();
    order.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then(a.2.total_cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    let mut allocation_room = budget;
    for (entity, ..) in order {
        let Ok((entity, id, transform, state)) = emitters.get_mut(entity) else {
            continue;
        };
        let id = &id.0;
        let live = running && state.is_some() && engine.has_component(id, "Emitter");
        let preview = previewing.as_deref() == Some(id.as_str());
        if !live && !preview {
            continue;
        }
        let Some(authored) = engine
            .actor(id)
            .and_then(|actor| actor.components.emitter())
        else {
            continue;
        };
        let mut spec = state
            .as_ref()
            .filter(|_| live)
            .and_then(|state| state.spec.clone())
            .unwrap_or_else(|| authored.clone());
        spec.normalize();
        let pinned = spec.ribbon.enabled && spec.ribbon.source == RibbonSource::Actor;
        let gpu = gpu_ok && spec.sim == SimMode::Auto && !pinned;
        let authored_capacity = spec.max.min(if gpu { GPU_MAX } else { CPU_MAX }).max(1);
        let capacity = around
            .scaling
            .as_ref()
            .map_or(authored_capacity, |s| {
                s.particle_budget(authored_capacity).max(1)
            })
            .min(allocation_room);
        if capacity == 0 {
            if draws.0.contains_key(id) {
                next.stolen += 1;
            }
            continue;
        }
        allocation_room -= capacity;
        next.allocated += capacity;
        let key = DrawKey {
            gpu,
            capacity,
            points: if spec.ribbon.enabled {
                spec.ribbon.points
            } else {
                2
            },
            ribbons: spec.ribbon.enabled,
            heads: !spec.ribbon.enabled || spec.ribbon.heads,
            tessellation: spec.ribbon.tessellation,
            image: spec.render.flipbook.image.clone(),
            mesh_surface: spec.shape == SpawnShape::MeshSurface,
        };
        if draws.0.get(id).is_some_and(|draw| draw.key != key)
            && let Some(old) = draws.0.remove(id)
        {
            Draws::drop_draw(&mut commands, old);
            counts.0.remove(id);
        }
        if !draws.0.contains_key(id) {
            let overdraw = meter.counter(&mut assets.buffers);
            let draw = new_draw(
                &mut commands,
                &mut assets,
                &engine,
                mode,
                id,
                key,
                &spec,
                overdraw,
            );
            draws.0.insert(id.clone(), draw);
        }
        let Some(draw) = draws.0.get_mut(id) else {
            continue;
        };
        draw.touched = true;
        for &entity in &draw.entities {
            commands
                .entity(entity)
                .insert(Transform::from_translation(transform.translation));
        }
        if draw.spec.as_ref() != Some(&spec) {
            let flipbook = !spec.render.flipbook.image.is_empty();
            for (i, handle) in draw.materials.iter().enumerate() {
                if let Some(mut material) = assets.materials.get_mut(handle) {
                    let ribbons = draw.key.ribbons && (i > 0 || !draw.key.heads);
                    material.look = ParticleLook::of(
                        &spec,
                        draw.key.capacity,
                        draw.key.points,
                        ribbons,
                        flipbook,
                    );
                    material.additive = spec.render.blend == ParticleBlend::Additive;
                }
            }
            draw.spec = Some(spec.clone());
        }
        next.emitters += 1;
        if frozen {
            next.particles += draw.alive;
            if let Some(sense) = senses.0.get(id) {
                sensed.insert(id.clone(), *sense);
            }
            continue;
        }
        if draw.key.mesh_surface && draw.surface.is_none() {
            draw.surface = mesh_surface(entity, transform, &around, &assets.meshes);
            if let (
                Some(surface),
                Sim::Gpu {
                    surface: handle, ..
                },
            ) = (&draw.surface, &mut draw.sim)
            {
                *handle = assets.buffers.add(surface_buffer(surface));
            }
        }

        // What spawns this frame.
        let emission = match state {
            Some(state) if live => state.into_inner(),
            _ => &mut draw.preview,
        };
        let mut spawn = if emission.playing {
            model::due(&spec, &mut emission.acc, emission.clock, dt)
        } else {
            0
        };
        spawn = spawn.saturating_add(std::mem::take(&mut emission.burst));
        emission.clock += dt as f64;
        if preview && emission.clock >= spec.duration as f64 {
            emission.clock = -1e-4;
        }
        spawn = spawn.min(room);
        room -= spawn;
        let emitter = Mat4::from_rotation_translation(transform.rotation, transform.translation);
        let targets: Vec<Option<Vec3>> = spec
            .modules
            .iter()
            .map(|module| match module {
                UpdateModule::Attractor { target, .. } if !target.is_empty() => {
                    target_position(&engine, target, &around.poses)
                }
                _ => None,
            })
            .collect();
        let events = match &mut draw.sim {
            Sim::Cpu(pool) => {
                let wind = around.wind.as_deref();
                let blow = |at: model::Vec3| {
                    wind.map_or(model::Vec3::ZERO, |wind| {
                        model::Vec3::from_array(wind.at(Vec3::from_array(at.to_array())).to_array())
                    })
                };
                let collide = |from: model::Vec3, to: model::Vec3| {
                    sense::read(|sensors| {
                        blockloom_core::physics_query::segment_contact(
                            sensors,
                            from.to_array(),
                            to.to_array(),
                            Some(id),
                        )
                    })
                    .map(|(point, normal)| model::Hit {
                        point: model::Vec3::from_array(point),
                        normal: model::Vec3::from_array(normal),
                    })
                };
                let core_targets: Vec<Option<model::Vec3>> = targets
                    .iter()
                    .map(|t| t.map(|t| model::Vec3::from_array(t.to_array())))
                    .collect();
                let input = model::StepInput {
                    dt,
                    time: time.elapsed_secs_wrapped(),
                    emitter: model::Mat4::from_cols_array(&emitter.to_cols_array()),
                    gravity: model::Vec3::from_array(gravity.to_array()),
                    flat: !mode.is_3d(),
                    spawn,
                    targets: &core_targets,
                    surface: draw.surface.as_ref(),
                    wind: &blow,
                    collide: &collide,
                };
                let events = pool.step(&spec, &input);
                draw.alive = pool.alive();
                let mut bytes = Vec::new();
                Particle::to_bytes(&pool.particles, &mut bytes);
                frame.uploads.push(Upload {
                    buffer: draw.particles.clone(),
                    bytes: Arc::new(bytes),
                });
                if draw.key.ribbons {
                    let bytes: Vec<u8> = pool
                        .trail
                        .iter()
                        .flat_map(|point| point.iter().flat_map(|v| v.to_le_bytes()))
                        .collect();
                    frame.uploads.push(Upload {
                        buffer: draw.trail.clone(),
                        bytes: Arc::new(bytes),
                    });
                }
                events
            }
            Sim::Gpu {
                state,
                surface,
                sequence,
                seen,
                reader,
            } => {
                let wind = around.wind.as_deref().map_or(Vec3::ZERO, |wind| {
                    wind.at(transform.translation) * spec.wind
                });
                *sequence = (*sequence + 1) % SEQUENCE_WRAP;
                let triangles = draw.surface.as_ref().map_or(0, |s| s.triangles.len());
                frame.steps.push(GpuStep {
                    params: sim_params(
                        &spec,
                        SimInput {
                            emitter,
                            gravity: gravity * spec.gravity_scale,
                            wind,
                            dt,
                            time: time.elapsed_secs_wrapped(),
                            capacity: draw.key.capacity,
                            spawn,
                            seed: model::pcg(*sequence ^ id_seed(id)),
                            flat: false,
                            ribbons: draw.key.ribbons,
                            points: draw.key.points,
                            triangles: triangles as u32,
                            sequence: *sequence,
                            targets: &targets,
                            colliders: shapes.len() as u32,
                            skip: shape_ids
                                .iter()
                                .position(|shape| shape == id)
                                .map_or(0, |i| i as u32 + 1),
                        },
                    ),
                    particles: draw.particles.clone(),
                    trail: draw.trail.clone(),
                    state: state.clone(),
                    surface: surface.clone(),
                    capacity: draw.key.capacity,
                });
                // A step's counts come back a frame or two late; each set
                // is used once.
                match counts.state_for(id, *reader) {
                    Some(counts) if counts.sequence != *seen => {
                        *seen = counts.sequence;
                        draw.alive = counts.alive;
                        counts.events()
                    }
                    _ => StepEvents::default(),
                }
            }
        };
        next.particles += draw.alive;
        let mut sense = senses.0.get(id).copied().unwrap_or_default();
        sense.record(draw.alive, &events);
        sensed.insert(id.clone(), sense);
        if draw.key.gpu {
            next.gpu_emitters += 1;
        }
        if live {
            for event in ParticleEvent::ALL {
                if events.count(event) > 0 {
                    fired.push(Event::Particles {
                        actor: id.clone(),
                        event,
                    });
                }
            }
        }
    }
    let stale: Vec<String> = draws
        .0
        .iter()
        .filter(|(_, draw)| !draw.touched)
        .map(|(id, _)| id.clone())
        .collect();
    for id in stale {
        counts.0.remove(&id);
        if let Some(draw) = draws.0.remove(&id) {
            Draws::drop_draw(&mut commands, draw);
        }
    }
    *stats = next;
    senses.0 = sensed;
    for event in fired {
        engine.fire(event);
    }
}

/// The world camera carries the sim's view marker, and a depth prepass
/// while a 3D emitter collides or fades softly.
fn mark_views(
    mut commands: Commands,
    draws: Res<Draws>,
    dimension: Res<Dimension>,
    cameras: Query<
        (
            Entity,
            Has<VfxView>,
            Has<bevy::core_pipeline::prepass::DepthPrepass>,
        ),
        With<WorldCamera>,
    >,
) {
    if !dimension.0.is_3d() {
        return;
    }
    let depth = draws.0.values().any(|draw| {
        draw.spec.as_ref().is_some_and(|spec| {
            spec.render.soft > 0.0
                || (draw.key.gpu && spec.modules.iter().any(UpdateModule::is_collide))
        })
    });
    for (camera, marked, has_depth) in &cameras {
        if !marked {
            commands.entity(camera).insert(VfxView);
        }
        if depth && !has_depth {
            commands
                .entity(camera)
                .insert(bevy::core_pipeline::prepass::DepthPrepass);
        }
    }
}

/// A stable number per actor, so two emitters don't spray in step.
fn id_seed(id: &str) -> u32 {
    id.bytes().fold(0x811C_9DC5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    })
}

fn target_position(
    engine: &Engine,
    target: &str,
    poses: &Query<&Transform, With<ActorId>>,
) -> Option<Vec3> {
    let id = if engine.entities.contains_key(target) {
        target.to_string()
    } else {
        engine
            .project
            .actors
            .iter()
            .find(|actor| actor.name.eq_ignore_ascii_case(target))
            .map(|actor| actor.id.clone())?
    };
    let entity = engine.entities.get(&id)?;
    poses.get(*entity).ok().map(|pose| pose.translation)
}

/// Most triangles a mesh surface gathers before thinning, so a detailed
/// model doesn't stall the frame it is read on.
const SURFACE_GATHER: usize = 1 << 16;

/// The actor's drawn meshes (its own and every part of a model under it),
/// as triangles in its frame: scaled, not turned or moved.
fn mesh_surface(
    entity: Entity,
    pose: &Transform,
    around: &Surroundings,
    meshes: &Assets<Mesh>,
) -> Option<Surface> {
    // From world into the emitter's frame, keeping its scale.
    let frame = Mat4::from_rotation_translation(pose.rotation, pose.translation).inverse();
    let mut triangles = Vec::new();
    let mut stack = vec![entity];
    while let Some(next) = stack.pop() {
        if let Ok((handle, world)) = around.shapes.get(next)
            && let Some(mesh) = meshes.get(&handle.0)
        {
            let into = frame * world.to_matrix();
            gather_triangles(mesh, into, &mut triangles);
        }
        if let Ok(children) = around.children.get(next) {
            stack.extend(children.iter());
        }
        if triangles.len() >= SURFACE_GATHER {
            break;
        }
    }
    (!triangles.is_empty()).then(|| Surface::new(triangles))
}

fn gather_triangles(mesh: &Mesh, into: Mat4, out: &mut Vec<Triangle>) {
    let Some(positions) = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|values| values.as_float3())
    else {
        return;
    };
    let at = |i: usize| -> model::Vec3 {
        let p = into.transform_point3(Vec3::from_array(positions[i]));
        model::Vec3::from_array(p.to_array())
    };
    let indices: Vec<usize> = match mesh.indices() {
        Some(indices) => indices.iter().collect(),
        None => (0..positions.len()).collect(),
    };
    out.extend(
        indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|tri| tri.iter().all(|&i| i < positions.len()))
            .map(|tri| Triangle {
                a: at(tri[0]),
                b: at(tri[1]),
                c: at(tri[2]),
            }),
    );
}

/// Actor shapes the GPU sim collides with, at most this many, nearest the
/// camera first.
pub const GPU_COLLIDERS: usize = 128;

/// Every actor with a body and a collider, as the CPU pool collides with,
/// as two lanes each (centre and kind, then half extents or radius), and
/// the ids in the same order.
fn collider_shapes(eye: Vec3) -> (Vec<[f32; 8]>, Vec<String>) {
    use blockloom_core::sense::ColliderShape;
    let mut found: Vec<(f32, String, [f32; 8])> = sense::read(|sensors| {
        sensors
            .actors
            .iter()
            .filter(|(_, actor)| actor.has_body)
            .flat_map(|(id, actor)| {
                let [x, y, z] = actor.position;
                let lanes = match &actor.shape {
                    ColliderShape::None => Vec::new(),
                    ColliderShape::Box { half } => {
                        vec![[x, y, z, 1.0, half[0], half[1], half[2], 0.0]]
                    }
                    ColliderShape::Ball { radius } => vec![[x, y, z, 2.0, *radius, 0.0, 0.0, 0.0]],
                    // A tilemap's solid runs, each its own box, placed with
                    // the map's turn (the boxes themselves stay upright).
                    ColliderShape::Parts(parts) => {
                        let [rx, ry, rz] = actor.rotation.map(f32::to_radians);
                        let turn = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                        parts
                            .iter()
                            .map(|part| {
                                let at = Vec3::from_array(actor.position)
                                    + turn * Vec3::from_array(part.offset);
                                let h = part.half;
                                [at.x, at.y, at.z, 1.0, h[0], h[1], h[2], 0.0]
                            })
                            .collect()
                    }
                };
                lanes.into_iter().map(|lanes| {
                    let far = Vec3::new(lanes[0], lanes[1], lanes[2]).distance_squared(eye);
                    (far, id.clone(), lanes)
                })
            })
            .collect()
    });
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    found.truncate(GPU_COLLIDERS);
    found.into_iter().map(|(_, id, lanes)| (lanes, id)).unzip()
}

/// The overdraw meter: one counter every particle draw adds its shaded
/// fragments to, zeroed before each frame and read back after it. Also
/// holds the GPU sim's shape list, since both are one small buffer a frame.
#[derive(Resource, Default)]
pub struct OverdrawMeter {
    counter: Option<Handle<ShaderBuffer>>,
    reader: Option<Entity>,
    shapes: Option<Handle<ShaderBuffer>>,
    /// Fragments the last frame read back shaded.
    fragments: u32,
}

impl OverdrawMeter {
    fn counter(&mut self, buffers: &mut Assets<ShaderBuffer>) -> Handle<ShaderBuffer> {
        self.counter
            .get_or_insert_with(|| buffers.add(zeroed(16)))
            .clone()
    }

    /// Zeroes the counter for this frame, and makes sure it is read back.
    fn restart(&mut self, commands: &mut Commands, buffers: &mut Assets<ShaderBuffer>) -> Upload {
        let counter = self.counter(buffers);
        if self.reader.is_none() {
            let reader = commands
                .spawn(Readback::buffer(counter.clone()))
                .observe(read_overdraw)
                .id();
            self.reader = Some(reader);
        }
        Upload {
            buffer: counter,
            bytes: Arc::new(vec![0; 16]),
        }
    }

    /// This frame's collider list, written into one fixed-size buffer.
    fn shapes(
        &mut self,
        buffers: &mut Assets<ShaderBuffer>,
        shapes: &[[f32; 8]],
        uploads: &mut Vec<Upload>,
    ) -> Option<Handle<ShaderBuffer>> {
        let handle = self
            .shapes
            .get_or_insert_with(|| buffers.add(zeroed(GPU_COLLIDERS as u64 * 32)))
            .clone();
        if !shapes.is_empty() {
            let bytes = shapes
                .iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            uploads.push(Upload {
                buffer: handle.clone(),
                bytes: Arc::new(bytes),
            });
        }
        Some(handle)
    }
}

fn read_overdraw(event: On<ReadbackComplete>, mut meter: ResMut<OverdrawMeter>) {
    if let Some(word) = event.data.get(0..4) {
        meter.fragments = u32::from_le_bytes(word.try_into().unwrap_or_default());
    }
}

/// Every emitter's `ParticleSense`, by actor id, which `publish_sensors`
/// copies into each actor's reading.
#[derive(Resource, Default)]
pub struct ParticleSenses(pub HashMap<String, blockloom_core::vfx::ParticleSense>);

/// Four lanes a triangle: a and the running area, b, c, normal.
fn surface_buffer(surface: &Surface) -> ShaderBuffer {
    let mut lanes: Vec<[f32; 4]> = Vec::with_capacity(surface.triangles.len() * 4);
    for (triangle, cdf) in surface.triangles.iter().zip(&surface.cdf) {
        let n = triangle.normal();
        lanes.push([triangle.a.x, triangle.a.y, triangle.a.z, *cdf]);
        lanes.push([triangle.b.x, triangle.b.y, triangle.b.z, 0.0]);
        lanes.push([triangle.c.x, triangle.c.y, triangle.c.z, 0.0]);
        lanes.push([n.x, n.y, n.z, 0.0]);
    }
    if lanes.is_empty() {
        lanes.push([0.0; 4]);
    }
    ShaderBuffer::new(lanes, RenderAssetUsages::RENDER_WORLD)
}

fn zeroed(bytes: u64) -> ShaderBuffer {
    ShaderBuffer::new(
        vec![0u8; bytes.max(16) as usize],
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn new_draw(
    commands: &mut Commands,
    assets: &mut DrawAssets,
    engine: &Engine,
    mode: Mode,
    id: &str,
    key: DrawKey,
    spec: &ParticleSpec,
    overdraw: Handle<ShaderBuffer>,
) -> Draw {
    let capacity = key.capacity;
    let particles = assets
        .buffers
        .add(zeroed(capacity as u64 * Particle::BYTES as u64));
    let trail_bytes = if key.ribbons {
        capacity as u64 * key.points as u64 * model::RIBBON_POINT_BYTES as u64
    } else {
        16
    };
    let trail = assets.buffers.add(zeroed(trail_bytes));
    let sim = if key.gpu {
        let state = assets.buffers.add(zeroed(STATE_BYTES));
        let surface = assets.buffers.add(zeroed(16));
        let reader = commands
            .spawn((Readback::buffer(state.clone()), StateReader(id.to_string())))
            .observe(read_state)
            .id();
        Sim::Gpu {
            state,
            surface,
            reader,
            sequence: 0,
            seen: u32::MAX,
        }
    } else {
        Sim::Cpu(Box::new(Pool::new(capacity, key.points, id_seed(id))))
    };
    let sheet = (!key.image.is_empty()).then(|| {
        assets.server.load(crate::world::asset_path(
            engine.project_dir.as_deref(),
            &key.image,
        ))
    });
    let mut materials = Vec::new();
    let mut entities = Vec::new();
    let mut layers: Vec<(bool, Mesh)> = Vec::new();
    if key.heads {
        layers.push((false, billboard_mesh(capacity)));
    }
    if key.ribbons {
        let slots = ribbon_slots(capacity, key.points, key.tessellation);
        layers.push((true, ribbon_mesh(slots, key.points, key.tessellation)));
    }
    for (ribbons, mesh) in layers {
        let material = assets.materials.add(ParticleMaterial {
            look: ParticleLook::of(spec, capacity, key.points, ribbons, sheet.is_some()),
            particles: particles.clone(),
            trail: trail.clone(),
            sheet: sheet.clone(),
            overdraw: overdraw.clone(),
            additive: spec.render.blend == ParticleBlend::Additive,
        });
        let mesh = assets.meshes.add(mesh);
        let mut entity = commands.spawn((
            Transform::default(),
            Visibility::default(),
            bevy::camera::visibility::NoFrustumCulling,
            Name::new(format!("particles {id}")),
        ));
        match mode {
            Mode::ThreeD => {
                entity.insert((
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    bevy::light::NotShadowCaster,
                    bevy::light::NotShadowReceiver,
                ));
            }
            Mode::TwoD => {
                entity.insert((bevy::mesh::Mesh2d(mesh), MeshMaterial2d(material.clone())));
            }
        }
        entities.push(entity.id());
        materials.push(material);
    }
    Draw {
        key,
        sim,
        particles,
        trail,
        entities,
        materials,
        spec: Some(spec.clone()),
        surface: None,
        preview: EmitterState::fresh(),
        alive: 0,
        touched: true,
        last_used: 0.0,
    }
}

struct SimInput<'a> {
    emitter: Mat4,
    gravity: Vec3,
    wind: Vec3,
    dt: f32,
    time: f32,
    capacity: u32,
    spawn: u32,
    seed: u32,
    flat: bool,
    ribbons: bool,
    points: u32,
    triangles: u32,
    sequence: u32,
    targets: &'a [Option<Vec3>],
    /// Actor shapes in this frame's list, and the emitter's own plus one.
    colliders: u32,
    skip: u32,
}

/// One GPU step's uniforms, bar the camera's half.
fn sim_params(spec: &ParticleSpec, input: SimInput) -> SimParams {
    let render = &spec.render;
    let size = render.size.bake();
    let mut flags = 0;
    if input.flat {
        flags |= SIM_FLAT;
    }
    if spec.direction == LaunchDirection::Outward {
        flags |= SIM_OUTWARD;
    }
    if input.ribbons {
        flags |= SIM_RIBBONS;
    }
    let shape = spec.shape.params();
    let mut modules = [Vec4::ZERO; 24];
    for (i, module) in spec.modules.iter().take(MAX_MODULES).enumerate() {
        let kind = module.code() as f32;
        let (a, b, c) = match module {
            UpdateModule::Force { force } => (
                Vec4::new(kind, force[0], force[1], force[2]),
                Vec4::ZERO,
                Vec4::ZERO,
            ),
            UpdateModule::Drag { amount } => {
                (Vec4::new(kind, *amount, 0.0, 0.0), Vec4::ZERO, Vec4::ZERO)
            }
            UpdateModule::CurlNoise {
                strength,
                scale,
                speed,
            } => (
                Vec4::new(kind, *strength, *scale, *speed),
                Vec4::ZERO,
                Vec4::ZERO,
            ),
            UpdateModule::Turbulence { strength, scale } => (
                Vec4::new(kind, *strength, *scale, 0.0),
                Vec4::ZERO,
                Vec4::ZERO,
            ),
            UpdateModule::Attractor {
                strength,
                radius,
                kill_radius,
                ..
            } => {
                let target = input
                    .targets
                    .get(i)
                    .copied()
                    .flatten()
                    .map_or(Vec4::ZERO, |t| t.extend(1.0));
                (
                    Vec4::new(kind, *strength, *radius, *kill_radius),
                    Vec4::ZERO,
                    target,
                )
            }
            UpdateModule::Collide {
                bounce,
                friction,
                lifetime_loss,
                kill,
            } => (
                Vec4::new(kind, *bounce, *friction, *lifetime_loss),
                Vec4::new(if *kill { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
                Vec4::ZERO,
            ),
            UpdateModule::KillPlane { point, normal } => (
                Vec4::new(kind, normal[0], normal[1], normal[2]),
                Vec4::new(point[0], point[1], point[2], 0.0),
                Vec4::ZERO,
            ),
        };
        modules[i * 3] = a;
        modules[i * 3 + 1] = b;
        modules[i * 3 + 2] = c;
    }
    SimParams {
        emitter: input.emitter,
        clip_from_world: Mat4::IDENTITY,
        world_from_clip: Mat4::IDENTITY,
        gravity: input.gravity.extend(input.dt),
        wind: input.wind.extend(input.time),
        launch: Vec4::new(
            spec.speed,
            spec.lifetime,
            (spec.spread.to_radians() * 0.5).clamp(0.0, std::f32::consts::PI),
            render.size_random,
        ),
        shape: Vec4::new(shape[0], shape[1], shape[2], spec.shape.code() as f32),
        look: Vec4::new(
            render.spin,
            if render.random_rotation { 1.0 } else { 0.0 },
            render.flipbook.frames() as f32,
            if render.flipbook.random_start {
                1.0
            } else {
                0.0
            },
        ),
        // Filled in the render world, from the view.
        screen: Vec4::new(1.0, 1.0, 0.0, COLLIDE_THICKNESS),
        size: Vec4::new(spec.size_start, spec.size_end, 1.0, 1.0),
        counts: UVec4::new(input.capacity, input.spawn, input.seed, flags),
        ribbon: Vec4::new(
            spec.ribbon.step,
            input.points as f32,
            input.triangles as f32,
            input.sequence as f32,
        ),
        size_lut: std::array::from_fn(|i| {
            Vec4::from_array(std::array::from_fn(|j| size[i * 4 + j]))
        }),
        modules,
        colliders: UVec4::new(input.colliders, input.skip, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_density_resizes_cpu_pools_and_enforces_the_total_allocation() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.running = true;
        engine.rebuild = false;
        engine.project.world.vfx.budget = 160;
        let mut actors = Vec::new();
        for id in ["a", "b"] {
            let mut actor = engine.project.actors[0].clone();
            actor.id = id.into();
            actor
                .components
                .insert(blockloom_core::components::ActorComponent::Emitter {
                    emitter: ParticleSpec {
                        max: 100,
                        sim: SimMode::Cpu,
                        ..default()
                    },
                });
            engine.attached.insert(id.into(), ["Emitter".into()].into());
            actors.push(actor);
        }
        engine.project.actors = actors;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<ShaderBuffer>()
            .init_asset::<ParticleMaterial>()
            .insert_non_send(engine)
            .insert_resource(Dimension(Mode::ThreeD))
            .init_resource::<crate::quality::Scaling>()
            .init_resource::<Draws>()
            .init_resource::<OverdrawMeter>()
            .init_resource::<ParticleSenses>()
            .init_resource::<VfxStats>()
            .init_resource::<GpuCounts>()
            .init_resource::<VfxFrame>()
            .add_systems(Update, step_emitters);
        for id in ["a", "b"] {
            app.world_mut().spawn((
                ActorId(id.into()),
                Transform::default(),
                EmitterState::fresh(),
            ));
        }
        app.update();
        assert_eq!(app.world().resource::<VfxStats>().allocated, 160);
        assert_eq!(app.world().resource::<Draws>().0["a"].key.capacity, 100);
        app.world_mut()
            .resource_mut::<crate::quality::Scaling>()
            .geometry
            .factors[3] = 0.5;
        app.update();
        let stats = app.world().resource::<VfxStats>();
        assert_eq!(stats.budget, 80);
        assert_eq!(stats.allocated, 80);
        let draws = app.world().resource::<Draws>();
        for (id, expected) in [("a", 50), ("b", 30)] {
            let draw = &draws.0[id];
            assert_eq!(draw.key.capacity, expected);
            let Sim::Cpu(pool) = &draw.sim else {
                panic!("expected CPU pool")
            };
            assert_eq!(pool.capacity(), expected);
        }
        // Retaining the reduction keeps the same allocations on quiet frames.
        app.update();
        assert_eq!(app.world().resource::<VfxStats>().allocated, 80);
    }

    #[test]
    fn the_sim_shader_compiles() {
        let source = include_str!("shaders/vfx_sim.wesl");
        for multisampled in [false, true] {
            blockloom_core::shader_lib::validate(source, &[("MULTISAMPLED", multisampled)])
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }

    #[test]
    fn the_particle_shaders_compile() {
        let view = "struct View { clip_from_world: mat4x4<f32>, clip_from_view: mat4x4<f32>, \
view_from_clip: mat4x4<f32>, world_from_view: mat4x4<f32>, world_position: vec3<f32>, viewport: vec4<f32> }\n\
@group(0) @binding(0) var<uniform> view: View;\n\
@group(0) @binding(1) var depth_prepass_texture: texture_depth_2d;\n\
fn depth_ndc_to_view_z(d: f32) -> f32 { return d; }\n\
struct Mat { base_color: vec4<f32>, perceptual_roughness: f32, metallic: f32, reflectance: vec3<f32> }\n\
struct Pbr { material: Mat, frag_coord: vec4<f32>, world_position: vec4<f32>, \
world_normal: vec3<f32>, N: vec3<f32>, V: vec3<f32>, is_orthographic: bool, flags: u32 }\n\
const MESH_FLAGS_SHADOW_RECEIVER_BIT: u32 = 1u;\n\
fn pbr_input_new() -> Pbr { var p: Pbr; return p; }\n\
fn apply_pbr_lighting(p: Pbr) -> vec4<f32> { return p.material.base_color; }\n";
        let source =
            crate::materials::tests::stubbed(include_str!("shaders/vfx_particles.wesl"), view)
                .replace("view_bindings::", "")
                .replace("@if(TONEMAP_IN_SHADER)\n\n", "\n");
        for depth in [false, true] {
            blockloom_core::shader_lib::validate(
                &source,
                &[("DEPTH_PREPASS", depth), ("TONEMAP_IN_SHADER", false)],
            )
            .unwrap_or_else(|error| panic!("{error}"));
        }
        let source =
            crate::materials::tests::stubbed(include_str!("shaders/vfx_particles_2d.wesl"), view)
                .replace(
                    "@if(TONEMAP_IN_SHADER)\n@if(SRGB_OUTPUT)\n@if(OKLAB_OUTPUT)\n",
                    "",
                );
        blockloom_core::shader_lib::validate(
            &source,
            &[
                ("TONEMAP_IN_SHADER", false),
                ("SRGB_OUTPUT", false),
                ("OKLAB_OUTPUT", false),
            ],
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn a_resized_gpu_pool_rejects_the_old_readers_counts() {
        let mut world = World::new();
        let old_reader = world.spawn_empty().id();
        let new_reader = world.spawn_empty().id();
        let state = GpuState {
            alive: 100,
            sequence: 1,
            ..default()
        };
        let mut counts = GpuCounts::default();
        counts.0.insert("sparks".into(), (old_reader, state));
        assert_eq!(counts.state_for("sparks", old_reader), Some(&state));
        assert_eq!(counts.state_for("sparks", new_reader), None);
        counts.0.insert(
            "sparks".into(),
            (new_reader, GpuState { alive: 5, ..state }),
        );
        assert_eq!(counts.state_for("sparks", new_reader).unwrap().alive, 5);
    }

    #[test]
    fn gpu_counts_parse_from_the_readback() {
        let mut bytes = vec![0u8; STATE_BYTES as usize];
        let put = |bytes: &mut Vec<u8>, i: usize, v: u32| {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes())
        };
        put(&mut bytes, 1, 7);
        put(&mut bytes, 3, 2);
        put(&mut bytes, 6, 42);
        put(&mut bytes, 12, 3.5f32.to_bits());
        put(&mut bytes, 15, 1.0f32.to_bits());
        let state = GpuState::parse(&bytes).unwrap();
        assert_eq!(state.alive, 7);
        assert_eq!(state.died, 2);
        assert_eq!(state.sequence, 42);
        assert_eq!(state.die_at, Some([3.5, 0.0, 0.0]));
        // Nothing has spawned into that lane yet.
        assert_eq!(state.spawn_at, None);
    }

    #[test]
    fn sim_params_pack_every_module() {
        let spec = ParticleSpec {
            modules: vec![
                UpdateModule::Drag { amount: 3.0 },
                UpdateModule::KillPlane {
                    point: [0.0, -2.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                },
            ],
            ..ParticleSpec::default()
        };
        let params = sim_params(
            &spec,
            SimInput {
                emitter: Mat4::IDENTITY,
                gravity: Vec3::ZERO,
                wind: Vec3::ZERO,
                dt: 0.016,
                time: 0.0,
                capacity: 64,
                spawn: 3,
                seed: 1,
                flat: false,
                ribbons: false,
                points: 2,
                triangles: 0,
                sequence: 9,
                targets: &[],
                colliders: 3,
                skip: 2,
            },
        );
        assert_eq!(params.modules[0], Vec4::new(2.0, 3.0, 0.0, 0.0));
        assert_eq!(params.modules[3].x, 7.0);
        assert_eq!(params.modules[4].y, -2.0);
        assert_eq!(params.counts, UVec4::new(64, 3, 1, 0));
        assert_eq!(params.ribbon.w, 9.0);
    }
}
