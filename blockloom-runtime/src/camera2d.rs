//! 2D camera finishing (`blockloom_core::camera2d`): axis locks, bounds, zoom,
//! pixel snap, roll and shake, laid over the pose `drive_camera` follows to.
//! The shake is render-only: it is taken off again before the follow runs, so
//! the camera never smooths towards its own tremor.

use crate::engine::{Dimension, Engine};
use crate::world::WorldCamera;
use bevy::prelude::*;
use blockloom_core::camera2d::{self as cam, Camera2dSettings};
use blockloom_core::sense::{self, Camera2dSense};

pub fn register(app: &mut App) {
    app.init_resource::<Camera2dState>().add_systems(
        Update,
        (
            remove_shake
                .before(crate::world::drive_camera)
                .run_if(is_2d),
            apply_camera
                .after(crate::world::drive_camera)
                .after(crate::world::publish_sensors)
                .run_if(is_2d),
        ),
    );
}

fn is_2d(dimension: Res<Dimension>) -> bool {
    dimension.0 == blockloom_core::scene::Mode::TwoD
}

/// What the camera remembers between frames.
#[derive(Resource, Default)]
pub struct Camera2dState {
    /// The shake offset currently added to the camera's position.
    offset: Vec2,
    trauma: f32,
    /// Where the locked axes were held, set the first frame a lock is on.
    anchor: Option<Vec2>,
    /// The scale the project gave the camera, kept for when no zoom is asked.
    base_scale: Option<f32>,
    /// Real-clock second the hitstop ends, when this system paused the world.
    hitstop_until: Option<f64>,
}

/// The project's camera rules with what blocks set this run laid over them.
pub fn effective(engine: &Engine) -> Camera2dSettings {
    let mut settings = engine.project.world.camera2d.clone();
    for (dial, value) in &engine.look2d.post {
        settings.set(*dial, value);
    }
    settings
}

fn live(engine: &Engine) -> bool {
    engine.running
}

/// Takes last frame's shake back off before the follow reads the pose.
fn remove_shake(
    engine: NonSend<Engine>,
    mut state: ResMut<Camera2dState>,
    mut cameras: Query<&mut Transform, With<WorldCamera>>,
) {
    if state.offset == Vec2::ZERO {
        return;
    }
    if !live(&engine) {
        state.offset = Vec2::ZERO;
        return;
    }
    if let Ok(mut transform) = cameras.single_mut() {
        transform.translation.x -= state.offset.x;
        transform.translation.y -= state.offset.y;
    }
    state.offset = Vec2::ZERO;
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_camera(
    mut engine: NonSendMut<Engine>,
    mut state: ResMut<Camera2dState>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    editor: Option<Res<crate::edit::SceneEditor>>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<WorldCamera>>,
) {
    if !live(&engine)
        || editor
            .as_ref()
            .is_some_and(|editor| crate::edit::editing(&engine, editor))
    {
        sense::publish_camera2d(Camera2dSense::default());
        return;
    }
    let Ok((mut transform, mut projection)) = cameras.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };
    let settings = effective(&engine);
    let dt = time.delta_secs();
    let now = real.elapsed_secs_f64();

    // Trauma comes in from `shake camera`, and goes out over time.
    let added = std::mem::take(&mut engine.look2d.shake);
    state.trauma = cam::decay((state.trauma + added).min(1.0), dt, settings.shake.decay);

    // Hitstop freezes the world on the real clock; the interface keeps going.
    let asked = std::mem::take(&mut engine.look2d.hitstop);
    if asked > 0.0 {
        let until = now + asked as f64;
        state.hitstop_until = Some(state.hitstop_until.map_or(until, |u| u.max(until)));
        if !engine.paused {
            crate::world::set_paused(&mut engine, true, time.elapsed_secs_f64());
        }
    }
    if let Some(until) = state.hitstop_until
        && now >= until
    {
        state.hitstop_until = None;
        crate::world::set_paused(&mut engine, false, time.elapsed_secs_f64());
    }

    // Zoom: view height wins over pixels per unit, which wins over the
    // project's own.
    let base = *state.base_scale.get_or_insert(ortho.scale);
    let (area_w, area_h) = (ortho.area.width(), ortho.area.height());
    let pixels = if ortho.scale > 0.0 {
        [area_w / ortho.scale, area_h / ortho.scale]
    } else {
        [0.0; 2]
    };
    let mut scale = if settings.zoom_height > 0.0 && pixels[1] > 0.0 {
        cam::scale_for_height(settings.zoom_height, pixels[1])
    } else if let Some(zoom) = settings.zoom {
        1.0 / zoom
    } else {
        base
    };
    if settings.pixel_snap {
        scale = cam::snap_scale(scale);
    }
    scale = scale.clamp(0.01, 200.0);
    if ortho.scale != scale {
        ortho.scale = scale;
    }

    let mut at = Vec2::new(transform.translation.x, transform.translation.y);
    // Locks hold an axis where it stood when the lock came on.
    if settings.lock_x || settings.lock_y {
        let anchor = *state.anchor.get_or_insert(at);
        if settings.lock_x {
            at.x = anchor.x;
        }
        if settings.lock_y {
            at.y = anchor.y;
        }
    } else {
        state.anchor = None;
    }
    let mut at_bounds = false;
    if let Some(bounds) = &settings.bounds
        && pixels[0] > 0.0
    {
        let half = [pixels[0] * scale * 0.5, pixels[1] * scale * 0.5];
        let (held, hit) = cam::confine(at.to_array(), half, bounds, settings.soft_edge);
        at = Vec2::from(held);
        at_bounds = hit;
    }
    if settings.pixel_snap {
        at = Vec2::from(cam::snap_position(at.to_array(), scale));
    }

    let (offset, angle) = cam::shake_at(state.trauma, real.elapsed_secs(), &settings.shake);
    let mut offset = Vec2::from(offset);
    if settings.pixel_snap {
        offset = Vec2::from(cam::snap_position(offset.to_array(), scale));
    }
    transform.translation.x = at.x + offset.x;
    transform.translation.y = at.y + offset.y;
    state.offset = offset;
    let roll = (settings.rotation + angle).to_radians();
    let rotation = Quat::from_rotation_z(roll);
    if transform.rotation != rotation {
        transform.rotation = rotation;
    }
    sense::publish_camera2d(Camera2dSense {
        zoom: 1.0 / scale,
        rotation: settings.rotation + angle,
        at_bounds,
        shaking: state.trauma > 0.01,
        cover: 0.0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::blocks::Look2dDial;

    #[test]
    fn blocks_override_the_project_camera() {
        let mut look = crate::light2d::Look2d::default();
        look.set(Look2dDial::LockX, "1");
        look.set(Look2dDial::Zoom, "2");
        look.set(Look2dDial::Shake, "0.6");
        look.set(Look2dDial::Shake, "0.6");
        look.set(Look2dDial::Hitstop, "0.1");
        assert_eq!(look.shake, 1.0);
        assert_eq!(look.hitstop, 0.1);
        let mut settings = Camera2dSettings::default();
        for (d, v) in &look.post {
            settings.set(*d, v);
        }
        assert!(settings.lock_x);
        assert_eq!(settings.zoom, Some(2.0));
    }
}
