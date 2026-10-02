//! Turns a core `PhysicsPlan` into rapier components: one entity per body and one
//! child entity per collider, so a body is made of any number of shapes and a shape
//! can be added, disabled or removed on its own.
//!
//! A legacy `Body` never reaches this module: the plan only holds actors that carry
//! Rigidbody or Collider components, and validation refuses an actor with both.
//! Core decides everything (poses, dimensions, mass, groups); this file only maps
//! those answers onto the backend.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use blockloom_core::physics::material::CombineMode;
use blockloom_core::physics::spec::{BodyType, CollisionDetection};
use blockloom_core::physics::{
    ColliderFilter, ColliderId, ExtraMass, LayerSettings, PhysicsPlan, PlannedMaterial,
    PlannedShape,
};
use blockloom_core::project::Project;
use blockloom_core::scene::Mode;
use std::collections::HashMap;

/// The layer rules the pair hooks consult.
#[derive(Resource, Default, Clone)]
pub struct PhysicsLayers {
    pub settings: LayerSettings,
    pub mode: Option<Mode>,
}

/// A rigid body this module installed.
#[derive(Component, Debug, Clone, Copy)]
pub struct PlannedBody {
    pub max_linear: f32,
    pub max_angular: f32,
    /// Axes the body may not move along or turn about, however it is pushed.
    pub frozen_linear: [bool; 3],
    pub frozen_angular: [bool; 3],
}

/// A collider entity this module installed.
#[derive(Component, Debug, Clone)]
pub struct PlannedCollider {
    /// Read by the Phase 3 queries and the inspector, not yet by the runtime.
    #[allow(dead_code)]
    pub id: ColliderId,
    /// The actor the collider component lives on.
    pub actor: String,
    pub filter: ColliderFilter,
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
pub fn install(commands: &mut Commands, project: &Project, entities: &HashMap<String, Entity>) {
    let mode = project.world.mode;
    let plan = PhysicsPlan::build(&project.actors, mode, &project.physics);
    commands.insert_resource(PhysicsLayers {
        settings: project.physics.layers.clone(),
        mode: Some(mode),
    });
    for issue in &plan.issues {
        if issue.is_error() {
            warn!("physics: {}", issue.message);
        }
    }
    if !plan.is_runnable() || plan.is_empty() {
        return;
    }
    match mode {
        Mode::ThreeD => d3::install(commands, &plan, entities),
        Mode::TwoD => d2::install(commands, &plan, entities),
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
            let PlannedShape::Three(solid) = &planned.shape else {
                continue;
            };
            let collider = match *solid {
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
            if material.static_friction != material.dynamic_friction {
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
                    filter: planned.filter,
                },
                Surface3(material),
            ));
            if planned.trigger {
                e.insert((
                    rp::Sensor,
                    rp::ActiveCollisionTypes::all() - rp::ActiveCollisionTypes::STATIC_STATIC,
                ));
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
            if planned.one_way {
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
                    filter: planned.filter,
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

    fn project() -> Project {
        let mut project = Project::starter("Fixtures", Mode::ThreeD);
        project.active_scene_mut().actors.clear();
        project
    }

    fn add(project: &mut Project, name: &str, at: [f32; 3]) -> String {
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

    fn body(project: &mut Project, id: &str, spec: RigidbodySpec, shapes: Vec<ColliderSpec>) {
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene.set_rigidbody(id, spec, &library).unwrap();
        for shape in shapes {
            scene.add_collider(id, shape, &library).unwrap();
        }
    }

    fn scenery(project: &mut Project, id: &str, shape: ColliderShape) -> ColliderId {
        let library = project.physics.materials.clone();
        project
            .active_scene_mut()
            .add_collider(id, ColliderSpec::new(shape), &library)
            .unwrap()
    }

    fn world(project: &Project) -> (App, HashMap<String, Entity>) {
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
        install(&mut commands, project, &ids);
        app.world_mut().flush();
        (app, ids)
    }

    fn run(app: &mut App, steps: usize) {
        for _ in 0..steps {
            app.update();
        }
    }

    fn at(app: &App, entity: Entity) -> Vec3 {
        app.world().get::<Transform>(entity).unwrap().translation
    }

    fn ball() -> ColliderSpec {
        ColliderSpec::new(ColliderShape::Sphere { radius: 0.5 })
    }

    fn floor(project: &mut Project) -> String {
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
}

#[cfg(test)]
mod tests_2d {
    use super::*;
    use bevy_rapier2d::prelude as rp;
    use blockloom_core::physics::{ColliderShape, ColliderSpec, MassSource, RigidbodySpec};
    use blockloom_core::project::Actor;
    use blockloom_core::scene::Visual;
    use std::time::Duration;

    fn project() -> Project {
        let mut project = Project::starter("Flat", Mode::TwoD);
        project.active_scene_mut().actors.clear();
        project
    }

    fn add(project: &mut Project, name: &str, at: [f32; 2]) -> String {
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

    fn body(project: &mut Project, id: &str, spec: RigidbodySpec, shapes: Vec<ColliderSpec>) {
        let library = project.physics.materials.clone();
        let scene = project.active_scene_mut();
        scene.set_rigidbody(id, spec, &library).unwrap();
        for shape in shapes {
            scene.add_collider(id, shape, &library).unwrap();
        }
    }

    fn world(project: &Project) -> (App, HashMap<String, Entity>) {
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

    fn run(app: &mut App, steps: usize) {
        for _ in 0..steps {
            app.update();
        }
    }

    fn y(app: &App, entity: Entity) -> f32 {
        app.world().get::<Transform>(entity).unwrap().translation.y
    }

    fn circle() -> ColliderSpec {
        ColliderSpec::new(ColliderShape::Circle { radius: 0.5 })
    }

    fn ground(project: &mut Project) {
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
