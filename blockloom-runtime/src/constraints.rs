//! The Rapier half of constraints: one joint entity per planned constraint,
//! a child of the actor that holds it, so an actor can carry several.
//!
//! Core decides the recipe (`blockloom_core::physics::joints::blueprint`); this
//! module maps it onto Rapier's generic joint, runs the commands blocks and
//! scripts queue, snaps joints that carry more than their break threshold and
//! publishes each joint's state for the reporters. A constraint with no target
//! hangs off a hidden fixed body at the world origin.

use std::collections::HashMap;

use bevy::prelude::*;
use blockloom_core::physics::joints::{
    self, ConstraintPlan, ConstraintSpec, JointStatus, JointVerb, MotorMode,
};
use blockloom_core::scene::Mode;

use crate::engine::Engine;

/// Marks a constraint's joint entity.
#[allow(dead_code)]
#[derive(Component, Debug, Clone)]
pub struct ConstraintLink {
    pub actor: String,
    pub handle: String,
}

/// One installed constraint.
struct Installed {
    entity: Entity,
    actor: String,
    target: Option<String>,
    handle: String,
    /// The spec as the run has changed it.
    spec: ConstraintSpec,
    plan: ConstraintPlan,
    broken: bool,
    enabled: bool,
    last_position: f32,
}

/// Every installed constraint of this world.
#[derive(Resource, Default)]
pub struct Constraints {
    items: Vec<Installed>,
}

#[cfg(test)]
impl Constraints {
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Forgets the registry: a world is being rebuilt or a run ended.
pub fn clear() {
    joints::reset();
}

fn quat(rotation: [f32; 4]) -> Quat {
    Quat::from_array(rotation).normalize()
}

/// Applies one tick's commands to a spec. Returns whether the joint must be
/// rebuilt.
fn apply_command(spec: &mut ConstraintSpec, verb: JointVerb, value: f32) -> bool {
    match verb {
        JointVerb::Enable | JointVerb::Disable | JointVerb::Break => false,
        JointVerb::MotorSpeed => {
            spec.motor.mode = MotorMode::Velocity;
            spec.motor.target = value;
            true
        }
        JointVerb::MotorTarget => {
            spec.motor.mode = MotorMode::Position;
            spec.motor.target = value;
            true
        }
        JointVerb::MotorForce => {
            spec.motor.max_force = value.max(0.0);
            true
        }
        JointVerb::MotorOff => {
            spec.motor.mode = MotorMode::Off;
            true
        }
        JointVerb::Stiffness => {
            spec.spring.stiffness = value.max(0.0);
            spec.motor.stiffness = value.max(0.0);
            true
        }
        JointVerb::Damping => {
            spec.spring.damping = value.max(0.0);
            spec.motor.damping = value.max(0.0);
            true
        }
    }
}

macro_rules! dimension {
    () => {
        /// The Rapier joint for `plan`'s recipe.
        pub fn build(plan: &ConstraintPlan, spec: &ConstraintSpec) -> rp::GenericJoint {
            let recipe = joints::blueprint(spec, MODE);
            let mut mask = rp::JointAxesMask::empty();
            for axis in &recipe.locked {
                mask |= rp::JointAxesMask::from(axis_of(*axis));
            }
            let f = &plan.frames;
            let mut joint = rp::GenericJointBuilder::new(mask)
                .contacts_enabled(recipe.contacts)
                // Rapier's first body is the target, the second the holder.
                .local_anchor1(vec_of(f.position_b))
                .local_basis1(rot_of(quat(f.rotation_b)))
                .local_anchor2(vec_of(f.position_a))
                .local_basis2(rot_of(quat(f.rotation_a)))
                .build();
            if recipe.coupled_linear {
                joint.set_coupled_axes(rp::JointAxesMask::LIN_AXES);
            }
            for limit in &recipe.limits {
                let (min, max) = if limit.axis.is_angular() {
                    (limit.min, limit.max)
                } else {
                    (len_of(limit.min), len_of(limit.max))
                };
                joint.set_limits(axis_of(limit.axis), [min, max]);
            }
            for motor in &recipe.motors {
                let axis = axis_of(motor.axis);
                let target = if motor.axis.is_angular() {
                    motor.target
                } else {
                    len_of(motor.target)
                };
                joint.set_motor_model(axis, rp::MotorModel::ForceBased);
                if motor.velocity {
                    joint.set_motor_velocity(axis, target, motor.damping);
                } else {
                    joint.set_motor_position(axis, target, motor.stiffness, motor.damping);
                }
                if let Some(force) = motor.max_force {
                    joint.set_motor_max_force(axis, force);
                }
            }
            joint
        }

        /// Spawns a joint entity for every planned constraint.
        pub fn install(
            commands: &mut Commands,
            planned: &[ConstraintPlan],
            entities: &HashMap<String, Entity>,
        ) {
            let items = spawn_items(commands, planned, entities);
            commands.insert_resource(Constraints { items });
        }

        /// Adds joints to a run in progress: a clone's constraints.
        pub fn install_more(
            commands: &mut Commands,
            planned: &[ConstraintPlan],
            entities: &HashMap<String, Entity>,
        ) {
            let items = spawn_items(commands, planned, entities);
            commands.queue(move |world: &mut World| {
                world
                    .get_resource_or_insert_with(Constraints::default)
                    .items
                    .extend(items);
            });
        }

        fn spawn_items(
            commands: &mut Commands,
            planned: &[ConstraintPlan],
            entities: &HashMap<String, Entity>,
        ) -> Vec<Installed> {
            let mut items = Vec::new();
            let mut world_anchor: Option<Entity> = None;
            for plan in planned {
                let Some(&holder) = entities.get(&plan.actor) else {
                    continue;
                };
                let other = match &plan.target {
                    Some(target) => match entities.get(target) {
                        Some(&e) => e,
                        None => continue,
                    },
                    None => *world_anchor.get_or_insert_with(|| {
                        commands
                            .spawn((
                                Name::new("constraint world anchor"),
                                rp::RigidBody::Fixed,
                                Transform::default(),
                            ))
                            .id()
                    }),
                };
                let joint = build(plan, &plan.spec);
                let entity = commands
                    .spawn((
                        Name::new(format!("constraint {}", plan.handle)),
                        rp::ImpulseJoint::new(other, joint),
                        ConstraintLink {
                            actor: plan.actor.clone(),
                            handle: plan.handle.clone(),
                        },
                        ChildOf(holder),
                    ))
                    .id();
                items.push(Installed {
                    entity,
                    actor: plan.actor.clone(),
                    target: plan.target.clone(),
                    handle: plan.handle.clone(),
                    spec: plan.spec.clone(),
                    plan: plan.clone(),
                    broken: false,
                    enabled: true,
                    last_position: 0.0,
                });
            }
            items
        }

        /// Runs the queued commands, snaps overloaded joints and publishes
        /// every joint's state. After the physics step has written impulses.
        pub fn drive(
            mut commands: Commands,
            mut engine: NonSendMut<Engine>,
            time: Res<Time<Fixed>>,
            installed: Option<ResMut<Constraints>>,
            mut joints_q: Query<(&mut rp::ImpulseJoint, &rp::ImpulseJointImpulses)>,
            poses: Query<&GlobalTransform>,
        ) {
            let Some(mut installed) = installed else {
                return;
            };
            if installed.items.is_empty() {
                return;
            }
            let dt = time.timestep().as_secs_f32().max(1e-6);
            for command in joints::take_commands() {
                let Some(item) = installed
                    .items
                    .iter_mut()
                    .find(|i| i.actor == command.actor && i.handle == command.handle)
                else {
                    continue;
                };
                if item.broken {
                    continue;
                }
                match command.verb {
                    JointVerb::Enable | JointVerb::Disable => {
                        item.enabled = command.verb == JointVerb::Enable;
                        if item.enabled {
                            commands
                                .entity(item.entity)
                                .remove::<rp::ImpulseJointDisabled>();
                        } else {
                            commands
                                .entity(item.entity)
                                .insert(rp::ImpulseJointDisabled);
                        }
                    }
                    JointVerb::Break => snap(&mut commands, &mut engine, item),
                    verb => {
                        if apply_command(&mut item.spec, verb, command.value)
                            && let Ok((mut joint, _)) = joints_q.get_mut(item.entity)
                        {
                            joint.data = build(&item.plan, &item.spec).into();
                        }
                    }
                }
            }
            for item in &mut installed.items {
                if item.broken {
                    continue;
                }
                let Ok((_, impulses)) = joints_q.get(item.entity) else {
                    continue;
                };
                let force = impulses_len(impulses.linear) / dt * force_scale();
                let torque = angular_len(impulses.angular) / dt;
                if joints::should_break(&item.spec, force, torque) {
                    snap(&mut commands, &mut engine, item);
                    continue;
                }
                let pose = |e: Entity| {
                    poses
                        .get(e)
                        .map(|g| {
                            let (_, r, t) = g.to_scale_rotation_translation();
                            (r.normalize().to_array(), pos_to_doc(t).to_array())
                        })
                        .unwrap_or(([0.0, 0.0, 0.0, 1.0], [0.0; 3]))
                };
                let a = engine.entities.get(&item.actor).copied().map(pose);
                let b = match &item.target {
                    Some(target) => engine.entities.get(target).copied().map(pose),
                    None => Some(([0.0, 0.0, 0.0, 1.0], [0.0; 3])),
                };
                let position = match (a, b) {
                    (Some(a), Some(b)) => joints::measure(&item.spec, &item.plan.frames, a, b),
                    _ => item.last_position,
                };
                let speed = (position - item.last_position) / dt;
                item.last_position = position;
                joints::publish(
                    &item.actor,
                    &item.handle,
                    JointStatus {
                        broken: false,
                        enabled: item.enabled,
                        position,
                        speed,
                        force,
                        torque,
                    },
                );
            }
        }

        fn snap(commands: &mut Commands, engine: &mut Engine, item: &mut Installed) {
            item.broken = true;
            commands.entity(item.entity).despawn();
            joints::publish(
                &item.actor,
                &item.handle,
                JointStatus {
                    broken: true,
                    enabled: false,
                    ..Default::default()
                },
            );
            if !item.spec.break_message.trim().is_empty() {
                engine.fire(blockloom_core::vm::Event::Message(
                    item.spec.break_message.trim().to_string(),
                ));
            }
        }
    };
}

pub mod d3 {
    use super::*;
    use bevy_rapier3d::prelude as rp;
    use blockloom_core::physics::joints::JointAxis;

    const MODE: Mode = Mode::ThreeD;

    fn axis_of(axis: JointAxis) -> rp::JointAxis {
        match axis {
            JointAxis::LinX => rp::JointAxis::LinX,
            JointAxis::LinY => rp::JointAxis::LinY,
            JointAxis::LinZ => rp::JointAxis::LinZ,
            JointAxis::AngX => rp::JointAxis::AngX,
            JointAxis::AngY => rp::JointAxis::AngY,
            JointAxis::AngZ => rp::JointAxis::AngZ,
        }
    }

    fn vec_of(v: [f32; 3]) -> Vec3 {
        Vec3::from(v)
    }

    fn rot_of(q: Quat) -> Quat {
        q
    }

    fn len_of(v: f32) -> f32 {
        v
    }

    fn pos_to_doc(t: Vec3) -> Vec3 {
        t
    }

    fn force_scale() -> f32 {
        1.0
    }

    fn impulses_len(v: Vec3) -> f32 {
        v.length()
    }

    fn angular_len(v: Vec3) -> f32 {
        v.length()
    }

    dimension!();
}

pub mod d2 {
    use super::*;
    use crate::dim2::PIXELS_PER_METER as PPM;
    use bevy_rapier2d::prelude as rp;
    use blockloom_core::physics::joints::JointAxis;

    const MODE: Mode = Mode::TwoD;

    fn axis_of(axis: JointAxis) -> rp::JointAxis {
        match axis {
            JointAxis::LinX => rp::JointAxis::LinX,
            JointAxis::LinY => rp::JointAxis::LinY,
            // 2D has one turn, and the plan only ever asks for `AngX`.
            _ => rp::JointAxis::AngX,
        }
    }

    fn vec_of(v: [f32; 3]) -> Vec2 {
        Vec2::new(v[0], v[1]) / PPM
    }

    fn rot_of(q: Quat) -> f32 {
        q.to_euler(EulerRot::XYZ).2
    }

    fn len_of(v: f32) -> f32 {
        v / PPM
    }

    /// Placement positions are document pixels.
    fn pos_to_doc(t: Vec3) -> Vec3 {
        t
    }

    /// Impulses come back in metres; reports are in document units too.
    fn force_scale() -> f32 {
        1.0
    }

    fn impulses_len(v: Vec2) -> f32 {
        v.length()
    }

    fn angular_len(v: f32) -> f32 {
        v.abs()
    }

    dimension!();
}
