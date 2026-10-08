//! The CharacterMotor: the reusable gameplay layer over a CharacterController.
//!
//! Input, AI, blocks and scripts all submit the same [`MovementIntent`]; the
//! motor owns velocity, jumping, crouching and grounding state and moves the
//! controller by a displacement each fixed tick. Core decides everything here
//! (`plan` and `settle` are pure); the runtime only supplies the world through
//! the controller service and reads the answers back.

use super::controller::{self, CollisionFlags, ControllerProperty, MoveMode, MoveResult, Surface};
use super::ids::ComponentId;
use crate::scene::Mode;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

/// Whose hands are on the motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MotorOwner {
    /// The runtime fills the intent from the project's actions each tick.
    #[default]
    Player,
    /// A brain or another system submits intents.
    Ai,
    /// Blocks and scripts submit intents.
    Script,
}

/// What a direction intent is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MoveSpace {
    World,
    /// Turned by the actor's own facing.
    Actor,
    /// Turned by the world camera's yaw.
    #[default]
    Camera,
}

/// What a motor statement does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MotorAction {
    /// Steer: the vector is the direction wanted (length 1 is full speed).
    #[default]
    Intent,
    Jump,
    JumpRelease,
    SprintOn,
    SprintOff,
    CrouchOn,
    CrouchOff,
    /// Knockback: the vector is added and fades by the external drag.
    Push,
    Stop,
}

impl MotorAction {
    pub const ALL: [MotorAction; 9] = [
        Self::Intent,
        Self::Jump,
        Self::JumpRelease,
        Self::SprintOn,
        Self::SprintOff,
        Self::CrouchOn,
        Self::CrouchOff,
        Self::Push,
        Self::Stop,
    ];

    /// The op `run_op` takes.
    pub fn op(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Jump => "jump",
            Self::JumpRelease => "jump release",
            Self::SprintOn => "sprint on",
            Self::SprintOff => "sprint off",
            Self::CrouchOn => "crouch on",
            Self::CrouchOff => "crouch off",
            Self::Push => "push",
            Self::Stop => "stop",
        }
    }
}

/// A motor setting a block can change during a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MotorProperty {
    Enabled,
    #[default]
    WalkSpeed,
    SprintSpeed,
    CrouchSpeed,
    Acceleration,
    Braking,
    AirAcceleration,
    AirControl,
    TurnSpeed,
    GravityScale,
    TerminalSpeed,
    JumpHeight,
    MaxJumps,
    CoyoteTime,
    JumpBuffer,
    SlideOnSteep,
}

impl MotorProperty {
    pub const ALL: [MotorProperty; 16] = [
        Self::Enabled,
        Self::WalkSpeed,
        Self::SprintSpeed,
        Self::CrouchSpeed,
        Self::Acceleration,
        Self::Braking,
        Self::AirAcceleration,
        Self::AirControl,
        Self::TurnSpeed,
        Self::GravityScale,
        Self::TerminalSpeed,
        Self::JumpHeight,
        Self::MaxJumps,
        Self::CoyoteTime,
        Self::JumpBuffer,
        Self::SlideOnSteep,
    ];

    /// The name `set` takes; the same order as [`PROPERTIES`].
    pub fn name(self) -> &'static str {
        PROPERTIES[Self::ALL.iter().position(|p| *p == self).unwrap_or(0)]
    }
}

/// A CharacterMotor component. Speeds are world units a second and sizes
/// world units: metres in 3D, pixels in 2D (see [`CharacterMotorSpec::for_mode`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterMotorSpec {
    pub id: ComponentId,
    pub enabled: bool,
    pub owner: MotorOwner,
    /// Which local player's actions drive it (0 is the first), when owned
    /// by the player.
    pub player: u8,
    pub space: MoveSpace,
    /// 2D only: move across the whole plane with no up, gravity or jump.
    pub top_down: bool,
    pub walk_speed: f32,
    pub sprint_speed: f32,
    pub crouch_speed: f32,
    /// Speeding up on the ground, units a second squared.
    pub ground_acceleration: f32,
    /// Slowing down on the ground when nothing is asked.
    pub ground_braking: f32,
    pub air_acceleration: f32,
    /// How much of the steering the air keeps (0 none, 1 all of it).
    pub air_control: f32,
    /// Turn the actor towards where it moves, degrees a second (0 = never).
    pub turn_speed: f32,
    /// Scale a diagonal intent back to unit length.
    pub normalize_input: bool,
    pub gravity_scale: f32,
    /// The fastest it falls, along up.
    pub terminal_fall_speed: f32,
    /// After a move that leaves the ground, reach down this far to stay on it.
    pub ground_snap_distance: f32,
    /// Slide down slopes past the controller's limit.
    pub slide_on_steep: bool,
    pub slide_speed: f32,
    pub jump_height: f32,
    pub max_jumps: u32,
    /// Letting go of jump while rising scales the rise by this (1 = no cut).
    pub jump_cut: f32,
    pub coyote_time: f32,
    pub jump_buffer: f32,
    /// The controller's height while crouching (0 = cannot crouch).
    pub crouch_height: f32,
    /// How fast knockback and impulses fade, units a second squared.
    pub external_drag: f32,
}

impl Default for CharacterMotorSpec {
    fn default() -> Self {
        Self::for_mode(Mode::ThreeD)
    }
}

impl CharacterMotorSpec {
    pub fn for_mode(mode: Mode) -> Self {
        let scale = match mode {
            Mode::ThreeD => 1.0,
            // About 48 pixels to the metre.
            Mode::TwoD => 48.0,
        };
        Self {
            id: ComponentId::generate(),
            enabled: true,
            owner: MotorOwner::Player,
            player: 0,
            space: if mode == Mode::ThreeD {
                MoveSpace::Camera
            } else {
                MoveSpace::World
            },
            top_down: false,
            walk_speed: 5.0 * scale,
            sprint_speed: 8.0 * scale,
            crouch_speed: 2.5 * scale,
            ground_acceleration: 50.0 * scale,
            ground_braking: 60.0 * scale,
            air_acceleration: 15.0 * scale,
            air_control: 1.0,
            turn_speed: if mode == Mode::ThreeD { 720.0 } else { 0.0 },
            normalize_input: true,
            gravity_scale: 1.0,
            terminal_fall_speed: 50.0 * scale,
            ground_snap_distance: 0.3 * scale,
            slide_on_steep: false,
            slide_speed: 6.0 * scale,
            jump_height: 1.2 * scale,
            max_jumps: 1,
            jump_cut: 0.5,
            coyote_time: 0.1,
            jump_buffer: 0.1,
            crouch_height: if mode == Mode::ThreeD { 1.0 } else { 32.0 },
            external_drag: 20.0 * scale,
        }
    }

    /// What is wrong with the numbers, by field.
    pub fn validate(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut bad = |field: &str, message: &str| out.push((field.to_string(), message.into()));
        let positive = |v: f32| v.is_finite() && v > 0.0;
        let non_negative = |v: f32| v.is_finite() && v >= 0.0;
        if !positive(self.walk_speed) {
            bad("walk_speed", "The walk speed must be above zero");
        }
        if !positive(self.sprint_speed) {
            bad("sprint_speed", "The sprint speed must be above zero");
        }
        if !non_negative(self.crouch_speed) {
            bad("crouch_speed", "The crouch speed cannot be negative");
        }
        if !positive(self.ground_acceleration) {
            bad("ground_acceleration", "The acceleration must be above zero");
        }
        if !positive(self.ground_braking) {
            bad("ground_braking", "The braking must be above zero");
        }
        if !non_negative(self.air_acceleration) {
            bad(
                "air_acceleration",
                "The air acceleration cannot be negative",
            );
        }
        if !(self.air_control.is_finite() && (0.0..=1.0).contains(&self.air_control)) {
            bad("air_control", "The air control must be 0 to 1");
        }
        if !non_negative(self.turn_speed) {
            bad("turn_speed", "The turn speed cannot be negative");
        }
        if !non_negative(self.gravity_scale) {
            bad("gravity_scale", "The gravity scale cannot be negative");
        }
        if !positive(self.terminal_fall_speed) {
            bad(
                "terminal_fall_speed",
                "The terminal speed must be above zero",
            );
        }
        if !non_negative(self.ground_snap_distance) {
            bad(
                "ground_snap_distance",
                "The snap distance cannot be negative",
            );
        }
        if !non_negative(self.slide_speed) {
            bad("slide_speed", "The slide speed cannot be negative");
        }
        if !non_negative(self.jump_height) {
            bad("jump_height", "The jump height cannot be negative");
        }
        if self.max_jumps > 8 {
            bad("max_jumps", "At most 8 jumps");
        }
        if !(self.jump_cut.is_finite() && (0.0..=1.0).contains(&self.jump_cut)) {
            bad("jump_cut", "The jump cut must be 0 to 1");
        }
        if !non_negative(self.coyote_time) {
            bad("coyote_time", "The coyote time cannot be negative");
        }
        if !non_negative(self.jump_buffer) {
            bad("jump_buffer", "The jump buffer cannot be negative");
        }
        if !non_negative(self.crouch_height) {
            bad("crouch_height", "The crouch height cannot be negative");
        }
        if !non_negative(self.external_drag) {
            bad("external_drag", "The drag cannot be negative");
        }
        out
    }

    /// The speed straight up a jump of `jump_height` needs under `gravity`
    /// (a positive pull along down), or `None` when nothing pulls back.
    pub fn jump_speed(&self, gravity: f32) -> Option<f32> {
        let pull = gravity * self.gravity_scale;
        (pull > 1e-6 && self.jump_height > 0.0).then(|| (2.0 * pull * self.jump_height).sqrt())
    }
}

/// What the owner asks for this tick. Held values persist until changed; a
/// jump press is an edge that one tick consumes.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct MovementIntent {
    /// Where to go, each axis -1 to 1, in the spec's [`MoveSpace`]: x right,
    /// z back in 3D; x and y in a top-down 2D game.
    pub direction: [f32; 3],
    pub jump_pressed: bool,
    pub jump_held: bool,
    pub sprint: bool,
    pub crouch: bool,
}

/// What the motor tells the game about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MotorEvent {
    Jump,
    #[default]
    Land,
    LeftGround,
    HeadHit,
    StanceChanged,
}

impl MotorEvent {
    pub const ALL: [MotorEvent; 5] = [
        Self::Jump,
        Self::Land,
        Self::LeftGround,
        Self::HeadHit,
        Self::StanceChanged,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Jump => "jump",
            Self::Land => "land",
            Self::LeftGround => "leave ground",
            Self::HeadHit => "head hit",
            Self::StanceChanged => "stance change",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        let word = word.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|e| e.name() == word)
    }
}

/// What the world gives the motor each tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotorEnv {
    pub mode: Mode,
    /// Unit up.
    pub up: [f32; 3],
    /// The world's gravity, units a second squared.
    pub gravity: [f32; 3],
    pub dt: f32,
    /// Radians about up that the spec's space turns an intent by.
    pub yaw: f32,
    /// How far the support moved since the last tick (platform carry).
    pub carry: [f32; 3],
    /// Whether standing up would clear what is overhead.
    pub headroom: bool,
    pub slope_limit: f32,
}

/// The motor's own memory between ticks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MotorState {
    /// Velocity the motor controls, world units a second.
    pub velocity: [f32; 3],
    /// The part of it along up, last tick.
    pub vertical: f32,
    /// Knockback and impulses, fading by the spec's drag.
    pub external: [f32; 3],
    pub grounded: bool,
    pub coyote: f32,
    pub buffer: f32,
    pub jumps_used: u32,
    /// A jump is rising and has not been cut or topped out.
    pub jumping: bool,
    pub crouching: bool,
    /// Horizontal speed actually made last tick.
    pub speed: f32,
    pub desired_speed: f32,
    pub slope: f32,
    pub support: Option<String>,
    pub ground_normal: [f32; 3],
    /// Set the tick it landed, for reporters.
    pub landed: bool,
    pub fall_speed: f32,
    /// A steep slope it stands against, for sliding.
    pub steep: Option<[f32; 3]>,
    /// The normal of a wall it pressed against last tick.
    pub wall: Option<[f32; 3]>,
    /// Which way the motor wants the actor to face (radians about up).
    pub face: Option<f32>,
    pub warning: Option<String>,
    /// What happened on the last driven tick, for reporters.
    pub last_events: Vec<MotorEvent>,
}

/// What `plan` decided.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MotorPlan {
    pub displacement: [f32; 3],
    /// The controller height to move to, if the stance changed.
    pub height: Option<f32>,
    pub events: Vec<MotorEvent>,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    add(a, scale(b, -1.0))
}

/// Moves `from` towards `to` by at most `step`.
fn approach(from: [f32; 3], to: [f32; 3], step: f32) -> [f32; 3] {
    let delta = sub(to, from);
    let distance = length(delta);
    if distance <= step || distance < 1e-9 {
        to
    } else {
        add(from, scale(delta, step / distance))
    }
}

impl MotorState {
    /// Vertical (along up) and horizontal parts of a vector.
    fn split(&self, v: [f32; 3], up: [f32; 3], top_down: bool) -> (f32, [f32; 3]) {
        if top_down {
            return (0.0, [v[0], v[1], 0.0]);
        }
        let vertical = dot(v, up);
        (vertical, sub(v, scale(up, vertical)))
    }

    /// Decides this tick's displacement. Pure: the move itself is the
    /// caller's, and its result comes back through [`MotorState::settle`].
    pub fn plan(
        &mut self,
        spec: &CharacterMotorSpec,
        intent: &MovementIntent,
        env: &MotorEnv,
    ) -> MotorPlan {
        let dt = env.dt;
        let up = env.up;
        let top_down = spec.top_down && env.mode == Mode::TwoD;
        let mut plan = MotorPlan::default();
        self.landed = false;
        self.warning = None;

        // Clocks run in simulation time: coyote while it recently stood,
        // the buffer after a press.
        if self.grounded {
            self.coyote = spec.coyote_time;
            self.jumps_used = if self.jumping { self.jumps_used } else { 0 };
        } else if self.coyote > 0.0 {
            self.coyote = (self.coyote - dt).max(0.0);
            if self.coyote == 0.0 && self.jumps_used == 0 {
                // Walked off a ledge: the first jump is gone.
                self.jumps_used = 1;
            }
        }
        self.buffer = if intent.jump_pressed {
            spec.jump_buffer
        } else {
            (self.buffer - dt).max(0.0)
        };

        // Stance: crouch at once, stand only with the headroom for it.
        let wants_crouch = intent.crouch && spec.crouch_height > 0.0;
        if wants_crouch != self.crouching && (wants_crouch || env.headroom) {
            self.crouching = wants_crouch;
            plan.events.push(MotorEvent::StanceChanged);
            plan.height = Some(if wants_crouch {
                spec.crouch_height
            } else {
                0.0
            });
        }

        // Where it wants to go, turned into the world.
        let mut wish = intent.direction;
        if env.mode == Mode::ThreeD && spec.space != MoveSpace::World && env.yaw != 0.0 {
            let (s, c) = env.yaw.sin_cos();
            wish = [
                wish[0] * c + wish[2] * s,
                wish[1],
                -wish[0] * s + wish[2] * c,
            ];
        }
        if !top_down {
            wish = sub(wish, scale(up, dot(wish, up)));
        } else {
            wish[2] = 0.0;
        }
        let magnitude = length(wish);
        let clamped = if magnitude > 1.0 && spec.normalize_input {
            1.0
        } else {
            magnitude.min(1.0)
        };
        let top_speed = if self.crouching {
            spec.crouch_speed
        } else if intent.sprint {
            spec.sprint_speed
        } else {
            spec.walk_speed
        };
        let target_speed = if spec.enabled {
            top_speed * clamped
        } else {
            0.0
        };
        let target = if magnitude > 1e-6 {
            scale(wish, target_speed / magnitude)
        } else {
            [0.0; 3]
        };
        self.desired_speed = target_speed;

        let (mut vertical, horizontal) = self.split(self.velocity, up, top_down);
        let rate = if self.grounded {
            if clamped > 1e-3 {
                spec.ground_acceleration
            } else {
                spec.ground_braking
            }
        } else if clamped > 1e-3 {
            spec.air_acceleration * spec.air_control
        } else {
            // The air does not brake what it carries.
            0.0
        };
        let mut horizontal = approach(horizontal, target, rate * dt);

        // Slide down what is too steep to stand on.
        if spec.slide_on_steep
            && !top_down
            && let Some(normal) = self.steep
        {
            let flat = sub(normal, scale(up, dot(normal, up)));
            let flat_length = length(flat);
            if flat_length > 1e-6 {
                let downhill = scale(flat, spec.slide_speed / flat_length);
                horizontal = approach(horizontal, downhill, spec.ground_acceleration * dt);
            }
        }

        // Gravity, jumping and falling.
        let pull = -dot(env.gravity, up);
        let mut jumped = false;
        if !top_down {
            let can_ground_jump = self.grounded || self.coyote > 0.0;
            let can_air_jump = !can_ground_jump && self.jumps_used < spec.max_jumps;
            if self.buffer > 0.0 && spec.enabled && (can_ground_jump || can_air_jump) {
                match spec.jump_speed(pull) {
                    Some(speed) => {
                        vertical = speed;
                        self.jumps_used += 1;
                        self.jumping = true;
                        self.grounded = false;
                        self.coyote = 0.0;
                        self.buffer = 0.0;
                        jumped = true;
                        plan.events.push(MotorEvent::Jump);
                    }
                    None => {
                        self.buffer = 0.0;
                        self.warning = Some(
                            "a jump needs gravity to bring it back down; there is none here"
                                .to_string(),
                        );
                    }
                }
            }
            if !jumped {
                if self.grounded {
                    // A small push into the floor keeps the probe honest.
                    vertical = vertical.min(0.0);
                } else {
                    vertical -= pull * spec.gravity_scale * dt;
                }
            }
            if self.jumping && !intent.jump_held && vertical > 0.0 && !jumped {
                vertical *= spec.jump_cut;
                self.jumping = false;
            }
            if vertical <= 0.0 {
                self.jumping = false;
            }
            vertical = vertical.max(-spec.terminal_fall_speed);
        }

        self.vertical = vertical;
        self.velocity = add(horizontal, scale(up, vertical));
        // Knockback fades; it never carries the motor's own velocity.
        let drag = spec.external_drag * dt;
        self.external = approach(self.external, [0.0; 3], drag);
        let total = add(self.velocity, self.external);
        let mut step = scale(total, dt);
        step = add(step, env.carry);
        plan.displacement = step;
        self.face = (spec.turn_speed > 0.0 && length(horizontal) > 0.1 && env.mode == Mode::ThreeD)
            .then(|| (-horizontal[0]).atan2(-horizontal[2]));
        plan
    }

    /// Takes the move's result: what blocked it, where it stands now, what
    /// happened. Returns the events and, when the motor left the ground
    /// without jumping, how far to reach down to stay on it.
    pub fn settle(
        &mut self,
        spec: &CharacterMotorSpec,
        result: &MoveResult,
        env: &MotorEnv,
        dt: f32,
    ) -> (Vec<MotorEvent>, Option<f32>) {
        let up = env.up;
        let top_down = spec.top_down && env.mode == Mode::TwoD;
        let mut events = Vec::new();
        let was_grounded = self.grounded;
        self.steep = None;
        self.wall = None;
        let rising = dot(self.velocity, up);
        let mut floor: Option<(&controller::ControllerHit, f32)> = None;
        for hit in &result.hits {
            // The normal points at the controller: moving into the surface
            // is a negative dot, which is the part to cancel.
            let surface = CollisionFlags::classify(hit.normal, up, env.slope_limit);
            // A floor only stops the fall (below); walking up a slope keeps
            // its speed, since the controller already slid along it.
            if surface != Surface::Floor {
                for velocity in [&mut self.velocity, &mut self.external] {
                    let into = dot(*velocity, hit.normal);
                    if into < 0.0 {
                        *velocity = sub(*velocity, scale(hit.normal, into));
                    }
                }
            }
            match surface {
                Surface::Floor => {
                    let cosine = dot(hit.normal, up).clamp(-1.0, 1.0);
                    if floor.is_none_or(|(_, best)| cosine > best) {
                        floor = Some((hit, cosine));
                    }
                }
                Surface::Wall => {
                    if dot(hit.normal, up).abs() <= 0.05 {
                        self.wall = Some(hit.normal);
                    }
                    let tilt = dot(hit.normal, up);
                    if tilt > 0.05 {
                        self.steep = Some(hit.normal);
                    }
                }
                Surface::Ceiling => {}
            }
        }
        if result.flags.above && !top_down {
            if rising > 0.0 {
                let now = dot(self.velocity, up);
                if now > 0.0 {
                    self.velocity = sub(self.velocity, scale(up, now));
                }
                events.push(MotorEvent::HeadHit);
            }
            self.jumping = false;
        }
        let grounded = result.grounded && !self.jumping;
        match floor {
            Some((hit, cosine)) => {
                self.slope = cosine.acos().to_degrees();
                self.support = Some(hit.actor.clone());
                self.ground_normal = hit.normal;
            }
            None if !grounded => {
                self.slope = 0.0;
                self.support = None;
                self.ground_normal = up;
            }
            None => {}
        }
        let fall = -dot(self.velocity, up);
        if grounded {
            if !was_grounded {
                self.landed = true;
                self.fall_speed = fall.max(0.0);
                events.push(MotorEvent::Land);
            }
            let down = dot(self.velocity, up);
            if down < 0.0 {
                self.velocity = sub(self.velocity, scale(up, down));
            }
            self.vertical = self.vertical.max(0.0);
            self.jumping = false;
            self.jumps_used = 0;
        } else if was_grounded {
            events.push(MotorEvent::LeftGround);
        }
        self.grounded = grounded;
        if dt > 0.0 {
            let (_, flat) = self.split(
                [
                    result.effective[0] / dt,
                    result.effective[1] / dt,
                    result.effective[2] / dt,
                ],
                up,
                top_down,
            );
            self.speed = length(flat);
        }
        // Leaving the ground without a jump reaches down to hold on.
        let snap = (was_grounded && !grounded && !self.jumping && spec.ground_snap_distance > 0.0)
            .then_some(spec.ground_snap_distance);
        (events, snap)
    }
}

/// The motor of one actor as core holds it.
#[derive(Debug, Clone)]
struct Tracked {
    spec: CharacterMotorSpec,
    state: MotorState,
    intent: MovementIntent,
    /// The controller's standing height, to come back to from a crouch.
    standing_height: Option<f32>,
}

#[derive(Default)]
struct Registry {
    motors: HashMap<String, Tracked>,
    /// Events since the engine last collected them.
    outbox: Vec<(String, MotorEvent)>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Forgets every motor: a world is being rebuilt or a run ended.
pub fn reset() {
    REGISTRY.with(|registry| *registry.borrow_mut() = Registry::default());
}

pub fn register(actor: &str, spec: CharacterMotorSpec) {
    REGISTRY.with(|registry| {
        registry.borrow_mut().motors.insert(
            actor.to_string(),
            Tracked {
                spec,
                state: MotorState::default(),
                intent: MovementIntent::default(),
                standing_height: None,
            },
        );
    });
}

pub fn has(actor: &str) -> bool {
    REGISTRY.with(|registry| registry.borrow().motors.contains_key(actor))
}

pub fn spec_of(actor: &str) -> Option<CharacterMotorSpec> {
    REGISTRY.with(|registry| registry.borrow().motors.get(actor).map(|t| t.spec.clone()))
}

pub fn owner_of(actor: &str) -> Option<MotorOwner> {
    REGISTRY.with(|registry| registry.borrow().motors.get(actor).map(|t| t.spec.owner))
}

/// Every actor with a motor.
pub fn actors() -> Vec<String> {
    REGISTRY.with(|registry| registry.borrow().motors.keys().cloned().collect())
}

/// Events raised since the last call, by actor.
pub fn take_events() -> Vec<(String, MotorEvent)> {
    REGISTRY.with(|registry| std::mem::take(&mut registry.borrow_mut().outbox))
}

/// The actor that holds the motor up, for platform carry.
pub fn support_of(actor: &str) -> Option<String> {
    REGISTRY.with(|registry| registry.borrow().motors.get(actor)?.state.support.clone())
}

/// One motor statement, the way every adapter (blocks, compiled logic,
/// scripts) states it. `op` is one of `intent` (the vector is the direction),
/// `jump`, `jump hold` / `jump release`, `sprint on|off`, `crouch on|off`,
/// `push` (the vector is added to its knockback), `set <property>` (value in
/// the first slot) or `stop`.
pub fn run_op(actor: &str, op: &str, vector: [f32; 3]) -> Result<(), String> {
    if vector.iter().any(|v| !v.is_finite()) {
        return Err("a motor statement needs finite numbers".to_string());
    }
    let words = op.trim().to_ascii_lowercase();
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let Some(t) = registry.motors.get_mut(actor) else {
            return Err("this actor has no CharacterMotor".to_string());
        };
        match words.as_str() {
            "intent" => t.intent.direction = vector,
            "jump" => {
                t.intent.jump_pressed = true;
                t.intent.jump_held = true;
            }
            "jump hold" => t.intent.jump_held = true,
            "jump release" => t.intent.jump_held = false,
            "sprint on" => t.intent.sprint = true,
            "sprint off" => t.intent.sprint = false,
            "crouch on" => t.intent.crouch = true,
            "crouch off" => t.intent.crouch = false,
            "push" => t.state.external = add(t.state.external, vector),
            "stop" => {
                t.intent = MovementIntent::default();
                t.state.velocity = [0.0; 3];
                t.state.external = [0.0; 3];
            }
            other => {
                let Some(name) = other.strip_prefix("set ") else {
                    return Err(format!("a motor has no statement called \"{op}\""));
                };
                set_property(&mut t.spec, name, vector[0])?;
            }
        }
        Ok(())
    })
}

/// Changes one motor setting for the rest of the run.
fn set_property(spec: &mut CharacterMotorSpec, name: &str, value: f32) -> Result<(), String> {
    let mut next = spec.clone();
    match name.trim() {
        "enabled" => next.enabled = value != 0.0,
        "walk speed" => next.walk_speed = value,
        "sprint speed" => next.sprint_speed = value,
        "crouch speed" => next.crouch_speed = value,
        "acceleration" => next.ground_acceleration = value,
        "braking" => next.ground_braking = value,
        "air acceleration" => next.air_acceleration = value,
        "air control" => next.air_control = value,
        "turn speed" => next.turn_speed = value,
        "gravity scale" => next.gravity_scale = value,
        "terminal speed" => next.terminal_fall_speed = value,
        "jump height" => next.jump_height = value,
        "max jumps" => next.max_jumps = value.max(0.0) as u32,
        "coyote time" => next.coyote_time = value,
        "jump buffer" => next.jump_buffer = value,
        "slide on steep" => next.slide_on_steep = value != 0.0,
        other => return Err(format!("a motor has no setting called \"{other}\"")),
    }
    if let Some((field, why)) = next.validate().into_iter().next() {
        return Err(format!("{field}: {why}"));
    }
    *spec = next;
    Ok(())
}

/// Properties `set` accepts, for dropdowns.
pub const PROPERTIES: [&str; 16] = [
    "enabled",
    "walk speed",
    "sprint speed",
    "crouch speed",
    "acceleration",
    "braking",
    "air acceleration",
    "air control",
    "turn speed",
    "gravity scale",
    "terminal speed",
    "jump height",
    "max jumps",
    "coyote time",
    "jump buffer",
    "slide on steep",
];

/// A number from `actor`'s motor, by the name a reporter gives it.
pub fn read_number(actor: &str, field: &str) -> f64 {
    REGISTRY.with(|registry| {
        let registry = registry.borrow();
        let Some(t) = registry.motors.get(actor) else {
            return 0.0;
        };
        let up = |v: [f32; 3]| f64::from(length(v));
        let n = |b: bool| f64::from(b);
        match field.trim().to_ascii_lowercase().as_str() {
            "exists" => 1.0,
            "grounded" => n(t.state.grounded),
            "rising" => n(!t.state.grounded && t.state.vertical > 0.01),
            "falling" => n(!t.state.grounded && t.state.vertical < -0.01),
            "landed" => n(t.state.landed),
            "jumped" => n(t.state.last_events.contains(&MotorEvent::Jump)),
            "left ground" => n(t.state.last_events.contains(&MotorEvent::LeftGround)),
            "hit head" => n(t.state.last_events.contains(&MotorEvent::HeadHit)),
            "changed stance" => n(t.state.last_events.contains(&MotorEvent::StanceChanged)),
            "crouching" => n(t.state.crouching),
            "on wall" => n(t.state.wall.is_some()),
            "wall normal x" => f64::from(t.state.wall.map_or(0.0, |w| w[0])),
            "wall normal y" => f64::from(t.state.wall.map_or(0.0, |w| w[1])),
            "speed" => f64::from(t.state.speed),
            "desired speed" => f64::from(t.state.desired_speed),
            "vertical speed" => f64::from(t.state.vertical),
            "can jump" => n(t.state.can_jump(&t.spec)),
            "jumps used" => f64::from(t.state.jumps_used),
            "jumps left" => f64::from(t.spec.max_jumps.saturating_sub(t.state.jumps_used)),
            "slope" => f64::from(t.state.slope),
            "ground normal x" => f64::from(t.state.ground_normal[0]),
            "ground normal y" => f64::from(t.state.ground_normal[1]),
            "ground normal z" => f64::from(t.state.ground_normal[2]),
            "knockback" => up(t.state.external),
            "walk speed" => f64::from(t.spec.walk_speed),
            "jump height" => f64::from(t.spec.jump_height),
            "enabled" => n(t.spec.enabled),
            _ => 0.0,
        }
    })
}

/// Words from `actor`'s motor: the support it stands on, or why it could not.
pub fn read_text(actor: &str, field: &str) -> String {
    REGISTRY.with(|registry| {
        let registry = registry.borrow();
        let Some(t) = registry.motors.get(actor) else {
            return String::new();
        };
        match field.trim().to_ascii_lowercase().as_str() {
            "support" => t.state.support.clone().unwrap_or_default(),
            "owner" => format!("{:?}", t.spec.owner),
            "warning" => t.state.warning.clone().unwrap_or_default(),
            "state" => t.state.label().to_string(),
            _ => String::new(),
        }
    })
}

impl MotorState {
    fn can_jump(&self, spec: &CharacterMotorSpec) -> bool {
        spec.enabled
            && spec.jump_height > 0.0
            && (self.grounded || self.coyote > 0.0 || self.jumps_used < spec.max_jumps)
    }

    /// `grounded`, `jumping`, `falling` or `crouching`, for animation bindings.
    pub fn label(&self) -> &'static str {
        if self.crouching && self.grounded {
            "crouching"
        } else if self.grounded {
            "grounded"
        } else if self.jumping || self.vertical > 0.01 {
            "rising"
        } else {
            "falling"
        }
    }
}

/// Hands the motor of `actor` the player's intent for this tick; used by the
/// runtime for [`MotorOwner::Player`] motors. Held values replace the old
/// ones; a jump press latches until a tick consumes it.
pub fn submit_input(
    actor: &str,
    direction: [f32; 3],
    jump_pressed: bool,
    jump_held: bool,
    sprint: bool,
    crouch: bool,
) {
    REGISTRY.with(|registry| {
        if let Some(t) = registry.borrow_mut().motors.get_mut(actor) {
            t.intent.direction = direction;
            t.intent.jump_pressed |= jump_pressed;
            t.intent.jump_held = jump_held;
            t.intent.sprint = sprint;
            t.intent.crouch = crouch;
        }
    });
}

/// What a drive did, for the runtime to apply to the actor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Driven {
    /// Which way to face, radians about up, if the spec turns the actor.
    pub face: Option<f32>,
    pub turn_speed: f32,
    pub events: Vec<MotorEvent>,
}

/// Runs one fixed tick of `actor`'s motor through its controller. Needs the
/// controller service in scope. `yaw` and `carry` come from the world.
pub fn drive(actor: &str, yaw: f32, carry: [f32; 3]) -> Result<Driven, String> {
    let Some((gravity, dt, mode, _)) = controller::service_info() else {
        return Err("a motor only moves while the game is running".to_string());
    };
    let Some(controller_spec) = controller::spec_of(actor) else {
        return Err("a CharacterMotor needs a CharacterController on the same actor".to_string());
    };
    let up = controller_spec.up_unit(mode);
    // Take the motor out so the controller calls below can borrow the registry.
    let Some(mut t) = REGISTRY.with(|registry| registry.borrow_mut().motors.remove(actor)) else {
        return Err("this actor has no CharacterMotor".to_string());
    };
    let outcome = (|| {
        if !t.spec.enabled || !controller_spec.enabled {
            t.intent.jump_pressed = false;
            return Driven::default();
        }
        let standing = *t.standing_height.get_or_insert(controller_spec.height);
        // Headroom for standing: a sweep up by the height the stance gives back.
        let headroom = if t.state.crouching {
            let extra = (standing - controller_spec.height).max(0.0);
            controller::probe_move(actor, scale(up, extra))
                .is_none_or(|got| dot(got, up) >= extra - 1e-3)
        } else {
            true
        };
        let env = MotorEnv {
            mode,
            up,
            gravity,
            dt,
            yaw,
            carry,
            headroom,
            slope_limit: controller_spec.slope_limit,
        };
        let intent = t.intent;
        t.intent.jump_pressed = false;
        let mut plan = t.state.plan(&t.spec, &intent, &env);
        if let Some(height) = plan.height {
            let target = if height <= 0.0 {
                standing
            } else {
                height.min(standing)
            };
            let _ = controller::set_property(
                actor,
                ControllerProperty::Height,
                f64::from(target),
                mode,
            );
        }
        let mut events = std::mem::take(&mut plan.events);
        let result = controller::move_call(actor, MoveMode::Move, plan.displacement);
        if result.error.is_none() {
            let (more, snap) = t.state.settle(&t.spec, &result, &env, dt);
            events.extend(more);
            if let Some(distance) = snap {
                let down = controller::move_call(actor, MoveMode::Move, scale(up, -distance));
                if down.grounded && down.error.is_none() {
                    t.state.grounded = true;
                    t.state.jumps_used = 0;
                    // The snap was not a fall: the leave-ground event is retracted.
                    events.retain(|e| *e != MotorEvent::LeftGround);
                    let into = dot(t.state.velocity, up);
                    if into < 0.0 {
                        t.state.velocity = sub(t.state.velocity, scale(up, into));
                    }
                }
            }
        } else {
            t.state.warning = result.error.clone();
        }
        t.state.last_events = events.clone();
        Driven {
            face: t.state.face,
            turn_speed: t.spec.turn_speed,
            events,
        }
    })();
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        for event in &outcome.events {
            registry.outbox.push((actor.to_string(), *event));
        }
        registry.motors.insert(actor.to_string(), t);
    });
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> MotorEnv {
        MotorEnv {
            mode: Mode::ThreeD,
            up: [0.0, 1.0, 0.0],
            gravity: [0.0, -10.0, 0.0],
            dt: 0.1,
            yaw: 0.0,
            carry: [0.0; 3],
            headroom: true,
            slope_limit: 45.0,
        }
    }

    fn spec() -> CharacterMotorSpec {
        let mut spec = CharacterMotorSpec::default();
        spec.ground_acceleration = 100.0;
        spec.ground_braking = 100.0;
        spec
    }

    fn standing() -> MotorState {
        MotorState {
            grounded: true,
            ..MotorState::default()
        }
    }

    fn run_right() -> MovementIntent {
        MovementIntent {
            direction: [1.0, 0.0, 0.0],
            ..MovementIntent::default()
        }
    }

    #[test]
    fn jump_speed_reaches_the_asked_height() {
        let mut s = spec();
        s.jump_height = 1.25;
        let v = s.jump_speed(10.0).unwrap();
        assert!((v * v / 20.0 - 1.25).abs() < 1e-5);
        assert_eq!(s.jump_speed(0.0), None, "zero gravity is explicit");
        s.gravity_scale = 0.0;
        assert_eq!(s.jump_speed(10.0), None);
    }

    #[test]
    fn walking_speeds_up_and_stops_at_the_walk_speed() {
        let s = spec();
        let mut m = standing();
        m.plan(&s, &run_right(), &env());
        assert!(
            (m.velocity[0] - 5.0).abs() < 1e-5,
            "100 a second squared over 0.1 s is capped at the speed"
        );
        let mut slow = spec();
        slow.ground_acceleration = 10.0;
        let mut m = standing();
        m.plan(&slow, &run_right(), &env());
        assert!((m.velocity[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sprint_and_crouch_pick_their_speeds() {
        let s = spec();
        let mut m = standing();
        m.plan(
            &s,
            &MovementIntent {
                sprint: true,
                ..run_right()
            },
            &env(),
        );
        assert!((m.velocity[0] - 8.0).abs() < 1e-5);
        let mut m = standing();
        let plan = m.plan(
            &s,
            &MovementIntent {
                crouch: true,
                ..run_right()
            },
            &env(),
        );
        assert!((m.velocity[0] - 2.5).abs() < 1e-5);
        assert_eq!(plan.height, Some(1.0));
        assert_eq!(plan.events, vec![MotorEvent::StanceChanged]);
    }

    #[test]
    fn a_diagonal_is_not_faster() {
        let s = spec();
        let mut m = standing();
        m.plan(
            &s,
            &MovementIntent {
                direction: [1.0, 0.0, 1.0],
                ..MovementIntent::default()
            },
            &env(),
        );
        let speed = (m.velocity[0].powi(2) + m.velocity[2].powi(2)).sqrt();
        assert!((speed - 5.0).abs() < 1e-4, "{speed}");
    }

    #[test]
    fn analog_input_scales_the_speed() {
        let s = spec();
        let mut m = standing();
        m.plan(
            &s,
            &MovementIntent {
                direction: [0.5, 0.0, 0.0],
                ..MovementIntent::default()
            },
            &env(),
        );
        assert!((m.velocity[0] - 2.5).abs() < 1e-5);
        assert_eq!(m.desired_speed, 2.5);
    }

    #[test]
    fn the_camera_turns_the_intent() {
        let s = spec();
        let mut e = env();
        e.yaw = std::f32::consts::FRAC_PI_2;
        let mut m = standing();
        // Forward is -z: with the camera turned a quarter, it becomes -x.
        m.plan(
            &s,
            &MovementIntent {
                direction: [0.0, 0.0, -1.0],
                ..MovementIntent::default()
            },
            &e,
        );
        assert!((m.velocity[0] + 5.0).abs() < 1e-4, "{:?}", m.velocity);
        assert!(m.velocity[2].abs() < 1e-4);
    }

    #[test]
    fn world_space_ignores_the_camera() {
        let mut s = spec();
        s.space = MoveSpace::World;
        let mut e = env();
        e.yaw = 1.0;
        let mut m = standing();
        m.plan(&s, &run_right(), &e);
        assert!((m.velocity[0] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn a_jump_leaves_at_the_jump_speed_and_events() {
        let s = spec();
        let mut m = standing();
        let plan = m.plan(
            &s,
            &MovementIntent {
                jump_pressed: true,
                jump_held: true,
                ..MovementIntent::default()
            },
            &env(),
        );
        let expected = (2.0f32 * 10.0 * s.jump_height).sqrt();
        assert!((m.velocity[1] - expected).abs() < 1e-4);
        assert!(plan.events.contains(&MotorEvent::Jump));
        assert!(!m.grounded);
        assert_eq!(m.jumps_used, 1);
    }

    #[test]
    fn releasing_jump_cuts_the_rise_once() {
        let s = spec();
        let mut m = standing();
        m.plan(
            &s,
            &MovementIntent {
                jump_pressed: true,
                jump_held: true,
                ..MovementIntent::default()
            },
            &env(),
        );
        let before = m.velocity[1];
        m.plan(&s, &MovementIntent::default(), &env());
        let after = m.velocity[1];
        assert!(after < before * 0.6 && after > 0.0, "{before} {after}");
        assert!(!m.jumping);
    }

    #[test]
    fn holding_jump_rises_the_whole_way() {
        let s = spec();
        let mut m = standing();
        let held = MovementIntent {
            jump_pressed: true,
            jump_held: true,
            ..MovementIntent::default()
        };
        m.plan(&s, &held, &env());
        let first = m.velocity[1];
        m.plan(
            &s,
            &MovementIntent {
                jump_held: true,
                ..MovementIntent::default()
            },
            &env(),
        );
        assert!((m.velocity[1] - (first - 10.0 * 0.1)).abs() < 1e-4);
    }

    #[test]
    fn coyote_time_allows_a_late_jump() {
        let mut s = spec();
        s.coyote_time = 0.25;
        let mut m = standing();
        // Walk off: not grounded, coyote still running.
        m.plan(&s, &MovementIntent::default(), &env());
        m.grounded = false;
        let plan = m.plan(
            &s,
            &MovementIntent {
                jump_pressed: true,
                jump_held: true,
                ..MovementIntent::default()
            },
            &env(),
        );
        assert!(
            plan.events.contains(&MotorEvent::Jump),
            "a jump inside the coyote window works"
        );
        // Past the window it does not.
        let mut late = standing();
        late.plan(&s, &MovementIntent::default(), &env());
        late.grounded = false;
        for _ in 0..3 {
            late.plan(&s, &MovementIntent::default(), &env());
        }
        let plan = late.plan(
            &s,
            &MovementIntent {
                jump_pressed: true,
                ..MovementIntent::default()
            },
            &env(),
        );
        assert!(!plan.events.contains(&MotorEvent::Jump));
    }

    #[test]
    fn a_buffered_press_jumps_on_landing() {
        let mut s = spec();
        s.jump_buffer = 0.25;
        let mut m = MotorState {
            jumps_used: 1,
            ..MotorState::default()
        };
        m.plan(
            &s,
            &MovementIntent {
                jump_pressed: true,
                ..MovementIntent::default()
            },
            &env(),
        );
        assert!(m.buffer > 0.0);
        m.grounded = true;
        let plan = m.plan(&s, &MovementIntent::default(), &env());
        assert!(
            plan.events.contains(&MotorEvent::Jump),
            "the press waited for the floor"
        );
    }

    #[test]
    fn a_double_jump_needs_the_extra_jump() {
        let mut s = spec();
        s.coyote_time = 0.0;
        let press = MovementIntent {
            jump_pressed: true,
            jump_held: true,
            ..MovementIntent::default()
        };
        let mut m = standing();
        m.plan(&s, &press, &env());
        let plan = m.plan(&s, &press, &env());
        assert!(!plan.events.contains(&MotorEvent::Jump), "one jump only");
        s.max_jumps = 2;
        let mut m = standing();
        m.plan(&s, &press, &env());
        let plan = m.plan(&s, &press, &env());
        assert!(plan.events.contains(&MotorEvent::Jump));
        assert_eq!(m.jumps_used, 2);
        let plan = m.plan(&s, &press, &env());
        assert!(!plan.events.contains(&MotorEvent::Jump));
    }

    #[test]
    fn no_gravity_refuses_a_jump_and_says_why() {
        let s = spec();
        let mut e = env();
        e.gravity = [0.0; 3];
        let mut m = standing();
        let plan = m.plan(
            &s,
            &MovementIntent {
                jump_pressed: true,
                ..MovementIntent::default()
            },
            &e,
        );
        assert!(!plan.events.contains(&MotorEvent::Jump));
        assert!(m.warning.as_deref().unwrap().contains("gravity"));
    }

    #[test]
    fn falling_is_capped_at_the_terminal_speed() {
        let mut s = spec();
        s.terminal_fall_speed = 12.0;
        let mut m = MotorState::default();
        for _ in 0..50 {
            m.plan(&s, &MovementIntent::default(), &env());
        }
        assert_eq!(m.velocity[1], -12.0);
    }

    #[test]
    fn the_air_keeps_its_momentum_and_steers_by_air_control() {
        let mut s = spec();
        s.air_control = 0.5;
        s.air_acceleration = 10.0;
        let mut m = MotorState::default();
        m.velocity = [4.0, 0.0, 0.0];
        m.plan(&s, &MovementIntent::default(), &env());
        assert_eq!(m.velocity[0], 4.0, "nothing brakes the air");
        m.plan(
            &s,
            &MovementIntent {
                direction: [-1.0, 0.0, 0.0],
                ..MovementIntent::default()
            },
            &env(),
        );
        assert!(
            (m.velocity[0] - 3.5).abs() < 1e-5,
            "half the 10 a second squared over 0.1 s"
        );
    }

    #[test]
    fn standing_needs_headroom() {
        let s = spec();
        let mut m = standing();
        m.crouching = true;
        let mut low = env();
        low.headroom = false;
        let plan = m.plan(&s, &MovementIntent::default(), &low);
        assert!(m.crouching, "still crouched under a low ceiling");
        assert_eq!(plan.height, None);
        let plan = m.plan(&s, &MovementIntent::default(), &env());
        assert!(!m.crouching);
        assert_eq!(
            plan.height,
            Some(0.0),
            "zero means back to the standing height"
        );
    }

    #[test]
    fn a_floor_hit_lands_and_resets_jumps() {
        let s = spec();
        let mut m = MotorState::default();
        m.velocity = [0.0, -8.0, 0.0];
        m.jumps_used = 1;
        let result = MoveResult {
            grounded: true,
            flags: CollisionFlags {
                below: true,
                ..Default::default()
            },
            hits: vec![hit([0.0, 1.0, 0.0])],
            effective: [0.0, -0.8, 0.0],
            ..MoveResult::default()
        };
        let (events, snap) = m.settle(&s, &result, &env(), 0.1);
        assert_eq!(events, vec![MotorEvent::Land]);
        assert_eq!(snap, None);
        assert!(m.grounded && m.landed);
        assert_eq!(m.velocity[1], 0.0);
        assert_eq!(m.jumps_used, 0);
        assert_eq!(m.fall_speed, 8.0);
        assert_eq!(m.support.as_deref(), Some("floor"));
    }

    fn hit(normal: [f32; 3]) -> controller::ControllerHit {
        controller::ControllerHit {
            actor: "floor".into(),
            body: None,
            collider: "floor".into(),
            point: [0.0; 3],
            normal,
            direction: [0.0; 3],
            move_length: 1.0,
            applied: [0.0; 3],
            tick: 0,
        }
    }

    #[test]
    fn a_ceiling_cancels_the_rise_and_reports_it() {
        let s = spec();
        let mut m = MotorState::default();
        m.velocity = [0.0, 5.0, 0.0];
        let result = MoveResult {
            flags: CollisionFlags {
                above: true,
                ..Default::default()
            },
            hits: vec![hit([0.0, -1.0, 0.0])],
            ..MoveResult::default()
        };
        let (events, _) = m.settle(&s, &result, &env(), 0.1);
        assert!(events.contains(&MotorEvent::HeadHit));
        assert_eq!(m.velocity[1], 0.0);
    }

    #[test]
    fn a_wall_cancels_only_the_blocked_velocity() {
        let s = spec();
        let mut m = MotorState::default();
        m.velocity = [3.0, 0.0, 2.0];
        let result = MoveResult {
            flags: CollisionFlags {
                sides: true,
                ..Default::default()
            },
            hits: vec![hit([-1.0, 0.0, 0.0])],
            ..MoveResult::default()
        };
        m.settle(&s, &result, &env(), 0.1);
        assert_eq!(m.velocity, [0.0, 0.0, 2.0]);
    }

    #[test]
    fn a_vertical_wall_is_remembered_for_one_tick() {
        let s = spec();
        let mut m = MotorState::default();
        let result = MoveResult {
            hits: vec![hit([-1.0, 0.0, 0.0])],
            ..MoveResult::default()
        };
        m.settle(&s, &result, &env(), 0.1);
        assert_eq!(m.wall, Some([-1.0, 0.0, 0.0]));
        m.settle(&s, &MoveResult::default(), &env(), 0.1);
        assert_eq!(m.wall, None);
    }

    #[test]
    fn leaving_a_ledge_reaches_down_to_stay_on_it() {
        let s = spec();
        let mut m = standing();
        let result = MoveResult {
            grounded: false,
            ..MoveResult::default()
        };
        let (events, snap) = m.settle(&s, &result, &env(), 0.1);
        assert_eq!(events, vec![MotorEvent::LeftGround]);
        assert_eq!(snap, Some(s.ground_snap_distance));
    }

    #[test]
    fn a_jump_does_not_snap_back() {
        let s = spec();
        let mut m = standing();
        m.plan(
            &s,
            &MovementIntent {
                jump_pressed: true,
                jump_held: true,
                ..MovementIntent::default()
            },
            &env(),
        );
        let (_, snap) = m.settle(&s, &MoveResult::default(), &env(), 0.1);
        assert_eq!(snap, None);
    }

    #[test]
    fn a_steep_slope_slides_when_asked() {
        let mut s = spec();
        s.slide_on_steep = true;
        s.slide_speed = 6.0;
        let mut m = standing();
        m.grounded = false;
        m.steep = Some([0.8, 0.6, 0.0]);
        m.plan(&s, &MovementIntent::default(), &env());
        assert!(m.velocity[0] > 0.0, "downhill is the way the normal leans");
    }

    #[test]
    fn external_velocity_fades() {
        let s = spec();
        let mut m = standing();
        m.external = [10.0, 0.0, 0.0];
        let first = m.plan(&s, &MovementIntent::default(), &env());
        assert!(first.displacement[0] > 0.0);
        for _ in 0..10 {
            m.plan(&s, &MovementIntent::default(), &env());
        }
        assert_eq!(m.external, [0.0; 3]);
    }

    #[test]
    fn top_down_moves_across_the_plane_with_no_gravity() {
        let mut s = CharacterMotorSpec::for_mode(Mode::TwoD);
        s.top_down = true;
        s.ground_acceleration = 100000.0;
        let mut e = env();
        e.mode = Mode::TwoD;
        e.gravity = [0.0; 3];
        let mut m = MotorState::default();
        m.plan(
            &s,
            &MovementIntent {
                direction: [0.0, 1.0, 0.0],
                ..MovementIntent::default()
            },
            &e,
        );
        assert!(m.velocity[1] > 0.0, "{:?}", m.velocity);
    }

    #[test]
    fn validation_names_the_field() {
        let mut s = spec();
        s.air_control = 2.0;
        s.walk_speed = 0.0;
        let fields: Vec<_> = s.validate().into_iter().map(|(f, _)| f).collect();
        assert!(fields.contains(&"air_control".to_string()));
        assert!(fields.contains(&"walk_speed".to_string()));
        assert!(spec().validate().is_empty());
        assert!(
            CharacterMotorSpec::for_mode(Mode::TwoD)
                .validate()
                .is_empty()
        );
    }

    #[test]
    fn statements_edit_the_intent_and_settings() {
        reset();
        register("a", spec());
        run_op("a", "intent", [1.0, 0.0, 0.0]).unwrap();
        run_op("a", "sprint on", [0.0; 3]).unwrap();
        run_op("a", "jump", [0.0; 3]).unwrap();
        run_op("a", "set walk speed", [7.0, 0.0, 0.0]).unwrap();
        assert_eq!(spec_of("a").unwrap().walk_speed, 7.0);
        assert!(run_op("a", "set walk speed", [-1.0, 0.0, 0.0]).is_err());
        assert!(run_op("a", "set mood", [1.0; 3]).is_err());
        assert!(run_op("a", "dance", [0.0; 3]).is_err());
        assert!(run_op("ghost", "jump", [0.0; 3]).is_err());
        assert_eq!(read_number("a", "exists"), 1.0);
        assert_eq!(read_number("ghost", "exists"), 0.0);
    }

    #[test]
    fn events_have_names_that_parse() {
        for event in MotorEvent::ALL {
            assert_eq!(MotorEvent::parse(event.name()), Some(event));
        }
        assert_eq!(MotorEvent::parse("fly"), None);
    }
}
