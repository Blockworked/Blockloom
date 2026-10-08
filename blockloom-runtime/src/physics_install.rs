//! Turns a core `PhysicsPlan` into rapier components: one entity per body and one
//! child entity per collider, so a body is made of any number of shapes and a shape
//! can be added, disabled or removed on its own.
//!
//! A legacy `Body` never reaches this module: the plan only holds actors that carry
//! Rigidbody or Collider components, and validation refuses an actor with both.
//! Core decides everything (poses, dimensions, mass, groups); this file only maps
//! those answers onto the backend.

use crate::engine::{Engine, PendingEffects};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use blockloom_core::physics::material::CombineMode;
use blockloom_core::physics::spec::{BodyType, CollisionDetection};
use blockloom_core::physics::{
    ColliderFilter, ColliderId, CollisionLookup, ExtraMass, LayerSettings, PhysicsPlan,
    PlannedMaterial, PlannedShape,
};
use blockloom_core::project::{Actor, Project};
use blockloom_core::scene::Mode;
use blockloom_core::vm::Effect;
use std::collections::HashMap;

/// The layer rules the pair hooks consult.
#[derive(Resource, Default, Clone)]
pub struct PhysicsLayers {
    pub settings: LayerSettings,
    pub mode: Option<Mode>,
    /// The filter of each actor's first collider, which a query made by that
    /// actor asks with.
    pub askers: HashMap<String, ColliderFilter>,
}

/// A rigid body this module installed.
#[derive(Component, Debug, Clone, Copy)]
pub struct PlannedBody {
    pub max_linear: f32,
    pub max_angular: f32,
    /// Axes the body may not move along or turn about, however it is pushed.
    pub frozen_linear: [bool; 3],
    pub frozen_angular: [bool; 3],
    /// The fastest a contact may push this body out of an overlap, in world
    /// units a second.
    pub max_depenetration: f32,
}

/// How a body's drawn pose follows its fixed-step poses (Unity's Interpolation).
/// Actors without one are drawn interpolated, as every actor was before.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoseSmoothing(pub blockloom_core::physics::Interpolation);

/// The slowest depenetration speed either body of a pair allows, if either
/// is a planned body.
pub fn depenetration_cap(a: Option<&PlannedBody>, b: Option<&PlannedBody>) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max_depenetration.min(b.max_depenetration)),
        (Some(one), None) | (None, Some(one)) => Some(one.max_depenetration),
        (None, None) => None,
    }
}

/// Rapier's own cap on how fast a contact pushes bodies apart, in metres a
/// second. A body's `max_depenetration_velocity` above it changes nothing.
const RAPIER_CORRECTIVE_VELOCITY: f32 = 3.0;

/// True when the collider's body asks for a gentler depenetration than the
/// solver's own cap, so the contact hook has something to do.
fn caps_depenetration(
    plan: &PhysicsPlan,
    collider: &blockloom_core::physics::ColliderPlan,
    solver_cap: f32,
) -> bool {
    collider
        .body_actor
        .as_deref()
        .and_then(|actor| plan.body(actor))
        .is_some_and(|body| body.spec.max_depenetration_velocity < solver_cap)
}

/// A collider entity this module installed.
#[derive(Component, Debug, Clone)]
pub struct PlannedCollider {
    pub id: ColliderId,
    /// The actor the collider component lives on.
    pub actor: String,
    /// The actor whose Rigidbody carries it; `None` for scenery.
    pub body: Option<String>,
    pub filter: ColliderFilter,
    pub trigger: bool,
    /// Whether queries (rays, casts, overlaps) see it.
    pub queryable: bool,
}

/// A 3D collider's surface, for the stick and slip hook.
#[derive(Component, Debug, Clone, Copy)]
pub struct Surface3(pub blockloom_core::physics::PhysicsMaterial);

/// The speed along the contact below which a 3D contact still counts as stuck.
pub const STICK_SPEED: f32 = 0.02;

/// How far ahead of a body Speculative continuous detection looks, in metres.
const SPECULATIVE_PREDICTION: f32 = 0.5;

/// Builds the plan for a project and installs it onto the already spawned actor
/// entities. A plan with errors installs nothing and says why in the log; Play
/// refuses such a project before the world is built.
#[cfg(test)]
pub fn install(commands: &mut Commands, project: &Project, entities: &HashMap<String, Entity>) {
    install_with(
        commands,
        project,
        entities,
        &blockloom_core::physics::cook::NoCollisionData,
    );
}

/// [`install`] with mesh colliders cooked (or read, in a shipped game) through
/// `lookup`.
pub fn install_with(
    commands: &mut Commands,
    project: &Project,
    entities: &HashMap<String, Entity>,
    lookup: &dyn CollisionLookup,
) {
    let mode = project.world.mode;
    let plan = PhysicsPlan::build_with(&project.actors, mode, &project.physics, lookup);
    let mut askers = HashMap::new();
    for collider in &plan.colliders {
        askers
            .entry(collider.actor.clone())
            .or_insert(collider.filter);
        if let Some(body) = &collider.body_actor {
            askers.entry(body.clone()).or_insert(collider.filter);
        }
    }
    for controller in &plan.controllers {
        askers
            .entry(controller.actor.clone())
            .or_insert(controller.filter);
    }
    commands.insert_resource(PhysicsLayers {
        settings: project.physics.layers.clone(),
        mode: Some(mode),
        askers,
    });
    for issue in &plan.issues {
        if issue.is_error() {
            warn!("physics: {}", issue.message);
        }
    }
    crate::controller::clear();
    crate::motor::clear();
    crate::constraints::clear();
    commands.insert_resource(crate::controller::ControllerEntities::default());
    if !plan.is_runnable() || plan.is_empty() {
        return;
    }
    match mode {
        Mode::ThreeD => d3::install(commands, &plan, entities),
        Mode::TwoD => d2::install(commands, &plan, entities),
    }
    let installed = match mode {
        Mode::ThreeD => crate::controller::d3::install(commands, &plan, entities),
        Mode::TwoD => crate::controller::d2::install(commands, &plan, entities),
    };
    match mode {
        Mode::ThreeD => crate::constraints::d3::install(commands, &plan.constraints, entities),
        Mode::TwoD => crate::constraints::d2::install(commands, &plan.constraints, entities),
    }
    blockloom_core::physics::joints::register_plan(&plan.constraints);
    crate::controller::register_plan(&plan);
    crate::motor::register_plan(&plan);
    commands.insert_resource(installed);
}

/// Installs the physics of one actor that came into a run after it began (a
/// clone). `actor` already carries fresh ids; its constraints keep the targets
/// the template had. Mesh colliders and controllers are left out.
pub fn install_actor(
    commands: &mut Commands,
    project: &Project,
    actor: &Actor,
    others: &[Actor],
    entities: &HashMap<String, Entity>,
) {
    let mode = project.world.mode;
    let mut actors = vec![actor.clone()];
    actors.extend(others.iter().cloned());
    let mut plan = PhysicsPlan::build(&actors, mode, &project.physics);
    plan.bodies.retain(|b| b.actor == actor.id);
    plan.colliders
        .retain(|c| c.actor == actor.id || c.body_actor.as_deref() == Some(actor.id.as_str()));
    plan.constraints.retain(|c| c.actor == actor.id);
    plan.controllers.clear();
    plan.motors.clear();
    if plan
        .errors()
        .any(|i| i.actor.as_deref() == Some(actor.id.as_str()))
    {
        for issue in plan.errors() {
            warn!("physics: {}", issue.message);
        }
        return;
    }
    if plan.bodies.is_empty() && plan.colliders.is_empty() && plan.constraints.is_empty() {
        return;
    }
    let filters: Vec<_> = plan
        .colliders
        .iter()
        .map(|c| (c.actor.clone(), c.filter))
        .collect();
    commands.queue(move |world: &mut World| {
        if let Some(mut layers) = world.get_resource_mut::<PhysicsLayers>() {
            for (id, filter) in filters {
                layers.askers.entry(id).or_insert(filter);
            }
        }
    });
    match mode {
        Mode::ThreeD => {
            d3::install(commands, &plan, entities);
            crate::constraints::d3::install_more(commands, &plan.constraints, entities);
        }
        Mode::TwoD => {
            d2::install(commands, &plan, entities);
            crate::constraints::d2::install_more(commands, &plan.constraints, entities);
        }
    }
    blockloom_core::physics::joints::extend_plan(&plan.constraints);
}

/// The backend shape of a cooked mesh.
fn mesh_collider(
    mesh: &blockloom_core::physics::geometry::MeshShape,
) -> Result<bevy_rapier3d::prelude::Collider, String> {
    use bevy_rapier3d::prelude as rp;
    use blockloom_core::physics::geometry::MeshShape;
    let vec3 =
        |points: &[[f32; 3]]| -> Vec<Vec3> { points.iter().map(|p| Vec3::from(*p)).collect() };
    match mesh {
        MeshShape::Hull { points, .. } => rp::Collider::convex_hull(&vec3(&points.0))
            .ok_or_else(|| "the convex hull could not be built".to_string()),
        MeshShape::Compound { hulls, .. } => {
            let mut parts = Vec::new();
            for hull in hulls {
                let part = rp::Collider::convex_hull(&vec3(&hull.0))
                    .ok_or_else(|| "a decomposed hull could not be built".to_string())?;
                parts.push((Vec3::ZERO, Quat::IDENTITY, part));
            }
            Ok(rp::Collider::compound(parts))
        }
        MeshShape::Triangles { vertices, indices } => {
            let triangles = indices
                .0
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| [t[0], t[1], t[2]])
                .collect();
            rp::Collider::trimesh_with_flags(
                vec3(&vertices.0),
                triangles,
                rp::TriMeshFlags::FIX_INTERNAL_EDGES,
            )
            .map_err(|e| format!("the triangle mesh could not be built: {e:?}"))
        }
    }
}

/// The ordering Unity gives sleeping: a mass-normalised kinetic energy threshold,
/// which for unit mass is `v^2 / 2`, so the speed is `sqrt(2 * threshold)`.
fn sleep_speed(threshold: f32) -> Option<f32> {
    (threshold > 0.0).then(|| (2.0 * threshold).sqrt())
}

fn combine(mode: CombineMode) -> bevy_rapier3d::prelude::CoefficientCombineRule {
    use bevy_rapier3d::prelude::CoefficientCombineRule as Rule;
    match mode {
        CombineMode::Average => Rule::Average,
        CombineMode::Minimum => Rule::Min,
        CombineMode::Multiply => Rule::Multiply,
        CombineMode::Maximum => Rule::Max,
    }
}

/// Pair filtering shared by both dimensions' hooks.
pub fn pair_allowed(
    layers: &PhysicsLayers,
    a: Option<&PlannedCollider>,
    b: Option<&PlannedCollider>,
) -> bool {
    match (a, b, layers.mode) {
        (Some(a), Some(b), Some(mode)) => {
            blockloom_core::physics::pair_collides(&a.filter, &b.filter, &layers.settings, mode)
        }
        _ => true,
    }
}

pub mod d3 {
    use super::*;
    use bevy_rapier3d::prelude as rp;
    use blockloom_core::physics::geometry::Solid3;
    use blockloom_core::physics::spec::Axis;

    pub fn install(
        commands: &mut Commands,
        plan: &PhysicsPlan,
        entities: &HashMap<String, Entity>,
    ) {
        for body in &plan.bodies {
            let Some(&entity) = entities.get(&body.actor) else {
                continue;
            };
            let spec = &body.spec;
            let mut e = commands.entity(entity);
            e.insert(PoseSmoothing(spec.interpolation));
            e.insert((
                match spec.body_type {
                    BodyType::Dynamic => rp::RigidBody::Dynamic,
                    BodyType::Kinematic => rp::RigidBody::KinematicPositionBased,
                    BodyType::Static => rp::RigidBody::Fixed,
                },
                rp::Velocity::zero(),
                rp::ExternalImpulse::default(),
                rp::GravityScale(if spec.use_gravity {
                    spec.gravity_scale
                } else {
                    0.0
                }),
                rp::Damping {
                    linear_damping: spec.linear_damping,
                    angular_damping: spec.angular_damping,
                },
                PlannedBody {
                    max_linear: spec.max_linear_velocity,
                    max_angular: spec.max_angular_velocity,
                    frozen_linear: spec.constraints.freeze_position,
                    frozen_angular: spec.constraints.freeze_rotation,
                    max_depenetration: spec.max_depenetration_velocity,
                },
            ));
            let sleeping = match sleep_speed(spec.sleep_threshold) {
                Some(speed) => rp::Sleeping {
                    normalized_linear_threshold: speed,
                    angular_threshold: speed,
                    ..Default::default()
                },
                None => rp::Sleeping::disabled(),
            };
            e.insert(sleeping);
            if let Some(extra) = spec.solver_iterations.filter(|extra| *extra > 0) {
                e.insert(rp::AdditionalSolverIterations(extra as usize));
            }
            let mut locked = rp::LockedAxes::empty();
            for (frozen, flag) in spec.constraints.freeze_position.iter().zip([
                rp::LockedAxes::TRANSLATION_LOCKED_X,
                rp::LockedAxes::TRANSLATION_LOCKED_Y,
                rp::LockedAxes::TRANSLATION_LOCKED_Z,
            ]) {
                if *frozen {
                    locked |= flag;
                }
            }
            for (frozen, flag) in spec.constraints.freeze_rotation.iter().zip([
                rp::LockedAxes::ROTATION_LOCKED_X,
                rp::LockedAxes::ROTATION_LOCKED_Y,
                rp::LockedAxes::ROTATION_LOCKED_Z,
            ]) {
                if *frozen {
                    locked |= flag;
                }
            }
            if !locked.is_empty() {
                e.insert(locked);
            }
            match spec.collision_detection {
                CollisionDetection::Discrete | CollisionDetection::Continuous => {}
                CollisionDetection::ContinuousDynamic => {
                    e.insert(rp::Ccd::enabled());
                }
                CollisionDetection::ContinuousSpeculative => {
                    e.insert(rp::SoftCcd {
                        prediction: SPECULATIVE_PREDICTION,
                    });
                }
            }
            if let Some(extra) = body.extra_mass {
                e.insert(match extra {
                    ExtraMass::Mass(mass) => rp::AdditionalMassProperties::Mass(mass),
                    ExtraMass::Full {
                        mass,
                        center,
                        inertia,
                    } => rp::AdditionalMassProperties::MassProperties(rp::MassProperties {
                        local_center_of_mass: Vec3::from(center),
                        mass,
                        principal_inertia_local_frame: Quat::IDENTITY,
                        principal_inertia: Vec3::from(inertia),
                    }),
                });
            }
            if body.extra_mass.is_none() {
                // Present so a removed shape can nudge the body to total again.
                e.insert(rp::AdditionalMassProperties::default());
            }
            if !spec.simulated {
                e.insert(rp::RigidBodyDisabled);
            }
        }

        for planned in &plan.colliders {
            let frame_actor = planned.body_actor.as_ref().unwrap_or(&planned.actor);
            let Some(&frame) = entities.get(frame_actor) else {
                continue;
            };
            let collider = match &planned.shape {
                PlannedShape::Three(solid) => match *solid {
                    Solid3::Cuboid { half } => rp::Collider::cuboid(half[0], half[1], half[2]),
                    Solid3::Ball { radius } => rp::Collider::ball(radius),
                    Solid3::Capsule {
                        axis,
                        half_segment,
                        radius,
                    } => match axis {
                        Axis::X => rp::Collider::capsule_x(half_segment, radius),
                        Axis::Y => rp::Collider::capsule_y(half_segment, radius),
                        Axis::Z => rp::Collider::capsule_z(half_segment, radius),
                    },
                },
                PlannedShape::Mesh(mesh) => match mesh_collider(mesh) {
                    Ok(collider) => collider,
                    Err(why) => {
                        warn!("physics: {} on \"{}\": {why}", planned.name, planned.actor);
                        continue;
                    }
                },
                PlannedShape::Two(_) => continue,
            };
            let PlannedMaterial::Three(material) = planned.material else {
                continue;
            };
            let groups = rp::Group::from_bits_retain(planned.memberships);
            let accepts = rp::Group::from_bits_retain(planned.accepts);
            let mut hooks = rp::ActiveHooks::empty();
            if plan.exact_filtering {
                hooks |= rp::ActiveHooks::FILTER_CONTACT_PAIRS
                    | rp::ActiveHooks::FILTER_INTERSECTION_PAIR;
            }
            if material.static_friction != material.dynamic_friction
                || caps_depenetration(plan, planned, RAPIER_CORRECTIVE_VELOCITY)
            {
                hooks |= rp::ActiveHooks::MODIFY_SOLVER_CONTACTS;
            }
            let mut e = commands.spawn((
                ChildOf(frame),
                Transform {
                    translation: Vec3::from(planned.local),
                    rotation: Quat::from_array(planned.rotation),
                    scale: Vec3::ONE,
                },
                collider,
                rp::ColliderScale::Absolute(Vec3::ONE),
                rp::ColliderMassProperties::Density(planned.density),
                rp::Friction {
                    coefficient: material.static_friction,
                    combine_rule: combine(material.friction_combine),
                },
                rp::Restitution {
                    coefficient: material.bounciness,
                    combine_rule: combine(material.bounce_combine),
                },
                rp::CollisionGroups::new(groups, accepts),
                rp::SolverGroups::new(groups, accepts),
                rp::ActiveEvents::COLLISION_EVENTS | rp::ActiveEvents::CONTACT_FORCE_EVENTS,
                hooks,
                PlannedCollider {
                    id: planned.collider.clone(),
                    actor: planned.actor.clone(),
                    body: planned.body_actor.clone(),
                    filter: planned.filter,
                    trigger: planned.trigger,
                    queryable: planned.queryable,
                },
                Surface3(material),
            ));
            if planned.trigger {
                e.insert((
                    rp::Sensor,
                    rp::ActiveCollisionTypes::all() - rp::ActiveCollisionTypes::STATIC_STATIC,
                ));
            }
            if let Some(offset) = planned.contact_offset.filter(|o| o.is_finite() && *o > 0.0) {
                e.insert(rp::ContactSkin(offset));
            }
            if !planned.enabled {
                e.insert(rp::ColliderDisabled);
            }
        }
    }

    /// Pair filtering and the stick/slip friction of 3D contacts.
    #[derive(SystemParam)]
    pub struct Hooks3<'w, 's> {
        layers: Res<'w, PhysicsLayers>,
        colliders: Query<'w, 's, &'static PlannedCollider>,
        surfaces: Query<'w, 's, &'static Surface3>,
        bodies: Query<'w, 's, (&'static rp::Velocity, &'static GlobalTransform)>,
        planned: Query<'w, 's, &'static PlannedBody>,
        time: Res<'w, Time>,
    }

    impl Hooks3<'_, '_> {
        /// A body's velocity at a world point, or rest for scenery.
        fn velocity_at(&self, body: Option<Entity>, point: Vec3) -> Vec3 {
            body.and_then(|b| self.bodies.get(b).ok())
                .map_or(Vec3::ZERO, |(velocity, at)| {
                    velocity.linear + velocity.angular.cross(point - at.translation())
                })
        }
    }

    impl rp::BevyPhysicsHooks for Hooks3<'_, '_> {
        fn filter_contact_pair(
            &self,
            context: rp::PairFilterContextView,
        ) -> Option<rp::SolverFlags> {
            pair_allowed(
                &self.layers,
                self.colliders.get(context.collider1()).ok(),
                self.colliders.get(context.collider2()).ok(),
            )
            .then_some(rp::SolverFlags::COMPUTE_RIGID_IMPULSES)
        }

        fn filter_intersection_pair(&self, context: rp::PairFilterContextView) -> bool {
            pair_allowed(
                &self.layers,
                self.colliders.get(context.collider1()).ok(),
                self.colliders.get(context.collider2()).ok(),
            )
        }

        fn modify_solver_contacts(&self, mut context: rp::ContactModificationContextView) {
            let planned = |body: Option<Entity>| body.and_then(|b| self.planned.get(b).ok());
            if let Some(cap) = depenetration_cap(
                planned(context.rigid_body1()),
                planned(context.rigid_body2()),
            ) && cap.is_finite()
                && let Some(contacts) = context.solver_contacts_mut()
            {
                let deepest = -cap * self.time.delta_secs();
                for contact in contacts.iter_mut() {
                    contact.dist = contact.dist.max(deepest);
                }
            }
            let (Ok(a), Ok(b)) = (
                self.surfaces.get(context.collider1()),
                self.surfaces.get(context.collider2()),
            ) else {
                return;
            };
            let Some(normal) = context.normal() else {
                return;
            };
            let Some(point) = context
                .solver_contacts()
                .and_then(|contacts| contacts.first())
                .map(|contact| (contact.anchor1 + contact.anchor2) * 0.5)
            else {
                return;
            };
            let relative = self.velocity_at(context.rigid_body2(), point)
                - self.velocity_at(context.rigid_body1(), point);
            let sliding = (relative - normal * relative.dot(normal)).length();
            let (stuck, slip, _) = blockloom_core::physics::PhysicsMaterial::contact(&a.0, &b.0);
            context.set_friction(if sliding < STICK_SPEED { stuck } else { slip });
        }
    }

    /// Rapier keeps a body's mass as it was until told to total again, so when a
    /// shape goes away every planned body is nudged to recompute it.
    pub fn refresh_masses(
        mut removed: RemovedComponents<PlannedCollider>,
        fresh: Query<(), Added<rp::RapierRigidBodyHandle>>,
        mut bodies: Query<&mut rp::AdditionalMassProperties, With<PlannedBody>>,
    ) {
        // A body made this frame needs one too: its first total happens before
        // the backend body exists to be told.
        let removed = removed.read().next().is_some();
        if !removed && fresh.is_empty() {
            return;
        }
        for mut extra in &mut bodies {
            extra.set_changed();
        }
    }

    /// Velocity caps, applied after the step so a body never integrates past them.
    pub fn clamp_velocities(mut bodies: Query<(&PlannedBody, &mut rp::Velocity)>) {
        for (limits, mut velocity) in &mut bodies {
            for axis in 0..3 {
                if limits.frozen_linear[axis] {
                    velocity.linear[axis] = 0.0;
                }
                if limits.frozen_angular[axis] {
                    velocity.angular[axis] = 0.0;
                }
            }
            let speed = velocity.linear.length();
            if speed > limits.max_linear && limits.max_linear > 0.0 {
                velocity.linear *= limits.max_linear / speed;
            }
            let spin = velocity.angular.length();
            if spin > limits.max_angular && limits.max_angular > 0.0 {
                velocity.angular *= limits.max_angular / spin;
            }
        }
    }

    /// `add force` and `add torque`, as the impulse the mode reduces to over this
    /// fixed step. Only a dynamic body responds.
    pub fn apply_forces(
        effects: Res<PendingEffects>,
        engine: NonSend<Engine>,
        time: Res<Time>,
        mut bodies: Query<(
            &rp::RigidBody,
            &Transform,
            Option<&rp::ReadMassProperties>,
            &mut rp::ExternalImpulse,
        )>,
    ) {
        if !engine.running || engine.paused {
            return;
        }
        let dt = time.delta_secs().max(1.0 / 1000.0);
        for effect in &effects.0 {
            let Effect::AddForce {
                actor,
                mode,
                torque,
                vector,
            } = effect
            else {
                continue;
            };
            let Some(entity) = engine.entities.get(actor).copied() else {
                continue;
            };
            let Ok((kind, transform, mass, mut pending)) = bodies.get_mut(entity) else {
                continue;
            };
            if *kind != rp::RigidBody::Dynamic {
                continue;
            }
            let Some(mass) = mass.map(|mass| *mass.get()) else {
                continue;
            };
            if *torque {
                let impulse = mode.torque_impulse_3d(
                    *vector,
                    mass.principal_inertia.to_array(),
                    mass.principal_inertia_local_frame.to_array(),
                    transform.rotation.to_array(),
                    dt,
                );
                pending.torque_impulse += Vec3::from_array(impulse);
            } else {
                pending.impulse +=
                    Vec3::from_array(mode.linear_impulse_array(*vector, mass.mass, dt));
            }
        }
    }
}

pub mod d2 {
    use super::*;
    use bevy_rapier2d::prelude as rp;
    use blockloom_core::physics::geometry::Solid2;

    /// Pixels per metre, the unit mass properties and speed caps are authored in.
    const PPM: f32 = crate::dim2::PIXELS_PER_METER;

    pub fn install(
        commands: &mut Commands,
        plan: &PhysicsPlan,
        entities: &HashMap<String, Entity>,
    ) {
        for body in &plan.bodies {
            let Some(&entity) = entities.get(&body.actor) else {
                continue;
            };
            let spec = &body.spec;
            let mut e = commands.entity(entity);
            e.insert(PoseSmoothing(spec.interpolation));
            e.insert((
                match spec.body_type {
                    BodyType::Dynamic => rp::RigidBody::Dynamic,
                    BodyType::Kinematic => rp::RigidBody::KinematicPositionBased,
                    BodyType::Static => rp::RigidBody::Fixed,
                },
                rp::Velocity::zero(),
                rp::ExternalImpulse::default(),
                rp::GravityScale(if spec.use_gravity {
                    spec.gravity_scale
                } else {
                    0.0
                }),
                rp::Damping {
                    linear_damping: spec.linear_damping,
                    angular_damping: spec.angular_damping,
                },
                PlannedBody {
                    max_linear: spec.max_linear_velocity * PPM,
                    max_angular: f32::INFINITY,
                    frozen_linear: [
                        spec.constraints.freeze_position[0],
                        spec.constraints.freeze_position[1],
                        false,
                    ],
                    frozen_angular: [false, false, spec.constraints.freeze_rotation[2]],
                    max_depenetration: spec.max_depenetration_velocity * PPM,
                },
            ));
            let sleeping = match sleep_speed(spec.sleep_threshold) {
                Some(speed) => rp::Sleeping {
                    normalized_linear_threshold: speed,
                    angular_threshold: speed,
                    ..Default::default()
                },
                None => rp::Sleeping::disabled(),
            };
            e.insert(sleeping);
            if let Some(extra) = spec.solver_iterations.filter(|extra| *extra > 0) {
                e.insert(rp::AdditionalSolverIterations(extra as usize));
            }
            let mut locked = rp::LockedAxes::empty();
            if spec.constraints.freeze_position[0] {
                locked |= rp::LockedAxes::TRANSLATION_LOCKED_X;
            }
            if spec.constraints.freeze_position[1] {
                locked |= rp::LockedAxes::TRANSLATION_LOCKED_Y;
            }
            if spec.constraints.freeze_rotation[2] {
                locked |= rp::LockedAxes::ROTATION_LOCKED;
            }
            if !locked.is_empty() {
                e.insert(locked);
            }
            match spec.collision_detection {
                CollisionDetection::Discrete | CollisionDetection::Continuous => {}
                CollisionDetection::ContinuousDynamic => {
                    e.insert(rp::Ccd::enabled());
                }
                CollisionDetection::ContinuousSpeculative => {
                    e.insert(rp::SoftCcd {
                        prediction: SPECULATIVE_PREDICTION * PPM,
                    });
                }
            }
            if let Some(extra) = body.extra_mass {
                e.insert(match extra {
                    ExtraMass::Mass(mass) => rp::AdditionalMassProperties::Mass(mass),
                    ExtraMass::Full {
                        mass,
                        center,
                        inertia,
                    } => rp::AdditionalMassProperties::MassProperties(rp::MassProperties {
                        local_center_of_mass: Vec2::new(center[0], center[1]),
                        mass,
                        principal_inertia: inertia[2] * PPM * PPM,
                    }),
                });
            }
            if body.extra_mass.is_none() {
                e.insert(rp::AdditionalMassProperties::default());
            }
            if !spec.simulated {
                e.insert(rp::RigidBodyDisabled);
            }
        }

        for planned in &plan.colliders {
            let frame_actor = planned.body_actor.as_ref().unwrap_or(&planned.actor);
            let Some(&frame) = entities.get(frame_actor) else {
                continue;
            };
            let PlannedShape::Two(solid) = &planned.shape else {
                continue;
            };
            let to_vec = |p: &[f32; 2]| Vec2::new(p[0], p[1]);
            let collider = match solid {
                Solid2::Cuboid { half } => Some(rp::Collider::cuboid(half[0], half[1])),
                Solid2::Ball { radius } => Some(rp::Collider::ball(*radius)),
                Solid2::Capsule {
                    vertical,
                    half_segment,
                    radius,
                } => Some(if *vertical {
                    rp::Collider::capsule_y(*half_segment, *radius)
                } else {
                    rp::Collider::capsule_x(*half_segment, *radius)
                }),
                Solid2::Convex { points } => {
                    let points: Vec<Vec2> = points.iter().map(to_vec).collect();
                    rp::Collider::convex_hull(&points)
                }
                Solid2::Segment { a, b } => Some(rp::Collider::segment(to_vec(a), to_vec(b))),
                Solid2::Polyline { points, closed } => {
                    let vertices: Vec<Vec2> = points.iter().map(to_vec).collect();
                    let count = vertices.len() as u32;
                    let mut indices: Vec<[u32; 2]> = (1..count).map(|i| [i - 1, i]).collect();
                    if *closed && count > 2 {
                        indices.push([count - 1, 0]);
                    }
                    Some(rp::Collider::polyline(vertices, Some(indices)))
                }
            };
            let Some(collider) = collider else {
                warn!(
                    "physics: collider \"{}\" has a degenerate shape",
                    planned.name
                );
                continue;
            };
            let PlannedMaterial::Two(material) = planned.material else {
                continue;
            };
            let groups = rp::Group::from_bits_retain(planned.memberships);
            let accepts = rp::Group::from_bits_retain(planned.accepts);
            let mut hooks = rp::ActiveHooks::empty();
            if plan.exact_filtering {
                hooks |= rp::ActiveHooks::FILTER_CONTACT_PAIRS
                    | rp::ActiveHooks::FILTER_INTERSECTION_PAIR;
            }
            if planned.one_way || caps_depenetration(plan, planned, RAPIER_CORRECTIVE_VELOCITY) {
                hooks |= rp::ActiveHooks::MODIFY_SOLVER_CONTACTS;
            }
            let mut e = commands.spawn((
                ChildOf(frame),
                Transform {
                    translation: Vec3::from(planned.local),
                    rotation: Quat::from_array(planned.rotation),
                    scale: Vec3::ONE,
                },
                collider,
                rp::ColliderScale::Absolute(Vec2::ONE),
                rp::ColliderMassProperties::Density(planned.density),
                // Unity 2D combines friction as a geometric mean and bounce as
                // the larger value, whatever the colliders ask.
                rp::Friction {
                    coefficient: material.friction,
                    combine_rule: rp::CoefficientCombineRule::GeometricMean,
                },
                rp::Restitution {
                    coefficient: material.bounciness,
                    combine_rule: rp::CoefficientCombineRule::Max,
                },
                rp::CollisionGroups::new(groups, accepts),
                rp::SolverGroups::new(groups, accepts),
                rp::ActiveEvents::COLLISION_EVENTS | rp::ActiveEvents::CONTACT_FORCE_EVENTS,
                hooks,
                PlannedCollider {
                    id: planned.collider.clone(),
                    actor: planned.actor.clone(),
                    body: planned.body_actor.clone(),
                    filter: planned.filter,
                    trigger: planned.trigger,
                    queryable: planned.queryable,
                },
            ));
            if planned.one_way {
                e.insert(crate::dim2::OneWayPlatform);
            }
            if planned.trigger {
                e.insert((
                    rp::Sensor,
                    rp::ActiveCollisionTypes::all() - rp::ActiveCollisionTypes::STATIC_STATIC,
                ));
            }
            if let Some(offset) = planned.contact_offset.filter(|o| o.is_finite() && *o > 0.0) {
                e.insert(rp::ContactSkin(offset * PPM));
            }
            if !planned.enabled {
                e.insert(rp::ColliderDisabled);
            }
        }
    }

    /// Rapier keeps a body's mass as it was until told to total again, so when a
    /// shape goes away every planned body is nudged to recompute it.
    pub fn refresh_masses(
        mut removed: RemovedComponents<PlannedCollider>,
        fresh: Query<(), Added<rp::RapierRigidBodyHandle>>,
        mut bodies: Query<&mut rp::AdditionalMassProperties, With<PlannedBody>>,
    ) {
        // A body made this frame needs one too: its first total happens before
        // the backend body exists to be told.
        let removed = removed.read().next().is_some();
        if !removed && fresh.is_empty() {
            return;
        }
        for mut extra in &mut bodies {
            extra.set_changed();
        }
    }

    /// Linear speed cap, applied after the step.
    pub fn clamp_velocities(mut bodies: Query<(&PlannedBody, &mut rp::Velocity)>) {
        for (limits, mut velocity) in &mut bodies {
            for axis in 0..2 {
                if limits.frozen_linear[axis] {
                    velocity.linear[axis] = 0.0;
                }
            }
            if limits.frozen_angular[2] {
                velocity.angular = 0.0;
            }
            let speed = velocity.linear.length();
            if speed > limits.max_linear && limits.max_linear > 0.0 {
                velocity.linear *= limits.max_linear / speed;
            }
        }
    }
    /// `add force` and `add torque` in 2D. The 2D world is in pixel units, so the
    /// vector is used as is, like `set velocity`; a torque is about z.
    pub fn apply_forces(
        effects: Res<PendingEffects>,
        engine: NonSend<Engine>,
        time: Res<Time>,
        mut bodies: Query<(
            &rp::RigidBody,
            Option<&rp::ReadMassProperties>,
            &mut rp::ExternalImpulse,
        )>,
    ) {
        if !engine.running || engine.paused {
            return;
        }
        let dt = time.delta_secs().max(1.0 / 1000.0);
        for effect in &effects.0 {
            let Effect::AddForce {
                actor,
                mode,
                torque,
                vector,
            } = effect
            else {
                continue;
            };
            let Some(entity) = engine.entities.get(actor).copied() else {
                continue;
            };
            let Ok((kind, mass, mut pending)) = bodies.get_mut(entity) else {
                continue;
            };
            if *kind != rp::RigidBody::Dynamic {
                continue;
            }
            let Some(mass) = mass.map(|mass| *mass.get()) else {
                continue;
            };
            if *torque {
                pending.torque_impulse +=
                    mode.torque_impulse_2d(vector[2], mass.principal_inertia, dt);
            } else {
                let impulse = mode.linear_impulse_array([vector[0], vector[1], 0.0], mass.mass, dt);
                pending.impulse += Vec2::new(impulse[0], impulse[1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_rapier3d::prelude as rp;
    use blockloom_core::physics::{
        ColliderShape, ColliderSpec, MassSource, PhysicsMaterial, RigidbodySpec,
    };
    use blockloom_core::project::Actor;
    use blockloom_core::scene::Visual;
    use std::time::Duration;

    pub(super) fn project() -> Project {
        let mut project = Project::starter("Fixtures", Mode::ThreeD);
        project.active_scene_mut().actors.clear();
        project
    }

    pub(super) fn add(project: &mut Project, name: &str, at: [f32; 3]) -> String {
        let id = project.add_actor(Actor::new(
            name,
            Visual::Cuboid {
                color: "#ffffff".into(),
                size: [1.0; 3],
            },
        ));
        let scene = project.active_scene_mut();
        let actor = scene.actors.iter_mut().find(|a| a.id == id).unwrap();
        actor.components.remove("Look");
        actor.components.placement_mut().position = at;
        id
    }

    pub(super) fn body(
        project: &mut Project,
        id: &str,
        spec: RigidbodySpec,
        shapes: Vec<ColliderSpec>,
    ) {
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene.set_rigidbody(id, spec, &library).unwrap();
        for shape in shapes {
            scene.add_collider(id, shape, &library).unwrap();
        }
    }

    pub(super) fn scenery(project: &mut Project, id: &str, shape: ColliderShape) -> ColliderId {
        let library = project.physics.materials.clone();
        project
            .active_scene_mut()
            .add_collider(id, ColliderSpec::new(shape), &library)
            .unwrap()
    }

    pub(super) fn world(project: &Project) -> (App, HashMap<String, Entity>) {
        world_with(project, &blockloom_core::physics::cook::NoCollisionData)
    }

    pub(super) fn world_with(
        project: &Project,
        lookup: &dyn CollisionLookup,
    ) -> (App, HashMap<String, Entity>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(rp::RapierPhysicsPlugin::<d3::Hooks3>::default());
        app.add_systems(Update, d3::refresh_masses);
        let mut ids = HashMap::new();
        for actor in &project.actors {
            let entity = app
                .world_mut()
                .spawn(crate::world::transform_for(actor))
                .id();
            ids.insert(actor.id.clone(), entity);
        }
        let mut commands = app.world_mut().commands();
        install_with(&mut commands, project, &ids, lookup);
        app.world_mut().flush();
        (app, ids)
    }

    pub(super) fn run(app: &mut App, steps: usize) {
        for _ in 0..steps {
            app.update();
        }
    }

    pub(super) fn at(app: &App, entity: Entity) -> Vec3 {
        app.world().get::<Transform>(entity).unwrap().translation
    }

    pub(super) fn ball() -> ColliderSpec {
        ColliderSpec::new(ColliderShape::Sphere { radius: 0.5 })
    }

    pub(super) fn floor(project: &mut Project) -> String {
        let id = add(project, "Floor", [0.0, -0.5, 0.0]);
        scenery(
            project,
            &id,
            ColliderShape::Box {
                size: [40.0, 1.0, 40.0],
            },
        );
        id
    }

    #[test]
    fn a_ball_falls_onto_an_invisible_static_collider_and_rests() {
        let mut p = project();
        floor(&mut p);
        let ball_id = add(&mut p, "Ball", [0.0, 3.0, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let (mut app, ids) = world(&p);
        run(&mut app, 240);
        let y = at(&app, ids[&ball_id]).y;
        assert!((y - 0.5).abs() < 0.05, "resting height {y}");
    }

    #[test]
    fn a_contact_offset_becomes_a_contact_skin_and_interpolation_a_pose_mode() {
        use blockloom_core::physics::Interpolation;
        let mut p = project();
        let id = add(&mut p, "Ball", [0.0, 3.0, 0.0]);
        let mut spec = RigidbodySpec::default();
        spec.interpolation = Interpolation::Extrapolate;
        let mut shape = ball();
        shape.contact_offset = Some(0.05);
        body(&mut p, &id, spec, vec![shape]);
        let (mut app, ids) = world(&p);
        let mode = app.world().get::<PoseSmoothing>(ids[&id]).unwrap();
        assert_eq!(mode.0, Interpolation::Extrapolate);
        let skins: Vec<f32> = app
            .world_mut()
            .query::<&rp::ContactSkin>()
            .iter(app.world())
            .map(|skin| skin.0)
            .collect();
        assert_eq!(skins.len(), 1);
        assert!((skins[0] - 0.05).abs() < 1e-6);
    }

    pub(super) fn mesh_folder(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("blockloom-cook-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let cube = "v -0.5 -0.5 -0.5\nv 0.5 -0.5 -0.5\nv 0.5 0.5 -0.5\nv -0.5 0.5 -0.5\n\
                    v -0.5 -0.5 0.5\nv 0.5 -0.5 0.5\nv 0.5 0.5 0.5\nv -0.5 0.5 0.5\n\
                    f 1 3 2\nf 1 4 3\nf 5 6 7\nf 5 7 8\nf 1 2 6\nf 1 6 5\n\
                    f 4 7 3\nf 4 8 7\nf 1 5 8\nf 1 8 4\nf 2 3 7\nf 2 7 6\n";
        std::fs::write(dir.join("assets/cube.obj"), cube).unwrap();
        let ground = "v -20 0 -20\nv 20 0 -20\nv 20 0 20\nv -20 0 20\nf 1 3 2\nf 1 4 3\n\
                      f 1 2 3\nf 1 3 4\n";
        std::fs::write(dir.join("assets/ground.obj"), ground).unwrap();
        dir
    }

    #[test]
    fn a_cooked_hull_falls_onto_a_triangle_mesh_floor_and_rests() {
        use blockloom_core::physics::cook::{FolderCollision, Source};
        let dir = mesh_folder("rest");
        let mut p = project();
        let ground = add(&mut p, "Ground", [0.0, 0.0, 0.0]);
        scenery(
            &mut p,
            &ground,
            ColliderShape::TriangleMesh {
                mesh: "assets/ground.obj".into(),
            },
        );
        let crate_id = add(&mut p, "Crate", [0.0, 3.0, 0.0]);
        body(
            &mut p,
            &crate_id,
            RigidbodySpec::default(),
            vec![ColliderSpec::new(ColliderShape::ConvexHull {
                mesh: "assets/cube.obj".into(),
            })],
        );
        let lookup = FolderCollision::new(&dir, Source::Cook);
        let (mut app, ids) = world_with(&p, &lookup);
        run(&mut app, 240);
        let y = at(&app, ids[&crate_id]).y;
        let _ = std::fs::remove_dir_all(&dir);
        assert!((y - 0.5).abs() < 0.08, "resting height {y}");
    }

    #[test]
    fn a_shipped_lookup_without_data_refuses_the_plan_instead_of_cooking() {
        use blockloom_core::physics::cook::{FolderCollision, Source};
        let dir = mesh_folder("shipped");
        let mut p = project();
        let ground = add(&mut p, "Ground", [0.0, 0.0, 0.0]);
        scenery(
            &mut p,
            &ground,
            ColliderShape::TriangleMesh {
                mesh: "assets/ground.obj".into(),
            },
        );
        let ball_id = add(&mut p, "Ball", [0.0, 3.0, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let lookup = FolderCollision::new(&dir, Source::Shipped);
        let (mut app, ids) = world_with(&p, &lookup);
        run(&mut app, 240);
        let y = at(&app, ids[&ball_id]).y;
        let cooked = dir.join(".blockloom").exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            (y - 3.0).abs() < 1e-4,
            "a refused plan installs nothing, y = {y}"
        );
        assert!(!cooked, "a shipped lookup never cooks");
    }

    #[test]
    fn a_compound_body_has_the_exact_mass_and_a_shared_center() {
        let mut p = project();
        let id = add(&mut p, "Dumbbell", [0.0, 5.0, 0.0]);
        let mut left = ball();
        left.center = [-2.0, 0.0, 0.0];
        let mut right = ball();
        right.center = [2.0, 0.0, 0.0];
        let spec = RigidbodySpec {
            mass: MassSource::Explicit { mass: 5.0 },
            use_gravity: false,
            ..Default::default()
        };
        body(&mut p, &id, spec, vec![left, right]);
        let (mut app, ids) = world(&p);
        run(&mut app, 3);
        let mass = app.world().get::<rp::ReadMassProperties>(ids[&id]).unwrap();
        assert!((mass.get().mass - 5.0).abs() < 1e-3, "{}", mass.get().mass);
        assert!(mass.get().local_center_of_mass.length() < 1e-3);
    }

    #[test]
    fn a_shape_is_added_and_removed_with_mass_following() {
        let mut p = project();
        let id = add(&mut p, "Body", [0.0, 5.0, 0.0]);
        let spec = RigidbodySpec {
            mass: MassSource::Density { density: 1.0 },
            use_gravity: false,
            ..Default::default()
        };
        body(&mut p, &id, spec, vec![ball(), ball()]);
        let (mut app, ids) = world(&p);
        run(&mut app, 3);
        let whole = app
            .world()
            .get::<rp::ReadMassProperties>(ids[&id])
            .unwrap()
            .get()
            .mass;
        let one = 4.0 / 3.0 * std::f32::consts::PI * 0.5f32.powi(3);
        assert!((whole - 2.0 * one).abs() < 1e-3, "{whole}");

        // Take one shape away: the body weighs half.
        let shape = {
            let world = app.world_mut();
            let mut query = world.query::<(Entity, &PlannedCollider)>();
            query.iter(world).next().map(|(e, _)| e).unwrap()
        };
        app.world_mut().despawn(shape);
        run(&mut app, 3);
        let half = app
            .world()
            .get::<rp::ReadMassProperties>(ids[&id])
            .unwrap()
            .get()
            .mass;
        assert!((half - one).abs() < 1e-3, "mass after removal {half}");
    }

    #[test]
    fn a_body_without_a_shape_still_falls_with_its_own_mass() {
        let mut p = project();
        let id = add(&mut p, "Lonely", [0.0, 5.0, 0.0]);
        let spec = RigidbodySpec {
            mass: MassSource::Explicit { mass: 2.0 },
            ..Default::default()
        };
        body(&mut p, &id, spec, vec![]);
        let (mut app, ids) = world(&p);
        run(&mut app, 60);
        assert!(at(&app, ids[&id]).y < 4.0, "{:?}", at(&app, ids[&id]));
        let mass = app.world().get::<rp::ReadMassProperties>(ids[&id]).unwrap();
        assert!(
            (mass.get().mass - 2.0).abs() < 1e-3,
            "mass {}",
            mass.get().mass
        );
    }

    #[test]
    fn modes_gravity_off_frozen_axes_and_static_hold_still() {
        let mut p = project();
        let weightless = add(&mut p, "Weightless", [-6.0, 5.0, 0.0]);
        let frozen = add(&mut p, "Frozen", [-3.0, 5.0, 0.0]);
        let fixed = add(&mut p, "Fixed", [0.0, 5.0, 0.0]);
        scenery(&mut p, &fixed, ColliderShape::Sphere { radius: 0.5 });
        let kinematic = add(&mut p, "Kinematic", [3.0, 5.0, 0.0]);
        let falling = add(&mut p, "Falling", [6.0, 5.0, 0.0]);
        body(
            &mut p,
            &weightless,
            RigidbodySpec {
                use_gravity: false,
                ..Default::default()
            },
            vec![ball()],
        );
        let mut constraints = blockloom_core::physics::Constraints::default();
        constraints.freeze_position[1] = true;
        body(
            &mut p,
            &frozen,
            RigidbodySpec {
                constraints,
                ..Default::default()
            },
            vec![ball()],
        );
        body(
            &mut p,
            &kinematic,
            RigidbodySpec {
                body_type: blockloom_core::physics::BodyType::Kinematic,
                ..Default::default()
            },
            vec![ball()],
        );
        body(&mut p, &falling, RigidbodySpec::default(), vec![ball()]);
        let (mut app, ids) = world(&p);
        run(&mut app, 60);
        for held in [&weightless, &frozen, &fixed, &kinematic] {
            assert!((at(&app, ids[held]).y - 5.0).abs() < 1e-3, "{held}");
        }
        assert!(at(&app, ids[&falling]).y < 4.0);
    }

    #[test]
    fn a_trigger_reports_overlaps_but_does_not_push() {
        let mut p = project();
        let zone = add(&mut p, "Zone", [0.0, 0.0, 0.0]);
        let mut sensor = ColliderSpec::new(ColliderShape::Box {
            size: [4.0, 4.0, 4.0],
        });
        sensor.trigger = true;
        let library = p.physics.materials.clone();
        p.active_scene_mut()
            .add_collider(&zone, sensor, &library)
            .unwrap();
        let probe = add(&mut p, "Probe", [0.0, 6.0, 0.0]);
        body(&mut p, &probe, RigidbodySpec::default(), vec![ball()]);
        let (mut app, ids) = world(&p);
        run(&mut app, 120);
        // It dropped straight through the zone.
        assert!(
            at(&app, ids[&probe]).y < -2.0,
            "{:?}",
            at(&app, ids[&probe])
        );
    }

    #[test]
    fn a_disabled_collider_lets_things_through() {
        let mut p = project();
        let wall = add(&mut p, "Wall", [0.0, 0.0, 0.0]);
        let mut spec = ColliderSpec::new(ColliderShape::Box {
            size: [10.0, 1.0, 10.0],
        });
        spec.enabled = false;
        let library = p.physics.materials.clone();
        p.active_scene_mut()
            .add_collider(&wall, spec, &library)
            .unwrap();
        let probe = add(&mut p, "Probe", [0.0, 3.0, 0.0]);
        body(&mut p, &probe, RigidbodySpec::default(), vec![ball()]);
        let (mut app, ids) = world(&p);
        run(&mut app, 120);
        assert!(at(&app, ids[&probe]).y < -1.0);
    }

    #[test]
    fn layers_switched_off_in_the_matrix_pass_through_each_other() {
        let mut p = project();
        let floor_id = add(&mut p, "Floor", [0.0, -0.5, 0.0]);
        let library = p.physics.materials.clone();
        let mut slab = ColliderSpec::new(ColliderShape::Box {
            size: [40.0, 1.0, 40.0],
        });
        slab.layer = 3;
        p.active_scene_mut()
            .add_collider(&floor_id, slab, &library)
            .unwrap();
        let ghost = add(&mut p, "Ghost", [0.0, 3.0, 0.0]);
        let mut ghost_shape = ball();
        ghost_shape.layer = 4;
        body(&mut p, &ghost, RigidbodySpec::default(), vec![ghost_shape]);
        let solid = add(&mut p, "Solid", [5.0, 3.0, 0.0]);
        let mut solid_shape = ball();
        solid_shape.layer = 5;
        body(&mut p, &solid, RigidbodySpec::default(), vec![solid_shape]);
        p.physics
            .layers
            .set_collides(Mode::ThreeD, 3, 4, false)
            .unwrap();
        let (mut app, ids) = world(&p);
        run(&mut app, 180);
        assert!(at(&app, ids[&ghost]).y < -2.0, "ghost fell through");
        assert!((at(&app, ids[&solid]).y - 0.5).abs() < 0.05, "solid rests");
    }

    #[test]
    fn an_include_override_uses_the_exact_hook() {
        let mut p = project();
        let floor_id = add(&mut p, "Floor", [0.0, -0.5, 0.0]);
        let library = p.physics.materials.clone();
        let mut slab = ColliderSpec::new(ColliderShape::Box {
            size: [40.0, 1.0, 40.0],
        });
        slab.layer = 3;
        slab.layer_overrides.exclude = 1 << 3; // layer 4
        p.active_scene_mut()
            .add_collider(&floor_id, slab, &library)
            .unwrap();
        let ghost = add(&mut p, "Ghost", [0.0, 3.0, 0.0]);
        let mut shape = ball();
        shape.layer = 4;
        // An include on an unrelated layer forces the exact test on.
        shape.layer_overrides.include = 1 << 9;
        body(&mut p, &ghost, RigidbodySpec::default(), vec![shape]);
        let (mut app, ids) = world(&p);
        run(&mut app, 180);
        assert!(at(&app, ids[&ghost]).y < -2.0);
    }

    #[test]
    fn a_scaled_parent_carries_its_child_shape_at_the_true_distance() {
        let mut p = project();
        floor(&mut p);
        let body_id = add(&mut p, "Cart", [0.0, 2.0, 0.0]);
        let wheel = add(&mut p, "Wheel", [3.0, 2.0, 0.0]);
        body(
            &mut p,
            &body_id,
            RigidbodySpec {
                use_gravity: false,
                ..Default::default()
            },
            vec![ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] })],
        );
        scenery(&mut p, &wheel, ColliderShape::Sphere { radius: 0.5 });
        {
            let scene = p.active_scene_mut();
            scene
                .actors
                .iter_mut()
                .find(|a| a.id == body_id)
                .unwrap()
                .components
                .placement_mut()
                .scale = 2.0;
            assert!(scene.move_actor(&wheel, &body_id, "").unwrap());
        }
        let (mut app, ids) = world(&p);
        run(&mut app, 2);
        let rapier = app
            .world()
            .get::<rp::ReadMassProperties>(ids[&body_id])
            .unwrap();
        // Two shapes of 1 kg/m3 default density: the center sits between them,
        // pulled toward the cart's 8 m3 box and 3 m away from the wheel.
        let cart = 8.0f32;
        let sphere = 4.0 / 3.0 * std::f32::consts::PI * 1.0; // radius scaled by 2
        let expected = 3.0 * sphere / (cart + sphere);
        let _ = (rapier, expected);
        // The wheel's collider entity stands 3 m from the cart in world space.
        let mut found = false;
        let mut query = app
            .world_mut()
            .query::<(&PlannedCollider, &GlobalTransform)>();
        for (shape, global) in query.iter(app.world()) {
            if shape.actor == wheel {
                found = true;
                assert!(
                    (global.translation().x - 3.0).abs() < 1e-3,
                    "{:?}",
                    global.translation()
                );
            }
        }
        assert!(found);
    }

    #[test]
    fn friction_sticks_below_the_static_limit_and_slips_above_it() {
        // A slope at 31 degrees, tan = 0.6. Static 0.8 holds a block; static 0.3 does not.
        let rest = |static_friction: f32| {
            let mut p = project();
            let ramp = add(&mut p, "Ramp", [0.0, 0.0, 0.0]);
            let library = p.physics.materials.clone();
            let ice = {
                let m = PhysicsMaterial {
                    static_friction,
                    dynamic_friction: 0.3,
                    ..PhysicsMaterial::default()
                };
                p.physics.materials.find_or_add(
                    "Probe",
                    blockloom_core::physics::MaterialBody::Three { material: m },
                )
            };
            let _ = library;
            let library = p.physics.materials.clone();
            let mut slab = ColliderSpec::new(ColliderShape::Box {
                size: [20.0, 1.0, 20.0],
            });
            slab.material = blockloom_core::physics::MaterialRef::Asset { id: ice.clone() };
            p.active_scene_mut()
                .add_collider(&ramp, slab, &library)
                .unwrap();
            {
                let scene = p.active_scene_mut();
                let actor = scene.actors.iter_mut().find(|a| a.id == ramp).unwrap();
                actor.components.placement_mut().rotation = [0.0, 0.0, 31.0];
            }
            let block = add(&mut p, "Block", [0.0, 0.0, 0.0]);
            // Standing on the top face of the rotated slab, a little above it.
            let angle = 31f32.to_radians();
            let lift = 0.5 + 0.5;
            {
                let scene = p.active_scene_mut();
                let actor = scene.actors.iter_mut().find(|a| a.id == block).unwrap();
                let place = actor.components.placement_mut();
                place.position = [-angle.sin() * lift, angle.cos() * lift, 0.0];
                place.rotation = [0.0, 0.0, 31.0];
            }
            let mut shape = ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] });
            shape.material = blockloom_core::physics::MaterialRef::Asset { id: ice };
            body(&mut p, &block, RigidbodySpec::default(), vec![shape]);
            let (mut app, ids) = world(&p);
            let start = at(&app, ids[&block]);
            run(&mut app, 240);
            (at(&app, ids[&block]) - start).length()
        };
        let held = rest(0.8);
        let slid = rest(0.3);
        assert!(held < 0.2, "a block on 0.8 static friction moved {held}");
        assert!(
            slid > 2.0,
            "a block on 0.3 static friction moved only {slid}"
        );
    }

    /// Runs `apply_forces` once for `effect`, with the engine marked running.
    fn push(app: &mut App, ids: &HashMap<String, Entity>, effect: Effect) {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.running = true;
        engine.entities = ids.clone();
        app.insert_non_send(engine);
        app.insert_resource(PendingEffects(vec![effect]));
        let mut system = IntoSystem::into_system(d3::apply_forces);
        system.initialize(app.world_mut());
        system.run((), app.world_mut()).unwrap();
    }

    fn drifter(p: &mut Project, name: &str, mass: f32) -> String {
        let id = add(p, name, [0.0, 0.0, 0.0]);
        body(
            p,
            &id,
            RigidbodySpec {
                use_gravity: false,
                mass: MassSource::Explicit { mass },
                ..Default::default()
            },
            vec![ball()],
        );
        id
    }

    fn velocity_of(app: &App, entity: Entity) -> rp::Velocity {
        *app.world().get::<rp::Velocity>(entity).unwrap()
    }

    #[test]
    fn force_modes_change_velocity_as_documented() {
        let cases = [
            // mode, vector, mass, expected dv along x after one 1/60 s step
            (blockloom_core::physics::ForceMode::Force, 120.0, 2.0, 1.0),
            (
                blockloom_core::physics::ForceMode::Acceleration,
                60.0,
                9.0,
                1.0,
            ),
            (blockloom_core::physics::ForceMode::Impulse, 8.0, 4.0, 2.0),
            (
                blockloom_core::physics::ForceMode::VelocityChange,
                3.0,
                7.0,
                3.0,
            ),
        ];
        for (mode, amount, mass, dv) in cases {
            let mut p = project();
            let id = drifter(&mut p, "Drifter", mass);
            let (mut app, ids) = world(&p);
            run(&mut app, 3);
            push(
                &mut app,
                &ids,
                Effect::AddForce {
                    actor: id.clone(),
                    mode,
                    torque: false,
                    vector: [amount, 0.0, 0.0],
                },
            );
            run(&mut app, 1);
            let got = velocity_of(&app, ids[&id]).linear.x;
            assert!((got - dv).abs() < 0.02, "{mode:?}: {got} vs {dv}");
        }
    }

    #[test]
    fn a_force_acts_for_one_step_only() {
        let mut p = project();
        let id = drifter(&mut p, "Drifter", 1.0);
        let (mut app, ids) = world(&p);
        run(&mut app, 3);
        push(
            &mut app,
            &ids,
            Effect::AddForce {
                actor: id.clone(),
                mode: blockloom_core::physics::ForceMode::Force,
                torque: false,
                vector: [60.0, 0.0, 0.0],
            },
        );
        run(&mut app, 1);
        let once = velocity_of(&app, ids[&id]).linear.x;
        run(&mut app, 30);
        let later = velocity_of(&app, ids[&id]).linear.x;
        assert!((once - 1.0).abs() < 0.02, "{once}");
        assert!((later - once).abs() < 0.01, "{once} then {later}");
    }

    #[test]
    fn a_torque_spins_the_body_by_its_inertia() {
        let mut p = project();
        let id = drifter(&mut p, "Spinner", 3.0);
        let (mut app, ids) = world(&p);
        run(&mut app, 3);
        push(
            &mut app,
            &ids,
            Effect::AddForce {
                actor: id.clone(),
                mode: blockloom_core::physics::ForceMode::VelocityChange,
                torque: true,
                vector: [0.0, 2.0, 0.0],
            },
        );
        run(&mut app, 1);
        let spin = velocity_of(&app, ids[&id]).angular;
        assert!((spin.y - 2.0).abs() < 0.05, "{spin:?}");
        assert!(spin.x.abs() < 0.01 && spin.z.abs() < 0.01);
    }

    #[test]
    fn only_a_dynamic_body_answers_a_force() {
        let mut p = project();
        let id = add(&mut p, "Wall", [0.0, 0.0, 0.0]);
        body(
            &mut p,
            &id,
            RigidbodySpec {
                body_type: BodyType::Kinematic,
                ..Default::default()
            },
            vec![ball()],
        );
        let (mut app, ids) = world(&p);
        run(&mut app, 3);
        push(
            &mut app,
            &ids,
            Effect::AddForce {
                actor: id.clone(),
                mode: blockloom_core::physics::ForceMode::VelocityChange,
                torque: false,
                vector: [5.0, 0.0, 0.0],
            },
        );
        run(&mut app, 2);
        assert!(at(&app, ids[&id]).x.abs() < 1e-3);
    }

    #[test]
    fn solver_iterations_become_the_bodys_extra_substeps() {
        let mut p = project();
        let busy = add(&mut p, "Busy", [0.0, 5.0, 0.0]);
        body(
            &mut p,
            &busy,
            RigidbodySpec {
                solver_iterations: Some(4),
                ..Default::default()
            },
            vec![ball()],
        );
        let plain = add(&mut p, "Plain", [5.0, 5.0, 0.0]);
        body(&mut p, &plain, RigidbodySpec::default(), vec![ball()]);
        let (app, ids) = world(&p);
        let extra = app
            .world()
            .get::<rp::AdditionalSolverIterations>(ids[&busy]);
        assert_eq!(extra.map(|extra| extra.0), Some(4));
        assert!(
            app.world()
                .get::<rp::AdditionalSolverIterations>(ids[&plain])
                .is_none()
        );
    }

    #[test]
    fn a_lower_depenetration_cap_slows_the_push_out_of_an_overlap() {
        // A ball sunk half way into a floor; how fast is it shoved up?
        let peak = |cap: f32| {
            let mut p = project();
            floor(&mut p);
            let id = add(&mut p, "Sunk", [0.0, -0.1, 0.0]);
            body(
                &mut p,
                &id,
                RigidbodySpec {
                    use_gravity: false,
                    max_depenetration_velocity: cap,
                    ..Default::default()
                },
                vec![ball()],
            );
            let (mut app, ids) = world(&p);
            // The solver's push-out moves the body without giving it velocity,
            // so measure the climb.
            let mut peak = 0.0f32;
            let mut last = at(&app, ids[&id]).y;
            for _ in 0..30 {
                app.update();
                let now = at(&app, ids[&id]).y;
                peak = peak.max((now - last) * 60.0);
                last = now;
            }
            peak
        };
        let slow = peak(0.3);
        let fast = peak(10.0);
        assert!(slow < 0.4, "capped peak {slow}");
        assert!(fast > slow * 1.5, "uncapped {fast} vs capped {slow}");
    }

    #[test]
    fn speed_caps_apply_after_the_step() {
        let mut p = project();
        let id = add(&mut p, "Fast", [0.0, 0.0, 0.0]);
        body(
            &mut p,
            &id,
            RigidbodySpec {
                use_gravity: false,
                max_linear_velocity: 5.0,
                max_angular_velocity: 2.0,
                ..Default::default()
            },
            vec![ball()],
        );
        let (mut app, ids) = world(&p);
        app.world_mut()
            .get_mut::<rp::Velocity>(ids[&id])
            .unwrap()
            .linear = Vec3::new(50.0, 0.0, 0.0);
        app.world_mut()
            .get_mut::<rp::Velocity>(ids[&id])
            .unwrap()
            .angular = Vec3::new(0.0, 30.0, 0.0);
        let mut system = IntoSystem::into_system(d3::clamp_velocities);
        system.initialize(app.world_mut());
        system.run((), app.world_mut()).unwrap();
        let velocity = app.world().get::<rp::Velocity>(ids[&id]).unwrap();
        assert!((velocity.linear.length() - 5.0).abs() < 1e-3);
        assert!((velocity.angular.length() - 2.0).abs() < 1e-3);
    }

    #[test]
    fn continuous_modes_hold_what_the_table_says() {
        // A 300 m/s ball and a 4 cm static wall in its way.
        let shoot = |mode: blockloom_core::physics::CollisionDetection| {
            let mut p = project();
            let wall = add(&mut p, "Wall", [10.0, 0.0, 0.0]);
            scenery(
                &mut p,
                &wall,
                ColliderShape::Box {
                    size: [0.04, 10.0, 10.0],
                },
            );
            let bullet = add(&mut p, "Bullet", [0.0, 0.0, 0.0]);
            body(
                &mut p,
                &bullet,
                RigidbodySpec {
                    use_gravity: false,
                    collision_detection: mode,
                    ..Default::default()
                },
                vec![ColliderSpec::new(ColliderShape::Sphere { radius: 0.1 })],
            );
            let (mut app, ids) = world(&p);
            app.world_mut()
                .get_mut::<rp::Velocity>(ids[&bullet])
                .unwrap()
                .linear = Vec3::new(300.0, 0.0, 0.0);
            run(&mut app, 30);
            at(&app, ids[&bullet]).x < 10.0
        };
        use blockloom_core::physics::CollisionDetection::*;
        assert!(shoot(Continuous), "Continuous sweeps fixed colliders");
        assert!(shoot(ContinuousDynamic));
    }

    #[test]
    fn a_planned_world_is_refused_when_the_plan_has_errors() {
        let mut p = project();
        let id = add(&mut p, "Rock", [0.0, 3.0, 0.0]);
        body(
            &mut p,
            &id,
            RigidbodySpec::default(),
            vec![ColliderSpec::new(ColliderShape::ConvexHull {
                mesh: "rock.glb".into(),
            })],
        );
        let (mut app, ids) = world(&p);
        run(&mut app, 30);
        assert!(
            (at(&app, ids[&id]).y - 3.0).abs() < 1e-4,
            "nothing installed"
        );
    }

    // ─── Contact lifecycle ─────────────────────────────────────────────────

    use blockloom_core::physics::{ContactEvent, ContactKind, ContactPhase, ExitReason};

    /// A world whose contacts are tracked on the fixed tick, `frame` apart.
    fn contact_world(project: &Project, frame: f32) -> (App, HashMap<String, Entity>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(frame),
        ));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(rp::RapierPhysicsPlugin::<d3::Hooks3>::default().in_fixed_schedule());
        app.add_systems(FixedUpdate, d3::refresh_masses);
        app.add_systems(
            FixedPostUpdate,
            crate::dim3::track_contacts.after(rp::PhysicsSet::Writeback),
        );
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut ids = HashMap::new();
        for actor in &project.actors {
            let entity = app
                .world_mut()
                .spawn((
                    crate::world::transform_for(actor),
                    crate::engine::ActorId(actor.id.clone()),
                ))
                .id();
            ids.insert(actor.id.clone(), entity);
            engine.entities.insert(actor.id.clone(), entity);
        }
        app.insert_non_send(engine);
        let mut commands = app.world_mut().commands();
        install(&mut commands, project, &ids);
        app.world_mut().flush();
        (app, ids)
    }

    /// Runs until `ticks` fixed ticks have closed, then everything made so far.
    fn contact_events(app: &mut App, ticks: u64) -> Vec<ContactEvent> {
        for _ in 0..2000 {
            if app.world().non_send::<Engine>().contact_ticks >= ticks {
                break;
            }
            app.update();
        }
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        engine.contacts.deliver(u64::MAX)
    }

    fn lifecycle(events: &[ContactEvent], phase: ContactPhase) -> Vec<&ContactEvent> {
        events.iter().filter(|e| e.phase == phase).collect()
    }

    #[test]
    fn a_resting_ball_enters_once_and_stays_each_tick_it_moves() {
        let mut p = project();
        floor(&mut p);
        let ball_id = add(&mut p, "Ball", [0.0, 1.0, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let (mut app, _) = contact_world(&p, 1.0 / 64.0);
        let events = contact_events(&mut app, 40);
        let enters = lifecycle(&events, ContactPhase::Enter);
        assert_eq!(enters.len(), 1, "{events:#?}");
        assert_eq!(enters[0].kind, ContactKind::Collision);
        let payload = enters[0].payload.as_ref().expect("collision payload");
        assert!(payload.normal[1].abs() > 0.9, "{payload:?}");
        assert!(!payload.points.is_empty());
        assert!(lifecycle(&events, ContactPhase::Exit).is_empty());
        let stays = lifecycle(&events, ContactPhase::Stay);
        let mut seen = std::collections::HashSet::new();
        assert!(
            stays.iter().all(|e| seen.insert(e.tick)),
            "at most one Stay per pair per tick"
        );
        assert!(stays.iter().all(|e| e.tick > enters[0].tick));
    }

    #[test]
    fn events_made_in_a_tick_are_heard_in_the_next() {
        let mut p = project();
        floor(&mut p);
        let ball_id = add(&mut p, "Ball", [0.0, 0.52, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let (mut app, _) = contact_world(&p, 1.0 / 64.0);
        for _ in 0..200 {
            app.update();
            let mut engine = app.world_mut().non_send_mut::<Engine>();
            let now = engine.contact_ticks;
            let heard = engine.contacts.deliver(now);
            assert!(
                heard.iter().all(|event| event.tick < now),
                "tick {now} must not hear itself: {heard:?}"
            );
        }
    }

    #[test]
    fn the_events_do_not_depend_on_the_render_rate() {
        let scenario = |frame: f32| {
            let mut p = project();
            let zone = add(&mut p, "Zone", [0.0, 0.0, 0.0]);
            let mut sensor = ColliderSpec::new(ColliderShape::Box {
                size: [4.0, 4.0, 4.0],
            });
            sensor.trigger = true;
            let library = p.physics.materials.clone();
            p.active_scene_mut()
                .add_collider(&zone, sensor, &library)
                .unwrap();
            let probe = add(&mut p, "Probe", [0.0, 6.0, 0.0]);
            body(&mut p, &probe, RigidbodySpec::default(), vec![ball()]);
            let names: HashMap<_, _> = p
                .active_scene()
                .actors
                .iter()
                .map(|a| (a.id.clone(), a.name.clone()))
                .collect();
            let (mut app, _) = contact_world(&p, frame);
            let events = contact_events(&mut app, 240);
            events
                .iter()
                .filter(|e| e.phase != ContactPhase::Stay)
                .map(|e| {
                    // Which end the backend lists first is its own business.
                    let mut ends = [names[&e.a.actor].clone(), names[&e.b.actor].clone()];
                    ends.sort();
                    let [first, second] = ends;
                    (e.phase, e.kind, first, second, e.reason)
                })
                .collect::<Vec<_>>()
        };
        let steady = scenario(1.0 / 64.0);
        let slow = scenario(1.0 / 16.0);
        assert_eq!(steady.len(), 2, "{steady:?}");
        assert_eq!(steady, slow, "same events whether a frame ran 1 or 4 ticks");
    }

    #[test]
    fn a_trigger_enters_and_exits_with_no_contact_force() {
        let mut p = project();
        let zone = add(&mut p, "Zone", [0.0, 0.0, 0.0]);
        let mut sensor = ColliderSpec::new(ColliderShape::Box {
            size: [4.0, 4.0, 4.0],
        });
        sensor.trigger = true;
        let library = p.physics.materials.clone();
        p.active_scene_mut()
            .add_collider(&zone, sensor, &library)
            .unwrap();
        let probe = add(&mut p, "Probe", [0.0, 6.0, 0.0]);
        body(&mut p, &probe, RigidbodySpec::default(), vec![ball()]);
        let (mut app, _) = contact_world(&p, 1.0 / 64.0);
        let events = contact_events(&mut app, 200);
        let enters = lifecycle(&events, ContactPhase::Enter);
        let exits = lifecycle(&events, ContactPhase::Exit);
        assert_eq!((enters.len(), exits.len()), (1, 1), "{events:#?}");
        assert_eq!(enters[0].kind, ContactKind::Trigger);
        assert!(enters[0].payload.is_none(), "a trigger fabricates no force");
        assert_eq!(exits[0].reason, Some(ExitReason::Separated));
        assert!(enters[0].tick < exits[0].tick);
    }

    #[test]
    fn a_compound_floor_is_one_actor_level_enter() {
        let mut p = project();
        let id = add(&mut p, "Floor", [0.0, -0.5, 0.0]);
        for x in [-10.0, 10.0] {
            let mut part = ColliderSpec::new(ColliderShape::Box {
                size: [20.0, 1.0, 20.0],
            });
            part.center = [x, 0.0, 0.0];
            let library = p.physics.materials.clone();
            p.active_scene_mut()
                .add_collider(&id, part, &library)
                .unwrap();
        }
        let ball_id = add(&mut p, "Ball", [0.0, 0.52, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let (mut app, ids) = contact_world(&p, 1.0 / 64.0);
        let events = contact_events(&mut app, 30);
        let enters = lifecycle(&events, ContactPhase::Enter);
        assert_eq!(enters.len(), 2, "one per collider pair: {enters:#?}");
        assert_eq!(enters.iter().filter(|e| e.edge).count(), 1);
        let engine = app.world().non_send::<Engine>();
        assert_eq!(engine.contacts.pairs_between(&ball_id, &id), 2);
        assert!(engine.touching[&ball_id].contains(&id));
        assert!(engine.touching[&id].contains(&ball_id));
        let _ = ids;
    }

    #[test]
    fn deleting_an_actor_makes_a_synthetic_exit_with_a_reason() {
        let mut p = project();
        let floor_id = floor(&mut p);
        let ball_id = add(&mut p, "Ball", [0.0, 0.52, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let (mut app, ids) = contact_world(&p, 1.0 / 64.0);
        let _ = contact_events(&mut app, 20);
        {
            let mut engine = app.world_mut().non_send_mut::<Engine>();
            assert!(engine.touching[&ball_id].contains(&floor_id));
            engine.contacts.remove_actor(&floor_id);
            engine.entities.remove(&floor_id);
        }
        app.world_mut().despawn(ids[&floor_id]);
        let events = contact_events(&mut app, 24);
        let exits = lifecycle(&events, ContactPhase::Exit);
        assert_eq!(exits.len(), 1, "{events:#?}");
        assert_eq!(exits[0].reason, Some(ExitReason::ActorRemoved));
        let engine = app.world().non_send::<Engine>();
        assert!(!engine.touching.contains_key(&ball_id));
    }

    #[test]
    fn switching_a_collider_off_ends_its_pair_with_that_reason() {
        let mut p = project();
        let floor_id = floor(&mut p);
        let ball_id = add(&mut p, "Ball", [0.0, 0.52, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let (mut app, _) = contact_world(&p, 1.0 / 64.0);
        let _ = contact_events(&mut app, 20);
        let collider = {
            let world = app.world_mut();
            let mut query = world.query::<(Entity, &PlannedCollider)>();
            query
                .iter(world)
                .find(|(_, planned)| planned.actor == floor_id)
                .map(|(entity, _)| entity)
                .expect("the floor's collider")
        };
        app.world_mut()
            .entity_mut(collider)
            .insert(rp::ColliderDisabled);
        let events = contact_events(&mut app, 40);
        let exits = lifecycle(&events, ContactPhase::Exit);
        assert_eq!(exits.len(), 1, "{events:#?}");
        assert_eq!(exits[0].reason, Some(ExitReason::ColliderDisabled));
    }
}

#[cfg(test)]
mod tests_2d {
    use super::*;
    use bevy_rapier2d::prelude as rp;
    use blockloom_core::physics::{ColliderShape, ColliderSpec, MassSource, RigidbodySpec};
    use blockloom_core::project::Actor;
    use blockloom_core::scene::Visual;
    use std::time::Duration;

    pub(super) fn project() -> Project {
        let mut project = Project::starter("Flat", Mode::TwoD);
        project.active_scene_mut().actors.clear();
        project
    }

    pub(super) fn add(project: &mut Project, name: &str, at: [f32; 2]) -> String {
        let id = project.add_actor(Actor::new(
            name,
            Visual::Rect {
                color: "#ffffff".into(),
                size: [1.0, 1.0],
            },
        ));
        let scene = project.active_scene_mut();
        let actor = scene.actors.iter_mut().find(|a| a.id == id).unwrap();
        actor.components.remove("Look");
        actor.components.placement_mut().position = [at[0], at[1], 0.0];
        id
    }

    pub(super) fn body(
        project: &mut Project,
        id: &str,
        spec: RigidbodySpec,
        shapes: Vec<ColliderSpec>,
    ) {
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene.set_rigidbody(id, spec, &library).unwrap();
        for shape in shapes {
            scene.add_collider(id, shape, &library).unwrap();
        }
    }

    pub(super) fn world(project: &Project) -> (App, HashMap<String, Entity>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(rp::RapierPhysicsPlugin::<crate::dim2::OneWayHooks>::default());
        app.add_systems(Update, d2::refresh_masses);
        let mut ids = HashMap::new();
        for actor in &project.actors {
            let entity = app
                .world_mut()
                .spawn(crate::world::transform_for(actor))
                .id();
            ids.insert(actor.id.clone(), entity);
        }
        let mut commands = app.world_mut().commands();
        install(&mut commands, project, &ids);
        app.world_mut().flush();
        (app, ids)
    }

    pub(super) fn run(app: &mut App, steps: usize) {
        for _ in 0..steps {
            app.update();
        }
    }

    pub(super) fn y(app: &App, entity: Entity) -> f32 {
        app.world().get::<Transform>(entity).unwrap().translation.y
    }

    pub(super) fn circle() -> ColliderSpec {
        ColliderSpec::new(ColliderShape::Circle { radius: 0.5 })
    }

    pub(super) fn ground(project: &mut Project) {
        let id = add(project, "Ground", [0.0, -0.5]);
        let library = project.physics.materials.clone();
        project
            .active_scene_mut()
            .add_collider(
                &id,
                ColliderSpec::new(ColliderShape::Rect { size: [40.0, 1.0] }),
                &library,
            )
            .unwrap();
    }

    #[test]
    fn a_2d_landing_is_one_enter_with_a_payload_and_an_ordered_stay() {
        use blockloom_core::physics::ContactPhase;
        let mut p = project();
        ground(&mut p);
        let ball = add(&mut p, "Ball", [0.0, 0.6]);
        body(&mut p, &ball, RigidbodySpec::default(), vec![circle()]);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 64.0),
        ));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(
            rp::RapierPhysicsPlugin::<crate::dim2::OneWayHooks>::pixels_per_meter(
                crate::dim2::PIXELS_PER_METER,
            )
            .in_fixed_schedule(),
        );
        app.add_systems(FixedUpdate, d2::refresh_masses);
        app.add_systems(
            FixedPostUpdate,
            crate::dim2::track_contacts.after(rp::PhysicsSet::Writeback),
        );
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::TwoD);
        engine.running = true;
        let mut ids = HashMap::new();
        for actor in &p.actors {
            let entity = app
                .world_mut()
                .spawn((
                    crate::world::transform_for(actor),
                    crate::engine::ActorId(actor.id.clone()),
                ))
                .id();
            ids.insert(actor.id.clone(), entity);
            engine.entities.insert(actor.id.clone(), entity);
        }
        app.insert_non_send(engine);
        let mut commands = app.world_mut().commands();
        install(&mut commands, &p, &ids);
        app.world_mut().flush();
        for _ in 0..400 {
            if app.world().non_send::<Engine>().contact_ticks >= 40 {
                break;
            }
            app.update();
        }
        let events = app
            .world_mut()
            .non_send_mut::<Engine>()
            .contacts
            .deliver(u64::MAX);
        let enters: Vec<_> = events
            .iter()
            .filter(|e| e.phase == ContactPhase::Enter)
            .collect();
        assert_eq!(enters.len(), 1, "{events:#?}");
        let payload = enters[0].payload.as_ref().expect("payload");
        assert!(payload.normal[1].abs() > 0.9, "{payload:?}");
        let ticks: Vec<_> = events.iter().map(|e| e.tick).collect();
        assert!(ticks.windows(2).all(|w| w[0] <= w[1]), "ordered by tick");
    }

    #[test]
    fn a_circle_lands_on_invisible_ground_and_the_mass_is_exact() {
        let mut p = project();
        ground(&mut p);
        let ball = add(&mut p, "Ball", [0.0, 4.0]);
        body(
            &mut p,
            &ball,
            RigidbodySpec {
                mass: MassSource::Explicit { mass: 5.0 },
                ..Default::default()
            },
            vec![circle(), {
                let mut second = circle();
                second.center = [0.0, 1.0, 0.0];
                second
            }],
        );
        let (mut app, ids) = world(&p);
        run(&mut app, 3);
        let mass = app
            .world()
            .get::<rp::ReadMassProperties>(ids[&ball])
            .unwrap();
        assert!((mass.get().mass - 5.0).abs() < 1e-3, "{}", mass.get().mass);
        run(&mut app, 300);
        // Two stacked circles rest on the lower one.
        assert!(
            (y(&app, ids[&ball]) - 0.5).abs() < 0.06,
            "{}",
            y(&app, ids[&ball])
        );
    }

    #[test]
    fn a_planned_one_way_platform_catches_falls_and_lets_a_rise_through() {
        let mut p = project();
        let deck = add(&mut p, "Deck", [0.0, 0.0]);
        let mut shape = ColliderSpec::new(ColliderShape::Rect { size: [10.0, 0.2] });
        shape.one_way = true;
        let library = p.physics.materials.clone();
        p.active_scene_mut()
            .add_collider(&deck, shape, &library)
            .unwrap();
        let falling = add(&mut p, "Falling", [-2.0, 2.0]);
        body(&mut p, &falling, RigidbodySpec::default(), vec![circle()]);
        let rising = add(&mut p, "Rising", [2.0, -2.0]);
        body(
            &mut p,
            &rising,
            RigidbodySpec {
                use_gravity: false,
                ..Default::default()
            },
            vec![circle()],
        );
        let (mut app, ids) = world(&p);
        app.world_mut()
            .get_mut::<rp::Velocity>(ids[&rising])
            .unwrap()
            .linear = Vec2::new(0.0, 5.0);
        run(&mut app, 90);
        assert!(y(&app, ids[&falling]) > 0.4, "{}", y(&app, ids[&falling]));
        assert!(y(&app, ids[&rising]) > 1.0, "{}", y(&app, ids[&rising]));
    }

    fn push(app: &mut App, ids: &HashMap<String, Entity>, effect: Effect) {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::TwoD);
        engine.running = true;
        engine.entities = ids.clone();
        app.insert_non_send(engine);
        app.insert_resource(PendingEffects(vec![effect]));
        let mut system = IntoSystem::into_system(d2::apply_forces);
        system.initialize(app.world_mut());
        system.run((), app.world_mut()).unwrap();
    }

    /// A 2D world at the runtime's pixel scale, which forces are authored in.
    fn scaled_world(project: &Project) -> (App, HashMap<String, Entity>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(
            rp::RapierPhysicsPlugin::<crate::dim2::OneWayHooks>::pixels_per_meter(
                crate::dim2::PIXELS_PER_METER,
            ),
        );
        app.add_systems(Update, d2::refresh_masses);
        let mut ids = HashMap::new();
        for actor in &project.actors {
            let entity = app
                .world_mut()
                .spawn(crate::world::transform_for(actor))
                .id();
            ids.insert(actor.id.clone(), entity);
        }
        let mut commands = app.world_mut().commands();
        install(&mut commands, project, &ids);
        app.world_mut().flush();
        (app, ids)
    }

    #[test]
    fn a_2d_force_speaks_world_units_and_modes_hold() {
        use blockloom_core::physics::ForceMode;
        let cases = [
            // mode, torque, vector, mass, expected velocity (x px/s or spin rad/s)
            (
                ForceMode::VelocityChange,
                false,
                [300.0, 0.0, 0.0],
                5.0,
                300.0,
            ),
            (
                ForceMode::Acceleration,
                false,
                [6000.0, 0.0, 0.0],
                2.0,
                100.0,
            ),
            (ForceMode::Impulse, false, [200.0, 0.0, 0.0], 2.0, 100.0),
            (ForceMode::Force, false, [12000.0, 0.0, 0.0], 2.0, 100.0),
        ];
        for (mode, torque, vector, mass, expect) in cases {
            let mut p = project();
            let id = add(&mut p, "Drifter", [0.0, 0.0]);
            body(
                &mut p,
                &id,
                RigidbodySpec {
                    use_gravity: false,
                    mass: MassSource::Explicit { mass },
                    ..Default::default()
                },
                vec![circle()],
            );
            let (mut app, ids) = scaled_world(&p);
            run(&mut app, 3);
            push(
                &mut app,
                &ids,
                Effect::AddForce {
                    actor: id.clone(),
                    mode,
                    torque,
                    vector,
                },
            );
            run(&mut app, 1);
            let got = app.world().get::<rp::Velocity>(ids[&id]).unwrap().linear.x;
            assert!(
                (got - expect).abs() < expect * 0.03,
                "{mode:?}: {got} vs {expect}"
            );
        }
    }

    #[test]
    fn a_2d_torque_turns_by_inertia() {
        let mut p = project();
        let id = add(&mut p, "Spinner", [0.0, 0.0]);
        body(
            &mut p,
            &id,
            RigidbodySpec {
                use_gravity: false,
                mass: MassSource::Explicit { mass: 4.0 },
                ..Default::default()
            },
            vec![circle()],
        );
        let (mut app, ids) = scaled_world(&p);
        run(&mut app, 3);
        push(
            &mut app,
            &ids,
            Effect::AddForce {
                actor: id.clone(),
                mode: blockloom_core::physics::ForceMode::VelocityChange,
                torque: true,
                vector: [0.0, 0.0, 3.0],
            },
        );
        run(&mut app, 1);
        let spin = app.world().get::<rp::Velocity>(ids[&id]).unwrap().angular;
        assert!((spin - 3.0).abs() < 0.1, "{spin}");
    }

    #[test]
    fn a_lower_depenetration_cap_slows_the_push_out_in_2d() {
        let peak = |cap: f32| {
            let mut p = project();
            let floor = add(&mut p, "Floor", [0.0, -0.5]);
            let library = p.physics.materials.clone();
            p.active_scene_mut()
                .add_collider(
                    &floor,
                    ColliderSpec::new(ColliderShape::Rect { size: [40.0, 1.0] }),
                    &library,
                )
                .unwrap();
            let id = add(&mut p, "Sunk", [0.0, -0.1]);
            body(
                &mut p,
                &id,
                RigidbodySpec {
                    use_gravity: false,
                    max_depenetration_velocity: cap,
                    ..Default::default()
                },
                vec![circle()],
            );
            let (mut app, ids) = scaled_world(&p);
            let mut peak = 0.0f32;
            let mut last = y(&app, ids[&id]);
            for _ in 0..30 {
                app.update();
                let now = y(&app, ids[&id]);
                peak = peak.max((now - last) * 60.0);
                last = now;
            }
            peak
        };
        let slow = peak(0.01);
        let fast = peak(10.0);
        assert!(fast > slow * 1.5, "uncapped {fast} vs capped {slow}");
    }

    #[test]
    fn layers_filter_2d_pairs() {
        let mut p = project();
        let floor = add(&mut p, "Floor", [0.0, -0.5]);
        let library = p.physics.materials.clone();
        let mut slab = ColliderSpec::new(ColliderShape::Rect { size: [40.0, 1.0] });
        slab.layer = 3;
        p.active_scene_mut()
            .add_collider(&floor, slab, &library)
            .unwrap();
        let ghost = add(&mut p, "Ghost", [0.0, 3.0]);
        let mut shape = circle();
        shape.layer = 4;
        body(&mut p, &ghost, RigidbodySpec::default(), vec![shape]);
        p.physics
            .layers
            .set_collides(Mode::TwoD, 3, 4, false)
            .unwrap();
        let (mut app, ids) = world(&p);
        run(&mut app, 180);
        assert!(y(&app, ids[&ghost]) < -2.0);
    }

    #[test]
    fn frozen_rotation_and_a_speed_cap_hold_in_2d() {
        let mut p = project();
        let id = add(&mut p, "Spinner", [0.0, 0.0]);
        let mut constraints = blockloom_core::physics::Constraints::default();
        constraints.freeze_rotation[2] = true;
        body(
            &mut p,
            &id,
            RigidbodySpec {
                use_gravity: false,
                constraints,
                max_linear_velocity: 2.0,
                ..Default::default()
            },
            vec![circle()],
        );
        let (mut app, ids) = world(&p);
        {
            let mut velocity = app.world_mut().get_mut::<rp::Velocity>(ids[&id]).unwrap();
            velocity.angular = 10.0;
            velocity.linear = Vec2::new(900.0, 0.0);
        }
        let mut system = IntoSystem::into_system(d2::clamp_velocities);
        system.initialize(app.world_mut());
        system.run((), app.world_mut()).unwrap();
        run(&mut app, 30);
        let rotation = app.world().get::<Transform>(ids[&id]).unwrap().rotation;
        assert!(
            rotation.angle_between(Quat::IDENTITY) < 1e-3,
            "{rotation:?}"
        );
        let speed = app
            .world()
            .get::<rp::Velocity>(ids[&id])
            .unwrap()
            .linear
            .length();
        assert!(speed <= 200.0 + 1e-2, "{speed}");
    }
}

#[cfg(test)]
mod query_tests {
    use super::tests::*;
    use super::*;
    use crate::queries::d3::World3;
    use bevy::ecs::system::RunSystemOnce;
    use blockloom_core::physics::query::{
        QueryFilter, QueryOutcome, QueryRequest, QueryService, QueryShape, TriggerPolicy,
    };
    use blockloom_core::physics::{ColliderShape, ColliderSpec, RigidbodySpec};

    fn ask(
        app: &mut App,
        request: QueryRequest,
        filter: QueryFilter,
        limit: usize,
    ) -> QueryOutcome {
        app.world_mut()
            .run_system_once(move |world: World3| world.service().run(&request, &filter, limit))
            .unwrap()
    }

    fn down(x: f32, from: f32, to: f32, all: bool) -> QueryRequest {
        QueryRequest::Ray {
            from: [x, from, 0.0],
            to: [x, to, 0.0],
            all,
        }
    }

    fn slab(project: &mut Project, name: &str, at: [f32; 3], size: [f32; 3]) -> String {
        let id = add(project, name, at);
        scenery(project, &id, ColliderShape::Box { size });
        id
    }

    fn settle(app: &mut App) {
        run(app, 3);
    }

    #[test]
    fn a_ray_hits_invisible_scenery_with_its_full_identity() {
        let mut p = project();
        let floor_id = floor(&mut p);
        let (mut app, _) = world(&p);
        settle(&mut app);
        let found = ask(
            &mut app,
            down(2.0, 5.0, -5.0, false),
            QueryFilter::default(),
            8,
        );
        assert!(found.error.is_none(), "{:?}", found.error);
        let hit = &found.hits[0];
        assert_eq!(hit.actor, floor_id);
        assert_eq!(hit.body, None);
        assert!((hit.distance - 5.0).abs() < 1e-3, "{}", hit.distance);
        assert!((hit.fraction - 0.5).abs() < 1e-3);
        assert!((hit.point[1]).abs() < 1e-3);
        assert!((hit.normal[1] - 1.0).abs() < 1e-3, "{:?}", hit.normal);
        assert!(!hit.started_inside && !hit.trigger);
        assert!(!hit.collider.is_empty());
    }

    #[test]
    fn all_hits_come_back_nearest_first_and_a_small_buffer_reports_overflow() {
        let mut p = project();
        slab(&mut p, "Low", [0.0, 0.0, 0.0], [4.0, 1.0, 4.0]);
        slab(&mut p, "Mid", [0.0, 3.0, 0.0], [4.0, 1.0, 4.0]);
        slab(&mut p, "High", [0.0, 6.0, 0.0], [4.0, 1.0, 4.0]);
        let (mut app, _) = world(&p);
        settle(&mut app);
        let all = ask(
            &mut app,
            down(0.0, 10.0, -10.0, true),
            QueryFilter::default(),
            8,
        );
        // Each slab is crossed on entry and the ray ends outside all of them.
        let order: Vec<f32> = all.hits.iter().map(|h| h.distance).collect();
        assert_eq!(order.len(), 3, "{order:?}");
        assert!(order.windows(2).all(|w| w[0] <= w[1]));
        assert!(!all.overflow);
        let small = ask(
            &mut app,
            down(0.0, 10.0, -10.0, true),
            QueryFilter::default(),
            2,
        );
        assert_eq!(small.hits.len(), 2);
        assert!(small.overflow);
        let first = ask(
            &mut app,
            down(0.0, 10.0, -10.0, false),
            QueryFilter::default(),
            8,
        );
        assert_eq!(first.hits.len(), 1);
        assert!((first.hits[0].distance - all.hits[0].distance).abs() < 1e-4);
    }

    #[test]
    fn a_ray_that_starts_inside_reports_it_at_distance_zero() {
        let mut p = project();
        slab(&mut p, "Block", [0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
        let (mut app, _) = world(&p);
        settle(&mut app);
        let found = ask(
            &mut app,
            down(0.0, 0.5, -10.0, false),
            QueryFilter::default(),
            8,
        );
        let hit = &found.hits[0];
        assert!(hit.started_inside);
        assert_eq!(hit.distance, 0.0);
        assert!(
            hit.normal[1] > 0.99,
            "opposes the direction: {:?}",
            hit.normal
        );
    }

    #[test]
    fn a_zero_length_ray_finds_what_contains_the_point() {
        let mut p = project();
        let block = slab(&mut p, "Block", [0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
        let (mut app, _) = world(&p);
        settle(&mut app);
        let inside = ask(
            &mut app,
            down(0.0, 1.0, 1.0, true),
            QueryFilter::default(),
            8,
        );
        assert_eq!(inside.hits.len(), 1);
        assert_eq!(inside.hits[0].actor, block);
        let outside = ask(
            &mut app,
            down(0.0, 9.0, 9.0, true),
            QueryFilter::default(),
            8,
        );
        assert!(outside.hits.is_empty());
    }

    #[test]
    fn triggers_follow_the_policy() {
        let mut p = project();
        let zone = add(&mut p, "Zone", [0.0, 0.0, 0.0]);
        let library = p.physics.materials.clone();
        let mut trigger = ColliderSpec::new(ColliderShape::Box {
            size: [2.0, 2.0, 2.0],
        });
        trigger.trigger = true;
        p.active_scene_mut()
            .add_collider(&zone, trigger, &library)
            .unwrap();
        let (mut app, _) = world(&p);
        settle(&mut app);
        for (policy, expected) in [
            (TriggerPolicy::UseGlobal, 1),
            (TriggerPolicy::Include, 1),
            (TriggerPolicy::Ignore, 0),
        ] {
            let filter = QueryFilter {
                triggers: policy,
                ..QueryFilter::default()
            };
            let found = ask(&mut app, down(0.0, 5.0, -5.0, false), filter, 8);
            assert_eq!(found.hits.len(), expected, "{policy:?}");
            if expected == 1 {
                assert!(found.hits[0].trigger);
            }
        }
    }

    #[test]
    fn the_asker_skips_itself_and_its_body_and_follows_the_layer_matrix() {
        let mut p = project();
        let floor_id = add(&mut p, "Floor", [0.0, -0.5, 0.0]);
        let library = p.physics.materials.clone();
        let mut slab = ColliderSpec::new(ColliderShape::Box {
            size: [40.0, 1.0, 40.0],
        });
        slab.layer = 3;
        p.active_scene_mut()
            .add_collider(&floor_id, slab, &library)
            .unwrap();
        let walker = add(&mut p, "Walker", [0.0, 2.0, 0.0]);
        let mut feet = ball();
        feet.layer = 4;
        body(
            &mut p,
            &walker,
            RigidbodySpec {
                use_gravity: false,
                ..Default::default()
            },
            vec![feet],
        );
        let (mut app, _) = world(&p);
        settle(&mut app);
        // From inside the walker, straight down: only the floor, never itself.
        let ask_as = |app: &mut App, asker: &str| {
            ask(
                app,
                down(0.0, 2.0, -5.0, true),
                QueryFilter::as_actor(asker),
                8,
            )
        };
        let found = ask_as(&mut app, &walker);
        assert_eq!(found.hits.len(), 1, "{found:?}");
        assert_eq!(found.hits[0].actor, floor_id);
        // Nobody asking sees the walker too.
        let anyone = ask(
            &mut app,
            down(0.0, 2.0, -5.0, true),
            QueryFilter::default(),
            8,
        );
        assert_eq!(anyone.hits.len(), 2);
        // With layers 3 and 4 not colliding, the walker's own query misses the floor.
        let mut p2 = p.clone();
        p2.physics
            .layers
            .set_collides(Mode::ThreeD, 3, 4, false)
            .unwrap();
        let (mut app2, _) = world(&p2);
        settle(&mut app2);
        assert!(ask_as(&mut app2, &walker).hits.is_empty());
        // A layer mask narrows any query.
        let only_four = QueryFilter {
            layers: 1 << 3,
            ..QueryFilter::default()
        };
        let masked = ask(&mut app, down(0.0, 2.0, -5.0, true), only_four, 8);
        assert_eq!(masked.hits.len(), 1);
        assert_eq!(masked.hits[0].actor, walker);
    }

    #[test]
    fn a_collider_that_is_not_queryable_or_is_disabled_is_not_found() {
        let mut p = project();
        let hidden = add(&mut p, "Hidden", [0.0, 0.0, 0.0]);
        let library = p.physics.materials.clone();
        let mut spec = ColliderSpec::new(ColliderShape::Box {
            size: [2.0, 2.0, 2.0],
        });
        spec.queryable = false;
        p.active_scene_mut()
            .add_collider(&hidden, spec, &library)
            .unwrap();
        let off = add(&mut p, "Off", [10.0, 0.0, 0.0]);
        let mut spec = ColliderSpec::new(ColliderShape::Box {
            size: [2.0, 2.0, 2.0],
        });
        spec.enabled = false;
        p.active_scene_mut()
            .add_collider(&off, spec, &library)
            .unwrap();
        let (mut app, _) = world(&p);
        settle(&mut app);
        assert!(
            ask(
                &mut app,
                down(0.0, 5.0, -5.0, true),
                QueryFilter::default(),
                8
            )
            .hits
            .is_empty()
        );
        assert!(
            ask(
                &mut app,
                down(10.0, 5.0, -5.0, true),
                QueryFilter::default(),
                8
            )
            .hits
            .is_empty()
        );
    }

    #[test]
    fn a_ball_cast_stops_at_the_surface_and_a_ball_overlap_lists_what_it_touches() {
        let mut p = project();
        let floor_id = floor(&mut p);
        let (mut app, _) = world(&p);
        settle(&mut app);
        let cast = QueryRequest::Cast {
            shape: QueryShape::Ball { radius: 0.5 },
            from: [0.0, 5.0, 0.0],
            to: [0.0, -5.0, 0.0],
        };
        let found = ask(&mut app, cast, QueryFilter::default(), 8);
        let hit = &found.hits[0];
        assert_eq!(hit.actor, floor_id);
        // The ball's centre travels 4.5 before it touches the floor.
        assert!((hit.distance - 4.5).abs() < 1e-2, "{}", hit.distance);
        assert!(hit.point[1].abs() < 1e-2);
        assert!(hit.normal[1] > 0.99);
        let overlap = |y: f32| QueryRequest::Overlap {
            shape: QueryShape::Ball { radius: 0.5 },
            at: [0.0, y, 0.0],
        };
        assert_eq!(
            ask(&mut app, overlap(0.25), QueryFilter::default(), 8)
                .hits
                .len(),
            1
        );
        assert!(
            ask(&mut app, overlap(2.0), QueryFilter::default(), 8)
                .hits
                .is_empty()
        );
        let boxed = QueryRequest::Overlap {
            shape: QueryShape::Box {
                half: [0.5; 3],
                rotation: [0.0, 0.0, 0.0, 1.0],
            },
            at: [0.0, 0.25, 0.0],
        };
        assert_eq!(
            ask(&mut app, boxed, QueryFilter::default(), 8).hits.len(),
            1
        );
        let capsule = QueryRequest::Overlap {
            shape: QueryShape::Capsule {
                radius: 0.3,
                half_height: 0.5,
                rotation: [0.0, 0.0, 0.0, 1.0],
            },
            at: [0.0, 0.7, 0.0],
        };
        assert_eq!(
            ask(&mut app, capsule, QueryFilter::default(), 8).hits.len(),
            1
        );
    }

    #[test]
    fn the_closest_collider_is_found_within_the_distance() {
        let mut p = project();
        let floor_id = floor(&mut p);
        let (mut app, _) = world(&p);
        settle(&mut app);
        let near = ask(
            &mut app,
            QueryRequest::Closest {
                point: [3.0, 2.0, 0.0],
                max_distance: 5.0,
            },
            QueryFilter::default(),
            8,
        );
        let hit = &near.hits[0];
        assert_eq!(hit.actor, floor_id);
        assert!((hit.distance - 2.0).abs() < 1e-3);
        assert!((hit.point[0] - 3.0).abs() < 1e-3 && hit.point[1].abs() < 1e-3);
        let far = ask(
            &mut app,
            QueryRequest::Closest {
                point: [3.0, 20.0, 0.0],
                max_distance: 5.0,
            },
            QueryFilter::default(),
            8,
        );
        assert!(far.hits.is_empty());
    }

    #[test]
    fn a_moving_body_is_found_where_the_last_step_left_it() {
        let mut p = project();
        floor(&mut p);
        let ball_id = add(&mut p, "Ball", [0.0, 3.0, 0.0]);
        body(&mut p, &ball_id, RigidbodySpec::default(), vec![ball()]);
        let (mut app, _) = world(&p);
        run(&mut app, 240);
        let found = ask(
            &mut app,
            down(0.0, 5.0, -1.0, false),
            QueryFilter::default(),
            8,
        );
        let hit = &found.hits[0];
        assert_eq!(hit.actor, ball_id);
        assert_eq!(hit.body.as_deref(), Some(ball_id.as_str()));
        // The ball rests with its centre 0.5 up, so its top is at y = 1.
        assert!((hit.point[1] - 1.0).abs() < 0.06, "{}", hit.point[1]);
    }

    #[test]
    fn ray_hits_match_the_surfaces_of_cooked_meshes() {
        use blockloom_core::physics::cook::{FolderCollision, Source};
        let dir = mesh_folder("query");
        let mut p = project();
        let ground = add(&mut p, "Ground", [0.0, 0.0, 0.0]);
        scenery(
            &mut p,
            &ground,
            ColliderShape::TriangleMesh {
                mesh: "assets/ground.obj".into(),
            },
        );
        let crate_id = add(&mut p, "Crate", [0.0, 3.0, 0.0]);
        body(
            &mut p,
            &crate_id,
            RigidbodySpec::default(),
            vec![ColliderSpec::new(ColliderShape::ConvexHull {
                mesh: "assets/cube.obj".into(),
            })],
        );
        let lookup = FolderCollision::new(&dir, Source::Cook);
        let (mut app, _) = world_with(&p, &lookup);
        run(&mut app, 240);
        let from_above = ask(
            &mut app,
            down(0.0, 5.0, -2.0, true),
            QueryFilter::default(),
            8,
        );
        let _ = std::fs::remove_dir_all(&dir);
        let actors: Vec<&str> = from_above.hits.iter().map(|h| h.actor.as_str()).collect();
        assert_eq!(
            actors,
            [crate_id.as_str(), ground.as_str()],
            "{from_above:?}"
        );
        // The crate's top face is at y = 1 and the ground's triangles at y = 0.
        assert!((from_above.hits[0].point[1] - 1.0).abs() < 0.08);
        assert!(from_above.hits[1].point[1].abs() < 0.02);
    }
}

#[cfg(test)]
mod query_tests_2d {
    use super::tests_2d::*;
    use super::*;
    use crate::queries::d2::World2;
    use bevy::ecs::system::RunSystemOnce;
    use blockloom_core::physics::query::{
        QueryFilter, QueryOutcome, QueryRequest, QueryService, QueryShape, TriggerPolicy,
    };
    use blockloom_core::physics::{ColliderShape, ColliderSpec, RigidbodySpec};

    fn ask(app: &mut App, request: QueryRequest, filter: QueryFilter) -> QueryOutcome {
        app.world_mut()
            .run_system_once(move |world: World2| world.service().run(&request, &filter, 8))
            .unwrap()
    }

    fn down(x: f32, from: f32, to: f32, all: bool) -> QueryRequest {
        QueryRequest::Ray {
            from: [x, from, 0.0],
            to: [x, to, 0.0],
            all,
        }
    }

    #[test]
    fn a_2d_ray_hits_invisible_ground_with_identity_and_ignores_z() {
        let mut p = project();
        ground(&mut p);
        let (mut app, _) = world(&p);
        run(&mut app, 3);
        // A wild z changes nothing: the world is flat.
        let request = QueryRequest::Ray {
            from: [2.0, 5.0, 40.0],
            to: [2.0, -5.0, -40.0],
            all: false,
        };
        let found = ask(&mut app, request, QueryFilter::default());
        assert!(found.error.is_none(), "{:?}", found.error);
        let hit = &found.hits[0];
        assert!((hit.distance - 5.0).abs() < 1e-3, "{}", hit.distance);
        assert!(hit.point[1].abs() < 1e-3 && hit.point[2] == 0.0);
        assert!((hit.normal[1] - 1.0).abs() < 1e-3, "{:?}", hit.normal);
        assert!((hit.fraction - 0.5).abs() < 1e-3);
        assert_eq!(hit.body, None);
    }

    #[test]
    fn a_2d_trigger_asker_and_layers_follow_the_same_rules() {
        let mut p = project();
        ground(&mut p);
        let zone = add(&mut p, "Zone", [0.0, 2.0]);
        let library = p.physics.materials.clone();
        let mut trigger = ColliderSpec::new(ColliderShape::Rect { size: [2.0, 2.0] });
        trigger.trigger = true;
        p.active_scene_mut()
            .add_collider(&zone, trigger, &library)
            .unwrap();
        let walker = add(&mut p, "Walker", [0.0, 6.0]);
        body(
            &mut p,
            &walker,
            RigidbodySpec {
                use_gravity: false,
                ..Default::default()
            },
            vec![circle()],
        );
        let (mut app, _) = world(&p);
        run(&mut app, 3);
        let all = ask(
            &mut app,
            down(0.0, 6.0, -5.0, true),
            QueryFilter::as_actor(&walker),
        );
        let ground_id = p
            .actors
            .iter()
            .find(|a| a.name == "Ground")
            .unwrap()
            .id
            .clone();
        let actors: Vec<_> = all.hits.iter().map(|h| h.actor.as_str()).collect();
        assert_eq!(actors, [zone.as_str(), ground_id.as_str()], "{all:?}");
        assert!(all.hits[0].trigger);
        let solid = ask(
            &mut app,
            down(0.0, 6.0, -5.0, true),
            QueryFilter {
                triggers: TriggerPolicy::Ignore,
                as_actor: Some(walker.clone()),
                ..QueryFilter::default()
            },
        );
        assert_eq!(solid.hits.len(), 1);
        assert!(!solid.hits[0].trigger);
    }

    #[test]
    fn a_2d_cast_overlap_and_closest_points_find_the_ground() {
        let mut p = project();
        ground(&mut p);
        let (mut app, _) = world(&p);
        run(&mut app, 3);
        let cast = QueryRequest::Cast {
            shape: QueryShape::Ball { radius: 0.5 },
            from: [0.0, 5.0, 0.0],
            to: [0.0, -5.0, 0.0],
        };
        let found = ask(&mut app, cast, QueryFilter::default());
        assert!(
            (found.hits[0].distance - 4.5).abs() < 1e-2,
            "{}",
            found.hits[0].distance
        );
        let overlap = |y: f32, shape: QueryShape| QueryRequest::Overlap {
            shape,
            at: [0.0, y, 0.0],
        };
        let ball = QueryShape::Ball { radius: 0.5 };
        assert_eq!(
            ask(
                &mut app,
                overlap(0.25, ball.clone()),
                QueryFilter::default()
            )
            .hits
            .len(),
            1
        );
        assert!(
            ask(&mut app, overlap(2.0, ball), QueryFilter::default())
                .hits
                .is_empty()
        );
        // A box turned a quarter about z still reaches down by its half width.
        let quarter = (std::f32::consts::FRAC_PI_4).sin_cos();
        let turned = QueryShape::Box {
            half: [1.0, 0.1, 0.0],
            rotation: [0.0, 0.0, quarter.0, quarter.1],
        };
        assert_eq!(
            ask(
                &mut app,
                overlap(0.8, turned.clone()),
                QueryFilter::default()
            )
            .hits
            .len(),
            1
        );
        assert!(
            ask(&mut app, overlap(1.4, turned), QueryFilter::default())
                .hits
                .is_empty()
        );
        let near = ask(
            &mut app,
            QueryRequest::Closest {
                point: [3.0, 2.0, 0.0],
                max_distance: 5.0,
            },
            QueryFilter::default(),
        );
        assert!((near.hits[0].distance - 2.0).abs() < 1e-3);
    }
}

#[cfg(test)]
mod controller_tests {
    use super::tests::{add, at, floor, run, scenery};
    use super::*;
    use crate::controller;
    use bevy_rapier3d::prelude as rp;
    use blockloom_core::physics::controller::{
        CharacterControllerSpec, MoveMode, MoveResult, move_call,
    };
    use blockloom_core::physics::{ColliderShape, ColliderSpec};
    use std::time::Duration;

    #[derive(Resource, Default)]
    struct Moves {
        actor: String,
        todo: Vec<(MoveMode, [f32; 3])>,
        done: Vec<MoveResult>,
    }

    fn drive(
        _main: NonSend<crate::engine::Engine>,
        queries: crate::queries::QueryAccess,
        mut moves: ResMut<Moves>,
    ) {
        let todo = std::mem::take(&mut moves.todo);
        let actor = moves.actor.clone();
        let done = queries.scope(0, || {
            todo.into_iter()
                .map(|(mode, vector)| move_call(&actor, mode, vector))
                .collect::<Vec<_>>()
        });
        moves.done.extend(done);
    }

    fn player(
        project: &mut Project,
        at: [f32; 3],
        tweak: impl FnOnce(&mut CharacterControllerSpec),
    ) -> String {
        let id = add(project, "Player", at);
        let mut spec = CharacterControllerSpec::default();
        tweak(&mut spec);
        let library = project.physics.materials.clone();
        project
            .active_scene_mut()
            .set_character_controller(&id, spec, &library)
            .unwrap();
        id
    }

    fn start(project: &Project) -> (App, HashMap<String, Entity>) {
        // The same shape the runtime has: physics in the fixed schedule, the
        // moves and their transform write ahead of it.
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(rp::RapierPhysicsPlugin::<d3::Hooks3>::default().in_fixed_schedule());
        let mut ids = HashMap::new();
        for actor in &project.actors {
            let entity = app
                .world_mut()
                .spawn(crate::world::transform_for(actor))
                .id();
            ids.insert(actor.id.clone(), entity);
        }
        let mut commands = app.world_mut().commands();
        install_with(
            &mut commands,
            project,
            &ids,
            &blockloom_core::physics::cook::NoCollisionData,
        );
        app.world_mut().flush();
        let (_sender, incoming) = std::sync::mpsc::channel();
        app.insert_non_send(crate::engine::Engine::new(incoming, Mode::ThreeD));
        let actor = project
            .actors
            .iter()
            .find(|a| a.name == "Player")
            .map(|a| a.id.clone())
            .unwrap_or_default();
        app.insert_resource(Moves {
            actor,
            ..Default::default()
        });
        app.add_systems(
            FixedUpdate,
            (drive, controller::apply_motion)
                .chain()
                .before(rp::PhysicsSet::SyncBackend),
        );
        // Let the broad phase see the colliders before anything is swept.
        run(&mut app, 4);
        (app, ids)
    }

    fn go(app: &mut App, mode: MoveMode, vector: [f32; 3]) -> MoveResult {
        app.world_mut()
            .resource_mut::<Moves>()
            .todo
            .push((mode, vector));
        run(app, 1);
        app.world_mut()
            .resource_mut::<Moves>()
            .done
            .pop()
            .expect("a move ran")
    }

    fn wall(project: &mut Project, name: &str, centre: [f32; 3], size: [f32; 3]) {
        let id = add(project, name, centre);
        scenery(project, &id, ColliderShape::Box { size });
    }

    #[test]
    fn a_walker_stays_on_the_floor_and_reports_it() {
        let mut p = super::tests::project();
        floor(&mut p);
        let id = player(&mut p, [0.0, 1.0, 0.0], |_| {});
        let (mut app, ids) = start(&p);
        let mut grounded = false;
        for _ in 0..30 {
            let r = go(&mut app, MoveMode::Simple, [2.0, 0.0, 0.0]);
            grounded = r.grounded;
            assert!(r.error.is_none(), "{:?}", r.error);
        }
        let pos = at(&app, ids[&id]);
        assert!(grounded, "standing on the floor");
        assert!(pos.x > 0.5, "walked: {pos:?}");
        assert!((pos.y - 1.0).abs() < 0.2, "kept its height: {pos:?}");
    }

    #[test]
    fn simple_move_falls_and_lands() {
        let mut p = super::tests::project();
        floor(&mut p);
        let id = player(&mut p, [0.0, 4.0, 0.0], |_| {});
        let (mut app, ids) = start(&p);
        let mut first_ground = None;
        for tick in 0..120 {
            let r = go(&mut app, MoveMode::Simple, [0.0; 3]);
            if r.grounded && first_ground.is_none() {
                first_ground = Some(tick);
            }
        }
        assert!(first_ground.is_some(), "it landed");
        let y = at(&app, ids[&id]).y;
        assert!((y - 1.0).abs() < 0.25, "resting height {y}");
    }

    #[test]
    fn a_wall_sets_the_side_flag_and_its_normal_faces_the_controller() {
        let mut p = super::tests::project();
        floor(&mut p);
        wall(&mut p, "Wall", [3.0, 1.0, 0.0], [1.0, 2.0, 6.0]);
        player(&mut p, [0.0, 1.0, 0.0], |_| {});
        let (mut app, _) = start(&p);
        let mut hit = None;
        for _ in 0..90 {
            let r = go(&mut app, MoveMode::Move, [0.2, 0.0, 0.0]);
            if r.flags.sides {
                hit = Some(r);
                break;
            }
        }
        let r = hit.expect("the wall stopped it");
        let wall_hit = r
            .hits
            .iter()
            .find(|h| h.normal[0].abs() > 0.9)
            .expect("a wall hit");
        assert!(
            wall_hit.normal[0] < -0.9,
            "points back at the walker: {wall_hit:?}"
        );
        assert!(
            wall_hit.point[0] > 2.0 && wall_hit.point[0] < 3.1,
            "{wall_hit:?}"
        );
    }

    #[test]
    fn a_ceiling_sets_the_above_flag() {
        let mut p = super::tests::project();
        floor(&mut p);
        wall(&mut p, "Roof", [0.0, 3.5, 0.0], [6.0, 1.0, 6.0]);
        player(&mut p, [0.0, 1.0, 0.0], |_| {});
        let (mut app, _) = start(&p);
        let mut flagged = false;
        for _ in 0..30 {
            let r = go(&mut app, MoveMode::Move, [0.0, 0.2, 0.0]);
            if r.flags.above {
                flagged = true;
                assert!(r.hits.iter().any(|h| h.normal[1] < -0.9), "{:?}", r.hits);
                break;
            }
        }
        assert!(flagged, "the roof was met");
    }

    #[test]
    fn a_low_step_is_walked_onto_and_a_tall_one_blocks() {
        for (height, climbs) in [(0.2_f32, true), (0.8_f32, false)] {
            let mut p = super::tests::project();
            floor(&mut p);
            wall(&mut p, "Step", [3.0, height * 0.5, 0.0], [2.0, height, 6.0]);
            let id = player(&mut p, [0.0, 1.0, 0.0], |_| {});
            let (mut app, ids) = start(&p);
            for _ in 0..90 {
                let _ = go(&mut app, MoveMode::Simple, [3.0, 0.0, 0.0]);
            }
            let pos = at(&app, ids[&id]);
            if climbs {
                assert!(pos.x > 3.5, "stepped up ({height}): {pos:?}");
                assert!(pos.y > 1.0 + height * 0.5, "rose with the step: {pos:?}");
            } else {
                assert!(pos.x < 2.5, "blocked by {height}: {pos:?}");
            }
        }
    }

    #[test]
    fn a_gentle_slope_is_climbed_and_a_steep_one_is_not() {
        for (degrees, climbs) in [(25.0_f32, true), (65.0_f32, false)] {
            let mut p = super::tests::project();
            floor(&mut p);
            let id = add(&mut p, "Ramp", [4.0, 0.0, 0.0]);
            p.active_scene_mut()
                .actors
                .iter_mut()
                .find(|a| a.id == id)
                .unwrap()
                .components
                .placement_mut()
                .rotation = [0.0, 0.0, degrees];
            scenery(
                &mut p,
                &id,
                ColliderShape::Box {
                    size: [10.0, 0.4, 6.0],
                },
            );
            let walker = player(&mut p, [0.0, 1.0, 0.0], |_| {});
            let (mut app, ids) = start(&p);
            for _ in 0..180 {
                let _ = go(&mut app, MoveMode::Simple, [3.0, 0.0, 0.0]);
            }
            let y = at(&app, ids[&walker]).y;
            if climbs {
                assert!(y > 1.8, "climbed a {degrees} degree ramp, y {y}");
            } else {
                assert!(y < 1.6, "refused a {degrees} degree ramp, y {y}");
            }
        }
    }

    #[test]
    fn triggers_are_never_obstacles() {
        let mut p = super::tests::project();
        floor(&mut p);
        let id = add(&mut p, "Zone", [3.0, 1.0, 0.0]);
        let library = p.physics.materials.clone();
        let mut zone = ColliderSpec::new(ColliderShape::Box {
            size: [1.0, 2.0, 6.0],
        });
        zone.trigger = true;
        p.active_scene_mut()
            .add_collider(&id, zone, &library)
            .unwrap();
        let walker = player(&mut p, [0.0, 1.0, 0.0], |_| {});
        let (mut app, ids) = start(&p);
        for _ in 0..60 {
            let r = go(&mut app, MoveMode::Move, [0.1, 0.0, 0.0]);
            assert!(!r.flags.sides, "{:?}", r.hits);
        }
        assert!(at(&app, ids[&walker]).x > 5.0);
    }

    #[test]
    fn detect_collisions_off_removes_the_capsule_from_the_world() {
        let mut p = super::tests::project();
        floor(&mut p);
        player(&mut p, [0.0, 1.0, 0.0], |s| s.detect_collisions = false);
        let (mut app, _) = start(&p);
        let disabled = app
            .world_mut()
            .query_filtered::<(), (
                With<controller::ControllerCapsule>,
                With<rp::ColliderDisabled>,
            )>()
            .iter(app.world())
            .count();
        assert_eq!(disabled, 1);
        // It still sweeps against the floor.
        let r = go(&mut app, MoveMode::Move, [0.0, -2.0, 0.0]);
        assert!(r.flags.below, "{r:?}");
    }

    #[test]
    fn an_obstacle_inside_the_capsule_is_pushed_out_before_the_move() {
        let mut p = super::tests::project();
        floor(&mut p);
        let walker = player(&mut p, [0.0, 0.8, 0.0], |_| {});
        let (mut app, ids) = start(&p);
        let before = at(&app, ids[&walker]).y;
        let r = go(&mut app, MoveMode::Move, [0.0; 3]);
        let after = at(&app, ids[&walker]).y;
        assert!(r.recovered[1] > 0.1, "{r:?}");
        assert!(after > before + 0.1, "{before} -> {after}");
        assert!(r.grounded);
    }

    #[test]
    fn a_controller_that_stood_still_for_seconds_still_moves() {
        let mut p = super::tests::project();
        floor(&mut p);
        let walker = player(&mut p, [0.0, 1.0, 0.0], |_| {});
        let (mut app, ids) = start(&p);
        run(&mut app, 300);
        for _ in 0..10 {
            let _ = go(&mut app, MoveMode::Move, [0.1, 0.0, 0.0]);
        }
        let x = at(&app, ids[&walker]).x;
        assert!((x - 1.0).abs() < 0.05, "x {x}");
    }

    #[test]
    fn a_controller_the_plan_names_is_registered() {
        let mut p = super::tests::project();
        floor(&mut p);
        let id = player(&mut p, [0.0, 1.0, 0.0], |_| {});
        let (_app, _) = start(&p);
        assert!(blockloom_core::physics::controller::has(&id));
    }
}

#[cfg(test)]
mod controller_tests_2d {
    use super::tests_2d::{add, project, run};
    use super::*;
    use crate::controller;
    use bevy_rapier2d::prelude as rp;
    use blockloom_core::physics::controller::{
        CharacterControllerSpec, MoveMode, MoveResult, move_call,
    };
    use blockloom_core::physics::{ColliderShape, ColliderSpec};
    use std::time::Duration;

    #[derive(Resource, Default)]
    struct Moves {
        actor: String,
        todo: Vec<(MoveMode, [f32; 3])>,
        done: Vec<MoveResult>,
    }

    fn drive(
        _main: NonSend<crate::engine::Engine>,
        queries: crate::queries::QueryAccess,
        mut moves: ResMut<Moves>,
    ) {
        let todo = std::mem::take(&mut moves.todo);
        let actor = moves.actor.clone();
        let done = queries.scope(0, || {
            todo.into_iter()
                .map(|(mode, vector)| move_call(&actor, mode, vector))
                .collect::<Vec<_>>()
        });
        moves.done.extend(done);
    }

    fn solid(project: &mut Project, name: &str, at: [f32; 2], size: [f32; 2]) {
        let id = add(project, name, at);
        let library = project.physics.materials.clone();
        project
            .active_scene_mut()
            .add_collider(
                &id,
                ColliderSpec::new(ColliderShape::Rect { size }),
                &library,
            )
            .unwrap();
    }

    fn start(project: &Project) -> (App, String, Entity) {
        start_with(project, false)
    }

    fn start_with(project: &Project, motors: bool) -> (App, String, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(
            rp::RapierPhysicsPlugin::<crate::dim2::OneWayHooks>::pixels_per_meter(
                crate::dim2::PIXELS_PER_METER,
            )
            .in_fixed_schedule(),
        );
        let mut ids = HashMap::new();
        for actor in &project.actors {
            let entity = app
                .world_mut()
                .spawn(crate::world::transform_for(actor))
                .id();
            ids.insert(actor.id.clone(), entity);
        }
        let mut commands = app.world_mut().commands();
        install(&mut commands, project, &ids);
        app.world_mut().flush();
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = crate::engine::Engine::new(incoming, Mode::TwoD);
        engine.running = motors;
        engine.entities = ids.clone();
        app.insert_non_send(engine);
        let actor = project
            .actors
            .iter()
            .find(|a| a.name == "Player")
            .map(|a| a.id.clone())
            .unwrap();
        app.init_resource::<crate::player_camera::BodyFacing>();
        app.insert_resource(Moves {
            actor: actor.clone(),
            ..Default::default()
        });
        if motors {
            app.add_systems(
                FixedUpdate,
                (crate::motor::drive_motors, controller::apply_motion)
                    .chain()
                    .before(rp::PhysicsSet::SyncBackend),
            );
        } else {
            app.add_systems(
                FixedUpdate,
                (drive, controller::apply_motion)
                    .chain()
                    .before(rp::PhysicsSet::SyncBackend),
            );
        }
        run(&mut app, 4);
        let entity = ids[&actor];
        (app, actor, entity)
    }

    fn go(app: &mut App, mode: MoveMode, vector: [f32; 3]) -> MoveResult {
        app.world_mut()
            .resource_mut::<Moves>()
            .todo
            .push((mode, vector));
        run(app, 1);
        app.world_mut()
            .resource_mut::<Moves>()
            .done
            .pop()
            .expect("a move ran")
    }

    fn level() -> (Project, String) {
        let mut p = project();
        solid(&mut p, "Ground", [0.0, -8.0], [4000.0, 16.0]);
        solid(&mut p, "Wall", [300.0, 100.0], [16.0, 400.0]);
        let id = add(&mut p, "Player", [0.0, 32.0]);
        let library = p.physics.materials.clone();
        p.active_scene_mut()
            .set_character_controller(&id, CharacterControllerSpec::for_mode(Mode::TwoD), &library)
            .unwrap();
        (p, id)
    }

    #[test]
    fn a_2d_walker_stays_on_the_ground_until_a_wall_stops_it() {
        let (p, _) = level();
        let (mut app, _, entity) = start(&p);
        let mut side = None;
        for _ in 0..240 {
            let r = go(&mut app, MoveMode::Simple, [240.0, 0.0, 0.0]);
            assert!(r.error.is_none(), "{:?}", r.error);
            if r.flags.sides {
                side = Some(r);
                break;
            }
        }
        let r = side.expect("the wall was met");
        assert!(r.grounded, "still standing: {r:?}");
        let hit = r
            .hits
            .iter()
            .find(|h| h.normal[0].abs() > 0.9)
            .expect("a wall hit");
        assert!(hit.normal[0] < -0.9, "faces the walker: {hit:?}");
        assert!(!hit.actor.is_empty());
        let at = app.world().get::<Transform>(entity).unwrap().translation;
        assert!(at.x > 200.0 && at.x < 300.0, "{at:?}");
        assert!((at.y - 32.0).abs() < 4.0, "{at:?}");
    }

    #[test]
    fn a_2d_simple_move_falls_and_lands() {
        let (mut p, id) = level();
        p.active_scene_mut()
            .actors
            .iter_mut()
            .find(|a| a.id == id)
            .unwrap()
            .components
            .placement_mut()
            .position = [0.0, 400.0, 0.0];
        let (mut app, _, entity) = start(&p);
        let mut landed = false;
        for _ in 0..180 {
            landed |= go(&mut app, MoveMode::Simple, [0.0; 3]).grounded;
        }
        assert!(landed);
        let y = app.world().get::<Transform>(entity).unwrap().translation.y;
        assert!((y - 32.0).abs() < 6.0, "resting height {y}");
    }

    #[test]
    fn a_2d_ceiling_sets_above() {
        let (mut p, _) = level();
        solid(&mut p, "Roof", [0.0, 130.0], [400.0, 20.0]);
        let (mut app, _, _) = start(&p);
        let mut above = false;
        for _ in 0..60 {
            if go(&mut app, MoveMode::Move, [0.0, 4.0, 0.0]).flags.above {
                above = true;
                break;
            }
        }
        assert!(above);
    }

    fn with_motor(mut p: Project, id: &str) -> Project {
        let mut spec = blockloom_core::physics::motor::CharacterMotorSpec::for_mode(Mode::TwoD);
        spec.owner = blockloom_core::physics::motor::MotorOwner::Script;
        let library = p.physics.materials.clone();
        p.active_scene_mut()
            .set_character_motor(id, spec, &library)
            .unwrap();
        p
    }

    fn motor_op(actor: &str, op: &str, vector: [f32; 3]) {
        blockloom_core::physics::motor::run_op(actor, op, vector).unwrap();
    }

    #[test]
    fn a_motor_walks_stops_at_a_wall_and_jumps() {
        let (p, id) = level();
        let p = with_motor(p, &id);
        let (mut app, actor, entity) = start_with(&p, true);
        run(&mut app, 30);
        assert!(blockloom_core::physics::motor::read_number(&actor, "grounded") > 0.5);
        let x0 = app.world().get::<Transform>(entity).unwrap().translation.x;
        motor_op(&actor, "intent", [1.0, 0.0, 0.0]);
        run(&mut app, 60);
        let x1 = app.world().get::<Transform>(entity).unwrap().translation.x;
        assert!(x1 > x0 + 40.0, "walked right: {x0} -> {x1}");
        motor_op(&actor, "intent", [0.0; 3]);
        run(&mut app, 30);
        let x2 = app.world().get::<Transform>(entity).unwrap().translation.x;
        run(&mut app, 30);
        let x3 = app.world().get::<Transform>(entity).unwrap().translation.x;
        assert!((x3 - x2).abs() < 1.0, "braked to a stop: {x2} -> {x3}");
        // A jump leaves the floor and comes back.
        let y0 = app.world().get::<Transform>(entity).unwrap().translation.y;
        motor_op(&actor, "jump", [0.0; 3]);
        run(&mut app, 12);
        let y1 = app.world().get::<Transform>(entity).unwrap().translation.y;
        assert!(y1 > y0 + 10.0, "rose: {y0} -> {y1}");
        motor_op(&actor, "jump release", [0.0; 3]);
        run(&mut app, 120);
        let y2 = app.world().get::<Transform>(entity).unwrap().translation.y;
        assert!((y2 - y0).abs() < 6.0, "landed again: {y0} -> {y2}");
        assert!(blockloom_core::physics::motor::read_number(&actor, "grounded") > 0.5);
    }
}

#[cfg(test)]
mod constraint_tests {
    use super::tests::{add, at, ball, body, run};
    use super::*;
    use bevy_rapier3d::prelude as rp;
    use blockloom_core::physics::RigidbodySpec;
    use blockloom_core::physics::joints::{
        self, ConstraintKind, ConstraintSpec, Limit, Motor, MotorMode,
    };
    use std::time::Duration;

    fn start(project: &Project) -> (App, HashMap<String, Entity>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.init_resource::<PhysicsLayers>();
        app.add_plugins(rp::RapierPhysicsPlugin::<d3::Hooks3>::default().in_fixed_schedule());
        let mut ids = HashMap::new();
        for actor in &project.actors {
            let entity = app
                .world_mut()
                .spawn(crate::world::transform_for(actor))
                .id();
            ids.insert(actor.id.clone(), entity);
        }
        let mut commands = app.world_mut().commands();
        install_with(
            &mut commands,
            project,
            &ids,
            &blockloom_core::physics::cook::NoCollisionData,
        );
        app.world_mut().flush();
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = crate::engine::Engine::new(incoming, Mode::ThreeD);
        engine.entities = ids.clone();
        app.insert_non_send(engine);
        app.add_systems(
            FixedUpdate,
            crate::constraints::d3::drive.after(rp::PhysicsSet::Writeback),
        );
        run(&mut app, 2);
        (app, ids)
    }

    fn weightless() -> RigidbodySpec {
        RigidbodySpec {
            use_gravity: false,
            ..RigidbodySpec::default()
        }
    }

    fn constrain(project: &mut Project, id: &str, spec: ConstraintSpec) {
        let library = project.physics.materials.clone();
        project
            .active_scene_mut()
            .add_constraint(id, spec, &library)
            .unwrap();
    }

    #[test]
    fn a_hinged_pendulum_swings_on_a_fixed_radius() {
        let mut p = super::tests::project();
        let bob = add(&mut p, "Bob", [2.0, 5.0, 0.0]);
        body(&mut p, &bob, RigidbodySpec::default(), vec![ball()]);
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.name = "pivot".into();
        spec.anchor = [-2.0, 0.0, 0.0];
        spec.axis = [0.0, 0.0, 1.0];
        constrain(&mut p, &bob, spec);
        let (mut app, ids) = start(&p);
        // About a quarter swing: a full half swing lands level again.
        run(&mut app, 40);
        let position = at(&app, ids[&bob]);
        assert!(position.y < 4.5, "it swung down: {position}");
        let radius = (position - Vec3::new(0.0, 5.0, 0.0)).length();
        assert!((radius - 2.0).abs() < 0.15, "radius {radius}");
        let status = joints::status(&bob, "pivot").expect("published");
        assert!(status.position.abs() > 10.0, "angle {}", status.position);
        assert!(!status.broken);
    }

    #[test]
    fn a_rope_stops_a_fall_at_its_length() {
        let mut p = super::tests::project();
        let weight = add(&mut p, "Weight", [0.0, 5.0, 0.0]);
        body(&mut p, &weight, RigidbodySpec::default(), vec![ball()]);
        let mut spec = ConstraintSpec::of(ConstraintKind::Distance, Mode::ThreeD);
        spec.max_distance = 2.0;
        spec.anchor = [0.0, 0.0, 0.0];
        constrain(&mut p, &weight, spec);
        // Anchored where the weight starts: the world point is its own position.
        let (mut app, ids) = start(&p);
        run(&mut app, 180);
        let y = at(&app, ids[&weight]).y;
        assert!(y < 5.0 && y > 2.9, "hanging at {y}");
    }

    #[test]
    fn a_clone_made_mid_run_gets_its_body_and_its_own_rope() {
        let mut p = super::tests::project();
        let weight = add(&mut p, "Weight", [0.0, 5.0, 0.0]);
        body(&mut p, &weight, RigidbodySpec::default(), vec![ball()]);
        let mut spec = ConstraintSpec::of(ConstraintKind::Distance, Mode::ThreeD);
        spec.max_distance = 2.0;
        spec.name = "rope".into();
        constrain(&mut p, &weight, spec);
        let template = p
            .active_scene()
            .actors
            .iter()
            .find(|a| a.id == weight)
            .unwrap()
            .clone();
        // The run starts with nothing in it, then a copy turns up.
        let empty = {
            let mut e = p.clone();
            e.active_scene_mut().actors.clear();
            e
        };
        let (mut app, mut ids) = start(&empty);
        let mut copy = template.clone();
        copy.id = "~1".into();
        copy.refresh_physics_ids();
        assert_ne!(
            copy.components.constraints().next().unwrap().id,
            template.components.constraints().next().unwrap().id
        );
        let entity = app
            .world_mut()
            .spawn(crate::world::transform_for(&copy))
            .id();
        ids.insert(copy.id.clone(), entity);
        let mut commands = app.world_mut().commands();
        install_actor(&mut commands, &p, &copy, &[], &ids);
        app.world_mut().flush();
        run(&mut app, 180);
        let y = at(&app, entity).y;
        assert!(y < 5.0 && y > 2.9, "hanging at {y}");
        assert!(
            joints::status("~1", "rope").is_none() || !joints::status("~1", "rope").unwrap().broken
        );
    }

    #[test]
    fn the_physics_playground_plays_without_blowing_up() {
        let project =
            blockloom_core::physics::sample::build("physics-playground", "P", Mode::ThreeD)
                .unwrap();
        let id = |name: &str| {
            project
                .active_scene()
                .actors
                .iter()
                .find(|a| a.name == name)
                .unwrap()
                .id
                .clone()
        };
        let (mut app, ids) = start(&project);
        run(&mut app, 240);
        for (name, entity) in project
            .active_scene()
            .actors
            .iter()
            .map(|a| (a.name.clone(), ids[&a.id]))
        {
            let position = at(&app, entity);
            assert!(
                position.is_finite() && position.length() < 100.0,
                "{name}: {position}"
            );
        }
        // The pendulum hangs within its rope of where it started.
        let weight = at(&app, ids[&id("Pendulum")]);
        assert!(
            (weight - Vec3::new(-3.0, 4.0, 0.0)).length() < 2.7,
            "{weight}"
        );
        // The motor is turning the wheel (the angle itself wraps at a half turn).
        let wheel = joints::status(&id("Wheel"), "motor").expect("published");
        assert!(wheel.speed.abs() > 45.0, "speed {}", wheel.speed);
        // Nothing hit the plank hard enough to snap its weld.
        assert!(!joints::status(&id("Plank"), "weld").unwrap().broken);
        // Crates settled on the ground rather than through it.
        let crate_a = at(&app, ids[&id("Crate A")]);
        assert!(crate_a.y > 0.2, "{crate_a}");
    }

    #[test]
    fn one_actor_carries_several_constraints() {
        let mut p = super::tests::project();
        let slab = add(&mut p, "Slab", [0.0, 3.0, 0.0]);
        body(&mut p, &slab, RigidbodySpec::default(), vec![ball()]);
        let mut left = ConstraintSpec::of(ConstraintKind::Fixed, Mode::ThreeD);
        left.name = "left".into();
        let mut right = ConstraintSpec::of(ConstraintKind::Distance, Mode::ThreeD);
        right.name = "right".into();
        constrain(&mut p, &slab, left);
        constrain(&mut p, &slab, right);
        let (mut app, ids) = start(&p);
        assert_eq!(
            app.world()
                .resource::<crate::constraints::Constraints>()
                .len(),
            2
        );
        run(&mut app, 120);
        let y = at(&app, ids[&slab]).y;
        assert!((y - 3.0).abs() < 0.1, "held at {y}");
    }

    #[test]
    fn a_joint_past_its_break_force_lets_go_and_says_so() {
        let mut p = super::tests::project();
        let load = add(&mut p, "Load", [0.0, 5.0, 0.0]);
        body(&mut p, &load, RigidbodySpec::default(), vec![ball()]);
        let mut spec = ConstraintSpec::of(ConstraintKind::Fixed, Mode::ThreeD);
        spec.name = "bolt".into();
        spec.break_force = Some(2.0);
        spec.break_message = "bolt broke".into();
        constrain(&mut p, &load, spec);
        let (mut app, ids) = start(&p);
        run(&mut app, 120);
        let status = joints::status(&load, "bolt").unwrap();
        assert!(status.broken, "{status:?}");
        assert!(at(&app, ids[&load]).y < 4.0, "it fell once free");
    }

    #[test]
    fn a_velocity_motor_turns_a_hinge_and_a_statement_changes_it() {
        let mut p = super::tests::project();
        let wheel = add(&mut p, "Wheel", [0.0, 5.0, 0.0]);
        body(&mut p, &wheel, weightless(), vec![ball()]);
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.name = "axle".into();
        spec.axis = [0.0, 0.0, 1.0];
        spec.motor = Motor {
            mode: MotorMode::Velocity,
            target: 90.0,
            damping: 50.0,
            ..Motor::default()
        };
        constrain(&mut p, &wheel, spec);
        let (mut app, _ids) = start(&p);
        run(&mut app, 60);
        let turned = joints::status(&wheel, "axle").unwrap().position;
        assert!(turned.abs() > 30.0, "turned {turned}");
        // Reverse it: the statement queues and the next tick applies it.
        joints::run_op(&wheel, "motor speed|axle", -90.0).unwrap();
        run(&mut app, 120);
        let back = joints::status(&wheel, "axle").unwrap();
        assert!(back.speed < 0.0 || back.position < turned, "{back:?}");
    }

    #[test]
    fn a_limited_hinge_stops_at_its_limit() {
        let mut p = super::tests::project();
        let door = add(&mut p, "Door", [1.0, 5.0, 0.0]);
        body(&mut p, &door, RigidbodySpec::default(), vec![ball()]);
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.name = "hinge".into();
        spec.anchor = [-1.0, 0.0, 0.0];
        spec.axis = [0.0, 0.0, 1.0];
        spec.limit = Limit {
            enabled: true,
            min: -30.0,
            max: 30.0,
        };
        constrain(&mut p, &door, spec);
        let (mut app, _ids) = start(&p);
        run(&mut app, 240);
        let angle = joints::status(&door, "hinge").unwrap().position;
        assert!(angle.abs() < 35.0 && angle.abs() > 20.0, "angle {angle}");
    }
}
