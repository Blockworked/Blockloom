//! The CharacterController: a capsule moved by code against the world, with
//! Unity's low-level contract (`Move`, `SimpleMove`, collision flags,
//! `isGrounded`, controller hits).
//!
//! Core owns the document, the rules that do not need a backend and the shape
//! of a move's answer. The runtime implements [`ControllerService`] over the
//! same Rapier world and layer rules the simulation uses and installs it for a
//! fixed step ([`with_service`]); every adapter (blocks, reporters, scripts,
//! compiled logic) moves a controller through [`move_call`]. Stated semantics:
//!
//! - `Move(displacement)` takes world units and adds no gravity. `SimpleMove`
//!   takes a speed in world units a second, ignores its component along `up`
//!   and adds gravity: the fall speed grows by the world gravity along `up`
//!   while the controller is airborne and is zeroed by a floor or a ceiling.
//! - Moves run on the spot, in submission order, each from where the previous
//!   one ended. The world they sweep is the one the previous fixed step left;
//!   the actor's pose is written once, at the end of the step
//!   ([`take_pending`]), by the sum of the effective displacements.
//! - A move shorter than `min_move_distance` does not move. It still probes
//!   the ground and recovers overlap, which Unity does not: a controller that
//!   stands still keeps a fresh `grounded`.
//! - A new controller's ground state is unknown until its first move or probe.
//! - Collision flags come from the hits of the move: a surface whose normal is
//!   within the slope limit of `up` is `Below`, within it of `-up` is `Above`,
//!   anything else (walls, unwalkable slopes) is `Sides`.
//! - `Detect Collisions` off removes the capsule from the world (nothing
//!   collides with the controller); the controller still sweeps against it.
//! - Triggers are never obstacles. A hit is recorded per obstacle contact of
//!   an actual sweep, not for persistent nearby overlap.

use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use super::spec::LayerOverrides;
use super::{ColliderId, ComponentId};
use crate::scene::Mode;

/// The most hits a move keeps; more set `overflow`.
pub const MAX_HITS: usize = 16;

fn yes() -> bool {
    true
}

fn default_layer() -> u8 {
    1
}

fn up_axis() -> [f32; 3] {
    [0.0, 1.0, 0.0]
}

/// Unity's defaults for a 3D controller, in metres.
pub const DEFAULT_RADIUS: f32 = 0.5;
pub const DEFAULT_HEIGHT: f32 = 2.0;
pub const DEFAULT_SLOPE_LIMIT: f32 = 45.0;
pub const DEFAULT_STEP_OFFSET: f32 = 0.3;
pub const DEFAULT_SKIN_WIDTH: f32 = 0.08;
pub const DEFAULT_MIN_MOVE: f32 = 0.001;

/// A CharacterController component. Sizes are world units: metres in 3D,
/// pixels in 2D (see [`CharacterControllerSpec::for_mode`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterControllerSpec {
    #[serde(default)]
    pub id: ComponentId,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Where the capsule's centre stands, from the actor's origin.
    #[serde(default)]
    pub center: [f32; 3],
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// Total height, caps included. At least twice the radius.
    #[serde(default = "default_height")]
    pub height: f32,
    /// Steepest walkable slope, degrees from flat.
    #[serde(default = "default_slope")]
    pub slope_limit: f32,
    /// Tallest step it walks onto.
    #[serde(default = "default_step")]
    pub step_offset: f32,
    /// The gap kept between the capsule and what it touches.
    #[serde(default = "default_skin")]
    pub skin_width: f32,
    /// A move shorter than this does not move.
    #[serde(default = "default_min_move")]
    pub min_move_distance: f32,
    /// Whether other bodies and controllers collide with this capsule.
    #[serde(default = "yes")]
    pub detect_collisions: bool,
    /// Push the capsule out of whatever it overlaps before it moves.
    #[serde(default = "yes")]
    pub overlap_recovery: bool,
    /// Layer slot, 1 to 32, that the capsule lives on and sweeps as.
    #[serde(default = "default_layer")]
    pub layer: u8,
    #[serde(default, skip_serializing_if = "LayerOverrides::is_empty")]
    pub layer_overrides: LayerOverrides,
    /// The direction that is up (3D profile: +Y). In 2D the z is ignored.
    #[serde(default = "up_axis")]
    pub up: [f32; 3],
}

fn default_radius() -> f32 {
    DEFAULT_RADIUS
}
fn default_height() -> f32 {
    DEFAULT_HEIGHT
}
fn default_slope() -> f32 {
    DEFAULT_SLOPE_LIMIT
}
fn default_step() -> f32 {
    DEFAULT_STEP_OFFSET
}
fn default_skin() -> f32 {
    DEFAULT_SKIN_WIDTH
}
fn default_min_move() -> f32 {
    DEFAULT_MIN_MOVE
}

impl Default for CharacterControllerSpec {
    fn default() -> Self {
        Self::for_mode(Mode::ThreeD)
    }
}

impl CharacterControllerSpec {
    /// A controller with Unity's numbers in 3D and a 32 by 64 pixel one in 2D.
    pub fn for_mode(mode: Mode) -> Self {
        let (radius, height, step, skin, min_move) = match mode {
            Mode::ThreeD => (
                DEFAULT_RADIUS,
                DEFAULT_HEIGHT,
                DEFAULT_STEP_OFFSET,
                DEFAULT_SKIN_WIDTH,
                DEFAULT_MIN_MOVE,
            ),
            Mode::TwoD => (16.0, 64.0, 12.0, 2.0, 0.05),
        };
        Self {
            id: ComponentId::generate(),
            enabled: true,
            center: [0.0; 3],
            radius,
            height,
            slope_limit: DEFAULT_SLOPE_LIMIT,
            step_offset: step,
            skin_width: skin,
            min_move_distance: min_move,
            detect_collisions: true,
            overlap_recovery: true,
            layer: 1,
            layer_overrides: LayerOverrides::default(),
            up: up_axis(),
        }
    }

    /// Half the straight part of the capsule: the distance from its centre to
    /// the centre of either cap.
    pub fn half_segment(&self) -> f32 {
        ((self.height - 2.0 * self.radius) * 0.5).max(0.0)
    }

    /// The unit up vector (3D: +Y unless the spec turns it).
    pub fn up_unit(&self, mode: Mode) -> [f32; 3] {
        let mut v = self.up;
        if mode == Mode::TwoD {
            v[2] = 0.0;
        }
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if length > 1e-6 && length.is_finite() {
            [v[0] / length, v[1] / length, v[2] / length]
        } else {
            up_axis()
        }
    }

    /// What is wrong with the numbers, by field.
    pub fn validate(&self, mode: Mode) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut bad = |field: &str, message: String| out.push((field.to_string(), message));
        let finite = |v: f32| v.is_finite();
        if !finite(self.radius) || self.radius <= 0.0 {
            bad("radius", "The radius must be above zero".into());
        }
        if !finite(self.height) || self.height <= 0.0 {
            bad("height", "The height must be above zero".into());
        } else if finite(self.radius) && self.height < 2.0 * self.radius {
            bad(
                "height",
                format!(
                    "The height ({}) must be at least twice the radius ({})",
                    self.height, self.radius
                ),
            );
        }
        if !finite(self.slope_limit) || !(0.0..=90.0).contains(&self.slope_limit) {
            bad(
                "slope_limit",
                "The slope limit must be 0 to 90 degrees".into(),
            );
        }
        if !finite(self.step_offset) || self.step_offset < 0.0 {
            bad("step_offset", "The step offset cannot be negative".into());
        } else if finite(self.height) && self.step_offset > self.height {
            bad(
                "step_offset",
                format!(
                    "The step offset ({}) is taller than the controller ({})",
                    self.step_offset, self.height
                ),
            );
        }
        if !finite(self.skin_width) || self.skin_width <= 0.0 {
            bad("skin_width", "The skin width must be above zero".into());
        } else if finite(self.radius) && self.skin_width > self.radius {
            bad(
                "skin_width",
                "The skin width cannot exceed the radius".into(),
            );
        }
        if !finite(self.min_move_distance) || self.min_move_distance < 0.0 {
            bad(
                "min_move_distance",
                "The minimum move cannot be negative".into(),
            );
        }
        if self.center.iter().any(|c| !c.is_finite()) {
            bad("center", "The centre must be finite".into());
        }
        if !(1..=super::spec::LAYER_SLOTS).contains(&self.layer) {
            bad("layer", "The layer must be 1 to 32".into());
        }
        if self.up.iter().any(|c| !c.is_finite())
            || self.up[..if mode == Mode::TwoD { 2 } else { 3 }]
                .iter()
                .all(|c| c.abs() < 1e-6)
        {
            bad("up", "The up direction must point somewhere".into());
        }
        out
    }
}

/// Which move a call asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MoveMode {
    /// A displacement in world units, no gravity.
    #[default]
    Move,
    /// A speed in world units a second, gravity applied, up ignored.
    Simple,
}

impl MoveMode {
    pub fn parse(word: &str) -> Self {
        match word.trim().to_ascii_lowercase().as_str() {
            "simple" | "simplemove" | "simple move" | "speed" | "at speed" => Self::Simple,
            _ => Self::Move,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            MoveMode::Move => "move",
            MoveMode::Simple => "simple move",
        }
    }
}

/// Unity's `CollisionFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct CollisionFlags {
    pub sides: bool,
    pub above: bool,
    pub below: bool,
}

impl CollisionFlags {
    /// Unity's bit values: none 0, sides 1, above 2, below 4.
    pub fn bits(self) -> u32 {
        u32::from(self.sides) | u32::from(self.above) << 1 | u32::from(self.below) << 2
    }

    /// Where a surface with this normal counts.
    pub fn classify(normal: [f32; 3], up: [f32; 3], slope_limit_degrees: f32) -> Surface {
        let along = normal[0] * up[0] + normal[1] * up[1] + normal[2] * up[2];
        let walkable = slope_limit_degrees.to_radians().cos();
        // A flat floor reads 1, a ceiling -1; the limit is how far from either
        // still counts as one.
        if along >= walkable - 1e-4 && along > 0.0 {
            Surface::Floor
        } else if -along >= walkable - 1e-4 && along < 0.0 {
            Surface::Ceiling
        } else {
            Surface::Wall
        }
    }

    pub fn add(&mut self, surface: Surface) {
        match surface {
            Surface::Floor => self.below = true,
            Surface::Ceiling => self.above = true,
            Surface::Wall => self.sides = true,
        }
    }
}

/// What a contact normal says about the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Floor,
    Ceiling,
    Wall,
}

/// One obstacle a move met.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ControllerHit {
    pub actor: String,
    pub body: Option<String>,
    pub collider: ColliderId,
    pub point: [f32; 3],
    /// The surface's normal, pointing at the controller.
    pub normal: [f32; 3],
    /// Which way the move was heading when it met this (unit), and how long
    /// the whole move asked to be.
    pub direction: [f32; 3],
    pub move_length: f32,
    /// How far the controller had already travelled when it met this.
    pub applied: [f32; 3],
    pub tick: u64,
}

/// What a move did.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct MoveResult {
    pub tick: u64,
    pub mode: MoveMode,
    /// The displacement the move set out to make (SimpleMove's included).
    pub requested: [f32; 3],
    pub effective: [f32; 3],
    /// What overlap recovery pushed the capsule by before the move.
    pub recovered: [f32; 3],
    pub flags: CollisionFlags,
    pub grounded: bool,
    /// Whether it walked onto a step.
    pub stepped: bool,
    /// Whether it was too short to move (below `min_move_distance`).
    pub skipped: bool,
    pub hits: Vec<ControllerHit>,
    pub overflow: bool,
    /// World units a second: the effective displacement over the step.
    pub velocity: [f32; 3],
    pub error: Option<String>,
}

/// Everything the service needs for one sweep.
#[derive(Debug, Clone)]
pub struct StepRequest<'a> {
    pub actor: &'a str,
    pub spec: &'a CharacterControllerSpec,
    /// What to move by, world units.
    pub displacement: [f32; 3],
    /// How far earlier moves this step already took the actor, which its pose
    /// has not yet had applied.
    pub pending: [f32; 3],
    pub dt: f32,
}

/// An obstacle the sweep met, before core classifies it.
#[derive(Debug, Clone, PartialEq)]
pub struct RawHit {
    pub actor: String,
    pub body: Option<String>,
    pub collider: ColliderId,
    pub point: [f32; 3],
    pub normal: [f32; 3],
    pub applied: [f32; 3],
}

/// What the backend found.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StepOutcome {
    pub effective: [f32; 3],
    pub recovered: [f32; 3],
    pub grounded: bool,
    pub stepped: bool,
    pub hits: Vec<RawHit>,
}

/// Moves a controller's capsule through the simulation's world.
pub trait ControllerService {
    fn step(&self, request: &StepRequest) -> Result<StepOutcome, String>;
    /// The world's gravity as a vector, world units a second squared.
    fn gravity(&self) -> [f32; 3];
    /// The fixed step, in seconds.
    fn timestep(&self) -> f32;
    fn mode(&self) -> Mode;
}

/// What core remembers of one controller between moves.
#[derive(Debug, Clone)]
struct Tracked {
    spec: CharacterControllerSpec,
    /// Downward speed SimpleMove has built up, along `-up`, world units a second.
    fall_speed: f32,
    /// `None` until the first move or probe.
    grounded: Option<bool>,
    pending: [f32; 3],
    last: Option<MoveResult>,
    /// The capsule changed shape and the world's copy should follow.
    reshaped: bool,
}

#[derive(Default)]
struct Registry {
    controllers: HashMap<String, Tracked>,
    /// Hits since the engine last collected them, to raise as events.
    outbox: Vec<(String, ControllerHit)>,
    moved: Vec<String>,
}

thread_local! {
    static SERVICE: Cell<Option<(*const dyn ControllerService, u64)>> = const { Cell::new(None) };
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Runs `f` with `service` answering every move on this thread, as the world
/// stood at `tick`. Nested scopes restore the outer one.
pub fn with_service<R>(service: &dyn ControllerService, tick: u64, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<(*const dyn ControllerService, u64)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SERVICE.with(|slot| slot.set(self.0));
        }
    }
    // SAFETY: the pointer is only read while `f` runs, `Restore` puts the
    // previous value back even on unwind, and the service outlives this call.
    let erased: *const dyn ControllerService = unsafe {
        std::mem::transmute::<&dyn ControllerService, &'static dyn ControllerService>(service)
    };
    let _restore = Restore(SERVICE.with(|slot| slot.replace(Some((erased, tick)))));
    f()
}

/// Forgets every controller: a world is being rebuilt or a run ended.
pub fn reset() {
    REGISTRY.with(|registry| *registry.borrow_mut() = Registry::default());
}

/// Starts tracking `actor`'s controller with the spec the document says.
pub fn register(actor: &str, spec: CharacterControllerSpec) {
    REGISTRY.with(|registry| {
        registry.borrow_mut().controllers.insert(
            actor.to_string(),
            Tracked {
                spec,
                fall_speed: 0.0,
                grounded: None,
                pending: [0.0; 3],
                last: None,
                reshaped: false,
            },
        );
    });
}

/// Stops tracking `actor` (deleted, or its component detached).
pub fn unregister(actor: &str) {
    REGISTRY.with(|registry| {
        registry.borrow_mut().controllers.remove(actor);
    });
}

/// Whether `actor` has a controller.
pub fn has(actor: &str) -> bool {
    REGISTRY.with(|registry| registry.borrow().controllers.contains_key(actor))
}

/// The controller's spec as it stands, with any change a run made.
pub fn spec_of(actor: &str) -> Option<CharacterControllerSpec> {
    REGISTRY.with(|registry| {
        registry
            .borrow()
            .controllers
            .get(actor)
            .map(|t| t.spec.clone())
    })
}

/// The properties a block or script can change during a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControllerProperty {
    Enabled,
    Radius,
    /// Keeps the feet where they are.
    Height,
    SlopeLimit,
    StepOffset,
    SkinWidth,
    MinMoveDistance,
    DetectCollisions,
    OverlapRecovery,
}

impl ControllerProperty {
    pub const ALL: [ControllerProperty; 9] = [
        Self::Enabled,
        Self::Radius,
        Self::Height,
        Self::SlopeLimit,
        Self::StepOffset,
        Self::SkinWidth,
        Self::MinMoveDistance,
        Self::DetectCollisions,
        Self::OverlapRecovery,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Radius => "radius",
            Self::Height => "height",
            Self::SlopeLimit => "slope limit",
            Self::StepOffset => "step offset",
            Self::SkinWidth => "skin width",
            Self::MinMoveDistance => "minimum move",
            Self::DetectCollisions => "detect collisions",
            Self::OverlapRecovery => "overlap recovery",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        let word = word.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| p.name() == word)
    }
}

/// Changes one property of `actor`'s controller for the rest of the run.
/// The new spec must still validate; `height` moves the centre so the feet
/// stay put. The error says why a change was refused.
pub fn set_property(
    actor: &str,
    property: ControllerProperty,
    value: f64,
    mode: Mode,
) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("the {} must be a finite number", property.name()));
    }
    let value = value as f32;
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let Some(tracked) = registry.controllers.get_mut(actor) else {
            return Err("this actor has no CharacterController".to_string());
        };
        let mut next = tracked.spec.clone();
        match property {
            ControllerProperty::Enabled => next.enabled = value != 0.0,
            ControllerProperty::Radius => next.radius = value,
            ControllerProperty::Height => {
                let up = next.up_unit(mode);
                let grow = (value - next.height) * 0.5;
                for (c, u) in next.center.iter_mut().zip(up) {
                    *c += grow * u;
                }
                next.height = value;
            }
            ControllerProperty::SlopeLimit => next.slope_limit = value,
            ControllerProperty::StepOffset => next.step_offset = value,
            ControllerProperty::SkinWidth => next.skin_width = value,
            ControllerProperty::MinMoveDistance => next.min_move_distance = value,
            ControllerProperty::DetectCollisions => next.detect_collisions = value != 0.0,
            ControllerProperty::OverlapRecovery => next.overlap_recovery = value != 0.0,
        }
        if let Some((_, why)) = next.validate(mode).into_iter().next() {
            return Err(why);
        }
        tracked.reshaped |= next.radius != tracked.spec.radius
            || next.height != tracked.spec.height
            || next.center != tracked.spec.center
            || next.enabled != tracked.spec.enabled
            || next.detect_collisions != tracked.spec.detect_collisions;
        tracked.spec = next;
        Ok(())
    })
}

/// Controllers whose capsule changed since this was last called, with the
/// spec the world's copy should take.
pub fn take_reshaped() -> Vec<(String, CharacterControllerSpec)> {
    REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .controllers
            .iter_mut()
            .filter(|(_, t)| t.reshaped)
            .map(|(actor, t)| {
                t.reshaped = false;
                (actor.clone(), t.spec.clone())
            })
            .collect()
    })
}

/// The displacement each controller moved this step, which the world applies
/// to its pose. Clears it.
pub fn take_pending() -> Vec<(String, [f32; 3])> {
    REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .controllers
            .iter_mut()
            .filter(|(_, t)| t.pending != [0.0; 3])
            .map(|(actor, t)| (actor.clone(), std::mem::take(&mut t.pending)))
            .collect()
    })
}

/// Hits recorded since the last call, by the actor that made them.
pub fn take_hits() -> Vec<(String, ControllerHit)> {
    REGISTRY.with(|registry| std::mem::take(&mut registry.borrow_mut().outbox))
}

/// What the installed service says about the world: gravity, the fixed step,
/// the dimension and the tick. `None` outside a run.
pub fn service_info() -> Option<([f32; 3], f32, Mode, u64)> {
    let (service, tick) = SERVICE.with(|slot| slot.get())?;
    // SAFETY: see `with_service`; the scope that set it is still running.
    let service: &dyn ControllerService = unsafe { &*service };
    Some((service.gravity(), service.timestep(), service.mode(), tick))
}

/// How far a move would get, without making it: the sweep of `vector` from
/// where the capsule stands now. Used for headroom before standing up.
pub fn probe_move(actor: &str, vector: [f32; 3]) -> Option<[f32; 3]> {
    let (service, _) = SERVICE.with(|slot| slot.get())?;
    // SAFETY: see `with_service`; the scope that set it is still running.
    let service: &dyn ControllerService = unsafe { &*service };
    let (spec, pending) = REGISTRY.with(|registry| {
        let registry = registry.borrow();
        let tracked = registry.controllers.get(actor)?;
        Some((tracked.spec.clone(), tracked.pending))
    })?;
    let request = StepRequest {
        actor,
        spec: &spec,
        displacement: vector,
        pending,
        dt: service.timestep(),
    };
    service.step(&request).ok().map(|outcome| outcome.effective)
}

/// The actors whose controllers made a move since the last call.
pub fn take_moved() -> Vec<String> {
    REGISTRY.with(|registry| std::mem::take(&mut registry.borrow_mut().moved))
}

/// Moves `actor`'s controller. The answer is also filed for the reporters.
/// Without a service (no world yet) or a controller, the result carries why.
pub fn move_call(actor: &str, mode: MoveMode, vector: [f32; 3]) -> MoveResult {
    let (service, tick) = match SERVICE.with(|slot| slot.get()) {
        Some(found) => found,
        None => {
            return failed(
                actor,
                mode,
                0,
                "a controller only moves while the game is running",
            );
        }
    };
    // SAFETY: see `with_service`; the scope that set it is still running.
    let service: &dyn ControllerService = unsafe { &*service };
    if vector.iter().any(|v| !v.is_finite()) {
        return failed(actor, mode, tick, "a move needs finite numbers");
    }
    let Some((spec, fall_speed, was_grounded, pending)) = REGISTRY.with(|registry| {
        registry
            .borrow()
            .controllers
            .get(actor)
            .map(|t| (t.spec.clone(), t.fall_speed, t.grounded, t.pending))
    }) else {
        return failed(actor, mode, tick, "this actor has no CharacterController");
    };
    let dt = service.timestep();
    let world_mode = service.mode();
    let up = spec.up_unit(world_mode);

    // What to set out to do.
    let (requested, mut fall) = match mode {
        MoveMode::Move => (vector, fall_speed),
        MoveMode::Simple => {
            let gravity = service.gravity();
            let g_up = gravity[0] * up[0] + gravity[1] * up[1] + gravity[2] * up[2];
            // Fall speed is positive downward; a floor under it holds it at
            // zero, gravity grows it, and zero gravity adds nothing.
            let mut fall = fall_speed;
            if was_grounded == Some(true) && fall > 0.0 {
                fall = 0.0;
            }
            if was_grounded != Some(true) {
                fall -= g_up * dt;
            }
            let along = vector[0] * up[0] + vector[1] * up[1] + vector[2] * up[2];
            let mut d = [0.0; 3];
            for i in 0..3 {
                d[i] = (vector[i] - along * up[i]) * dt - up[i] * fall * dt;
            }
            (d, fall)
        }
    };

    let mut result = MoveResult {
        tick,
        mode,
        requested,
        ..MoveResult::default()
    };
    if !spec.enabled {
        result.skipped = true;
        result.grounded = was_grounded.unwrap_or(false);
        return file(actor, result, None, fall_speed);
    }
    let length =
        (requested[0] * requested[0] + requested[1] * requested[1] + requested[2] * requested[2])
            .sqrt();
    let skipped = length < spec.min_move_distance;
    let displacement = if skipped { [0.0; 3] } else { requested };
    let request = StepRequest {
        actor,
        spec: &spec,
        displacement,
        pending,
        dt,
    };
    let outcome = match service.step(&request) {
        Ok(outcome) => outcome,
        Err(why) => {
            result.error = Some(why);
            return file(actor, result, None, fall_speed);
        }
    };
    result.skipped = skipped;
    result.effective = outcome.effective;
    result.recovered = outcome.recovered;
    result.stepped = outcome.stepped;
    let direction = if length > 1e-9 {
        [
            requested[0] / length,
            requested[1] / length,
            requested[2] / length,
        ]
    } else {
        [0.0; 3]
    };
    for raw in outcome.hits {
        result
            .flags
            .add(CollisionFlags::classify(raw.normal, up, spec.slope_limit));
        if result.hits.len() >= MAX_HITS {
            result.overflow = true;
            continue;
        }
        result.hits.push(ControllerHit {
            actor: raw.actor,
            body: raw.body,
            collider: raw.collider,
            point: raw.point,
            normal: raw.normal,
            direction,
            move_length: length,
            applied: raw.applied,
            tick,
        });
    }
    result.grounded = outcome.grounded || result.flags.below;
    if dt > 0.0 {
        result.velocity = [
            result.effective[0] / dt,
            result.effective[1] / dt,
            result.effective[2] / dt,
        ];
    }
    // A floor holds the fall; a ceiling kills a rise.
    if result.grounded {
        fall = 0.0;
    } else if result.flags.above && fall < 0.0 {
        fall = 0.0;
    }
    file(actor, result, Some(outcome.effective), fall)
}

fn failed(actor: &str, mode: MoveMode, tick: u64, why: &str) -> MoveResult {
    let result = MoveResult {
        tick,
        mode,
        error: Some(why.to_string()),
        ..MoveResult::default()
    };
    REGISTRY.with(|registry| {
        if let Some(tracked) = registry.borrow_mut().controllers.get_mut(actor) {
            tracked.last = Some(result.clone());
        }
    });
    result
}

/// Files a finished move under its actor and queues its hits and motion.
fn file(actor: &str, result: MoveResult, moved: Option<[f32; 3]>, fall_speed: f32) -> MoveResult {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let hits = result.hits.clone();
        if let Some(tracked) = registry.controllers.get_mut(actor) {
            if let Some(moved) = moved {
                for i in 0..3 {
                    tracked.pending[i] += moved[i] + result.recovered[i];
                }
                tracked.grounded = Some(result.grounded);
            }
            tracked.fall_speed = fall_speed;
            tracked.last = Some(result.clone());
        }
        registry
            .outbox
            .extend(hits.into_iter().map(|hit| (actor.to_string(), hit)));
        registry.moved.push(actor.to_string());
    });
    result
}

/// One controller statement, the way every adapter (blocks, compiled logic,
/// scripts) states it: `op` is `move`, `simple move` or `set <property>`, and
/// `vector` is the move's numbers or, for a set, the value in `vector[0]`.
/// Answers the move's flags (see [`CollisionFlags::bits`], with 8 added when
/// grounded) or why it was refused.
pub fn run_op(actor: &str, op: &str, vector: [f32; 3]) -> Result<u32, String> {
    let op = op.trim();
    // The motor shares this channel so every adapter reaches it for free.
    if let Some(rest) = op.strip_prefix("motor ") {
        return super::motor::run_op(actor, rest, vector).map(|()| 0);
    }
    if let Some(rest) = op.strip_prefix("joint ") {
        return super::joints::run_op(actor, rest, vector[0]).map(|()| 0);
    }
    if let Some(name) = op.strip_prefix("set ") {
        let property = ControllerProperty::parse(name)
            .ok_or_else(|| format!("a controller has no setting called \"{name}\""))?;
        let mode = SERVICE
            .with(|slot| slot.get())
            // SAFETY: see `with_service`; the scope that set it is running.
            .map_or(Mode::ThreeD, |(service, _)| unsafe { (*service).mode() });
        set_property(actor, property, f64::from(vector[0]), mode)?;
        return Ok(0);
    }
    let result = move_call(actor, MoveMode::parse(op), vector);
    match result.error {
        Some(why) => Err(why),
        None => Ok(result.flags.bits() | u32::from(result.grounded) << 3),
    }
}

/// The actor's last move, as its reporters read it.
pub fn last_move(actor: &str) -> Option<MoveResult> {
    REGISTRY.with(|registry| registry.borrow().controllers.get(actor)?.last.clone())
}

/// Whether the controller stood on something after its last move; `None`
/// before the first move or probe.
pub fn grounded(actor: &str) -> Option<bool> {
    REGISTRY.with(|registry| registry.borrow().controllers.get(actor)?.grounded)
}

/// The fall speed SimpleMove has built up, downward positive.
pub fn fall_speed(actor: &str) -> f32 {
    REGISTRY.with(|registry| {
        registry
            .borrow()
            .controllers
            .get(actor)
            .map_or(0.0, |t| t.fall_speed)
    })
}

/// A number from `actor`'s last move, by the name a reporter gives it. A hit
/// field reads hit `n` (from 1); anything missing reads zero.
pub fn read_number(actor: &str, field: &str, n: usize) -> f64 {
    if let Some(rest) = field.trim().strip_prefix("motor ") {
        return super::motor::read_number(actor, rest);
    }
    if let Some(rest) = field.trim().strip_prefix("joint ") {
        let (field, name) = rest.split_once('|').unwrap_or((rest, ""));
        return super::joints::read_number(actor, name, field);
    }
    let Some(last) = last_move(actor) else {
        return match field.trim().to_ascii_lowercase().as_str() {
            "controller exists" | "exists" => f64::from(has(actor)),
            _ => 0.0,
        };
    };
    let hit = n.checked_sub(1).and_then(|i| last.hits.get(i));
    let axis = |v: [f32; 3], i: usize| f64::from(v[i]);
    match field.trim().to_ascii_lowercase().as_str() {
        "grounded" | "is grounded" => f64::from(grounded(actor).unwrap_or(false)),
        "ground known" => f64::from(grounded(actor).is_some()),
        "flags" => f64::from(last.flags.bits()),
        "sides" => f64::from(last.flags.sides),
        "above" => f64::from(last.flags.above),
        "below" => f64::from(last.flags.below),
        "velocity x" => axis(last.velocity, 0),
        "velocity y" => axis(last.velocity, 1),
        "velocity z" => axis(last.velocity, 2),
        "moved x" => axis(last.effective, 0),
        "moved y" => axis(last.effective, 1),
        "moved z" => axis(last.effective, 2),
        "asked x" => axis(last.requested, 0),
        "asked y" => axis(last.requested, 1),
        "asked z" => axis(last.requested, 2),
        "recovered" => {
            let r = last.recovered;
            f64::from((r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt())
        }
        "stepped" => f64::from(last.stepped),
        "skipped" => f64::from(last.skipped),
        "overflowed" => f64::from(last.overflow),
        "tick" => last.tick as f64,
        "fall speed" => f64::from(fall_speed(actor)),
        "hit count" | "count" => last.hits.len() as f64,
        "hit x" => hit.map_or(0.0, |h| axis(h.point, 0)),
        "hit y" => hit.map_or(0.0, |h| axis(h.point, 1)),
        "hit z" => hit.map_or(0.0, |h| axis(h.point, 2)),
        "normal x" => hit.map_or(0.0, |h| axis(h.normal, 0)),
        "normal y" => hit.map_or(0.0, |h| axis(h.normal, 1)),
        "normal z" => hit.map_or(0.0, |h| axis(h.normal, 2)),
        "hit length" => hit.map_or(0.0, |h| f64::from(h.move_length)),
        "radius" => spec_of(actor).map_or(0.0, |s| f64::from(s.radius)),
        "height" => spec_of(actor).map_or(0.0, |s| f64::from(s.height)),
        _ => 0.0,
    }
}

/// Words from the actor's last move: a hit's actor, body or collider, or the
/// error. Anything missing reads empty.
pub fn read_text(actor: &str, field: &str, n: usize) -> String {
    if let Some(rest) = field.trim().strip_prefix("motor ") {
        return super::motor::read_text(actor, rest);
    }
    let Some(last) = last_move(actor) else {
        return String::new();
    };
    let hit = n.checked_sub(1).and_then(|i| last.hits.get(i));
    match field.trim().to_ascii_lowercase().as_str() {
        "actor" | "actor id" => hit.map(|h| h.actor.clone()).unwrap_or_default(),
        "body" => hit.and_then(|h| h.body.clone()).unwrap_or_default(),
        "collider" => hit.map(|h| h.collider.to_string()).unwrap_or_default(),
        "error" => last.error.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world that is flat ground at height zero and nothing else.
    struct Flat;

    impl ControllerService for Flat {
        fn step(&self, r: &StepRequest) -> Result<StepOutcome, String> {
            // The centre stands `height / 2` over the ground.
            let foot = r.pending[1] + r.spec.center[1] - r.spec.height * 0.5;
            let mut effective = r.displacement;
            let mut hits = Vec::new();
            if foot + effective[1] < 0.0 {
                effective[1] = -foot;
                hits.push(RawHit {
                    actor: "ground".into(),
                    body: None,
                    collider: ColliderId::from("ground"),
                    point: [0.0; 3],
                    normal: [0.0, 1.0, 0.0],
                    applied: effective,
                });
            }
            Ok(StepOutcome {
                grounded: foot + effective[1] <= 1e-4,
                effective,
                hits,
                ..StepOutcome::default()
            })
        }

        fn gravity(&self) -> [f32; 3] {
            [0.0, -10.0, 0.0]
        }

        fn timestep(&self) -> f32 {
            0.1
        }

        fn mode(&self) -> Mode {
            Mode::ThreeD
        }
    }

    fn standing(at_height: f32) -> CharacterControllerSpec {
        let mut spec = CharacterControllerSpec::default();
        // The actor's origin is the centre, so the feet are a metre under it.
        spec.center = [0.0, at_height, 0.0];
        spec
    }

    fn run<R>(f: impl FnOnce() -> R) -> R {
        reset();
        with_service(&Flat, 7, f)
    }

    #[test]
    fn the_defaults_are_unitys_and_validate() {
        let spec = CharacterControllerSpec::default();
        assert_eq!(spec.radius, 0.5);
        assert_eq!(spec.height, 2.0);
        assert_eq!(spec.slope_limit, 45.0);
        assert_eq!(spec.step_offset, 0.3);
        assert_eq!(spec.skin_width, 0.08);
        assert!(spec.validate(Mode::ThreeD).is_empty());
        assert!(
            CharacterControllerSpec::for_mode(Mode::TwoD)
                .validate(Mode::TwoD)
                .is_empty()
        );
    }

    #[test]
    fn impossible_dimensions_are_refused() {
        let mut spec = CharacterControllerSpec::default();
        spec.radius = -1.0;
        assert!(
            spec.validate(Mode::ThreeD)
                .iter()
                .any(|(f, _)| f == "radius")
        );
        let mut spec = CharacterControllerSpec::default();
        spec.height = 0.8;
        assert!(
            spec.validate(Mode::ThreeD)
                .iter()
                .any(|(f, _)| f == "height")
        );
        let mut spec = CharacterControllerSpec::default();
        spec.step_offset = 5.0;
        assert!(
            spec.validate(Mode::ThreeD)
                .iter()
                .any(|(f, _)| f == "step_offset")
        );
        let mut spec = CharacterControllerSpec::default();
        spec.skin_width = 0.0;
        assert!(
            spec.validate(Mode::ThreeD)
                .iter()
                .any(|(f, _)| f == "skin_width")
        );
        let mut spec = CharacterControllerSpec::default();
        spec.slope_limit = 120.0;
        assert!(
            spec.validate(Mode::ThreeD)
                .iter()
                .any(|(f, _)| f == "slope_limit")
        );
        let mut spec = CharacterControllerSpec::default();
        spec.up = [0.0; 3];
        assert!(spec.validate(Mode::ThreeD).iter().any(|(f, _)| f == "up"));
    }

    #[test]
    fn surfaces_are_floors_ceilings_or_walls_by_the_slope_limit() {
        let up = [0.0, 1.0, 0.0];
        assert_eq!(
            CollisionFlags::classify([0.0, 1.0, 0.0], up, 45.0),
            Surface::Floor
        );
        // A 30 degree slope is walkable at a 45 degree limit; a 60 one is a wall.
        let slope = |deg: f32| [deg.to_radians().sin(), deg.to_radians().cos(), 0.0];
        assert_eq!(
            CollisionFlags::classify(slope(30.0), up, 45.0),
            Surface::Floor
        );
        assert_eq!(
            CollisionFlags::classify(slope(60.0), up, 45.0),
            Surface::Wall
        );
        assert_eq!(
            CollisionFlags::classify([1.0, 0.0, 0.0], up, 45.0),
            Surface::Wall
        );
        assert_eq!(
            CollisionFlags::classify([0.0, -1.0, 0.0], up, 45.0),
            Surface::Ceiling
        );
        let mut flags = CollisionFlags::default();
        flags.add(Surface::Floor);
        flags.add(Surface::Wall);
        assert_eq!(flags.bits(), 5);
    }

    #[test]
    fn a_move_adds_no_gravity_and_a_simple_move_does() {
        run(|| {
            register("a", standing(5.0));
            let moved = move_call("a", MoveMode::Move, [1.0, 0.0, 0.0]);
            assert_eq!(moved.effective, [1.0, 0.0, 0.0]);
            assert!(!moved.grounded);
            // A simple move ignores the y it was given and falls by gravity.
            let simple = move_call("a", MoveMode::Simple, [10.0, 99.0, 0.0]);
            assert!((simple.effective[0] - 1.0).abs() < 1e-5);
            assert!(simple.effective[1] < 0.0);
            let second = move_call("a", MoveMode::Simple, [0.0, 0.0, 0.0]);
            assert!(
                second.effective[1] < simple.effective[1],
                "the fall speeds up"
            );
        });
    }

    #[test]
    fn a_floor_sets_the_flags_the_ground_state_and_stops_the_fall() {
        run(|| {
            register("a", standing(2.0));
            assert_eq!(grounded("a"), None, "unknown before the first move");
            let result = move_call("a", MoveMode::Move, [0.0, -2.0, 0.0]);
            assert!(result.flags.below && !result.flags.above && !result.flags.sides);
            assert!(result.grounded);
            assert_eq!(grounded("a"), Some(true));
            assert!((result.effective[1] + 1.0).abs() < 1e-5);
            assert_eq!(result.hits.len(), 1);
            assert_eq!(result.hits[0].tick, 7);
            // Standing there, a simple move keeps no fall speed.
            let _ = move_call("a", MoveMode::Simple, [0.0; 3]);
            assert_eq!(fall_speed("a"), 0.0);
        });
    }

    #[test]
    fn moves_run_in_order_each_from_where_the_last_ended() {
        run(|| {
            register("a", standing(2.0));
            let _ = move_call("a", MoveMode::Move, [0.0, 3.0, 0.0]);
            // Up three, then down five: the floor stops it one under the start.
            let down = move_call("a", MoveMode::Move, [0.0, -5.0, 0.0]);
            assert!(
                (down.effective[1] + 4.0).abs() < 1e-5,
                "{:?}",
                down.effective
            );
            let pending = take_pending();
            assert_eq!(pending.len(), 1);
            assert!(
                (pending[0].1[1] + 1.0).abs() < 1e-5,
                "net {:?}",
                pending[0].1
            );
            assert!(take_pending().is_empty(), "taking clears it");
        });
    }

    #[test]
    fn a_move_shorter_than_the_minimum_does_not_move_but_still_probes() {
        run(|| {
            register("a", standing(1.0));
            let result = move_call("a", MoveMode::Move, [0.0, -0.0005, 0.0]);
            assert!(result.skipped);
            assert_eq!(result.effective, [0.0; 3]);
            assert_eq!(grounded("a"), Some(true), "standing on the floor");
        });
    }

    #[test]
    fn a_disabled_controller_does_nothing() {
        run(|| {
            let mut spec = standing(1.0);
            spec.enabled = false;
            register("a", spec);
            let result = move_call("a", MoveMode::Move, [1.0, 0.0, 0.0]);
            assert!(result.skipped);
            assert_eq!(result.effective, [0.0; 3]);
            assert!(take_pending().is_empty());
        });
    }

    #[test]
    fn asking_without_a_controller_or_a_world_says_why() {
        reset();
        let none = move_call("a", MoveMode::Move, [1.0, 0.0, 0.0]);
        assert!(none.error.unwrap().contains("running"));
        run(|| {
            let missing = move_call("ghost", MoveMode::Move, [1.0, 0.0, 0.0]);
            assert!(missing.error.unwrap().contains("no CharacterController"));
            register("a", standing(1.0));
            let bad = move_call("a", MoveMode::Move, [f32::NAN, 0.0, 0.0]);
            assert!(bad.error.is_some());
        });
    }

    #[test]
    fn height_changes_keep_the_feet_where_they_are() {
        run(|| {
            register("a", standing(1.0));
            let feet = |s: &CharacterControllerSpec| s.center[1] - s.height * 0.5;
            let before = feet(&spec_of("a").unwrap());
            set_property("a", ControllerProperty::Height, 1.2, Mode::ThreeD).unwrap();
            let after = spec_of("a").unwrap();
            assert!((feet(&after) - before).abs() < 1e-5);
            assert_eq!(take_reshaped().len(), 1);
            assert!(take_reshaped().is_empty());
        });
    }

    #[test]
    fn a_property_that_would_break_the_rules_is_refused_and_changes_nothing() {
        run(|| {
            register("a", standing(1.0));
            assert!(set_property("a", ControllerProperty::Height, 0.4, Mode::ThreeD).is_err());
            assert!(set_property("a", ControllerProperty::Radius, 0.0, Mode::ThreeD).is_err());
            assert!(set_property("a", ControllerProperty::Radius, f64::NAN, Mode::ThreeD).is_err());
            assert_eq!(spec_of("a").unwrap().height, 2.0);
            assert!(set_property("ghost", ControllerProperty::Radius, 1.0, Mode::ThreeD).is_err());
        });
    }

    #[test]
    fn statements_run_through_one_entry() {
        run(|| {
            register("a", standing(2.0));
            let flags = run_op("a", "move", [0.0, -3.0, 0.0]).unwrap();
            assert_eq!(flags, 4 | 8, "below and grounded");
            assert_eq!(run_op("a", "set radius", [0.4, 0.0, 0.0]), Ok(0));
            assert_eq!(spec_of("a").unwrap().radius, 0.4);
            assert!(run_op("a", "set mood", [1.0; 3]).is_err());
            assert!(run_op("a", "set radius", [-1.0, 0.0, 0.0]).is_err());
            assert!(run_op("ghost", "move", [1.0; 3]).is_err());
        });
    }

    #[test]
    fn reporters_read_the_last_move() {
        run(|| {
            register("a", standing(1.0));
            let _ = move_call("a", MoveMode::Move, [0.0, -2.0, 0.0]);
            assert_eq!(read_number("a", "grounded", 0), 1.0);
            assert_eq!(read_number("a", "below", 0), 1.0);
            assert_eq!(read_number("a", "sides", 0), 0.0);
            assert_eq!(read_number("a", "flags", 0), 4.0);
            assert_eq!(read_number("a", "hit count", 0), 1.0);
            assert_eq!(read_number("a", "normal y", 1), 1.0);
            assert_eq!(read_number("a", "normal y", 2), 0.0, "no second hit");
            assert_eq!(read_number("a", "tick", 0), 7.0);
            assert_eq!(read_text("a", "collider", 1), "ground");
            assert_eq!(read_text("a", "actor", 2), "");
            let hits = take_hits();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].0, "a");
        });
    }

    #[test]
    fn a_ceiling_ends_a_rise() {
        struct Roof;
        impl ControllerService for Roof {
            fn step(&self, r: &StepRequest) -> Result<StepOutcome, String> {
                let mut effective = r.displacement;
                let mut hits = Vec::new();
                if effective[1] > 0.5 {
                    effective[1] = 0.5;
                    hits.push(RawHit {
                        actor: "roof".into(),
                        body: None,
                        collider: ColliderId::from("roof"),
                        point: [0.0; 3],
                        normal: [0.0, -1.0, 0.0],
                        applied: effective,
                    });
                }
                Ok(StepOutcome {
                    effective,
                    hits,
                    ..StepOutcome::default()
                })
            }
            fn gravity(&self) -> [f32; 3] {
                [0.0, -10.0, 0.0]
            }
            fn timestep(&self) -> f32 {
                0.1
            }
            fn mode(&self) -> Mode {
                Mode::ThreeD
            }
        }
        reset();
        with_service(&Roof, 1, || {
            register("a", standing(1.0));
            let result = move_call("a", MoveMode::Move, [0.0, 2.0, 0.0]);
            assert!(result.flags.above && !result.grounded);
        });
    }
}
