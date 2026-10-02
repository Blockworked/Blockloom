//! Body operations: how a force, acceleration, impulse or velocity change becomes
//! the impulse a backend applies.
//!
//! Every mode reduces to an impulse over one fixed step, so a submitted force acts
//! for exactly that tick and never lingers. Torque follows the same rules with the
//! body's inertia in place of its mass.

use glam::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Unity's `ForceMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ForceMode {
    /// A continuous force in newtons, applied for the step: `f * dt / mass`.
    #[default]
    Force,
    /// A continuous acceleration, mass independent: `a * dt`.
    Acceleration,
    /// An instant impulse in newton seconds: `impulse / mass`.
    Impulse,
    /// An instant velocity change, mass independent.
    VelocityChange,
}

impl ForceMode {
    pub const ALL: [ForceMode; 4] = [
        ForceMode::Force,
        ForceMode::Acceleration,
        ForceMode::Impulse,
        ForceMode::VelocityChange,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ForceMode::Force => "Force",
            ForceMode::Acceleration => "Acceleration",
            ForceMode::Impulse => "Impulse",
            ForceMode::VelocityChange => "VelocityChange",
        }
    }

    pub fn parse(text: &str) -> Option<ForceMode> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name().eq_ignore_ascii_case(text.trim()))
    }

    /// The linear impulse (kg m/s) for `vector` under this mode.
    pub fn linear_impulse(self, vector: Vec3, mass: f32, dt: f32) -> Vec3 {
        match self {
            ForceMode::Force => vector * dt,
            ForceMode::Acceleration => vector * (mass * dt),
            ForceMode::Impulse => vector,
            ForceMode::VelocityChange => vector * mass,
        }
    }

    /// The angular impulse for a torque `vector` under this mode, given the
    /// body's inertia tensor in world axes.
    pub fn angular_impulse(self, vector: Vec3, inertia: Mat3, dt: f32) -> Vec3 {
        match self {
            ForceMode::Force => vector * dt,
            ForceMode::Acceleration => inertia * (vector * dt),
            ForceMode::Impulse => vector,
            ForceMode::VelocityChange => inertia * vector,
        }
    }
}

/// A body's inertia tensor in world axes, from its principal moments, the frame
/// those moments are measured in and the body's rotation.
pub fn world_inertia(principal: Vec3, frame: Quat, body: Quat) -> Mat3 {
    let axes = Mat3::from_quat(body * frame);
    axes * Mat3::from_diagonal(principal) * axes.transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn the_four_modes_change_velocity_as_documented() {
        let v = Vec3::X;
        let mass = 4.0;
        let dv = |mode: ForceMode| mode.linear_impulse(v, mass, DT) / mass;
        assert!((dv(ForceMode::Force) - Vec3::X * DT / mass).length() < 1e-6);
        assert!((dv(ForceMode::Acceleration) - Vec3::X * DT).length() < 1e-6);
        assert!((dv(ForceMode::Impulse) - Vec3::X / mass).length() < 1e-6);
        assert!((dv(ForceMode::VelocityChange) - Vec3::X).length() < 1e-6);
    }

    #[test]
    fn torque_uses_the_inertia_in_world_axes() {
        let inertia = world_inertia(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::IDENTITY,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        );
        // A quarter turn about z swaps the x and y moments.
        assert!((inertia.x_axis.x - 2.0).abs() < 1e-5);
        assert!((inertia.y_axis.y - 1.0).abs() < 1e-5);
        let spin = ForceMode::VelocityChange.angular_impulse(Vec3::X, inertia, DT);
        assert!((spin - Vec3::X * 2.0).length() < 1e-5);
        let accel = ForceMode::Acceleration.angular_impulse(Vec3::Y, inertia, DT);
        assert!((accel - Vec3::Y * DT).length() < 1e-5);
    }

    #[test]
    fn names_round_trip() {
        for mode in ForceMode::ALL {
            assert_eq!(ForceMode::parse(mode.name()), Some(mode));
        }
        assert_eq!(ForceMode::parse("velocity change"), None);
        assert_eq!(ForceMode::parse(" impulse "), Some(ForceMode::Impulse));
    }
}
