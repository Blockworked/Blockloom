//! The blended environment: the background, sun, ambient, exposure and post
//! the world looks through right now. Volumes lay their overrides onto the
//! project's own settings, and the result is the one `Environment` every pass
//! reads - the camera, the sun and the render world - instead of the project.

use crate::engine::{Dimension, Engine};
use crate::hdr::{HdrDebug, HdrFrame};
use crate::world::{WorldCamera, WorldLight, parse_color};
use bevy::anti_alias::contrast_adaptive_sharpening::ContrastAdaptiveSharpening;
use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::DirectionalLightShadowMap;
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceReflections};
use bevy::post_process::bloom::Bloom;
use bevy::post_process::dof::{DepthOfField, DepthOfFieldMode};
use bevy::post_process::effect_stack::{ChromaticAberration, Vignette};
use bevy::post_process::motion_blur::MotionBlur;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::view::Msaa;
use blockloom_core::scene::{self, Mode, TonemapName};
use blockloom_core::volume::VolumeOverrides;

pub fn register(app: &mut App) {
    app.init_resource::<Environment>()
        .init_resource::<EnvironmentVolumes>()
        .init_resource::<ExposureClaims>()
        .add_plugins(ExtractResourcePlugin::<Environment>::default());
}

/// What the world looks like this frame, volumes and all.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Environment {
    pub background: Color,
    pub sun: Sun,
    pub ambient_color: Color,
    pub ambient_brightness: f32,
    pub ao: bool,
    /// EV100, already resolved through `ExposureClaims`. The one exposure
    /// value every pass reads.
    pub exposure: f32,
    /// The rest of the post chain, blended. Its `exposure_ev` is the manual
    /// EV before any claim; `exposure` above is what the camera uses.
    pub post: scene::PostProcess,
    /// Multiplier on reflection probes and the sky's light. Only volumes
    /// move it off 1.
    pub reflections: f32,
    /// Multiplier on irradiance volumes. Only volumes move it off 1.
    pub indirect: f32,
    /// EV added to the sky, after `exposure`.
    pub sky_exposure: f32,
    /// Multiplier on the sky's diffuse light alone.
    pub ambient_dimmer: f32,
    /// Height fog's extinction per metre at its base, 0 for none.
    pub fog_density: f32,
    /// Height fog's color by day, at dusk and by night.
    pub fog_colors: [Color; 3],
    pub fog_height: f32,
    /// Volumetric fog's extinction per metre at its base, 0 for none.
    pub volumetric_density: f32,
    pub volumetric_albedo: Color,
    /// Multiplier on every light's beam density.
    pub beams: f32,
    /// Aerial haze's extinction per metre, 0 for none.
    pub haze: f32,
    pub clouds: blockloom_core::clouds::Clouds,
    /// Snow cover 0-1, which surface snow masks settle by.
    pub snow: f32,
    /// Wetness 0-1, which surface wetness masks darken by.
    pub wetness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Where the light comes from, towards the origin. Never zero.
    pub direction: Vec3,
    pub color: Color,
    pub illuminance: f32,
    pub shadow_bias: f32,
    pub shadow_map_size: usize,
    /// Sunlight before the air: `color` times `illuminance`, lux per
    /// channel. The physical sky scatters this; the light itself carries
    /// what gets through.
    pub above_air: Vec3,
}

impl Default for Environment {
    fn default() -> Self {
        Self::from_world(&scene::World::default())
    }
}

impl ExtractResource<RenderApp> for Environment {
    type Source = Environment;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

impl Environment {
    /// The project's own settings, before any volume.
    pub fn from_world(world: &scene::World) -> Self {
        let lighting = &world.lighting;
        let post = &world.post;
        // The sky decides where the sun stands, from the light direction by
        // default.
        let direction = Vec3::from_array(world.sky.sun_direction(lighting.light_direction));
        Self {
            snow: world.surface.snow,
            wetness: world.surface.wetness,
            clouds: world.clouds.clone(),
            background: parse_color(&world.background),
            sun: Sun {
                direction,
                color: parse_color(&lighting.light_color),
                illuminance: lighting.illuminance.max(0.0),
                shadow_bias: lighting.shadow_bias,
                shadow_map_size: shadow_map_size(lighting.shadow_map_size),
                above_air: Vec3::ZERO,
            },
            ambient_color: parse_color(&lighting.ambient_color),
            ambient_brightness: lighting.ambient_brightness.max(0.0),
            ao: lighting.ao_enabled,
            exposure: post.exposure_ev,
            post: {
                let mut post = post.clone();
                post.normalize();
                post
            },
            reflections: 1.0,
            indirect: 1.0,
            sky_exposure: world.sky.exposure,
            ambient_dimmer: world.sky.ambient_dimmer,
            fog_density: if world.fog.height.enabled {
                blockloom_core::fog::density_for_distance(world.fog.height.distance)
            } else {
                0.0
            },
            fog_colors: [
                parse_color(&world.fog.height.day_color),
                parse_color(&world.fog.height.dusk_color),
                parse_color(&world.fog.height.night_color),
            ],
            fog_height: world.fog.height.base_height,
            volumetric_density: if world.fog.volumetric.enabled {
                world.fog.volumetric.density
            } else {
                0.0
            },
            volumetric_albedo: parse_color(&world.fog.volumetric.albedo),
            beams: 1.0,
            haze: if world.fog.aerial.enabled {
                haze_for_distance(world.fog.aerial.distance)
            } else {
                0.0
            },
        }
    }

    /// Lets the physical sky's air redden and dim the sun, once the volumes
    /// have had their say on where it stands.
    pub fn through_air(&mut self, sky: &blockloom_core::sky::Sky) {
        let color = self.sun.color.to_linear();
        self.sun.above_air = Vec3::new(color.red, color.green, color.blue) * self.sun.illuminance;
        let [r, g, b] = sky.sun_transmittance(self.sun.direction.to_array());
        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        if luminance >= 1.0 {
            return;
        }
        // The hue goes on the color and the dimming on the lux, so the light
        // keeps a color a picker can show.
        let peak = r.max(g).max(b);
        if peak > 0.0 {
            self.sun.color = LinearRgba::rgb(
                color.red * r / peak,
                color.green * g / peak,
                color.blue * b / peak,
            )
            .into();
        }
        self.sun.illuminance *= peak;
    }

    /// `set fog density to`: height fog takes the density, and volumetric
    /// fog and beams scale by it against the project's own.
    pub fn set_fog_density(&mut self, fog: &blockloom_core::fog::Fog, density: f32) {
        let scale = blockloom_core::fog::fog_scale(fog, density);
        self.fog_density = density.max(0.0);
        self.volumetric_density *= scale;
        self.beams *= scale;
    }

    /// Lays one volume over this at `weight` (0 to 1). Numbers and colors
    /// lerp; a switch or a tonemapper takes the volume's once it is half in.
    pub fn blend(&mut self, over: &EnvironmentOverride, weight: f32) {
        let t = weight.clamp(0.0, 1.0);
        if t <= 0.0 {
            return;
        }
        let color = |from: &mut Color, to: Option<Color>| {
            if let Some(to) = to {
                *from = from.to_linear().mix(&to.to_linear(), t).into();
            }
        };
        let number = |from: &mut f32, to: Option<f32>| {
            if let Some(to) = to {
                *from += (to - *from) * t;
            }
        };
        fn switch<T: Copy>(from: &mut T, to: Option<T>, t: f32) {
            if let Some(to) = to
                && t >= 0.5
            {
                *from = to;
            }
        }

        color(&mut self.background, over.background);
        if let Some(to) = over.sun_direction.and_then(Vec3::try_normalize) {
            let from = self.sun.direction;
            self.sun.direction = from.lerp(to, t).try_normalize().unwrap_or(to);
        }
        color(&mut self.sun.color, over.sun_color);
        number(&mut self.sun.illuminance, over.illuminance);
        color(&mut self.ambient_color, over.ambient_color);
        number(&mut self.ambient_brightness, over.ambient_brightness);
        switch(&mut self.ao, over.ao, t);
        number(&mut self.exposure, over.exposure);
        let triple = |from: &mut [f32; 3], to: Option<[f32; 3]>| {
            if let Some(to) = to {
                for (from, to) in from.iter_mut().zip(to) {
                    *from += (to - *from) * t;
                }
            }
        };
        let post = &mut self.post;
        switch(&mut post.tonemapping, over.tonemapping, t);
        switch(&mut post.bloom_enabled, over.bloom, t);
        number(&mut post.bloom_threshold, over.bloom_threshold);
        number(&mut post.bloom_intensity, over.bloom_intensity);
        number(&mut post.bloom_knee, over.bloom_knee);
        number(&mut post.bloom_scatter, over.bloom_scatter);
        number(&mut post.bloom_dirt_intensity, over.bloom_dirt_intensity);
        number(&mut post.vignette_strength, over.vignette);
        let auto = &mut post.auto_exposure;
        switch(&mut auto.enabled, over.auto_exposure, t);
        number(&mut auto.min_ev, over.auto_exposure_min);
        number(&mut auto.max_ev, over.auto_exposure_max);
        number(&mut auto.compensation, over.exposure_compensation);
        number(&mut post.tone.toe, over.tone_toe);
        number(&mut post.tone.shoulder, over.tone_shoulder);
        let grading = &mut post.grading;
        number(&mut grading.temperature, over.temperature);
        number(&mut grading.tint, over.tint);
        triple(&mut grading.lift, over.lift);
        triple(&mut grading.gamma, over.gamma);
        triple(&mut grading.gain, over.gain);
        number(&mut grading.saturation, over.saturation);
        number(&mut grading.contrast, over.contrast);
        number(&mut grading.lut_contribution, over.lut_contribution);
        let dof = &mut post.depth_of_field;
        switch(&mut dof.enabled, over.depth_of_field, t);
        number(&mut dof.focus_distance, over.focus_distance);
        number(&mut dof.f_stops, over.f_stops);
        switch(&mut post.motion_blur.enabled, over.motion_blur, t);
        number(&mut post.motion_blur.shutter_angle, over.shutter_angle);
        number(&mut post.ao.radius, over.ao_radius);
        switch(&mut post.ssr.enabled, over.ssr, t);
        number(&mut post.ssr.roughness_cutoff, over.ssr_roughness);
        number(&mut post.chromatic_aberration, over.chromatic_aberration);
        number(&mut post.grain.intensity, over.grain);
        number(&mut post.sharpen, over.sharpen);
        number(&mut self.reflections, over.reflections);
        number(&mut self.indirect, over.indirect);
        number(&mut self.sky_exposure, over.sky_exposure);
        number(&mut self.ambient_dimmer, over.ambient_dimmer);
        number(&mut self.clouds.coverage, over.cloud_coverage);
        number(&mut self.clouds.density, over.cloud_density);
        number(&mut self.clouds.cloud_type, over.cloud_type);
        number(&mut self.fog_density, over.fog_density);
        for fog in &mut self.fog_colors {
            color(fog, over.fog_color);
        }
        number(&mut self.fog_height, over.fog_height);
        number(&mut self.volumetric_density, over.volumetric_density);
        color(&mut self.volumetric_albedo, over.volumetric_albedo);
        number(&mut self.beams, over.beams);
        number(&mut self.haze, over.haze);
        number(&mut self.snow, over.snow);
        number(&mut self.wetness, over.wetness);
    }
}

/// What one volume changes. `None` leaves the value to whatever is under it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnvironmentOverride {
    pub background: Option<Color>,
    pub sun_direction: Option<Vec3>,
    pub sun_color: Option<Color>,
    pub illuminance: Option<f32>,
    pub ambient_color: Option<Color>,
    pub ambient_brightness: Option<f32>,
    pub ao: Option<bool>,
    pub exposure: Option<f32>,
    pub tonemapping: Option<TonemapName>,
    pub bloom: Option<bool>,
    pub bloom_threshold: Option<f32>,
    pub bloom_intensity: Option<f32>,
    pub vignette: Option<f32>,
    pub auto_exposure: Option<bool>,
    pub auto_exposure_min: Option<f32>,
    pub auto_exposure_max: Option<f32>,
    pub exposure_compensation: Option<f32>,
    pub bloom_knee: Option<f32>,
    pub bloom_scatter: Option<f32>,
    pub bloom_dirt_intensity: Option<f32>,
    pub tone_toe: Option<f32>,
    pub tone_shoulder: Option<f32>,
    pub temperature: Option<f32>,
    pub tint: Option<f32>,
    pub lift: Option<[f32; 3]>,
    pub gamma: Option<[f32; 3]>,
    pub gain: Option<[f32; 3]>,
    pub saturation: Option<f32>,
    pub contrast: Option<f32>,
    pub lut_contribution: Option<f32>,
    pub depth_of_field: Option<bool>,
    pub focus_distance: Option<f32>,
    pub f_stops: Option<f32>,
    pub motion_blur: Option<bool>,
    pub shutter_angle: Option<f32>,
    pub ao_radius: Option<f32>,
    pub ssr: Option<bool>,
    pub ssr_roughness: Option<f32>,
    pub chromatic_aberration: Option<f32>,
    pub grain: Option<f32>,
    pub sharpen: Option<f32>,
    pub reflections: Option<f32>,
    pub indirect: Option<f32>,
    pub sky_exposure: Option<f32>,
    pub ambient_dimmer: Option<f32>,
    pub cloud_coverage: Option<f32>,
    pub cloud_density: Option<f32>,
    pub cloud_type: Option<f32>,
    pub fog_density: Option<f32>,
    pub fog_color: Option<Color>,
    pub fog_height: Option<f32>,
    pub volumetric_density: Option<f32>,
    pub volumetric_albedo: Option<Color>,
    pub beams: Option<f32>,
    /// Extinction per metre, from the volume's haze distance.
    pub haze: Option<f32>,
    pub snow: Option<f32>,
    pub wetness: Option<f32>,
}

/// Haze extinction per metre that halves a far object's light over
/// `distance`.
pub fn haze_for_distance(distance: f32) -> f32 {
    std::f32::consts::LN_2 / distance.max(1.0)
}

impl EnvironmentOverride {
    /// A `Volume` component's checked properties.
    pub fn from_volume(overrides: &VolumeOverrides) -> Self {
        let color = |value: Option<String>| value.map(|hex| parse_color(&hex));
        let within = |value: Option<f32>, lo: f32, hi: f32| {
            value.filter(|v| v.is_finite()).map(|v| v.clamp(lo, hi))
        };
        let triple = |value: Option<[f32; 3]>, lo: f32, hi: f32| {
            value
                .filter(|v| v.iter().all(|c| c.is_finite()))
                .map(|v| v.map(|c| c.clamp(lo, hi)))
        };
        Self {
            background: color(overrides.background.get()),
            sun_direction: overrides.sun_direction.get().map(Vec3::from),
            sun_color: color(overrides.sun_color.get()),
            illuminance: overrides.illuminance.get().map(|lux| lux.max(0.0)),
            ambient_color: color(overrides.ambient_color.get()),
            ambient_brightness: overrides.ambient_brightness.get().map(|b| b.max(0.0)),
            ao: overrides.ao.get(),
            exposure: overrides.exposure.get(),
            tonemapping: overrides.tonemapping.get(),
            bloom: overrides.bloom.get(),
            bloom_threshold: overrides.bloom_threshold.get(),
            bloom_intensity: overrides.bloom_intensity.get(),
            vignette: overrides.vignette.get().map(|v| v.clamp(0.0, 1.0)),
            auto_exposure: overrides.auto_exposure.get(),
            auto_exposure_min: within(overrides.auto_exposure_min.get(), -10.0, 30.0),
            auto_exposure_max: within(overrides.auto_exposure_max.get(), -10.0, 30.0),
            exposure_compensation: within(overrides.exposure_compensation.get(), -10.0, 10.0),
            bloom_knee: within(overrides.bloom_knee.get(), 0.0, 1.0),
            bloom_scatter: within(overrides.bloom_scatter.get(), 0.0, 1.0),
            bloom_dirt_intensity: within(overrides.bloom_dirt_intensity.get(), 0.0, 10.0),
            tone_toe: within(overrides.tone_toe.get(), -1.0, 1.0),
            tone_shoulder: within(overrides.tone_shoulder.get(), 0.0, 1.0),
            temperature: within(overrides.temperature.get(), -1.0, 1.0),
            tint: within(overrides.tint.get(), -1.0, 1.0),
            lift: triple(overrides.lift.get(), -1.0, 1.0),
            gamma: triple(overrides.gamma.get(), 0.1, 10.0),
            gain: triple(overrides.gain.get(), 0.0, 10.0),
            saturation: within(overrides.saturation.get(), 0.0, 2.0),
            contrast: within(overrides.contrast.get(), 0.0, 2.0),
            lut_contribution: within(overrides.lut_contribution.get(), 0.0, 1.0),
            depth_of_field: overrides.depth_of_field.get(),
            focus_distance: within(overrides.focus_distance.get(), 0.05, 100_000.0),
            f_stops: within(overrides.f_stops.get(), 0.5, 64.0),
            motion_blur: overrides.motion_blur.get(),
            shutter_angle: within(overrides.shutter_angle.get(), 0.0, 360.0),
            ao_radius: within(overrides.ao_radius.get(), 0.01, 10.0),
            ssr: overrides.ssr.get(),
            ssr_roughness: within(overrides.ssr_roughness.get(), 0.05, 1.0),
            chromatic_aberration: within(overrides.chromatic_aberration.get(), 0.0, 1.0),
            grain: within(overrides.grain.get(), 0.0, 1.0),
            sharpen: within(overrides.sharpen.get(), 0.0, 1.0),
            reflections: overrides.reflections.get().map(|m| m.max(0.0)),
            indirect: overrides.indirect.get().map(|m| m.max(0.0)),
            sky_exposure: overrides.sky_exposure.get().filter(|ev| ev.is_finite()),
            ambient_dimmer: overrides.ambient_dimmer.get().map(|m| m.max(0.0)),
            cloud_coverage: overrides
                .cloud_coverage
                .get()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0)),
            cloud_density: overrides
                .cloud_density
                .get()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 10.0)),
            cloud_type: overrides
                .cloud_type
                .get()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0)),
            fog_density: overrides
                .fog_density
                .get()
                .filter(|d| d.is_finite())
                .map(|d| d.max(0.0)),
            fog_color: color(overrides.fog_color.get()),
            fog_height: overrides.fog_height.get().filter(|h| h.is_finite()),
            volumetric_density: overrides
                .volumetric_density
                .get()
                .filter(|d| d.is_finite())
                .map(|d| d.max(0.0)),
            volumetric_albedo: color(overrides.volumetric_albedo.get()),
            beams: overrides
                .beams
                .get()
                .filter(|m| m.is_finite())
                .map(|m| m.max(0.0)),
            haze: overrides
                .haze_distance
                .get()
                .filter(|d| d.is_finite())
                .map(haze_for_distance),
            snow: overrides
                .snow
                .get()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0)),
            wetness: overrides
                .wetness
                .get()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0)),
        }
    }

    /// What it asks for, as display text, by the names `Environment::readings`
    /// uses. Unchecked properties are left out.
    pub fn readings(&self) -> Vec<(&'static str, String)> {
        [
            ("background", self.background.map(show_color)),
            ("sun_direction", self.sun_direction.map(show_vec)),
            ("sun_color", self.sun_color.map(show_color)),
            ("illuminance", self.illuminance.map(show_number)),
            ("ambient_color", self.ambient_color.map(show_color)),
            (
                "ambient_brightness",
                self.ambient_brightness.map(show_number),
            ),
            ("ao", self.ao.map(show_bool)),
            ("exposure", self.exposure.map(show_number)),
            ("tonemapping", self.tonemapping.map(|t| format!("{t:?}"))),
            ("bloom", self.bloom.map(show_bool)),
            ("bloom_threshold", self.bloom_threshold.map(show_number)),
            ("bloom_intensity", self.bloom_intensity.map(show_number)),
            ("vignette", self.vignette.map(show_number)),
            ("auto_exposure", self.auto_exposure.map(show_bool)),
            ("auto_exposure_min", self.auto_exposure_min.map(show_number)),
            ("auto_exposure_max", self.auto_exposure_max.map(show_number)),
            (
                "exposure_compensation",
                self.exposure_compensation.map(show_number),
            ),
            ("bloom_knee", self.bloom_knee.map(show_number)),
            ("bloom_scatter", self.bloom_scatter.map(show_number)),
            (
                "bloom_dirt_intensity",
                self.bloom_dirt_intensity.map(show_number),
            ),
            ("tone_toe", self.tone_toe.map(show_number)),
            ("tone_shoulder", self.tone_shoulder.map(show_number)),
            ("temperature", self.temperature.map(show_number)),
            ("tint", self.tint.map(show_number)),
            ("lift", self.lift.map(show_triple)),
            ("gamma", self.gamma.map(show_triple)),
            ("gain", self.gain.map(show_triple)),
            ("saturation", self.saturation.map(show_number)),
            ("contrast", self.contrast.map(show_number)),
            ("lut_contribution", self.lut_contribution.map(show_number)),
            ("depth_of_field", self.depth_of_field.map(show_bool)),
            ("focus_distance", self.focus_distance.map(show_number)),
            ("f_stops", self.f_stops.map(show_number)),
            ("motion_blur", self.motion_blur.map(show_bool)),
            ("shutter_angle", self.shutter_angle.map(show_number)),
            ("ao_radius", self.ao_radius.map(show_number)),
            ("ssr", self.ssr.map(show_bool)),
            ("ssr_roughness", self.ssr_roughness.map(show_number)),
            (
                "chromatic_aberration",
                self.chromatic_aberration.map(show_number),
            ),
            ("grain", self.grain.map(show_number)),
            ("sharpen", self.sharpen.map(show_number)),
            ("reflections", self.reflections.map(show_number)),
            ("indirect", self.indirect.map(show_number)),
            ("sky_exposure", self.sky_exposure.map(show_number)),
            ("ambient_dimmer", self.ambient_dimmer.map(show_number)),
            ("cloud_coverage", self.cloud_coverage.map(show_number)),
            ("cloud_density", self.cloud_density.map(show_number)),
            ("cloud_type", self.cloud_type.map(show_number)),
            ("fog_density", self.fog_density.map(show_number)),
            ("fog_color", self.fog_color.map(show_color)),
            ("fog_height", self.fog_height.map(show_number)),
            (
                "volumetric_density",
                self.volumetric_density.map(show_number),
            ),
            ("volumetric_albedo", self.volumetric_albedo.map(show_color)),
            ("beams", self.beams.map(show_number)),
            ("haze_distance", self.haze.map(show_haze)),
            ("snow", self.snow.map(show_number)),
            ("wetness", self.wetness.map(show_number)),
        ]
        .into_iter()
        .filter_map(|(name, value)| Some((name, value?)))
        .collect()
    }
}

impl Environment {
    /// Every property a volume can blend, as display text.
    pub fn readings(&self) -> Vec<(&'static str, String)> {
        vec![
            ("background", show_color(self.background)),
            ("sun_direction", show_vec(self.sun.direction)),
            ("sun_color", show_color(self.sun.color)),
            ("illuminance", show_number(self.sun.illuminance)),
            ("ambient_color", show_color(self.ambient_color)),
            ("ambient_brightness", show_number(self.ambient_brightness)),
            ("ao", show_bool(self.ao)),
            ("exposure", show_number(self.exposure)),
            ("tonemapping", format!("{:?}", self.post.tonemapping)),
            ("bloom", show_bool(self.post.bloom_enabled)),
            ("bloom_threshold", show_number(self.post.bloom_threshold)),
            ("bloom_intensity", show_number(self.post.bloom_intensity)),
            ("vignette", show_number(self.post.vignette_strength)),
            ("auto_exposure", show_bool(self.post.auto_exposure.enabled)),
            (
                "auto_exposure_min",
                show_number(self.post.auto_exposure.min_ev),
            ),
            (
                "auto_exposure_max",
                show_number(self.post.auto_exposure.max_ev),
            ),
            (
                "exposure_compensation",
                show_number(self.post.auto_exposure.compensation),
            ),
            ("bloom_knee", show_number(self.post.bloom_knee)),
            ("bloom_scatter", show_number(self.post.bloom_scatter)),
            (
                "bloom_dirt_intensity",
                show_number(self.post.bloom_dirt_intensity),
            ),
            ("tone_toe", show_number(self.post.tone.toe)),
            ("tone_shoulder", show_number(self.post.tone.shoulder)),
            ("temperature", show_number(self.post.grading.temperature)),
            ("tint", show_number(self.post.grading.tint)),
            ("lift", show_triple(self.post.grading.lift)),
            ("gamma", show_triple(self.post.grading.gamma)),
            ("gain", show_triple(self.post.grading.gain)),
            ("saturation", show_number(self.post.grading.saturation)),
            ("contrast", show_number(self.post.grading.contrast)),
            (
                "lut_contribution",
                show_number(self.post.grading.lut_contribution),
            ),
            (
                "depth_of_field",
                show_bool(self.post.depth_of_field.enabled),
            ),
            (
                "focus_distance",
                show_number(self.post.depth_of_field.focus_distance),
            ),
            ("f_stops", show_number(self.post.depth_of_field.f_stops)),
            ("motion_blur", show_bool(self.post.motion_blur.enabled)),
            (
                "shutter_angle",
                show_number(self.post.motion_blur.shutter_angle),
            ),
            ("ao_radius", show_number(self.post.ao.radius)),
            ("ssr", show_bool(self.post.ssr.enabled)),
            ("ssr_roughness", show_number(self.post.ssr.roughness_cutoff)),
            (
                "chromatic_aberration",
                show_number(self.post.chromatic_aberration),
            ),
            ("grain", show_number(self.post.grain.intensity)),
            ("sharpen", show_number(self.post.sharpen)),
            ("reflections", show_number(self.reflections)),
            ("indirect", show_number(self.indirect)),
            ("sky_exposure", show_number(self.sky_exposure)),
            ("ambient_dimmer", show_number(self.ambient_dimmer)),
            ("cloud_coverage", show_number(self.clouds.coverage)),
            ("cloud_density", show_number(self.clouds.density)),
            ("cloud_type", show_number(self.clouds.cloud_type)),
            ("fog_density", show_number(self.fog_density)),
            ("fog_color", show_color(self.fog_colors[0])),
            ("fog_height", show_number(self.fog_height)),
            ("volumetric_density", show_number(self.volumetric_density)),
            ("volumetric_albedo", show_color(self.volumetric_albedo)),
            ("beams", show_number(self.beams)),
            ("haze_distance", show_haze(self.haze)),
            ("snow", show_number(self.snow)),
            ("wetness", show_number(self.wetness)),
        ]
    }
}

/// Haze as the distance a volume types, or "off".
fn show_haze(haze: f32) -> String {
    if haze > 0.0 {
        show_number(std::f32::consts::LN_2 / haze)
    } else {
        "off".to_string()
    }
}

fn show_color(color: Color) -> String {
    color.to_srgba().to_hex()
}

fn show_triple(v: [f32; 3]) -> String {
    show_vec(Vec3::from(v))
}

fn show_vec(v: Vec3) -> String {
    format!("{:.3}, {:.3}, {:.3}", v.x, v.y, v.z)
}

fn show_number(n: f32) -> String {
    format!("{n:.3}")
}

fn show_bool(b: bool) -> String {
    if b { "on" } else { "off" }.to_string()
}

/// The volumes over the project's settings this frame, lowest priority
/// first, each with its weight at the camera.
#[derive(Resource, Default)]
pub struct EnvironmentVolumes(pub Vec<(f32, EnvironmentOverride)>);

/// Writers that outrank the blended manual EV. The director track beats
/// auto-exposure, which beats the manual value, so the dials never fight.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ExposureClaims {
    pub director: Option<f32>,
    pub auto: Option<f32>,
}

impl ExposureClaims {
    pub fn resolve(&self, manual: f32) -> f32 {
        self.director.or(self.auto).unwrap_or(manual)
    }
}

/// `set exposure to` takes the director's slot for the rest of the run.
pub fn apply_exposure_effects(
    effects: Res<crate::engine::PendingEffects>,
    engine: NonSend<Engine>,
    mut claims: ResMut<ExposureClaims>,
) {
    if !engine.running || engine.paused {
        return;
    }
    for effect in &effects.0 {
        if let blockloom_core::vm::Effect::SetExposure { ev } = effect
            && ev.is_finite()
        {
            claims.director = Some(*ev);
        }
    }
}

/// Builds this frame's `Environment`. Only a real change marks it changed,
/// so `apply_environment` leaves the camera alone on a quiet frame.
pub fn blend_environment(
    engine: NonSend<Engine>,
    volumes: Res<EnvironmentVolumes>,
    claims: Res<ExposureClaims>,
    mut environment: ResMut<Environment>,
) {
    let mut blended = Environment::from_world(&engine.project.world);
    for (weight, over) in &volumes.0 {
        blended.blend(over, *weight);
    }
    blended.through_air(&engine.project.world.sky);
    blended.exposure = claims.resolve(blended.exposure);
    // `set fog density` outlasts every volume, like the director's exposure,
    // and moves volumetric fog and beams by the same ratio.
    if let Some(density) = engine.fog_density.filter(|d| d.is_finite()) {
        blended.set_fog_density(&engine.project.world.fog, density);
    }
    engine.clouds.apply(&mut blended.clouds);
    // `set snow cover` and `set surface wetness` outlast every volume too.
    if let Some(snow) = engine.surface.snow {
        blended.snow = snow;
    }
    if let Some(wetness) = engine.surface.wetness {
        blended.wetness = wetness;
    }
    environment.set_if_neq(blended);
}

/// Writes the environment onto the world camera and sun: whenever it
/// changes, and onto any camera or sun a rebuild has just spawned.
#[allow(clippy::too_many_arguments)]
pub fn apply_environment(
    mut commands: Commands,
    environment: Res<Environment>,
    debug: Res<HdrDebug>,
    frame: Res<HdrFrame>,
    dimension: Res<Dimension>,
    mut clear_color: ResMut<ClearColor>,
    cameras: Query<(Entity, Ref<WorldCamera>, Has<Camera3d>)>,
    mut suns: Query<(Ref<WorldLight>, &mut DirectionalLight, &mut Transform)>,
) {
    let changed = environment.is_changed();
    let env = &*environment;
    if changed {
        clear_color.0 = env.background;
    }
    for (entity, camera, is_3d) in &cameras {
        if !changed && !debug.is_changed() && !frame.is_changed() && !camera.is_added() {
            continue;
        }
        let mut camera = commands.entity(entity);
        // HDR output brings its own tone curve, as do some debug views.
        let tonemapping = if debug.bypasses_tonemapping() || frame.is_hdr() {
            Tonemapping::None
        } else {
            tonemapping_of(env.post.tonemapping)
        };
        // Bevy's dither rides its tonemapper, so it goes where that does.
        let deband = if tonemapping == Tonemapping::None {
            DebandDither::Disabled
        } else {
            DebandDither::Enabled
        };
        camera.insert((
            bevy::camera::Exposure {
                ev100: env.exposure,
            },
            tonemapping,
            deband,
        ));
        // Linear FP16 all the way to the tonemapper, bloom or not, so lights,
        // sky and emissives can pass 1.0 without clipping. An SDR-only build
        // stays 8-bit.
        if frame.fp16 {
            camera.insert(Hdr);
        } else {
            camera.remove::<Hdr>();
        }
        debug.apply(&mut camera, is_3d, &frame);
        frame.apply(&mut camera, is_3d);
        // Bloom is Blockloom's own chain (`post.rs`), so Bevy's stays off.
        camera.remove::<Bloom>();
        let post = &env.post;
        if post.vignette_strength > 0.0 {
            // Radius and softness stay at Bevy's defaults; games tune how
            // dark the corners get.
            camera.insert(Vignette {
                intensity: post.vignette_strength,
                ..default()
            });
        } else {
            camera.remove::<Vignette>();
        }
        if post.chromatic_aberration > 0.0 {
            camera.insert(ChromaticAberration {
                intensity: post.chromatic_aberration * CHROMATIC_ABERRATION_MAX,
                ..default()
            });
        } else {
            camera.remove::<ChromaticAberration>();
        }
        if post.sharpen > 0.0 {
            camera.insert(ContrastAdaptiveSharpening {
                enabled: true,
                sharpening_strength: post.sharpen,
                denoise: false,
            });
        } else {
            camera.remove::<ContrastAdaptiveSharpening>();
        }
        if is_3d {
            apply_camera_3d(&mut camera, env);
        }
    }

    if dimension.0 != Mode::ThreeD {
        return;
    }
    for (sun, mut light, mut transform) in &mut suns {
        if !changed && !sun.is_added() {
            continue;
        }
        light.color = env.sun.color;
        light.illuminance = env.sun.illuminance;
        light.shadow_depth_bias = env.sun.shadow_bias;
        *transform =
            Transform::from_translation(env.sun.direction * 16.0).looking_at(Vec3::ZERO, Vec3::Y);
    }
    if changed {
        // One size for every cascade, so it is a resource, not a light field.
        commands.insert_resource(DirectionalLightShadowMap {
            size: env.sun.shadow_map_size,
        });
        commands.insert_resource(GlobalAmbientLight {
            color: env.ambient_color,
            brightness: env.ambient_brightness,
            ..default()
        });
    }
}

/// Bevy's chromatic aberration at a project's 1: strong fringes at the edges.
const CHROMATIC_ABERRATION_MAX: f32 = 0.08;

/// The 3D-only screen-space effects: SSAO, SSR, depth of field and motion
/// blur, each Bevy's own. Autofocus (`post::focus_depth_of_field`) moves the
/// depth of field's distance after this.
fn apply_camera_3d(camera: &mut EntityCommands, env: &Environment) {
    let post = &env.post;
    // SSAO and the G-buffer SSR reads both need multisampling off on the
    // camera, or Bevy logs a mismatch and skips them.
    camera.insert(if env.wants_msaa_off() {
        Msaa::Off
    } else {
        Msaa::default()
    });
    if env.ao {
        camera.insert(ScreenSpaceAmbientOcclusion {
            radius: post.ao.radius,
            ..default()
        });
    } else {
        camera.remove::<ScreenSpaceAmbientOcclusion>();
    }
    if post.ssr.enabled {
        let cutoff = post.ssr.roughness_cutoff;
        camera.insert(ScreenSpaceReflections {
            // Fades out over the last fifth below the cutoff.
            min_perceptual_roughness: 0.0..0.0,
            max_perceptual_roughness: cutoff * 0.8..cutoff,
            thickness: post.ssr.thickness,
            ..default()
        });
    } else {
        camera.remove::<ScreenSpaceReflections>();
    }
    let dof = &post.depth_of_field;
    if dof.enabled {
        let far = if dof.far_limit > 0.0 {
            dof.far_limit
        } else {
            f32::INFINITY
        };
        camera.insert(DepthOfField {
            mode: match dof.bokeh {
                scene::Bokeh::Hexagonal => DepthOfFieldMode::Bokeh,
                scene::Bokeh::Circular => DepthOfFieldMode::Gaussian,
            },
            focal_distance: dof.focus_distance,
            aperture_f_stops: dof.f_stops,
            max_circle_of_confusion_diameter: dof.max_blur,
            // A negative limit asks the patched shader to leave the near
            // field sharp (`post::patch_depth_of_field`).
            max_depth: if dof.near { far } else { -far },
            ..default()
        });
    } else {
        camera.remove::<DepthOfField>();
    }
    let blur = &post.motion_blur;
    if blur.enabled && blur.shutter_angle > 0.0 {
        camera.insert(MotionBlur {
            shutter_angle: blur.shutter_angle / 360.0,
            samples: blur.samples,
        });
    } else {
        camera.remove::<MotionBlur>();
    }
}

impl Environment {
    /// SSAO and SSR can't share the camera with multisampling.
    pub fn wants_msaa_off(&self) -> bool {
        self.ao || self.post.ssr.enabled
    }
}

/// The tonemapper as the camera component. TonyMcMapface is Bevy's own
/// default, so spelling it out changes nothing for old projects.
fn tonemapping_of(name: TonemapName) -> Tonemapping {
    match name {
        // Linear is what `None` meant before 0.20: an identity curve that still
        // applies exposure and grading. `None` now skips those too.
        TonemapName::None => Tonemapping::Linear,
        TonemapName::Reinhard => Tonemapping::Reinhard,
        TonemapName::ReinhardLuminance => Tonemapping::ReinhardLuminance,
        TonemapName::AcesFitted => Tonemapping::AcesFitted,
        TonemapName::TonyMcMapface => Tonemapping::TonyMcMapface,
        TonemapName::Filmic => Tonemapping::BlenderFilmic,
        TonemapName::AgX => Tonemapping::AgX,
        TonemapName::Neutral => Tonemapping::KhronosPbrNeutral,
    }
}

/// Snap a shadow map size to the powers of two Bevy accepts.
fn shadow_map_size(size: u32) -> usize {
    const SIZES: &[usize] = &[512, 1024, 2048, 4096, 8192];
    let wanted = size.max(512) as usize;
    SIZES
        .iter()
        .copied()
        .min_by_key(|candidate| candidate.abs_diff(wanted))
        .unwrap_or(2048)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Environment {
        Environment::from_world(&scene::World::default())
    }

    #[test]
    fn clouds_blend_by_volume_weight() {
        let mut env = Environment::default();
        env.clouds.coverage = 0.2;
        env.clouds.density = 0.5;
        let overrides = EnvironmentOverride {
            cloud_coverage: Some(1.0),
            cloud_density: Some(1.5),
            cloud_type: Some(0.0),
            ..default()
        };
        env.blend(&overrides, 0.5);
        assert!((env.clouds.coverage - 0.6).abs() < 1e-6);
        assert_eq!(env.clouds.density, 1.0);
        assert_eq!(env.clouds.cloud_type, 0.35);
    }

    #[test]
    fn zero_weight_changes_nothing() {
        let mut env = base();
        env.blend(
            &EnvironmentOverride {
                exposure: Some(2.0),
                ao: Some(true),
                ..default()
            },
            0.0,
        );
        assert_eq!(env, base());
    }

    #[test]
    fn numbers_lerp_and_switches_flip_at_half() {
        let over = EnvironmentOverride {
            exposure: Some(base().exposure + 4.0),
            ao: Some(true),
            ..default()
        };
        let mut quarter = base();
        quarter.blend(&over, 0.25);
        assert!((quarter.exposure - (base().exposure + 1.0)).abs() < 1e-5);
        assert!(!quarter.ao);

        let mut half = base();
        half.blend(&over, 0.5);
        assert!(half.ao);
    }

    #[test]
    fn untouched_properties_stay_the_projects() {
        let mut env = base();
        env.blend(
            &EnvironmentOverride {
                background: Some(Color::BLACK),
                ..default()
            },
            1.0,
        );
        assert_eq!(env.background.to_linear(), LinearRgba::BLACK);
        assert_eq!(env.sun, base().sun);
        assert_eq!(env.exposure, base().exposure);
    }

    #[test]
    fn sun_direction_stays_a_unit_vector() {
        let mut env = base();
        env.blend(
            &EnvironmentOverride {
                sun_direction: Some(Vec3::new(-8.0, 16.0, -8.0)),
                ..default()
            },
            0.5,
        );
        assert!((env.sun.direction.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn exposure_claims_resolve_by_precedence() {
        let manual = 9.7;
        let mut claims = ExposureClaims::default();
        assert_eq!(claims.resolve(manual), manual);
        claims.auto = Some(12.0);
        assert_eq!(claims.resolve(manual), 12.0);
        claims.director = Some(6.0);
        assert_eq!(claims.resolve(manual), 6.0);
    }

    #[test]
    fn apply_follows_the_environment_onto_camera_and_sun() {
        let mut app = App::new();
        let mut env = base();
        env.post.bloom_enabled = true;
        env.ao = true;
        env.exposure = 7.0;
        app.insert_resource(Dimension(Mode::ThreeD))
            .insert_resource(ClearColor(Color::WHITE))
            .init_resource::<HdrDebug>()
            .init_resource::<HdrFrame>()
            .insert_resource(env)
            .add_systems(Update, apply_environment);
        let camera = app
            .world_mut()
            .spawn((WorldCamera, Camera3d::default()))
            .id();
        let sun = app
            .world_mut()
            .spawn((
                WorldLight,
                DirectionalLight::default(),
                Transform::default(),
            ))
            .id();
        app.update();

        let world = app.world();
        assert_eq!(
            world.get::<bevy::camera::Exposure>(camera).unwrap().ev100,
            7.0
        );
        // Bloom is the post stack's, never Bevy's.
        assert!(world.get::<Bloom>(camera).is_none());
        assert!(world.get::<Hdr>(camera).is_some());
        assert!(world.get::<ScreenSpaceAmbientOcclusion>(camera).is_some());
        assert_eq!(world.resource::<ClearColor>().0, base().background);
        assert_eq!(
            world.get::<DirectionalLight>(sun).unwrap().illuminance,
            base().sun.illuminance
        );
        assert!(world.contains_resource::<GlobalAmbientLight>());

        let mut env = app.world_mut().resource_mut::<Environment>();
        env.post.bloom_enabled = false;
        env.ao = false;
        app.update();
        assert!(
            app.world()
                .get::<ScreenSpaceAmbientOcclusion>(camera)
                .is_none()
        );
        // Bloom going away leaves the frame HDR.
        assert!(app.world().get::<Hdr>(camera).is_some());

        app.world_mut().resource_mut::<HdrDebug>().0 = blockloom_protocol::DebugView::FalseColor;
        app.update();
        assert_eq!(
            app.world().get::<Tonemapping>(camera),
            Some(&Tonemapping::None)
        );
        assert!(
            app.world()
                .get::<crate::hdr::HdrDebugView3d>(camera)
                .is_some()
        );
        app.world_mut().resource_mut::<HdrDebug>().0 = blockloom_protocol::DebugView::Lit;
        app.update();
        assert_eq!(
            app.world().get::<Tonemapping>(camera),
            Some(&Tonemapping::TonyMcMapface)
        );

        // HDR output takes over from the tonemapper; an SDR-only build
        // drops the FP16 frame.
        let both = [
            blockloom_core::scene::OutputSpace::Sdr,
            blockloom_core::scene::OutputSpace::Scrgb,
        ];
        let display = blockloom_core::scene::DisplayOutput {
            space: blockloom_core::scene::OutputSpace::Scrgb,
            ..default()
        };
        *app.world_mut().resource_mut::<HdrFrame>() =
            HdrFrame::resolve(display, Some(&both), crate::hdr::HdrPolicy::default());
        app.update();
        assert_eq!(
            app.world().get::<Tonemapping>(camera),
            Some(&Tonemapping::None)
        );
        assert!(app.world().get::<crate::hdr::HdrEncode3d>(camera).is_some());
        let clamped = crate::hdr::HdrPolicy {
            allow: false,
            windowed: true,
        };
        *app.world_mut().resource_mut::<HdrFrame>() =
            HdrFrame::resolve(display, Some(&both), clamped);
        app.update();
        assert!(app.world().get::<Hdr>(camera).is_none());
        assert!(app.world().get::<crate::hdr::HdrEncode3d>(camera).is_none());
    }

    #[test]
    fn a_zero_light_direction_points_straight_down() {
        let mut world = scene::World::default();
        world.lighting.light_direction = [0.0; 3];
        assert_eq!(Environment::from_world(&world).sun.direction, Vec3::Y);
    }
}
