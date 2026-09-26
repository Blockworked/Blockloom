//! What hangs over the sky at night, 3D only: stars, the Milky Way and
//! aurora (`blockloom_core::sky::{Stars, Aurora}`), drawn by the sky's
//! background pass from `SpaceRender` (`shaders/space.wesl`), and the moon
//! as a real directional light.
//!
//! `SpaceParams` is kept apart from `SkyParams` because it moves every
//! frame - twinkling, flowing, flashing - and a change to the sky's own
//! parameters rewrites and refilters its light cubes.

use crate::engine::{Engine, PendingEffects};
use crate::environment::Environment;
use crate::lightning::LightningFlash;
use crate::world::parse_color;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::ShaderType;
use blockloom_core::sky::Sky;
use blockloom_core::vm::Effect;

pub fn register(app: &mut App) {
    app.init_resource::<MoonState>()
        .init_resource::<SpaceRender>()
        .init_resource::<MilkyWay>()
        .add_plugins(ExtractResourcePlugin::<SpaceRender>::default())
        .add_systems(
            FixedUpdate,
            apply_space_effects
                .in_set(crate::world::SimulationSet)
                .after(crate::world::apply_common)
                .before(crate::world::clear_effects),
        )
        .add_systems(
            Update,
            (sync_moon, resolve_space)
                .chain()
                .after(crate::environment::apply_environment),
        );
}

/// The moon's light: which way it comes from and its lux per channel.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct MoonState {
    pub direction: Vec3,
    pub lux: Vec3,
}

impl Default for MoonState {
    fn default() -> Self {
        Self {
            direction: Vec3::Y,
            lux: Vec3::ZERO,
        }
    }
}

/// Marks the moon's directional light.
#[derive(Component)]
pub struct MoonLight;

/// `SpaceParams` in `shaders/space.wesl`, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct SpaceParams {
    pub stars: Vec4,
    pub twinkle: Vec4,
    pub milky_way: Vec4,
    pub aurora: Vec4,
    pub aurora_look: Vec4,
    pub aurora_bottom: Vec4,
    pub aurora_top: Vec4,
    pub time: Vec4,
    pub flash: Vec4,
}

/// This frame's space, for the sky's background pass.
#[derive(Resource, Clone, Default, Debug, PartialEq)]
pub struct SpaceRender {
    pub params: SpaceParams,
    /// The Milky Way panorama, once loaded.
    pub milky_way: Option<Handle<Image>>,
}

impl ExtractResource<RenderApp> for SpaceRender {
    type Source = SpaceRender;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

/// The Milky Way file in use and its handle.
#[derive(Resource, Default)]
struct MilkyWay {
    path: String,
    handle: Option<Handle<Image>>,
}

fn linear(hex: &str) -> Vec3 {
    let c = parse_color(hex).to_linear();
    Vec3::new(c.red, c.green, c.blue)
}

impl SpaceParams {
    /// Stars and aurora as the sky pass reads them. `kp` is the aurora's
    /// KP this run (0 for none); `flash` the lightning's.
    pub fn resolve(
        sky: &Sky,
        env: &Environment,
        kp: f32,
        time: f32,
        flash: &LightningFlash,
    ) -> SpaceParams {
        let night = sky.star_visibility(env.sun.direction.to_array());
        let s = &sky.stars;
        let a = &sky.aurora;
        let stars_on = if s.enabled { night } else { 0.0 };
        let has_milky_way = s.enabled && !s.milky_way.is_empty();
        // The flash lights the whole sky by the same jump as the ambient.
        let flash_nits = flash.color * env.ambient_brightness.max(1.0) * flash.pulse;
        SpaceParams {
            stars: Vec4::new(
                s.density,
                s.brightness,
                s.magnitude_slope,
                s.color_variation,
            ),
            twinkle: Vec4::new(
                s.twinkle,
                s.twinkle_speed,
                s.horizon_fade.to_radians().sin(),
                stars_on,
            ),
            milky_way: Vec4::new(
                if has_milky_way {
                    s.milky_way_brightness * stars_on
                } else {
                    0.0
                },
                s.milky_way_rotation.to_radians(),
                s.milky_way_tilt.to_radians(),
                0.0,
            ),
            aurora: Vec4::new((kp / 9.0).clamp(0.0, 1.0), a.altitude, a.height, a.width),
            aurora_look: Vec4::new(a.ray_scale, a.brightness, a.speed, a.horizon_glow),
            aurora_bottom: linear(&a.bottom_color).extend(a.layers.clamp(1, 3) as f32),
            aurora_top: linear(&a.top_color).extend(if kp > 0.0 { night } else { 0.0 }),
            time: Vec4::new(time, a.pole_azimuth.to_radians(), 0.0, flash.level),
            flash: flash_nits.extend(0.0),
        }
    }
}

/// The aurora's KP this run: `set aurora` over the project's.
pub fn aurora_kp(engine: &Engine) -> f32 {
    let aurora = &engine.project.world.sky.aurora;
    engine
        .aurora_kp
        .unwrap_or(if aurora.enabled { aurora.kp } else { 0.0 })
        .clamp(0.0, 9.0)
}

/// `set aurora` and `set fog density` for the rest of the run.
fn apply_space_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::SetAurora { kp } if kp.is_finite() => {
                engine.aurora_kp = Some(kp.clamp(0.0, 9.0));
            }
            Effect::SetFogDensity { density } if density.is_finite() => {
                engine.fog_density = Some(density.clamp(0.0, 10.0));
            }
            _ => {}
        }
    }
}

/// Keeps the moon's light where the sky says the moon is, while it has any
/// light to give.
fn sync_moon(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut state: ResMut<MoonState>,
    mut moons: Query<(Entity, &mut DirectionalLight, &mut Transform), With<MoonLight>>,
) {
    let sky = &engine.project.world.sky;
    let lux = sky.moon_illuminance();
    let direction = Vec3::from_array(sky.moon_direction())
        .try_normalize()
        .unwrap_or(Vec3::Y);
    let color = parse_color(&sky.physical.moon_color);
    let next = MoonState {
        direction,
        lux: linear(&sky.physical.moon_color) * lux,
    };
    state.set_if_neq(next);
    let transform = Transform::from_translation(direction * 16.0).looking_at(Vec3::ZERO, Vec3::Y);
    let shadows = sky.physical.moon_shadows;
    if lux <= 0.0 {
        for (entity, ..) in &moons {
            commands.entity(entity).despawn();
        }
        return;
    }
    let mut found = false;
    for (_, mut light, mut placed) in &mut moons {
        found = true;
        if light.illuminance != lux || light.color != color || light.shadow_maps_enabled != shadows
        {
            light.illuminance = lux;
            light.color = color;
            light.shadow_maps_enabled = shadows;
        }
        if *placed != transform {
            *placed = transform;
        }
    }
    if !found {
        commands.spawn((
            MoonLight,
            DirectionalLight {
                color,
                illuminance: lux,
                shadow_maps_enabled: shadows,
                ..default()
            },
            transform,
            Name::new("moon"),
        ));
    }
}

/// Works out this frame's stars and aurora, and loads the Milky Way.
#[allow(clippy::too_many_arguments)]
fn resolve_space(
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    time: Res<Time>,
    flash: Option<Res<LightningFlash>>,
    assets: Res<AssetServer>,
    mut milky_way: ResMut<MilkyWay>,
    mut sources: Option<ResMut<crate::atmosphere::AtmosphereSources>>,
    mut render: ResMut<SpaceRender>,
) {
    let sky = &engine.project.world.sky;
    let kp = aurora_kp(&engine);
    let flash = flash.map(|f| *f).unwrap_or_default();
    let params = SpaceParams::resolve(sky, &environment, kp, time.elapsed_secs_wrapped(), &flash);
    let path = if sky.stars.enabled {
        sky.stars.milky_way.clone()
    } else {
        String::new()
    };
    if path != milky_way.path {
        milky_way.handle = (!path.is_empty()).then(|| {
            let resolved = engine
                .project_dir
                .as_deref()
                .and_then(|dir| blockloom_core::assets::resolve(dir, &path))
                .map(|resolved| resolved.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            assets.load::<Image>(resolved)
        });
        milky_way.path = path;
    }
    let next = SpaceRender {
        params,
        milky_way: milky_way.handle.clone(),
    };
    if *render != next {
        *render = next;
    }
    if let Some(sources) = sources.as_mut() {
        sources.aurora = kp;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::{Mode, World};
    use blockloom_core::sky::{SkyKind, SunMode};

    fn env(sky: &Sky) -> Environment {
        let world = World {
            mode: Mode::ThreeD,
            sky: sky.clone(),
            ..default()
        };
        let mut env = Environment::from_world(&world);
        env.through_air(sky);
        env
    }

    fn night_sky() -> Sky {
        let mut sky = Sky {
            kind: SkyKind::Physical,
            ..default()
        };
        sky.sun.mode = SunMode::Manual;
        sky.sun.elevation = -30.0;
        sky.stars.enabled = true;
        sky.aurora.enabled = true;
        sky
    }

    #[test]
    fn stars_and_aurora_show_at_night_only() {
        let sky = night_sky();
        let flash = LightningFlash::default();
        let night = SpaceParams::resolve(&sky, &env(&sky), 5.0, 0.0, &flash);
        assert_eq!(night.twinkle.w, 1.0);
        assert_eq!(night.aurora_top.w, 1.0);
        assert!((night.aurora.x - 5.0 / 9.0).abs() < 1e-6);
        let mut day = sky.clone();
        day.sun.elevation = 40.0;
        let noon = SpaceParams::resolve(&day, &env(&day), 5.0, 0.0, &flash);
        assert_eq!(noon.twinkle.w, 0.0);
        assert_eq!(noon.aurora_top.w, 0.0);
        // No KP, no aurora, night or not.
        let calm = SpaceParams::resolve(&sky, &env(&sky), 0.0, 0.0, &flash);
        assert_eq!(calm.aurora_top.w, 0.0);
    }

    #[test]
    fn a_flash_lights_the_whole_sky() {
        let sky = night_sky();
        let flash = LightningFlash {
            level: 0.5,
            color: Vec3::ONE,
            pulse: 4.0,
        };
        let params = SpaceParams::resolve(&sky, &env(&sky), 0.0, 0.0, &flash);
        assert_eq!(params.time.w, 0.5);
        assert!(params.flash.x > 0.0);
    }

    #[test]
    fn space_params_match_the_wesl_layout() {
        let source = blockloom_core::shader_lib::module("space").unwrap();
        let body = source.split("struct SpaceParams {").nth(1).unwrap();
        let body = &body[..body.find("\n}").unwrap()];
        let fields = body.lines().filter(|line| line.contains(": vec4<")).count();
        assert_eq!(SpaceParams::min_size().get(), fields as u64 * 16);
        assert_eq!(fields, 9);
    }

    #[test]
    fn the_moon_lights_the_world_while_it_is_up() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        let sky = &mut engine.project.world.sky;
        sky.kind = SkyKind::Physical;
        sky.physical.moon = true;
        sky.physical.moon_elevation = 40.0;
        app.insert_non_send(engine)
            .init_resource::<MoonState>()
            .add_systems(Update, sync_moon);
        app.update();
        let mut moons = app
            .world_mut()
            .query_filtered::<&DirectionalLight, With<MoonLight>>();
        let lux = moons.single(app.world()).unwrap().illuminance;
        assert!((lux - 0.25).abs() < 1e-4, "{lux}");
        assert!(app.world().resource::<MoonState>().lux.x > 0.0);

        app.world_mut()
            .non_send_mut::<Engine>()
            .project
            .world
            .sky
            .physical
            .moon_elevation = -20.0;
        app.update();
        assert_eq!(moons.iter(app.world()).count(), 0);
    }
}
