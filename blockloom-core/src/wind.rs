//! Wind: one field the clouds, fog, particles and anything else that moves
//! with the air all read, so a storm reads as one storm.
//!
//! The global wind blows one way at one speed, gusting on 1D gradient noise
//! over time and slowing towards the ground on a log-law profile. `Volume`s
//! carry local zones that override it, add to it or swirl round their axis,
//! with turbulence on top. Everything here is plain arithmetic on the fixed
//! tick's clock, so one seed replays the same weather however frames fall.

use crate::volume::{VolumePose, VolumeSpec};
use glam::{Quat, Vec2, Vec3, Vec3Swizzles};
use serde::{Deserialize, Serialize};

/// The project's wind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wind {
    /// Degrees the wind blows towards, clockwise from north: -Z in 3D (the
    /// sky's azimuth), up the screen in 2D.
    pub direction: f32,
    /// World units per second at the reference height.
    pub speed: f32,
    /// Extra speed at the peak of a gust.
    pub gust: f32,
    /// Roughly how many gusts a second.
    pub gust_frequency: f32,
    /// Degrees the gusts swing the direction either way.
    pub veer: f32,
    /// Slow the wind towards the ground on a log-law profile (3D only).
    pub profile: bool,
    /// Metres above the ground where `speed` is measured.
    pub reference_height: f32,
    /// Roughness length in metres: calm below it, and the larger it is the
    /// more the ground holds the wind back.
    pub roughness: f32,
    /// World y of the ground the profile measures from.
    pub ground: f32,
    /// 0-1: scales speed, gusts and turbulence together.
    pub storm: f32,
    /// Seeds the gusts, so one seed is one day's wind.
    pub seed: u32,
    pub clouds: CloudDrift,
}

impl Default for Wind {
    fn default() -> Self {
        Self {
            direction: 45.0,
            speed: 0.0,
            gust: 0.0,
            gust_frequency: 0.2,
            veer: 15.0,
            profile: true,
            reference_height: 10.0,
            roughness: 0.1,
            ground: 0.0,
            storm: 0.0,
            seed: 1,
            clouds: CloudDrift::default(),
        }
    }
}

/// How the clouds ride the wind. Read by the cloud passes as offsets the
/// runtime integrates each tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudDrift {
    /// Metres up the clouds take their wind from.
    pub altitude: f32,
    /// How much of the wind aloft moves the cloud shapes.
    pub follow: f32,
    /// Extra drift of the shapes, world units per second.
    pub advection: [f32; 3],
    /// Drift of the erosion detail against the shapes, so edges boil.
    pub erosion: [f32; 3],
    /// Multiplies the wind aloft for planar cloud layers' UV scroll.
    pub layer_scroll: f32,
    /// 1-1000: runs the clouds faster than the world, for demo skies.
    pub time_lapse: f32,
    /// Seeds the cloud noise. A new seed is a new sky with the same dials.
    pub seed: u32,
}

impl Default for CloudDrift {
    fn default() -> Self {
        Self {
            altitude: 1500.0,
            follow: 1.0,
            advection: [0.0; 3],
            erosion: [0.0, 0.5, 0.0],
            layer_scroll: 1.0,
            time_lapse: 1.0,
            seed: 1,
        }
    }
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

impl Wind {
    pub fn normalize(&mut self) {
        let d = Wind::default();
        self.direction = finite(self.direction, d.direction).rem_euclid(360.0);
        self.speed = finite(self.speed, 0.0).clamp(0.0, MAX_SPEED);
        self.gust = finite(self.gust, 0.0).clamp(0.0, MAX_SPEED);
        self.gust_frequency = finite(self.gust_frequency, d.gust_frequency).clamp(0.0, 10.0);
        self.veer = finite(self.veer, d.veer).clamp(0.0, 180.0);
        self.reference_height =
            finite(self.reference_height, d.reference_height).clamp(0.01, 10_000.0);
        self.roughness = finite(self.roughness, d.roughness).clamp(0.0001, 10.0);
        self.ground = finite(self.ground, 0.0).clamp(-1.0e6, 1.0e6);
        self.storm = finite(self.storm, 0.0).clamp(0.0, 1.0);
        let c = &mut self.clouds;
        let dc = CloudDrift::default();
        c.altitude = finite(c.altitude, dc.altitude).clamp(0.0, 100_000.0);
        c.follow = finite(c.follow, dc.follow).clamp(0.0, 100.0);
        c.advection = c
            .advection
            .map(|v| finite(v, 0.0).clamp(-MAX_SPEED, MAX_SPEED));
        c.erosion = c
            .erosion
            .map(|v| finite(v, 0.0).clamp(-MAX_SPEED, MAX_SPEED));
        c.layer_scroll = finite(c.layer_scroll, dc.layer_scroll).clamp(0.0, 100.0);
        c.time_lapse = finite(c.time_lapse, 1.0).clamp(1.0, 1000.0);
    }

    /// The dials as they stand this run: the project's, with what blocks
    /// and scripts set laid over them.
    pub fn dials(&self, over: &WindOverrides) -> WindDials {
        WindDials {
            direction: over.direction.unwrap_or(self.direction),
            speed: over.speed.unwrap_or(self.speed),
            gust: over.gust.unwrap_or(self.gust),
            storm: over.storm.unwrap_or(self.storm),
        }
    }

    /// How much of the reference speed blows at world height `y`: 0 at the
    /// roughness length, 1 at the reference height, and on up by the log.
    pub fn height_factor(&self, y: f32) -> f32 {
        if !self.profile {
            return 1.0;
        }
        let z0 = self.roughness.max(1.0e-4);
        let above = (y - self.ground).max(z0);
        let reference = self.reference_height.max(z0 * 1.001);
        ((above / z0).ln() / (reference / z0).ln()).clamp(0.0, 4.0)
    }

    /// The gusting wind at the reference height at `time` seconds into the
    /// run, and how much of its speed is gust. Horizontal, per the dimension.
    pub fn base(&self, dials: &WindDials, time: f32, flat: bool) -> ([f32; 3], f32) {
        let (wind, gust) = self.base_vec(dials, time, flat);
        (wind.to_array(), gust)
    }

    fn base_vec(&self, dials: &WindDials, time: f32, flat: bool) -> (Vec3, f32) {
        let storm = StormScale::of(dials.storm);
        let frequency = self.gust_frequency * storm.frequency;
        let t = time * frequency;
        // Two octaves, squared so gusts come in bursts over a lull.
        let n = 0.5
            + 0.5
                * (0.7 * gradient_noise(self.seed, 0, t)
                    + 0.3 * gradient_noise(self.seed, 1, t * 2.3));
        let gust = dials.gust.max(0.0) * storm.gust * n.clamp(0.0, 1.0).powi(2);
        let veer = self.veer * gradient_noise(self.seed, 2, t * 0.5);
        let speed = dials.speed.max(0.0) * storm.speed + gust;
        (heading_vec(dials.direction + veer, flat) * speed, gust)
    }

    /// The wind at `point`: the base wind, slowed or sped by the profile,
    /// then every zone covering it in blend order.
    pub fn sample(
        &self,
        dials: &WindDials,
        time: f32,
        point: [f32; 3],
        flat: bool,
        zones: &[WindZone],
    ) -> [f32; 3] {
        let point = Vec3::from(point);
        let (base, _) = self.base_vec(dials, time, flat);
        let height = if flat {
            1.0
        } else {
            self.height_factor(point.y)
        };
        let storm = StormScale::of(dials.storm);
        let mut wind = base * height;
        for zone in zones {
            wind = zone.apply(wind, point, time, flat, storm.turbulence);
        }
        wind.to_array()
    }

    /// The wind the clouds ride: the base wind at their altitude, no zones.
    pub fn aloft(&self, dials: &WindDials, time: f32, flat: bool) -> [f32; 3] {
        let (base, _) = self.base_vec(dials, time, flat);
        let height = if flat {
            1.0
        } else {
            self.height_factor(self.ground + self.clouds.altitude)
        };
        (base * height).to_array()
    }
}

/// No wind blows faster than this, whatever is typed: fast enough for any
/// tornado in metres, or a gale in 2D pixels.
pub const MAX_SPEED: f32 = 10_000.0;

/// The live dials blocks and scripts drive.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindDials {
    pub direction: f32,
    pub speed: f32,
    pub gust: f32,
    pub storm: f32,
}

/// What `set wind`, `set storm` and `set cloud drift` set this run, over
/// the project's own wind.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindOverrides {
    pub direction: Option<f32>,
    pub speed: Option<f32>,
    pub gust: Option<f32>,
    pub storm: Option<f32>,
    pub cloud_drift: Option<[f32; 3]>,
}

impl WindOverrides {
    /// Writes one dial, clamped as the project's own would be. A value that
    /// isn't finite is ignored.
    pub fn set(&mut self, property: WindProperty, value: f32) {
        if !value.is_finite() {
            return;
        }
        match property {
            WindProperty::Direction => self.direction = Some(value.rem_euclid(360.0)),
            WindProperty::Speed => self.speed = Some(value.clamp(0.0, MAX_SPEED)),
            WindProperty::Gust => self.gust = Some(value.clamp(0.0, MAX_SPEED)),
            WindProperty::Storm => self.storm = Some(value.clamp(0.0, 1.0)),
        }
    }

    pub fn set_cloud_drift(&mut self, drift: [f32; 3]) {
        if drift.iter().all(|v| v.is_finite()) {
            self.cloud_drift = Some(drift.map(|v| v.clamp(-MAX_SPEED, MAX_SPEED)));
        }
    }
}

/// Which dial `set wind _ to` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WindProperty {
    Direction,
    Speed,
    Gust,
    Storm,
}

impl WindProperty {
    pub fn name(self) -> &'static str {
        match self {
            WindProperty::Direction => "Direction",
            WindProperty::Speed => "Speed",
            WindProperty::Gust => "Gust",
            WindProperty::Storm => "Storm",
        }
    }

    /// Case-insensitive, so a script's `"speed"` works too.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "direction" => Some(WindProperty::Direction),
            "speed" => Some(WindProperty::Speed),
            "gust" => Some(WindProperty::Gust),
            "storm" => Some(WindProperty::Storm),
            _ => None,
        }
    }
}

/// How a storm scales the rest: at 1 the wind triples, gusts quadruple and
/// come twice as often, and turbulence triples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StormScale {
    pub speed: f32,
    pub gust: f32,
    pub frequency: f32,
    pub turbulence: f32,
}

impl StormScale {
    pub fn of(storm: f32) -> Self {
        let s = finite(storm, 0.0).clamp(0.0, 1.0);
        Self {
            speed: 1.0 + 2.0 * s,
            gust: 1.0 + 3.0 * s,
            frequency: 1.0 + s,
            turbulence: 1.0 + 2.0 * s,
        }
    }
}

/// A unit vector for a direction in degrees, clockwise from north.
pub fn heading(degrees: f32, flat: bool) -> [f32; 3] {
    heading_vec(degrees, flat).to_array()
}

fn heading_vec(degrees: f32, flat: bool) -> Vec3 {
    let r = degrees.to_radians();
    if flat {
        Vec3::new(r.sin(), r.cos(), 0.0)
    } else {
        Vec3::new(r.sin(), 0.0, -r.cos())
    }
}

/// The compass direction of a wind vector, the inverse of [`heading`].
pub fn direction_of(wind: [f32; 3], flat: bool) -> f32 {
    let [x, y, z] = wind;
    let (east, north) = if flat { (x, y) } else { (x, -z) };
    if east == 0.0 && north == 0.0 {
        return 0.0;
    }
    east.atan2(north).to_degrees().rem_euclid(360.0)
}

/// What a zone does to the wind inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WindZoneMode {
    /// Blows its own way instead: speed 0 is interior stillness.
    #[default]
    Override,
    /// Blows its own way on top of the wind: valley drift.
    Add,
    /// Spins round the actor's up axis, pulling in and lifting: a funnel.
    Swirl,
}

/// Wind a `Volume` makes inside its shape, fading across its blend distance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalWind {
    pub enabled: bool,
    pub mode: WindZoneMode,
    /// Degrees, as the global wind's direction. Override and Add.
    pub direction: f32,
    pub speed: f32,
    /// Speed round the axis. Swirl.
    pub swirl: f32,
    /// Speed towards the axis. Swirl.
    pub inflow: f32,
    /// Speed up the axis. Swirl.
    pub updraft: f32,
    /// Extra speed of the turbulence, any way at all.
    pub turbulence: f32,
    /// World units across one eddy.
    pub turbulence_scale: f32,
}

impl Default for LocalWind {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: WindZoneMode::Override,
            direction: 0.0,
            speed: 0.0,
            swirl: 20.0,
            inflow: 2.0,
            updraft: 5.0,
            turbulence: 0.0,
            turbulence_scale: 10.0,
        }
    }
}

impl LocalWind {
    pub fn normalize(&mut self) {
        let d = LocalWind::default();
        self.direction = finite(self.direction, 0.0).rem_euclid(360.0);
        self.speed = finite(self.speed, 0.0).clamp(0.0, MAX_SPEED);
        self.swirl = finite(self.swirl, d.swirl).clamp(-MAX_SPEED, MAX_SPEED);
        self.inflow = finite(self.inflow, d.inflow).clamp(-MAX_SPEED, MAX_SPEED);
        self.updraft = finite(self.updraft, d.updraft).clamp(-MAX_SPEED, MAX_SPEED);
        self.turbulence = finite(self.turbulence, 0.0).clamp(0.0, MAX_SPEED);
        self.turbulence_scale =
            finite(self.turbulence_scale, d.turbulence_scale).clamp(0.01, 1.0e5);
    }
}

/// One volume's wind as placed in the world, ready to sample.
#[derive(Debug, Clone, PartialEq)]
pub struct WindZone {
    pub spec: VolumeSpec,
    pub pose: VolumePose,
}

impl WindZone {
    /// Wind zones among `volumes`, in blend order. Takes what
    /// `volumes::placed` hands over: enabled and weighted already.
    pub fn from_volumes<'a>(
        volumes: impl IntoIterator<Item = (&'a str, VolumeSpec, VolumePose)>,
    ) -> Vec<WindZone> {
        let mut zones: Vec<(String, WindZone)> = volumes
            .into_iter()
            .filter(|(_, spec, _)| spec.wind.enabled)
            .map(|(id, mut spec, pose)| {
                spec.wind.normalize();
                (id.to_string(), WindZone { spec, pose })
            })
            .collect();
        crate::volume::blend_order(&mut zones, |(id, z)| (z.spec.priority, id.as_str()));
        zones.into_iter().map(|(_, zone)| zone).collect()
    }

    fn apply(&self, wind: Vec3, point: Vec3, time: f32, flat: bool, turbulence: f32) -> Vec3 {
        let w = self.spec.coverage(&self.pose, point.to_array(), flat)
            * self.spec.weight.clamp(0.0, 1.0);
        if w <= 0.0 {
            return wind;
        }
        let local = &self.spec.wind;
        let own = heading_vec(local.direction, flat) * local.speed;
        let mut out = match local.mode {
            WindZoneMode::Override => wind.lerp(own, w),
            WindZoneMode::Add => wind + own * w,
            WindZoneMode::Swirl => wind + self.swirl(point, flat) * w,
        };
        if local.turbulence > 0.0 {
            let eddy = eddy(point / local.turbulence_scale.max(0.01), time);
            let eddy = if flat { eddy.with_z(0.0) } else { eddy };
            out += eddy * local.turbulence * turbulence * w;
        }
        out
    }

    /// Round, in and up about the actor's up axis (its facing in 2D, out
    /// of the screen).
    fn swirl(&self, point: Vec3, flat: bool) -> Vec3 {
        let local = &self.spec.wind;
        let rotation = Quat::from_array(self.pose.rotation).normalize();
        let rotation = if rotation.is_finite() {
            rotation
        } else {
            Quat::IDENTITY
        };
        let axis = if flat { Vec3::Z } else { rotation * Vec3::Y };
        let offset = point - Vec3::from(self.pose.position);
        let radial = offset - axis * offset.dot(axis);
        let r = radial.length();
        if r < 1.0e-4 {
            return if flat {
                Vec3::ZERO
            } else {
                axis * local.updraft
            };
        }
        let out = radial / r;
        // Solid-body within a unit of the axis, so the eye isn't a spike.
        let core = r.min(1.0);
        let round = axis.cross(out) * local.swirl * core;
        let lift = if flat {
            Vec3::ZERO
        } else {
            axis * local.updraft
        };
        round - out * local.inflow * core + lift
    }
}

/// Where the clouds have drifted to, integrated a tick at a time.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CloudOffsets {
    /// How far the cloud shapes have moved.
    pub advection: [f32; 3],
    /// How far the erosion detail has moved.
    pub erosion: [f32; 3],
    /// How far planar layers have scrolled, across the ground plane (x and
    /// z in 3D, x and y in 2D).
    pub layers: [f32; 2],
}

impl CloudOffsets {
    /// Advances by `dt` seconds of world time under `aloft`, the wind at
    /// the clouds. `drift` is `set cloud drift`, or the authored drift.
    pub fn step(
        &mut self,
        clouds: &CloudDrift,
        aloft: [f32; 3],
        drift: [f32; 3],
        dt: f32,
        flat: bool,
    ) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let (aloft, drift) = (Vec3::from(aloft), Vec3::from(drift));
        let dt = dt * clouds.time_lapse.clamp(1.0, 1000.0);
        let shape = aloft * clouds.follow + drift;
        self.advection = (Vec3::from(self.advection) + shape * dt).to_array();
        self.erosion =
            (Vec3::from(self.erosion) + (shape + Vec3::from(clouds.erosion)) * dt).to_array();
        let layer = aloft * clouds.layer_scroll + drift;
        let across = if flat { layer.truncate() } else { layer.xz() };
        self.layers = (Vec2::from(self.layers) + across * dt).to_array();
    }
}

/// 1D gradient noise in -1..1, lattice at whole numbers.
pub fn gradient_noise(seed: u32, lane: u32, x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let cell = x.floor();
    let f = x - cell;
    let i = cell as i64 as u32;
    let slope = |n: u32| unit(hash(seed, n, lane)) * 2.0 - 1.0;
    let a = slope(i) * f;
    let b = slope(i.wrapping_add(1)) * (f - 1.0);
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    // Peaks at 0.5 for slopes of 1, so doubled to span -1..1.
    ((a + (b - a) * s) * 2.0).clamp(-1.0, 1.0)
}

/// A drifting eddy: three lanes of value noise over space and time, -1..1
/// each.
fn eddy(p: Vec3, time: f32) -> Vec3 {
    let q = p + Vec3::new(0.37, 0.61, 0.23) * time;
    Vec3::new(value_noise(q, 3), value_noise(q, 4), value_noise(q, 5))
}

fn value_noise(p: Vec3, lane: u32) -> f32 {
    if !p.is_finite() {
        return 0.0;
    }
    let cell = p.floor();
    let f = p - cell;
    let s = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let corner = |dx: i32, dy: i32, dz: i32| {
        let (x, y, z) = (cell.x as i32 + dx, cell.y as i32 + dy, cell.z as i32 + dz);
        let n = (x as u32).wrapping_mul(73_856_093)
            ^ (y as u32).wrapping_mul(19_349_663)
            ^ (z as u32).wrapping_mul(83_492_791);
        unit(hash(0x5EED, n, lane)) * 2.0 - 1.0
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), s.x);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), s.x);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), s.x);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), s.x);
    lerp(lerp(x00, x10, s.y), lerp(x01, x11, s.y), s.z)
}

fn hash(seed: u32, n: u32, lane: u32) -> u32 {
    // PCG, as `blockloom::hash` does it on the GPU.
    let v = seed
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(n.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(lane.wrapping_mul(0xC2B2_AE35));
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

fn unit(x: u32) -> f32 {
    (x >> 8) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::VolumeShape;

    fn base(wind: &Wind, dials: &WindDials, time: f32, flat: bool) -> (Vec3, f32) {
        let (v, gust) = wind.base(dials, time, flat);
        (Vec3::from(v), gust)
    }

    fn sample(
        wind: &Wind,
        dials: &WindDials,
        time: f32,
        point: Vec3,
        flat: bool,
        zones: &[WindZone],
    ) -> Vec3 {
        Vec3::from(wind.sample(dials, time, point.to_array(), flat, zones))
    }

    fn aloft(wind: &Wind, dials: &WindDials, time: f32, flat: bool) -> Vec3 {
        Vec3::from(wind.aloft(dials, time, flat))
    }

    fn breeze() -> Wind {
        Wind {
            direction: 90.0,
            speed: 10.0,
            ..Wind::default()
        }
    }

    fn dials(wind: &Wind) -> WindDials {
        wind.dials(&WindOverrides::default())
    }

    #[test]
    fn a_steady_wind_blows_the_way_its_heading_says() {
        let wind = Wind {
            veer: 0.0,
            ..breeze()
        };
        let (v, gust) = base(&wind, &dials(&wind), 3.0, false);
        assert_eq!(gust, 0.0);
        assert!((v - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4, "{v}");
        // North is up the screen in 2D.
        let north = Wind {
            direction: 0.0,
            ..wind.clone()
        };
        let (v, _) = base(&north, &dials(&north), 0.0, true);
        assert!((v.normalize() - Vec3::Y).length() < 1e-4, "{v}");
        assert!((direction_of([0.0, 0.0, 5.0], false) - 180.0).abs() < 1e-3);
        assert!((direction_of(heading(123.0, true), true) - 123.0).abs() < 1e-3);
    }

    #[test]
    fn the_profile_is_calm_at_the_ground_and_one_at_the_reference_height() {
        let wind = breeze();
        assert_eq!(wind.height_factor(0.0), 0.0);
        assert!((wind.height_factor(10.0) - 1.0).abs() < 1e-5);
        assert!(wind.height_factor(100.0) > 1.0);
        assert!(wind.height_factor(2.0) < wind.height_factor(5.0));
        let off = Wind {
            profile: false,
            ..breeze()
        };
        assert_eq!(off.height_factor(0.0), 1.0);
    }

    #[test]
    fn gusts_come_and_go_and_replay_from_the_seed() {
        let wind = Wind {
            gust: 8.0,
            ..breeze()
        };
        let d = dials(&wind);
        let gusts: Vec<f32> = (0..600)
            .map(|i| base(&wind, &d, i as f32 * 0.1, false).1)
            .collect();
        let (lo, hi) = gusts
            .iter()
            .fold((f32::MAX, 0.0f32), |(lo, hi), g| (lo.min(*g), hi.max(*g)));
        assert!(lo < 1.0 && hi > 3.0 && hi <= 8.0, "{lo}..{hi}");
        let again: Vec<f32> = (0..600)
            .map(|i| base(&wind, &d, i as f32 * 0.1, false).1)
            .collect();
        assert_eq!(gusts, again);
        let reseeded = Wind { seed: 9, ..wind };
        assert_ne!(base(&reseeded, &d, 12.3, false).1, gusts[123]);
    }

    #[test]
    fn a_storm_scales_everything_up() {
        let wind = Wind {
            veer: 0.0,
            ..breeze()
        };
        let calm = base(&wind, &dials(&wind), 0.0, false).0.length();
        let over = WindOverrides {
            storm: Some(1.0),
            ..WindOverrides::default()
        };
        let stormy = base(&wind, &wind.dials(&over), 0.0, false).0.length();
        assert!((stormy - calm * 3.0).abs() < 1e-3);
    }

    #[test]
    fn overrides_clamp_and_ignore_nonsense() {
        let mut over = WindOverrides::default();
        over.set(WindProperty::Direction, -90.0);
        over.set(WindProperty::Storm, 7.0);
        over.set(WindProperty::Speed, f32::NAN);
        assert_eq!(over.direction, Some(270.0));
        assert_eq!(over.storm, Some(1.0));
        assert_eq!(over.speed, None);
        assert_eq!(WindProperty::parse(" GUST"), Some(WindProperty::Gust));
        assert_eq!(WindProperty::parse("rain"), None);
        for p in [
            WindProperty::Direction,
            WindProperty::Speed,
            WindProperty::Gust,
            WindProperty::Storm,
        ] {
            assert_eq!(WindProperty::parse(p.name()), Some(p));
        }
    }

    fn zone(local: LocalWind, shape: VolumeShape) -> WindZone {
        WindZone {
            spec: VolumeSpec {
                shape,
                radius: 5.0,
                blend_distance: 2.0,
                wind: LocalWind {
                    enabled: true,
                    ..local
                },
                ..VolumeSpec::default()
            },
            pose: VolumePose::default(),
        }
    }

    #[test]
    fn a_still_zone_calms_its_inside_and_fades_across_its_blend() {
        let wind = Wind {
            veer: 0.0,
            profile: false,
            ..breeze()
        };
        let still = [zone(LocalWind::default(), VolumeShape::Box)];
        let d = dials(&wind);
        assert_eq!(
            sample(&wind, &d, 0.0, Vec3::ZERO, false, &still),
            Vec3::ZERO
        );
        let edge = sample(&wind, &d, 0.0, Vec3::new(6.0, 0.0, 0.0), false, &still);
        assert!((edge.x - 5.0).abs() < 1e-3, "{edge}");
        let outside = sample(&wind, &d, 0.0, Vec3::new(20.0, 0.0, 0.0), false, &still);
        assert!((outside.x - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_swirl_spins_round_pulls_in_and_lifts() {
        let wind = Wind::default();
        let funnel = [zone(
            LocalWind {
                mode: WindZoneMode::Swirl,
                swirl: 10.0,
                inflow: 2.0,
                updraft: 3.0,
                ..LocalWind::default()
            },
            VolumeShape::Sphere,
        )];
        let v = sample(
            &wind,
            &dials(&wind),
            0.0,
            Vec3::new(3.0, 0.0, 0.0),
            false,
            &funnel,
        );
        // Y cross +X is -Z: anticlockwise seen from above.
        assert!((v - Vec3::new(-2.0, 3.0, -10.0)).length() < 1e-3, "{v}");
    }

    #[test]
    fn turbulence_stirs_only_inside_and_replays() {
        let wind = Wind::default();
        let stirred = [zone(
            LocalWind {
                mode: WindZoneMode::Add,
                turbulence: 4.0,
                turbulence_scale: 3.0,
                ..LocalWind::default()
            },
            VolumeShape::Sphere,
        )];
        let d = dials(&wind);
        let a = sample(&wind, &d, 1.3, Vec3::new(1.2, 0.4, -0.7), false, &stirred);
        assert!(a.length() > 0.0 && a.length() <= 4.0 * 3.0_f32.sqrt() + 1e-3);
        assert_eq!(
            a,
            sample(&wind, &d, 1.3, Vec3::new(1.2, 0.4, -0.7), false, &stirred)
        );
        let far = sample(&wind, &d, 1.3, Vec3::splat(50.0), false, &stirred);
        assert_eq!(far, Vec3::ZERO);
    }

    #[test]
    fn zones_blend_lowest_priority_first() {
        let mut low = zone(
            LocalWind {
                mode: WindZoneMode::Override,
                speed: 3.0,
                ..LocalWind::default()
            },
            VolumeShape::Global,
        );
        low.spec.priority = -1.0;
        let high = zone(LocalWind::default(), VolumeShape::Global);
        let zones = WindZone::from_volumes([
            ("b", high.spec.clone(), high.pose),
            ("a", low.spec.clone(), low.pose),
        ]);
        let wind = breeze();
        assert_eq!(
            sample(&wind, &dials(&wind), 0.0, Vec3::ONE, false, &zones),
            Vec3::ZERO
        );
    }

    #[test]
    fn clouds_ride_the_wind_aloft_faster_under_time_lapse() {
        let wind = Wind {
            veer: 0.0,
            ..breeze()
        };
        let aloft = aloft(&wind, &dials(&wind), 0.0, false);
        assert!(aloft.x > 10.0, "{aloft}");
        let mut offsets = CloudOffsets::default();
        offsets.step(&wind.clouds, aloft.to_array(), [0.0; 3], 1.0, false);
        assert!((Vec3::from(offsets.advection) - aloft).length() < 1e-3);
        assert!((Vec2::from(offsets.layers) - aloft.xz()).length() < 1e-3);
        assert!((offsets.erosion[1] - 0.5).abs() < 1e-3);
        let lapse = CloudDrift {
            time_lapse: 100.0,
            ..CloudDrift::default()
        };
        let mut fast = CloudOffsets::default();
        fast.step(&lapse, aloft.to_array(), [0.0; 3], 1.0, false);
        assert!((Vec3::from(fast.advection) - aloft * 100.0).length() < 1e-2);
    }

    #[test]
    fn gradient_noise_is_smooth_and_bounded() {
        let mut last = gradient_noise(4, 0, 0.0);
        assert_eq!(last, 0.0);
        for i in 1..2000 {
            let n = gradient_noise(4, 0, i as f32 * 0.01);
            assert!((-1.0..=1.0).contains(&n));
            assert!((n - last).abs() < 0.1);
            last = n;
        }
    }

    #[test]
    fn normalize_keeps_dials_in_range_and_old_documents_load() {
        let mut wind = Wind {
            direction: 400.0,
            speed: -3.0,
            storm: 3.0,
            clouds: CloudDrift {
                time_lapse: 0.0,
                ..CloudDrift::default()
            },
            ..Wind::default()
        };
        wind.normalize();
        assert_eq!(wind.direction, 40.0);
        assert_eq!(wind.speed, 0.0);
        assert_eq!(wind.storm, 1.0);
        assert_eq!(wind.clouds.time_lapse, 1.0);
        let old: Wind = serde_json::from_str("{}").unwrap();
        assert_eq!(old, Wind::default());
    }
}
