//! The sense snapshot's atmosphere slot: sun, wind, fog and weather, sampled
//! once per fixed tick so the VM, compiled logic and scripts all read the
//! same air on the same tick.
//!
//! The sun comes from the blended `Environment`. Wind, fog and weather have no
//! writer yet; Phase 5's systems fill `AtmosphereSources` and this picks them up.

use crate::engine::Dimension;
use crate::engine::Engine;
use crate::environment::Environment;
use crate::hdr::HdrFrame;
use crate::luminance::SceneLuminance;
use bevy::prelude::*;
use blockloom_core::sense::{self, ATMOSPHERE_VERSION, AtmosphereSense};

pub fn register(app: &mut App) {
    app.init_resource::<AtmosphereSources>()
        .init_resource::<Atmosphere>();
}

/// What the wind, fog and weather systems say right now. Each writes its own
/// part; the defaults are calm, clear and mild.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct AtmosphereSources {
    /// Direction times speed, world units per second.
    pub wind: Vec3,
    pub gust: f32,
    pub fog_density: f32,
    pub fog_color: LinearRgba,
    pub cloud_cover: f32,
    pub rain: f32,
    pub snow: f32,
    pub wetness: f32,
    pub temperature: f32,
}

impl Default for AtmosphereSources {
    fn default() -> Self {
        let calm = AtmosphereSense::default();
        Self {
            wind: Vec3::ZERO,
            gust: calm.wind_gust,
            fog_density: calm.fog_density,
            fog_color: LinearRgba::WHITE,
            cloud_cover: calm.cloud_cover,
            rain: calm.rain,
            snow: calm.snow,
            wetness: calm.wetness,
            temperature: calm.temperature,
        }
    }
}

/// The last sample, which `publish_sensors` copies into every frame's
/// snapshot so a frame between ticks doesn't read a fresher one.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Atmosphere(pub AtmosphereSense);

/// Builds this tick's slot and publishes it ahead of the schedulers. A paused
/// world keeps its air; a stopped one resamples at tick 0, so a reporter
/// previewed in the editor still answers with the project's sun.
pub fn sample_atmosphere(
    engine: NonSend<Engine>,
    environment: Res<Environment>,
    sources: Res<AtmosphereSources>,
    display: Display,
    mut atmosphere: ResMut<Atmosphere>,
) {
    let tick = if !engine.running {
        0
    } else if engine.paused {
        return;
    } else {
        atmosphere.0.tick + 1
    };
    atmosphere.0 = sample(tick, &environment, &sources, &display.reading(&environment));
    sense::publish_atmosphere(atmosphere.0.clone());
}

/// What the display half of the slot reads from. Optional, so a bare world
/// without the HDR frame or the meter still samples.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Display<'w> {
    frame: Option<Res<'w, HdrFrame>>,
    luminance: Option<Res<'w, SceneLuminance>>,
    dimension: Option<Res<'w, Dimension>>,
}

/// The display readings for one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct DisplayReading {
    luminance: f32,
    hdr: bool,
    peak: f32,
}

impl Display<'_> {
    fn reading(&self, environment: &Environment) -> DisplayReading {
        let frame = self.frame.as_deref().copied().unwrap_or_default();
        let mode = self
            .dimension
            .as_deref()
            .map_or(blockloom_core::scene::Mode::ThreeD, |d| d.0);
        DisplayReading {
            luminance: self.luminance.as_deref().map_or(0.0, |meter| {
                meter.nits(mode, environment.exposure, frame.paper_white_nits)
            }),
            hdr: frame.is_hdr(),
            peak: frame.peak_nits,
        }
    }
}

fn sample(
    tick: u64,
    environment: &Environment,
    sources: &AtmosphereSources,
    display: &DisplayReading,
) -> AtmosphereSense {
    let rgb = |color: LinearRgba| [color.red, color.green, color.blue];
    let wind_speed = sources.wind.length();
    AtmosphereSense {
        version: ATMOSPHERE_VERSION,
        tick,
        sun_direction: environment.sun.direction.to_array(),
        sun_color: rgb(environment.sun.color.to_linear()),
        sun_illuminance: environment.sun.illuminance,
        wind_direction: sources.wind.normalize_or_zero().to_array(),
        wind_speed,
        wind_gust: sources.gust.max(0.0),
        fog_density: sources.fog_density.max(0.0),
        fog_color: rgb(sources.fog_color),
        cloud_cover: sources.cloud_cover.clamp(0.0, 1.0),
        rain: sources.rain.clamp(0.0, 1.0),
        snow: sources.snow.clamp(0.0, 1.0),
        wetness: sources.wetness.clamp(0.0, 1.0),
        temperature: sources.temperature,
        exposure: environment.exposure,
        luminance: display.luminance,
        hdr_display: display.hdr,
        peak_brightness: display.peak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Mode;

    fn app() -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        app.insert_non_send(Engine::new(rx, Mode::ThreeD))
            .init_resource::<Environment>();
        register(&mut app);
        app.add_systems(Update, sample_atmosphere);
        app
    }

    #[test]
    fn the_slot_counts_ticks_only_while_running() {
        let mut app = app();
        app.update();
        assert_eq!(app.world().resource::<Atmosphere>().0.tick, 0);

        app.world_mut().non_send_mut::<Engine>().running = true;
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Atmosphere>().0.tick, 2);

        app.world_mut().non_send_mut::<Engine>().paused = true;
        app.update();
        assert_eq!(app.world().resource::<Atmosphere>().0.tick, 2);
    }

    #[test]
    fn the_display_half_reads_the_frame_and_the_meter() {
        let mut app = app();
        app.update();
        let air = app.world().resource::<Atmosphere>().0.clone();
        assert_eq!((air.luminance, air.hdr_display), (0.0, false));

        app.insert_resource(Dimension(blockloom_core::scene::Mode::ThreeD))
            .insert_resource(HdrFrame {
                space: blockloom_core::scene::OutputSpace::Scrgb,
                peak_nits: 600.0,
                ..default()
            })
            .insert_resource(SceneLuminance {
                average: 0.5,
                peak: 1.0,
                measured: true,
            });
        app.world_mut().resource_mut::<Environment>().exposure = 0.0;
        app.update();
        let air = &app.world().resource::<Atmosphere>().0;
        assert!(air.hdr_display);
        assert_eq!(air.peak_brightness, 600.0);
        assert_eq!(air.luminance, 0.6);
    }

    #[test]
    fn the_sample_reaches_the_published_snapshot() {
        let mut app = app();
        app.world_mut().resource_mut::<AtmosphereSources>().wind = Vec3::new(0.0, 0.0, -5.0);
        app.update();
        let (speed, direction, version) = sense::read(|sensors| {
            let air = &sensors.atmosphere;
            (air.wind_speed, air.wind_direction, air.version)
        });
        assert_eq!(speed, 5.0);
        assert_eq!(direction, [0.0, 0.0, -1.0]);
        assert_eq!(version, ATMOSPHERE_VERSION);
        let sun = app
            .world()
            .resource::<Environment>()
            .sun
            .direction
            .to_array();
        assert_eq!(app.world().resource::<Atmosphere>().0.sun_direction, sun);
    }
}
