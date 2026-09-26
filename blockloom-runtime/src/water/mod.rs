//! Water (`blockloom_core::water`) in the world. `sample_water` resolves
//! every body at the head of each fixed tick, before the schedulers, into
//! `WaterSample` (what the reporters, scripts and buoyancy read) and
//! `WaterState` (what the surfaces draw). The drawn surface sums the same
//! waves from the same numbers, so what floats sits on what is drawn.

mod buoy;
mod flat;
mod surface;
mod under;

pub use buoy::{float_bodies_2d, float_bodies_3d};
pub use surface::WaterSurface;

use crate::atmosphere::sample_atmosphere;
use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use crate::wind::WindField;
use bevy::prelude::*;
use blockloom_core::scene::Mode;
use blockloom_core::sense;
use blockloom_core::vm::Effect;
use blockloom_core::water::{
    LiveDetail, MAX_RIPPLES, WaterBody, WaterDials, WaterKind, WaterSense, WaterSpec, WaveSet,
    Waves, sea_target, settle_sea,
};
use std::collections::HashMap;

pub fn register(app: &mut App, mode: Mode) {
    app.init_resource::<WaterState>()
        .init_resource::<WaterSample>()
        .add_systems(
            FixedUpdate,
            (
                sample_water
                    .in_set(crate::world::SimulationSet)
                    .after(sample_atmosphere)
                    .before(crate::world::step_vm),
                apply_water_effects
                    .in_set(crate::world::SimulationSet)
                    .after(crate::world::apply_common)
                    .before(crate::world::clear_effects),
            ),
        );
    match mode {
        Mode::TwoD => flat::register(app),
        Mode::ThreeD => {
            surface::register(app);
            under::register(app);
        }
    }
}

/// Seconds a splash ripple lasts.
const RIPPLE_LIFE: f32 = 4.0;

/// Every body as of the last fixed tick, as the reporters read it.
#[derive(Resource, Clone, Debug, Default)]
pub struct WaterSample(pub WaterSense);

/// A ring spreading from a splash.
#[derive(Clone, Debug)]
pub struct Ripple {
    pub body: String,
    pub at: [f32; 2],
    pub age: f32,
    /// Height of the ring, world units.
    pub strength: f32,
}

/// One body resolved for drawing.
#[derive(Clone, Debug)]
pub struct LiveBody {
    pub body: WaterBody,
    pub spec: WaterSpec,
    pub dials: WaterDials,
    pub detail: Vec<LiveDetail>,
}

impl LiveBody {
    pub fn foam(&self) -> f32 {
        self.dials.foam.unwrap_or(self.spec.foam.amount)
    }
}

/// Everything the water keeps between ticks.
#[derive(Resource, Default)]
pub struct WaterState {
    /// Seconds of water time: runs with the world, holds while paused and
    /// starts over on Stop.
    pub time: f32,
    pub bodies: Vec<LiveBody>,
    pub ripples: Vec<Ripple>,
    /// Each body's sea state against its authored height.
    sea: HashMap<String, f32>,
    sets: HashMap<String, (SetKey, WaveSet)>,
    /// Whether each moving actor was in the water last tick, for splashes.
    wet: HashMap<Entity, bool>,
}

/// What a body's waves were built from: a new one of these rebuilds them.
#[derive(Clone, Debug, PartialEq)]
struct SetKey {
    waves: Waves,
    detail_scale: f32,
    heading: f32,
    gravity: f32,
    flat: bool,
}

impl WaterState {
    fn waves_for(&mut self, id: &str, key: SetKey, spec: &WaterSpec) -> &WaveSet {
        let stale = self.sets.get(id).is_none_or(|(built, _)| *built != key);
        if stale {
            let set = WaveSet::build(spec, key.heading, key.gravity);
            let set = if key.flat { set.flattened() } else { set };
            self.sets.insert(id.to_string(), (key, set));
        }
        &self.sets[id].1
    }

    /// Adds a ripple, dropping the oldest past the shader's limit.
    pub fn ripple(&mut self, ripple: Ripple) {
        self.ripples.push(ripple);
        if self.ripples.len() > MAX_RIPPLES {
            let extra = self.ripples.len() - MAX_RIPPLES;
            self.ripples.drain(..extra);
        }
    }

    pub fn body(&self, id: &str) -> Option<&LiveBody> {
        self.bodies.iter().find(|live| live.body.id == id)
    }
}

/// The project's pull, straight down, or the dimension's default when a
/// project has none: waves need some gravity to travel.
pub fn gravity_of(engine: &Engine, flat: bool) -> f32 {
    let g = engine.project.world.gravity[1].abs();
    if g > 1.0e-3 {
        g
    } else if flat {
        981.0
    } else {
        9.81
    }
}

/// A hex color as linear rgb.
pub(crate) fn rgb(hex: &str) -> Vec3 {
    Vec4::from(blockloom_core::material::hex_to_linear(hex)).truncate()
}

/// Resolves every water body for this tick and publishes it ahead of the
/// schedulers. A stopped world holds the water at the start of a run.
#[allow(clippy::too_many_arguments)]
pub fn sample_water(
    engine: NonSend<Engine>,
    dimension: Res<Dimension>,
    time: Res<Time<Fixed>>,
    wind: Option<Res<WindField>>,
    actors: Query<(&ActorId, &Transform)>,
    mut state: ResMut<WaterState>,
    mut sample: ResMut<WaterSample>,
) {
    if engine.running && engine.paused {
        return;
    }
    let flat = dimension.0 == Mode::TwoD;
    let dt = time.delta_secs();
    let state = &mut *state;
    if engine.running {
        state.time += dt;
        for ripple in &mut state.ripples {
            ripple.age += dt;
        }
        state.ripples.retain(|ripple| ripple.age < RIPPLE_LIFE);
    } else {
        state.time = 0.0;
        state.ripples.clear();
        state.wet.clear();
    }
    let gravity = gravity_of(&engine, flat);
    let heading = engine.project.world.wind.direction;
    let mut bodies = Vec::new();
    for (id, transform) in &actors {
        if !engine.has_component(&id.0, "Water") {
            continue;
        }
        let Some(spec) = engine
            .actor(&id.0)
            .and_then(|actor| actor.components.water())
        else {
            continue;
        };
        let dials = engine.water.for_body(&id.0);
        let mut center = transform.translation;
        if let Some(level) = dials.level {
            center.y = level;
        }
        let (axis, flow, half) = if flat {
            (
                [1.0, 0.0],
                [spec.flow[0], 0.0],
                [spec.size[0] * 0.5 * transform.scale.x.abs(), 0.0],
            )
        } else {
            let x = transform.rotation * Vec3::X;
            let axis = Vec2::new(x.x, x.z).normalize_or(Vec2::X);
            let flow = transform.rotation * Vec3::new(spec.flow[0], 0.0, spec.flow[1]);
            (
                axis.to_array(),
                [flow.x, flow.z],
                [
                    spec.size[0] * 0.5 * transform.scale.x.abs(),
                    spec.size[1] * 0.5 * transform.scale.z.abs(),
                ],
            )
        };
        let key = SetKey {
            waves: spec.waves.clone(),
            detail_scale: spec.detail.scale,
            heading,
            gravity,
            flat,
        };
        let air = wind
            .as_deref()
            .map_or(0.0, |field| field.at(center).length());
        // The fetch fit is in metres: a 2D pixel is a hundredth of one.
        let (air_m, gravity_m) = if flat {
            (air / 100.0, gravity / 100.0)
        } else {
            (air, gravity)
        };
        let target = sea_target(air_m, spec.waves.fetch, spec.waves.wind, gravity_m);
        let sea = state.sea.entry(id.0.clone()).or_insert(target);
        *sea = if engine.running {
            settle_sea(*sea, target, dt)
        } else {
            target
        };
        let sea = *sea;
        let chop = dials.chop.unwrap_or(spec.waves.chop);
        let set = state.waves_for(&id.0, key, spec);
        let waves = set.resolve(sea, chop, spec.waves.steepness);
        let detail = set.resolve_detail(spec.detail.strength, chop, air, gravity);
        let body = WaterBody {
            id: id.0.clone(),
            kind: spec.kind,
            center: center.to_array(),
            axis,
            half: if spec.kind == WaterKind::Ocean {
                [f32::INFINITY; 2]
            } else {
                half
            },
            depth: spec.depth * if flat { transform.scale.y.abs() } else { 1.0 },
            flow,
            waves,
            flat,
        };
        bodies.push(LiveBody {
            body,
            spec: spec.clone(),
            dials,
            detail,
        });
    }
    // Bodies that went away take their caches with them.
    state
        .sea
        .retain(|id, _| bodies.iter().any(|live| live.body.id == *id));
    state
        .sets
        .retain(|id, _| bodies.iter().any(|live| live.body.id == *id));
    state
        .ripples
        .retain(|ripple| bodies.iter().any(|live| live.body.id == ripple.body));
    state.bodies = bodies;
    sample.0 = WaterSense {
        bodies: state.bodies.iter().map(|live| live.body.clone()).collect(),
        time: state.time,
    };
    sense::publish_water(sample.0.clone());
}

/// `set water level/chop/foam` for the rest of the run: on the water actor
/// that ran it, or on every body from anyone else.
fn apply_water_effects(effects: Res<PendingEffects>, mut engine: NonSendMut<Engine>) {
    if !engine.running {
        return;
    }
    for effect in &effects.0 {
        if let Effect::SetWater {
            actor,
            property,
            value,
        } = effect
        {
            let body = engine
                .has_component(actor, "Water")
                .then_some(actor.as_str());
            let body = body.map(str::to_string);
            engine.water.set(body.as_deref(), *property, *value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::components::ActorComponent;
    use blockloom_core::water::WaterProperty;

    fn app(mode: Mode) -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        let mut engine = Engine::new(rx, mode);
        let mut pool = engine.project.actors[0].clone();
        pool.id = "pool".to_string();
        pool.name = "Pool".to_string();
        let spec = match mode {
            Mode::TwoD => WaterSpec::flat(),
            Mode::ThreeD => WaterSpec::default(),
        };
        pool.components
            .insert(ActorComponent::Water { water: spec });
        engine.project.actors.push(pool);
        engine
            .attached
            .insert("pool".to_string(), ["Water".to_string()].into());
        app.insert_non_send(engine)
            .insert_resource(Dimension(mode))
            .init_resource::<Time<Fixed>>()
            .init_resource::<PendingEffects>()
            .init_resource::<WaterState>()
            .init_resource::<WaterSample>()
            .add_systems(Update, (apply_water_effects, sample_water).chain());
        app.world_mut().spawn((
            ActorId("pool".to_string()),
            Transform::from_xyz(0.0, 2.0, 0.0),
        ));
        app
    }

    #[test]
    fn a_body_is_published_at_its_level() {
        let mut app = app(Mode::ThreeD);
        app.update();
        let sense = app.world().resource::<WaterSample>().0.clone();
        assert_eq!(sense.bodies.len(), 1);
        let height = sense.height_at(0.0, 0.0).unwrap();
        assert!((height - 2.0).abs() < 1.0, "{height}");
        assert!(sense.height_at(100.0, 0.0).is_none());
        assert!(sense.underwater([0.0, -2.0, 0.0]));
        assert!(!sense.underwater([0.0, 5.0, 0.0]));
        assert!(sense::read(|s| s.water.bodies.len()) == 1);
    }

    #[test]
    fn set_water_level_moves_the_surface_for_the_run() {
        let mut app = app(Mode::ThreeD);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::SetWater {
                actor: "someone".to_string(),
                property: WaterProperty::Level,
                value: -10.0,
            });
        app.update();
        let sense = app.world().resource::<WaterSample>().0.clone();
        assert_eq!(sense.bodies[0].center[1], -10.0);
    }

    #[test]
    fn flat_water_waves_run_along_x() {
        let mut app = app(Mode::TwoD);
        app.update();
        let sense = app.world().resource::<WaterSample>().0.clone();
        let body = &sense.bodies[0];
        assert!(body.flat);
        assert!(body.waves.iter().all(|wave| wave.dir[1] == 0.0));
        assert!(sense.height_at(0.0, 999.0).is_some());
    }
}
