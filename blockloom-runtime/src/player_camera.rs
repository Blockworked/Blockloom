//! Drives the world camera for an actor with a PlayerCamera: look input,
//! orbit, wall avoidance, zoom, dead zones and smoothing. Core owns the
//! arithmetic (`blockloom_core::player_camera`); this supplies input, the
//! target's pose and a swept ball.

use crate::engine::Engine;
use crate::queries::QueryAccess;
use bevy::prelude::*;
use blockloom_core::components::{CameraAttach, CameraView};
use blockloom_core::physics::query::{self, QueryFilter, QueryRequest, QueryShape, TriggerPolicy};
use blockloom_core::player_camera::{PlayerCameraSpec, smooth};
use std::collections::HashMap;

/// The yaw (radians) a camera wants its actor's body turned to, by actor.
/// Rewritten every frame; the motor driver applies it.
#[derive(Resource, Default)]
pub struct BodyFacing(pub HashMap<String, f32>);

/// What the camera remembers between frames.
#[derive(Default)]
pub struct RigState {
    actor: String,
    /// Degrees about up, independent of the body.
    yaw: f32,
    /// Where the target stood last frame, for its velocity.
    last: Option<Vec3>,
    ahead: [f32; 2],
    eye: Option<Vec3>,
}

impl RigState {
    fn start(&mut self, actor: &str, yaw: f32) {
        *self = Self {
            actor: actor.to_string(),
            yaw,
            ..Self::default()
        };
    }
}

fn yaw_of(target: &Transform) -> f32 {
    target.rotation.to_euler(EulerRot::YXZ).0.to_degrees()
}

/// Whether a camera with look input should take the pointer when a run
/// starts: the first-person and third-person presets read a locked mouse.
pub fn wants_lock(engine: &Engine) -> bool {
    engine.project.actors.iter().any(|actor| {
        actor.components.contains("Camera")
            && actor
                .components
                .player_camera()
                .is_some_and(|spec| spec.enabled && spec.look)
    })
}

/// How far a crouch has pulled the top of the controller down.
fn crouch_drop(engine: &Engine, actor: &str) -> f32 {
    let Some(standing) = engine
        .actor(actor)
        .and_then(|a| a.components.character_controller().map(|c| c.height))
    else {
        return 0.0;
    };
    blockloom_core::physics::controller::spec_of(actor)
        .map_or(0.0, |live| (standing - live.height).max(0.0))
}

/// The camera in a 2D project: dead zone, look-ahead and smoothing.
pub fn follow_2d(
    spec: &PlayerCameraSpec,
    state: &mut RigState,
    rig: &CameraAttach,
    target: &Transform,
    camera: &mut Transform,
    dt: f32,
    actor: &str,
) {
    if state.actor != actor {
        state.start(actor, 0.0);
    }
    let velocity = match state.last {
        Some(last) if dt > 0.0 => {
            let d = (target.translation - last) / dt;
            [d.x, d.y]
        }
        _ => [0.0; 2],
    };
    state.last = Some(target.translation);
    state.ahead = spec.lead(state.ahead, velocity, dt);
    let aim = [
        target.translation.x + rig.offset[0],
        target.translation.y + rig.offset[1],
    ];
    let at = spec.follow_2d(
        [camera.translation.x, camera.translation.y],
        aim,
        state.ahead,
        dt,
    );
    camera.translation.x = at[0];
    camera.translation.y = at[1];
}

/// The camera in a 3D project. With no PlayerCamera this is the Camera
/// component's own framing, unchanged.
#[allow(clippy::too_many_arguments)]
pub fn pose_3d(
    engine: &Engine,
    queries: &QueryAccess,
    spec: Option<&PlayerCameraSpec>,
    state: &mut RigState,
    rig: &mut CameraAttach,
    target: &Transform,
    actor: &str,
    scroll: f32,
    dt: f32,
    camera: &mut Transform,
    facing: &mut BodyFacing,
) {
    let offset = Vec3::from(rig.offset);
    let Some(spec) = spec else {
        legacy_3d(engine, rig, target, offset, camera);
        return;
    };
    if state.actor != actor {
        state.start(actor, yaw_of(target));
    }
    let live = engine.running && !engine.paused;
    if !engine.running {
        // The next run starts from where the body faces.
        state.actor.clear();
    }
    if spec.look && live {
        let (mouse, stick) = blockloom_core::sense::read(|s| {
            let mouse = if s.mouse_locked {
                s.mouse_delta
            } else {
                [0.0; 2]
            };
            let axis = |name: &str| {
                let v = s.gamepad_axes.get(name).copied().unwrap_or(0.0);
                if v.abs() < 0.1 { 0.0 } else { v }
            };
            (mouse, [axis("rightstickx"), axis("rightsticky")])
        });
        let turn = spec.look_turn(mouse, stick, dt);
        let (yaw, pitch) = spec.turned(rig.view, state.yaw, rig.pitch, turn);
        state.yaw = yaw;
        rig.pitch = pitch;
    } else if !spec.look {
        state.yaw = yaw_of(target);
    }
    if live {
        rig.distance = spec.zoomed(rig.distance, scroll);
    }
    let yaw = Quat::from_rotation_y(state.yaw.to_radians());
    let drop = if rig.view == CameraView::FirstPerson && spec.eye_follows_stance {
        crouch_drop(engine, actor)
    } else {
        0.0
    };
    let pivot = target.translation + target.rotation * (offset - Vec3::Y * drop);
    let (eye, rotation) = match rig.view {
        CameraView::FirstPerson => {
            if spec.turn_body && spec.look && live {
                facing.0.insert(actor.to_string(), state.yaw.to_radians());
            }
            (pivot, yaw * Quat::from_rotation_x(rig.pitch.to_radians()))
        }
        CameraView::ThirdPerson => {
            let pitch = rig.pitch.to_radians();
            let heading = if spec.look { yaw } else { target.rotation };
            let back = heading * Vec3::new(0.0, pitch.sin(), pitch.cos());
            let wanted = rig.distance.max(0.01);
            let hit = spec
                .collision
                .then(|| first_contact(engine, queries, spec, actor, pivot, back, wanted))
                .flatten();
            let distance = spec.clear_distance(wanted, hit);
            let eye = pivot + back * distance;
            let look = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
            (eye, look.rotation)
        }
        CameraView::Follow => {
            let settings = &engine.project.world.camera;
            let boom = Vec3::from(settings.position) - Vec3::from(settings.look_at);
            let eye = target.translation + boom;
            let look = Transform::from_translation(eye).looking_at(target.translation, Vec3::Y);
            (eye, look.rotation)
        }
    };
    // Smoothing trails the position; the view direction stays exact so
    // movement relative to it never lags the screen.
    let eye = match (state.eye, spec.smoothing > 0.0 && live) {
        (Some(last), true) => Vec3::new(
            smooth(last.x, eye.x, dt, spec.smoothing),
            smooth(last.y, eye.y, dt, spec.smoothing),
            smooth(last.z, eye.z, dt, spec.smoothing),
        ),
        _ => eye,
    };
    state.eye = Some(eye);
    camera.translation = eye;
    camera.rotation = rotation;
}

/// How far along `direction` from `pivot` a swept ball gets before touching
/// the world, ignoring the camera's own actor.
fn first_contact(
    engine: &Engine,
    queries: &QueryAccess,
    spec: &PlayerCameraSpec,
    actor: &str,
    pivot: Vec3,
    direction: Vec3,
    wanted: f32,
) -> Option<f32> {
    let request = QueryRequest::Cast {
        shape: QueryShape::Ball {
            radius: spec.collision_radius.max(0.01),
        },
        from: pivot.to_array(),
        to: (pivot + direction * wanted).to_array(),
    };
    let filter = QueryFilter {
        triggers: TriggerPolicy::Ignore,
        exclude_actors: vec![actor.to_string()],
        ..QueryFilter::default()
    };
    let outcome = queries.scope(engine.contact_ticks, || {
        query::dispatch(&request, &filter, 1)
    });
    if outcome.error.is_some() {
        return None;
    }
    outcome.hits.first().map(|hit| hit.distance)
}

/// What the Camera component alone does: the rig's own framing.
fn legacy_3d(
    engine: &Engine,
    rig: &CameraAttach,
    target: &Transform,
    offset: Vec3,
    camera: &mut Transform,
) {
    // The offset is in the actor's own frame, so an eye stays on its head
    // however the actor is turned.
    let pivot = target.translation + target.rotation * offset;
    match rig.view {
        CameraView::FirstPerson => {
            camera.translation = pivot;
            // Yaw from the body, pitch from the rig: the FPS composition,
            // where turning the body never weakens looking up and down.
            camera.rotation = target.rotation * Quat::from_rotation_x(rig.pitch.to_radians());
        }
        CameraView::ThirdPerson => {
            let pitch = rig.pitch.to_radians();
            let back = -crate::world::forward_of(target, blockloom_core::scene::Mode::ThreeD)
                * rig.distance
                * pitch.cos();
            camera.translation = pivot + back + Vec3::Y * rig.distance * pitch.sin();
            camera.look_at(pivot, Vec3::Y);
        }
        CameraView::Follow => {
            let settings = &engine.project.world.camera;
            let boom = Vec3::from(settings.position) - Vec3::from(settings.look_at);
            camera.translation = target.translation + boom;
            camera.look_at(target.translation, Vec3::Y);
        }
    }
}
