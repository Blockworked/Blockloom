//! The blended environment: the background, sun, ambient, exposure and post
//! the world looks through right now. Volumes lay their overrides onto the
//! project's own settings, and the result is the one `Environment` every pass
//! reads - the camera, the sun and the render world - instead of the project.

use crate::engine::{Dimension, Engine};
use crate::hdr::{HdrDebug, HdrFrame};
use crate::world::{WorldCamera, WorldLight, parse_color};
use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::DirectionalLightShadowMap;
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::post_process::bloom::{Bloom, BloomPrefilter};
use bevy::post_process::effect_stack::Vignette;
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
    pub tonemapping: TonemapName,
    pub bloom: bool,
    pub bloom_threshold: f32,
    pub bloom_intensity: f32,
    pub vignette: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Where the light comes from, towards the origin. Never zero.
    pub direction: Vec3,
    pub color: Color,
    pub illuminance: f32,
    pub shadow_bias: f32,
    pub shadow_map_size: usize,
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
        // A zero direction has nowhere to point, so fall back to straight down.
        let direction = Vec3::from_array(lighting.light_direction)
            .try_normalize()
            .unwrap_or(Vec3::Y);
        Self {
            background: parse_color(&world.background),
            sun: Sun {
                direction,
                color: parse_color(&lighting.light_color),
                illuminance: lighting.illuminance.max(0.0),
                shadow_bias: lighting.shadow_bias,
                shadow_map_size: shadow_map_size(lighting.shadow_map_size),
            },
            ambient_color: parse_color(&lighting.ambient_color),
            ambient_brightness: lighting.ambient_brightness.max(0.0),
            ao: lighting.ao_enabled,
            exposure: post.exposure_ev,
            tonemapping: post.tonemapping,
            bloom: post.bloom_enabled,
            bloom_threshold: post.bloom_threshold,
            bloom_intensity: post.bloom_intensity,
            vignette: post.vignette_strength.clamp(0.0, 1.0),
        }
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
        switch(&mut self.tonemapping, over.tonemapping, t);
        switch(&mut self.bloom, over.bloom, t);
        number(&mut self.bloom_threshold, over.bloom_threshold);
        number(&mut self.bloom_intensity, over.bloom_intensity);
        number(&mut self.vignette, over.vignette);
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
}

impl EnvironmentOverride {
    /// A `Volume` component's checked properties.
    pub fn from_volume(overrides: &VolumeOverrides) -> Self {
        let color = |value: Option<String>| value.map(|hex| parse_color(&hex));
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
            ("tonemapping", format!("{:?}", self.tonemapping)),
            ("bloom", show_bool(self.bloom)),
            ("bloom_threshold", show_number(self.bloom_threshold)),
            ("bloom_intensity", show_number(self.bloom_intensity)),
            ("vignette", show_number(self.vignette)),
        ]
    }
}

fn show_color(color: Color) -> String {
    color.to_srgba().to_hex()
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
    blended.exposure = claims.resolve(blended.exposure);
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
            tonemapping_of(env.tonemapping)
        };
        camera.insert((
            bevy::camera::Exposure {
                ev100: env.exposure,
            },
            tonemapping,
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
        if env.bloom {
            camera.insert(Bloom {
                intensity: env.bloom_intensity,
                prefilter: BloomPrefilter {
                    threshold: env.bloom_threshold,
                    ..default()
                },
                ..default()
            });
        } else {
            camera.remove::<Bloom>();
        }
        if env.vignette > 0.0 {
            // Radius and softness stay at Bevy's defaults; games tune how
            // dark the corners get.
            camera.insert(Vignette {
                intensity: env.vignette,
                ..default()
            });
        } else {
            camera.remove::<Vignette>();
        }
        if is_3d {
            if env.ao {
                // SSAO needs multisampling off on the same camera, or
                // `bevy_pbr` logs a mismatch and skips the effect.
                camera.insert((ScreenSpaceAmbientOcclusion::default(), Msaa::Off));
            } else {
                camera
                    .remove::<ScreenSpaceAmbientOcclusion>()
                    .insert(Msaa::default());
            }
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
        env.bloom = true;
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
        assert!(world.get::<Bloom>(camera).is_some());
        assert!(world.get::<Hdr>(camera).is_some());
        assert!(world.get::<ScreenSpaceAmbientOcclusion>(camera).is_some());
        assert_eq!(world.resource::<ClearColor>().0, base().background);
        assert_eq!(
            world.get::<DirectionalLight>(sun).unwrap().illuminance,
            base().sun.illuminance
        );
        assert!(world.contains_resource::<GlobalAmbientLight>());

        let mut env = app.world_mut().resource_mut::<Environment>();
        env.bloom = false;
        env.ao = false;
        app.update();
        assert!(app.world().get::<Bloom>(camera).is_none());
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
