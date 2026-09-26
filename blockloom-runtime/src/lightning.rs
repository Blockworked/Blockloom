//! Lightning (`blockloom_core::lightning`) in both dimensions: a strike is
//! a flash of light above where it lands (3D), a pulse of the ambient light
//! and background, and thunder late by the distance to the camera. The
//! storm is `StormDirector` stepped on the fixed tick, so one seed throws
//! the same storm however the frames fall.

use crate::engine::{Dimension, Engine, PendingEffects};
use crate::environment::Environment;
use crate::sound::SoundState;
use crate::world::{WorldCamera, parse_color};
use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use blockloom_core::lightning::{Lightning, StormDirector, thunder_wav};
use blockloom_core::scene::Mode;
use blockloom_core::sound::SoundBus;
use blockloom_core::vm::Effect;

pub fn register(app: &mut App) {
    app.init_resource::<LightningState>()
        .init_resource::<LightningFlash>()
        .add_systems(
            FixedUpdate,
            (apply_lightning_effects, direct_storm)
                .chain()
                .in_set(crate::world::SimulationSet)
                .after(crate::world::apply_common)
                .before(crate::world::clear_effects),
        )
        .add_systems(
            Update,
            (step_strikes, flash_environment)
                .chain()
                .after(crate::environment::blend_environment)
                .before(crate::environment::apply_environment),
        );
}

/// A strike on its way from flash to thunder.
struct Strike {
    at: Vec3,
    /// Seconds since it struck.
    since: f32,
    /// Seconds until its thunder is heard, once worked out.
    thunder: Option<f32>,
    heard: bool,
    light: Option<Entity>,
}

#[derive(Resource, Default)]
pub struct LightningState {
    director: StormDirector,
    /// Strikes queued on the fixed tick, for the frame to carry out.
    queued: Vec<Vec3>,
    strikes: Vec<Strike>,
    thunder: Option<Handle<AudioSource>>,
    custom: Option<(String, Handle<AudioSource>)>,
}

/// How bright the sky is from lightning right now, for the sky and the
/// atmosphere slot.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct LightningFlash {
    /// 0-1.
    pub level: f32,
    /// The flash's color, linear.
    pub color: Vec3,
    /// How far the ambient light jumps at the peak, as a multiple.
    pub pulse: f32,
}

/// Marks thunder, so `stop all` can silence it.
#[derive(Component)]
pub struct Thunder;

/// Marks a flash's light.
#[derive(Component)]
pub struct FlashLight;

/// `strike lightning` and `set lightning storm` for the rest of the run.
fn apply_lightning_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    mut state: ResMut<LightningState>,
    thunder: Query<Entity, With<Thunder>>,
) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::StrikeLightning { at } if at.iter().all(|v| v.is_finite()) => {
                state.queued.push(Vec3::from(*at));
            }
            Effect::SetLightningRate { rate } if rate.is_finite() => {
                engine.lightning_rate = Some(rate.clamp(0.0, 600.0));
            }
            Effect::Stopped => {
                for entity in &thunder {
                    commands.entity(entity).despawn();
                }
            }
            _ => {}
        }
    }
}

/// Steps the storm on the fixed tick. A stopped world starts it over.
fn direct_storm(
    engine: NonSend<Engine>,
    time: Res<Time<Fixed>>,
    mut state: ResMut<LightningState>,
) {
    if !engine.running {
        state.director = StormDirector::default();
        return;
    }
    if engine.paused {
        return;
    }
    let lightning = &engine.project.world.lightning;
    let rate = engine
        .lightning_rate
        .unwrap_or(if lightning.storm { lightning.rate } else { 0.0 });
    let strikes = state.director.step(lightning, rate, time.delta_secs());
    state
        .queued
        .extend(strikes.into_iter().map(Vec3::from));
}

/// Carries strikes out: lights their flash, plays their thunder once the
/// sound has had time to arrive, and lets them go.
#[allow(clippy::too_many_arguments)]
fn step_strikes(
    mut commands: Commands,
    engine: NonSend<Engine>,
    time: Res<Time>,
    dimension: Res<Dimension>,
    mut state: ResMut<LightningState>,
    mut flash: ResMut<LightningFlash>,
    mut sources: Option<ResMut<crate::atmosphere::AtmosphereSources>>,
    sound: Option<Res<SoundState>>,
    mut audio: ResMut<Assets<AudioSource>>,
    assets: Res<AssetServer>,
    cameras: Query<&GlobalTransform, With<WorldCamera>>,
    mut lights: Query<&mut PointLight, With<FlashLight>>,
) {
    let lightning = &engine.project.world.lightning;
    let state = &mut *state;
    if !engine.running {
        for strike in state.strikes.drain(..) {
            if let Some(light) = strike.light {
                commands.entity(light).despawn();
            }
        }
        state.queued.clear();
        flash.level = 0.0;
        return;
    }
    let mode = dimension.0;
    let camera = cameras.iter().next().map(|c| c.translation());
    for at in state.queued.drain(..) {
        let at = if mode == Mode::TwoD {
            at.with_z(0.0)
        } else {
            at
        };
        let light = (mode == Mode::ThreeD).then(|| {
            commands
                .spawn((
                    FlashLight,
                    PointLight {
                        color: parse_color(&lightning.color),
                        intensity: 0.0,
                        range: lightning.range,
                        radius: 2.0,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(at + Vec3::Y * lightning.flash_height),
                ))
                .id()
        });
        let distance = camera.map_or(0.0, |camera| {
            let metres = camera.distance(at);
            if mode == Mode::TwoD {
                metres / crate::dim2::PIXELS_PER_METER
            } else {
                metres
            }
        });
        state.strikes.push(Strike {
            at,
            since: 0.0,
            thunder: lightning
                .thunder
                .then(|| Lightning::thunder_delay(distance)),
            heard: false,
            light,
        });
    }

    let dt = if engine.paused {
        0.0
    } else {
        time.delta_secs()
    };
    let mut level = 0.0f32;
    let mut done = Vec::new();
    for (index, strike) in state.strikes.iter_mut().enumerate() {
        strike.since += dt;
        let now = lightning.flash(strike.since);
        level = level.max(now);
        if let Some(light) = strike.light
            && let Ok(mut point) = lights.get_mut(light)
        {
            point.intensity = lightning.intensity * now;
        }
        if let Some(delay) = strike.thunder
            && !strike.heard
            && strike.since >= delay
        {
            strike.heard = true;
            let distance = camera.map_or(0.0, |camera| camera.distance(strike.at));
            let handle = thunder_sound(
                &engine,
                lightning,
                &mut state.thunder,
                &mut state.custom,
                &mut audio,
                &assets,
            );
            let mix = sound.as_ref().map_or(1.0, |sound| {
                sound.mixer.master_volume * sound.mixer.gain(SoundBus::Sfx)
            });
            // Far thunder is quieter, but never silent.
            let fade = 1.0 / (1.0 + distance / 1500.0);
            let volume = mix * lightning.thunder_volume / 100.0 * fade;
            commands.spawn((
                AudioPlayer(handle),
                PlaybackSettings {
                    volume: Volume::Linear(volume.max(0.0)),
                    ..PlaybackSettings::DESPAWN
                },
                Thunder,
            ));
        }
        let flashing = strike.since < lightning.decay * 1.5;
        let waiting = strike.thunder.is_some() && !strike.heard;
        if !flashing && !waiting {
            done.push(index);
        }
    }
    for index in done.into_iter().rev() {
        let strike = state.strikes.remove(index);
        if let Some(light) = strike.light {
            commands.entity(light).despawn();
        }
    }
    let color = parse_color(&lightning.color).to_linear();
    *flash = LightningFlash {
        level,
        color: Vec3::new(color.red, color.green, color.blue),
        pulse: lightning.sky_pulse,
    };
    if let Some(sources) = sources.as_mut() {
        sources.lightning = level;
    }
}

/// The project's thunder file, or the built-in rumble.
fn thunder_sound(
    engine: &Engine,
    lightning: &Lightning,
    built_in: &mut Option<Handle<AudioSource>>,
    custom: &mut Option<(String, Handle<AudioSource>)>,
    audio: &mut Assets<AudioSource>,
    assets: &AssetServer,
) -> Handle<AudioSource> {
    let path = lightning.thunder_sound.as_str();
    if !path.is_empty() {
        if let Some((cached, handle)) = custom.as_ref()
            && cached == path
        {
            return handle.clone();
        }
        let resolved = engine
            .project_dir
            .as_deref()
            .and_then(|dir| blockloom_core::assets::resolve(dir, path))
            .map(|resolved| resolved.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        let handle = assets.load::<AudioSource>(resolved);
        *custom = Some((path.to_string(), handle.clone()));
        return handle;
    }
    built_in
        .get_or_insert_with(|| {
            audio.add(AudioSource {
                bytes: thunder_wav(lightning.seed).into(),
            })
        })
        .clone()
}

/// Lays the flash over the blended environment: the ambient light jumps
/// and the background washes towards the flash's color.
fn flash_environment(flash: Res<LightningFlash>, mut environment: ResMut<Environment>) {
    if flash.level <= 0.0 {
        return;
    }
    let env = &mut *environment;
    let jump = flash.level * flash.pulse;
    env.ambient_brightness *= 1.0 + jump;
    let wash = (flash.level * (flash.pulse / 6.0).min(1.0) * 0.6).clamp(0.0, 1.0);
    let target = LinearRgba::rgb(flash.color.x, flash.color.y, flash.color.z);
    env.background = env.background.to_linear().mix(&target, wash).into();
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::vm::Effect;

    fn app(mode: Mode) -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::audio::AudioPlugin::default(),
        ))
        .insert_non_send(Engine::new(rx, mode))
        .insert_resource(Dimension(mode))
        .init_resource::<PendingEffects>()
        .init_resource::<Environment>()
        .init_resource::<crate::atmosphere::AtmosphereSources>();
        app.init_resource::<LightningState>()
            .init_resource::<LightningFlash>()
            .add_systems(
                Update,
                (
                    apply_lightning_effects,
                    direct_storm,
                    step_strikes,
                    flash_environment,
                )
                    .chain(),
            );
        app
    }

    #[test]
    fn a_strike_flashes_the_sky_and_lights_the_world_in_3d() {
        let mut app = app(Mode::ThreeD);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::StrikeLightning {
                at: [10.0, 0.0, -5.0],
            });
        let before = Environment::default().ambient_brightness;
        app.update();
        let flash = *app.world().resource::<LightningFlash>();
        assert!(flash.level > 0.9, "{flash:?}");
        assert!(app.world().resource::<Environment>().ambient_brightness > before * 2.0);
        let mut lights = app
            .world_mut()
            .query_filtered::<&Transform, With<FlashLight>>();
        let light = lights.single(app.world()).unwrap();
        assert_eq!(light.translation, Vec3::new(10.0, 60.0, -5.0));
        assert!(
            app.world()
                .resource::<crate::atmosphere::AtmosphereSources>()
                .lightning
                > 0.9
        );
    }

    #[test]
    fn a_2d_strike_has_no_light_but_still_flashes() {
        let mut app = app(Mode::TwoD);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::StrikeLightning { at: [0.0; 3] });
        app.update();
        assert!(app.world().resource::<LightningFlash>().level > 0.9);
        let mut lights = app.world_mut().query_filtered::<(), With<FlashLight>>();
        assert_eq!(lights.iter(app.world()).count(), 0);
    }

    #[test]
    fn the_storm_block_sets_the_rate_for_the_run() {
        let mut app = app(Mode::ThreeD);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::SetLightningRate { rate: 30.0 });
        app.update();
        assert_eq!(app.world().non_send::<Engine>().lightning_rate, Some(30.0));
    }

    #[test]
    fn a_stopped_world_forgets_its_strikes() {
        let mut app = app(Mode::ThreeD);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::StrikeLightning { at: [0.0; 3] });
        app.update();
        app.world_mut().non_send_mut::<Engine>().running = false;
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        app.update();
        assert_eq!(app.world().resource::<LightningFlash>().level, 0.0);
        let mut lights = app.world_mut().query_filtered::<(), With<FlashLight>>();
        assert_eq!(lights.iter(app.world()).count(), 0);
    }
}
