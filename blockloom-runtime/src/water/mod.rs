//! Water (`blockloom_core::water`) in the world. `sample_water` resolves
//! every body at the head of each fixed tick, before the schedulers, into
//! `WaterSample` (what the reporters, scripts and buoyancy read) and
//! `WaterState` (what the surfaces draw). The drawn surface sums the same
//! waves from the same numbers, so what floats sits on what is drawn. Each
//! body's ripple field steps here too, and its swell turns with the wind by
//! fading from the old heading's waves to the new one's.

mod buoy;
mod flat;
mod mirror;
mod surface;
mod under;

pub use buoy::{float_bodies_2d, float_bodies_3d};
pub use surface::WaterSurface;

use crate::atmosphere::sample_atmosphere;
use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use crate::wind::WindField;
use crate::world::WorldCamera;
use bevy::prelude::*;
use blockloom_core::scene::Mode;
use blockloom_core::sense;
use blockloom_core::vm::Effect;
use blockloom_core::water::{
    CALM_FROM, LiveDetail, LiveWave, RIPPLE_CELLS, RIPPLE_CELLS_FLAT, RippleField, RippleSim,
    SEA_SETTLE, TURN_THRESHOLD, WaterBody, WaterDials, WaterKind, WaterSense, WaterSpec, WaveSet,
    Waves, blend_detail, blend_waves, heading_of, sea_target, settle_sea, turn_between,
};
use std::collections::HashMap;
use std::sync::Arc;

pub fn register(app: &mut App, mode: Mode) {
    let _ = mode;
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
    // Both dimensions' surfaces live side by side for live cross-dimension
    // switches; each finds only its own bodies in the wrong dimension.
    flat::register(app);
    surface::register(app);
    under::register(app);
}

/// Every body as of the last fixed tick, as the reporters read it.
#[derive(Resource, Clone, Debug, Default)]
pub struct WaterSample(pub WaterSense);

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
    /// Each body's sea state against its authored height.
    sea: HashMap<String, f32>,
    swells: HashMap<String, Swell>,
    fields: HashMap<String, RippleSim>,
    /// Whether each moving actor was in the water last tick, for splashes.
    wet: HashMap<Entity, bool>,
    /// Counts samples, so a surface knows when its ripples moved.
    pub tick: u64,
}

/// What a body's waves were built from, heading aside: a new one of these
/// rebuilds them on the spot.
#[derive(Clone, Debug, PartialEq)]
struct SetKey {
    waves: Waves,
    detail_scale: f32,
    gravity: f32,
    flat: bool,
}

/// A body's waves, and the ones it is turning away from.
struct Swell {
    key: SetKey,
    /// Where the wind has been blowing lately, eased.
    heading: f32,
    now: WaveSet,
    was: Option<WaveSet>,
    /// 0-1 through the turn from `was` to `now`.
    turned: f32,
}

impl Swell {
    fn build(key: &SetKey, spec: &WaterSpec, heading: f32) -> WaveSet {
        let mut spec = spec.clone();
        spec.waves.follow_wind = false;
        spec.waves.direction = heading;
        let set = WaveSet::build(&spec, heading, key.gravity);
        if key.flat { set.flattened() } else { set }
    }

    fn new(key: SetKey, spec: &WaterSpec, heading: f32) -> Self {
        Swell {
            now: Self::build(&key, spec, heading),
            key,
            heading,
            was: None,
            turned: 1.0,
        }
    }

    /// Eases towards `target` and starts a turn once the waves are far
    /// enough off it. A 2D swell only turns when it changes side.
    fn follow(&mut self, spec: &WaterSpec, target: f32, dt: f32) {
        let ease = 1.0 - (-dt / SEA_SETTLE).exp();
        self.heading = (self.heading + turn_between(self.heading, target) * ease).rem_euclid(360.0);
        if self.was.is_some() {
            self.turned += dt / spec.waves.turn.max(1e-3);
            if self.turned >= 1.0 {
                self.was = None;
            }
            return;
        }
        let off = if self.key.flat {
            let side = |h: f32| h.to_radians().sin() >= 0.0;
            if side(self.heading) != side(self.now.heading) {
                180.0
            } else {
                0.0
            }
        } else {
            turn_between(self.now.heading, self.heading).abs()
        };
        if off > TURN_THRESHOLD {
            let next = Self::build(&self.key, spec, self.heading);
            self.was = Some(std::mem::replace(&mut self.now, next));
            self.turned = 0.0;
        }
    }

    fn resolve(
        &self,
        spec: &WaterSpec,
        sea: f32,
        chop: f32,
        wind: f32,
        gravity: f32,
    ) -> (Vec<LiveWave>, Vec<LiveDetail>) {
        let waves = |set: &WaveSet| set.resolve(sea, chop, spec.waves.steepness);
        let detail = |set: &WaveSet| set.resolve_detail(spec.detail.strength, chop, wind, gravity);
        match &self.was {
            Some(was) => (
                blend_waves(waves(&self.now), waves(was), self.turned),
                blend_detail(detail(&self.now), detail(was), self.turned),
            ),
            None => (waves(&self.now), detail(&self.now)),
        }
    }
}

impl WaterState {
    /// Sets `body`'s ripples going at (x, z): a bump `radius` across, pushed
    /// `amount` world units. See [`RippleSim::disturb`].
    pub fn disturb(&mut self, body: &str, at: [f32; 2], radius: f32, amount: f32, moving: bool) {
        if let Some(sim) = self.fields.get_mut(body)
            && sim.covers(at[0], at[1])
        {
            sim.disturb(at[0], at[1], radius, amount, moving);
        }
    }

    pub fn body(&self, id: &str) -> Option<&LiveBody> {
        self.bodies.iter().find(|live| live.body.id == id)
    }
}

/// The ripple patch a body wants: its cells, the world units each spans,
/// and where it centres. A body that fits is covered whole; anything bigger
/// gets a patch following `eye`.
fn field_for(body: &WaterBody, spec: &WaterSpec, eye: Vec3) -> ([usize; 2], f32, [f32; 2]) {
    let extent = spec.ripples.extent;
    if body.flat {
        let n = RIPPLE_CELLS_FLAT;
        let span = 2.0 * body.half[0];
        return if body.kind != WaterKind::Ocean && span <= extent {
            ([n, 1], span / (n - 2) as f32, [body.center[0], 0.0])
        } else {
            ([n, 1], extent / n as f32, [eye.x, 0.0])
        };
    }
    let n = RIPPLE_CELLS;
    // The turned rectangle's world-aligned box.
    let [ax, az] = body.axis;
    let [hx, hz] = body.half;
    let reach = (hx * ax.abs() + hz * az.abs()).max(hx * az.abs() + hz * ax.abs());
    if body.kind != WaterKind::Ocean && 2.0 * reach <= extent {
        (
            [n, n],
            2.0 * reach / (n - 2) as f32,
            [body.center[0], body.center[2]],
        )
    } else {
        ([n, n], extent / n as f32, [eye.x, eye.z])
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

/// A ripple field as the shaders read it: height and the last tick's
/// height per texel, two floats each.
pub(crate) fn ripple_image(field: &RippleField) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let data = field
        .height
        .iter()
        .zip(&field.before)
        .flat_map(|(h, b)| [h.to_le_bytes(), b.to_le_bytes()])
        .flatten()
        .collect();
    Image::new(
        Extent3d {
            width: field.cells[0] as u32,
            height: field.cells[1] as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rg32Float,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

/// Keeps a body's ripple image in step with its field: made the first
/// time, written in place after. `None` when the body has no ripples.
pub(crate) fn sync_ripple_image(
    slot: &mut Option<(u64, Handle<Image>)>,
    live: &LiveBody,
    tick: u64,
    images: &mut Assets<Image>,
) -> Option<Handle<Image>> {
    let Some(field) = live.body.ripples.as_deref() else {
        *slot = None;
        return None;
    };
    let size = UVec2::new(field.cells[0] as u32, field.cells[1] as u32);
    match slot {
        Some((seen, handle)) if images.get(&*handle).is_some_and(|i| i.size() == size) => {
            if *seen != tick
                && let Some(mut image) = images.get_mut(&*handle)
            {
                *image = ripple_image(field);
                *seen = tick;
            }
        }
        _ => *slot = Some((tick, images.add(ripple_image(field)))),
    }
    slot.as_ref().map(|(_, handle)| handle.clone())
}

/// How far between its last two ticks the ripples draw.
pub(crate) fn between_ticks(engine: &Engine, fixed: &Time<Fixed>) -> f32 {
    if engine.running && !engine.paused {
        fixed.overstep_fraction().clamp(0.0, 1.0)
    } else {
        1.0
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
    cameras: Query<&GlobalTransform, With<WorldCamera>>,
    mut state: ResMut<WaterState>,
    mut sample: ResMut<WaterSample>,
) {
    if engine.running && engine.paused {
        return;
    }
    let flat = dimension.0 == Mode::TwoD;
    let dt = time.delta_secs();
    let state = &mut *state;
    state.tick += 1;
    if engine.running {
        state.time += dt;
    } else {
        state.time = 0.0;
        state.wet.clear();
    }
    let eye = cameras
        .iter()
        .next()
        .map_or(Vec3::ZERO, GlobalTransform::translation);
    let gravity = gravity_of(&engine, flat);
    let authored = engine.project.world.wind.direction;
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
            gravity,
            flat,
        };
        let blowing = wind.as_deref().map_or(Vec3::ZERO, |field| field.at(center));
        let air = blowing.length();
        let heading = if !spec.waves.follow_wind {
            spec.waves.direction
        } else if engine.running {
            heading_of(blowing.to_array(), flat).unwrap_or(authored)
        } else {
            authored
        };
        // The fetch fit is in metres: a 2D pixel is a hundredth of one.
        let (air_m, gravity_m) = if flat {
            (
                air / blockloom_core::scene::PIXELS_PER_METRE,
                gravity / blockloom_core::scene::PIXELS_PER_METRE,
            )
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
        let swell = state
            .swells
            .entry(id.0.clone())
            .or_insert_with(|| Swell::new(key.clone(), spec, heading));
        // An edit, or a stopped world, rebuilds on the spot.
        if swell.key != key || (!engine.running && swell.now.heading != heading) {
            *swell = Swell::new(key, spec, heading);
        } else if engine.running {
            swell.follow(spec, heading, dt);
        }
        let (waves, detail) = swell.resolve(spec, sea, chop, air, gravity);
        let mut body = WaterBody {
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
            calm: if flat {
                [0.0; 3]
            } else {
                [eye.x, eye.z, CALM_FROM * spec.waves.wavelength]
            },
            ripples: None,
        };
        if spec.ripples.enabled {
            let (cells, cell, centre) = field_for(&body, spec, eye);
            let sim = state
                .fields
                .entry(id.0.clone())
                .or_insert_with(|| RippleSim::new(cells, cell, centre));
            if sim.field.cells != cells || sim.field.cell != cell {
                *sim = RippleSim::new(cells, cell, centre);
            }
            sim.recentre(centre);
            if engine.running {
                sim.step(dt, spec.ripples.speed, spec.ripples.fade);
            } else {
                sim.clear();
            }
            body.ripples = Some(Arc::new(sim.field.clone()));
        } else {
            state.fields.remove(&id.0);
        }
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
        .swells
        .retain(|id, _| bodies.iter().any(|live| live.body.id == *id));
    state
        .fields
        .retain(|id, _| bodies.iter().any(|live| live.body.id == *id));
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
    fn a_splash_ripples_the_surface_buoyancy_reads() {
        let mut app = app(Mode::ThreeD);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.update();
        let calm = app.world().resource::<WaterSample>().0.clone();
        app.world_mut()
            .resource_mut::<WaterState>()
            .disturb("pool", [1.0, 1.0], 1.0, -0.3, false);
        app.update();
        let sense = app.world().resource::<WaterSample>().0.clone();
        let field = sense.bodies[0].ripples.as_ref().expect("a ripple field");
        assert_eq!(field.cells, [RIPPLE_CELLS, RIPPLE_CELLS]);
        let dip =
            sense.height_at(1.0, 1.0).unwrap() - calm.bodies[0].height_at(1.0, 1.0, sense.time);
        assert!(dip < -0.1, "{dip}");
        // Stopping puts the water back.
        app.world_mut().non_send_mut::<Engine>().running = false;
        app.update();
        let still = app.world().resource::<WaterSample>().0.clone();
        assert!(
            still.bodies[0]
                .ripples
                .as_ref()
                .unwrap()
                .height
                .iter()
                .all(|h| *h == 0.0)
        );
    }

    #[test]
    fn the_swell_turns_with_the_wind_over_its_turn_time() {
        let mut app = app(Mode::ThreeD);
        app.world_mut().non_send_mut::<Engine>().running = true;
        let mut wind = WindField::default();
        wind.dials.speed = 8.0;
        wind.dials.direction = 0.0;
        app.insert_resource(wind);
        app.update();
        let first = app.world().resource::<WaterSample>().0.bodies[0]
            .waves
            .clone();
        // The wind swings right round; the waves follow over a few seconds.
        app.world_mut().resource_mut::<WindField>().dials.direction = 180.0;
        let turning = |app: &App| {
            app.world()
                .resource::<WaterState>()
                .swells
                .values()
                .next()
                .is_some_and(|swell| swell.was.is_some())
        };
        let mut started = false;
        for _ in 0..600 {
            app.world_mut()
                .resource_mut::<Time<Fixed>>()
                .advance_by(std::time::Duration::from_secs_f32(1.0 / 60.0));
            app.update();
            started |= turning(&app);
        }
        assert!(started);
        let last = app.world().resource::<WaterSample>().0.bodies[0]
            .waves
            .clone();
        assert_ne!(first[0].dir, last[0].dir);
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
