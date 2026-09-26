//! Water: the `Water` component (an ocean, a lake or a river) and the one
//! wave model every reader shares.
//!
//! The surface is a sum of 8-12 Gerstner waves ([`WaveSet`]) built from the
//! spec's seed, so a replay moves the same water. [`WaterBody`] is one body
//! resolved for a tick: where it is, how far it reaches and its waves with
//! the live sea state laid on. The height reporter, `is underwater?`,
//! buoyancy and splashes all sample that on the CPU, and `shaders/water.wesl`
//! (`blockloom::water`) displaces the drawn surface with the same sums from
//! the same numbers, so change the two together.
//!
//! Finer ripples are Phillips-spectrum detail waves ([`DetailWave`]) that
//! only bend the shading normal: nothing floats on them.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Most Gerstner waves one body sums. `MAX_WAVES` in `water.wesl`.
pub const MAX_WAVES: usize = 12;
/// Phillips detail waves, normals only. `DETAIL_WAVES` in `water.wesl`.
pub const DETAIL_WAVES: usize = 16;
/// Ripples a surface draws at once. `MAX_RIPPLES` in `water.wesl`.
pub const MAX_RIPPLES: usize = 16;
/// How far an ocean reaches from the camera, in world units.
pub const OCEAN_REACH: f32 = 20_000.0;
/// Seconds the sea takes to mostly follow a change in the wind.
pub const SEA_SETTLE: f32 = 8.0;
/// Wind speed, world units a second, the authored amplitude stands for.
const REFERENCE_WIND: f32 = 8.0;

/// What kind of water an actor is. The kind only decides its reach and the
/// defaults an editor offers; every body has the same waves, foam and color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum WaterKind {
    /// Unbounded: the surface follows the camera out to the horizon.
    Ocean,
    /// A rectangle `size` across, centred on the actor.
    #[default]
    Lake,
    /// A lake with a current: `flow` carries the surface, the foam and
    /// anything floating on it.
    River,
}

/// The Gerstner waves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Waves {
    /// Crest height of the longest wave, world units.
    pub amplitude: f32,
    /// Length of the longest wave, world units. The rest shorten from it.
    pub wavelength: f32,
    /// 0-1: how sharp the crests are. 1 is as sharp as a Gerstner wave gets
    /// before it folds over itself.
    pub steepness: f32,
    /// 0-1: how much of the height the short waves carry. 0 is a glassy
    /// swell, 1 a wind-whipped chop.
    pub chop: f32,
    /// How many waves sum, 8 to 12.
    pub count: u32,
    /// Degrees the waves travel towards, clockwise from north, when they
    /// don't follow the wind.
    pub direction: f32,
    /// Degrees either side of that the waves scatter.
    pub spread: f32,
    /// Take the direction from the project's wind instead.
    pub follow_wind: bool,
    /// 0-1: how much the live wind raises and calms the waves.
    pub wind: f32,
    /// Kilometres of open water the wind blows over. A long fetch lets the
    /// same wind build bigger waves; a pond stays small in a gale.
    pub fetch: f32,
    /// Plays the waves faster or slower than real water.
    pub speed: f32,
    pub seed: u32,
}

impl Default for Waves {
    fn default() -> Self {
        Self {
            amplitude: 0.25,
            wavelength: 12.0,
            steepness: 0.5,
            chop: 0.4,
            count: 10,
            direction: 45.0,
            spread: 40.0,
            follow_wind: true,
            wind: 0.5,
            fetch: 2.0,
            speed: 1.0,
            seed: 1,
        }
    }
}

/// The ripples the shading sees but nothing floats on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Detail {
    /// How strongly the ripples bend the light, 0-2.
    pub strength: f32,
    /// Length of the longest ripple, world units.
    pub scale: f32,
}

impl Default for Detail {
    fn default() -> Self {
        Self {
            strength: 1.0,
            scale: 3.0,
        }
    }
}

/// What the water looks like on the way through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WaterLook {
    /// The tint of a shallow, lit edge.
    pub shallow: String,
    /// The color deep water fades to.
    pub deep: String,
    /// World units light travels into the water before most of it is gone
    /// (Beer's law). Short is murky, long is clear.
    pub absorption: f32,
    /// 0-1: how much of what is behind the surface shows through at all.
    pub clarity: f32,
    /// How far the waves bend what is seen through them, 0-1.
    pub refraction: f32,
    /// How rough the surface is to reflections and the sun's glint, 0-1.
    pub roughness: f32,
    /// How bright the sun's glint is, 0-4.
    pub glint: f32,
}

impl Default for WaterLook {
    fn default() -> Self {
        Self {
            shallow: "#3AB3A6".to_string(),
            deep: "#0B2E4A".to_string(),
            absorption: 6.0,
            clarity: 0.85,
            refraction: 0.3,
            roughness: 0.06,
            glint: 1.0,
        }
    }
}

/// White water.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Foam {
    /// 0-2: how much foam there is. 0 turns it off.
    pub amount: f32,
    pub color: String,
    /// World units of depth over which foam fades in against a shore or
    /// anything standing in the water.
    pub shore: f32,
    /// 0-1: how pinched a crest has to be before it foams. Lower foams sooner.
    pub crest: f32,
    /// World units across a patch of the noise that breaks foam up.
    pub scale: f32,
    /// How fast foam drifts along with the current, world units a second,
    /// on top of the flow.
    pub drift: f32,
}

impl Default for Foam {
    fn default() -> Self {
        Self {
            amount: 1.0,
            color: "#F2F6F8".to_string(),
            shore: 0.6,
            crest: 0.45,
            scale: 2.5,
            drift: 0.2,
        }
    }
}

/// Where reflections come from, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReflectionMode {
    /// Screen-space reflections, a live probe where they miss, then the sky.
    #[default]
    Auto,
    /// A live probe captured at the surface, then the sky.
    Probe,
    /// Screen-space reflections, then the sky.
    ScreenSpace,
    /// The sky's color alone. Cheapest.
    Sky,
}

impl ReflectionMode {
    pub fn screen_space(self) -> bool {
        matches!(self, Self::Auto | Self::ScreenSpace)
    }

    pub fn probe(self) -> bool {
        matches!(self, Self::Auto | Self::Probe)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reflections {
    pub mode: ReflectionMode,
    /// Texels across each face of the surface probe.
    pub probe_resolution: u32,
    /// Frames between refreshes of the probe. Higher is cheaper and staler.
    pub probe_refresh: u32,
    /// 0-1: how strongly reflections show at all.
    pub strength: f32,
}

impl Default for Reflections {
    fn default() -> Self {
        Self {
            mode: ReflectionMode::Auto,
            probe_resolution: 128,
            probe_refresh: 30,
            strength: 1.0,
        }
    }
}

/// What the camera sees once it is below the surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Underwater {
    /// The color the water fogs towards.
    pub fog: String,
    /// World units until the view is mostly fog.
    pub distance: f32,
    /// How bright the dancing caustic light is on what is under the
    /// surface, 0-4. 0 turns it off.
    pub caustics: f32,
    /// World units across one caustic cell.
    pub caustics_scale: f32,
    /// A grayscale image to use for the caustics instead of the built-in
    /// pattern. Empty for the built-in one.
    pub caustics_texture: String,
}

impl Default for Underwater {
    fn default() -> Self {
        Self {
            fog: "#0F4C5C".to_string(),
            distance: 18.0,
            caustics: 1.0,
            caustics_scale: 3.0,
            caustics_texture: String::new(),
        }
    }
}

/// What happens when something crosses the surface fast enough.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Splash {
    /// World units a second through the surface below which nothing splashes.
    pub min_speed: f32,
    /// Droplets thrown by a splash at `min_speed`; faster throws more.
    pub particles: u32,
    /// A ring spreading over the surface.
    pub ripples: bool,
    /// A sound asset played where it happened. Empty for silence.
    pub sound: String,
}

impl Default for Splash {
    fn default() -> Self {
        Self {
            min_speed: 1.5,
            particles: 16,
            ripples: true,
            sound: String::new(),
        }
    }
}

/// An ocean, a lake or a river on an actor. The actor's `Place` is the
/// surface's centre at rest; its yaw turns the rectangle and the current.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WaterSpec {
    pub kind: WaterKind,
    /// World units across x and z (ignored by an ocean). A 2D body reads the
    /// first as its width.
    pub size: [f32; 2],
    /// World units from the surface at rest to the bottom. Below that is
    /// not underwater.
    pub depth: f32,
    pub waves: Waves,
    pub detail: Detail,
    pub look: WaterLook,
    pub foam: Foam,
    pub reflections: Reflections,
    /// The current in the actor's own frame, world units a second.
    pub flow: [f32; 2],
    pub underwater: Underwater,
    pub splash: Splash,
}

impl Default for WaterSpec {
    fn default() -> Self {
        Self {
            kind: WaterKind::Lake,
            size: [40.0, 40.0],
            depth: 8.0,
            waves: Waves::default(),
            detail: Detail::default(),
            look: WaterLook::default(),
            foam: Foam::default(),
            reflections: Reflections::default(),
            flow: [0.0, 0.0],
            underwater: Underwater::default(),
            splash: Splash::default(),
        }
    }
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

impl WaterSpec {
    /// Clamps every dial into what the model and the shader can take. What
    /// the editor and the loader run on every write.
    pub fn normalize(&mut self) {
        let d = WaterSpec::default();
        for (value, fallback) in self.size.iter_mut().zip(d.size) {
            *value = finite(*value, fallback).max(0.01);
        }
        self.depth = finite(self.depth, d.depth).max(0.0);
        let w = &mut self.waves;
        w.amplitude = finite(w.amplitude, d.waves.amplitude).max(0.0);
        w.wavelength = finite(w.wavelength, d.waves.wavelength).max(0.01);
        w.steepness = finite(w.steepness, d.waves.steepness).clamp(0.0, 1.0);
        w.chop = finite(w.chop, d.waves.chop).clamp(0.0, 1.0);
        w.count = w.count.clamp(1, MAX_WAVES as u32);
        w.direction = finite(w.direction, 0.0).rem_euclid(360.0);
        w.spread = finite(w.spread, d.waves.spread).clamp(0.0, 180.0);
        w.wind = finite(w.wind, d.waves.wind).clamp(0.0, 1.0);
        w.fetch = finite(w.fetch, d.waves.fetch).clamp(0.01, 5000.0);
        w.speed = finite(w.speed, 1.0).clamp(0.0, 10.0);
        self.detail.strength = finite(self.detail.strength, 1.0).clamp(0.0, 2.0);
        self.detail.scale = finite(self.detail.scale, d.detail.scale).max(0.01);
        let l = &mut self.look;
        l.absorption = finite(l.absorption, d.look.absorption).max(0.01);
        l.clarity = finite(l.clarity, d.look.clarity).clamp(0.0, 1.0);
        l.refraction = finite(l.refraction, d.look.refraction).clamp(0.0, 1.0);
        l.roughness = finite(l.roughness, d.look.roughness).clamp(0.02, 1.0);
        l.glint = finite(l.glint, 1.0).clamp(0.0, 4.0);
        let f = &mut self.foam;
        f.amount = finite(f.amount, 1.0).clamp(0.0, 2.0);
        f.shore = finite(f.shore, d.foam.shore).max(0.0);
        f.crest = finite(f.crest, d.foam.crest).clamp(0.0, 1.0);
        f.scale = finite(f.scale, d.foam.scale).max(0.01);
        f.drift = finite(f.drift, 0.0);
        let r = &mut self.reflections;
        r.probe_resolution = r.probe_resolution.clamp(32, 1024);
        r.probe_refresh = r.probe_refresh.clamp(1, 600);
        r.strength = finite(r.strength, 1.0).clamp(0.0, 1.0);
        self.flow = self.flow.map(|v| finite(v, 0.0));
        let u = &mut self.underwater;
        u.distance = finite(u.distance, d.underwater.distance).max(0.01);
        u.caustics = finite(u.caustics, 1.0).clamp(0.0, 4.0);
        u.caustics_scale = finite(u.caustics_scale, d.underwater.caustics_scale).max(0.01);
        let s = &mut self.splash;
        s.min_speed = finite(s.min_speed, d.splash.min_speed).max(0.0);
        s.particles = s.particles.min(128);
    }

    /// The same water in a 2D project's pixels: a pool 800 wide and 300 deep
    /// with waves a few pixels tall.
    pub fn flat() -> Self {
        let mut spec = Self {
            size: [800.0, 300.0],
            depth: 300.0,
            ..Self::default()
        };
        spec.waves.amplitude = 6.0;
        spec.waves.wavelength = 220.0;
        spec.waves.direction = 90.0;
        spec.waves.spread = 20.0;
        spec.detail.scale = 60.0;
        spec.look.absorption = 220.0;
        spec.foam.shore = 12.0;
        spec.foam.scale = 40.0;
        spec.underwater.distance = 600.0;
        spec.underwater.caustics_scale = 60.0;
        spec.splash.min_speed = 60.0;
        spec
    }
}

// ─── Blocks ────────────────────────────────────────────────────────────────

/// Which dial `set water _ to` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WaterProperty {
    /// The surface's height at rest, in world units.
    Level,
    /// 0-1, as [`Waves::chop`].
    Chop,
    /// 0-2, as [`Foam::amount`].
    Foam,
}

impl WaterProperty {
    pub fn name(self) -> &'static str {
        match self {
            Self::Level => "Level",
            Self::Chop => "Chop",
            Self::Foam => "Foam",
        }
    }

    /// Case-insensitive, so a script's `"level"` works too.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "level" | "height" => Some(Self::Level),
            "chop" | "choppiness" => Some(Self::Chop),
            "foam" => Some(Self::Foam),
            _ => None,
        }
    }
}

/// One body's (or every body's) dials for the run.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WaterDials {
    pub level: Option<f32>,
    pub chop: Option<f32>,
    pub foam: Option<f32>,
}

impl WaterDials {
    fn set(&mut self, property: WaterProperty, value: f32) {
        match property {
            WaterProperty::Level => self.level = Some(value),
            WaterProperty::Chop => self.chop = Some(value.clamp(0.0, 1.0)),
            WaterProperty::Foam => self.foam = Some(value.clamp(0.0, 2.0)),
        }
    }

    /// `self` where it says something, `under` where it doesn't.
    fn over(self, under: WaterDials) -> WaterDials {
        WaterDials {
            level: self.level.or(under.level),
            chop: self.chop.or(under.chop),
            foam: self.foam.or(under.foam),
        }
    }
}

/// What `set water` set this run. A block run by a water actor moves that
/// body alone; from anyone else it moves every body.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterOverrides {
    pub all: WaterDials,
    pub bodies: std::collections::HashMap<String, WaterDials>,
}

impl WaterOverrides {
    /// `body` is the water actor the block ran on, or `None` for all of
    /// them. A value that isn't finite is ignored.
    pub fn set(&mut self, body: Option<&str>, property: WaterProperty, value: f32) {
        if !value.is_finite() {
            return;
        }
        match body {
            Some(id) => self
                .bodies
                .entry(id.to_string())
                .or_default()
                .set(property, value),
            None => {
                self.all.set(property, value);
                // A later "all" beats an earlier "just this one".
                for dials in self.bodies.values_mut() {
                    dials.set(property, value);
                }
            }
        }
    }

    pub fn for_body(&self, id: &str) -> WaterDials {
        self.bodies
            .get(id)
            .copied()
            .unwrap_or_default()
            .over(self.all)
    }
}

// ─── The wave model ────────────────────────────────────────────────────────

/// One Gerstner wave, as built from the spec. Amplitudes are before the sea
/// state and chop are laid on; [`WaveSet::resolve`] does that.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wave {
    /// Unit direction of travel in world xz.
    pub dir: [f32; 2],
    /// Wavenumber, radians per world unit.
    pub k: f32,
    /// Radians a second.
    pub omega: f32,
    pub phase: f32,
    /// Crest height with a calm reference sea and full chop.
    pub amplitude: f32,
    /// 0 for the longest wave, 1 for the shortest: how much chop moves it.
    pub short: f32,
}

/// One Phillips detail wave: a direction, a wavenumber and a phase. How
/// much it tilts the normal follows the live wind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailWave {
    pub dir: [f32; 2],
    pub k: f32,
    pub omega: f32,
    pub phase: f32,
}

/// A resolved wave: ready to sum. `water.wesl` packs one as two vec4s.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LiveWave {
    pub dir: [f32; 2],
    pub k: f32,
    pub omega: f32,
    pub phase: f32,
    /// Vertical amplitude.
    pub amplitude: f32,
    /// Horizontal (Gerstner) amplitude.
    pub horizontal: f32,
}

/// A resolved detail wave: its slope rather than its height.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LiveDetail {
    pub dir: [f32; 2],
    pub k: f32,
    pub omega: f32,
    pub phase: f32,
    pub slope: f32,
}

/// Every wave one body sums, built once per spec.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveSet {
    pub waves: Vec<Wave>,
    pub detail: Vec<DetailWave>,
    /// Degrees the waves travel towards, clockwise from north.
    pub heading: f32,
}

/// Splitmix64, one step: a deterministic stream for building waves.
struct Stream(u64);

impl Stream {
    fn new(seed: u32, lane: u32) -> Self {
        Stream((seed as u64) << 32 ^ lane as u64 ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    /// -1 to 1.
    fn signed(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }
}

/// A unit vector in world xz for a heading in degrees clockwise from north
/// (-Z), the way the wind measures it. In 2D only its x survives.
pub fn heading_dir(degrees: f32) -> [f32; 2] {
    let r = degrees.to_radians();
    [r.sin(), -r.cos()]
}

impl WaveSet {
    /// Builds the waves for a spec. `wind_heading` is the project's wind
    /// direction, used when the waves follow it; `gravity` is the project's
    /// pull (world units a second squared), which sets how fast a wave of a
    /// given length travels (deep water: ω² = g k).
    pub fn build(spec: &WaterSpec, wind_heading: f32, gravity: f32) -> WaveSet {
        let w = &spec.waves;
        let heading = if w.follow_wind {
            wind_heading
        } else {
            w.direction
        };
        let g = gravity.abs().max(0.01);
        let count = w.count.clamp(1, MAX_WAVES as u32) as usize;
        let mut random = Stream::new(w.seed, 0);
        let mut waves = Vec::with_capacity(count);
        for i in 0..count {
            let t = if count > 1 {
                i as f32 / (count - 1) as f32
            } else {
                0.0
            };
            // Lengths fall geometrically to a twelfth of the longest.
            let length = w.wavelength * (1.0f32 / 12.0).powf(t) * (1.0 + 0.12 * random.signed());
            let k = std::f32::consts::TAU / length.max(0.001);
            // The first wave carries the heading exactly; the rest scatter.
            let turn = if i == 0 {
                0.0
            } else {
                w.spread * random.signed()
            };
            // Constant steepness across the spectrum: height goes with length.
            let amplitude = w.amplitude * (length / w.wavelength) * (0.8 + 0.4 * random.next());
            waves.push(Wave {
                dir: heading_dir(heading + turn),
                k,
                omega: (g * k).sqrt() * w.speed,
                phase: random.next() * std::f32::consts::TAU,
                amplitude,
                short: t,
            });
        }
        let mut random = Stream::new(w.seed, 1);
        let detail = (0..DETAIL_WAVES)
            .map(|i| {
                let t = i as f32 / (DETAIL_WAVES - 1) as f32;
                let length =
                    spec.detail.scale * (1.0f32 / 16.0).powf(t) * (1.0 + 0.2 * random.signed());
                let k = std::f32::consts::TAU / length.max(0.001);
                DetailWave {
                    dir: heading_dir(heading + 90.0 * random.signed()),
                    k,
                    omega: (g * k).sqrt() * w.speed,
                    phase: random.next() * std::f32::consts::TAU,
                }
            })
            .collect();
        WaveSet {
            waves,
            detail,
            heading,
        }
    }

    /// The same waves for a 2D body: each runs straight left or right,
    /// whichever way its heading leans, so a north wind still makes waves.
    pub fn flattened(mut self) -> WaveSet {
        let side = |dir: [f32; 2]| [if dir[0] < 0.0 { -1.0 } else { 1.0 }, 0.0];
        for wave in &mut self.waves {
            wave.dir = side(wave.dir);
        }
        for wave in &mut self.detail {
            wave.dir = side(wave.dir);
        }
        self
    }

    /// The waves with the sea state `sea` (1 is the authored height) and
    /// `chop` laid on. Horizontal amplitudes shrink together when the sum
    /// would fold a crest over itself.
    pub fn resolve(&self, sea: f32, chop: f32, steepness: f32) -> Vec<LiveWave> {
        let chop = chop.clamp(0.0, 1.0);
        let heights: Vec<f32> = self
            .waves
            .iter()
            .map(|wave| {
                let short_weight = 0.15 + 0.85 * chop;
                wave.amplitude * sea.max(0.0) * (1.0 + (short_weight - 1.0) * wave.short)
            })
            .collect();
        let slope: f32 = self
            .waves
            .iter()
            .zip(&heights)
            .map(|(wave, a)| wave.k * a)
            .sum();
        let fold = slope.max(1.0);
        self.waves
            .iter()
            .zip(heights)
            .map(|(wave, amplitude)| LiveWave {
                dir: wave.dir,
                k: wave.k,
                omega: wave.omega,
                phase: wave.phase,
                amplitude,
                horizontal: steepness.clamp(0.0, 1.0) * amplitude / fold,
            })
            .collect()
    }

    /// The detail waves' slopes for a wind of `wind_speed` world units a
    /// second, `gravity` as in [`WaveSet::build`]. Phillips: long ripples
    /// need a strong wind, and ripples crossing it are weak.
    pub fn resolve_detail(
        &self,
        strength: f32,
        chop: f32,
        wind_speed: f32,
        gravity: f32,
    ) -> Vec<LiveDetail> {
        let g = gravity.abs().max(0.01);
        let wind = wind_speed.max(0.5);
        let reach = wind * wind / g;
        let along = Vec2::from(heading_dir(self.heading));
        let raw: Vec<f32> = self
            .detail
            .iter()
            .map(|wave| {
                let kl = wave.k * reach;
                let facing = Vec2::from(wave.dir).dot(along);
                let p = (-1.0 / (kl * kl).max(1e-6)).exp() / wave.k.powi(4)
                    * (0.15 + 0.85 * facing * facing);
                // Slope is height times wavenumber.
                p.sqrt() * wave.k
            })
            .collect();
        let total: f32 = raw.iter().sum::<f32>().max(1e-12);
        let budget = strength.max(0.0) * 0.35 * (0.3 + 0.7 * chop.clamp(0.0, 1.0));
        self.detail
            .iter()
            .zip(raw)
            .map(|(wave, r)| LiveDetail {
                dir: wave.dir,
                k: wave.k,
                omega: wave.omega,
                phase: wave.phase,
                slope: budget * r / total,
            })
            .collect()
    }
}

/// Significant wave height (world units) a wind of `wind` world units a
/// second raises over `fetch_km` of open water: the fetch-limited JONSWAP
/// fit, capped at the fully developed Pierson-Moskowitz sea.
pub fn significant_height(wind: f32, fetch_km: f32, gravity: f32) -> f32 {
    let g = gravity.abs().max(0.01);
    let u = wind.max(0.0);
    let limited = 0.0016 * u * (fetch_km.max(0.0) * 1000.0).sqrt() * (9.81 / g).sqrt();
    let developed = 0.21 * u * u / g;
    limited.min(developed)
}

/// How high the sea stands against the authored amplitude, for a wind of
/// `wind` world units a second: 1 at the reference wind, following the
/// fetch-limited sea above and below it by `influence` (the spec's
/// [`Waves::wind`]).
pub fn sea_target(wind: f32, fetch_km: f32, influence: f32, gravity: f32) -> f32 {
    let influence = influence.clamp(0.0, 1.0);
    let reference = significant_height(REFERENCE_WIND, fetch_km, gravity).max(1e-6);
    let ratio = (significant_height(wind, fetch_km, gravity) / reference).clamp(0.0, 4.0);
    1.0 - influence + influence * ratio
}

/// Eases the sea towards a target on the fixed tick, so a gust doesn't pump
/// the waves.
pub fn settle_sea(current: f32, target: f32, dt: f32) -> f32 {
    let blend = 1.0 - (-dt.max(0.0) / SEA_SETTLE).exp();
    current + (target - current) * blend
}

// ─── One body, resolved ────────────────────────────────────────────────────

/// A water body as the world stands on one tick: where its surface is at
/// rest, how far it reaches, and its live waves.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterBody {
    /// The actor it rides.
    pub id: String,
    pub kind: WaterKind,
    /// The surface's centre at rest, world units.
    pub center: [f32; 3],
    /// Unit vector along the body's own x, in world xz.
    pub axis: [f32; 2],
    /// Half the size along its own x and z. Ignored by an ocean.
    pub half: [f32; 2],
    pub depth: f32,
    /// The current in world xz, world units a second.
    pub flow: [f32; 2],
    pub waves: Vec<LiveWave>,
    /// A 2D project's water: waves run along x only and z means nothing.
    pub flat: bool,
}

/// The surface at one point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterSample {
    pub height: f32,
    pub normal: [f32; 3],
    /// How fast the surface there is moving, current included.
    pub velocity: [f32; 3],
    /// Below 1 where the surface is being pinched into a crest: where foam
    /// starts. 1 on flat water.
    pub jacobian: f32,
}

impl WaterBody {
    /// Whether (x, z) is over this body's rectangle. In 2D z is ignored.
    pub fn covers(&self, x: f32, z: f32) -> bool {
        if self.kind == WaterKind::Ocean {
            return true;
        }
        let dx = x - self.center[0];
        if self.flat {
            return dx.abs() <= self.half[0];
        }
        let dz = z - self.center[2];
        let [ax, az] = self.axis;
        let along = dx * ax + dz * az;
        let across = -dx * az + dz * ax;
        along.abs() <= self.half[0] && across.abs() <= self.half[1]
    }

    fn theta(&self, wave: &LiveWave, q: Vec2, t: f32) -> f32 {
        let d = Vec2::from(wave.dir);
        let drift = Vec2::from(self.flow) * t;
        wave.k * d.dot(q - drift) - wave.omega * t + wave.phase
    }

    /// Where the rest point `q` has moved to horizontally, and its height
    /// above the level, at time `t`.
    pub fn displace(&self, q: [f32; 2], t: f32) -> ([f32; 2], f32) {
        let q = self.planar(Vec2::from(q));
        let mut offset = Vec2::ZERO;
        let mut height = 0.0;
        for wave in &self.waves {
            let theta = self.theta(wave, q, t);
            offset += self.planar(Vec2::from(wave.dir)) * wave.horizontal * theta.cos();
            height += wave.amplitude * theta.sin();
        }
        ((q + offset).to_array(), height)
    }

    /// Drops z in 2D.
    fn planar(&self, v: Vec2) -> Vec2 {
        if self.flat { Vec2::new(v.x, 0.0) } else { v }
    }

    /// The rest point whose displaced position lands over (x, z). A few
    /// fixed-point steps: Gerstner waves move points sideways, so the point
    /// under a spot isn't the spot itself.
    fn rest_point(&self, x: f32, z: f32, t: f32) -> Vec2 {
        let target = self.planar(Vec2::new(x, z));
        let mut q = target;
        for _ in 0..4 {
            let (moved, _) = self.displace(q.to_array(), t);
            q += target - Vec2::from(moved);
        }
        q
    }

    /// The surface height over (x, z) at time `t`.
    pub fn height_at(&self, x: f32, z: f32, t: f32) -> f32 {
        let q = self.rest_point(x, z, t);
        self.center[1] + self.displace(q.to_array(), t).1
    }

    /// Height, normal, velocity and pinch over (x, z) at time `t`.
    pub fn sample(&self, x: f32, z: f32, t: f32) -> WaterSample {
        let q = self.rest_point(x, z, t);
        let flow = Vec2::from(self.flow);
        let mut height = 0.0;
        let (mut xx, mut zz, mut xz) = (0.0, 0.0, 0.0);
        let (mut sx, mut sz) = (0.0, 0.0);
        let mut velocity = Vec3::new(flow.x, 0.0, flow.y);
        for wave in &self.waves {
            let d = self.planar(Vec2::from(wave.dir));
            let theta = self.theta(wave, q, t);
            let (sin, cos) = theta.sin_cos();
            height += wave.amplitude * sin;
            let wa = wave.k * wave.amplitude;
            let wh = wave.k * wave.horizontal;
            sx += d.x * wa * cos;
            sz += d.y * wa * cos;
            xx += d.x * d.x * wh * sin;
            zz += d.y * d.y * wh * sin;
            xz += d.x * d.y * wh * sin;
            // dθ/dt, current included.
            let rate = -wave.omega - wave.k * d.dot(flow);
            let horizontal = d * (-wave.horizontal * sin * rate);
            velocity += Vec3::new(horizontal.x, wave.amplitude * cos * rate, horizontal.y);
        }
        let tangent_x = Vec3::new(1.0 - xx, sx, -xz);
        let tangent_z = Vec3::new(-xz, sz, 1.0 - zz);
        let normal = if self.flat {
            Vec3::new(-sx, 1.0 - xx, 0.0).normalize_or(Vec3::Y)
        } else {
            tangent_z.cross(tangent_x).normalize_or(Vec3::Y)
        };
        let jacobian = if self.flat {
            1.0 - xx
        } else {
            (1.0 - xx) * (1.0 - zz) - xz * xz
        };
        if self.flat {
            velocity.z = 0.0;
        }
        WaterSample {
            height: self.center[1] + height,
            normal: normal.to_array(),
            velocity: velocity.to_array(),
            jacobian,
        }
    }

    /// The bottom of the body, which nothing is under the water below.
    pub fn floor(&self) -> f32 {
        self.center[1] - self.depth
    }
}

// ─── What blocks and scripts read ──────────────────────────────────────────

/// Every water body, as of the last fixed tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterSense {
    pub bodies: Vec<WaterBody>,
    /// Seconds of water time the tick was sampled at.
    pub time: f32,
}

impl WaterSense {
    /// The highest surface over (x, z), if any water covers it. In 2D z is
    /// ignored.
    pub fn surface_at(&self, x: f32, z: f32) -> Option<(&WaterBody, WaterSample)> {
        self.bodies
            .iter()
            .filter(|body| body.covers(x, z))
            .map(|body| (body, body.sample(x, z, self.time)))
            .max_by(|a, b| a.1.height.total_cmp(&b.1.height))
    }

    /// The surface height over (x, z), if there is water there.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        self.bodies
            .iter()
            .filter(|body| body.covers(x, z))
            .map(|body| body.height_at(x, z, self.time))
            .reduce(f32::max)
    }

    /// Whether a point is under some body's surface and above its floor.
    /// In 2D the point's y is its height and z is ignored.
    pub fn underwater(&self, point: [f32; 3]) -> bool {
        self.depth_at(point).is_some_and(|depth| depth > 0.0)
    }

    /// How far below the surface a point is (negative above it), over the
    /// body it is in. `None` where no body covers it or below every floor.
    pub fn depth_at(&self, point: [f32; 3]) -> Option<f32> {
        let [x, y, z] = point;
        self.bodies
            .iter()
            .filter(|body| body.covers(x, z) && y >= body.floor())
            .map(|body| body.height_at(x, z, self.time) - y)
            .reduce(f32::max)
    }
}

// ─── Floating ──────────────────────────────────────────────────────────────

/// Makes a body float: water under its sample points pushes up by the
/// weight of what they displace, and drags against the current.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuoyancySpec {
    /// Against water's: 0.5 floats half under, 1 hangs, above 1 sinks.
    pub density: f32,
    /// How hard the water holds the body to its own motion, per second.
    pub drag: f32,
    /// How hard it damps the body's spin, per second.
    pub angular_drag: f32,
    /// Where the water is sampled: 1 at the centre, 4 on the corners of the
    /// footprint (tips and rolls with the waves), 8 on the box's corners.
    pub points: u32,
    /// Whether this body splashes when it hits the water.
    pub splash: bool,
}

impl Default for BuoyancySpec {
    fn default() -> Self {
        Self {
            density: 0.5,
            drag: 1.0,
            angular_drag: 1.0,
            points: 4,
            splash: true,
        }
    }
}

impl BuoyancySpec {
    pub fn normalize(&mut self) {
        self.density = finite(self.density, 0.5).clamp(0.01, 10.0);
        self.drag = finite(self.drag, 1.0).clamp(0.0, 50.0);
        self.angular_drag = finite(self.angular_drag, 1.0).clamp(0.0, 50.0);
        self.points = match self.points {
            0 | 1 => 1,
            2..=4 => 4,
            _ => 8,
        };
    }

    /// Sample points in the body's own frame, for a box of half extents
    /// `half`. Each stands for an equal share of the body. In 2D (`flat`)
    /// the four points are the box's corners in x and y.
    pub fn sample_points(&self, half: [f32; 3], flat: bool) -> Vec<[f32; 3]> {
        let [x, y, z] = half;
        match (self.points, flat) {
            (1, _) => vec![[0.0; 3]],
            (4, true) => vec![[-x, -y, 0.0], [x, -y, 0.0], [-x, y, 0.0], [x, y, 0.0]],
            (4, false) => vec![[-x, 0.0, -z], [x, 0.0, -z], [-x, 0.0, z], [x, 0.0, z]],
            (_, true) => vec![
                [-x, -y, 0.0],
                [x, -y, 0.0],
                [-x, y, 0.0],
                [x, y, 0.0],
                [0.0, -y, 0.0],
                [0.0, y, 0.0],
                [-x, 0.0, 0.0],
                [x, 0.0, 0.0],
            ],
            (_, false) => {
                let mut points = Vec::with_capacity(8);
                for sx in [-x, x] {
                    for sy in [-y, y] {
                        for sz in [-z, z] {
                            points.push([sx, sy, sz]);
                        }
                    }
                }
                points
            }
        }
    }
}

/// How much of the slab a sample point stands for is under water: 0 when
/// the point is `span` or more above the surface, 1 when it is `span` or
/// more below. `depth` is the surface height minus the point's.
pub fn submerged(depth: f32, span: f32) -> f32 {
    let span = span.max(1e-4);
    ((depth + span) / (2.0 * span)).clamp(0.0, 1.0)
}

/// The upward push on one of `points` sample points, for a body of `mass`
/// under `gravity`: a fully sunk body is pushed with its weight over its
/// density, so density 0.5 comes to rest half under.
pub fn buoyant_force(mass: f32, gravity: f32, density: f32, points: usize, submersion: f32) -> f32 {
    mass * gravity.abs() / density.max(0.01) * submersion / points.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(spec: &WaterSpec, sea: f32) -> WaterBody {
        let set = WaveSet::build(spec, 30.0, 9.81);
        WaterBody {
            id: "w".to_string(),
            kind: spec.kind,
            center: [0.0, 2.0, 0.0],
            axis: [1.0, 0.0],
            half: [spec.size[0] / 2.0, spec.size[1] / 2.0],
            depth: spec.depth,
            flow: [0.0, 0.0],
            waves: set.resolve(sea, spec.waves.chop, spec.waves.steepness),
            flat: false,
        }
    }

    #[test]
    fn old_documents_and_bad_inputs_load() {
        assert_eq!(
            serde_json::from_str::<WaterSpec>("{}").unwrap(),
            WaterSpec::default()
        );
        let mut spec: WaterSpec =
            serde_json::from_str(r#"{"waves":{"count":40,"steepness":3},"depth":-2}"#).unwrap();
        spec.look.absorption = f32::NAN;
        spec.normalize();
        assert_eq!(spec.waves.count, MAX_WAVES as u32);
        assert_eq!(spec.waves.steepness, 1.0);
        assert_eq!(spec.depth, 0.0);
        assert_eq!(spec.look.absorption, WaterLook::default().absorption);
        assert_eq!(spec.waves.wavelength, Waves::default().wavelength);
    }

    #[test]
    fn one_seed_builds_the_same_sea() {
        let spec = WaterSpec::default();
        assert_eq!(
            WaveSet::build(&spec, 0.0, 9.81),
            WaveSet::build(&spec, 0.0, 9.81)
        );
        let mut other = spec.clone();
        other.waves.seed = 2;
        assert_ne!(
            WaveSet::build(&spec, 0.0, 9.81),
            WaveSet::build(&other, 0.0, 9.81)
        );
    }

    #[test]
    fn waves_follow_the_wind_or_their_own_heading() {
        let mut spec = WaterSpec::default();
        let set = WaveSet::build(&spec, 90.0, 9.81);
        // East is +x.
        assert!((set.waves[0].dir[0] - 1.0).abs() < 1e-5);
        spec.waves.follow_wind = false;
        spec.waves.direction = 180.0;
        let set = WaveSet::build(&spec, 90.0, 9.81);
        // South is +z.
        assert!((set.waves[0].dir[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn flat_waves_run_along_x_whatever_the_heading() {
        let set = WaveSet::build(&WaterSpec::flat(), 0.0, 9.81).flattened();
        for wave in &set.waves {
            assert_eq!(wave.dir[1], 0.0);
            assert_eq!(wave.dir[0].abs(), 1.0);
        }
    }

    #[test]
    fn deep_water_dispersion_sets_the_speed() {
        let set = WaveSet::build(&WaterSpec::default(), 0.0, 9.81);
        for wave in &set.waves {
            assert!((wave.omega * wave.omega - 9.81 * wave.k).abs() < 1e-3 * wave.k.max(1.0));
        }
        // Longest first.
        assert!(set.waves[0].k < set.waves.last().unwrap().k);
    }

    #[test]
    fn crests_never_fold_even_in_a_storm() {
        let mut spec = WaterSpec::default();
        spec.waves.steepness = 1.0;
        spec.waves.amplitude = 3.0;
        let set = WaveSet::build(&spec, 0.0, 9.81);
        let live = set.resolve(4.0, 1.0, 1.0);
        let fold: f32 = live.iter().map(|w| w.k * w.horizontal).sum();
        assert!(fold <= 1.0 + 1e-4, "{fold}");
    }

    #[test]
    fn the_height_under_a_point_is_where_the_surface_lands() {
        let mut spec = WaterSpec::default();
        spec.waves.steepness = 0.9;
        spec.waves.amplitude = 0.6;
        let water = body(&spec, 1.0);
        for (x, z, t) in [(1.3, -2.0, 0.4), (7.0, 3.5, 2.1), (-4.2, 0.2, 9.0)] {
            let h = water.height_at(x, z, t);
            // Walk the rest grid densely and find the displaced point closest
            // to (x, z): its height should match.
            let mut best = (f32::MAX, 0.0);
            for i in -300..300 {
                for j in -300..300 {
                    let q = [x + i as f32 * 0.01, z + j as f32 * 0.01];
                    let (moved, y) = water.displace(q, t);
                    let d = (moved[0] - x).powi(2) + (moved[1] - z).powi(2);
                    if d < best.0 {
                        best = (d, y);
                    }
                }
            }
            assert!((h - 2.0 - best.1).abs() < 0.02, "{h} vs {}", best.1 + 2.0);
        }
    }

    #[test]
    fn velocity_is_how_fast_the_surface_moves() {
        let water = body(&WaterSpec::default(), 1.0);
        let (x, z, t, dt) = (3.0, -1.0, 1.5, 1e-3);
        let sample = water.sample(x, z, t);
        let q = water.rest_point(x, z, t);
        let (_, y0) = water.displace(q.to_array(), t - dt);
        let (_, y1) = water.displace(q.to_array(), t + dt);
        let numeric = (y1 - y0) / (2.0 * dt);
        assert!(
            (sample.velocity[1] - numeric).abs() < 0.02,
            "{} vs {numeric}",
            sample.velocity[1]
        );
    }

    #[test]
    fn the_normal_matches_the_slope() {
        let water = body(&WaterSpec::default(), 1.0);
        let (x, z, t, e) = (2.0, 5.0, 0.7, 1e-3);
        let n = Vec3::from(water.sample(x, z, t).normal);
        let dx = (water.height_at(x + e, z, t) - water.height_at(x - e, z, t)) / (2.0 * e);
        let dz = (water.height_at(x, z + e, t) - water.height_at(x, z - e, t)) / (2.0 * e);
        let numeric = Vec3::new(-dx, 1.0, -dz).normalize();
        assert!(n.dot(numeric) > 0.999, "{n} vs {numeric}");
    }

    #[test]
    fn a_calm_sea_is_flat_and_foamless() {
        let water = body(&WaterSpec::default(), 0.0);
        let s = water.sample(3.0, 4.0, 2.0);
        assert!((s.height - 2.0).abs() < 1e-6);
        assert_eq!(s.normal, [0.0, 1.0, 0.0]);
        assert!((s.jacobian - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_current_carries_the_surface() {
        let mut water = body(&WaterSpec::default(), 0.0);
        water.flow = [2.0, -1.0];
        let s = water.sample(0.0, 0.0, 1.0);
        assert_eq!(s.velocity, [2.0, 0.0, -1.0]);
    }

    #[test]
    fn a_lake_covers_its_turned_rectangle_and_an_ocean_everything() {
        let mut water = body(&WaterSpec::default(), 1.0);
        water.half = [10.0, 2.0];
        let turn = 90f32.to_radians();
        water.axis = [turn.cos(), turn.sin()];
        assert!(water.covers(0.0, 9.0));
        assert!(!water.covers(9.0, 0.0));
        water.kind = WaterKind::Ocean;
        assert!(water.covers(1e4, -1e4));
    }

    #[test]
    fn two_d_waves_run_along_x_only() {
        let mut water = body(&WaterSpec::flat(), 1.0);
        water.flat = true;
        let a = water.height_at(10.0, 0.0, 1.0);
        let b = water.height_at(10.0, 500.0, 1.0);
        assert_eq!(a, b);
        assert_eq!(water.sample(10.0, 0.0, 1.0).normal[2], 0.0);
        assert!(water.covers(399.0, 1e6));
        assert!(!water.covers(401.0, 0.0));
    }

    #[test]
    fn underwater_means_below_the_surface_and_above_the_floor() {
        let sense = WaterSense {
            bodies: vec![body(&WaterSpec::default(), 0.0)],
            time: 0.0,
        };
        assert!(sense.underwater([0.0, 1.0, 0.0]));
        assert!(!sense.underwater([0.0, 3.0, 0.0]));
        assert!(!sense.underwater([0.0, -7.0, 0.0]));
        assert!(!sense.underwater([100.0, 1.0, 0.0]));
        assert_eq!(sense.height_at(100.0, 0.0), None);
        assert_eq!(sense.height_at(0.0, 0.0), Some(2.0));
    }

    #[test]
    fn the_wind_and_the_fetch_raise_the_sea() {
        // The reference wind stands for the authored waves.
        assert!((sea_target(REFERENCE_WIND, 10.0, 1.0, 9.81) - 1.0).abs() < 1e-5);
        assert!(sea_target(20.0, 10.0, 1.0, 9.81) > 1.5);
        assert!(sea_target(1.0, 10.0, 1.0, 9.81) < 0.2);
        // No influence: the authored waves whatever the wind.
        assert_eq!(sea_target(30.0, 10.0, 0.0, 9.81), 1.0);
        // A pond stays small: fetch-limited height grows with the fetch.
        assert!(significant_height(20.0, 0.5, 9.81) < significant_height(20.0, 200.0, 9.81));
        // And settling eases towards the target.
        let s = settle_sea(1.0, 2.0, 1.0 / 60.0);
        assert!(s > 1.0 && s < 1.01);
    }

    #[test]
    fn detail_waves_carry_a_fixed_slope_budget() {
        let spec = WaterSpec::default();
        let set = WaveSet::build(&spec, 0.0, 9.81);
        let calm = set.resolve_detail(1.0, 0.0, 2.0, 9.81);
        let stormy = set.resolve_detail(1.0, 1.0, 20.0, 9.81);
        let total = |d: &[LiveDetail]| d.iter().map(|w| w.slope).sum::<f32>();
        assert!(total(&stormy) > total(&calm));
        assert_eq!(calm.len(), DETAIL_WAVES);
        assert!(
            set.resolve_detail(0.0, 1.0, 10.0, 9.81)
                .iter()
                .all(|w| w.slope == 0.0)
        );
    }

    #[test]
    fn water_dials_for_one_body_and_for_all() {
        let mut over = WaterOverrides::default();
        over.set(Some("lake"), WaterProperty::Level, 3.0);
        over.set(None, WaterProperty::Chop, 5.0);
        over.set(Some("lake"), WaterProperty::Foam, f32::NAN);
        let lake = over.for_body("lake");
        assert_eq!(lake.level, Some(3.0));
        assert_eq!(lake.chop, Some(1.0));
        assert_eq!(lake.foam, None);
        assert_eq!(over.for_body("sea").level, None);
        over.set(None, WaterProperty::Level, 1.0);
        assert_eq!(over.for_body("lake").level, Some(1.0));
        assert_eq!(WaterProperty::parse(" LEVEL "), Some(WaterProperty::Level));
        assert_eq!(WaterProperty::parse("waves"), None);
    }

    #[test]
    fn a_half_dense_body_rests_half_under() {
        let spec = BuoyancySpec::default();
        let points = spec.sample_points([0.5, 0.5, 0.5], false);
        assert_eq!(points.len(), 4);
        // Floating with the surface at its middle: half its sample slab is
        // under, which pushes exactly its weight.
        let push: f32 = points
            .iter()
            .map(|_| buoyant_force(2.0, 9.81, 0.5, points.len(), submerged(0.0, 0.5)))
            .sum();
        assert!((push - 2.0 * 9.81).abs() < 1e-4);
        assert_eq!(submerged(-2.0, 0.5), 0.0);
        assert_eq!(submerged(2.0, 0.5), 1.0);
        let mut odd = BuoyancySpec {
            points: 6,
            density: -1.0,
            ..Default::default()
        };
        odd.normalize();
        assert_eq!(odd.points, 8);
        assert_eq!(odd.sample_points([1.0; 3], true).len(), 8);
        assert_eq!(odd.density, 0.01);
    }
}
