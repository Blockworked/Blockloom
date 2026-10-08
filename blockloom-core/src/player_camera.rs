//! The PlayerCamera: how the actor's Camera reacts to look input, walls and
//! the target's motion.
//!
//! It configures the one world camera the Camera component already owns and
//! never makes a second. Everything here is pure arithmetic so the runtime
//! only supplies input, the target's pose and a swept ball.

use crate::components::CameraView;
use serde::{Deserialize, Serialize};

/// A PlayerCamera component. Angles are degrees, lengths world units (metres
/// in 3D, pixels in 2D).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerCameraSpec {
    pub enabled: bool,
    /// Which local player's Look actions turn it (0 is the first).
    pub player: u8,
    /// Look input turns the view (3D). Off keeps the Camera component's
    /// fixed framing.
    pub look: bool,
    /// Degrees a mouse count turns the view.
    pub sensitivity: f32,
    /// Degrees a second at full stick deflection.
    pub stick_speed: f32,
    pub invert_y: bool,
    pub min_pitch: f32,
    pub max_pitch: f32,
    /// Turn the body to face where the camera looks (first person).
    pub turn_body: bool,
    /// Seconds the camera takes to catch up (0 follows exactly).
    pub smoothing: f32,
    /// Third person: pull in so walls never sit between camera and target.
    pub collision: bool,
    /// Radius of the swept ball that keeps the lens off a wall.
    pub collision_radius: f32,
    /// Closest the camera is pulled to the target.
    pub min_distance: f32,
    /// Scroll or triggers change the distance within these limits.
    pub zoom: bool,
    pub zoom_min: f32,
    pub zoom_max: f32,
    pub zoom_speed: f32,
    /// Side and top-down views: how far the target walks before the camera
    /// follows, each side of the centre.
    pub dead_zone: [f32; 2],
    /// How far ahead of a moving target the camera looks.
    pub look_ahead: f32,
    /// Seconds the look-ahead takes to build or fade.
    pub look_ahead_smoothing: f32,
    /// A first-person eye drops and rises with a crouch.
    pub eye_follows_stance: bool,
}

impl Default for PlayerCameraSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            player: 0,
            look: true,
            sensitivity: 0.12,
            stick_speed: 180.0,
            invert_y: false,
            min_pitch: -80.0,
            max_pitch: 80.0,
            turn_body: false,
            smoothing: 0.0,
            collision: true,
            collision_radius: 0.25,
            min_distance: 0.6,
            zoom: false,
            zoom_min: 2.0,
            zoom_max: 12.0,
            zoom_speed: 1.0,
            dead_zone: [0.0, 0.0],
            look_ahead: 0.0,
            look_ahead_smoothing: 0.25,
            eye_follows_stance: true,
        }
    }
}

impl PlayerCameraSpec {
    /// Defaults that suit a 2D project: nothing to look around, pixels not
    /// metres.
    pub fn for_2d() -> Self {
        Self {
            look: false,
            collision: false,
            smoothing: 0.15,
            dead_zone: [48.0, 32.0],
            look_ahead: 64.0,
            collision_radius: 0.0,
            min_distance: 0.0,
            ..Self::default()
        }
    }

    /// First person: the body follows the view and the eye tracks the stance.
    pub fn first_person() -> Self {
        Self {
            turn_body: true,
            collision: false,
            ..Self::default()
        }
    }

    /// Why a setting is wrong, with its field name, or nothing.
    pub fn validate(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut bad = |field: &str, why: &str| out.push((field.to_string(), why.to_string()));
        let finite = [
            ("sensitivity", self.sensitivity),
            ("stick_speed", self.stick_speed),
            ("min_pitch", self.min_pitch),
            ("max_pitch", self.max_pitch),
            ("smoothing", self.smoothing),
            ("collision_radius", self.collision_radius),
            ("min_distance", self.min_distance),
            ("zoom_min", self.zoom_min),
            ("zoom_max", self.zoom_max),
            ("zoom_speed", self.zoom_speed),
            ("dead_zone", self.dead_zone[0] + self.dead_zone[1]),
            ("look_ahead", self.look_ahead),
            ("look_ahead_smoothing", self.look_ahead_smoothing),
        ];
        for (field, value) in finite {
            if !value.is_finite() {
                bad(field, "must be a finite number");
            }
        }
        if self.sensitivity < 0.0 || self.stick_speed < 0.0 {
            bad("sensitivity", "cannot be negative");
        }
        if !(-89.0..=89.0).contains(&self.min_pitch) || !(-89.0..=89.0).contains(&self.max_pitch) {
            bad("min_pitch", "pitch limits stay inside -89 to 89 degrees");
        } else if self.min_pitch > self.max_pitch {
            bad("min_pitch", "must not be above the maximum pitch");
        }
        if self.smoothing < 0.0 || self.look_ahead_smoothing < 0.0 {
            bad("smoothing", "cannot be negative");
        }
        if self.collision_radius < 0.0 || self.min_distance < 0.0 {
            bad("collision_radius", "lengths cannot be negative");
        }
        if self.zoom && !(self.zoom_min > 0.0 && self.zoom_min <= self.zoom_max) {
            bad("zoom_min", "needs 0 < zoom min <= zoom max");
        }
        if self.dead_zone[0] < 0.0 || self.dead_zone[1] < 0.0 {
            bad("dead_zone", "cannot be negative");
        }
        out
    }

    /// Look input to a turn: `mouse` counts this frame (down positive, as the
    /// sensors give it) and `stick` deflection (up positive). Returns the yaw
    /// and pitch change in degrees; right is positive yaw, looking up
    /// positive pitch.
    pub fn look_turn(&self, mouse: [f32; 2], stick: [f32; 2], dt: f32) -> (f32, f32) {
        let yaw = mouse[0] * self.sensitivity + stick[0] * self.stick_speed * dt;
        let up = -mouse[1] * self.sensitivity + stick[1] * self.stick_speed * dt;
        (yaw, if self.invert_y { -up } else { up })
    }

    /// The clamp third person and first person share, for the pitch a rig
    /// stores (looking up positive).
    pub fn clamp_pitch(&self, pitch: f32) -> f32 {
        pitch.clamp(self.min_pitch, self.max_pitch)
    }

    /// Turns a stored yaw and pitch by a look turn. In third person the
    /// stored pitch is the camera's elevation, which falls as the player
    /// looks up.
    pub fn turned(&self, view: CameraView, yaw: f32, pitch: f32, turn: (f32, f32)) -> (f32, f32) {
        let yaw = wrap_degrees(yaw + turn.0);
        let pitch = match view {
            CameraView::ThirdPerson => pitch - turn.1,
            _ => pitch + turn.1,
        };
        (yaw, self.clamp_pitch(pitch))
    }

    /// A distance changed by scroll or trigger input, kept inside the zoom
    /// limits.
    pub fn zoomed(&self, distance: f32, scroll: f32) -> f32 {
        if !self.zoom || scroll == 0.0 {
            return distance;
        }
        (distance - scroll * self.zoom_speed).clamp(self.zoom_min, self.zoom_max)
    }

    /// How far out a collision-avoiding camera may sit: the swept ball's
    /// first contact (`hit`, a distance along the boom) pulls it in, never
    /// closer than the minimum and never past what was asked.
    pub fn clear_distance(&self, wanted: f32, hit: Option<f32>) -> f32 {
        if !self.collision {
            return wanted;
        }
        let clear = hit.map_or(wanted, |h| h.min(wanted));
        clear.max(self.min_distance.min(wanted))
    }

    /// Where a side or top view puts its centre: the target must leave the
    /// dead zone before the camera moves, the look-ahead leads it, and the
    /// result catches up over `smoothing` seconds.
    pub fn follow_2d(
        &self,
        camera: [f32; 2],
        target: [f32; 2],
        ahead: [f32; 2],
        dt: f32,
    ) -> [f32; 2] {
        let mut want = camera;
        for axis in 0..2 {
            let aim = target[axis] + ahead[axis];
            let half = self.dead_zone[axis];
            let offset = aim - camera[axis];
            if offset > half {
                want[axis] = aim - half;
            } else if offset < -half {
                want[axis] = aim + half;
            }
        }
        [
            smooth(camera[0], want[0], dt, self.smoothing),
            smooth(camera[1], want[1], dt, self.smoothing),
        ]
    }

    /// The look-ahead vector for a target moving at `velocity`: the
    /// direction of travel times the look-ahead distance, eased by
    /// `look_ahead_smoothing` from the last one.
    pub fn lead(&self, previous: [f32; 2], velocity: [f32; 2], dt: f32) -> [f32; 2] {
        if self.look_ahead <= 0.0 {
            return [0.0; 2];
        }
        let speed = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
        let goal = if speed > 1e-3 {
            [
                velocity[0] / speed * self.look_ahead,
                velocity[1] / speed * self.look_ahead,
            ]
        } else {
            [0.0; 2]
        };
        [
            smooth(previous[0], goal[0], dt, self.look_ahead_smoothing),
            smooth(previous[1], goal[1], dt, self.look_ahead_smoothing),
        ]
    }
}

/// Moves `current` towards `target` with a time constant of `seconds`, the
/// same at any frame rate. Zero seconds lands on it.
pub fn smooth(current: f32, target: f32, dt: f32, seconds: f32) -> f32 {
    if seconds <= 1e-4 || dt <= 0.0 {
        return target;
    }
    current + (target - current) * (1.0 - (-dt / seconds).exp())
}

/// An angle in degrees brought into -180..=180.
pub fn wrap_degrees(angle: f32) -> f32 {
    let wrapped = (angle + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 && angle > 0.0 {
        180.0
    } else {
        wrapped
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn a_default_spec_is_valid_and_bad_ones_are_named() {
        assert!(PlayerCameraSpec::default().validate().is_empty());
        assert!(PlayerCameraSpec::for_2d().validate().is_empty());
        let mut spec = PlayerCameraSpec::default();
        spec.min_pitch = 30.0;
        spec.max_pitch = 10.0;
        assert_eq!(spec.validate()[0].0, "min_pitch");
        spec = PlayerCameraSpec::default();
        spec.sensitivity = f32::NAN;
        assert!(!spec.validate().is_empty());
        spec = PlayerCameraSpec::default();
        spec.zoom = true;
        spec.zoom_min = 5.0;
        spec.zoom_max = 2.0;
        assert_eq!(spec.validate()[0].0, "zoom_min");
    }

    #[test]
    fn looking_up_raises_a_first_person_pitch_and_lowers_a_third_person_one() {
        let spec = PlayerCameraSpec::default();
        let turn = spec.look_turn([0.0, -100.0], [0.0; 2], 0.016);
        assert!(turn.1 > 0.0, "mouse up looks up: {turn:?}");
        let (_, first) = spec.turned(CameraView::FirstPerson, 0.0, 0.0, turn);
        let (_, third) = spec.turned(CameraView::ThirdPerson, 0.0, 15.0, turn);
        assert!(first > 0.0);
        assert!(third < 15.0, "the camera sinks as the player looks up");
    }

    #[test]
    fn pitch_stops_at_its_limits_and_yaw_wraps() {
        let spec = PlayerCameraSpec::default();
        let (yaw, pitch) = spec.turned(CameraView::FirstPerson, 170.0, 70.0, (30.0, 40.0));
        assert_eq!(pitch, spec.max_pitch);
        assert!((yaw - -160.0).abs() < 1e-4, "{yaw}");
    }

    #[test]
    fn inverting_y_flips_the_pitch_turn_only() {
        let mut spec = PlayerCameraSpec::default();
        let base = spec.look_turn([10.0, 10.0], [0.0; 2], 0.016);
        spec.invert_y = true;
        let flipped = spec.look_turn([10.0, 10.0], [0.0; 2], 0.016);
        assert_eq!(base.0, flipped.0);
        assert_eq!(base.1, -flipped.1);
    }

    #[test]
    fn a_stick_turns_by_time_not_by_frame() {
        let spec = PlayerCameraSpec::default();
        let slow: f32 = (0..60)
            .map(|_| spec.look_turn([0.0; 2], [1.0, 0.0], 1.0 / 60.0).0)
            .sum();
        let fast: f32 = (0..120)
            .map(|_| spec.look_turn([0.0; 2], [1.0, 0.0], 1.0 / 120.0).0)
            .sum();
        assert!((slow - fast).abs() < 1e-3);
        assert!((slow - spec.stick_speed).abs() < 1e-3);
    }

    #[test]
    fn a_wall_pulls_the_camera_in_but_not_inside_the_minimum() {
        let spec = PlayerCameraSpec::default();
        assert_eq!(spec.clear_distance(6.0, None), 6.0);
        assert_eq!(spec.clear_distance(6.0, Some(3.0)), 3.0);
        assert_eq!(spec.clear_distance(6.0, Some(0.1)), spec.min_distance);
        assert_eq!(
            spec.clear_distance(0.3, Some(0.1)),
            0.3,
            "never beyond what was asked"
        );
        let mut off = spec;
        off.collision = false;
        assert_eq!(off.clear_distance(6.0, Some(1.0)), 6.0);
    }

    #[test]
    fn zoom_stays_inside_its_limits() {
        let mut spec = PlayerCameraSpec::default();
        assert_eq!(spec.zoomed(6.0, 3.0), 6.0, "zoom is off");
        spec.zoom = true;
        assert_eq!(spec.zoomed(6.0, 2.0), 4.0);
        assert_eq!(spec.zoomed(6.0, 20.0), spec.zoom_min);
        assert_eq!(spec.zoomed(6.0, -20.0), spec.zoom_max);
    }

    #[test]
    fn a_side_view_waits_for_the_target_to_leave_the_dead_zone() {
        let mut spec = PlayerCameraSpec::for_2d();
        spec.smoothing = 0.0;
        spec.dead_zone = [40.0, 20.0];
        let inside = spec.follow_2d([0.0, 0.0], [30.0, 10.0], [0.0; 2], 0.016);
        assert_eq!(inside, [0.0, 0.0]);
        let right = spec.follow_2d([0.0, 0.0], [100.0, 0.0], [0.0; 2], 0.016);
        assert_eq!(
            right,
            [60.0, 0.0],
            "the target sits on the dead zone's edge"
        );
        let below = spec.follow_2d([0.0, 0.0], [0.0, -50.0], [0.0; 2], 0.016);
        assert_eq!(below, [0.0, -30.0]);
    }

    #[test]
    fn smoothing_converges_at_any_frame_rate() {
        let run = |steps: u32| {
            let dt = 1.0 / steps as f32;
            let mut at = 0.0;
            for _ in 0..steps {
                at = smooth(at, 10.0, dt, 0.25);
            }
            at
        };
        assert!((run(60) - run(240)).abs() < 1e-3);
        assert_eq!(smooth(1.0, 5.0, 0.016, 0.0), 5.0);
    }

    #[test]
    fn the_look_ahead_leads_the_motion_and_fades_when_still() {
        let spec = PlayerCameraSpec::for_2d();
        let mut ahead = [0.0; 2];
        for _ in 0..120 {
            ahead = spec.lead(ahead, [200.0, 0.0], 1.0 / 60.0);
        }
        assert!((ahead[0] - spec.look_ahead).abs() < 1.0, "{ahead:?}");
        for _ in 0..240 {
            ahead = spec.lead(ahead, [0.0; 2], 1.0 / 60.0);
        }
        assert!(ahead[0].abs() < 1.0);
        let mut none = spec;
        none.look_ahead = 0.0;
        assert_eq!(none.lead([5.0, 5.0], [9.0, 0.0], 0.016), [0.0; 2]);
    }

    #[test]
    fn wrap_keeps_angles_in_a_half_turn() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
        assert_eq!(wrap_degrees(45.0), 45.0);
        assert_eq!(wrap_degrees(180.0), 180.0);
    }
}
