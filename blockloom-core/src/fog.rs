//! The air between the camera and the world, 3D only: an analytic height
//! fog, a froxel volumetric fog lit by the sun, the moon and the lights,
//! and aerial perspective that hazes and blues distant things.
//!
//! This is the model and the CPU half of the maths. The runtime renders it
//! in one pass over the frame (`blockloom-runtime/src/fog.rs`).

use serde::{Deserialize, Serialize};

/// Everything in the air.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Fog {
    pub height: HeightFog,
    pub volumetric: VolumetricFog,
    pub aerial: AerialPerspective,
}

/// Exponential height fog, integrated exactly along each view ray: thick
/// at its base, thinning upwards, lit by the sun and the ambient light.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeightFog {
    pub enabled: bool,
    /// World height where the fog is as thick as `distance` says.
    pub base_height: f32,
    /// Per metre: how fast it thins going up. 0 is the same everywhere.
    pub falloff: f32,
    /// Metres of fog at the base that swallow 95% of what is behind it.
    pub distance: f32,
    /// Metres from the camera before any fog starts.
    pub start: f32,
    /// The fog's color with the sun high, low and gone.
    pub day_color: String,
    pub dusk_color: String,
    pub night_color: String,
    /// Extra glow looking towards the sun, as a fraction of its light.
    pub sun_boost: f32,
    /// How tight that glow is, 0-0.99.
    pub sun_boost_g: f32,
}

impl Default for HeightFog {
    fn default() -> Self {
        Self {
            enabled: false,
            base_height: 0.0,
            falloff: 0.05,
            distance: 400.0,
            start: 0.0,
            day_color: "#C2CAD2".to_string(),
            dusk_color: "#E8A778".to_string(),
            night_color: "#2A3344".to_string(),
            sun_boost: 0.5,
            sun_boost_g: 0.75,
        }
    }
}

/// Resolution of the froxel grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FogQuality {
    Low,
    #[default]
    Medium,
    High,
}

impl FogQuality {
    /// Froxels across, down and deep.
    pub fn grid(self) -> [u32; 3] {
        match self {
            FogQuality::Low => [80, 45, 48],
            FogQuality::Medium => [128, 72, 64],
            FogQuality::High => [160, 90, 96],
        }
    }
}

/// A participating medium in a froxel grid over the camera's first `range`
/// metres: shadowed sunlight and moonlight, the lights that ask for it and
/// the ambient light scatter in it, it glows by itself, and noise drifts
/// through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumetricFog {
    pub enabled: bool,
    /// Extinction per metre at the base height.
    pub density: f32,
    pub base_height: f32,
    /// Per metre going up, as for height fog.
    pub falloff: f32,
    /// Henyey-Greenstein g, -0.9 to 0.9: positive glows towards the light.
    pub anisotropy: f32,
    /// How much of what it stops it scatters, per channel.
    pub albedo: String,
    /// Glow of its own, for neon smog.
    pub emissive: String,
    /// Nits of the emissive color at a density of 1 per metre.
    pub emissive_strength: f32,
    /// 0-1: how much noise carves the fog.
    pub noise: f32,
    /// Metres across one noise feature.
    pub noise_scale: f32,
    /// Metres per second the noise drifts.
    pub noise_wind: [f32; 3],
    /// Metres from the camera the grid covers.
    pub range: f32,
    pub quality: FogQuality,
    /// Whether the sun and moon light it. Each light has its own switch.
    pub sun: bool,
    /// Multiplier on the ambient light it scatters.
    pub ambient: f32,
    /// Dust motes hanging in the air near the ground around the camera,
    /// whether or not the froxels are on.
    pub dust: Motes,
    /// Metres above `base_height` the dust reaches.
    pub dust_height: f32,
}

impl Default for VolumetricFog {
    fn default() -> Self {
        Self {
            enabled: false,
            density: 0.02,
            base_height: 0.0,
            falloff: 0.1,
            anisotropy: 0.6,
            albedo: "#FFFFFF".to_string(),
            emissive: "#000000".to_string(),
            emissive_strength: 0.0,
            noise: 0.5,
            noise_scale: 12.0,
            noise_wind: [1.0, 0.0, 0.5],
            range: 96.0,
            quality: FogQuality::Medium,
            sun: true,
            ambient: 1.0,
            dust: Motes {
                count: 600,
                ..Motes::default()
            },
            dust_height: 3.0,
        }
    }
}

/// Distance haze: far things take on the sky's color and lose theirs,
/// less so higher up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AerialPerspective {
    pub enabled: bool,
    /// Metres at which half of a far object's own light is gone.
    pub distance: f32,
    /// The haze's color where there is no sky to take it from, and a tint
    /// on the sky's where there is.
    pub tint: String,
    /// 0-1: how gray far things turn.
    pub desaturation: f32,
    /// Metres over which the haze thins going up, so peaks stay crisp.
    pub height_scale: f32,
    /// How much more blue fades than red, 0 gray to 1 fully Rayleigh.
    pub blue_shift: f32,
}

impl Default for AerialPerspective {
    fn default() -> Self {
        Self {
            enabled: false,
            distance: 8000.0,
            tint: "#FFFFFF".to_string(),
            desaturation: 0.4,
            height_scale: 1200.0,
            blue_shift: 0.7,
        }
    }
}

/// Fog a `Volume` adds inside its shape, fading out across its blend
/// distance. Only volumetric fog reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalFog {
    pub enabled: bool,
    /// Extinction per metre added at full coverage.
    pub density: f32,
    pub albedo: String,
    pub emissive: String,
    /// Nits of the emissive color at a density of 1 per metre.
    pub emissive_strength: f32,
}

impl Default for LocalFog {
    fn default() -> Self {
        Self {
            enabled: false,
            density: 0.2,
            albedo: "#FFFFFF".to_string(),
            emissive: "#000000".to_string(),
            emissive_strength: 0.0,
        }
    }
}

/// How a light's beam is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BeamMode {
    /// In the froxels while volumetric fog is on above Low quality, as a
    /// shaft cone otherwise.
    #[default]
    Auto,
    /// Always in the froxels, which then run for the beam alone.
    Volumetric,
    /// Always a shaft cone: additive geometry, cheap, spots only.
    Shaft,
}

/// A visible beam: extra medium inside a spot's cone (or around a point)
/// that only its own light scatters. Scaled by `set fog density`, so it
/// thickens in fog and vanishes on a clear day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beam {
    /// Extinction per metre added in the beam. 0 for no beam.
    pub density: f32,
    /// Henyey-Greenstein g for this light alone, or the fog's when unset.
    pub anisotropy: Option<f32>,
    /// Exponent on `1 - distance / range`: 0 flat, 1 linear, 2 quadratic.
    pub falloff: f32,
    /// Metres from the light over which the beam fades in.
    pub near_fade: f32,
    /// Metres before the range over which it fades out.
    pub far_fade: f32,
    pub mode: BeamMode,
    /// Shaft cone brightness multiplier.
    pub shaft_intensity: f32,
    /// How much the shaft's noise carves it, 0-1, and how fast it scrolls.
    pub shaft_noise: f32,
    pub shaft_scroll: f32,
    pub motes: Motes,
}

impl Default for Beam {
    fn default() -> Self {
        Self {
            density: 0.0,
            anisotropy: None,
            falloff: 1.0,
            near_fade: 0.5,
            far_fade: 2.0,
            mode: BeamMode::Auto,
            shaft_intensity: 1.0,
            shaft_noise: 0.4,
            shaft_scroll: 0.3,
            motes: Motes::default(),
        }
    }
}

/// Billboard dust drifting in a beam or in the air.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Motes {
    pub enabled: bool,
    pub count: u32,
    /// Metres across one mote.
    pub size: f32,
    /// 0-1 opacity at full light.
    pub alpha: f32,
    /// 0-1: how much each mote flickers.
    pub twinkle: f32,
    /// Metres per second each mote wanders.
    pub drift: f32,
}

impl Default for Motes {
    fn default() -> Self {
        Self {
            enabled: false,
            count: 160,
            size: 0.012,
            alpha: 0.6,
            twinkle: 0.5,
            drift: 0.05,
        }
    }
}

/// Most motes one field draws.
pub const MAX_MOTES: u32 = 4096;

impl Motes {
    pub fn normalize(&mut self) {
        let d = Motes::default();
        self.count = self.count.min(MAX_MOTES);
        self.size = finite(self.size, d.size).clamp(0.001, 1.0);
        self.alpha = finite(self.alpha, d.alpha).clamp(0.0, 1.0);
        self.twinkle = finite(self.twinkle, d.twinkle).clamp(0.0, 1.0);
        self.drift = finite(self.drift, d.drift).clamp(0.0, 10.0);
    }
}

impl Beam {
    pub fn normalize(&mut self) {
        let d = Beam::default();
        self.density = finite(self.density, 0.0).clamp(0.0, 10.0);
        self.anisotropy = self
            .anisotropy
            .filter(|g| g.is_finite())
            .map(|g| g.clamp(-0.9, 0.9));
        self.falloff = finite(self.falloff, d.falloff).clamp(0.0, 8.0);
        self.near_fade = finite(self.near_fade, d.near_fade).clamp(0.0, 1000.0);
        self.far_fade = finite(self.far_fade, d.far_fade).clamp(0.0, 1000.0);
        self.shaft_intensity = finite(self.shaft_intensity, 1.0).clamp(0.0, 1000.0);
        self.shaft_noise = finite(self.shaft_noise, d.shaft_noise).clamp(0.0, 1.0);
        self.shaft_scroll = finite(self.shaft_scroll, d.shaft_scroll).clamp(-100.0, 100.0);
        self.motes.normalize();
    }

    /// Whether this beam is a shaft cone rather than froxel medium, given
    /// the project's volumetric fog.
    pub fn uses_shaft(&self, fog: &VolumetricFog) -> bool {
        match self.mode {
            BeamMode::Shaft => true,
            BeamMode::Volumetric => false,
            BeamMode::Auto => !fog.enabled || fog.quality == FogQuality::Low,
        }
    }
}

/// How much of a beam is left `distance` metres from its light: the
/// falloff curve over the range, faded in near the light and out near the
/// range. Mirrors `beam_fade` in `shaders/fog.wesl`.
pub fn beam_fade(distance: f32, range: f32, near: f32, far: f32, curve: f32) -> f32 {
    let range = range.max(0.01);
    let t = (distance / range).clamp(0.0, 1.0);
    let mut fade = (1.0 - t).max(1e-6).powf(curve);
    if near > 0.0 {
        fade *= smoothstep(0.0, near, distance);
    }
    if far > 0.0 {
        fade *= 1.0 - smoothstep(range - far, range, distance);
    }
    fade
}

/// What `set fog density to` multiplies volumetric fog and beams by: the
/// asked density over the project's own height fog (or the default one's
/// when the project has none), so 0 clears every beam.
pub fn fog_scale(fog: &Fog, density: f32) -> f32 {
    let distance = if fog.height.enabled {
        fog.height.distance
    } else {
        HeightFog::default().distance
    };
    (density.max(0.0) / density_for_distance(distance)).clamp(0.0, 50.0)
}

/// Extinction per metre that swallows 95% of the light over `distance`:
/// e^-3 is 5%.
pub fn density_for_distance(distance: f32) -> f32 {
    3.0 / distance.max(0.01)
}

pub fn distance_for_density(density: f32) -> f32 {
    3.0 / density.max(1e-6)
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

impl Fog {
    /// Pulls every number into the range the renderer accepts.
    pub fn normalize(&mut self) {
        let h = &mut self.height;
        let d = HeightFog::default();
        h.base_height = finite(h.base_height, 0.0).clamp(-100_000.0, 100_000.0);
        h.falloff = finite(h.falloff, d.falloff).clamp(0.0, 10.0);
        h.distance = finite(h.distance, d.distance).clamp(0.1, 1.0e7);
        h.start = finite(h.start, 0.0).clamp(0.0, 1.0e6);
        h.sun_boost = finite(h.sun_boost, d.sun_boost).clamp(0.0, 100.0);
        h.sun_boost_g = finite(h.sun_boost_g, d.sun_boost_g).clamp(0.0, 0.99);

        let v = &mut self.volumetric;
        let d = VolumetricFog::default();
        v.density = finite(v.density, d.density).clamp(0.0, 10.0);
        v.base_height = finite(v.base_height, 0.0).clamp(-100_000.0, 100_000.0);
        v.falloff = finite(v.falloff, d.falloff).clamp(0.0, 10.0);
        v.anisotropy = finite(v.anisotropy, d.anisotropy).clamp(-0.9, 0.9);
        v.emissive_strength = finite(v.emissive_strength, 0.0).clamp(0.0, 1.0e6);
        v.noise = finite(v.noise, d.noise).clamp(0.0, 1.0);
        v.noise_scale = finite(v.noise_scale, d.noise_scale).clamp(0.1, 10_000.0);
        v.noise_wind = v.noise_wind.map(|w| finite(w, 0.0).clamp(-1000.0, 1000.0));
        v.range = finite(v.range, d.range).clamp(4.0, 2000.0);
        v.ambient = finite(v.ambient, 1.0).clamp(0.0, 100.0);
        v.dust.normalize();
        v.dust_height = finite(v.dust_height, 3.0).clamp(0.0, 10_000.0);

        let a = &mut self.aerial;
        let d = AerialPerspective::default();
        a.distance = finite(a.distance, d.distance).clamp(1.0, 1.0e7);
        a.desaturation = finite(a.desaturation, d.desaturation).clamp(0.0, 1.0);
        a.height_scale = finite(a.height_scale, d.height_scale).clamp(1.0, 100_000.0);
        a.blue_shift = finite(a.blue_shift, d.blue_shift).clamp(0.0, 1.0);
    }

    /// The colors, by name, for a caller that checks them.
    pub fn colors_mut(&mut self) -> [(&'static str, &mut String); 6] {
        [
            ("fog day", &mut self.height.day_color),
            ("fog dusk", &mut self.height.dusk_color),
            ("fog night", &mut self.height.night_color),
            ("fog albedo", &mut self.volumetric.albedo),
            ("fog emissive", &mut self.volumetric.emissive),
            ("haze tint", &mut self.aerial.tint),
        ]
    }
}

impl LocalFog {
    pub fn normalize(&mut self) {
        self.density = finite(self.density, 0.2).clamp(0.0, 10.0);
        self.emissive_strength = finite(self.emissive_strength, 0.0).clamp(0.0, 1.0e6);
    }
}

/// How the fog's color moves with the sun: `[day, dusk, night]` weights
/// summing to 1, from the sine of the sun's elevation. Gray at noon, warm
/// from about 12 degrees down to the horizon, night once it is 6 below.
pub fn color_weights(sun_height: f32) -> [f32; 3] {
    let night = smoothstep(0.0, -0.1, sun_height);
    let day = smoothstep(0.0, 0.2, sun_height);
    let dusk = (1.0 - day - night).max(0.0);
    [day, dusk, night]
}

/// Optical depth of exponential height fog along a ray from `origin_y`
/// rising `dir_y` per unit length, over `length`: the integral of
/// `density * e^(-falloff * (y - base))`. Mirrors `height_fog_depth` in
/// the runtime's `shaders/fog.wesl`.
pub fn height_fog_depth(
    density: f32,
    falloff: f32,
    base: f32,
    origin_y: f32,
    dir_y: f32,
    length: f32,
) -> f32 {
    let at_origin = density * (-falloff * (origin_y - base)).clamp(-80.0, 80.0).exp();
    let k = falloff * dir_y;
    if k.abs() < 1e-5 {
        return at_origin * length;
    }
    at_origin * (1.0 - (-(k * length).clamp(-80.0, 80.0)).exp()) / k
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_and_density_are_each_others_inverse() {
        let density = density_for_distance(300.0);
        assert!((density - 0.01).abs() < 1e-6);
        assert!((distance_for_density(density) - 300.0).abs() < 1e-3);
        // 95% gone at the extinction distance.
        assert!(((-density * 300.0).exp() - 0.0498).abs() < 1e-3);
    }

    #[test]
    fn height_fog_thins_upwards_and_matches_a_march() {
        let flat = height_fog_depth(0.01, 0.0, 0.0, 50.0, 0.3, 100.0);
        assert!((flat - 1.0).abs() < 1e-5);
        let level = height_fog_depth(0.01, 0.05, 0.0, 0.0, 0.0, 100.0);
        assert!((level - 1.0).abs() < 1e-5);
        let up = height_fog_depth(0.01, 0.05, 0.0, 0.0, 0.5, 100.0);
        let down = height_fog_depth(0.01, 0.05, 0.0, 10.0, -0.1, 100.0);
        assert!(up < level);
        // A numeric march agrees with the closed form.
        let (steps, length, dir_y, y0) = (10_000, 100.0f32, -0.1f32, 10.0f32);
        let dt = length / steps as f32;
        let marched: f32 = (0..steps)
            .map(|i| {
                let y = y0 + dir_y * (i as f32 + 0.5) * dt;
                0.01 * (-0.05 * y).exp() * dt
            })
            .sum();
        assert!((marched - down).abs() < 1e-3, "{marched} {down}");
    }

    #[test]
    fn fog_color_warms_at_dusk_and_darkens_at_night() {
        assert_eq!(color_weights(0.9), [1.0, 0.0, 0.0]);
        let dusk = color_weights(0.03);
        assert!(dusk[1] > 0.8, "{dusk:?}");
        assert_eq!(color_weights(-0.5), [0.0, 0.0, 1.0]);
        for h in [-0.3, -0.05, 0.0, 0.1, 0.5] {
            let w = color_weights(h);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn normalize_pulls_values_into_range_and_old_documents_default() {
        let fog: Fog = serde_json::from_str("{}").unwrap();
        assert_eq!(fog, Fog::default());
        let mut fog = Fog::default();
        fog.height.distance = -5.0;
        fog.volumetric.anisotropy = 3.0;
        fog.volumetric.noise_wind = [f32::NAN, 1.0, 2.0];
        fog.aerial.desaturation = 2.0;
        fog.normalize();
        assert_eq!(fog.height.distance, 0.1);
        assert_eq!(fog.volumetric.anisotropy, 0.9);
        assert_eq!(fog.volumetric.noise_wind, [0.0, 1.0, 2.0]);
        assert_eq!(fog.aerial.desaturation, 1.0);
    }

    #[test]
    fn a_beam_fades_in_at_the_light_and_out_at_its_range() {
        assert!(beam_fade(0.0, 10.0, 1.0, 2.0, 1.0) < 1e-5);
        assert!(beam_fade(10.0, 10.0, 1.0, 2.0, 1.0) < 1e-5);
        // Flat curve with no fades is the whole beam.
        assert!((beam_fade(5.0, 10.0, 0.0, 0.0, 0.0) - 1.0).abs() < 1e-5);
        let linear = beam_fade(5.0, 10.0, 0.0, 0.0, 1.0);
        let quadratic = beam_fade(5.0, 10.0, 0.0, 0.0, 2.0);
        assert!((linear - 0.5).abs() < 1e-5 && (quadratic - 0.25).abs() < 1e-5);
        assert!(beam_fade(10.0, 10.0, 0.0, 0.0, 0.0).is_finite());
    }

    #[test]
    fn fog_density_scales_beams_against_the_projects_height_fog() {
        let mut fog = Fog::default();
        let default = density_for_distance(HeightFog::default().distance);
        assert!((fog_scale(&fog, default) - 1.0).abs() < 1e-5);
        assert_eq!(fog_scale(&fog, 0.0), 0.0);
        fog.height.enabled = true;
        fog.height.distance = 300.0;
        assert!((fog_scale(&fog, 0.02) - 2.0).abs() < 1e-4);
        assert_eq!(fog_scale(&fog, 1000.0), 50.0);
    }

    #[test]
    fn auto_beams_fall_back_to_shafts_without_good_froxels() {
        let mut fog = VolumetricFog::default();
        let beam = Beam::default();
        assert!(beam.uses_shaft(&fog));
        fog.enabled = true;
        assert!(!beam.uses_shaft(&fog));
        fog.quality = FogQuality::Low;
        assert!(beam.uses_shaft(&fog));
        let froxel = Beam {
            mode: BeamMode::Volumetric,
            ..Beam::default()
        };
        assert!(!froxel.uses_shaft(&fog));
    }

    #[test]
    fn beams_and_motes_normalize() {
        let mut beam: Beam = serde_json::from_str("{}").unwrap();
        assert_eq!(beam, Beam::default());
        beam.density = -1.0;
        beam.anisotropy = Some(4.0);
        beam.motes.count = 1_000_000;
        beam.normalize();
        assert_eq!(beam.density, 0.0);
        assert_eq!(beam.anisotropy, Some(0.9));
        assert_eq!(beam.motes.count, MAX_MOTES);
    }

    #[test]
    fn quality_picks_the_froxel_grid() {
        assert_eq!(FogQuality::Medium.grid(), [128, 72, 64]);
        assert!(FogQuality::Low.grid()[2] < FogQuality::High.grid()[2]);
    }
}
