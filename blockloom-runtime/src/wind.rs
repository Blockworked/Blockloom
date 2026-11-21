//! The wind (`blockloom_core::wind`) in the world, both dimensions. Stepped
//! on the fixed tick's own clock, so a replay gusts the same, and handed on
//! as `WindField`: particles, fog noise and the clouds read that one field,
//! and the atmosphere slot reports it at the camera.

use crate::atmosphere::{AtmosphereSources, sample_atmosphere};
use crate::director::WeatherNow;
use crate::engine::{ActorId, Dimension, PendingEffects, SceneDocument};
use crate::module::{BlockloomModule, RunState, WorldRebuilt};
use crate::volumes::LiveVolumes;
use crate::world::WorldCamera;
use bevy::prelude::*;
use blockloom_core::scene::Mode;
use blockloom_core::vm::Effect;
use blockloom_core::wind::{CloudOffsets, Wind, WindDials, WindOverrides, WindZone, direction_of};

/// The first `BlockloomModule`: no `Engine` anywhere in it.
pub struct WindModule;

impl BlockloomModule for WindModule {
    fn name(&self) -> &'static str {
        "wind"
    }

    fn build(&self, app: &mut App) {
        app.init_resource::<WindField>()
            .init_resource::<WindRun>()
            .init_resource::<WeatherNow>()
            .init_resource::<LiveVolumes>()
            .add_systems(
                FixedUpdate,
                (
                    step_wind
                        .in_set(crate::world::SimulationSet)
                        .after(crate::director::publish_weather)
                        .after(crate::volumes::publish_live_volumes)
                        .before(sample_atmosphere),
                    apply_wind_effects
                        .in_set(crate::world::SimulationSet)
                        .after(crate::world::apply_common)
                        .before(crate::world::clear_effects),
                ),
            )
            .add_systems(Update, forget_wind_run.after(crate::world::rebuild_world));
    }
}

/// What `set wind` and `set cloud drift` wrote this run.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct WindRun(pub WindOverrides);

fn forget_wind_run(mut rebuilt: MessageReader<WorldRebuilt>, mut run: ResMut<WindRun>) {
    if rebuilt.read().count() > 0 {
        *run = WindRun::default();
    }
}

/// The air as of the last fixed tick.
#[derive(Resource, Clone, Debug, Default)]
pub struct WindField {
    pub wind: Wind,
    pub dials: WindDials,
    /// Seconds of run the gusts are at.
    pub time: f32,
    pub flat: bool,
    pub zones: Vec<WindZone>,
    /// How far the base wind has carried the air since Play: what drifting
    /// noise (fog, dust) scrolls by.
    pub drift: Vec3,
    pub clouds: CloudOffsets,
    /// The wind where the camera is.
    pub at_camera: Vec3,
}

impl WindField {
    /// The wind at a point, zones and all.
    pub fn at(&self, point: Vec3) -> Vec3 {
        Vec3::from(self.wind.sample(
            &self.dials,
            self.time,
            point.to_array(),
            self.flat,
            &self.zones,
        ))
    }
}

/// `set wind` and `set cloud drift` for the rest of the run.
fn apply_wind_effects(
    effects: Res<PendingEffects>,
    state: Res<RunState>,
    mut run: ResMut<WindRun>,
) {
    if !state.running {
        return;
    }
    for effect in &effects.0 {
        match effect {
            Effect::SetWind { property, value } => run.0.set(*property, *value),
            Effect::SetCloudDrift { drift } => run.0.set_cloud_drift(*drift),
            _ => {}
        }
    }
}

/// Advances the wind a tick and samples it at the camera for the
/// atmosphere slot. A stopped world holds it at the start of a run; a
/// paused one holds it where it is.
#[allow(clippy::too_many_arguments)]
fn step_wind(
    state: Res<RunState>,
    document: Res<SceneDocument>,
    run: Res<WindRun>,
    weather: Res<WeatherNow>,
    volumes: Res<LiveVolumes>,
    dimension: Res<Dimension>,
    time: Res<Time<Fixed>>,
    cameras: Query<&Transform, With<WorldCamera>>,
    actors: Query<(&ActorId, &Transform), Without<WorldCamera>>,
    mut field: ResMut<WindField>,
    mut sources: ResMut<AtmosphereSources>,
) {
    if state.running && state.paused {
        return;
    }
    let flat = dimension.0 == Mode::TwoD;
    let overrides = &run.0;
    let wind = &document.world.wind;
    let mut dials = wind.dials(overrides);
    // The weather blend lays under explicit `set wind` dials: where a block
    // moved a dial, the block wins; everywhere else the weather shows.
    if weather.active {
        let blended = weather.values;
        if overrides.direction.is_none() {
            dials.direction = blended.wind_direction;
        }
        if overrides.speed.is_none() {
            dials.speed = blended.wind_speed;
        }
        if overrides.storm.is_none() {
            dials.storm = blended.storm;
        }
    }
    let field = &mut *field;
    if state.running {
        let dt = time.delta_secs();
        // This tick's wind carries the air over the step it starts.
        let (base, _) = wind.base(&dials, field.time, flat);
        let aloft = wind.aloft(&dials, field.time, flat);
        let extra = overrides.cloud_drift.unwrap_or(wind.clouds.advection);
        field.drift += Vec3::from(base) * dt;
        field.clouds.step(&wind.clouds, aloft, extra, dt, flat);
        field.time += dt;
    } else {
        field.time = 0.0;
        field.drift = Vec3::ZERO;
        field.clouds = CloudOffsets::default();
    }
    field.wind.clone_from(wind);
    field.dials = dials;
    field.flat = flat;
    field.zones = WindZone::from_volumes(volumes.placed(actors.iter()));
    let eye = cameras
        .iter()
        .next()
        .map_or(Vec3::ZERO, |camera| camera.translation);
    field.at_camera = field.at(eye);

    let (_, gust) = wind.base(&dials, field.time, flat);
    sources.wind = field.at_camera;
    sources.gust = gust;
    sources.wind_heading = if field.at_camera.length_squared() > 1.0e-12 {
        direction_of(field.at_camera.to_array(), flat)
    } else {
        dials.direction
    };
    sources.storm = dials.storm;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, InsertEngine};
    use blockloom_core::wind::WindProperty;

    fn app(mode: Mode) -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        let mut engine = Engine::new(rx, mode);
        engine.project.world.wind = Wind {
            direction: 90.0,
            speed: 4.0,
            veer: 0.0,
            profile: false,
            ..Wind::default()
        };
        app.insert_engine(engine)
            .insert_resource(Dimension(mode))
            .init_resource::<PendingEffects>()
            .init_resource::<AtmosphereSources>()
            .init_resource::<Time<Fixed>>()
            .init_resource::<WindField>()
            .init_resource::<WindRun>()
            .init_resource::<RunState>()
            .init_resource::<WeatherNow>()
            .init_resource::<LiveVolumes>()
            .add_message::<WorldRebuilt>()
            .add_systems(
                Update,
                (step_wind, apply_wind_effects, forget_wind_run).chain(),
            );
        app
    }

    #[test]
    fn a_stopped_world_reports_the_projects_wind_at_the_start() {
        let mut app = app(Mode::ThreeD);
        app.update();
        let sources = app.world().resource::<AtmosphereSources>();
        assert!((sources.wind - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-4);
        assert!((sources.wind_heading - 90.0).abs() < 1e-3);
        assert_eq!(app.world().resource::<WindField>().time, 0.0);
    }

    #[test]
    fn blocks_move_the_dials_for_the_rest_of_the_run() {
        let mut app = app(Mode::TwoD);
        app.world_mut().resource_mut::<RunState>().running = true;
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![
            Effect::SetWind {
                property: WindProperty::Direction,
                value: 0.0,
            },
            Effect::SetWind {
                property: WindProperty::Storm,
                value: 1.0,
            },
            Effect::SetCloudDrift {
                drift: [1.0, 2.0, 3.0],
            },
        ];
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.update();
        let sources = app.world().resource::<AtmosphereSources>();
        // North is up the screen in 2D, and a full storm triples the speed.
        assert!(
            (sources.wind - Vec3::new(0.0, 12.0, 0.0)).length() < 1e-3,
            "{}",
            sources.wind
        );
        assert_eq!(sources.storm, 1.0);
        let run = app.world().resource::<WindRun>();
        assert_eq!(run.0.cloud_drift, Some([1.0, 2.0, 3.0]));
    }

    #[test]
    fn a_rebuild_forgets_the_runs_dials() {
        let mut app = app(Mode::ThreeD);
        app.world_mut().resource_mut::<WindRun>().0.cloud_drift = Some([1.0, 0.0, 0.0]);
        app.world_mut().write_message(WorldRebuilt);
        app.update();
        assert_eq!(app.world().resource::<WindRun>().0.cloud_drift, None);
    }
}
