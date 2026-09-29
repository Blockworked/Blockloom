//! Time-of-day and weather director: the thing that makes the
//! environment shippable. A 24h clock drives curve tracks for sun, moon,
//! exposure, fog, clouds, wind and the rest, and named weather presets blend
//! into each other without popping.
//!
//! Everything here is plain arithmetic on the fixed tick's clock, so one seed
//! replays the same day however frames fall - the same rule the wind and the
//! storm director keep.

use serde::{Deserialize, Serialize};

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// One key on a 24h track: a value at `time` hours, with Bezier tangents in
/// value-per-hour. Tangents default to 0 (a smooth hold through the key).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeKey {
    pub time: f32,
    pub value: f32,
    pub in_tangent: f32,
    pub out_tangent: f32,
}

impl Default for TimeKey {
    fn default() -> Self {
        Self {
            time: 12.0,
            value: 0.0,
            in_tangent: 0.0,
            out_tangent: 0.0,
        }
    }
}

impl TimeKey {
    pub fn at(time: f32, value: f32) -> Self {
        Self {
            time,
            value,
            in_tangent: 0.0,
            out_tangent: 0.0,
        }
    }

    fn normalized(mut self) -> Self {
        self.time = finite(self.time, 12.0).rem_euclid(24.0);
        self.value = finite(self.value, 0.0);
        self.in_tangent = finite(self.in_tangent, 0.0).clamp(-1.0e6, 1.0e6);
        self.out_tangent = finite(self.out_tangent, 0.0).clamp(-1.0e6, 1.0e6);
        self
    }
}

/// One 24h curve: Bezier keys over the day, sampled by the clock. Empty means
/// "the director leaves this alone", so old projects with no tracks keep
/// exactly the look they shipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TimeTrack {
    pub keys: Vec<TimeKey>,
    /// Wrap past midnight back to 0. Off clamps at the last key.
    pub loop_enabled: bool,
}

impl TimeTrack {
    /// A flat track holding one value all day.
    pub fn flat(value: f32) -> Self {
        Self {
            keys: vec![TimeKey::at(0.0, value), TimeKey::at(24.0, value)],
            loop_enabled: true,
        }
    }

    pub fn normalize(&mut self) {
        for key in &mut self.keys {
            *key = key.normalized();
        }
        self.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.keys.dedup_by(|a, b| (a.time - b.time).abs() < 1e-6);
        if self.keys.len() > 64 {
            self.keys.truncate(64);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Sample at `time` hours. `None` for an empty track.
    pub fn sample(&self, time: f32) -> Option<f32> {
        let keys = &self.keys;
        if keys.is_empty() {
            return None;
        }
        if keys.len() == 1 {
            return Some(keys[0].value);
        }
        let mut t = finite(time, 12.0);
        if self.loop_enabled {
            t = t.rem_euclid(24.0);
        } else {
            t = t.clamp(0.0, 24.0);
        }
        if t <= keys[0].time {
            return Some(keys[0].value);
        }
        if t >= keys[keys.len() - 1].time {
            return Some(keys[keys.len() - 1].value);
        }
        let mut i = 0;
        while i + 1 < keys.len() && keys[i + 1].time < t {
            i += 1;
        }
        let (a, b) = (keys[i], keys[i + 1]);
        let span = (b.time - a.time).max(1e-6);
        let u = ((t - a.time) / span).clamp(0.0, 1.0);
        // Cubic Hermite with tangents in value-per-hour.
        let m0 = a.out_tangent * span;
        let m1 = b.in_tangent * span;
        let u2 = u * u;
        let u3 = u2 * u;
        Some(
            (2.0 * u3 - 3.0 * u2 + 1.0) * a.value
                + (u3 - 2.0 * u2 + u) * m0
                + (-2.0 * u3 + 3.0 * u2) * b.value
                + (u3 - u2) * m1,
        )
    }
}

/// Which precipitation `set precipitation to` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrecipitationKind {
    Rain,
    Snow,
}

impl PrecipitationKind {
    pub fn name(self) -> &'static str {
        match self {
            PrecipitationKind::Rain => "Rain",
            PrecipitationKind::Snow => "Snow",
        }
    }

    /// Case-insensitive, so a script's `"rain"` works too.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "rain" | "precipitation" => Some(PrecipitationKind::Rain),
            "snow" => Some(PrecipitationKind::Snow),
            _ => None,
        }
    }
}

/// What `set precipitation` set this run, over the director and the project.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PrecipitationOverrides {
    pub rain: Option<f32>,
    pub snow: Option<f32>,
}

impl PrecipitationOverrides {
    pub fn set(&mut self, kind: PrecipitationKind, value: f32) {
        if !value.is_finite() {
            return;
        }
        let value = value.clamp(0.0, 1.0);
        match kind {
            PrecipitationKind::Rain => self.rain = Some(value),
            PrecipitationKind::Snow => self.snow = Some(value),
        }
    }
}

/// The director's authored state: a clock plus one track per dial the 24h
/// curve editor owns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Director {
    /// The director runs at all. Off leaves sun, exposure and weather to the
    /// project, volumes and blocks exactly as before.
    pub enabled: bool,
    /// Hours, 0-24. Where the clock starts on Play.
    pub time_of_day: f32,
    /// Seconds for one full day. 0 freezes the clock (blocks still move it).
    pub day_length: f32,
    pub loop_enabled: bool,
    pub sun_azimuth: TimeTrack,
    pub sun_elevation: TimeTrack,
    pub moon_azimuth: TimeTrack,
    pub moon_elevation: TimeTrack,
    pub exposure: TimeTrack,
    pub temperature: TimeTrack,
    pub fog_density: TimeTrack,
    pub cloud_coverage: TimeTrack,
    pub cloud_type: TimeTrack,
    pub precipitation: TimeTrack,
    pub wetness: TimeTrack,
    pub wind_speed: TimeTrack,
    pub wind_direction: TimeTrack,
    pub aurora_kp: TimeTrack,
    /// 0-1 weight of the grading LUT. The LUT file itself stays the project's.
    pub lut_weight: TimeTrack,
    /// Named weather presets in this project, user ones included. Built-ins
    /// are answered by [`WeatherPreset::builtin`] when no authored one shares
    /// the name.
    pub presets: Vec<WeatherPreset>,
}

impl Default for Director {
    fn default() -> Self {
        Self {
            enabled: false,
            time_of_day: 12.0,
            day_length: 600.0,
            loop_enabled: true,
            sun_azimuth: TimeTrack::default(),
            sun_elevation: TimeTrack::default(),
            moon_azimuth: TimeTrack::default(),
            moon_elevation: TimeTrack::default(),
            exposure: TimeTrack::default(),
            temperature: TimeTrack::default(),
            fog_density: TimeTrack::default(),
            cloud_coverage: TimeTrack::default(),
            cloud_type: TimeTrack::default(),
            precipitation: TimeTrack::default(),
            wetness: TimeTrack::default(),
            wind_speed: TimeTrack::default(),
            wind_direction: TimeTrack::default(),
            aurora_kp: TimeTrack::default(),
            lut_weight: TimeTrack::default(),
            presets: Vec::new(),
        }
    }
}

impl Director {
    pub fn normalize(&mut self) {
        self.time_of_day = finite(self.time_of_day, 12.0).rem_euclid(24.0);
        self.day_length = finite(self.day_length, 600.0).clamp(0.0, 86400.0);
        for track in [
            &mut self.sun_azimuth,
            &mut self.sun_elevation,
            &mut self.moon_azimuth,
            &mut self.moon_elevation,
            &mut self.exposure,
            &mut self.temperature,
            &mut self.fog_density,
            &mut self.cloud_coverage,
            &mut self.cloud_type,
            &mut self.precipitation,
            &mut self.wetness,
            &mut self.wind_speed,
            &mut self.wind_direction,
            &mut self.aurora_kp,
            &mut self.lut_weight,
        ] {
            track.normalize();
        }
        for preset in &mut self.presets {
            preset.normalize();
        }
        if self.presets.len() > 32 {
            self.presets.truncate(32);
        }
    }

    /// Hours advanced per second of run. 0 when frozen or disabled.
    pub fn rate(&self) -> f32 {
        if !self.enabled || self.day_length <= 0.0 {
            return 0.0;
        }
        24.0 / self.day_length
    }

    /// A keyframe preset for the curve editor: dawn, noon, dusk or midnight.
    /// Each returns a director with a sun track through that moment plus
    /// matching exposure, fog and cloud tracks; anything else stays off.
    pub fn keyframe_preset(name: &str) -> Option<Self> {
        let (sun_el, exposure, fog, clouds) = match name.trim().to_ascii_lowercase().as_str() {
            "dawn" => (8.0, 8.0, 0.004, 0.45),
            "noon" => (62.0, 9.7, 0.0, 0.35),
            "dusk" => (4.0, 7.5, 0.006, 0.55),
            "midnight" => (-38.0, 4.0, 0.001, 0.15),
            _ => return None,
        };
        let mut director = Director {
            enabled: true,
            time_of_day: match name.trim().to_ascii_lowercase().as_str() {
                "dawn" => 6.0,
                "noon" => 12.0,
                "dusk" => 18.0,
                _ => 0.0,
            },
            ..Director::default()
        };
        director.sun_elevation = TimeTrack::flat(sun_el);
        director.sun_azimuth = TimeTrack::flat(match name.trim().to_ascii_lowercase().as_str() {
            "dawn" => 90.0,
            "noon" => 180.0,
            "dusk" => 270.0,
            _ => 0.0,
        });
        director.exposure = TimeTrack::flat(exposure);
        director.fog_density = TimeTrack::flat(fog);
        director.cloud_coverage = TimeTrack::flat(clouds);
        Some(director)
    }

    /// The preset `name` spells: an authored one first, else a built-in.
    pub fn preset(&self, name: &str) -> Option<WeatherPreset> {
        if let Some(found) = self
            .presets
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
        {
            return Some(found.clone());
        }
        WeatherPreset::builtin(name)
    }
}

/// One weather: full sky/cloud/fog/light/post deltas as plain values, so a
/// blend is a lerp. Sky exposure, sunlight, ambient, bloom and saturation
/// ride beside the fog/cloud/wind dials, so a preset carries the whole look:
/// sky, lights and post, not just the air. Volumes and blocks lay over the
/// result the same way they lay over the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WeatherPreset {
    pub name: String,
    pub values: WeatherValues,
}

impl WeatherPreset {
    pub fn builtin(name: &str) -> Option<Self> {
        let values = match name.trim().to_ascii_lowercase().as_str() {
            "clear" => WeatherValues {
                fog_density: 0.0,
                cloud_coverage: 0.12,
                cloud_density: 0.7,
                cloud_type: 0.4,
                precipitation: 0.0,
                snow: 0.0,
                wetness: 0.0,
                temperature: 21.0,
                wind_speed: 2.0,
                wind_direction: 45.0,
                storm: 0.0,
                aurora_kp: 0.0,
                exposure: 9.7,
                sun_elevation: 48.0,
                sun_azimuth: 160.0,
                lut_weight: 0.0,
                sunlight: 1.0,
                ambient: 1.0,
                sky_exposure: 0.0,
                bloom: 1.0,
                saturation: 1.0,
            },
            "overcast" => WeatherValues {
                fog_density: 0.004,
                cloud_coverage: 0.92,
                cloud_density: 1.4,
                cloud_type: 0.25,
                precipitation: 0.0,
                snow: 0.0,
                wetness: 0.25,
                temperature: 14.0,
                wind_speed: 6.0,
                wind_direction: 250.0,
                storm: 0.2,
                aurora_kp: 0.0,
                exposure: 10.6,
                sun_elevation: 30.0,
                sun_azimuth: 190.0,
                lut_weight: 0.35,
                sunlight: 0.8,
                ambient: 1.1,
                sky_exposure: -0.3,
                bloom: 1.1,
                saturation: 0.9,
            },
            "storm" => WeatherValues {
                fog_density: 0.012,
                cloud_coverage: 1.0,
                cloud_density: 2.2,
                cloud_type: 0.85,
                precipitation: 1.0,
                snow: 0.0,
                wetness: 1.0,
                temperature: 11.0,
                wind_speed: 22.0,
                wind_direction: 260.0,
                storm: 1.0,
                aurora_kp: 0.0,
                exposure: 11.2,
                sun_elevation: 12.0,
                sun_azimuth: 210.0,
                lut_weight: 0.6,
                sunlight: 0.4,
                ambient: 1.2,
                sky_exposure: -1.0,
                bloom: 0.8,
                saturation: 0.8,
            },
            "sunset" => WeatherValues {
                fog_density: 0.003,
                cloud_coverage: 0.45,
                cloud_density: 0.9,
                cloud_type: 0.65,
                precipitation: 0.0,
                snow: 0.0,
                wetness: 0.0,
                temperature: 18.0,
                wind_speed: 3.0,
                wind_direction: 280.0,
                storm: 0.0,
                aurora_kp: 0.0,
                exposure: 8.2,
                sun_elevation: 5.0,
                sun_azimuth: 272.0,
                lut_weight: 0.8,
                sunlight: 1.2,
                ambient: 0.9,
                sky_exposure: 0.5,
                bloom: 1.3,
                saturation: 1.25,
            },
            "night" => WeatherValues {
                fog_density: 0.002,
                cloud_coverage: 0.2,
                cloud_density: 0.8,
                cloud_type: 0.5,
                precipitation: 0.0,
                snow: 0.0,
                wetness: 0.1,
                temperature: 8.0,
                wind_speed: 1.5,
                wind_direction: 20.0,
                storm: 0.0,
                aurora_kp: 4.0,
                exposure: 4.5,
                sun_elevation: -32.0,
                sun_azimuth: 10.0,
                lut_weight: 0.5,
                sunlight: 0.15,
                ambient: 0.7,
                sky_exposure: 0.0,
                bloom: 1.2,
                saturation: 0.9,
            },
            _ => return None,
        };
        Some(Self {
            name: name.trim().to_string(),
            values,
        })
    }

    /// Every built-in, in palette order.
    pub fn builtins() -> Vec<Self> {
        ["Clear", "Overcast", "Storm", "Sunset", "Night"]
            .into_iter()
            .filter_map(Self::builtin)
            .collect()
    }

    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            self.name = "Weather".to_string();
        }
        if self.name.len() > 64 {
            self.name.truncate(64);
        }
        self.values.clamp();
    }
}

/// The values a blend moves: one number per dial a preset owns.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherValues {
    pub fog_density: f32,
    pub cloud_coverage: f32,
    pub cloud_density: f32,
    pub cloud_type: f32,
    /// 0-1 rain. Snow rides `snow`.
    pub precipitation: f32,
    pub snow: f32,
    pub wetness: f32,
    pub temperature: f32,
    pub wind_speed: f32,
    pub wind_direction: f32,
    pub storm: f32,
    pub aurora_kp: f32,
    pub exposure: f32,
    pub sun_elevation: f32,
    pub sun_azimuth: f32,
    pub lut_weight: f32,
    /// Multiplier on the sun's illuminance: the light half of the preset.
    pub sunlight: f32,
    /// Multiplier on the ambient dimmer: the sky's fill light.
    pub ambient: f32,
    /// EV added to the sky itself, after the exposure.
    pub sky_exposure: f32,
    /// Multiplier on the bloom intensity: the post half of the preset.
    pub bloom: f32,
    /// Multiplier on the grading saturation.
    pub saturation: f32,
}

impl Default for WeatherValues {
    fn default() -> Self {
        Self {
            fog_density: 0.0,
            cloud_coverage: 0.35,
            cloud_density: 0.8,
            cloud_type: 0.7,
            precipitation: 0.0,
            snow: 0.0,
            wetness: 0.0,
            temperature: 20.0,
            wind_speed: 0.0,
            wind_direction: 45.0,
            storm: 0.0,
            aurora_kp: 0.0,
            exposure: 9.7,
            sun_elevation: 54.7,
            sun_azimuth: 135.0,
            lut_weight: 0.0,
            sunlight: 1.0,
            ambient: 1.0,
            sky_exposure: 0.0,
            bloom: 1.0,
            saturation: 1.0,
        }
    }
}

impl WeatherValues {
    fn clamp(&mut self) {
        self.fog_density = finite(self.fog_density, 0.0).clamp(0.0, 10.0);
        self.cloud_coverage = finite(self.cloud_coverage, 0.35).clamp(0.0, 1.0);
        self.cloud_density = finite(self.cloud_density, 0.8).clamp(0.0, 10.0);
        self.cloud_type = finite(self.cloud_type, 0.7).clamp(0.0, 1.0);
        self.precipitation = finite(self.precipitation, 0.0).clamp(0.0, 1.0);
        self.snow = finite(self.snow, 0.0).clamp(0.0, 1.0);
        self.wetness = finite(self.wetness, 0.0).clamp(0.0, 1.0);
        self.temperature = finite(self.temperature, 20.0).clamp(-60.0, 60.0);
        self.wind_speed = finite(self.wind_speed, 0.0).clamp(0.0, crate::wind::MAX_SPEED);
        self.wind_direction = finite(self.wind_direction, 45.0).rem_euclid(360.0);
        self.storm = finite(self.storm, 0.0).clamp(0.0, 1.0);
        self.aurora_kp = finite(self.aurora_kp, 0.0).clamp(0.0, 9.0);
        self.exposure = finite(self.exposure, 9.7).clamp(0.0, 20.0);
        self.sun_elevation = finite(self.sun_elevation, 54.7).clamp(-90.0, 90.0);
        self.sun_azimuth = finite(self.sun_azimuth, 135.0).rem_euclid(360.0);
        self.lut_weight = finite(self.lut_weight, 0.0).clamp(0.0, 1.0);
        self.sunlight = finite(self.sunlight, 1.0).clamp(0.0, 4.0);
        self.ambient = finite(self.ambient, 1.0).clamp(0.0, 2.0);
        self.sky_exposure = finite(self.sky_exposure, 0.0).clamp(-6.0, 6.0);
        self.bloom = finite(self.bloom, 1.0).clamp(0.0, 4.0);
        self.saturation = finite(self.saturation, 1.0).clamp(0.0, 2.0);
    }

    /// Lerp towards `to` by `t`. Wind direction takes the short way round.
    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let mut turn = (to.wind_direction - self.wind_direction).rem_euclid(360.0);
        if turn > 180.0 {
            turn -= 360.0;
        }
        let mut out = Self {
            fog_density: mix(self.fog_density, to.fog_density),
            cloud_coverage: mix(self.cloud_coverage, to.cloud_coverage),
            cloud_density: mix(self.cloud_density, to.cloud_density),
            cloud_type: mix(self.cloud_type, to.cloud_type),
            precipitation: mix(self.precipitation, to.precipitation),
            snow: mix(self.snow, to.snow),
            wetness: mix(self.wetness, to.wetness),
            temperature: mix(self.temperature, to.temperature),
            wind_speed: mix(self.wind_speed, to.wind_speed),
            wind_direction: (self.wind_direction + turn * t).rem_euclid(360.0),
            storm: mix(self.storm, to.storm),
            aurora_kp: mix(self.aurora_kp, to.aurora_kp),
            exposure: mix(self.exposure, to.exposure),
            sun_elevation: mix(self.sun_elevation, to.sun_elevation),
            sun_azimuth: {
                let mut turn = (to.sun_azimuth - self.sun_azimuth).rem_euclid(360.0);
                if turn > 180.0 {
                    turn -= 360.0;
                }
                (self.sun_azimuth + turn * t).rem_euclid(360.0)
            },
            lut_weight: mix(self.lut_weight, to.lut_weight),
            sunlight: mix(self.sunlight, to.sunlight),
            ambient: mix(self.ambient, to.ambient),
            sky_exposure: mix(self.sky_exposure, to.sky_exposure),
            bloom: mix(self.bloom, to.bloom),
            saturation: mix(self.saturation, to.saturation),
        };
        out.clamp();
        out
    }
}

/// How a blend eases from the current weather into the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum WeatherEase {
    #[default]
    Smooth,
    Linear,
    EaseInOut,
}

impl WeatherEase {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "smooth" => Some(WeatherEase::Smooth),
            "linear" => Some(WeatherEase::Linear),
            "easeinout" | "ease-in-out" | "ease_in_out" => Some(WeatherEase::EaseInOut),
            _ => None,
        }
    }

    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            WeatherEase::Linear => t,
            WeatherEase::Smooth => t * t * (3.0 - 2.0 * t),
            WeatherEase::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}

/// The live blend: what the weather is right now, and where it is going.
/// Starting a new blend from the sampled current values means rapid changes
/// never pop. The seed makes the blend deterministic: one seed, one path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherState {
    /// What the air is right now, as values.
    pub current: WeatherValues,
    /// Where the blend is going, by preset name. Empty for none yet.
    pub current_name: String,
    target: WeatherValues,
    target_name: String,
    /// Seconds into the blend, and how long it runs.
    pub t: f32,
    pub duration: f32,
    pub ease: WeatherEase,
    /// Blends so far; each one's seed hashes from it and the target name.
    pub blends: u32,
    /// Whether any blend has ever run: before the first one the weather
    /// layer stays out of the way entirely.
    pub active: bool,
}

impl Default for WeatherState {
    fn default() -> Self {
        Self {
            current: WeatherValues::default(),
            current_name: String::new(),
            target: WeatherValues::default(),
            target_name: String::new(),
            t: 0.0,
            duration: 0.0,
            ease: WeatherEase::Smooth,
            blends: 0,
            active: false,
        }
    }
}

impl WeatherState {
    /// Starts blending towards `preset` over `seconds`. A non-positive
    /// duration cuts straight to it. The start is the sampled current, so a
    /// blend begun mid-blend carries on from where the air is.
    pub fn start_blend(&mut self, preset: &WeatherPreset, seconds: f32) {
        let sampled = self.sampled();
        self.current = sampled;
        self.target = preset.values;
        self.target_name = preset.name.clone();
        self.t = 0.0;
        self.duration = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        self.blends = self.blends.wrapping_add(1);
        self.active = true;
        if self.duration <= 0.0 {
            self.current = self.target;
            self.current_name = self.target_name.clone();
            self.target_name.clear();
        }
    }

    /// Advances by `dt` seconds. Answers the new current values and whether
    /// the blend just finished (with its name).
    pub fn step(&mut self, dt: f32) -> (WeatherValues, Option<String>) {
        if !self.active || self.target_name.is_empty() {
            return (self.sampled(), None);
        }
        if !dt.is_finite() || dt <= 0.0 {
            return (self.sampled(), None);
        }
        self.t += dt;
        if self.t >= self.duration {
            self.current = self.target;
            self.current_name = self.target_name.clone();
            self.target_name.clear();
            self.t = self.duration;
            return (self.current, Some(self.current_name.clone()));
        }
        (self.sampled(), None)
    }

    /// The values as they stand mid-blend, eased.
    pub fn sampled(&self) -> WeatherValues {
        if !self.active || self.target_name.is_empty() || self.duration <= 0.0 {
            return self.current;
        }
        let u = (self.t / self.duration).clamp(0.0, 1.0);
        self.current.lerp(&self.target, self.ease.apply(u))
    }

    /// Deterministic seed for this blend: one name, one path.
    pub fn seed(&self) -> u32 {
        let mut h = 0x9E37_79B9u32.wrapping_add(self.blends.wrapping_mul(0x85EB_CA6B));
        for b in self.target_name.bytes().chain(self.current_name.bytes()) {
            h = h.wrapping_mul(0x0100_0193) ^ b as u32;
        }
        h
    }

    /// Whether the weather layer should move anything at all.
    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// How fast open ground dries, per second: warm air, high sun and wind all
/// pull water out faster. Cold, dark and still barely dries at all, so a
/// storm at midnight stays wet till morning.
pub fn evaporation_rate(temperature: f32, sun_elevation: f32, wind_speed: f32) -> f32 {
    let warm = if temperature.is_finite() {
        (temperature / 30.0).clamp(0.0, 1.5)
    } else {
        0.7
    };
    let sun = if sun_elevation.is_finite() {
        sun_elevation.to_radians().sin().max(0.0)
    } else {
        0.5
    };
    let wind = if wind_speed.is_finite() {
        (wind_speed / 15.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    0.004 * (0.25 + 0.75 * warm) * (1.0 + sun) * (1.0 + 0.5 * wind)
}

/// How fast rain soaks the ground, per second at full downpour.
pub const WET_RATE: f32 = 0.08;

/// Wetness as a map, not a number: a coarse grid of ground dampness around
/// the camera. Rain soaks every cell while evaporation dries each one at its
/// own uneven pace, so puddles linger in patches instead of the whole world
/// drying on one fixed slope. The weather's own wetness is the floor the map
/// dries towards, never below it.
#[derive(Debug, Clone, PartialEq)]
pub struct WetnessMap {
    cells: Vec<f32>,
    center: (f32, f32),
    /// The last sample taken, what surfaces and reporters read.
    pub sampled: f32,
}

impl Default for WetnessMap {
    fn default() -> Self {
        Self {
            cells: vec![0.0; Self::GRID * Self::GRID],
            center: (0.0, 0.0),
            sampled: 0.0,
        }
    }
}

impl WetnessMap {
    /// Cells across, and metres across: 2 m cells over 64 m.
    pub const GRID: usize = 32;
    pub const EXTENT: f32 = 64.0;

    pub fn cell_size() -> f32 {
        Self::EXTENT / Self::GRID as f32
    }

    /// The raw cells, row-major, for the GPU upload.
    pub fn cells(&self) -> &[f32] {
        &self.cells
    }

    /// Where the grid is centred in world XZ.
    pub fn center(&self) -> (f32, f32) {
        self.center
    }

    /// The shader frame: centre x/z, extent metres, unused. It matches
    /// `sample`'s maths, so the texture reads what the CPU reads.
    pub fn frame(&self) -> [f32; 4] {
        [self.center.0, self.center.1, Self::EXTENT, 0.0]
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn hash01(ix: usize, iy: usize) -> f32 {
        let mut h = (ix as u32).wrapping_mul(0x85EB_CA6B) ^ (iy as u32).wrapping_mul(0xC2B2_AE35);
        h ^= h >> 13;
        h = h.wrapping_mul(0x5BD1_E995);
        h ^= h >> 15;
        (h % 1000) as f32 / 1000.0
    }

    /// Drying varies per cell, so the map dries unevenly like real ground.
    fn dry_variation(ix: usize, iy: usize) -> f32 {
        0.7 + 0.6 * Self::hash01(ix, iy)
    }

    /// Moves the map with the camera, shifting cells along. Newly covered
    /// ground starts at the map's mean, so a pan never pops the wetness.
    pub fn recenter(&mut self, x: f32, z: f32) {
        if !x.is_finite() || !z.is_finite() {
            return;
        }
        let size = Self::cell_size();
        let raw = (
            ((x - self.center.0) / size).round() as isize,
            ((z - self.center.1) / size).round() as isize,
        );
        if raw == (0, 0) {
            return;
        }
        let mean: f32 = self.cells.iter().sum::<f32>() / (Self::GRID * Self::GRID) as f32;
        let mut next = vec![mean; Self::GRID * Self::GRID];
        for iy in 0..Self::GRID {
            for ix in 0..Self::GRID {
                let sx = ix as isize - raw.0;
                let sz = iy as isize - raw.1;
                if sx >= 0 && sz >= 0 && sx < Self::GRID as isize && sz < Self::GRID as isize {
                    next[iy * Self::GRID + ix] = self.cells[sz as usize * Self::GRID + sx as usize];
                }
            }
        }
        self.cells = next;
        self.center = (
            self.center.0 + raw.0 as f32 * size,
            self.center.1 + raw.1 as f32 * size,
        );
    }

    /// Steps the map by `dt` seconds: rain soaks towards 1, damp weather
    /// eases dry cells up to its target, and evaporation pulls everything
    /// back down to that target at each cell's own pace.
    pub fn step(&mut self, target: f32, rain: f32, evaporation: f32, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let target = finite(target, 0.0).clamp(0.0, 1.0);
        let rain = finite(rain, 0.0).clamp(0.0, 1.0);
        let evaporation = finite(evaporation, 0.0).max(0.0);
        let cap = if rain > 0.0 { 1.0 } else { target };
        let rise = 1.0 - (-dt * 0.5).exp();
        for iy in 0..Self::GRID {
            for ix in 0..Self::GRID {
                let i = iy * Self::GRID + ix;
                let mut c = self.cells[i];
                if c < cap {
                    c = (c + rain * WET_RATE * dt + (cap - c) * rise).min(cap);
                }
                if c > target {
                    c = (c - evaporation * Self::dry_variation(ix, iy) * dt).max(target);
                }
                self.cells[i] = c.clamp(0.0, 1.0);
            }
        }
    }

    /// Bilinear sample at a world XZ, edge-clamped outside the map.
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let size = Self::cell_size();
        let half = Self::EXTENT / 2.0;
        let gx = ((x - (self.center.0 - half)) / size - 0.5).clamp(0.0, Self::GRID as f32 - 1.001);
        let gz = ((z - (self.center.1 - half)) / size - 0.5).clamp(0.0, Self::GRID as f32 - 1.001);
        if !gx.is_finite() || !gz.is_finite() {
            return self.sampled;
        }
        let x0 = gx.floor() as usize;
        let z0 = gz.floor() as usize;
        let fx = gx - x0 as f32;
        let fz = gz - z0 as f32;
        let at = |ix: usize, iz: usize| self.cells[iz * Self::GRID + ix];
        let x1 = (x0 + 1).min(Self::GRID - 1);
        let z1 = (z0 + 1).min(Self::GRID - 1);
        (at(x0, z0) * (1.0 - fx) + at(x1, z0) * fx) * (1.0 - fz)
            + (at(x0, z1) * (1.0 - fx) + at(x1, z1) * fx) * fz
    }

    /// Steps, then samples at the camera: the one call the fixed tick makes.
    pub fn step_at(
        &mut self,
        x: f32,
        z: f32,
        target: f32,
        rain: f32,
        evaporation: f32,
        dt: f32,
    ) -> f32 {
        self.recenter(x, z);
        self.step(target, rain, evaporation, dt);
        self.sampled = self.sample(x, z);
        self.sampled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_track_leaves_the_dial_alone() {
        assert_eq!(TimeTrack::default().sample(9.0), None);
        assert_eq!(TimeTrack::flat(3.0).sample(9.0), Some(3.0));
    }

    #[test]
    fn keys_sample_and_hold_their_ends() {
        let mut track = TimeTrack {
            keys: vec![TimeKey::at(6.0, 0.0), TimeKey::at(18.0, 12.0)],
            loop_enabled: false,
        };
        track.normalize();
        assert_eq!(track.sample(0.0), Some(0.0));
        assert_eq!(track.sample(24.0), Some(12.0));
        let mid = track.sample(12.0).unwrap();
        assert!((mid - 6.0).abs() < 1e-4, "{mid}");
    }

    #[test]
    fn tangents_shape_the_curve() {
        let mut track = TimeTrack {
            keys: vec![
                TimeKey {
                    time: 0.0,
                    value: 0.0,
                    out_tangent: 2.0,
                    ..TimeKey::default()
                },
                TimeKey::at(2.0, 2.0),
            ],
            loop_enabled: false,
        };
        track.normalize();
        // A rising out-tangent overshoots the straight line early.
        assert!(track.sample(0.5).unwrap() > 0.5);
    }

    #[test]
    fn keyframe_presets_name_their_moments() {
        for (name, time) in [
            ("dawn", 6.0),
            ("noon", 12.0),
            ("dusk", 18.0),
            ("midnight", 0.0),
        ] {
            let director = Director::keyframe_preset(name).unwrap();
            assert_eq!(director.time_of_day, time);
            assert!(director.sun_elevation.sample(time).is_some());
        }
        assert_eq!(Director::keyframe_preset("storm"), None);
    }

    #[test]
    fn builtins_cover_the_palette_and_round_trip() {
        let names: Vec<_> = WeatherPreset::builtins()
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(names, ["Clear", "Overcast", "Storm", "Sunset", "Night"]);
        for preset in WeatherPreset::builtins() {
            let json = serde_json::to_string(&preset).unwrap();
            assert_eq!(
                serde_json::from_str::<WeatherPreset>(&json).unwrap(),
                preset
            );
        }
        assert!(WeatherPreset::builtin("drizzle").is_none());
        assert_eq!(
            PrecipitationKind::parse("RAIN"),
            Some(PrecipitationKind::Rain)
        );
        assert_eq!(PrecipitationKind::parse("hail"), None);
    }

    #[test]
    fn a_blend_eases_and_a_rapid_retarget_does_not_pop() {
        let clear = WeatherPreset::builtin("Clear").unwrap();
        let storm = WeatherPreset::builtin("Storm").unwrap();
        let mut state = WeatherState::default();
        state.start_blend(&clear, 0.0);
        assert_eq!(state.current_name, "Clear");
        state.start_blend(&storm, 10.0);
        let (half, done) = state.step(5.0);
        assert!(done.is_none());
        assert!(half.precipitation > 0.0 && half.precipitation < 1.0);
        // Retarget mid-blend: the start is where the air is, not Clear.
        let before = state.sampled();
        let overcast = WeatherPreset::builtin("Overcast").unwrap();
        state.start_blend(&overcast, 10.0);
        assert_eq!(state.sampled(), before);
        // Finishing lands exactly on the target.
        let (end, done) = state.step(10.0);
        assert_eq!(done, Some("Overcast".to_string()));
        assert_eq!(end, overcast.values);
    }

    #[test]
    fn one_name_is_one_seed() {
        let mut a = WeatherState::default();
        let storm = WeatherPreset::builtin("Storm").unwrap();
        a.start_blend(&storm, 5.0);
        let mut b = WeatherState::default();
        b.start_blend(&storm, 5.0);
        assert_eq!(a.seed(), b.seed());
        let clear = WeatherPreset::builtin("Clear").unwrap();
        b.start_blend(&clear, 5.0);
        assert_ne!(a.seed(), b.seed());
    }

    #[test]
    fn normalize_keeps_old_documents_loading() {
        let director: Director = serde_json::from_str("{}").unwrap();
        assert_eq!(director, Director::default());
        let mut director = Director {
            time_of_day: 25.0,
            day_length: f32::NAN,
            ..Director::default()
        };
        director.normalize();
        assert_eq!(director.time_of_day, 1.0);
        assert_eq!(director.day_length, 600.0);
    }

    #[test]
    fn presets_carry_the_whole_look() {
        let storm = WeatherPreset::builtin("Storm").unwrap();
        assert!(storm.values.sunlight < 1.0);
        assert!(storm.values.sky_exposure < 0.0);
        assert!(storm.values.saturation < 1.0);
        let sunset = WeatherPreset::builtin("Sunset").unwrap();
        assert!(sunset.values.sunlight > 1.0);
        assert!(sunset.values.saturation > 1.0);
        // Old documents without the new dials load as neutral.
        let values: WeatherValues = serde_json::from_str("{}").unwrap();
        assert_eq!(values, WeatherValues::default());
        let mut values = WeatherValues {
            sunlight: f32::NAN,
            sky_exposure: -99.0,
            ..WeatherValues::default()
        };
        values.clamp();
        assert_eq!(values.sunlight, 1.0);
        assert_eq!(values.sky_exposure, -6.0);
        // A blend moves every new dial too.
        let half = WeatherValues::default().lerp(&storm.values, 0.5);
        assert!((half.sunlight - (1.0 + storm.values.sunlight) / 2.0).abs() < 1e-6);
        assert!((half.sky_exposure - storm.values.sky_exposure / 2.0).abs() < 1e-6);
    }

    #[test]
    fn evaporation_needs_warmth_sun_and_wind() {
        let warm = evaporation_rate(30.0, 60.0, 10.0);
        let cold = evaporation_rate(-10.0, -20.0, 0.0);
        assert!(warm > evaporation_rate(20.0, 60.0, 0.0));
        assert!(evaporation_rate(20.0, 60.0, 10.0) > evaporation_rate(20.0, 60.0, 0.0));
        assert!(cold < 0.002);
        assert!(evaporation_rate(f32::NAN, f32::NAN, f32::NAN).is_finite());
    }

    #[test]
    fn the_map_soaks_in_rain_and_dries_unevenly() {
        let mut map = WetnessMap::default();
        // A downpour soaks the ground in seconds.
        for _ in 0..60 {
            map.step(0.0, 1.0, 0.0, 1.0);
        }
        assert!(map.sample(0.0, 0.0) > 0.9);
        // Drying never reaches below the weather's damp floor, and cells
        // dry at their own pace, so the map is uneven mid-dry.
        for _ in 0..30 {
            map.step(0.2, 0.0, 0.01, 1.0);
        }
        let mut lo = 1.0f32;
        let mut hi = 0.0f32;
        for iy in 0..WetnessMap::GRID {
            for ix in 0..WetnessMap::GRID {
                let c = map.cells[iy * WetnessMap::GRID + ix];
                assert!(c >= 0.2 - 1e-6, "{c}");
                lo = lo.min(c);
                hi = hi.max(c);
            }
        }
        assert!(hi - lo > 0.01, "{lo}..{hi}");
        // Damp weather alone eases dry ground up to its target.
        let mut dry = WetnessMap::default();
        for _ in 0..600 {
            dry.step(0.5, 0.0, 0.0, 0.1);
        }
        assert!((dry.sample(3.0, -4.0) - 0.5).abs() < 0.01);
    }

    #[test]
    fn the_map_follows_the_camera_without_popping() {
        let mut map = WetnessMap::default();
        for _ in 0..60 {
            map.step(0.0, 1.0, 0.0, 1.0);
        }
        let before = map.sample(0.0, 0.0);
        map.recenter(4.0, 0.0);
        assert!((map.sample(4.0, 0.0) - before).abs() < 0.05);
        // Far outside still answers with the edge, never NaN.
        assert!(map.sample(9999.0, -9999.0).is_finite());
        assert!(map.sample(f32::NAN, 0.0).is_finite());
    }

    #[test]
    fn the_frame_names_what_the_gpu_uploads() {
        let mut map = WetnessMap::default();
        map.recenter(6.0, -4.0);
        assert_eq!(map.center(), map.center);
        assert_eq!(
            map.frame(),
            [map.center.0, map.center.1, WetnessMap::EXTENT, 0.0]
        );
        assert_eq!(map.cells().len(), WetnessMap::GRID * WetnessMap::GRID);
        // Cell order is row-major, so the upload's first row is the map's.
        for _ in 0..60 {
            map.step(0.0, 1.0, 0.0, 1.0);
        }
        assert!(map.cells().iter().all(|c| *c > 0.9));
    }
}
