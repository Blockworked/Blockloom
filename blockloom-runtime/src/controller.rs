//! The Rapier half of the CharacterController: installs each controller's
//! capsule and sweeps it through the simulation's world.
//!
//! Core decides the contract (see `blockloom_core::physics::controller`); this
//! module answers [`ControllerService`] with `KinematicCharacterController`
//! over the same query pipeline, layer matrix and trigger rules queries use.
//! Moves are swept on the spot from the settled pose plus what earlier moves of
//! the step already took; [`apply_motion`] then writes the summed displacement
//! to the actor's transform once, before the physics step.

use crate::physics_install::{PhysicsLayers, PlannedCollider};
use crate::queries::Lookups;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use blockloom_core::physics::ColliderId;
use blockloom_core::physics::controller::{
    CharacterControllerSpec, ControllerService, RawHit, StepOutcome, StepRequest,
};
use blockloom_core::physics::query::{QueryFilter, TriggerPolicy};
use blockloom_core::scene::Mode;
use std::collections::{HashMap, HashSet};

/// The entities of this world's controllers, by actor.
#[derive(Resource, Default, Clone)]
pub struct ControllerEntities {
    pub actors: HashMap<String, Entity>,
    /// The capsule collider, a child of the actor.
    pub capsules: HashMap<String, Entity>,
}

/// Marks a controller's capsule collider.
#[derive(Component, Debug, Clone)]
pub struct ControllerCapsule;

/// The id a controller's capsule goes by in contacts and queries.
pub fn capsule_id(actor: &str) -> ColliderId {
    ColliderId::from(format!("{actor}#controller").as_str())
}

/// What a sweep may hit: solid things the controller's layer rules allow.
fn sweep_filter(actor: &str) -> QueryFilter {
    QueryFilter {
        triggers: TriggerPolicy::Ignore,
        ..QueryFilter::as_actor(actor)
    }
}

/// Registers every controller of a plan and forgets the last world's.
pub fn register_plan(plan: &blockloom_core::physics::PhysicsPlan) {
    blockloom_core::physics::controller::reset();
    for planned in &plan.controllers {
        blockloom_core::physics::controller::register(&planned.actor, planned.spec.clone());
    }
}

/// Forgets every controller: the world is going away.
pub fn clear() {
    blockloom_core::physics::controller::reset();
}

fn tilt_3d(up: [f32; 3]) -> Quat {
    Quat::from_rotation_arc(Vec3::Y, Vec3::from(up))
}

fn tilt_2d(up: [f32; 3]) -> Quat {
    Quat::from_rotation_z((-up[0]).atan2(up[1]))
}

/// A system parameter for both dimensions' controller service.
#[derive(SystemParam)]
pub struct ControllerAccess<'w, 's> {
    two: d2::Access2<'w, 's>,
    three: d3::Access3<'w, 's>,
    layers: Option<Res<'w, PhysicsLayers>>,
}

impl ControllerAccess<'_, '_> {
    /// Runs `f` with controllers moving through the world as it stands.
    pub fn scope<R>(&self, tick: u64, f: impl FnOnce() -> R) -> R {
        use blockloom_core::physics::controller::with_service;
        match self.layers.as_ref().and_then(|layers| layers.mode) {
            Some(Mode::TwoD) => with_service(&self.two.service(), tick, f),
            Some(Mode::ThreeD) => with_service(&self.three.service(), tick, f),
            None => f(),
        }
    }
}

/// Writes each controller's displacement for the step to its transform and
/// reshapes capsules a block changed.
pub fn apply_motion(
    // Core keeps controllers in thread-local state: stay on the main thread.
    mut engine: NonSendMut<crate::engine::Engine>,
    time: Res<Time<Fixed>>,
    mut touching: Local<HashMap<String, HashSet<String>>>,
    mut commands: Commands,
    entities: Option<Res<ControllerEntities>>,
    layers: Option<Res<PhysicsLayers>>,
    mut transforms: Query<(&mut Transform, &mut GlobalTransform)>,
) {
    use blockloom_core::physics::controller as core;
    let Some(entities) = entities else {
        return;
    };
    let mode = layers
        .and_then(|layers| layers.mode)
        .unwrap_or(Mode::ThreeD);
    for (actor, delta) in core::take_pending() {
        let Some(&entity) = entities.actors.get(&actor) else {
            continue;
        };
        if let Ok((mut transform, mut global)) = transforms.get_mut(entity) {
            transform.translation.x += delta[0];
            transform.translation.y += delta[1];
            if mode == Mode::ThreeD {
                transform.translation.z += delta[2];
            }
            // The physics sync reads the global transform, which its own
            // propagation leaves stale for a root with a child collider.
            *global = GlobalTransform::from(*transform);
        }
    }
    announce_hits(&mut engine, &mut touching, time.timestep().as_secs_f32());
    for (actor, spec) in core::take_reshaped() {
        let Some(&capsule) = entities.capsules.get(&actor) else {
            continue;
        };
        let mut e = commands.entity(capsule);
        let transform = |tilt| Transform {
            translation: Vec3::from(spec.center),
            rotation: tilt,
            scale: Vec3::ONE,
        };
        match mode {
            Mode::ThreeD => {
                e.insert((
                    d3::capsule(&spec),
                    transform(tilt_3d(spec.up_unit(Mode::ThreeD))),
                ));
            }
            Mode::TwoD => {
                e.insert((
                    d2::capsule(&spec),
                    transform(tilt_2d(spec.up_unit(Mode::TwoD))),
                ));
            }
        }
        match mode {
            Mode::ThreeD => d3::set_enabled(&mut e, spec.enabled && spec.detect_collisions),
            Mode::TwoD => d2::set_enabled(&mut e, spec.enabled && spec.detect_collisions),
        }
    }
}

/// Tells the blocks what each move met. A fixed obstacle the controller met
/// and had not met on its last move is an Enter, one it keeps meeting a Stay
/// and one it no longer meets an Exit; bodies that move are left to the
/// contact lifecycle, which already hears them.
fn announce_hits(
    engine: &mut crate::engine::Engine,
    touching: &mut HashMap<String, HashSet<String>>,
    dt: f32,
) {
    use blockloom_core::physics::controller as core;
    use blockloom_core::physics::{ContactKind, ContactPhase};
    let moved = core::take_moved();
    let mut now: HashMap<String, HashMap<String, f32>> = HashMap::new();
    for (actor, hit) in core::take_hits() {
        if hit.body.is_none() {
            let speed = if dt > 0.0 { hit.move_length / dt } else { 0.0 };
            now.entry(actor)
                .or_default()
                .insert(hit.actor.clone(), speed);
        }
    }
    let fire = |engine: &mut crate::engine::Engine, actor: &str, with: &str, phase, speed| {
        engine.fire(blockloom_core::vm::Event::Collision {
            actor: actor.to_string(),
            with: with.to_string(),
            phase,
            kind: ContactKind::Collision,
            impulse: 0.0,
            speed,
        });
    };
    for actor in moved {
        let hit = now.remove(&actor).unwrap_or_default();
        let before = touching.remove(&actor).unwrap_or_default();
        for (with, speed) in &hit {
            let phase = if before.contains(with) {
                ContactPhase::Stay
            } else {
                ContactPhase::Enter
            };
            fire(engine, &actor, with, phase, *speed);
        }
        for with in before.iter().filter(|with| !hit.contains_key(*with)) {
            fire(engine, &actor, with, ContactPhase::Exit, 0.0);
        }
        if !hit.is_empty() {
            touching.insert(actor, hit.into_keys().collect());
        }
    }
}

pub mod d3 {
    use super::*;
    use bevy_rapier3d::prelude as rp;
    use bevy_rapier3d::rapier::control::{
        CharacterAutostep, CharacterLength, KinematicCharacterController,
    };
    use bevy_rapier3d::rapier::math::Pose;
    use bevy_rapier3d::rapier::parry::shape::Capsule;

    pub fn capsule(spec: &CharacterControllerSpec) -> rp::Collider {
        rp::Collider::capsule_y(spec.half_segment(), spec.radius)
    }

    pub fn set_enabled(e: &mut EntityCommands, on: bool) {
        if on {
            e.remove::<rp::ColliderDisabled>();
        } else {
            e.insert(rp::ColliderDisabled);
        }
    }

    /// Gives every planned controller its kinematic body and capsule.
    pub fn install(
        commands: &mut Commands,
        plan: &blockloom_core::physics::PhysicsPlan,
        entities: &HashMap<String, Entity>,
    ) -> ControllerEntities {
        let mut out = ControllerEntities::default();
        for planned in &plan.controllers {
            let Some(&frame) = entities.get(&planned.actor) else {
                continue;
            };
            let spec = &planned.spec;
            if !planned.has_body {
                commands.entity(frame).insert((
                    rp::RigidBody::KinematicPositionBased,
                    // A sleeping kinematic body drops the move that would wake it.
                    rp::Sleeping::disabled(),
                    crate::physics_install::PoseSmoothing(
                        blockloom_core::physics::Interpolation::Interpolate,
                    ),
                ));
            }
            let groups = rp::Group::from_bits_retain(planned.memberships);
            let accepts = rp::Group::from_bits_retain(planned.accepts);
            let mut e = commands.spawn((
                ChildOf(frame),
                Transform {
                    translation: Vec3::from(spec.center),
                    rotation: tilt_3d(spec.up_unit(Mode::ThreeD)),
                    scale: Vec3::ONE,
                },
                capsule(spec),
                rp::ColliderScale::Absolute(Vec3::ONE),
                rp::ColliderMassProperties::Density(0.0),
                rp::CollisionGroups::new(groups, accepts),
                rp::SolverGroups::new(groups, accepts),
                rp::ActiveEvents::COLLISION_EVENTS,
                PlannedCollider {
                    id: capsule_id(&planned.actor),
                    actor: planned.actor.clone(),
                    body: Some(planned.actor.clone()),
                    filter: planned.filter,
                    trigger: false,
                    queryable: true,
                },
                ControllerCapsule,
            ));
            if !spec.enabled || !spec.detect_collisions {
                e.insert(rp::ColliderDisabled);
            }
            out.actors.insert(planned.actor.clone(), frame);
            out.capsules.insert(planned.actor.clone(), e.id());
        }
        out
    }

    #[derive(SystemParam)]
    pub struct Access3<'w, 's> {
        context: rp::ReadRapierContext<'w, 's>,
        lookups: Lookups<'w, 's>,
        entities: Option<Res<'w, ControllerEntities>>,
        poses: Query<'w, 's, &'static Transform>,
        config: Query<'w, 's, &'static rp::RapierConfiguration>,
        time: Option<Res<'w, Time<Fixed>>>,
    }

    impl Access3<'_, '_> {
        pub fn service(&self) -> View3<'_, '_, '_> {
            View3(self)
        }
    }

    pub struct View3<'a, 'w, 's>(&'a Access3<'w, 's>);

    /// Rapier's controller configured the way the spec asks.
    pub(super) fn configured(
        spec: &CharacterControllerSpec,
        up: Vec3,
    ) -> KinematicCharacterController {
        KinematicCharacterController {
            up,
            offset: CharacterLength::Absolute(spec.skin_width),
            slide: true,
            autostep: (spec.step_offset > 0.0).then(|| CharacterAutostep {
                max_height: CharacterLength::Absolute(spec.step_offset),
                min_width: CharacterLength::Absolute(spec.radius * 0.5),
                include_dynamic_bodies: true,
            }),
            max_slope_climb_angle: spec.slope_limit.to_radians(),
            // Unity never slides on its own: only a move pushes it down a slope.
            min_slope_slide_angle: std::f32::consts::FRAC_PI_2,
            snap_to_ground: None,
            normal_nudge_factor: 1.0e-4,
        }
    }

    impl ControllerService for View3<'_, '_, '_> {
        fn step(&self, r: &StepRequest) -> Result<StepOutcome, String> {
            let world = self.0;
            let context = world
                .context
                .single()
                .map_err(|_| "the physics world is not built yet".to_string())?;
            let entity = world
                .entities
                .as_ref()
                .and_then(|e| e.actors.get(r.actor))
                .copied()
                .ok_or_else(|| "this actor's controller is not part of the world".to_string())?;
            let transform = world
                .poses
                .get(entity)
                .map_err(|_| "this actor has no transform".to_string())?;
            let spec = r.spec;
            let up = Vec3::from(spec.up_unit(Mode::ThreeD));
            let shape = Capsule::new_y(spec.half_segment(), spec.radius);
            let tilt = Quat::from_rotation_arc(Vec3::Y, up);
            let centre = transform.rotation * Vec3::from(spec.center);
            let start = transform.translation + Vec3::from(r.pending) + centre;
            let controller = configured(spec, up);
            let dt = r.dt.max(1.0e-4);

            let filter = sweep_filter(r.actor);
            let lookups = &world.lookups;
            let predicate =
                |entity: Entity, raw: &bevy_rapier3d::rapier::geometry::Collider| -> bool {
                    raw.is_enabled() && !raw.is_sensor() && {
                        let ident = lookups.ident(entity, false, raw.parent().is_some());
                        lookups.admits(&ident, true, &filter)
                    }
                };
            let query_filter = rp::QueryFilter::default().predicate(&predicate);
            Ok(context.with_query_pipeline(query_filter, |pipeline| {
                let queries = &pipeline.query_pipeline;
                let mut outcome = StepOutcome::default();
                let mut pose = Pose::from_parts(start, tilt);
                let mut recovered = Vec3::ZERO;
                if spec.overlap_recovery {
                    let fix = controller.move_shape(dt, queries, &shape, &pose, Vec3::ZERO, |_| {});
                    recovered = fix.translation;
                    pose = Pose::from_parts(start + recovered, tilt);
                    outcome.grounded = fix.grounded;
                }
                let displacement = Vec3::from(r.displacement);
                let mut collisions = Vec::new();
                if displacement.length_squared() > 0.0 {
                    let moved =
                        controller.move_shape(dt, queries, &shape, &pose, displacement, |c| {
                            collisions.push(c)
                        });
                    outcome.effective = moved.translation.to_array();
                    outcome.grounded = moved.grounded;
                } else if !spec.overlap_recovery {
                    let probe =
                        controller.move_shape(dt, queries, &shape, &pose, Vec3::ZERO, |_| {});
                    outcome.grounded = probe.grounded;
                }
                outcome.recovered = recovered.to_array();
                for c in collisions {
                    let Some(hit_entity) = context.colliders.collider_entity(c.handle) else {
                        continue;
                    };
                    let (sensor, carried) = context
                        .colliders
                        .colliders
                        .get(c.handle)
                        .map_or((false, false), |raw| {
                            (raw.is_sensor(), raw.parent().is_some())
                        });
                    let ident = lookups.ident(hit_entity, sensor, carried);
                    let (point, normal) = (c.hit.witness1, c.hit.normal1);
                    outcome.hits.push(RawHit {
                        actor: ident.actor,
                        body: ident.body,
                        collider: ident.collider,
                        point: point.to_array(),
                        normal: normal.to_array(),
                        applied: c.translation_applied.to_array(),
                    });
                }
                outcome
            }))
        }

        fn gravity(&self) -> [f32; 3] {
            self.0
                .config
                .single()
                .map_or([0.0, -9.81, 0.0], |config| config.gravity.to_array())
        }

        fn timestep(&self) -> f32 {
            self.0
                .time
                .as_ref()
                .map_or(1.0 / 60.0, |time| time.timestep().as_secs_f32())
        }

        fn mode(&self) -> Mode {
            Mode::ThreeD
        }
    }
}

pub mod d2 {
    use super::*;
    use bevy_rapier2d::prelude as rp;
    use bevy_rapier2d::rapier::control::{
        CharacterAutostep, CharacterLength, KinematicCharacterController,
    };
    use bevy_rapier2d::rapier::math::Pose;
    use bevy_rapier2d::rapier::parry::shape::Capsule;

    pub fn capsule(spec: &CharacterControllerSpec) -> rp::Collider {
        rp::Collider::capsule_y(spec.half_segment(), spec.radius)
    }

    pub fn set_enabled(e: &mut EntityCommands, on: bool) {
        if on {
            e.remove::<rp::ColliderDisabled>();
        } else {
            e.insert(rp::ColliderDisabled);
        }
    }

    /// Gives every planned controller its kinematic body and capsule.
    pub fn install(
        commands: &mut Commands,
        plan: &blockloom_core::physics::PhysicsPlan,
        entities: &HashMap<String, Entity>,
    ) -> ControllerEntities {
        let mut out = ControllerEntities::default();
        for planned in &plan.controllers {
            let Some(&frame) = entities.get(&planned.actor) else {
                continue;
            };
            let spec = &planned.spec;
            if !planned.has_body {
                commands.entity(frame).insert((
                    rp::RigidBody::KinematicPositionBased,
                    // A sleeping kinematic body drops the move that would wake it.
                    rp::Sleeping::disabled(),
                    crate::physics_install::PoseSmoothing(
                        blockloom_core::physics::Interpolation::Interpolate,
                    ),
                ));
            }
            let groups = rp::Group::from_bits_retain(planned.memberships);
            let accepts = rp::Group::from_bits_retain(planned.accepts);
            let mut e = commands.spawn((
                ChildOf(frame),
                Transform {
                    translation: Vec3::from(spec.center),
                    rotation: tilt_2d(spec.up_unit(Mode::TwoD)),
                    scale: Vec3::ONE,
                },
                capsule(spec),
                rp::ColliderScale::Absolute(Vec2::ONE),
                rp::ColliderMassProperties::Density(0.0),
                rp::CollisionGroups::new(groups, accepts),
                rp::SolverGroups::new(groups, accepts),
                rp::ActiveEvents::COLLISION_EVENTS,
                PlannedCollider {
                    id: capsule_id(&planned.actor),
                    actor: planned.actor.clone(),
                    body: Some(planned.actor.clone()),
                    filter: planned.filter,
                    trigger: false,
                    queryable: true,
                },
                ControllerCapsule,
            ));
            if !spec.enabled || !spec.detect_collisions {
                e.insert(rp::ColliderDisabled);
            }
            out.actors.insert(planned.actor.clone(), frame);
            out.capsules.insert(planned.actor.clone(), e.id());
        }
        out
    }

    #[derive(SystemParam)]
    pub struct Access2<'w, 's> {
        context: rp::ReadRapierContext<'w, 's>,
        lookups: Lookups<'w, 's>,
        entities: Option<Res<'w, ControllerEntities>>,
        poses: Query<'w, 's, &'static Transform>,
        config: Query<'w, 's, &'static rp::RapierConfiguration>,
        time: Option<Res<'w, Time<Fixed>>>,
    }

    impl Access2<'_, '_> {
        pub fn service(&self) -> View2<'_, '_, '_> {
            View2(self)
        }
    }

    pub struct View2<'a, 'w, 's>(&'a Access2<'w, 's>);

    fn planar(v: [f32; 3]) -> Vec2 {
        Vec2::new(v[0], v[1])
    }

    fn lift(v: Vec2) -> [f32; 3] {
        [v.x, v.y, 0.0]
    }

    pub(super) fn configured(
        spec: &CharacterControllerSpec,
        up: Vec2,
    ) -> KinematicCharacterController {
        KinematicCharacterController {
            up,
            offset: CharacterLength::Absolute(spec.skin_width),
            slide: true,
            autostep: (spec.step_offset > 0.0).then(|| CharacterAutostep {
                max_height: CharacterLength::Absolute(spec.step_offset),
                min_width: CharacterLength::Absolute(spec.radius * 0.5),
                include_dynamic_bodies: true,
            }),
            max_slope_climb_angle: spec.slope_limit.to_radians(),
            min_slope_slide_angle: std::f32::consts::FRAC_PI_2,
            snap_to_ground: None,
            normal_nudge_factor: 1.0e-4,
        }
    }

    impl ControllerService for View2<'_, '_, '_> {
        fn step(&self, r: &StepRequest) -> Result<StepOutcome, String> {
            let world = self.0;
            let context = world
                .context
                .single()
                .map_err(|_| "the physics world is not built yet".to_string())?;
            let entity = world
                .entities
                .as_ref()
                .and_then(|e| e.actors.get(r.actor))
                .copied()
                .ok_or_else(|| "this actor's controller is not part of the world".to_string())?;
            let transform = world
                .poses
                .get(entity)
                .map_err(|_| "this actor has no transform".to_string())?;
            let spec = r.spec;
            let up = planar(spec.up_unit(Mode::TwoD));
            let shape = Capsule::new_y(spec.half_segment(), spec.radius);
            let angle = (-up.x).atan2(up.y);
            let turn = transform.rotation * Vec3::from(spec.center);
            let start = transform.translation.truncate() + planar(r.pending) + turn.truncate();
            let controller = configured(spec, up);
            let dt = r.dt.max(1.0e-4);

            let filter = sweep_filter(r.actor);
            let lookups = &world.lookups;
            let predicate =
                |entity: Entity, raw: &bevy_rapier2d::rapier::geometry::Collider| -> bool {
                    raw.is_enabled() && !raw.is_sensor() && {
                        let ident = lookups.ident(entity, false, raw.parent().is_some());
                        lookups.admits(&ident, true, &filter)
                    }
                };
            let query_filter = rp::QueryFilter::default().predicate(&predicate);
            Ok(context.with_query_pipeline(query_filter, |pipeline| {
                let queries = &pipeline.query_pipeline;
                let mut outcome = StepOutcome::default();
                let mut pose = Pose::new(start, angle);
                let mut recovered = Vec2::ZERO;
                if spec.overlap_recovery {
                    let fix = controller.move_shape(dt, queries, &shape, &pose, Vec2::ZERO, |_| {});
                    recovered = fix.translation;
                    pose = Pose::new(start + recovered, angle);
                    outcome.grounded = fix.grounded;
                }
                let displacement = planar(r.displacement);
                let mut collisions = Vec::new();
                if displacement.length_squared() > 0.0 {
                    let moved =
                        controller.move_shape(dt, queries, &shape, &pose, displacement, |c| {
                            collisions.push(c)
                        });
                    outcome.effective = lift(moved.translation);
                    outcome.grounded = moved.grounded;
                } else if !spec.overlap_recovery {
                    let probe =
                        controller.move_shape(dt, queries, &shape, &pose, Vec2::ZERO, |_| {});
                    outcome.grounded = probe.grounded;
                }
                outcome.recovered = lift(recovered);
                for c in collisions {
                    let Some(hit_entity) = context.colliders.collider_entity(c.handle) else {
                        continue;
                    };
                    let (sensor, carried) = context
                        .colliders
                        .colliders
                        .get(c.handle)
                        .map_or((false, false), |raw| {
                            (raw.is_sensor(), raw.parent().is_some())
                        });
                    let ident = lookups.ident(hit_entity, sensor, carried);
                    let (point, normal) = (c.hit.witness1, c.hit.normal1);
                    outcome.hits.push(RawHit {
                        actor: ident.actor,
                        body: ident.body,
                        collider: ident.collider,
                        point: lift(point),
                        normal: lift(normal),
                        applied: lift(c.translation_applied),
                    });
                }
                outcome
            }))
        }

        fn gravity(&self) -> [f32; 3] {
            self.0
                .config
                .single()
                .map_or([0.0, -981.0, 0.0], |config| lift(config.gravity))
        }

        fn timestep(&self) -> f32 {
            self.0
                .time
                .as_ref()
                .map_or(1.0 / 60.0, |time| time.timestep().as_secs_f32())
        }

        fn mode(&self) -> Mode {
            Mode::TwoD
        }
    }
}
