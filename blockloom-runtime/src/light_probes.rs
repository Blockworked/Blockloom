//! Light probes in the world, 3D only: an actor's `Probe` component as a
//! Bevy reflection probe (a box-projected cubemap) or irradiance volume (a
//! brick grid of ambient cubes), lit from its bake.
//!
//! Bakes come from the capture service (`probes.rs`). The editor asks for
//! one with `BakeProbes`, and a probe marked auto-bake asks for itself in the
//! scene view once its stamp (`blockloom_core::probe::stamp`) stops matching
//! the one on disk; both write under `.blockloom/probes`. `capture probes`
//! does the same capture in a running game but keeps it in memory.

use crate::engine::{ActorId, Engine};
use crate::probes::{ProbeCaptured, ProbeId, ProbeRequest, ProbeService};
use crate::streaming::Warmup;
use crate::world::WorldCamera;
use bevy::asset::RenderAssetUsages;
use bevy::light::{GeneratedEnvironmentMapLight, IrradianceVolume, LightProbe, ParallaxCorrection};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::tasks::AsyncComputeTaskPool;
use blockloom_core::pipeline::{bc6h, hdr::HdrCube};
use blockloom_core::probe::{self, AmbientCube, IrradianceGrid, ProbeKind, ProbeSpec};
use blockloom_protocol::RuntimeMessage;
use std::collections::HashMap;
use std::path::PathBuf;

pub fn register(app: &mut App) {
    app.init_resource::<ProbeBaker>()
        .init_resource::<LoadedProbes>();
}

/// Face size an irradiance brick captures at: it is averaged down to six
/// colors, so detail is wasted.
const BRICK_RESOLUTION: u32 = 32;
/// Brick captures in flight at once, so a big grid doesn't spawn hundreds of
/// cameras in one frame.
const BRICKS_IN_FLIGHT: usize = 8;

/// A probe's light, as loaded from disk or captured this run.
struct Loaded {
    kind: ProbeKind,
    /// The bake's stamp, or `None` for a live capture nothing wrote.
    stamp: Option<u64>,
    image: Handle<Image>,
}

#[derive(Resource, Default)]
pub struct LoadedProbes(HashMap<String, Loaded>);

/// Bakes asked for and bakes under way.
#[derive(Resource, Default)]
pub struct ProbeBaker {
    requested: Vec<(Vec<String>, bool)>,
    jobs: Vec<Job>,
}

impl ProbeBaker {
    /// Bakes `actors` (every probe when empty); `persist` writes the result
    /// under `.blockloom/probes` as well as lighting with it.
    pub fn request(&mut self, actors: Vec<String>, persist: bool) {
        self.requested.push((actors, persist));
    }

    pub fn busy(&self) -> bool {
        !self.requested.is_empty() || !self.jobs.is_empty()
    }
}

struct Job {
    actor: String,
    name: String,
    spec: ProbeSpec,
    stamp: Option<u64>,
    persist: bool,
    work: Work,
}

enum Work {
    Reflection {
        capture: Option<ProbeId>,
    },
    Irradiance {
        /// Every brick's world position, x fastest.
        points: Vec<Vec3>,
        next: usize,
        pending: HashMap<ProbeId, usize>,
        cubes: Vec<Option<AmbientCube>>,
    },
}

/// The Bevy probe standing in for an actor's `Probe`, and what it was built
/// from.
#[derive(Component)]
pub struct ProbeOf {
    actor: String,
    spec: ProbeSpec,
    image: AssetId<Image>,
}

fn probe_actors(engine: &Engine) -> Vec<(String, String, ProbeSpec)> {
    engine
        .project
        .actors
        .iter()
        .filter(|actor| engine.has_component(&actor.id, "Probe"))
        .filter_map(|actor| {
            let spec = actor.components.probe()?.clone();
            Some((actor.id.clone(), actor.name.clone(), spec))
        })
        .collect()
}

/// On every rebuild: loads bakes that changed on disk, forgets probes that
/// went away, and in the editor's scene view queues an auto-bake for every
/// probe whose bake is stale.
pub fn load_baked(
    engine: NonSend<Engine>,
    mut loaded: ResMut<LoadedProbes>,
    mut baker: ResMut<ProbeBaker>,
    mut service: ResMut<ProbeService>,
    mut images: ResMut<Assets<Image>>,
    fresh: Query<(), Added<WorldCamera>>,
) {
    if fresh.is_empty() {
        return;
    }
    // A rebuild moves things: whatever was mid-bake is capturing a world
    // that is gone.
    for job in baker.jobs.drain(..) {
        cancel(&job, &mut service);
    }
    let probes = probe_actors(&engine);
    loaded
        .0
        .retain(|actor, _| probes.iter().any(|(id, ..)| id == actor));
    let Some(dir) = engine.project_dir.clone() else {
        return;
    };
    let editing = crate::bridge::attached() && !engine.running && !engine.starting;
    let mut stale = Vec::new();
    for (actor, name, spec) in &probes {
        let info = probe::read_info(&dir, actor).filter(|info| info.kind == spec.kind);
        let current = loaded
            .0
            .get(actor)
            .is_some_and(|l| l.kind == spec.kind && info.as_ref().map(|i| i.stamp) == l.stamp);
        if let Some(info) = &info
            && !current
        {
            match load_bake(&dir, actor, spec.kind) {
                Ok(image) => {
                    loaded.0.insert(
                        actor.clone(),
                        Loaded {
                            kind: spec.kind,
                            stamp: Some(info.stamp),
                            image: images.add(image),
                        },
                    );
                }
                Err(message) => crate::bridge::send(&RuntimeMessage::Error {
                    actor: name.clone(),
                    message: format!("The probe's bake didn't load: {message}"),
                }),
            }
        }
        let now = probe::stamp(&engine.project, actor);
        if editing && spec.auto_bake && info.is_none_or(|info| Some(info.stamp) != now) {
            stale.push(actor.clone());
        }
    }
    if !stale.is_empty() {
        baker.request(stale, true);
    }
}

fn load_bake(dir: &std::path::Path, actor: &str, kind: ProbeKind) -> Result<Image, String> {
    match kind {
        ProbeKind::Reflection => {
            let path = probe::cube_path(dir, actor);
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let (size, blocks) = bc6h::read_dds_cube(&bytes)?;
            if !size.is_power_of_two() {
                return Err(format!("a {size} texel face isn't a power of two"));
            }
            let face = blocks.len() / 6;
            let mut texels = Vec::with_capacity((size * size * 8 * 6) as usize);
            for blocks in blocks.chunks_exact(face) {
                texels.extend(bc6h::decode_face(blocks, size)?);
            }
            Ok(cube_image(size, texels))
        }
        ProbeKind::Irradiance => Ok(grid_image(&IrradianceGrid::read(dir, actor)?)),
    }
}

/// Keeps a Bevy probe on every probe actor that has light to give, at the
/// actor's place and turn but the probe's own size.
#[allow(clippy::type_complexity)]
pub fn sync_probes(
    mut commands: Commands,
    engine: NonSend<Engine>,
    loaded: Res<LoadedProbes>,
    actors: Query<(&ActorId, &GlobalTransform)>,
    mut probes: Query<(Entity, &ProbeOf, &mut Transform)>,
) {
    let places: HashMap<&str, &GlobalTransform> =
        actors.iter().map(|(id, at)| (id.0.as_str(), at)).collect();
    let wanted: Vec<(String, ProbeSpec)> = probe_actors(&engine)
        .into_iter()
        .map(|(id, _, spec)| (id, spec))
        .filter(|(id, spec)| {
            loaded.0.get(id).is_some_and(|l| l.kind == spec.kind)
                && places.contains_key(id.as_str())
        })
        .collect();
    let transform_of = |actor: &str, spec: &ProbeSpec| {
        let (_, rotation, translation) = places[actor].to_scale_rotation_translation();
        Transform {
            translation,
            rotation,
            scale: Vec3::from_array(spec.size.map(|side| side.abs().max(0.01))),
        }
    };

    let mut have = Vec::new();
    for (entity, of, mut transform) in &mut probes {
        let keep = wanted.iter().find(|(id, spec)| {
            *id == of.actor && *spec == of.spec && loaded.0[id].image.id() == of.image
        });
        match keep {
            Some((id, spec)) => {
                let at = transform_of(id, spec);
                if *transform != at {
                    *transform = at;
                }
                have.push(id.clone());
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for (id, spec) in &wanted {
        if have.contains(id) {
            continue;
        }
        let image = loaded.0[id].image.clone();
        let mut entity = commands.spawn((
            ProbeOf {
                actor: id.clone(),
                spec: spec.clone(),
                image: image.id(),
            },
            LightProbe {
                falloff: Vec3::splat(spec.falloff.clamp(0.0, 1.0)),
            },
            transform_of(id, spec),
            Name::new("light probe"),
        ));
        let intensity = spec.intensity.max(0.0);
        match spec.kind {
            ProbeKind::Reflection => {
                entity.insert((
                    GeneratedEnvironmentMapLight {
                        environment_map: image,
                        intensity,
                        ..default()
                    },
                    if spec.box_projection {
                        ParallaxCorrection::Auto
                    } else {
                        ParallaxCorrection::None
                    },
                ));
            }
            ProbeKind::Irradiance => {
                entity.insert(IrradianceVolume {
                    voxels: image,
                    intensity,
                    affects_lightmapped_meshes: true,
                });
            }
        }
    }
}

/// Turns requests (the editor's, auto-bakes, `capture probes`) into jobs and
/// feeds their captures to the service once the world has warmed up.
pub fn start_bakes(
    mut engine: NonSendMut<Engine>,
    mut baker: ResMut<ProbeBaker>,
    mut service: ResMut<ProbeService>,
    warmup: Option<Res<Warmup>>,
    actors: Query<(&ActorId, &GlobalTransform)>,
) {
    if std::mem::take(&mut engine.capture_probes) {
        baker.request(Vec::new(), false);
    }
    if !baker.busy() || warmup.is_some_and(|warmup| warmup.active()) || engine.rebuild {
        return;
    }
    let places: HashMap<&str, &GlobalTransform> =
        actors.iter().map(|(id, at)| (id.0.as_str(), at)).collect();
    let baker = &mut *baker;
    for (actors, persist) in std::mem::take(&mut baker.requested) {
        for (actor, name, spec) in probe_actors(&engine) {
            if !actors.is_empty() && !actors.contains(&actor) {
                continue;
            }
            // A newer request for the same probe replaces an older one.
            if let Some(at) = baker.jobs.iter().position(|job| job.actor == actor) {
                let old = baker.jobs.remove(at);
                cancel(&old, &mut service);
            }
            let Some(place) = places.get(actor.as_str()) else {
                continue;
            };
            let (_, rotation, centre) = place.to_scale_rotation_translation();
            let work = match spec.kind {
                ProbeKind::Reflection => Work::Reflection { capture: None },
                ProbeKind::Irradiance => {
                    let [nx, ny, nz] = spec.bricks();
                    let size = Vec3::from_array(spec.size.map(f32::abs));
                    let mut points = Vec::new();
                    for z in 0..nz {
                        for y in 0..ny {
                            for x in 0..nx {
                                let local = Vec3::from_array(spec.brick_point(x, y, z)) * size;
                                points.push(centre + rotation * local);
                            }
                        }
                    }
                    let count = points.len();
                    Work::Irradiance {
                        points,
                        next: 0,
                        pending: HashMap::new(),
                        cubes: vec![None; count],
                    }
                }
            };
            baker.jobs.push(Job {
                stamp: probe::stamp(&engine.project, &actor),
                actor,
                name,
                spec,
                persist: persist && engine.project_dir.is_some(),
                work,
            });
        }
    }
    for job in &mut baker.jobs {
        let Some(place) = places.get(job.actor.as_str()) else {
            continue;
        };
        match &mut job.work {
            Work::Reflection { capture } => {
                if capture.is_none() {
                    let (_, rotation, position) = place.to_scale_rotation_translation();
                    let mut request =
                        ProbeRequest::reflection(position, job.spec.face_resolution());
                    request.near = 0.05;
                    // Faces follow the box, since Bevy samples a probe's cube
                    // in the probe's own frame.
                    request.rotation = rotation;
                    *capture = Some(service.request(request));
                }
            }
            Work::Irradiance {
                points,
                next,
                pending,
                ..
            } => {
                while pending.len() < BRICKS_IN_FLIGHT && *next < points.len() {
                    // World-aligned: the irradiance lookup uses the world
                    // normal.
                    let mut request = ProbeRequest::reflection(points[*next], BRICK_RESOLUTION);
                    request.near = 0.05;
                    pending.insert(service.request(request), *next);
                    *next += 1;
                }
            }
        }
    }
}

fn cancel(job: &Job, service: &mut ProbeService) {
    match &job.work {
        Work::Reflection { capture } => {
            if let Some(id) = capture {
                service.cancel(*id);
            }
        }
        Work::Irradiance { pending, .. } => {
            for id in pending.keys() {
                service.cancel(*id);
            }
        }
    }
}

/// Picks captures up as they land, and finishes the jobs they complete.
pub fn collect_bakes(
    engine: NonSend<Engine>,
    mut baker: ResMut<ProbeBaker>,
    mut loaded: ResMut<LoadedProbes>,
    mut images: ResMut<Assets<Image>>,
    mut captured: MessageReader<ProbeCaptured>,
) {
    let dir = engine.project_dir.clone();
    for message in captured.read() {
        let Some(cube) = message.cube.as_ref() else {
            continue;
        };
        let Some(image) = images.remove(cube) else {
            continue;
        };
        // Faces are exposed like the world camera; undo it back to radiance.
        let radiance = 2f32.powf(message.ev100) * 1.2;
        let Some((size, faces)) = cube_faces(&image, radiance) else {
            continue;
        };
        let Some(at) = baker.jobs.iter().position(|job| owns(job, message.id)) else {
            continue;
        };
        let done = match &mut baker.jobs[at].work {
            Work::Reflection { .. } => Some(Finished::Cube(HdrCube { size, faces })),
            Work::Irradiance {
                pending,
                cubes,
                points,
                ..
            } => {
                if let Some(index) = pending.remove(&message.id) {
                    cubes[index] = Some(probe::ambient_cube(&faces, size));
                }
                (pending.is_empty()
                    && cubes.iter().all(Option::is_some)
                    && cubes.len() == points.len())
                .then(|| Finished::Grid(cubes.iter().flatten().copied().collect()))
            }
        };
        let Some(done) = done else {
            continue;
        };
        let job = baker.jobs.remove(at);
        finish(job, done, dir.clone(), &mut loaded, &mut images);
    }
}

fn owns(job: &Job, id: ProbeId) -> bool {
    match &job.work {
        Work::Reflection { capture } => *capture == Some(id),
        Work::Irradiance { pending, .. } => pending.contains_key(&id),
    }
}

enum Finished {
    Cube(HdrCube),
    Grid(Vec<AmbientCube>),
}

fn finish(
    job: Job,
    done: Finished,
    dir: Option<PathBuf>,
    loaded: &mut LoadedProbes,
    images: &mut Assets<Image>,
) {
    let stamp = job.persist.then_some(job.stamp).flatten();
    let (image, kind) = match &done {
        Finished::Cube(cube) => (
            cube_image(cube.size, half_texels(cube)),
            ProbeKind::Reflection,
        ),
        Finished::Grid(cubes) => {
            let grid = IrradianceGrid {
                bricks: job.spec.bricks(),
                cubes: cubes.clone(),
            };
            (grid_image(&grid), ProbeKind::Irradiance)
        }
    };
    loaded.0.insert(
        job.actor.clone(),
        Loaded {
            kind,
            stamp,
            image: images.add(image),
        },
    );
    let (Some(dir), Some(stamp)) = (dir, stamp) else {
        return;
    };
    // Encoding a cube to BC6H takes a moment; keep it off the frame.
    let bricks = job.spec.bricks();
    AsyncComputeTaskPool::get()
        .spawn(async move {
            let written = match done {
                Finished::Cube(cube) => probe::write_cube(&dir, &job.actor, &cube, stamp),
                Finished::Grid(cubes) => {
                    probe::write_grid(&dir, &job.actor, &IrradianceGrid { bricks, cubes }, stamp)
                }
            };
            crate::bridge::send(&match written {
                Ok(()) => RuntimeMessage::Say {
                    actor: job.name.clone(),
                    text: "Baked this light probe".to_string(),
                },
                Err(message) => RuntimeMessage::Error {
                    actor: job.name.clone(),
                    message: format!("The probe's bake wasn't saved: {message}"),
                },
            });
        })
        .detach();
}

/// A read-back FP16 cube as six faces of linear RGB, scaled by `scale`.
fn cube_faces(image: &Image, scale: f32) -> Option<(u32, [Vec<[f32; 3]>; 6])> {
    let size = image.texture_descriptor.size.width;
    let data = image.data.as_ref()?;
    let face_bytes = (size * size * 8) as usize;
    if data.len() < face_bytes * 6 {
        return None;
    }
    let half = |bytes: &[u8]| half::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32() * scale;
    let faces = std::array::from_fn(|face| {
        data[face * face_bytes..(face + 1) * face_bytes]
            .chunks_exact(8)
            .map(|texel| [half(&texel[0..2]), half(&texel[2..4]), half(&texel[4..6])])
            .collect()
    });
    Some((size, faces))
}

fn half_texels(cube: &HdrCube) -> Vec<u8> {
    let one = half::f16::ONE.to_le_bytes();
    let mut texels = Vec::with_capacity((cube.size * cube.size * 8 * 6) as usize);
    for texel in cube.faces.iter().flatten() {
        for channel in texel {
            texels.extend_from_slice(&half::f16::from_f32(*channel).to_le_bytes());
        }
        texels.extend_from_slice(&one);
    }
    texels
}

fn cube_image(size: u32, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.label = Some("probe_cube");
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

/// The grid as the 3D texture Bevy's irradiance volumes sample.
fn grid_image(grid: &IrradianceGrid) -> Image {
    let (size, texels) = grid.atlas();
    let mut data = Vec::with_capacity(texels.len() * 8);
    for texel in texels {
        for channel in texel {
            data.extend_from_slice(&half::f16::from_f32(channel).to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: size[2],
        },
        TextureDimension::D3,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.label = Some("probe_irradiance");
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probes;
    use blockloom_core::components::ActorComponent;
    use blockloom_core::scene::Mode;

    fn app(kind: ProbeKind) -> (App, String) {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.rebuild = false;
        let actor = &mut engine.project.actors[0];
        actor.components.insert(ActorComponent::Probe {
            probe: ProbeSpec {
                kind,
                grid: [2, 1, 1],
                size: [4.0, 2.0, 2.0],
                ..ProbeSpec::default()
            },
        });
        let id = actor.id.clone();
        engine
            .attached
            .entry(id.clone())
            .or_default()
            .insert("Probe".to_string());
        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<Assets<Image>>()
            .init_resource::<crate::environment::Environment>();
        probes::register(&mut app);
        register(&mut app);
        app.add_systems(
            Update,
            (
                start_bakes,
                probes::run_captures,
                collect_bakes,
                sync_probes,
            )
                .chain(),
        );
        app.world_mut().spawn((
            ActorId(id.clone()),
            GlobalTransform::from(Transform::from_xyz(1.0, 2.0, 3.0)),
        ));
        (app, id)
    }

    /// Hands every capture in flight a uniform read-back, as the GPU would.
    fn deliver(app: &mut App, value: f32) {
        let world = app.world_mut();
        let live: Vec<ProbeId> = world.resource::<ProbeService>().live_ids();
        for id in live {
            let size = world.resource::<ProbeService>().resolution(id).unwrap();
            let texel: Vec<u8> = [value, value, value, 1.0]
                .iter()
                .flat_map(|c| half::f16::from_f32(*c).to_le_bytes())
                .collect();
            let face: Vec<u8> = texel.repeat((size * size) as usize);
            for f in 0..6 {
                world
                    .resource_mut::<ProbeService>()
                    .deliver_for_test(id, f, face.clone());
            }
        }
    }

    fn probe_entities(app: &mut App) -> Vec<Entity> {
        app.world_mut()
            .query_filtered::<Entity, With<ProbeOf>>()
            .iter(app.world())
            .collect()
    }

    #[test]
    fn capture_probes_lights_a_reflection_probe_in_memory() {
        let (mut app, _) = app(ProbeKind::Reflection);
        app.world_mut().non_send_mut::<Engine>().capture_probes = true;
        app.update();
        app.update();
        deliver(&mut app, 0.5);
        app.update();
        app.update();
        let probes = probe_entities(&mut app);
        assert_eq!(probes.len(), 1);
        let world = app.world();
        let transform = world.get::<Transform>(probes[0]).unwrap();
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform.scale, Vec3::new(4.0, 2.0, 2.0));
        assert!(
            world
                .get::<GeneratedEnvironmentMapLight>(probes[0])
                .is_some()
        );
        assert!(!world.resource::<ProbeBaker>().busy());
    }

    #[test]
    fn an_irradiance_probe_bakes_one_brick_per_cell() {
        let (mut app, _) = app(ProbeKind::Irradiance);
        app.world_mut()
            .resource_mut::<ProbeBaker>()
            .request(Vec::new(), false);
        app.update();
        assert_eq!(app.world().resource::<ProbeService>().in_flight(), 2);
        app.update();
        deliver(&mut app, 0.25);
        app.update();
        app.update();
        let probes = probe_entities(&mut app);
        assert_eq!(probes.len(), 1);
        let volume = app.world().get::<IrradianceVolume>(probes[0]).unwrap();
        let images = app.world().resource::<Assets<Image>>();
        let voxels = images.get(&volume.voxels).unwrap();
        assert_eq!(voxels.texture_descriptor.size.width, 2);
        assert_eq!(voxels.texture_descriptor.size.height, 2);
        assert_eq!(voxels.texture_descriptor.size.depth_or_array_layers, 3);
    }
}
