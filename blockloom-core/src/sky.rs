//! The 3D sky: what the world draws behind everything, and what its ambient
//! light and reflections come from. One of four kinds - a flat color, a
//! physical atmosphere, a stylized gradient or an HDR image - and whichever
//! it is feeds the background, the diffuse light and the reflections
//! together, so they never disagree.
//!
//! This is the model and the CPU half of the atmosphere: where the sun and
//! moon stand and how much sunlight gets through the air to the ground. The
//! runtime renders the rest (`blockloom-runtime/src/sky.rs`).

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Which sky a world has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SkyKind {
    /// The world's background color and flat ambient light: no sky at all.
    #[default]
    Flat,
    /// Rayleigh, Mie and ozone scattering of the sun, plus a sun and a moon.
    Physical,
    /// Three color stops, cheap and stylized.
    Gradient,
    /// An HDR panorama or strip of cube faces.
    Hdri,
}

/// Where the sun's position comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SunMode {
    /// The lighting's own light direction.
    #[default]
    Light,
    /// Azimuth and elevation typed in.
    Manual,
    /// Latitude, longitude, day and time.
    Geographic,
}

/// Where the sun stands. Azimuth is degrees clockwise from north (-Z)
/// towards east (+X), elevation degrees above the horizon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SunPlacement {
    pub mode: SunMode,
    pub azimuth: f32,
    pub elevation: f32,
    /// Degrees, north positive.
    pub latitude: f32,
    /// Degrees, east positive.
    pub longitude: f32,
    /// 1-365.
    pub day_of_year: u32,
    /// Local clock time in hours, 0-24.
    pub time_of_day: f32,
    /// Hours the local clock is ahead of UTC.
    pub utc_offset: f32,
}

impl Default for SunPlacement {
    fn default() -> Self {
        // The default light direction, [8, 16, 8].
        Self {
            mode: SunMode::Light,
            azimuth: 135.0,
            elevation: 54.7,
            latitude: 40.0,
            longitude: 0.0,
            day_of_year: 172,
            time_of_day: 12.0,
            utc_offset: 0.0,
        }
    }
}

/// A physically based atmosphere, in the units of Hillaire's model:
/// scattering in 1/Mm, heights and radii in km.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhysicalSky {
    /// Degrees across. The real sun is about 0.53.
    pub sun_size: f32,
    /// 0-1: how much darker the disk's rim is than its centre.
    pub limb_darkening: f32,
    /// Multiplier on the disk's physical brightness.
    pub sun_intensity: f32,
    /// Redden and dim the sun's light by the air it comes through, so a
    /// low sun lights the world orange and a set one not at all.
    pub tint_sun: bool,
    pub moon: bool,
    /// Degrees across.
    pub moon_size: f32,
    /// 0 new, 0.5 full, 1 new again.
    pub moon_phase: f32,
    /// Nits of the lit part of the disk.
    pub moon_brightness: f32,
    /// Glow around the moon, as a fraction of its brightness.
    pub moon_halo: f32,
    /// How tight the glow is; higher hugs the disk.
    pub moon_halo_power: f32,
    /// Where the moon stands unless the sun is geographic, when it trails
    /// the sun by its phase.
    pub moon_azimuth: f32,
    pub moon_elevation: f32,
    /// Rayleigh scattering per color channel, 1/Mm.
    pub rayleigh: [f32; 3],
    /// Mie scattering, 1/Mm.
    pub mie: f32,
    /// Mie anisotropy g, -1 to 1: how strongly haze glows around the sun.
    pub mie_g: f32,
    /// Ozone absorption per color channel, 1/Mm.
    pub ozone: [f32; 3],
    /// Scale heights, km.
    pub rayleigh_height: f32,
    pub mie_height: f32,
    pub ground_albedo: String,
    /// Reshapes the sky from horizon to zenith: 1 is physical, below 1
    /// spreads the horizon's color up, above 1 pulls it down.
    pub horizon_curve: f32,
    /// Km.
    pub planet_radius: f32,
    pub atmosphere_height: f32,
    /// The sky once the sun has gone, in nits.
    pub night_color: String,
    pub night_brightness: f32,
    /// Sun elevations in degrees where night starts fading in and where it
    /// is complete.
    pub night_ramp: [f32; 2],
    /// Let the moon light the world as a directional light of its own.
    pub moon_light: bool,
    /// Lux under a full moon overhead; a real one gives about 0.25.
    pub moon_lux: f32,
    pub moon_color: String,
    pub moon_shadows: bool,
}

impl Default for PhysicalSky {
    fn default() -> Self {
        Self {
            sun_size: 0.53,
            limb_darkening: 0.6,
            sun_intensity: 1.0,
            tint_sun: true,
            moon: false,
            moon_size: 0.52,
            moon_phase: 0.5,
            moon_brightness: 2500.0,
            moon_halo: 0.03,
            moon_halo_power: 1000.0,
            moon_azimuth: 300.0,
            moon_elevation: 30.0,
            rayleigh: [5.802, 13.558, 33.1],
            mie: 3.996,
            mie_g: 0.8,
            ozone: [0.650, 1.881, 0.085],
            rayleigh_height: 8.0,
            mie_height: 1.2,
            ground_albedo: "#5A5A5A".to_string(),
            horizon_curve: 1.0,
            planet_radius: 6360.0,
            atmosphere_height: 100.0,
            night_color: "#1A2B4D".to_string(),
            night_brightness: 1.0,
            night_ramp: [2.0, -12.0],
            moon_light: true,
            moon_lux: 0.25,
            moon_color: "#C9D6FF".to_string(),
            moon_shadows: false,
        }
    }
}

/// Three color stops around a horizon line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GradientSky {
    pub top: String,
    pub middle: String,
    pub bottom: String,
    /// Moves the horizon line, -1 to 1 (the sine of its elevation).
    pub horizon_offset: f32,
    /// 0-1: how far towards the zenith the horizon's color takes to fade.
    pub softness: f32,
    /// 0-1: how warm the horizon turns while the sun is low.
    pub warmth: f32,
    pub warm_color: String,
    /// Noise under the gradient against banding.
    pub dither: bool,
    /// Nits of a color of 1.0.
    pub brightness: f32,
}

impl Default for GradientSky {
    fn default() -> Self {
        Self {
            top: "#2F6BC4".to_string(),
            middle: "#A9CBE8".to_string(),
            bottom: "#3A3F47".to_string(),
            horizon_offset: 0.0,
            softness: 0.4,
            warmth: 0.5,
            warm_color: "#FF9A50".to_string(),
            dither: true,
            brightness: 1000.0,
        }
    }
}

/// An HDR image: an equirectangular panorama, or a 6:1 / 1:6 strip of cube
/// faces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HdriSky {
    /// `.hdr` or `.exr` asset.
    pub path: String,
    /// Nits of a texel of 1.0.
    pub brightness: f32,
    /// Degrees about the vertical.
    pub rotation: f32,
    /// Degrees the image leans forward, for a panorama shot off level.
    pub tilt: f32,
    pub tint: String,
    /// 0-1: how blurred the background is. The light is filtered anyway.
    pub blur: f32,
    /// Degrees either side of a panorama's wrap seam over which its two
    /// edges are cross-faded, for a pano that doesn't quite meet. 0 is off.
    pub seam_fix: f32,
}

impl Default for HdriSky {
    fn default() -> Self {
        Self {
            path: String::new(),
            brightness: 1000.0,
            rotation: 0.0,
            tilt: 0.0,
            tint: "#FFFFFF".to_string(),
            blur: 0.0,
            seam_fix: 0.0,
        }
    }
}

/// A procedural star field over any sky but a flat one, fading in as the
/// sun goes down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stars {
    pub enabled: bool,
    /// 0-1: the share of the sky's cells holding a star.
    pub density: f32,
    /// Nits of the brightest star.
    pub brightness: f32,
    /// How steeply faint stars outnumber bright ones: higher is fainter.
    pub magnitude_slope: f32,
    /// 0-1: how far colors spread from white towards red and blue.
    pub color_variation: f32,
    /// 0-1: how much a star's brightness flickers.
    pub twinkle: f32,
    /// Flickers a second.
    pub twinkle_speed: f32,
    /// Degrees above the horizon over which stars fade out.
    pub horizon_fade: f32,
    /// Sun elevations in degrees where stars start to show and where they
    /// are at full strength.
    pub sun_fade: [f32; 2],
    /// A panorama of the Milky Way, laid over the field. Empty for none.
    pub milky_way: String,
    /// Nits of a texel of 1.0.
    pub milky_way_brightness: f32,
    /// Degrees the band is turned about the vertical, and tilted.
    pub milky_way_rotation: f32,
    pub milky_way_tilt: f32,
}

impl Default for Stars {
    fn default() -> Self {
        Self {
            enabled: false,
            density: 0.35,
            brightness: 60.0,
            magnitude_slope: 3.0,
            color_variation: 0.5,
            twinkle: 0.3,
            twinkle_speed: 1.5,
            horizon_fade: 8.0,
            sun_fade: [-2.0, -14.0],
            milky_way: String::new(),
            milky_way_brightness: 2.0,
            milky_way_rotation: 0.0,
            milky_way_tilt: 60.0,
        }
    }
}

/// Curtains of aurora high over the camera, flowing slowly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Aurora {
    pub enabled: bool,
    /// The KP index, 0-9: 0 is none, 9 fills the sky from the pole down.
    pub kp: f32,
    /// 1-3 sheets, one behind the other.
    pub layers: u32,
    /// Km to the curtains' foot, and how tall they hang.
    pub altitude: f32,
    pub height: f32,
    /// Km across one fold of a curtain.
    pub width: f32,
    /// Km between the fine vertical rays.
    pub ray_scale: f32,
    pub bottom_color: String,
    pub top_color: String,
    /// Nits at the brightest.
    pub brightness: f32,
    /// How fast the folds flow.
    pub speed: f32,
    /// 0-1: a diffuse glow along the poleward horizon.
    pub horizon_glow: f32,
    /// Degrees clockwise from north the pole lies towards.
    pub pole_azimuth: f32,
}

impl Default for Aurora {
    fn default() -> Self {
        Self {
            enabled: false,
            kp: 4.0,
            layers: 2,
            altitude: 100.0,
            height: 150.0,
            width: 60.0,
            ray_scale: 1.5,
            bottom_color: "#38FF8A".to_string(),
            top_color: "#A64DFF".to_string(),
            brightness: 8.0,
            speed: 1.0,
            horizon_glow: 0.3,
            pole_azimuth: 0.0,
        }
    }
}

/// The sky, and how much of the world it lights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sky {
    pub kind: SkyKind,
    pub sun: SunPlacement,
    pub physical: PhysicalSky,
    pub gradient: GradientSky,
    pub hdri: HdriSky,
    /// Draw it behind the world. Off shows the background color instead.
    pub background: bool,
    /// Take reflections from it.
    pub reflections: bool,
    /// Take diffuse ambient light from it.
    pub lighting: bool,
    /// Multiplier on its diffuse light alone.
    pub ambient_dimmer: f32,
    /// EV added to the sky's own brightness, on top of the camera's
    /// exposure rather than instead of it.
    pub exposure: f32,
    pub stars: Stars,
    pub aurora: Aurora,
}

impl Default for Sky {
    fn default() -> Self {
        Self {
            kind: SkyKind::Flat,
            sun: SunPlacement::default(),
            physical: PhysicalSky::default(),
            gradient: GradientSky::default(),
            hdri: HdriSky::default(),
            background: true,
            reflections: true,
            lighting: true,
            ambient_dimmer: 1.0,
            exposure: 0.0,
            stars: Stars::default(),
            aurora: Aurora::default(),
        }
    }
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn triple(value: [f32; 3], fallback: [f32; 3], max: f32) -> [f32; 3] {
    std::array::from_fn(|i| finite(value[i], fallback[i]).clamp(0.0, max))
}

impl Sky {
    /// Pulls every number into the range the renderer accepts.
    pub fn normalize(&mut self) {
        let sun = &mut self.sun;
        let defaults = SunPlacement::default();
        sun.azimuth = finite(sun.azimuth, defaults.azimuth).rem_euclid(360.0);
        sun.elevation = finite(sun.elevation, defaults.elevation).clamp(-90.0, 90.0);
        sun.latitude = finite(sun.latitude, defaults.latitude).clamp(-90.0, 90.0);
        sun.longitude = finite(sun.longitude, defaults.longitude).clamp(-180.0, 180.0);
        sun.day_of_year = sun.day_of_year.clamp(1, 365);
        sun.time_of_day = finite(sun.time_of_day, defaults.time_of_day).rem_euclid(24.0);
        sun.utc_offset = finite(sun.utc_offset, 0.0).clamp(-14.0, 14.0);

        let p = &mut self.physical;
        let d = PhysicalSky::default();
        p.sun_size = finite(p.sun_size, d.sun_size).clamp(0.05, 20.0);
        p.limb_darkening = finite(p.limb_darkening, d.limb_darkening).clamp(0.0, 1.0);
        p.sun_intensity = finite(p.sun_intensity, 1.0).clamp(0.0, 100.0);
        p.moon_size = finite(p.moon_size, d.moon_size).clamp(0.05, 20.0);
        p.moon_phase = finite(p.moon_phase, d.moon_phase).rem_euclid(1.0);
        p.moon_brightness = finite(p.moon_brightness, d.moon_brightness).clamp(0.0, 1.0e6);
        p.moon_halo = finite(p.moon_halo, d.moon_halo).clamp(0.0, 1.0);
        p.moon_halo_power = finite(p.moon_halo_power, d.moon_halo_power).clamp(1.0, 100_000.0);
        p.moon_azimuth = finite(p.moon_azimuth, d.moon_azimuth).rem_euclid(360.0);
        p.moon_elevation = finite(p.moon_elevation, d.moon_elevation).clamp(-90.0, 90.0);
        p.rayleigh = triple(p.rayleigh, d.rayleigh, 1000.0);
        p.mie = finite(p.mie, d.mie).clamp(0.0, 1000.0);
        p.mie_g = finite(p.mie_g, d.mie_g).clamp(-0.99, 0.99);
        p.ozone = triple(p.ozone, d.ozone, 1000.0);
        p.rayleigh_height = finite(p.rayleigh_height, d.rayleigh_height).clamp(0.1, 100.0);
        p.mie_height = finite(p.mie_height, d.mie_height).clamp(0.1, 100.0);
        p.horizon_curve = finite(p.horizon_curve, 1.0).clamp(0.1, 10.0);
        p.planet_radius = finite(p.planet_radius, d.planet_radius).clamp(1.0, 100_000.0);
        p.atmosphere_height = finite(p.atmosphere_height, d.atmosphere_height).clamp(1.0, 1000.0);
        p.night_brightness = finite(p.night_brightness, d.night_brightness).clamp(0.0, 100_000.0);
        p.night_ramp = [
            finite(p.night_ramp[0], d.night_ramp[0]).clamp(-90.0, 90.0),
            finite(p.night_ramp[1], d.night_ramp[1]).clamp(-90.0, 90.0),
        ];

        let g = &mut self.gradient;
        g.horizon_offset = finite(g.horizon_offset, 0.0).clamp(-1.0, 1.0);
        g.softness = finite(g.softness, 0.4).clamp(0.001, 1.0);
        g.warmth = finite(g.warmth, 0.5).clamp(0.0, 1.0);
        g.brightness = finite(g.brightness, 1000.0).clamp(0.0, 1.0e6);

        let h = &mut self.hdri;
        h.path = h.path.trim().replace('\\', "/");
        h.brightness = finite(h.brightness, 1000.0).clamp(0.0, 1.0e6);
        h.rotation = finite(h.rotation, 0.0).rem_euclid(360.0);
        h.tilt = finite(h.tilt, 0.0).clamp(-90.0, 90.0);
        h.blur = finite(h.blur, 0.0).clamp(0.0, 1.0);
        h.seam_fix = finite(h.seam_fix, 0.0).clamp(0.0, 45.0);

        p.moon_lux = finite(p.moon_lux, d.moon_lux).clamp(0.0, 1.0e5);

        self.ambient_dimmer = finite(self.ambient_dimmer, 1.0).clamp(0.0, 10.0);
        self.exposure = finite(self.exposure, 0.0).clamp(-16.0, 16.0);

        let s = &mut self.stars;
        let d = Stars::default();
        s.density = finite(s.density, d.density).clamp(0.0, 1.0);
        s.brightness = finite(s.brightness, d.brightness).clamp(0.0, 1.0e6);
        s.magnitude_slope = finite(s.magnitude_slope, d.magnitude_slope).clamp(0.5, 10.0);
        s.color_variation = finite(s.color_variation, d.color_variation).clamp(0.0, 1.0);
        s.twinkle = finite(s.twinkle, d.twinkle).clamp(0.0, 1.0);
        s.twinkle_speed = finite(s.twinkle_speed, d.twinkle_speed).clamp(0.0, 50.0);
        s.horizon_fade = finite(s.horizon_fade, d.horizon_fade).clamp(0.0, 90.0);
        s.sun_fade = [
            finite(s.sun_fade[0], d.sun_fade[0]).clamp(-90.0, 90.0),
            finite(s.sun_fade[1], d.sun_fade[1]).clamp(-90.0, 90.0),
        ];
        s.milky_way = s.milky_way.trim().replace('\\', "/");
        s.milky_way_brightness = finite(s.milky_way_brightness, 2.0).clamp(0.0, 1.0e6);
        s.milky_way_rotation = finite(s.milky_way_rotation, 0.0).rem_euclid(360.0);
        s.milky_way_tilt = finite(s.milky_way_tilt, d.milky_way_tilt).clamp(-90.0, 90.0);

        let a = &mut self.aurora;
        let d = Aurora::default();
        a.kp = finite(a.kp, d.kp).clamp(0.0, 9.0);
        a.layers = a.layers.clamp(1, 3);
        a.altitude = finite(a.altitude, d.altitude).clamp(1.0, 1000.0);
        a.height = finite(a.height, d.height).clamp(1.0, 1000.0);
        a.width = finite(a.width, d.width).clamp(0.1, 10_000.0);
        a.ray_scale = finite(a.ray_scale, d.ray_scale).clamp(0.01, 1000.0);
        a.brightness = finite(a.brightness, d.brightness).clamp(0.0, 1.0e6);
        a.speed = finite(a.speed, d.speed).clamp(0.0, 100.0);
        a.horizon_glow = finite(a.horizon_glow, d.horizon_glow).clamp(0.0, 1.0);
        a.pole_azimuth = finite(a.pole_azimuth, 0.0).rem_euclid(360.0);
    }

    /// The colors, by name, for a caller that checks them.
    pub fn colors_mut(&mut self) -> [(&'static str, &mut String); 10] {
        [
            ("ground", &mut self.physical.ground_albedo),
            ("night", &mut self.physical.night_color),
            ("moon", &mut self.physical.moon_color),
            ("top", &mut self.gradient.top),
            ("middle", &mut self.gradient.middle),
            ("bottom", &mut self.gradient.bottom),
            ("warm", &mut self.gradient.warm_color),
            ("tint", &mut self.hdri.tint),
            ("aurora bottom", &mut self.aurora.bottom_color),
            ("aurora top", &mut self.aurora.top_color),
        ]
    }

    /// Whether anything is drawn over the sky beyond its kind: stars or an
    /// aurora.
    pub fn has_space(&self) -> bool {
        self.stars.enabled || (self.aurora.enabled && self.aurora.kp > 0.0)
    }

    /// 0 by day to 1 once the sun is low enough for stars, on `sun_fade`.
    pub fn star_visibility(&self, towards_sun: [f32; 3]) -> f32 {
        let [start, end] = self.stars.sun_fade;
        ramp_on_elevation(towards_sun, start, end)
    }

    /// Lux the moon lights the world with, 0 with no moon light: its
    /// brightness by its phase and by how high it stands.
    pub fn moon_illuminance(&self) -> f32 {
        let p = &self.physical;
        if self.active_kind() != SkyKind::Physical || !p.moon || !p.moon_light {
            return 0.0;
        }
        let lit = 0.5 - 0.5 * (p.moon_phase * std::f32::consts::TAU).cos();
        let up = Vec3::from_array(self.moon_direction()).y;
        // Fades across the horizon like the sun does.
        let risen = smoothstep(-0.02, 0.06, up);
        p.moon_lux * lit * risen
    }

    /// The kind that actually draws: an HDRI with no file is no sky.
    pub fn active_kind(&self) -> SkyKind {
        match self.kind {
            SkyKind::Hdri if self.hdri.path.is_empty() => SkyKind::Flat,
            kind => kind,
        }
    }

    /// Towards the sun, unit length. `light_direction` is the lighting's own,
    /// which the `Light` mode keeps; a zero one points straight up.
    pub fn sun_direction(&self, light_direction: [f32; 3]) -> [f32; 3] {
        let sun = &self.sun;
        let dir = match sun.mode {
            SunMode::Light => Vec3::from_array(light_direction)
                .try_normalize()
                .unwrap_or(Vec3::Y),
            SunMode::Manual => direction(sun.azimuth, sun.elevation),
            SunMode::Geographic => {
                let (azimuth, elevation) = solar_position(sun, sun.time_of_day);
                direction(azimuth, elevation)
            }
        };
        dir.to_array()
    }

    /// Towards the moon. A geographic sun takes the moon along: it stands
    /// where the sun stood `phase` of a day ago, so a full moon rises as the
    /// sun sets.
    pub fn moon_direction(&self) -> [f32; 3] {
        let sun = &self.sun;
        let moon = &self.physical;
        let dir = match sun.mode {
            SunMode::Geographic => {
                let hour = sun.time_of_day - moon.moon_phase * 24.0;
                let (azimuth, elevation) = solar_position(sun, hour.rem_euclid(24.0));
                direction(azimuth, elevation)
            }
            _ => direction(moon.moon_azimuth, moon.moon_elevation),
        };
        dir.to_array()
    }

    /// What fraction of each color of sunlight reaches the ground through
    /// the physical atmosphere, 1 everywhere for any other kind or with the
    /// tint off. Fades out as the sun's disk sinks below the horizon.
    pub fn sun_transmittance(&self, towards_sun: [f32; 3]) -> [f32; 3] {
        if self.active_kind() != SkyKind::Physical || !self.physical.tint_sun {
            return [1.0; 3];
        }
        self.physical.transmittance(towards_sun)
    }
}

impl PhysicalSky {
    /// Camera height above the ground the sky is seen from, km.
    pub const EYE_HEIGHT: f32 = 0.05;

    /// Mie extinction over scattering: a little of what haze stops it keeps.
    pub const MIE_EXTINCTION: f32 = 1.0 / 0.9;

    /// Transmittance from the eye to space towards `dir`, with the sun's
    /// disk fading as it sets rather than switching off.
    pub fn transmittance(&self, dir: [f32; 3]) -> [f32; 3] {
        let visible = self.sun_visible(dir);
        if visible <= 0.0 {
            return [0.0; 3];
        }
        self.air_transmittance(dir).map(|c| c * visible)
    }

    /// How much of the sun's disk is above the horizon, 0-1.
    pub fn sun_visible(&self, dir: [f32; 3]) -> f32 {
        let Some(dir) = Vec3::from_array(dir).try_normalize() else {
            return 1.0;
        };
        // Half the disk's angular size, as a sine: how far below the geometric
        // horizon some of the sun still shows.
        let half = (self.sun_size.to_radians() * 0.5).sin();
        let horizon = self.horizon_sine();
        smoothstep(horizon - half, horizon + half, dir.y)
    }

    /// The sine of the horizon's (slightly negative) elevation, seen from
    /// the eye.
    pub fn horizon_sine(&self) -> f32 {
        horizon_sine(self.planet_radius + Self::EYE_HEIGHT, self.planet_radius)
    }

    /// Transmittance through the air towards `dir`, which is bent up to the
    /// horizon if it points below, so a setting sun reads the thickest air
    /// rather than the ground. Mirrors `depth_to_sun` in `shaders/sky.wesl`.
    pub fn air_transmittance(&self, dir: [f32; 3]) -> [f32; 3] {
        let Some(dir) = Vec3::from_array(dir).try_normalize() else {
            return [1.0; 3];
        };
        let radius = self.planet_radius;
        let top = radius + self.atmosphere_height;
        let origin = Vec3::new(0.0, radius + Self::EYE_HEIGHT, 0.0);
        let horizon = self.horizon_sine();
        let ray = if dir.y < horizon {
            let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::X);
            (flat * (1.0 - horizon * horizon).max(0.0).sqrt() + Vec3::Y * horizon).normalize()
        } else {
            dir
        };
        let length = ray_sphere_exit(origin, ray, top);
        const STEPS: usize = 32;
        let step = length / STEPS as f32;
        let (mut rayleigh, mut mie, mut ozone) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..STEPS {
            let p = origin + ray * ((i as f32 + 0.5) * step);
            let h = (p.length() - radius).max(0.0);
            rayleigh += (-h / self.rayleigh_height).exp() * step;
            mie += (-h / self.mie_height).exp() * step;
            ozone += ozone_density(h) * step;
        }
        std::array::from_fn(|c| {
            // 1/Mm over km.
            let depth = (self.rayleigh[c] * rayleigh
                + self.mie * Self::MIE_EXTINCTION * mie
                + self.ozone[c] * ozone)
                * 1.0e-3;
            (-depth).exp()
        })
    }

    /// 0 by day, 1 by night, ramped on the sun's elevation.
    pub fn night(&self, towards_sun: [f32; 3]) -> f32 {
        let [start, end] = self.night_ramp;
        ramp_on_elevation(towards_sun, start, end)
    }
}

/// 0 with the sun above `start` degrees, 1 below `end`, eased between.
fn ramp_on_elevation(towards_sun: [f32; 3], start: f32, end: f32) -> f32 {
    let elevation = Vec3::from_array(towards_sun)
        .normalize_or(Vec3::Y)
        .y
        .clamp(-1.0, 1.0)
        .asin()
        .to_degrees();
    if (start - end).abs() < 1e-3 {
        return if elevation <= end { 1.0 } else { 0.0 };
    }
    smoothstep(start, end, elevation)
}

/// Ozone sits in a layer 25 km up, 30 km thick.
pub fn ozone_density(height: f32) -> f32 {
    (1.0 - (height - 25.0).abs() / 15.0).max(0.0)
}

/// The sine of the geometric horizon's elevation (negative) seen from
/// `height` above the centre of a planet of `radius`.
fn horizon_sine(height: f32, radius: f32) -> f32 {
    -(1.0 - (radius / height).powi(2)).max(0.0).sqrt()
}

/// Distance from `origin`, inside a sphere of `radius` about the origin, to
/// where the ray leaves it.
fn ray_sphere_exit(origin: Vec3, dir: Vec3, radius: f32) -> f32 {
    let b = origin.dot(dir);
    let c = origin.length_squared() - radius * radius;
    (-b + (b * b - c).max(0.0).sqrt()).max(0.0)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Unit vector for an azimuth (clockwise from north, -Z, towards east, +X)
/// and an elevation, both degrees.
pub fn direction(azimuth: f32, elevation: f32) -> Vec3 {
    let (az, el) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos())
}

/// The sun's azimuth and elevation in degrees at `hour` local time, by
/// NOAA's low-precision formulas (good to a fraction of a degree).
pub fn solar_position(sun: &SunPlacement, hour: f32) -> (f32, f32) {
    use std::f32::consts::TAU;
    let day = sun.day_of_year.clamp(1, 365) as f32;
    let g = TAU / 365.0 * (day - 1.0 + (hour - 12.0) / 24.0);
    let equation_of_time = 229.18
        * (0.000075 + 0.001868 * g.cos()
            - 0.032077 * g.sin()
            - 0.014615 * (2.0 * g).cos()
            - 0.040849 * (2.0 * g).sin());
    let declination = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin()
        - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    let minutes = hour * 60.0 + equation_of_time + 4.0 * sun.longitude - 60.0 * sun.utc_offset;
    let hour_angle = (minutes / 4.0 - 180.0).to_radians();
    let latitude = sun.latitude.to_radians();
    let elevation = (latitude.sin() * declination.sin()
        + latitude.cos() * declination.cos() * hour_angle.cos())
    .clamp(-1.0, 1.0)
    .asin();
    // Measured from south, west positive, then turned to measure from north.
    let from_south = hour_angle
        .sin()
        .atan2(hour_angle.cos() * latitude.sin() - declination.tan() * latitude.cos());
    let azimuth = (from_south.to_degrees() + 180.0).rem_euclid(360.0);
    (azimuth, elevation.to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(azimuth: f32, elevation: f32) -> [f32; 3] {
        direction(azimuth, elevation).to_array()
    }

    fn physical() -> Sky {
        Sky {
            kind: SkyKind::Physical,
            ..Sky::default()
        }
    }

    #[test]
    fn the_default_sun_follows_the_light_direction() {
        let sky = Sky::default();
        let dir = Vec3::from_array(sky.sun_direction([8.0, 16.0, 8.0]));
        assert!((dir - Vec3::new(8.0, 16.0, 8.0).normalize()).length() < 1e-5);
        // The manual defaults point the same way.
        let manual = direction(sky.sun.azimuth, sky.sun.elevation);
        assert!((manual - dir).length() < 0.01, "{manual} vs {dir}");
    }

    #[test]
    fn azimuth_turns_clockwise_from_north() {
        assert!((direction(0.0, 0.0) - Vec3::NEG_Z).length() < 1e-5);
        assert!((direction(90.0, 0.0) - Vec3::X).length() < 1e-5);
        assert!((direction(0.0, 90.0) - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn the_geographic_sun_rises_in_the_east_and_peaks_south_at_noon() {
        let sun = SunPlacement {
            mode: SunMode::Geographic,
            latitude: 51.5,
            longitude: 0.0,
            day_of_year: 172,
            ..SunPlacement::default()
        };
        let (azimuth, elevation) = solar_position(&sun, 12.0);
        // Midsummer noon in London: due south, about 62 degrees up.
        assert!((azimuth - 180.0).abs() < 3.0, "{azimuth}");
        assert!((elevation - 62.0).abs() < 1.0, "{elevation}");
        let (morning, _) = solar_position(&sun, 6.0);
        assert!((45.0..135.0).contains(&morning), "{morning}");
        let (_, midnight) = solar_position(&sun, 0.0);
        assert!(midnight < 0.0);
        // Southern winter: the noon sun is to the north.
        let south = SunPlacement {
            latitude: -34.0,
            ..sun
        };
        let (azimuth, _) = solar_position(&south, 12.0);
        assert!(!(90.0..270.0).contains(&azimuth), "{azimuth}");
    }

    #[test]
    fn a_full_moon_stands_opposite_a_geographic_sun() {
        let mut sky = physical();
        sky.sun.mode = SunMode::Geographic;
        sky.sun.latitude = 0.0;
        sky.sun.day_of_year = 80;
        sky.sun.time_of_day = 12.0;
        sky.physical.moon_phase = 0.5;
        let sun = Vec3::from_array(sky.sun_direction([0.0, 1.0, 0.0]));
        let moon = Vec3::from_array(sky.moon_direction());
        assert!(sun.dot(moon) < -0.95, "{sun} {moon}");
        sky.physical.moon_phase = 0.0;
        let sun = Vec3::from_array(sky.sun_direction([0.0; 3]));
        assert!(sun.dot(Vec3::from_array(sky.moon_direction())) > 0.99);
    }

    #[test]
    fn a_low_sun_is_redder_and_a_set_one_is_dark() {
        let sky = physical();
        let high = sky.sun_transmittance(dir(0.0, 60.0));
        let low = sky.sun_transmittance(dir(0.0, 3.0));
        let set = sky.sun_transmittance(dir(0.0, -5.0));
        assert!(high.iter().all(|&c| c > 0.5 && c <= 1.0), "{high:?}");
        // Blue goes first.
        assert!(high[2] < high[0]);
        assert!(low[2] / low[0] < high[2] / high[0], "{low:?}");
        assert_eq!(set, [0.0; 3]);
        // Other skies, or the tint off, leave the sun alone.
        assert_eq!(Sky::default().sun_transmittance(dir(0.0, 3.0)), [1.0; 3]);
        let mut untinted = physical();
        untinted.physical.tint_sun = false;
        assert_eq!(untinted.sun_transmittance(dir(0.0, 3.0)), [1.0; 3]);
    }

    #[test]
    fn the_sun_fades_across_the_horizon_rather_than_snapping() {
        let mut sky = physical();
        sky.physical.sun_size = 4.0;
        let at = |elevation: f32| sky.sun_transmittance(dir(0.0, elevation))[0];
        assert!(at(-0.5) > 0.0);
        assert!(at(-0.5) < at(0.5));
        assert!(at(0.5) < at(3.0));
    }

    #[test]
    fn night_ramps_on_the_suns_elevation() {
        let sky = PhysicalSky::default();
        assert_eq!(sky.night(dir(0.0, 30.0)), 0.0);
        assert_eq!(sky.night(dir(0.0, -30.0)), 1.0);
        let dusk = sky.night(dir(0.0, -5.0));
        assert!(dusk > 0.0 && dusk < 1.0);
    }

    #[test]
    fn an_hdri_without_a_file_is_no_sky() {
        let mut sky = Sky {
            kind: SkyKind::Hdri,
            ..Sky::default()
        };
        assert_eq!(sky.active_kind(), SkyKind::Flat);
        sky.hdri.path = "assets/sky.hdr".into();
        assert_eq!(sky.active_kind(), SkyKind::Hdri);
    }

    #[test]
    fn normalize_pulls_values_into_range() {
        let mut sky = Sky::default();
        sky.sun.azimuth = -90.0;
        sky.sun.day_of_year = 0;
        sky.physical.mie_g = 2.0;
        sky.physical.moon_phase = 1.25;
        sky.gradient.softness = 0.0;
        sky.hdri.path = " assets\\sky.hdr ".into();
        sky.exposure = f32::NAN;
        sky.normalize();
        assert_eq!(sky.sun.azimuth, 270.0);
        assert_eq!(sky.sun.day_of_year, 1);
        assert_eq!(sky.physical.mie_g, 0.99);
        assert_eq!(sky.physical.moon_phase, 0.25);
        assert!(sky.gradient.softness > 0.0);
        assert_eq!(sky.hdri.path, "assets/sky.hdr");
        assert_eq!(sky.exposure, 0.0);
    }

    #[test]
    fn stars_come_out_as_the_sun_goes_down() {
        let sky = Sky::default();
        assert_eq!(sky.star_visibility(dir(0.0, 30.0)), 0.0);
        assert_eq!(sky.star_visibility(dir(0.0, -30.0)), 1.0);
        let dusk = sky.star_visibility(dir(0.0, -8.0));
        assert!(dusk > 0.0 && dusk < 1.0);
    }

    #[test]
    fn the_moon_lights_by_its_phase_and_height() {
        let mut sky = physical();
        sky.physical.moon = true;
        sky.physical.moon_elevation = 45.0;
        sky.physical.moon_phase = 0.5;
        assert!((sky.moon_illuminance() - 0.25).abs() < 1e-5);
        sky.physical.moon_phase = 0.25;
        assert!((sky.moon_illuminance() - 0.125).abs() < 1e-5);
        sky.physical.moon_elevation = -10.0;
        assert_eq!(sky.moon_illuminance(), 0.0);
        sky.physical.moon_elevation = 45.0;
        sky.physical.moon_light = false;
        assert_eq!(sky.moon_illuminance(), 0.0);
        // No moon on other skies.
        let mut gradient = sky.clone();
        gradient.kind = SkyKind::Gradient;
        gradient.physical.moon_light = true;
        assert_eq!(gradient.moon_illuminance(), 0.0);
    }

    #[test]
    fn space_is_stars_or_an_aurora_with_some_kp() {
        let mut sky = Sky::default();
        assert!(!sky.has_space());
        sky.aurora.enabled = true;
        assert!(sky.has_space());
        sky.aurora.kp = 0.0;
        assert!(!sky.has_space());
        sky.stars.enabled = true;
        assert!(sky.has_space());
    }

    #[test]
    fn a_sky_round_trips_and_old_documents_default() {
        let sky: Sky = serde_json::from_str("{}").unwrap();
        assert_eq!(sky, Sky::default());
        let json = serde_json::to_string(&physical()).unwrap();
        assert_eq!(serde_json::from_str::<Sky>(&json).unwrap(), physical());
    }
}
