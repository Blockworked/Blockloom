//! Floating and splashing. A `Buoyancy` body is pushed up at its sample
//! points by what they displace and dragged towards the water's own motion;
//! anything with a rigid body that drops into water fast enough splashes:
//! droplets, a dip in the ripple field and the body's splash sound. A
//! floating body that moves stirs the ripples as it goes: its wake.

use super::{WaterSample, WaterState};
use crate::engine::{ActorId, Engine, PendingEffects};
use crate::fx::FxCache;
use bevy::prelude::*;
use blockloom_core::project::Actor;
use blockloom_core::scene::{Mode, Visual};
use blockloom_core::sound::SoundBus;
use blockloom_core::vm::Effect;
use blockloom_core::water::{BuoyancySpec, WaterSense, buoyant_force, submerged};

/// Half extents of an actor's look, scale included: what the sample points
/// spread over.
fn half_extents(actor: &Actor, scale: Vec3) -> Vec3 {
    let half = match actor.visual() {
        Some(Visual::Rect { size, .. } | Visual::Image { size, .. }) => {
            Vec3::new(size[0] * 0.5, size[1] * 0.5, 0.0)
        }
        Some(Visual::Circle { radius, .. } | Visual::Sphere { radius, .. }) => Vec3::splat(*radius),
        Some(Visual::Cuboid { size, .. }) => Vec3::from(*size) * 0.5,
        Some(Visual::Capsule { radius, height, .. }) => {
            Vec3::new(*radius, height * 0.5 + radius, *radius)
        }
        Some(Visual::Plane { size, .. }) => Vec3::new(size[0] * 0.5, 0.05, size[1] * 0.5),
        Some(Visual::Model { scale, .. }) => Vec3::from(*scale) * 0.5,
        _ => Vec3::splat(0.5),
    };
    half * scale.abs()
}

/// Half extents of a body's colliders as (half size, local centre) pairs, scale
/// included. `None` when it has none.
fn collider_extents(parts: impl Iterator<Item = (Vec3, Vec3)>, scale: Vec3) -> Option<Vec3> {
    let scale = scale.abs();
    parts
        .map(|(half, at)| (at.abs() * scale) + half)
        .reduce(Vec3::max)
        .map(|half| half.max(Vec3::splat(1.0e-3)))
}

/// What the water does to one body this tick.
#[derive(Debug, Default, PartialEq)]
struct Float {
    force: Vec3,
    torque: Vec3,
    /// 0-1: how much of it is under.
    under: f32,
    /// Where it cuts the surface: body, point, and how fast it moves
    /// through the water there.
    stir: Vec<(String, Vec3, f32)>,
}

/// Sums the push and drag at each of `spec`'s sample points.
#[allow(clippy::too_many_arguments)]
fn float(
    sense: &WaterSense,
    spec: &BuoyancySpec,
    pose: &Transform,
    half: Vec3,
    flat: bool,
    mass: f32,
    gravity: f32,
    linvel: Vec3,
    angvel: Vec3,
    com: Vec3,
) -> Float {
    let points = spec.sample_points(half.to_array(), flat);
    let span = if spec.points == 4 && !flat || spec.points == 1 {
        half.y
    } else {
        half.y * 0.5
    }
    .max(1.0e-3);
    let mut out = Float::default();
    for local in &points {
        let at = pose.translation + pose.rotation * Vec3::from(*local);
        let z = if flat { 0.0 } else { at.z };
        let Some((body, water)) = sense.surface_at(at.x, z) else {
            continue;
        };
        if at.y < body.floor() {
            continue;
        }
        let under = submerged(water.height - at.y, span);
        if under <= 0.0 {
            continue;
        }
        let lift = buoyant_force(mass, gravity, spec.density, points.len(), under);
        let moving = linvel + angvel.cross(at - com);
        let mut drag = (Vec3::from(water.velocity) - moving)
            * (mass / points.len() as f32 * spec.drag * under);
        if flat {
            drag.z = 0.0;
        }
        if under < 1.0 {
            let mut through = moving - Vec3::from(water.velocity);
            through.y = 0.0;
            out.stir.push((body.id.clone(), at, through.length()));
        }
        let force = Vec3::Y * lift + drag;
        out.force += force;
        out.torque += (at - com).cross(force);
        out.under += under / points.len() as f32;
    }
    out
}

/// A body's depth at its centre, for splashes: positive under.
fn centre_depth(sense: &WaterSense, at: Vec3, flat: bool) -> Option<(String, f32)> {
    let z = if flat { 0.0 } else { at.z };
    let (body, water) = sense.surface_at(at.x, z)?;
    (at.y >= body.floor()).then(|| (body.id.clone(), water.height - at.y))
}

/// Everything a splash needs besides the entity queries.
struct Splasher<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    mode: Mode,
    cache: &'a mut FxCache,
    meshes: &'a mut Assets<Mesh>,
    materials: &'a mut Assets<StandardMaterial>,
    effects: &'a mut PendingEffects,
    seed: &'a mut u64,
}

/// Splashes `actor` into `water` if it just went in fast enough.
#[allow(clippy::too_many_arguments)]
fn maybe_splash(
    engine: &Engine,
    state: &mut WaterState,
    sense: &WaterSense,
    splasher: &mut Splasher,
    entity: Entity,
    actor: &str,
    at: Vec3,
    half: Vec3,
    fall: f32,
    enabled: bool,
) {
    let flat = splasher.mode == Mode::TwoD;
    let depth = centre_depth(sense, at, flat);
    let wet = depth.as_ref().is_some_and(|(_, depth)| *depth > 0.0);
    let was = state.wet.insert(entity, wet);
    let Some((water, _)) = depth else {
        return;
    };
    // Only a crossing splashes, and the first tick of a run counts as dry.
    if !wet || was != Some(false) || !enabled || water == actor {
        return;
    }
    let Some(spec) = engine.actor(&water).and_then(|a| a.components.water()) else {
        return;
    };
    let splash = &spec.splash;
    if fall < splash.min_speed.max(1.0e-3) {
        return;
    }
    let hard = (fall / splash.min_speed.max(1.0e-3)).clamp(1.0, 3.0);
    let surface = sense.height_at(at.x, if flat { 0.0 } else { at.z });
    let point = Vec3::new(at.x, surface.unwrap_or(at.y), at.z);
    let size = half.x.max(half.z).max(1.0e-3);
    if splash.particles > 0 {
        let color = crate::world::parse_color(&spec.foam.color);
        crate::fx::spawn_splash(
            splasher.commands,
            splasher.mode,
            splasher.cache,
            splasher.meshes,
            splasher.materials,
            point,
            (splash.particles as f32 * hard.min(2.0)) as u32,
            fall * 0.5,
            size * 0.25,
            color,
            splasher.seed,
        );
    }
    if splash.ripples && spec.ripples.enabled {
        state.disturb(&water, [point.x, point.z], size, -size * 0.25 * hard, false);
    }
    splasher.effects.0.push(Effect::Splash {
        at: point.to_array(),
        radius: size,
        strength: 0.0,
    });
    if !splash.sound.is_empty() {
        splasher.effects.0.push(Effect::PlaySound {
            actor: water,
            sound: splash.sound.clone(),
            volume: (0.35 * hard).min(1.0),
            pitch: 1.0,
            loop_: false,
            bus: SoundBus::Sfx,
            at: Some(actor.to_string()),
        });
    }
}

/// A floating body's wake: each point cutting the surface pushes the water
/// down in proportion to how fast it moves through it.
fn stir(engine: &Engine, state: &mut WaterState, result: &Float, half: Vec3, dt: f32) {
    let radius = half.x.max(half.z).max(1.0e-3) * 0.5;
    for (body, at, speed) in &result.stir {
        let Some(spec) = engine.actor(body).and_then(|a| a.components.water()) else {
            continue;
        };
        let ripples = &spec.ripples;
        if !ripples.enabled || ripples.wake <= 0.0 || *speed <= 0.0 {
            continue;
        }
        // Capped so a speeding boat doesn't dig a hole.
        let push = (speed * dt * 0.2 * ripples.wake).min(radius * 0.1);
        state.disturb(body, [at.x, at.z], radius, -push, true);
    }
}

macro_rules! float_bodies {
    ($name:ident, $rp:ident, $mode:expr, $lin:expr, $ang:expr, $com:expr, $shape:expr, $apply:expr) => {
        /// Pushes and drags every floating body, and splashes whatever
        /// just went in. Impulses go through rapier's own step.
        #[allow(clippy::too_many_arguments)]
        pub fn $name(
            mut commands: Commands,
            engine: NonSend<Engine>,
            time: Res<Time<Fixed>>,
            sample: Res<WaterSample>,
            mut state: ResMut<WaterState>,
            mut effects: ResMut<PendingEffects>,
            mut bodies: Query<(
                Entity,
                &ActorId,
                &Transform,
                &mut $rp::prelude::Velocity,
                &mut $rp::prelude::ExternalImpulse,
                Option<&$rp::prelude::ReadMassProperties>,
                &$rp::prelude::RigidBody,
                Option<&Children>,
            )>,
            colliders: Query<(&$rp::prelude::Collider, &Transform)>,
            mut cache: ResMut<FxCache>,
            mut meshes: ResMut<Assets<Mesh>>,
            mut materials: ResMut<Assets<StandardMaterial>>,
            mut seed: Local<u64>,
        ) {
            if !engine.running || engine.paused || sample.0.bodies.is_empty() {
                return;
            }
            let dt = time.delta_secs();
            let flat = $mode == Mode::TwoD;
            let gravity = super::gravity_of(&engine, flat);
            let mut splasher = Splasher {
                commands: &mut commands,
                mode: $mode,
                cache: &mut cache,
                meshes: &mut meshes,
                materials: &mut materials,
                effects: &mut effects,
                seed: &mut seed,
            };
            for (entity, id, pose, mut velocity, mut impulse, mass, kind, children) in &mut bodies {
                let Some(actor) = engine.actor(&id.0) else {
                    continue;
                };
                // The body's colliders decide its size, so a body with no Look floats too.
                let half = collider_extents(
                    children.into_iter().flatten().filter_map(|child| {
                        colliders
                            .get(*child)
                            .ok()
                            .map(|(c, t)| (($shape)(c), t.translation))
                    }),
                    pose.scale,
                )
                .unwrap_or_else(|| half_extents(actor, pose.scale));
                let linvel: Vec3 = ($lin)(&*velocity);
                let buoyancy = engine
                    .has_component(&id.0, "Buoyancy")
                    .then(|| actor.components.buoyancy())
                    .flatten();
                let enabled = buoyancy.is_none_or(|spec| spec.splash);
                maybe_splash(
                    &engine,
                    &mut state,
                    &sample.0,
                    &mut splasher,
                    entity,
                    &id.0,
                    pose.translation,
                    half,
                    -linvel.y,
                    enabled,
                );
                let Some(spec) = buoyancy else {
                    continue;
                };
                if *kind != $rp::prelude::RigidBody::Dynamic {
                    continue;
                }
                // Mass properties are read back after rapier's first step.
                let Some(mass) = mass else {
                    splasher
                        .commands
                        .entity(entity)
                        .insert($rp::prelude::ReadMassProperties::default());
                    continue;
                };
                let props = mass.get();
                if props.mass <= 0.0 {
                    continue;
                }
                let com = pose.translation + pose.rotation * ($com)(props.local_center_of_mass);
                let angvel: Vec3 = ($ang)(&*velocity);
                let result = float(
                    &sample.0, spec, pose, half, flat, props.mass, gravity, linvel, angvel, com,
                );
                if result.under <= 0.0 {
                    continue;
                }
                stir(&engine, &mut state, &result, half, dt);
                ($apply)(
                    &mut *impulse,
                    &mut *velocity,
                    &result,
                    dt,
                    spec.angular_drag,
                );
            }
        }
    };
}

float_bodies!(
    float_bodies_3d,
    bevy_rapier3d,
    Mode::ThreeD,
    |v: &bevy_rapier3d::prelude::Velocity| v.linear,
    |v: &bevy_rapier3d::prelude::Velocity| v.angular,
    |c: Vec3| c,
    |c: &bevy_rapier3d::prelude::Collider| c.raw.compute_local_aabb().half_extents(),
    |impulse: &mut bevy_rapier3d::prelude::ExternalImpulse,
     velocity: &mut bevy_rapier3d::prelude::Velocity,
     result: &Float,
     dt: f32,
     angular: f32| {
        impulse.impulse += result.force * dt;
        impulse.torque_impulse += result.torque * dt;
        velocity.angular *= (-angular * result.under * dt).exp();
    }
);

float_bodies!(
    float_bodies_2d,
    bevy_rapier2d,
    Mode::TwoD,
    |v: &bevy_rapier2d::prelude::Velocity| v.linear.extend(0.0),
    |v: &bevy_rapier2d::prelude::Velocity| Vec3::Z * v.angular,
    |c: Vec2| c.extend(0.0),
    |c: &bevy_rapier2d::prelude::Collider| {
        c.raw.compute_local_aabb().half_extents().extend(0.0) * crate::dim2::PIXELS_PER_METER
    },
    |impulse: &mut bevy_rapier2d::prelude::ExternalImpulse,
     velocity: &mut bevy_rapier2d::prelude::Velocity,
     result: &Float,
     dt: f32,
     angular: f32| {
        impulse.impulse += result.force.truncate() * dt;
        impulse.torque_impulse += result.torque.z * dt;
        velocity.angular *= (-angular * result.under * dt).exp();
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::water::{WaterBody, WaterKind};

    fn calm(level: f32, flat: bool) -> WaterSense {
        WaterSense {
            bodies: vec![WaterBody {
                id: "sea".to_string(),
                kind: WaterKind::Ocean,
                center: [0.0, level, 0.0],
                axis: [1.0, 0.0],
                half: [f32::INFINITY; 2],
                depth: 100.0,
                flow: [0.0, 0.0],
                waves: Vec::new(),
                flat,
                calm: [0.0; 3],
                ripples: None,
            }],
            time: 0.0,
        }
    }

    fn push(y: f32, density: f32) -> Float {
        let spec = BuoyancySpec {
            density,
            ..BuoyancySpec::default()
        };
        float(
            &calm(0.0, false),
            &spec,
            &Transform::from_xyz(0.0, y, 0.0),
            Vec3::splat(0.5),
            false,
            2.0,
            9.81,
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::new(0.0, y, 0.0),
        )
    }

    #[test]
    fn colliders_size_a_body_with_no_look() {
        let half = collider_extents(
            [
                (Vec3::new(1.0, 0.5, 0.5), Vec3::ZERO),
                (Vec3::new(0.25, 0.25, 0.25), Vec3::new(2.0, 0.0, 0.0)),
            ]
            .into_iter(),
            Vec3::splat(2.0),
        )
        .unwrap();
        assert_eq!(half, Vec3::new(4.25, 0.5, 0.5));
        assert!(collider_extents(std::iter::empty(), Vec3::ONE).is_none());
    }

    #[test]
    fn half_under_holds_a_half_density_body_up() {
        let result = push(0.0, 0.5);
        assert!((result.under - 0.5).abs() < 1e-4);
        // Its weight, exactly.
        assert!((result.force.y - 2.0 * 9.81).abs() < 1e-3, "{:?}", result);
        assert!(result.torque.length() < 1e-4);
    }

    #[test]
    fn nothing_pushes_a_body_out_of_the_water() {
        assert_eq!(push(3.0, 0.5), Float::default());
        // Cutting the surface stirs it; sunk deep doesn't.
        assert_eq!(push(0.0, 0.5).stir.len(), 4);
        assert!(push(-5.0, 0.5).stir.is_empty());
        // Deeper pushes harder, up to fully under.
        assert!(push(-0.25, 0.5).force.y > push(0.0, 0.5).force.y);
        let sunk = push(-5.0, 0.5);
        assert!((sunk.under - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_tilted_body_is_turned_back_level() {
        let spec = BuoyancySpec::default();
        let pose = Transform::from_rotation(Quat::from_rotation_z(0.3));
        let result = float(
            &calm(0.0, false),
            &spec,
            &pose,
            Vec3::splat(0.5),
            false,
            1.0,
            9.81,
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::ZERO,
        );
        // Rolled positive about z, so the push rolls it back.
        assert!(result.torque.z < 0.0, "{:?}", result);
    }

    #[test]
    fn drag_pulls_towards_the_water() {
        let spec = BuoyancySpec::default();
        let result = float(
            &calm(0.0, true),
            &spec,
            &Transform::default(),
            Vec3::new(20.0, 20.0, 0.0),
            true,
            1.0,
            981.0,
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        assert!(result.force.x < 0.0);
        assert_eq!(result.force.z, 0.0);
    }
}
