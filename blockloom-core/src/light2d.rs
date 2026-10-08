//! 2D lighting: point and spot lights, an ambient level that follows the
//! time of day, shadows from solid tiles and shape actors, and the maths the
//! renderer's shader and the `light level at` reporter share.
//!
//! The look is a multiply layer over the lit part of the world: ambient plus
//! the sum of every light's contribution at a point. A shadow-casting light
//! keeps a polar distance map (the nearest occluder along each of
//! [`SHADOW_SAMPLES`] directions); a point is lit when it is no farther than
//! that, and fades to dark over the light's softness past it. The CPU
//! functions here are the reference the shader copies.

use crate::material::hex_to_linear;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

/// Lights the renderer draws at once; the nearest to the camera win.
pub const MAX_LIGHTS: usize = 32;
/// Lights that may cast shadows at once, taken in the order they are listed.
pub const MAX_SHADOW_LIGHTS: usize = 16;
/// Directions in a light's shadow map.
pub const SHADOW_SAMPLES: usize = 256;
/// Keys in the time-of-day ambient ramp.
pub const MAX_RAMP_KEYS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Light2dKind {
    /// Shines every way from the actor.
    #[default]
    Point,
    /// A cone along the actor's facing (+X turned by its rotation).
    Spot,
}

/// Flicker for torches and neon: the light's brightness wobbles by value
/// noise on the run clock, so one seed replays the same.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flicker {
    /// 0-1, how much of the brightness the noise can take away.
    pub amount: f32,
    /// Noise steps a second.
    pub speed: f32,
    pub seed: u32,
}

impl Default for Flicker {
    fn default() -> Self {
        Self {
            amount: 0.0,
            speed: 8.0,
            seed: 1,
        }
    }
}

impl Flicker {
    pub fn normalize(&mut self) {
        self.amount = finite(self.amount, 0.0).clamp(0.0, 1.0);
        self.speed = finite(self.speed, 8.0).clamp(0.0, 120.0);
    }

    /// The brightness multiplier at `time` seconds, 1 minus up to `amount`.
    pub fn factor(&self, time: f32) -> f32 {
        if self.amount <= 0.0 || self.speed <= 0.0 {
            return 1.0;
        }
        let x = time.max(0.0) * self.speed;
        let i = x.floor();
        let t = x - i;
        let (a, b) = (
            hash01(self.seed, i as u32),
            hash01(self.seed, (i as u32).wrapping_add(1)),
        );
        let s = t * t * (3.0 - 2.0 * t);
        1.0 - self.amount * (a + (b - a) * s)
    }
}

/// A well-mixed value in 0-1 from a seed and a lattice point.
fn hash01(seed: u32, i: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B1) ^ i.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// A point or spot light riding the actor, in world units (pixels in 2D).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Light2dSpec {
    pub kind: Light2dKind,
    pub color: String,
    /// Color multiplier; above 1 passes into bloom on an HDR frame.
    pub intensity: f32,
    /// World units the light reaches.
    pub range: f32,
    /// How fast the light dies away: 1 is gentle, 4 is tight.
    pub falloff: f32,
    /// Degrees from the spot's axis where the cone starts to fade.
    pub inner_angle: f32,
    /// Degrees from the spot's axis where the light ends.
    pub outer_angle: f32,
    /// Whether solid tiles and shadow-casting actors block this light.
    pub shadows: bool,
    /// World units over which a shadow's edge fades.
    pub softness: f32,
    pub flicker: Flicker,
}

impl Default for Light2dSpec {
    fn default() -> Self {
        Self {
            kind: Light2dKind::Point,
            color: "#FFE2A8".to_string(),
            intensity: 1.0,
            range: 240.0,
            falloff: 2.0,
            inner_angle: 25.0,
            outer_angle: 40.0,
            shadows: false,
            softness: 8.0,
            flicker: Flicker::default(),
        }
    }
}

impl Light2dSpec {
    pub fn normalize(&mut self) {
        let d = Light2dSpec::default();
        self.intensity = finite(self.intensity, d.intensity).clamp(0.0, 64.0);
        self.range = finite(self.range, d.range).clamp(1.0, 100_000.0);
        self.falloff = finite(self.falloff, d.falloff).clamp(0.25, 8.0);
        self.outer_angle = finite(self.outer_angle, d.outer_angle).clamp(1.0, 180.0);
        self.inner_angle = finite(self.inner_angle, d.inner_angle).clamp(0.0, self.outer_angle);
        self.softness = finite(self.softness, d.softness).clamp(0.5, 1000.0);
        self.flicker.normalize();
    }

    /// The light as the renderer takes it: where it stands, which way it
    /// faces (radians), and what the blocks and the clock have done to it.
    pub fn sample(
        &self,
        position: [f32; 2],
        facing: f32,
        intensity: f32,
        time: f32,
    ) -> LightSample {
        let color = hex_to_linear(&self.color);
        let power = intensity.max(0.0) * self.flicker.factor(time);
        let (cos_inner, cos_outer) = match self.kind {
            Light2dKind::Point => (1.0, -2.0),
            Light2dKind::Spot => (
                self.inner_angle.to_radians().cos(),
                self.outer_angle.to_radians().cos(),
            ),
        };
        LightSample {
            position,
            direction: [facing.cos(), facing.sin()],
            color: [color[0] * power, color[1] * power, color[2] * power],
            range: self.range,
            falloff: self.falloff,
            cos_inner,
            cos_outer,
            softness: self.softness,
            shadows: self.shadows,
        }
    }
}

/// One light as a frame draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct LightSample {
    pub position: [f32; 2],
    /// Unit vector of the spot's axis.
    pub direction: [f32; 2],
    /// Linear color with intensity and flicker folded in.
    pub color: [f32; 3],
    pub range: f32,
    pub falloff: f32,
    /// Cosines of the cone's edges; `cos_outer` below -1 is no cone.
    pub cos_inner: f32,
    pub cos_outer: f32,
    pub softness: f32,
    pub shadows: bool,
}

/// How much of a light reaches `distance`: 1 at the light, 0 at its range.
pub fn attenuation(distance: f32, range: f32, falloff: f32) -> f32 {
    let r = distance / range.max(1e-3);
    (1.0 - r * r).clamp(0.0, 1.0).powf(falloff)
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    if hi <= lo {
        return if x >= hi { 1.0 } else { 0.0 };
    }
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Which two shadow-map directions a direction falls between.
fn shadow_slots(dx: f32, dy: f32) -> (usize, usize) {
    let u = (dy.atan2(dx) / TAU).rem_euclid(1.0) * SHADOW_SAMPLES as f32;
    let i0 = (u.floor() as usize) % SHADOW_SAMPLES;
    (i0, (i0 + 1) % SHADOW_SAMPLES)
}

impl LightSample {
    /// What the light adds at `point`, given its shadow map if it has one.
    pub fn at(&self, shadow: Option<&[f32]>, point: [f32; 2]) -> [f32; 3] {
        let (dx, dy) = (point[0] - self.position[0], point[1] - self.position[1]);
        let distance = (dx * dx + dy * dy).sqrt();
        if distance >= self.range {
            return [0.0; 3];
        }
        let mut amount = attenuation(distance, self.range, self.falloff);
        if self.cos_outer > -1.0 && distance > 1e-4 {
            let cos = (dx * self.direction[0] + dy * self.direction[1]) / distance;
            amount *= smoothstep(self.cos_outer, self.cos_inner, cos);
        }
        if let Some(map) = shadow
            && distance > 1e-4
        {
            let (a, b) = shadow_slots(dx, dy);
            // The nearer of the two neighbours, so a corner never leaks.
            let blocked_at = map[a].min(map[b]);
            let soft = self.softness.max(1e-3);
            amount *= ((blocked_at + soft - distance) / soft).clamp(0.0, 1.0);
        }
        [
            self.color[0] * amount,
            self.color[1] * amount,
            self.color[2] * amount,
        ]
    }
}

/// Something that stops light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Occluder {
    Circle {
        center: [f32; 2],
        radius: f32,
    },
    /// A rectangle turned by `angle` radians about its centre.
    Box {
        center: [f32; 2],
        half: [f32; 2],
        angle: f32,
    },
}

impl Occluder {
    /// How far a ray from `origin` along unit `dir` travels before it
    /// enters this occluder. A ray starting inside one is not blocked by it:
    /// a lamp set into a wall still lights the room.
    pub fn hit(&self, origin: [f32; 2], dir: [f32; 2]) -> Option<f32> {
        match *self {
            Occluder::Circle { center, radius } => {
                let (ox, oy) = (origin[0] - center[0], origin[1] - center[1]);
                let c = ox * ox + oy * oy - radius * radius;
                if c <= 0.0 {
                    return None;
                }
                let b = ox * dir[0] + oy * dir[1];
                let disc = b * b - c;
                if disc < 0.0 {
                    return None;
                }
                let t = -b - disc.sqrt();
                (t >= 0.0).then_some(t)
            }
            Occluder::Box {
                center,
                half,
                angle,
            } => {
                let (s, c) = angle.sin_cos();
                let (rx, ry) = (origin[0] - center[0], origin[1] - center[1]);
                // Into the box's frame.
                let o = [rx * c + ry * s, -rx * s + ry * c];
                let d = [dir[0] * c + dir[1] * s, -dir[0] * s + dir[1] * c];
                if o[0].abs() <= half[0] && o[1].abs() <= half[1] {
                    return None;
                }
                let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
                for axis in 0..2 {
                    if d[axis].abs() < 1e-9 {
                        if o[axis].abs() > half[axis] {
                            return None;
                        }
                        continue;
                    }
                    let (a, b) = (
                        (-half[axis] - o[axis]) / d[axis],
                        (half[axis] - o[axis]) / d[axis],
                    );
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                }
                (near <= far && near >= 0.0).then_some(near)
            }
        }
    }

    /// Whether any part of the occluder is within `range` of `origin`.
    pub fn near(&self, origin: [f32; 2], range: f32) -> bool {
        match *self {
            Occluder::Circle { center, radius } => {
                let (dx, dy) = (center[0] - origin[0], center[1] - origin[1]);
                (dx * dx + dy * dy).sqrt() <= range + radius
            }
            Occluder::Box { center, half, .. } => {
                let reach = (half[0] * half[0] + half[1] * half[1]).sqrt();
                let (dx, dy) = (center[0] - origin[0], center[1] - origin[1]);
                (dx * dx + dy * dy).sqrt() <= range + reach
            }
        }
    }
}

/// The nearest occluder along each of [`SHADOW_SAMPLES`] directions from
/// `origin`, capped at `range`.
pub fn distance_map(origin: [f32; 2], range: f32, occluders: &[Occluder]) -> Vec<f32> {
    let near: Vec<&Occluder> = occluders.iter().filter(|o| o.near(origin, range)).collect();
    (0..SHADOW_SAMPLES)
        .map(|i| {
            let angle = i as f32 / SHADOW_SAMPLES as f32 * TAU;
            let dir = [angle.cos(), angle.sin()];
            near.iter()
                .filter_map(|o| o.hit(origin, dir))
                .fold(range, f32::min)
        })
        .collect()
}

/// The time-of-day ambient ramp's keys and everything else the world says
/// about the 2D lit look. Off by default, so old projects look as they did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lighting2d {
    /// Light the 2D world: everything on a layer up to `unlit_above` is
    /// multiplied by ambient plus the lights.
    pub enabled: bool,
    pub ambient_color: String,
    /// The ambient multiplier: 1 leaves the picture as drawn, 0 is black
    /// where no light reaches.
    pub ambient: f32,
    /// Layers above this draw over the lighting, unlit. HUD-like overlays and
    /// glows that should ignore the dark go there.
    pub unlit_above: i32,
    /// Ambient by hour; with the time-of-day director on, the ambient follows
    /// it round the clock instead of the two fields above.
    pub ramp: Vec<AmbientKey>,
    /// Hours the day starts and ends, for `is night?`.
    pub sunrise: f32,
    pub sunset: f32,
}

impl Default for Lighting2d {
    fn default() -> Self {
        Self {
            enabled: false,
            ambient_color: "#FFFFFF".to_string(),
            ambient: 0.3,
            unlit_above: 100,
            ramp: Vec::new(),
            sunrise: 6.0,
            sunset: 18.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmbientKey {
    /// Hours, 0-24.
    pub hour: f32,
    pub color: String,
    pub intensity: f32,
}

impl Lighting2d {
    pub fn normalize(&mut self) {
        let d = Lighting2d::default();
        self.ambient = finite(self.ambient, d.ambient).clamp(0.0, 16.0);
        self.unlit_above = self.unlit_above.clamp(-10_000, 10_000);
        self.sunrise = finite(self.sunrise, d.sunrise).rem_euclid(24.0);
        self.sunset = finite(self.sunset, d.sunset).rem_euclid(24.0);
        self.ramp.truncate(MAX_RAMP_KEYS);
        for key in &mut self.ramp {
            key.hour = finite(key.hour, 0.0).rem_euclid(24.0);
            key.intensity = finite(key.intensity, 1.0).clamp(0.0, 16.0);
        }
        self.ramp.sort_by(|a, b| a.hour.total_cmp(&b.hour));
    }

    /// The ambient light at `hour` as linear color times intensity: the
    /// ramp's blend (wrapping midnight) when `follow_clock`, else the fixed
    /// pair.
    pub fn ambient_at(&self, hour: f32, follow_clock: bool) -> [f32; 3] {
        let fixed = || scaled(hex_to_linear(&self.ambient_color), self.ambient);
        if !follow_clock || self.ramp.is_empty() {
            return fixed();
        }
        let hour = hour.rem_euclid(24.0);
        let keys = &self.ramp;
        let after = keys.iter().position(|k| k.hour > hour).unwrap_or(0);
        let before = (after + keys.len() - 1) % keys.len();
        let (a, b) = (&keys[before], &keys[after]);
        let span = (b.hour - a.hour).rem_euclid(24.0);
        let t = if span <= 1e-6 {
            0.0
        } else {
            (hour - a.hour).rem_euclid(24.0) / span
        };
        let (ca, cb) = (
            scaled(hex_to_linear(&a.color), a.intensity),
            scaled(hex_to_linear(&b.color), b.intensity),
        );
        [
            ca[0] + (cb[0] - ca[0]) * t,
            ca[1] + (cb[1] - ca[1]) * t,
            ca[2] + (cb[2] - ca[2]) * t,
        ]
    }

    pub fn is_night(&self, hour: f32) -> bool {
        let hour = hour.rem_euclid(24.0);
        if self.sunrise <= self.sunset {
            hour < self.sunrise || hour >= self.sunset
        } else {
            hour >= self.sunset && hour < self.sunrise
        }
    }
}

fn scaled(color: [f32; 4], k: f32) -> [f32; 3] {
    [color[0] * k, color[1] * k, color[2] * k]
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// The lit look as a frame samples it: what the renderer draws and what
/// `light level at` and `is night?` read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Light2dSense {
    pub enabled: bool,
    /// Linear ambient, color times intensity.
    pub ambient: [f32; 3],
    pub night: bool,
    pub lights: Vec<LightSample>,
    /// One distance map per light that casts shadows, `None` for the rest.
    pub maps: Vec<Option<Vec<f32>>>,
}

impl Light2dSense {
    /// The light on the ground at `point`: ambient plus every light.
    pub fn color_at(&self, point: [f32; 2]) -> [f32; 3] {
        let mut sum = self.ambient;
        for (light, map) in self.lights.iter().zip(&self.maps) {
            let add = light.at(map.as_deref(), point);
            for (s, a) in sum.iter_mut().zip(add) {
                *s += a;
            }
        }
        sum
    }

    /// `color_at` as one number (Rec. 709 luma), 1 being a picture shown as
    /// drawn. A world without lighting reads 1 everywhere.
    pub fn level_at(&self, point: [f32; 2]) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        let c = self.color_at(point);
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(position: [f32; 2]) -> LightSample {
        Light2dSpec {
            color: "#FFFFFF".to_string(),
            range: 100.0,
            ..Light2dSpec::default()
        }
        .sample(position, 0.0, 1.0, 0.0)
    }

    #[test]
    fn a_light_fades_to_nothing_at_its_range() {
        let l = lamp([0.0, 0.0]);
        let near = l.at(None, [1.0, 0.0])[0];
        let mid = l.at(None, [50.0, 0.0])[0];
        assert!(near > 0.99);
        assert!(mid < near && mid > 0.0);
        assert_eq!(l.at(None, [100.0, 0.0]), [0.0; 3]);
        assert_eq!(l.at(None, [0.0, 150.0]), [0.0; 3]);
    }

    #[test]
    fn a_spot_lights_only_its_cone() {
        let spec = Light2dSpec {
            kind: Light2dKind::Spot,
            color: "#FFFFFF".to_string(),
            range: 200.0,
            inner_angle: 10.0,
            outer_angle: 30.0,
            ..Light2dSpec::default()
        };
        let l = spec.sample([0.0, 0.0], 0.0, 1.0, 0.0);
        let ahead = l.at(None, [50.0, 0.0])[0];
        let edge = l.at(None, [50.0, 20.0])[0];
        let behind = l.at(None, [-50.0, 0.0])[0];
        let aside = l.at(None, [0.0, 50.0])[0];
        assert!(ahead > 0.1);
        assert!(edge < ahead);
        assert_eq!(behind, 0.0);
        assert_eq!(aside, 0.0);
        // Turning the actor turns the cone.
        let up = spec.sample([0.0, 0.0], std::f32::consts::FRAC_PI_2, 1.0, 0.0);
        assert!(up.at(None, [0.0, 50.0])[0] > 0.1);
        assert_eq!(up.at(None, [50.0, 0.0])[0], 0.0);
    }

    #[test]
    fn a_circle_casts_a_shadow_behind_it() {
        let wall = [Occluder::Circle {
            center: [40.0, 0.0],
            radius: 10.0,
        }];
        let map = distance_map([0.0, 0.0], 100.0, &wall);
        assert_eq!(map.len(), SHADOW_SAMPLES);
        // Straight at the circle the nearest point is 30 away.
        assert!((map[0] - 30.0).abs() < 1.0);
        // Away from it nothing stands in the way.
        assert_eq!(map[SHADOW_SAMPLES / 2], 100.0);
        let l = Light2dSpec {
            color: "#FFFFFF".to_string(),
            range: 100.0,
            softness: 4.0,
            ..Light2dSpec::default()
        }
        .sample([0.0, 0.0], 0.0, 1.0, 0.0);
        let before = l.at(Some(&map), [20.0, 0.0])[0];
        let behind = l.at(Some(&map), [70.0, 0.0])[0];
        let aside = l.at(Some(&map), [0.0, 70.0])[0];
        assert!(before > 0.4);
        assert_eq!(behind, 0.0);
        assert!(aside > 0.0);
    }

    #[test]
    fn a_turned_box_blocks_by_its_edges() {
        let wall = Occluder::Box {
            center: [30.0, 0.0],
            half: [5.0, 20.0],
            angle: 0.0,
        };
        assert!((wall.hit([0.0, 0.0], [1.0, 0.0]).unwrap() - 25.0).abs() < 1e-4);
        assert!(wall.hit([0.0, 0.0], [0.0, 1.0]).is_none());
        assert!(wall.hit([0.0, 0.0], [-1.0, 0.0]).is_none());
        // Turned a quarter, the tall side lies along x.
        let turned = Occluder::Box {
            center: [30.0, 0.0],
            half: [5.0, 20.0],
            angle: std::f32::consts::FRAC_PI_2,
        };
        assert!((turned.hit([0.0, 0.0], [1.0, 0.0]).unwrap() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_light_inside_an_occluder_is_not_blocked_by_it() {
        let wall = Occluder::Box {
            center: [0.0, 0.0],
            half: [10.0, 10.0],
            angle: 0.0,
        };
        assert!(wall.hit([0.0, 0.0], [1.0, 0.0]).is_none());
        let map = distance_map([0.0, 0.0], 80.0, &[wall]);
        assert!(map.iter().all(|d| *d == 80.0));
    }

    #[test]
    fn a_shadow_edge_never_leaks_through_a_corner() {
        let wall = [Occluder::Box {
            center: [50.0, 0.0],
            half: [5.0, 5.0],
            angle: 0.0,
        }];
        let map = distance_map([0.0, 0.0], 150.0, &wall);
        let l = Light2dSpec {
            color: "#FFFFFF".to_string(),
            range: 150.0,
            softness: 1.0,
            ..Light2dSpec::default()
        }
        .sample([0.0, 0.0], 0.0, 1.0, 0.0);
        // Dead behind the box is dark, however the two slots straddle it.
        for y in [-3.0, -1.0, 0.0, 1.0, 3.0] {
            assert_eq!(l.at(Some(&map), [90.0, y])[0], 0.0, "y {y}");
        }
    }

    #[test]
    fn flicker_is_seeded_bounded_and_off_at_zero() {
        let steady = Flicker::default();
        assert_eq!(steady.factor(3.3), 1.0);
        let f = Flicker {
            amount: 0.5,
            speed: 10.0,
            seed: 7,
        };
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..400 {
            let v = f.factor(i as f32 * 0.037);
            assert!((0.5..=1.0).contains(&v), "{v}");
            lo = lo.min(v);
            hi = hi.max(v);
            assert_eq!(v, f.factor(i as f32 * 0.037));
        }
        assert!(hi - lo > 0.2, "it should actually move: {lo}..{hi}");
        let other = Flicker { seed: 8, ..f };
        assert_ne!(f.factor(0.5), other.factor(0.5));
    }

    #[test]
    fn the_ramp_blends_round_midnight() {
        let mut l = Lighting2d {
            ramp: vec![
                AmbientKey {
                    hour: 6.0,
                    color: "#FFFFFF".to_string(),
                    intensity: 1.0,
                },
                AmbientKey {
                    hour: 22.0,
                    color: "#FFFFFF".to_string(),
                    intensity: 0.2,
                },
            ],
            ..Lighting2d::default()
        };
        l.normalize();
        let noon = l.ambient_at(6.0, true)[0];
        let dusk = l.ambient_at(22.0, true)[0];
        let late = l.ambient_at(2.0, true)[0];
        assert!((noon - 1.0).abs() < 1e-4);
        assert!((dusk - 0.2).abs() < 1e-4);
        // 2:00 is a quarter of the way from 22:00 back up to 06:00 + 8h.
        assert!(late > 0.2 && late < 1.0);
        assert!((late - (0.2 + 0.8 * 0.5)).abs() < 1e-3, "{late}");
        // Off the clock, the fixed pair answers.
        let fixed = l.ambient_at(22.0, false)[0];
        assert!((fixed - 0.3).abs() < 1e-4);
    }

    #[test]
    fn night_is_outside_the_day() {
        let l = Lighting2d::default();
        assert!(l.is_night(2.0));
        assert!(!l.is_night(12.0));
        assert!(l.is_night(18.0));
        assert!(l.is_night(23.9));
        let polar = Lighting2d {
            sunrise: 20.0,
            sunset: 4.0,
            ..Lighting2d::default()
        };
        assert!(!polar.is_night(23.0));
        assert!(polar.is_night(12.0));
    }

    #[test]
    fn level_at_adds_ambient_and_lights() {
        let sense = Light2dSense {
            enabled: true,
            ambient: [0.2, 0.2, 0.2],
            night: false,
            lights: vec![lamp([0.0, 0.0])],
            maps: vec![None],
        };
        let at_lamp = sense.level_at([0.0, 0.0]);
        let far = sense.level_at([500.0, 0.0]);
        assert!(at_lamp > 1.1);
        assert!((far - 0.2).abs() < 1e-4);
        let off = Light2dSense::default();
        assert_eq!(off.level_at([0.0, 0.0]), 1.0);
    }

    #[test]
    fn old_documents_load_with_lighting_off() {
        let l: Lighting2d = serde_json::from_str("{}").unwrap();
        assert!(!l.enabled);
        let spec: Light2dSpec = serde_json::from_str(r#"{"kind":"Spot"}"#).unwrap();
        assert_eq!(spec.kind, Light2dKind::Spot);
        assert_eq!(spec.range, 240.0);
    }

    #[test]
    fn normalize_clamps_nonsense() {
        let mut spec = Light2dSpec {
            intensity: f32::NAN,
            range: -5.0,
            inner_angle: 90.0,
            outer_angle: 30.0,
            ..Light2dSpec::default()
        };
        spec.normalize();
        assert_eq!(spec.intensity, 1.0);
        assert_eq!(spec.range, 1.0);
        assert!(spec.inner_angle <= spec.outer_angle);
    }
}
