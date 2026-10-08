//! 2D movement helpers a character motor reacts to: conveyor belts that carry
//! whatever stands on them, and hazards that knock a motor back and give it a
//! window of invulnerability.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A belt: whatever the actor's motor stands on it is carried along the
/// actor's +X turned by its rotation, units a second.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConveyorSpec {
    pub speed: f32,
    pub enabled: bool,
}

impl Default for ConveyorSpec {
    fn default() -> Self {
        Self {
            speed: 96.0,
            enabled: true,
        }
    }
}

impl ConveyorSpec {
    pub fn normalize(&mut self) {
        if !self.speed.is_finite() {
            self.speed = 0.0;
        }
        self.speed = self.speed.clamp(-10_000.0, 10_000.0);
    }

    /// The belt's surface velocity for an actor turned `rotation_z` radians.
    pub fn velocity(&self, rotation_z: f32) -> [f32; 2] {
        if !self.enabled {
            return [0.0; 2];
        }
        [self.speed * rotation_z.cos(), self.speed * rotation_z.sin()]
    }
}

/// A hurt volume: a motor touching the actor is thrown away from it and
/// cannot be hurt again for `invulnerability` seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HazardSpec {
    pub enabled: bool,
    /// Speed of the throw, units a second.
    pub knockback: f32,
    /// How much of the throw goes up (0-1, the rest is away from the hazard).
    pub lift: f32,
    pub invulnerability: f32,
    /// Broadcast when it hurts someone, empty for none. The victim is
    /// readable as `the actor I hurt` is not offered: the message is global.
    pub message: String,
}

impl Default for HazardSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            knockback: 288.0,
            lift: 0.5,
            invulnerability: 1.0,
            message: "hurt".to_string(),
        }
    }
}

impl HazardSpec {
    pub fn normalize(&mut self) {
        let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
        self.knockback = finite(self.knockback, 0.0).clamp(0.0, 10_000.0);
        self.lift = finite(self.lift, 0.5).clamp(0.0, 1.0);
        self.invulnerability = finite(self.invulnerability, 0.0).clamp(0.0, 60.0);
        self.message = self.message.trim().to_string();
    }

    /// The throw for a victim at `victim` off a hazard at `hazard`: away along
    /// x (up when stacked), with `lift` of it going up the screen.
    pub fn throw(&self, hazard: [f32; 2], victim: [f32; 2]) -> [f32; 2] {
        let dx = victim[0] - hazard[0];
        let side = if dx.abs() < 1e-3 { 0.0 } else { dx.signum() };
        let away = (1.0 - self.lift).max(0.0);
        [self.knockback * away * side, self.knockback * self.lift]
    }
}

/// Who is safe from being hurt again, and until when (run clock seconds).
#[derive(Debug, Default, Clone)]
pub struct Invulnerable {
    until: HashMap<String, f64>,
}

impl Invulnerable {
    pub fn clear(&mut self) {
        self.until.clear();
    }

    /// Hurts `victim` at `now` unless it is still safe; true when it hurts.
    pub fn strike(&mut self, victim: &str, now: f64, window: f32) -> bool {
        if self.until.get(victim).is_some_and(|&until| now < until) {
            return false;
        }
        self.until
            .insert(victim.to_string(), now + f64::from(window));
        true
    }

    pub fn is_safe(&self, victim: &str, now: f64) -> bool {
        self.until.get(victim).is_some_and(|&until| now < until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_belt_runs_along_its_facing() {
        let belt = ConveyorSpec::default();
        let v = belt.velocity(0.0);
        assert!((v[0] - 96.0).abs() < 1e-4 && v[1].abs() < 1e-4);
        let up = belt.velocity(std::f32::consts::FRAC_PI_2);
        assert!(up[0].abs() < 1e-3 && (up[1] - 96.0).abs() < 1e-3);
        let off = ConveyorSpec {
            enabled: false,
            ..belt
        };
        assert_eq!(off.velocity(0.0), [0.0, 0.0]);
    }

    #[test]
    fn a_hazard_throws_away_and_up() {
        let h = HazardSpec {
            knockback: 100.0,
            lift: 0.25,
            ..HazardSpec::default()
        };
        assert_eq!(h.throw([0.0, 0.0], [10.0, 0.0]), [75.0, 25.0]);
        assert_eq!(h.throw([0.0, 0.0], [-10.0, 0.0]), [-75.0, 25.0]);
        assert_eq!(
            h.throw([5.0, 0.0], [5.0, 3.0]),
            [0.0, 25.0],
            "stacked goes up"
        );
    }

    #[test]
    fn invulnerability_blocks_until_the_window_closes() {
        let mut safe = Invulnerable::default();
        assert!(safe.strike("p", 1.0, 1.0));
        assert!(!safe.strike("p", 1.5, 1.0));
        assert!(safe.is_safe("p", 1.99));
        assert!(safe.strike("p", 2.0, 1.0));
        assert!(safe.strike("q", 1.2, 1.0), "each victim has its own window");
    }

    #[test]
    fn normalize_clamps_wild_numbers() {
        let mut h = HazardSpec {
            knockback: f32::NAN,
            lift: 4.0,
            invulnerability: -3.0,
            ..HazardSpec::default()
        };
        h.normalize();
        assert_eq!((h.knockback, h.lift, h.invulnerability), (0.0, 1.0, 0.0));
    }

    #[test]
    fn components_round_trip_and_default_missing_fields() {
        use crate::components::ActorComponent;
        let belt: ActorComponent =
            serde_json::from_str(r#"{"component":"Conveyor","conveyor":{"speed":12}}"#).unwrap();
        let ActorComponent::Conveyor { conveyor } = belt else {
            panic!("a conveyor");
        };
        assert!(conveyor.enabled && conveyor.speed == 12.0);
        let hazard: ActorComponent =
            serde_json::from_str(r#"{"component":"Hazard","hazard":{}}"#).unwrap();
        assert!(matches!(hazard, ActorComponent::Hazard { .. }));
    }
}
