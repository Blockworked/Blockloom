//! Soundscape rules: what the world's state does to the mix.
//!
//! [`Soundscape`] is saved on the [`crate::scene::World`] and is neutral by
//! default, so old projects sound the same. The runtime reads it every frame:
//! weather loops follow the atmosphere, ducking dips a bus under a trigger, and
//! the underwater muffle pulls buses down while the camera is below a surface.
//! This module holds the data and the maths; the runtime owns the voices.

use blockloom_schema_derive::Schema;
use serde::{Deserialize, Serialize};

use crate::sound::{SoundBus, clamp_gain};

/// The atmosphere reading a weather loop follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Schema)]
pub enum WeatherSource {
    /// Wind speed at the camera, world units per second.
    #[default]
    Wind,
    /// Rain intensity, 0-1.
    Rain,
    /// Snow intensity, 0-1.
    Snow,
    /// Storm dial, 0-1.
    Storm,
}

impl WeatherSource {
    pub const ALL: &[WeatherSource] = &[
        WeatherSource::Wind,
        WeatherSource::Rain,
        WeatherSource::Snow,
        WeatherSource::Storm,
    ];
}

/// A looping sound whose volume follows the weather: silent at `from`, at
/// `max_gain` from `to` up, linear between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct WeatherLoop {
    /// Project-relative sound path.
    pub sound: String,
    #[serde(default)]
    pub source: WeatherSource,
    #[serde(default)]
    pub from: f32,
    #[serde(default = "one")]
    pub to: f32,
    #[serde(default = "one")]
    pub max_gain: f32,
    #[serde(default)]
    pub bus: SoundBus,
}

fn one() -> f32 {
    1.0
}

impl WeatherLoop {
    /// The loop's own gain for a reading. Equal bounds make a hard switch.
    pub fn gain(&self, reading: f32) -> f32 {
        let span = self.to - self.from;
        let t = if span.abs() < f32::EPSILON {
            if reading >= self.to { 1.0 } else { 0.0 }
        } else {
            ((reading - self.from) / span).clamp(0.0, 1.0)
        };
        clamp_gain(self.max_gain) * t
    }
}

/// What makes a duck rule fire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub enum DuckTrigger {
    /// The game is paused: a pause menu quiets the soundtrack.
    Paused,
    /// A voice of this sound file is playing, such as a line of dialogue.
    Sound(String),
}

/// Dips one bus while its trigger holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct DuckRule {
    pub trigger: DuckTrigger,
    pub target: SoundBus,
    /// Share of the bus's gain removed at full duck, 0-1.
    #[serde(default = "half")]
    pub amount: f32,
    /// Seconds to reach full duck.
    #[serde(default = "quick")]
    pub attack: f32,
    /// Seconds to recover.
    #[serde(default = "slow")]
    pub release: f32,
}

fn half() -> f32 {
    0.5
}
fn quick() -> f32 {
    0.1
}
fn slow() -> f32 {
    0.5
}

impl DuckRule {
    /// Moves a 0-1 duck level one step towards where the trigger wants it.
    pub fn step(&self, level: f32, active: bool, dt: f32) -> f32 {
        ease(
            level,
            if active { 1.0 } else { 0.0 },
            dt,
            if active { self.attack } else { self.release },
        )
    }

    /// The gain factor a duck level applies to the target bus.
    pub fn factor(&self, level: f32) -> f32 {
        1.0 - self.amount.clamp(0.0, 1.0) * level.clamp(0.0, 1.0)
    }
}

/// Buses that sink while the camera is underwater.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct UnderwaterMuffle {
    /// Empty means off.
    #[serde(default)]
    pub buses: Vec<SoundBus>,
    /// Share of gain removed at full muffle, 0-1.
    #[serde(default = "muffled")]
    pub amount: f32,
    /// Seconds to go fully under or come back.
    #[serde(default = "quick")]
    pub fade: f32,
}

fn muffled() -> f32 {
    0.7
}

impl Default for UnderwaterMuffle {
    fn default() -> Self {
        Self {
            buses: Vec::new(),
            amount: muffled(),
            fade: quick(),
        }
    }
}

impl UnderwaterMuffle {
    pub fn step(&self, level: f32, underwater: bool, dt: f32) -> f32 {
        ease(level, if underwater { 1.0 } else { 0.0 }, dt, self.fade)
    }

    /// The gain factor for `bus` at a muffle level.
    pub fn factor(&self, bus: SoundBus, level: f32) -> f32 {
        if bus_is_affected(&self.buses, bus) {
            1.0 - self.amount.clamp(0.0, 1.0) * level.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

/// A master effect is heard on every routed voice.
pub fn bus_is_affected(buses: &[SoundBus], bus: SoundBus) -> bool {
    buses.contains(&SoundBus::Master) || buses.contains(&bus)
}

/// A parametric reverb: a room the dry sound is sent into.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Schema)]
pub struct ReverbSpec {
    /// How much reverb is mixed in, 0-1. Zero is dry.
    #[serde(default)]
    pub mix: f32,
    /// Room size, 0-1.
    #[serde(default = "mid")]
    pub size: f32,
    /// Seconds for the tail to fall 60 dB.
    #[serde(default = "decay_default")]
    pub decay: f32,
    /// How fast highs die in the tail, 0-1.
    #[serde(default = "mid")]
    pub damping: f32,
    /// How much the tail's delays wander, 0-1. Higher is lusher.
    #[serde(default = "excursion_default")]
    pub excursion: f32,
}

fn mid() -> f32 {
    0.5
}
fn decay_default() -> f32 {
    1.5
}
fn excursion_default() -> f32 {
    0.2
}

impl Default for ReverbSpec {
    fn default() -> Self {
        Self {
            mix: 0.0,
            size: 0.5,
            decay: 1.5,
            damping: 0.5,
            excursion: 0.2,
        }
    }
}

/// Names `set reverb to` and the zone inspector offer.
pub const REVERB_PRESETS: &[&str] = &[
    "Off",
    "Small room",
    "Hall",
    "Cave",
    "Cathedral",
    "Tunnel",
    "Underwater",
];

impl ReverbSpec {
    /// A preset by name (case-insensitive), at its own mix.
    pub fn preset(name: &str) -> Option<ReverbSpec> {
        let (mix, size, decay, damping, excursion) = match name.trim().to_lowercase().as_str() {
            "off" | "none" | "" => (0.0, 0.5, 1.5, 0.5, 0.2),
            "small room" | "room" => (0.25, 0.2, 0.6, 0.6, 0.1),
            "hall" => (0.35, 0.6, 2.2, 0.4, 0.25),
            "cave" => (0.5, 0.8, 3.5, 0.3, 0.35),
            "cathedral" => (0.55, 1.0, 5.5, 0.35, 0.3),
            "tunnel" => (0.45, 0.45, 2.8, 0.5, 0.15),
            "underwater" => (0.5, 0.6, 2.0, 0.9, 0.6),
            _ => return None,
        };
        Some(ReverbSpec {
            mix,
            size,
            decay,
            damping,
            excursion,
        })
    }

    pub fn is_off(&self) -> bool {
        self.mix <= 0.001
    }

    /// `self` moved `t` of the way to `other`.
    pub fn lerp(self, other: ReverbSpec, t: f32) -> ReverbSpec {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| a + (b - a) * t;
        ReverbSpec {
            mix: mix(self.mix, other.mix),
            size: mix(self.size, other.size),
            decay: mix(self.decay, other.decay),
            damping: mix(self.damping, other.damping),
            excursion: mix(self.excursion, other.excursion),
        }
    }

    pub fn normalize(&mut self) {
        let fix =
            |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.mix = fix(self.mix, 0.0, 1.0, 0.0);
        self.size = fix(self.size, 0.0, 1.0, 0.5);
        self.decay = fix(self.decay, 0.1, 20.0, 1.5);
        self.damping = fix(self.damping, 0.0, 1.0, 0.5);
        self.excursion = fix(self.excursion, 0.0, 1.0, 0.2);
    }
}

/// The lowpass cutoff for a muffle amount: open at 0, a dull 400 Hz at 1.
pub fn cutoff_for(amount: f32) -> f32 {
    const OPEN: f32 = 20_000.0;
    const CLOSED: f32 = 400.0;
    OPEN * (CLOSED / OPEN).powf(amount.clamp(0.0, 1.0))
}

/// How walls and corners change a positional sound on its way to the
/// listener. Only static scenery blocks: bodies and triggers never do.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Schema)]
pub struct Occlusion {
    #[serde(default)]
    pub enabled: bool,
    /// Gain kept per wall crossed, 0-1.
    #[serde(default = "wall_gain")]
    pub wall_gain: f32,
    /// Muffle added per wall crossed, 0-1.
    #[serde(default = "wall_muffle")]
    pub wall_muffle: f32,
    /// Walls past this change nothing more.
    #[serde(default = "max_walls")]
    pub max_walls: u32,
    /// Muffle when a sound is blocked but can bend round an edge, 0-1.
    /// Zero turns obstruction off.
    #[serde(default = "obstruction")]
    pub obstruction: f32,
}

fn wall_gain() -> f32 {
    0.6
}
fn wall_muffle() -> f32 {
    0.45
}
fn max_walls() -> u32 {
    3
}
fn obstruction() -> f32 {
    0.35
}

impl Default for Occlusion {
    fn default() -> Self {
        Self {
            enabled: false,
            wall_gain: wall_gain(),
            wall_muffle: wall_muffle(),
            max_walls: max_walls(),
            obstruction: obstruction(),
        }
    }
}

/// What a source's path to the listener does to it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PathEffect {
    pub gain: f32,
    /// 0-1, through [`cutoff_for`].
    pub muffle: f32,
}

impl Occlusion {
    /// `walls` on the direct line; `bends` when a detour round the edge is
    /// clear.
    pub fn effect(&self, walls: u32, bends: bool) -> PathEffect {
        if !self.enabled || walls == 0 {
            return PathEffect {
                gain: 1.0,
                muffle: 0.0,
            };
        }
        if bends && self.obstruction > 0.0 {
            return PathEffect {
                gain: 1.0,
                muffle: self.obstruction.clamp(0.0, 1.0),
            };
        }
        let n = walls.min(self.max_walls.max(1)) as i32;
        PathEffect {
            gain: self.wall_gain.clamp(0.0, 1.0).powi(n),
            muffle: (self.wall_muffle.clamp(0.0, 1.0) * n as f32).min(1.0),
        }
    }

    pub fn normalize(&mut self) {
        let unit = |v: f32, d: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
        self.wall_gain = unit(self.wall_gain, wall_gain());
        self.wall_muffle = unit(self.wall_muffle, wall_muffle());
        self.obstruction = unit(self.obstruction, obstruction());
        self.max_walls = self.max_walls.clamp(1, 16);
    }
}

/// A region that changes how the listener hears: its own reverb and a
/// muffle, blended in by how far the listener is inside. Attached to an
/// actor as the `SoundZone` component; the actor's place is the zone's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct SoundZoneSpec {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub shape: crate::volume::VolumeShape,
    #[serde(default = "extents")]
    pub half_extents: [f32; 3],
    #[serde(default = "radius")]
    pub radius: f32,
    /// Higher wins where zones overlap.
    #[serde(default)]
    pub priority: f32,
    /// Distance outside the shape over which it fades in.
    #[serde(default = "blend")]
    pub blend_distance: f32,
    #[serde(default)]
    pub reverb: ReverbSpec,
    /// Lowpass and loss inside, 0-1.
    #[serde(default)]
    pub muffle: f32,
    /// Buses the zone's reverb and muffle apply to.
    #[serde(default = "default_buses")]
    pub buses: Vec<SoundBus>,
    /// A recorded room (a WAV impulse response) to convolve with instead of
    /// the parametric tail. Empty uses the parametric reverb.
    #[serde(default)]
    pub impulse: String,
}

fn yes() -> bool {
    true
}
fn extents() -> [f32; 3] {
    [5.0; 3]
}
fn radius() -> f32 {
    5.0
}
fn blend() -> f32 {
    1.0
}
fn default_buses() -> Vec<SoundBus> {
    vec![SoundBus::Sfx]
}

impl Default for SoundZoneSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: crate::volume::VolumeShape::Box,
            half_extents: extents(),
            radius: radius(),
            priority: 0.0,
            blend_distance: blend(),
            reverb: ReverbSpec::preset("Hall").unwrap_or_default(),
            muffle: 0.0,
            buses: default_buses(),
            impulse: String::new(),
        }
    }
}

impl SoundZoneSpec {
    /// 1 inside, easing to 0 across the blend distance. Same shape maths as
    /// an environment volume.
    pub fn coverage(&self, pose: &crate::volume::VolumePose, point: [f32; 3], flat: bool) -> f32 {
        let shape = crate::volume::VolumeSpec {
            shape: self.shape,
            half_extents: self.half_extents,
            radius: self.radius,
            blend_distance: self.blend_distance,
            ..crate::volume::VolumeSpec::default()
        };
        shape.coverage(pose, point, flat)
    }

    pub fn normalize(&mut self) {
        self.reverb.normalize();
        self.muffle = if self.muffle.is_finite() {
            self.muffle.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.impulse = self.impulse.trim().to_string();
        self.blend_distance = self.blend_distance.max(0.0);
        self.radius = self.radius.max(0.0);
        for h in &mut self.half_extents {
            *h = h.max(0.0);
        }
        self.buses.dedup();
    }
}

/// The listener's blended acoustics: the project's ambience with every zone
/// laid over it, lowest priority first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Acoustics {
    pub reverb: ReverbSpec,
    pub muffle: f32,
    /// The impulse response in force, empty for the parametric reverb.
    pub impulse: String,
}

impl Acoustics {
    /// Applies one zone to one bus. Call in ascending priority order.
    pub fn apply_zone(&mut self, bus: SoundBus, weight: f32, zone: &SoundZoneSpec) {
        if !bus_is_affected(&zone.buses, bus) {
            return;
        }
        if weight >= 0.5 {
            self.impulse.clone_from(&zone.impulse);
        }
        self.reverb = self.reverb.lerp(zone.reverb, weight);
        self.muffle += (zone.muffle - self.muffle) * weight.clamp(0.0, 1.0);
    }
}

/// The saved soundscape rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, Schema)]
pub struct Soundscape {
    #[serde(default)]
    pub weather: Vec<WeatherLoop>,
    #[serde(default)]
    pub ducking: Vec<DuckRule>,
    #[serde(default)]
    pub underwater: UnderwaterMuffle,
    /// The project's own reverb, where no zone says otherwise.
    #[serde(default)]
    pub reverb: ReverbSpec,
    /// A WAV impulse response for the project reverb; empty is parametric.
    #[serde(default)]
    pub reverb_impulse: String,
    /// Buses the project reverb applies to.
    #[serde(default = "default_buses")]
    pub reverb_buses: Vec<SoundBus>,
    #[serde(default)]
    pub occlusion: Occlusion,
}

impl Soundscape {
    pub fn is_empty(&self) -> bool {
        self.weather.is_empty()
            && self.ducking.is_empty()
            && self.underwater.buses.is_empty()
            && self.reverb.is_off()
            && self.reverb_impulse.is_empty()
            && !self.occlusion.enabled
    }

    /// Cleans asset paths and keeps unfinished weather rows editable.
    pub fn normalize(&mut self) {
        for w in &mut self.weather {
            w.sound = crate::sound::normalize_sound(&w.sound);
            if !w.from.is_finite() {
                w.from = 0.0;
            }
            if !w.to.is_finite() {
                w.to = 1.0;
            }
            w.max_gain = if w.max_gain.is_finite() {
                clamp_gain(w.max_gain)
            } else {
                1.0
            };
        }
        for rule in &mut self.ducking {
            if let DuckTrigger::Sound(path) = &mut rule.trigger {
                *path = crate::sound::normalize_sound(path);
            }
            rule.amount = if rule.amount.is_finite() {
                rule.amount.clamp(0.0, 1.0)
            } else {
                0.5
            };
            rule.attack = rule.attack.max(0.0);
            rule.release = rule.release.max(0.0);
        }
        self.underwater.amount = if self.underwater.amount.is_finite() {
            self.underwater.amount.clamp(0.0, 1.0)
        } else {
            muffled()
        };
        self.underwater.fade = self.underwater.fade.max(0.0);
        self.reverb.normalize();
        self.reverb_impulse = self.reverb_impulse.trim().to_string();
        self.occlusion.normalize();
    }
}

/// Moves `level` towards `target` so a full 0-1 swing takes `seconds`.
fn ease(level: f32, target: f32, dt: f32, seconds: f32) -> f32 {
    if seconds <= f32::EPSILON {
        return target;
    }
    let step = dt.max(0.0) / seconds;
    if target > level {
        (level + step).min(target)
    } else {
        (level - step).max(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wind() -> WeatherLoop {
        WeatherLoop {
            sound: "assets/sounds/wind.ogg".into(),
            source: WeatherSource::Wind,
            from: 2.0,
            to: 10.0,
            max_gain: 0.8,
            bus: SoundBus::Sfx,
        }
    }

    #[test]
    fn weather_gain_ramps_between_bounds() {
        let l = wind();
        assert_eq!(l.gain(0.0), 0.0);
        assert!((l.gain(6.0) - 0.4).abs() < 1e-6);
        assert!((l.gain(50.0) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn equal_bounds_switch_hard() {
        let l = WeatherLoop {
            from: 1.0,
            to: 1.0,
            ..wind()
        };
        assert_eq!(l.gain(0.5), 0.0);
        assert!((l.gain(1.0) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn ducking_attacks_then_releases() {
        let rule = DuckRule {
            trigger: DuckTrigger::Paused,
            target: SoundBus::Music,
            amount: 0.6,
            attack: 0.2,
            release: 1.0,
        };
        let mut level = 0.0;
        for _ in 0..10 {
            level = rule.step(level, true, 0.02);
        }
        assert!((level - 1.0).abs() < 1e-5);
        assert!((rule.factor(level) - 0.4).abs() < 1e-6);
        level = rule.step(level, false, 0.5);
        assert!((level - 0.5).abs() < 1e-6);
    }

    #[test]
    fn muffle_only_touches_its_buses() {
        let m = UnderwaterMuffle {
            buses: vec![SoundBus::Sfx],
            amount: 0.5,
            fade: 0.0,
        };
        assert_eq!(m.step(0.0, true, 0.016), 1.0);
        assert_eq!(m.factor(SoundBus::Sfx, 1.0), 0.5);
        assert_eq!(m.factor(SoundBus::Music, 1.0), 1.0);
    }

    #[test]
    fn presets_resolve_by_name() {
        assert!(ReverbSpec::preset("Cave").is_some_and(|r| r.decay > 3.0));
        assert!(ReverbSpec::preset("nowhere").is_none());
        for name in REVERB_PRESETS {
            assert!(ReverbSpec::preset(name).is_some(), "{name}");
        }
        assert!(ReverbSpec::preset("off").unwrap().is_off());
    }

    #[test]
    fn walls_cost_gain_and_bends_only_muffle() {
        let o = Occlusion {
            enabled: true,
            ..Occlusion::default()
        };
        assert_eq!(o.effect(0, false).gain, 1.0);
        let two = o.effect(2, false);
        assert!((two.gain - 0.36).abs() < 1e-6);
        assert!((two.muffle - 0.9).abs() < 1e-6);
        let bent = o.effect(2, true);
        assert_eq!(bent.gain, 1.0);
        assert!((bent.muffle - 0.35).abs() < 1e-6);
        assert_eq!(o.effect(9, false).gain, o.effect(3, false).gain);
        assert!(cutoff_for(0.0) > cutoff_for(0.5) && cutoff_for(0.5) > cutoff_for(1.0));
    }

    #[test]
    fn zones_blend_over_the_ambience_only_on_their_buses() {
        let cave = SoundZoneSpec {
            reverb: ReverbSpec::preset("Cave").unwrap(),
            muffle: 0.4,
            ..SoundZoneSpec::default()
        };
        let mut room = Acoustics::default();
        room.apply_zone(SoundBus::Music, 1.0, &cave);
        assert_eq!(room, Acoustics::default());
        room.apply_zone(SoundBus::Sfx, 0.5, &cave);
        assert!((room.muffle - 0.2).abs() < 1e-6);
        room.apply_zone(SoundBus::Sfx, 1.0, &cave);
        assert_eq!(room.reverb, cave.reverb);
        assert!((room.muffle - 0.4).abs() < 1e-6);
    }

    #[test]
    fn an_old_document_gets_the_neutral_soundscape() {
        let s: Soundscape = serde_json::from_str("{}").unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn normalize_preserves_unfinished_loops_and_clamps() {
        let mut s = Soundscape {
            weather: vec![
                WeatherLoop {
                    sound: "  ".into(),
                    ..wind()
                },
                wind(),
            ],
            ducking: vec![DuckRule {
                trigger: DuckTrigger::Paused,
                target: SoundBus::Music,
                amount: 4.0,
                attack: -1.0,
                release: 0.2,
            }],
            underwater: UnderwaterMuffle::default(),
            ..Soundscape::default()
        };
        s.normalize();
        assert_eq!(s.weather.len(), 2);
        assert!(s.weather[0].sound.is_empty());
        assert_eq!(s.ducking[0].amount, 1.0);
        assert_eq!(s.ducking[0].attack, 0.0);
    }
}
