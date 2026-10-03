//! Drives each CharacterMotor: fills the player's intent from the project's
//! actions, steps the motor through its controller every fixed tick and turns
//! the actor towards where it moves.
//!
//! Core decides (see `blockloom_core::physics::motor`); this module only
//! supplies the world around it: the camera's yaw, the platform under the
//! feet and the actor's transform.

use crate::controller::ControllerAccess;
use crate::engine::Engine;
use crate::world::WorldCamera;
use bevy::prelude::*;
use blockloom_core::physics::PhysicsPlan;
use blockloom_core::physics::motor::{self, MotorOwner, MoveSpace};
use blockloom_core::scene::Mode;
use std::collections::HashMap;

/// Registers every motor of a plan and forgets the last world's.
pub fn register_plan(plan: &PhysicsPlan) {
    motor::reset();
    for planned in &plan.motors {
        motor::register(&planned.actor, planned.spec.clone());
    }
}

/// Forgets every motor: the world is going away.
pub fn clear() {
    motor::reset();
}

/// The action's name for the nth local player.
fn action_name(base: &str, player: u8) -> String {
    if player == 0 {
        base.to_string()
    } else {
        format!("{base} P{}", player + 1)
    }
}

/// Fills the intent of every player-owned motor from the published actions.
/// Runs after the sensors are published, so a press this frame is seen.
pub fn latch_player_input(
    engine: NonSend<Engine>,
    layers: Option<Res<crate::physics_install::PhysicsLayers>>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let mode = layers
        .and_then(|layers| layers.mode)
        .unwrap_or(Mode::ThreeD);
    for actor in motor::actors() {
        let Some(spec) = motor::spec_of(&actor) else {
            continue;
        };
        if spec.owner != MotorOwner::Player {
            continue;
        }
        let (direction, jump_pressed, jump_held, sprint, crouch) =
            blockloom_core::sense::read(|sense| {
                let find = |base: &str| sense.actions.get(&action_name(base, spec.player));
                let stick = find("Move").map(|a| a.vector).unwrap_or([0.0; 2]);
                let jump = find("Jump");
                (
                    match mode {
                        // Up on the stick is away from the camera (-Z).
                        Mode::ThreeD => [stick[0], 0.0, -stick[1]],
                        Mode::TwoD => [stick[0], stick[1], 0.0],
                    },
                    jump.is_some_and(|a| a.pressed),
                    jump.is_some_and(|a| a.held),
                    find("Sprint").is_some_and(|a| a.held),
                    find("Crouch").is_some_and(|a| a.held),
                )
            });
        motor::submit_input(&actor, direction, jump_pressed, jump_held, sprint, crouch);
    }
}

/// Where the platform an actor stands on was when it last looked.
type Carried = HashMap<String, (String, Vec3)>;

/// Steps every motor one fixed tick.
pub fn drive_motors(
    // Core keeps motors in thread-local state: stay on the main thread.
    mut engine: NonSendMut<Engine>,
    access: ControllerAccess,
    cameras: Query<&GlobalTransform, With<WorldCamera>>,
    facing: Res<crate::player_camera::BodyFacing>,
    transforms: Query<(&Transform, &GlobalTransform)>,
    mut commands: Commands,
    time: Res<Time<Fixed>>,
    mut carried: Local<Carried>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let actors = motor::actors();
    if actors.is_empty() {
        carried.clear();
        return;
    }
    let camera_yaw = cameras.iter().next().map(|camera| {
        let forward = camera.forward();
        (-forward.x).atan2(-forward.z)
    });
    let dt = time.timestep().as_secs_f32();
    let tick = engine.contact_ticks;
    for actor in actors {
        let Some(spec) = motor::spec_of(&actor) else {
            continue;
        };
        let Some(&entity) = engine.entities.get(&actor) else {
            continue;
        };
        let Ok((transform, global)) = transforms.get(entity) else {
            continue;
        };
        let own_yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
        let yaw = match spec.space {
            MoveSpace::World => 0.0,
            MoveSpace::Actor => own_yaw,
            MoveSpace::Camera => camera_yaw.unwrap_or(own_yaw),
        };
        let carry = carry_of(&actor, &engine, &transforms, &mut carried);
        let _ = global;
        let driven = access.scope(tick, || motor::drive(&actor, yaw, carry));
        match driven {
            Ok(driven) => {
                // A camera that turns the body wins over the motor's own facing.
                // The write is deferred: the controller service reads every Transform.
                let turned = if let Some(&yaw) = facing.0.get(&actor) {
                    Some(Quat::from_rotation_y(yaw))
                } else if let (Some(face), true) = (driven.face, driven.turn_speed > 0.0) {
                    let mut delta = face - own_yaw;
                    while delta > std::f32::consts::PI {
                        delta -= std::f32::consts::TAU;
                    }
                    while delta < -std::f32::consts::PI {
                        delta += std::f32::consts::TAU;
                    }
                    let most = driven.turn_speed.to_radians() * dt;
                    Some(Quat::from_rotation_y(own_yaw + delta.clamp(-most, most)))
                } else {
                    None
                };
                if let Some(rotation) = turned {
                    commands
                        .entity(entity)
                        .queue(move |mut entity: EntityWorldMut| {
                            if let Some(mut transform) = entity.get_mut::<Transform>() {
                                transform.rotation = rotation;
                            }
                        });
                }
            }
            Err(why) => {
                // Said once per cause, not once per tick.
                if engine.motor_warned.insert(format!("{actor}: {why}")) {
                    warn!("motor {actor}: {why}");
                }
            }
        }
    }
    // Events are for reporters (read off the motor); nothing queues them here.
    motor::take_events();
}

/// How far the thing an actor stands on moved since last tick. A jump past a
/// few metres is a teleport, not a ride.
fn carry_of(
    actor: &str,
    engine: &Engine,
    transforms: &Query<(&Transform, &GlobalTransform)>,
    carried: &mut Carried,
) -> [f32; 3] {
    const TELEPORT: f32 = 5.0;
    let Some(support) = motor::support_of(actor) else {
        carried.remove(actor);
        return [0.0; 3];
    };
    let Some(&entity) = engine.entities.get(&support) else {
        carried.remove(actor);
        return [0.0; 3];
    };
    let Ok((_, global)) = transforms.get(entity) else {
        return [0.0; 3];
    };
    let now = global.translation();
    let delta = match carried.get(actor) {
        Some((was, before)) if *was == support => now - *before,
        _ => Vec3::ZERO,
    };
    carried.insert(actor.to_string(), (support, now));
    if delta.length() > TELEPORT {
        [0.0; 3]
    } else {
        delta.to_array()
    }
}
