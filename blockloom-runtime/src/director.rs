//! Time-of-day and weather director in the world: a 24h clock driving the
//! authored curve tracks, and named weather presets blending into each other
//! without popping. Stepped on the fixed tick's own clock, so a replay gusts
//! the same, and sampled into the atmosphere slot ahead of the schedulers.

use crate::atmosphere::{AtmosphereSources, sample_atmosphere};
use crate::engine::{Engine, PendingEffects};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::storage::ShaderBuffer;
use blockloom_core::director::{
    Director, WeatherPreset, WeatherValues, WetnessMap, evaporation_rate,
};
use blockloom_core::vm::{Effect, Event};

pub fn register(app: &mut App) {
    app.init_resource::<DirectorClock>()
        .init_resource::<WetnessField>()
        .init_resource::<WetnessTexture>()
        .add_systems(
            FixedUpdate,
            (
                step_director
                    .in_set(crate::world::SimulationSet)
                    .before(sample_atmosphere),
                apply_director_effects
                    .in_set(crate::world::SimulationSet)
                    .after(crate::world::apply_common)
                    .before(crate::world::clear_effects),
            ),
        )
        .add_systems(Update, upload_wetness.after(crate::world::rebuild_world));
}

/// The ground wetness map: rain soaks it, evaporation dries it unevenly.
/// `sampled` is the last read at the camera, what surfaces and reporters get.
#[derive(Resource, Debug)]
pub struct WetnessField {
    pub map: WetnessMap,
    pub sampled: f32,
}

impl Default for WetnessField {
    fn default() -> Self {
        Self {
            map: WetnessMap::default(),
            sampled: 0.0,
        }
    }
}

/// The wetness map as a GPU texture, so surfaces read per-pixel dampness
/// instead of one sampled value. Row-major like the map's cells, wetness in
/// R; the frame in `SurfaceGlobals` says where it sits in the world.
#[derive(Resource, Default)]
pub struct WetnessTexture {
    image: Option<Handle<Image>>,
    pixels: Vec<u8>,
}

fn upload_wetness(
    wetness: Res<WetnessField>,
    mut texture: ResMut<WetnessTexture>,
    mut images: ResMut<Assets<Image>>,
    mut globals: ResMut<crate::materials::SurfaceGlobals>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut boxes: ResMut<Assets<crate::materials::BoxMaterial>>,
    terrain: Option<ResMut<Assets<crate::terrain::material::TerrainMaterial>>>,
) {
    let pixels: Vec<u8> = wetness
        .map
        .cells()
        .iter()
        .flat_map(|c| [(c.clamp(0.0, 1.0) * 255.0).round() as u8, 0, 0, 255])
        .collect();
    if texture.image.is_none() {
        let mut image = Image::new(
            Extent3d {
                width: WetnessMap::GRID as u32,
                height: WetnessMap::GRID as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels.clone(),
            TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default(),
        );
        image.texture_descriptor.label = Some("director/wetness_map");
        texture.image = Some(images.add(image));
    } else if pixels != texture.pixels
        && let Some(handle) = texture.image.as_ref()
        && let Some(mut image) = images.get_mut(handle)
    {
        image.data = Some(pixels.clone());
    }
    texture.pixels = pixels;
    // The frame lands only once the texture exists, so a material never
    // samples the fallback image as real dampness.
    if texture.image.is_some() {
        let frame = wetness.map.frame();
        let frame = Vec4::new(frame[0], frame[1], frame[2], frame[3]);
        if frame != globals.data.wet_frame {
            globals.data.wet_frame = frame;
            if let Some(mut buffer) = buffers.get_mut(&globals.buffer) {
                *buffer = ShaderBuffer::from(vec![globals.data]);
            }
        }
    }
    let handle = texture.image.clone();
    let missing: Vec<_> = boxes
        .iter()
        .filter(|(_, m)| m.extension.wet_state != handle)
        .map(|(id, _)| id)
        .collect();
    for id in missing {
        if let Some(mut material) = boxes.get_mut(id) {
            material.extension.wet_state = handle.clone();
        }
    }
    if let Some(mut terrain) = terrain {
        let missing: Vec<_> = terrain
            .iter()
            .filter(|(_, m)| m.extension.wet_state != handle)
            .map(|(id, _)| id)
            .collect();
        for id in missing {
            if let Some(mut material) = terrain.get_mut(id) {
                material.extension.wet_state = handle.clone();
            }
        }
    }
}

/// The clock as of the last fixed tick, in hours 0-24.
#[derive(Resource, Clone, Copy, Debug)]
pub struct DirectorClock {
    pub time: f32,
}

impl Default for DirectorClock {
    fn default() -> Self {
        Self { time: 12.0 }
    }
}

/// `set time of day`, `advance time by`, `set precipitation` and
/// `blend weather to` for the rest of the run.
fn apply_director_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::SetTimeOfDay { time } if time.is_finite() => {
                engine.director_time = Some(time.rem_euclid(24.0));
            }
            Effect::AdvanceTime { hours } if hours.is_finite() => {
                let now = engine
                    .director_time
                    .unwrap_or_else(|| engine.project.world.director.time_of_day.rem_euclid(24.0));
                engine.director_time = Some((now + hours).rem_euclid(24.0));
            }
            Effect::SetPrecipitation { property, value } => {
                engine.precipitation.set(*property, *value);
            }
            Effect::BlendWeather { weather, seconds } => {
                let preset = engine.project.world.director.preset(weather).or_else(|| {
                    WeatherPreset::builtin(weather).or_else(|| {
                        // An unknown name reports itself the way a bad slot does.
                        crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
                            actor: "Blockloom".into(),
                            message: format!("there's no weather called \"{weather}\""),
                        });
                        None
                    })
                });
                if let Some(preset) = preset {
                    engine.weather.start_blend(&preset, *seconds);
                }
            }
            _ => {}
        }
    }
}

/// Advances the clock, steps the blend, and samples both into the
/// atmosphere sources. A stopped world holds the project's start; a paused
/// one holds where it is.
#[allow(clippy::too_many_arguments)]
fn step_director(
    mut engine: NonSendMut<Engine>,
    time: Res<Time<Fixed>>,
    mut clock: ResMut<DirectorClock>,
    mut sources: ResMut<AtmosphereSources>,
    mut wetness: ResMut<WetnessField>,
    camera: Query<&GlobalTransform, With<crate::world::WorldCamera>>,
) {
    let director = engine.project.world.director.clone();
    if !engine.running {
        clock.time = director.time_of_day.rem_euclid(24.0);
        engine.weather = Default::default();
        wetness.map.reset();
        write_sources(&director, clock.time, &engine, &mut sources);
        wetness.sampled = sources.wetness;
        wetness.map.sampled = sources.wetness;
        return;
    }
    if engine.paused {
        return;
    }
    let dt = time.delta_secs();
    // The clock: explicit block time wins, else the director's own rate.
    if let Some(set) = engine.director_time {
        clock.time = set.rem_euclid(24.0);
    } else if director.enabled {
        clock.time = (clock.time + director.rate() * dt).rem_euclid(24.0);
        if !director.loop_enabled {
            clock.time = clock.time.clamp(0.0, 24.0);
        }
    } else {
        clock.time = director.time_of_day.rem_euclid(24.0);
    }
    // The blend, on the same tick. A finished blend fires its hats.
    let (_, finished) = engine.weather.step(dt);
    if let Some(name) = finished {
        engine.fire(Event::Weather { weather: name });
    }
    write_sources(&director, clock.time, &engine, &mut sources);
    // The wetness map: rain soaks the ground round the camera while warm
    // sun and wind dry it back towards the weather's damp, unevenly.
    let weather = engine.weather.sampled();
    let active = engine.weather.is_active();
    let track = |t: &blockloom_core::director::TimeTrack| t.sample(clock.time);
    let target = if active {
        weather.wetness
    } else {
        track(&director.wetness).unwrap_or(wetness.sampled)
    };
    let sun_el = director
        .sun_elevation
        .sample(clock.time)
        .or(if active {
            Some(weather.sun_elevation)
        } else {
            None
        })
        .unwrap_or(45.0);
    let wind_speed = director
        .wind_speed
        .sample(clock.time)
        .or(if active {
            Some(weather.wind_speed)
        } else {
            None
        })
        .unwrap_or(0.0);
    let (cx, cz) = camera
        .iter()
        .next()
        .map(|t| (t.translation().x, t.translation().z))
        .unwrap_or((0.0, 0.0));
    let evaporation = if sources.snow > 0.5 && sources.temperature < 0.0 {
        0.0
    } else {
        evaporation_rate(sources.temperature, sun_el, wind_speed)
    };
    sources.wetness = wetness
        .map
        .step_at(cx, cz, target, sources.rain, evaporation, dt);
    wetness.sampled = sources.wetness;
}

/// Rain, snow, temperature, clock and weather name as of this tick.
/// Explicit `set precipitation` wins over tracks and blends. Wetness is the
/// map's read now (stepped just below), so the target below only seeds it.
fn write_sources(director: &Director, time: f32, engine: &Engine, sources: &mut AtmosphereSources) {
    let weather = engine.weather.sampled();
    let active = engine.weather.is_active();
    let track = |t: &blockloom_core::director::TimeTrack| t.sample(time);
    // Precipitation: explicit blocks, then the blend, then the track.
    let rain = engine.precipitation.rain.or_else(|| {
        if active {
            Some(weather.precipitation)
        } else {
            track(&director.precipitation)
        }
    });
    let snow = engine
        .precipitation
        .snow
        .or(if active { Some(weather.snow) } else { None });
    if let Some(rain) = rain {
        sources.rain = rain.clamp(0.0, 1.0);
    }
    if let Some(snow) = snow {
        sources.snow = snow.clamp(0.0, 1.0);
    }
    // Wetness lags the rain through the ground map (stepped below): seed
    // the sources from the weather's damp so a fresh run doesn't start dry
    // in a storm.
    let target = if active {
        weather.wetness
    } else {
        track(&director.wetness).unwrap_or(sources.wetness)
    };
    sources.wetness = target.clamp(0.0, 1.0);
    if active {
        sources.temperature = weather.temperature;
    } else if let Some(t) = track(&director.temperature) {
        sources.temperature = t;
    }
    sources.time_of_day = time.rem_euclid(24.0);
    sources.weather = engine.weather.current_name.clone();
}

/// The EV the director asks for this tick, if any: an explicit weather
/// blend's exposure, else the exposure track's.
pub fn director_exposure(
    director: &Director,
    time: f32,
    weather_active: bool,
    weather: &WeatherValues,
) -> Option<f32> {
    if !director.enabled && !weather_active {
        return None;
    }
    if weather_active {
        return Some(weather.exposure);
    }
    director.exposure.sample(time)
}

/// The sun's azimuth/elevation this tick, if the director moves it: tracks
/// first, then the weather blend's sun.
pub fn director_sun(
    director: &Director,
    time: f32,
    weather: &WeatherValues,
    active: bool,
) -> Option<(f32, f32)> {
    let az = director.sun_azimuth.sample(time).or(if active {
        Some(weather.sun_azimuth)
    } else {
        None
    });
    let el = director.sun_elevation.sample(time).or(if active {
        Some(weather.sun_elevation)
    } else {
        None
    });
    match (az, el) {
        (Some(az), Some(el)) => Some((az, el)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Mode;

    fn app() -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        let engine = Engine::new(rx, Mode::ThreeD);
        app.insert_non_send(engine)
            .init_resource::<PendingEffects>()
            .init_resource::<AtmosphereSources>()
            .init_resource::<Time<Fixed>>()
            .init_resource::<DirectorClock>()
            .init_resource::<WetnessField>()
            .add_systems(Update, (step_director, apply_director_effects).chain());
        app
    }

    #[test]
    fn a_stopped_world_reports_the_projects_clock() {
        let mut app = app();
        app.world_mut()
            .non_send_mut::<Engine>()
            .project
            .world
            .director = Director {
            enabled: true,
            time_of_day: 6.0,
            ..Director::default()
        };
        app.update();
        assert_eq!(app.world().resource::<DirectorClock>().time, 6.0);
    }

    #[test]
    fn blocks_move_the_clock_and_start_blends() {
        let mut app = app();
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![
            Effect::SetTimeOfDay { time: 18.5 },
            Effect::SetPrecipitation {
                property: blockloom_core::director::PrecipitationKind::Rain,
                value: 0.7,
            },
            Effect::BlendWeather {
                weather: "Storm".to_string(),
                seconds: 10.0,
            },
        ];
        // Effects land after the step that made them, the way wind and
        // lightning blocks do: one update applies, the next samples.
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        let engine = app.world().non_send::<Engine>();
        assert_eq!(engine.director_time, Some(18.5));
        assert_eq!(engine.precipitation.rain, Some(0.7));
        assert!(engine.weather.is_active());
        let sources = app.world().resource::<AtmosphereSources>();
        assert_eq!(sources.time_of_day, 18.5);
        assert!((sources.rain - 0.7).abs() < 1e-6);
    }

    #[test]
    fn rain_soaks_the_map_instead_of_snapping() {
        let mut app = app();
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut().resource_mut::<AtmosphereSources>().rain = 1.0;
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
        // The ground lags the rain: wet after a second, nowhere near soaked.
        let wet = app.world().resource::<AtmosphereSources>().wetness;
        assert!(wet > 0.0 && wet < 0.5, "{wet}");
        // A minute of downpour nearly soaks it.
        for _ in 0..60 {
            app.world_mut()
                .resource_mut::<Time<Fixed>>()
                .advance_by(std::time::Duration::from_secs(1));
            app.update();
        }
        assert!(app.world().resource::<AtmosphereSources>().wetness > 0.9);
    }

    #[test]
    fn an_unknown_weather_reports_itself() {
        let mut app = app();
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::BlendWeather {
            weather: "Drizzle".to_string(),
            seconds: 5.0,
        }];
        app.update();
        assert!(!app.world().non_send::<Engine>().weather.is_active());
    }
}
