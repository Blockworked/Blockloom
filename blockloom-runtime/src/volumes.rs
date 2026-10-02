//! Environment volumes in the world: which ones cover the camera and how
//! much, handed to `blend_environment` lowest priority first. Also their
//! debug views - bounds and the frozen lerp (the heat map is `volume_heat`).

use crate::bridge;
use crate::edit::{SceneEditor, editing};
use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use crate::environment::{Environment, EnvironmentOverride, EnvironmentVolumes, ExposureClaims};
use crate::world::WorldCamera;
use bevy::prelude::*;
use blockloom_core::scene::Mode;
use blockloom_core::vm::Effect;
use blockloom_core::volume::{VolumePose, VolumeShape, VolumeSpec, blend_order};
use blockloom_protocol::{
    RuntimeMessage, VolumeDebug, VolumeStatus, VolumeTraceRow, VolumeTraceStep,
};

pub fn register(app: &mut App) {
    app.init_resource::<VolumeBlend>()
        .init_resource::<VolumeEye>()
        .init_resource::<VolumeDebugView>();
}

/// Where volumes are weighed. A game with a window weighs them at its world
/// camera; a server has no camera, so it names an actor or a point instead and
/// the weights don't depend on what any one client is looking at.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub enum VolumeEye {
    #[default]
    Camera,
    Actor(String),
    Point(Vec3),
}

/// The editor's volume debug settings.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeDebugView(pub VolumeDebug);

/// One volume showing at the camera.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveVolume {
    pub actor: String,
    pub name: String,
    pub priority: f32,
    pub coverage: f32,
    /// Coverage times the volume's own weight: what it blends at.
    pub weight: f32,
    pub over: EnvironmentOverride,
    pub overrides: Vec<&'static str>,
}

/// This frame's volumes at the camera, in blend order.
#[derive(Resource, Clone, Debug, Default)]
pub struct VolumeBlend {
    pub active: Vec<ActiveVolume>,
    /// Filled while frozen: each property's lerp on the frame it froze.
    pub trace: Vec<VolumeTraceRow>,
}

impl VolumeBlend {
    /// What `active volumes` reports.
    pub fn names(&self) -> Vec<String> {
        self.active.iter().map(|v| v.name.clone()).collect()
    }

    pub fn statuses(&self) -> Vec<VolumeStatus> {
        self.active
            .iter()
            .map(|v| VolumeStatus {
                actor: v.actor.clone(),
                name: v.name.clone(),
                priority: v.priority,
                coverage: v.coverage,
                weight: v.weight,
                overrides: v.overrides.iter().map(|s| s.to_string()).collect(),
            })
            .collect()
    }
}

/// An actor's volume as it stands this run: carried right now, with what
/// the blocks switched and weighed since Play. One attached mid-run that the
/// editor never authored has the defaults.
pub fn live_spec(engine: &Engine, id: &str) -> Option<VolumeSpec> {
    if !engine.has_component(id, "Volume") {
        return None;
    }
    let mut spec = engine
        .actor(id)?
        .components
        .volume()
        .cloned()
        .unwrap_or_default();
    if let Some(enabled) = engine.volume_enabled.get(id) {
        spec.enabled = *enabled;
    }
    if let Some(weight) = engine.volume_weight.get(id) {
        spec.weight = *weight;
    }
    Some(spec)
}

fn pose_of(transform: &Transform) -> VolumePose {
    VolumePose {
        position: transform.translation.to_array(),
        rotation: transform.rotation.to_array(),
        scale: transform.scale.to_array(),
    }
}

/// Every switched-on volume in the world, with where it stands.
pub(crate) fn placed<'a>(
    engine: &'a Engine,
    actors: impl Iterator<Item = (&'a ActorId, &'a Transform)>,
) -> Vec<(&'a str, VolumeSpec, VolumePose)> {
    actors
        .filter_map(|(id, transform)| {
            let spec = live_spec(engine, &id.0)?;
            (spec.enabled && spec.weight > 0.0).then(|| (id.0.as_str(), spec, pose_of(transform)))
        })
        .collect()
}

/// `enable volume` and `set weight of volume` for the rest of the run.
pub fn apply_volume_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        let (actor, volume) = match effect {
            Effect::SetVolumeEnabled { actor, volume, .. }
            | Effect::SetVolumeWeight { actor, volume, .. } => (actor, volume),
            _ => continue,
        };
        let targets = targets(&engine, actor, volume);
        if targets.is_empty() {
            bridge::send(&RuntimeMessage::Error {
                actor: actor.clone(),
                message: format!("there's no volume named \"{volume}\""),
            });
            continue;
        }
        for id in targets {
            match effect {
                Effect::SetVolumeEnabled { enabled, .. } => {
                    engine.volume_enabled.insert(id, *enabled);
                }
                Effect::SetVolumeWeight { weight, .. } if weight.is_finite() => {
                    engine.volume_weight.insert(id, weight.clamp(0.0, 1.0));
                }
                _ => {}
            }
        }
    }
}

/// Which actors a block means: itself for an empty slot, then an id, then
/// every actor answering to the name - a clone of a volume is one too.
fn targets(engine: &Engine, running: &str, wanted: &str) -> Vec<String> {
    let wanted = wanted.trim();
    if wanted.is_empty()
        || wanted.eq_ignore_ascii_case("myself")
        || wanted.eq_ignore_ascii_case("me")
    {
        return vec![running.to_string()];
    }
    if engine.entities.contains_key(wanted) {
        return vec![wanted.to_string()];
    }
    let mut ids: Vec<String> = engine
        .actor_ids()
        .filter(|id| {
            engine
                .actor(id)
                .is_some_and(|actor| actor.name.eq_ignore_ascii_case(wanted))
        })
        .cloned()
        .collect();
    ids.sort();
    ids
}

/// Weighs every volume at the camera and hands them to the blend. Frozen,
/// it keeps last frame's list and records the lerp once.
#[allow(clippy::too_many_arguments)]
pub fn gather_volumes(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    debug: Res<VolumeDebugView>,
    claims: Res<ExposureClaims>,
    eye: Res<VolumeEye>,
    cameras: Query<&Transform, With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform), Without<WorldCamera>>,
    mut blend: ResMut<VolumeBlend>,
    mut volumes: ResMut<EnvironmentVolumes>,
) {
    if debug.0.freeze {
        if blend.trace.is_empty() {
            let base = Environment::from_world(&engine.project.world);
            blend.trace = trace(&base, &blend.active, &claims);
        }
        return;
    }
    blend.trace.clear();
    let at = match &*eye {
        VolumeEye::Camera => cameras.iter().next().map(|camera| camera.translation),
        VolumeEye::Actor(id) => actors
            .iter()
            .find(|(actor, _)| actor.0 == *id)
            .map(|(_, transform)| transform.translation),
        VolumeEye::Point(point) => Some(*point),
    };
    let Some(eye) = at else {
        blend.active.clear();
        volumes.0.clear();
        return;
    };
    blend.active = weigh(&engine, dimension.0, eye, actors.iter());
    volumes.0 = blend
        .active
        .iter()
        .map(|volume| (volume.weight, volume.over.clone()))
        .collect();
}

fn weigh<'a>(
    engine: &'a Engine,
    mode: Mode,
    eye: Vec3,
    actors: impl Iterator<Item = (&'a ActorId, &'a Transform)>,
) -> Vec<ActiveVolume> {
    let flat = mode == Mode::TwoD;
    let mut active: Vec<ActiveVolume> = placed(engine, actors)
        .into_iter()
        .filter_map(|(id, spec, pose)| {
            let coverage = spec.coverage(&pose, eye.to_array(), flat);
            let weight = coverage * spec.weight.clamp(0.0, 1.0);
            (weight > 0.0).then(|| ActiveVolume {
                actor: id.to_string(),
                name: engine
                    .actor(id)
                    .map_or_else(|| id.to_string(), |a| a.name.clone()),
                priority: spec.priority,
                coverage,
                weight,
                over: EnvironmentOverride::from_volume(&spec.overrides),
                overrides: spec.overrides.checked(),
            })
        })
        .collect();
    blend_order(&mut active, |v| (v.priority, v.actor.as_str()));
    active
}

/// Each property from the project's value through every volume that
/// touches it, the way `blend_environment` walks them.
fn trace(
    base: &Environment,
    active: &[ActiveVolume],
    claims: &ExposureClaims,
) -> Vec<VolumeTraceRow> {
    let mut rows: Vec<VolumeTraceRow> = base
        .readings()
        .into_iter()
        .map(|(property, value)| VolumeTraceRow {
            property: property.to_string(),
            base: value,
            steps: Vec::new(),
            result: String::new(),
        })
        .collect();
    let mut env = base.clone();
    for volume in active {
        env.blend(&volume.over, volume.weight);
        let after = env.readings();
        for (property, target) in volume.over.readings() {
            let Some(row) = rows.iter_mut().find(|row| row.property == property) else {
                continue;
            };
            let after = after
                .iter()
                .find(|(name, _)| *name == property)
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            row.steps.push(VolumeTraceStep {
                volume: volume.name.clone(),
                weight: volume.weight,
                target,
                after,
            });
        }
    }
    env.exposure = claims.resolve(env.exposure);
    for (row, (_, value)) in rows.iter_mut().zip(env.readings()) {
        row.result = value;
    }
    rows
}

const COLD: Color = Color::srgba(0.45, 0.6, 0.85, 0.5);
const HOT: Color = Color::srgba(1.0, 0.6, 0.15, 0.9);

/// Volume bounds in the scene view. The heat map is `volume_heat`'s pass.
pub fn draw_volumes(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    debug: Res<VolumeDebugView>,
    editor: Option<Res<SceneEditor>>,
    blend: Res<VolumeBlend>,
    actors: Query<(&ActorId, &Transform), Without<WorldCamera>>,
    mut gizmos: Gizmos,
) {
    let mode = dimension.0;
    let editing = editor.is_some_and(|editor| editing(&engine, &editor));
    let volumes = placed(&engine, actors.iter());
    if debug.0.bounds && editing {
        for (id, spec, pose) in &volumes {
            let weight = blend
                .active
                .iter()
                .find(|v| v.actor == *id)
                .map_or(0.0, |v| v.weight);
            let color = COLD.mix(&HOT, weight);
            draw_bounds(&mut gizmos, spec, pose, mode, color);
        }
    }
}

fn draw_bounds(
    gizmos: &mut Gizmos,
    spec: &VolumeSpec,
    pose: &VolumePose,
    mode: Mode,
    color: Color,
) {
    let position = Vec3::from(pose.position);
    let rotation = Quat::from_array(pose.rotation);
    let scale = Vec3::from(pose.scale).abs();
    let blend = spec.blend_distance.max(0.0);
    let feather = color.with_alpha(color.alpha() * 0.35);
    match (spec.shape, mode) {
        (VolumeShape::Global, _) => {
            gizmos.cross(Isometry3d::from_translation(position), 0.5, color);
        }
        (VolumeShape::Sphere, Mode::ThreeD) => {
            let radius = spec.radius.max(0.0) * scale.max_element();
            gizmos.sphere(Isometry3d::from_translation(position), radius, color);
            if blend > 0.0 {
                gizmos.sphere(
                    Isometry3d::from_translation(position),
                    radius + blend,
                    feather,
                );
            }
        }
        (VolumeShape::Sphere, Mode::TwoD) => {
            let radius = spec.radius.max(0.0) * scale.max_element();
            gizmos
                .circle(Isometry3d::from_translation(position), radius, color)
                .resolution(64);
            if blend > 0.0 {
                gizmos
                    .circle(
                        Isometry3d::from_translation(position),
                        radius + blend,
                        feather,
                    )
                    .resolution(64);
            }
        }
        (VolumeShape::Box, Mode::ThreeD) => {
            let half = Vec3::from(spec.half_extents).max(Vec3::ZERO) * scale;
            gizmos.cube(
                Transform {
                    translation: position,
                    rotation,
                    scale: half * 2.0,
                },
                color,
            );
            if blend > 0.0 {
                gizmos.cube(
                    Transform {
                        translation: position,
                        rotation,
                        scale: (half + Vec3::splat(blend)) * 2.0,
                    },
                    feather,
                );
            }
        }
        (VolumeShape::Box, Mode::TwoD) => {
            let half = (Vec3::from(spec.half_extents).max(Vec3::ZERO) * scale).truncate();
            gizmos.rect(Isometry3d::new(position, rotation), half * 2.0, color);
            if blend > 0.0 {
                gizmos.rect(
                    Isometry3d::new(position, rotation),
                    (half + Vec2::splat(blend)) * 2.0,
                    feather,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::components::ActorComponent;
    use blockloom_core::project::Actor;
    use blockloom_core::scene::Visual;
    use blockloom_core::volume::{Override, VolumeOverrides};

    fn volume(name: &str, priority: f32, exposure: f32) -> Actor {
        let mut actor = Actor::new(
            name,
            Visual::Rect {
                color: "#FFFFFF".into(),
                size: [1.0, 1.0],
            },
        );
        actor.components.insert(ActorComponent::Volume {
            volume: VolumeSpec {
                priority,
                half_extents: [2.0; 3],
                blend_distance: 2.0,
                overrides: VolumeOverrides {
                    exposure: Override {
                        on: true,
                        value: exposure,
                    },
                    ..VolumeOverrides::default()
                },
                ..VolumeSpec::default()
            },
        });
        actor
    }

    fn app(actors: Vec<(Actor, Vec3)>) -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.project.actors.clear();
        let mut app = App::new();
        app.insert_resource(Dimension(Mode::ThreeD))
            .init_resource::<ExposureClaims>()
            .init_resource::<EnvironmentVolumes>()
            .init_resource::<Environment>()
            .init_resource::<PendingEffects>();
        register(&mut app);
        for (actor, at) in actors {
            let entity = app
                .world_mut()
                .spawn((ActorId(actor.id.clone()), Transform::from_translation(at)))
                .id();
            engine.entities.insert(actor.id.clone(), entity);
            engine.attached.insert(
                actor.id.clone(),
                actor
                    .components
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect(),
            );
            engine.project.actors.push(actor);
        }
        app.insert_non_send(engine);
        app.world_mut().spawn((WorldCamera, Transform::default()));
        app.add_systems(
            Update,
            (
                apply_volume_effects,
                gather_volumes,
                crate::environment::blend_environment,
            )
                .chain(),
        );
        app
    }

    fn camera_to(app: &mut App, at: Vec3) {
        let mut cameras = app
            .world_mut()
            .query_filtered::<&mut Transform, With<WorldCamera>>();
        for mut transform in cameras.iter_mut(app.world_mut()) {
            transform.translation = at;
        }
    }

    #[test]
    fn a_volume_blends_in_by_where_the_camera_stands() {
        let base = Environment::default().exposure;
        let mut app = app(vec![(volume("Cave", 0.0, base + 4.0), Vec3::ZERO)]);
        app.update();
        assert!((app.world().resource::<Environment>().exposure - (base + 4.0)).abs() < 1e-4);

        // Half way across the feather.
        camera_to(&mut app, Vec3::new(3.0, 0.0, 0.0));
        app.update();
        assert!((app.world().resource::<Environment>().exposure - (base + 2.0)).abs() < 1e-4);

        camera_to(&mut app, Vec3::new(10.0, 0.0, 0.0));
        app.update();
        assert_eq!(app.world().resource::<Environment>().exposure, base);
        assert!(app.world().resource::<VolumeBlend>().active.is_empty());
    }

    #[test]
    fn a_world_without_a_camera_weighs_volumes_at_its_named_eye() {
        let base = Environment::default().exposure;
        let mut app = app(vec![(volume("Cave", 0.0, base + 4.0), Vec3::ZERO)]);
        // No camera at all, the way a server runs.
        let cameras: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<WorldCamera>>()
            .iter(app.world())
            .collect();
        for camera in cameras {
            app.world_mut().despawn(camera);
        }
        app.update();
        assert!(app.world().resource::<VolumeBlend>().active.is_empty());

        *app.world_mut().resource_mut::<VolumeEye>() = VolumeEye::Point(Vec3::new(3.0, 0.0, 0.0));
        app.update();
        assert!((app.world().resource::<Environment>().exposure - (base + 2.0)).abs() < 1e-4);

        // An actor as the eye: the volume's own actor stands in the volume.
        let id = app.world().non_send::<Engine>().project.actors[0]
            .id
            .clone();
        *app.world_mut().resource_mut::<VolumeEye>() = VolumeEye::Actor(id);
        app.update();
        assert!((app.world().resource::<Environment>().exposure - (base + 4.0)).abs() < 1e-4);
    }

    #[test]
    fn the_higher_priority_wins_where_two_overlap() {
        let mut app = app(vec![
            (volume("High", 5.0, 3.0), Vec3::ZERO),
            (volume("Low", 1.0, 13.0), Vec3::ZERO),
        ]);
        app.update();
        assert_eq!(app.world().resource::<Environment>().exposure, 3.0);
        assert_eq!(
            app.world().resource::<VolumeBlend>().names(),
            vec!["Low", "High"]
        );
    }

    #[test]
    fn blocks_switch_and_weigh_a_volume_by_name() {
        let base = Environment::default().exposure;
        let mut app = app(vec![(volume("Cave", 0.0, base + 4.0), Vec3::ZERO)]);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::SetVolumeWeight {
            actor: "someone".into(),
            volume: "cave".into(),
            weight: 0.25,
        }];
        app.update();
        assert!((app.world().resource::<Environment>().exposure - (base + 1.0)).abs() < 1e-4);

        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::SetVolumeEnabled {
            actor: "someone".into(),
            volume: "Cave".into(),
            enabled: false,
        }];
        app.update();
        assert_eq!(app.world().resource::<Environment>().exposure, base);
    }

    #[test]
    fn freezing_holds_the_blend_and_traces_each_lerp() {
        let base = Environment::default().exposure;
        let mut app = app(vec![(volume("Cave", 0.0, base + 4.0), Vec3::ZERO)]);
        camera_to(&mut app, Vec3::new(3.0, 0.0, 0.0));
        app.update();
        app.world_mut().resource_mut::<VolumeDebugView>().0.freeze = true;
        camera_to(&mut app, Vec3::new(50.0, 0.0, 0.0));
        app.update();
        assert!((app.world().resource::<Environment>().exposure - (base + 2.0)).abs() < 1e-4);

        let trace = &app.world().resource::<VolumeBlend>().trace;
        let exposure = trace.iter().find(|row| row.property == "exposure").unwrap();
        assert_eq!(exposure.steps.len(), 1);
        assert_eq!(exposure.steps[0].volume, "Cave");
        assert!((exposure.steps[0].weight - 0.5).abs() < 1e-5);
        assert_eq!(exposure.result, format!("{:.3}", base + 2.0));
        let sky = trace
            .iter()
            .find(|row| row.property == "background")
            .unwrap();
        assert!(sky.steps.is_empty());
    }
}
