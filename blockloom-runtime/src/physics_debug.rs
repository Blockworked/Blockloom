//! The Physics Debug view: Rapier's own debug renderer, switched by the scene
//! view's `physics` flags. Off draws nothing and costs nothing.

use crate::edit::SceneEditor;
use bevy::prelude::*;
use blockloom_protocol::PhysicsDebug;

macro_rules! sync_flags {
    ($rp:ident, $view:expr, $context:expr) => {{
        let view: PhysicsDebug = $view;
        if let Some(mut context) = $context {
            let mode = $rp::prelude::DebugRenderModeFlags {
                collider_shapes: view.shapes,
                collider_aabbs: view.aabbs,
                contacts: view.contacts,
                solver_contacts: false,
                impulse_joints: view.joints,
                multibody_joints: view.joints,
                rigid_body_axes: view.axes,
                soft_bodies: false,
                pseudo_normals: false,
                soft_volume_contacts: false,
                soft_body_stress: false,
            };
            let on = view.any();
            if context.enabled != on {
                context.enabled = on;
            }
            if context.mode != mode {
                context.mode = mode;
            }
        }
    }};
}

thread_local! {
    static STATS: std::cell::RefCell<Stats> = std::cell::RefCell::new(Stats::default());
}

/// What the profiler shows of the physics world.
#[derive(Default, Clone, Copy)]
struct Stats {
    bodies: usize,
    active: usize,
    colliders: usize,
    joints: usize,
    /// Smoothed fixed-step cost, milliseconds.
    step_ms: f64,
}

/// The profiler rows: name and value.
pub fn metrics() -> Vec<(&'static str, f64)> {
    let s = STATS.with(|s| *s.borrow());
    if s.bodies + s.colliders + s.joints == 0 {
        return Vec::new();
    }
    vec![
        ("physics/bodies", s.bodies as f64),
        ("physics/active_bodies", s.active as f64),
        (
            "physics/sleeping_bodies",
            s.bodies.saturating_sub(s.active) as f64,
        ),
        ("physics/colliders", s.colliders as f64),
        ("physics/joints", s.joints as f64),
        ("physics/step_ms", s.step_ms),
    ]
}

macro_rules! counters {
    ($module:ident, $rp:ident) => {
        pub mod $module {
            use super::*;
            use bevy::prelude::*;

            /// Marks the start of the physics step.
            pub fn begin(mut started: Local<Option<web_time::Instant>>) {
                *started = Some(web_time::Instant::now());
            }

            /// Times the step and counts what is in it.
            pub fn end(
                started: Local<Option<web_time::Instant>>,
                bodies: Query<Option<&$rp::prelude::Sleeping>, With<$rp::prelude::RigidBody>>,
                colliders: Query<(), With<$rp::prelude::Collider>>,
                joints: Query<(), With<$rp::prelude::ImpulseJoint>>,
            ) {
                let ms = started.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
                let total = bodies.iter().count();
                let active = bodies
                    .iter()
                    .filter(|sleep| sleep.is_none_or(|s| !s.sleeping))
                    .count();
                STATS.with(|s| {
                    let mut s = s.borrow_mut();
                    s.bodies = total;
                    s.active = active;
                    s.colliders = colliders.iter().count();
                    s.joints = joints.iter().count();
                    s.step_ms += 0.1 * (ms - s.step_ms);
                });
            }
        }
    };
}

counters!(d3, bevy_rapier3d);
counters!(d2, bevy_rapier2d);

/// Copies the scene view's physics flags onto both dimensions' debug renderers.
pub fn sync(
    scene: Option<Res<SceneEditor>>,
    context_3d: Option<ResMut<bevy_rapier3d::render::DebugRenderContext>>,
    context_2d: Option<ResMut<bevy_rapier2d::render::DebugRenderContext>>,
) {
    let view = scene.map_or_else(PhysicsDebug::default, |s| s.view.physics);
    sync_flags!(bevy_rapier3d, view, context_3d);
    sync_flags!(bevy_rapier2d, view, context_2d);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_on_means_nothing_drawn() {
        let mut view = PhysicsDebug::default();
        assert!(!view.any());
        view.joints = true;
        assert!(view.any());
    }
}
