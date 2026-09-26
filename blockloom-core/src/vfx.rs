//! The VFX graph: the modules an `Emitter` is built from beyond its base
//! dials, and the CPU pool that runs them where the GPU sim can't.
//!
//! An emitter is a fixed stack rather than free nodes, the way Niagara's
//! emitter stacks read: spawn (rate, bursts, a shape), initialize (the base
//! dials on [`ParticleSpec`]), update (an ordered list of [`UpdateModule`]s),
//! render ([`ParticleRender`], curves over life, an optional [`RibbonSpec`]).
//! The GPU sim (`shaders/vfx_sim.wesl` in the runtime) and [`Pool`] run the
//! same stack over the same [`Particle`] layout, so either one fills the
//! buffer the renderer draws.

use crate::material::{ParticleSpec, hex_to_linear};
// Core's glam isn't Bevy's: the runtime builds these through arrays.
pub use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

/// Samples in every baked curve and gradient.
pub const LUT: usize = 32;
/// Update modules one emitter runs. Past it the rest are ignored.
pub const MAX_MODULES: usize = 8;
/// Keys one curve or gradient keeps.
pub const MAX_KEYS: usize = 16;
/// Live particles one CPU pool holds.
pub const CPU_MAX: u32 = 4096;
/// Live particles one GPU emitter holds.
pub const GPU_MAX: u32 = 65536;
/// Points a ribbon remembers.
pub const MAX_RIBBON_POINTS: u32 = 64;

// ─── Curves ──────────────────────────────────────────────────────────────

/// One key of a [`Curve`]: `v` at life fraction `t`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurveKey {
    pub t: f32,
    pub v: f32,
}

/// A value over a particle's life (or along a ribbon), piecewise linear
/// between keys and flat past the ends. No keys reads as 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    #[serde(default)]
    pub keys: Vec<CurveKey>,
}

impl Default for Curve {
    fn default() -> Self {
        Curve::constant(1.0)
    }
}

impl Curve {
    pub fn constant(v: f32) -> Self {
        Curve {
            keys: vec![CurveKey { t: 0.0, v }],
        }
    }

    pub fn linear(from: f32, to: f32) -> Self {
        Curve {
            keys: vec![CurveKey { t: 0.0, v: from }, CurveKey { t: 1.0, v: to }],
        }
    }

    pub fn sample(&self, t: f32) -> f32 {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let Some(first) = self.keys.first() else {
            return 1.0;
        };
        if t <= first.t {
            return first.v;
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t <= b.t {
                let span = (b.t - a.t).max(1e-6);
                return a.v + (b.v - a.v) * ((t - a.t) / span);
            }
        }
        self.keys.last().map_or(1.0, |key| key.v)
    }

    /// [`LUT`] samples from `t = 0` to `t = 1`, which is what the GPU reads.
    pub fn bake(&self) -> [f32; LUT] {
        std::array::from_fn(|i| self.sample(i as f32 / (LUT - 1) as f32))
    }

    /// Sorted, finite, in range and at most [`MAX_KEYS`] long.
    pub fn normalize(&mut self) {
        self.keys
            .retain(|key| key.t.is_finite() && key.v.is_finite());
        for key in &mut self.keys {
            key.t = key.t.clamp(0.0, 1.0);
            key.v = key.v.clamp(-1.0e4, 1.0e4);
        }
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.keys.truncate(MAX_KEYS);
    }
}

/// One key of a [`Gradient`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientKey {
    pub t: f32,
    pub color: String,
    #[serde(default = "one")]
    pub alpha: f32,
}

fn one() -> f32 {
    1.0
}

/// A color over life, multiplied over the emitter's start-to-end tint. Mixed
/// in linear light. No keys reads as white.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Gradient {
    #[serde(default)]
    pub keys: Vec<GradientKey>,
}

impl Gradient {
    pub fn linear(from: &str, to: &str) -> Self {
        Gradient {
            keys: vec![
                GradientKey {
                    t: 0.0,
                    color: from.to_string(),
                    alpha: 1.0,
                },
                GradientKey {
                    t: 1.0,
                    color: to.to_string(),
                    alpha: 1.0,
                },
            ],
        }
    }

    /// Linear RGBA at `t`.
    pub fn sample(&self, t: f32) -> [f32; 4] {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let color = |key: &GradientKey| {
            let [r, g, b, _] = hex_to_linear(&key.color);
            [r, g, b, key.alpha]
        };
        let Some(first) = self.keys.first() else {
            return [1.0; 4];
        };
        if t <= first.t {
            return color(first);
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if t <= b.t {
                let f = (t - a.t) / (b.t - a.t).max(1e-6);
                let (a, b) = (color(a), color(b));
                return std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f);
            }
        }
        self.keys.last().map_or([1.0; 4], color)
    }

    pub fn bake(&self) -> [[f32; 4]; LUT] {
        std::array::from_fn(|i| self.sample(i as f32 / (LUT - 1) as f32))
    }

    pub fn normalize(&mut self) {
        self.keys
            .retain(|key| key.t.is_finite() && key.alpha.is_finite());
        for key in &mut self.keys {
            key.t = key.t.clamp(0.0, 1.0);
            key.alpha = key.alpha.clamp(0.0, 1.0);
        }
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.keys.truncate(MAX_KEYS);
    }
}

// ─── Spawn ───────────────────────────────────────────────────────────────

/// Where a particle is born, in the emitter's frame (turned with the actor,
/// not scaled by it).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "shape")]
pub enum SpawnShape {
    /// The emitter's own position.
    #[default]
    Point,
    /// In a ball, or on its skin with `surface`.
    Sphere {
        #[serde(default = "default_radius")]
        radius: f32,
        #[serde(default)]
        surface: bool,
    },
    /// In a box `size` across.
    Box {
        #[serde(default = "default_box")]
        size: [f32; 3],
    },
    /// On a disk of `radius` across the facing, spraying through the
    /// emitter's `spread`: a nozzle.
    Cone {
        #[serde(default = "default_radius")]
        radius: f32,
    },
    /// On the surface of the actor's own drawn mesh, area weighted. Falls
    /// back to a point where the actor draws no mesh (2D, or still loading).
    MeshSurface,
}

fn default_radius() -> f32 {
    1.0
}
fn default_box() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl SpawnShape {
    /// The kind number `vfx.wesl` switches on.
    pub fn code(&self) -> u32 {
        match self {
            SpawnShape::Point => 0,
            SpawnShape::Sphere { .. } => 1,
            SpawnShape::Box { .. } => 2,
            SpawnShape::Cone { .. } => 3,
            SpawnShape::MeshSurface => 4,
        }
    }

    /// The shape's numbers, as the GPU reads them.
    pub fn params(&self) -> [f32; 3] {
        match self {
            SpawnShape::Point | SpawnShape::MeshSurface => [0.0; 3],
            SpawnShape::Sphere { radius, surface } => {
                [*radius, if *surface { 1.0 } else { 0.0 }, 0.0]
            }
            SpawnShape::Box { size } => *size,
            SpawnShape::Cone { radius } => [*radius, 0.0, 0.0],
        }
    }

    fn normalize(&mut self) {
        match self {
            SpawnShape::Sphere { radius, .. } | SpawnShape::Cone { radius } => {
                *radius = finite_or(*radius, 1.0).clamp(0.0, 1.0e4);
            }
            SpawnShape::Box { size } => {
                *size = size.map(|s| finite_or(s, 1.0).clamp(0.0, 1.0e4));
            }
            _ => {}
        }
    }
}

/// Which way a newborn particle flies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LaunchDirection {
    /// Within the emitter's `spread` cone around the actor's facing.
    #[default]
    Facing,
    /// Away from the emitter's centre (or along the mesh's normal).
    Outward,
}

/// A burst on the emitter's own clock: `count` particles at `time` seconds
/// after the emitter starts, then every `interval` for `cycles` more (0
/// cycles repeats for as long as it plays).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Burst {
    #[serde(default)]
    pub time: f32,
    #[serde(default = "default_burst_count")]
    pub count: u32,
    #[serde(default = "one_cycle")]
    pub cycles: u32,
    #[serde(default = "one")]
    pub interval: f32,
}

fn default_burst_count() -> u32 {
    16
}
fn one_cycle() -> u32 {
    1
}

impl Burst {
    /// How many particles fire in the clock window `(from, to]`.
    pub fn due(&self, from: f64, to: f64) -> u32 {
        if to <= from {
            return 0;
        }
        let interval = (self.interval as f64).max(0.01);
        let start = self.time.max(0.0) as f64;
        // The k-th shot lands at start + k * interval.
        let first = ((from - start) / interval).floor() as i64 + 1;
        let first = if from < start { 0 } else { first.max(0) };
        let last = ((to - start) / interval).floor() as i64;
        if last < first || to < start {
            return 0;
        }
        let limit = if self.cycles == 0 {
            i64::MAX
        } else {
            self.cycles as i64 - 1
        };
        let last = last.min(limit);
        if last < first {
            return 0;
        }
        ((last - first + 1) as u64 * self.count as u64).min(u32::MAX as u64) as u32
    }
}

// ─── Update ──────────────────────────────────────────────────────────────

/// One step of the update stack, run in order on every live particle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "module")]
pub enum UpdateModule {
    /// A constant push, world units per second squared.
    Force {
        #[serde(default)]
        force: [f32; 3],
    },
    /// Loses `amount` of its velocity per second, exponentially.
    Drag {
        #[serde(default = "default_drag")]
        amount: f32,
    },
    /// A divergence-free swirl (the curl of a noise field): smoke that
    /// curls without bunching up.
    CurlNoise {
        #[serde(default = "default_noise_strength")]
        strength: f32,
        /// Noise features per world unit.
        #[serde(default = "default_noise_scale")]
        scale: f32,
        /// How fast the field itself drifts.
        #[serde(default = "default_noise_speed")]
        speed: f32,
    },
    /// Plain noise pushes: jittery, unlike the curl's swirl.
    Turbulence {
        #[serde(default = "default_noise_strength")]
        strength: f32,
        #[serde(default = "default_noise_scale")]
        scale: f32,
    },
    /// Pulls towards an actor (the emitter itself when empty), within
    /// `radius` (0 is everywhere), and kills what gets inside
    /// `kill_radius`. A negative strength pushes away.
    Attractor {
        #[serde(default)]
        target: String,
        #[serde(default = "default_attract")]
        strength: f32,
        #[serde(default)]
        radius: f32,
        #[serde(default)]
        kill_radius: f32,
    },
    /// Bounces off the world: the depth buffer on the GPU sim, bodies'
    /// colliders on the CPU pool. `lifetime_loss` of a life per hit.
    Collide {
        #[serde(default = "default_bounce")]
        bounce: f32,
        #[serde(default = "default_friction")]
        friction: f32,
        #[serde(default)]
        lifetime_loss: f32,
        /// Die on the first hit instead of bouncing.
        #[serde(default)]
        kill: bool,
    },
    /// Kills whatever crosses to the back of a plane through `point` facing
    /// `normal`, in world space.
    KillPlane {
        #[serde(default)]
        point: [f32; 3],
        #[serde(default = "default_up")]
        normal: [f32; 3],
    },
}

fn default_drag() -> f32 {
    1.0
}
fn default_noise_strength() -> f32 {
    2.0
}
fn default_noise_scale() -> f32 {
    0.5
}
fn default_noise_speed() -> f32 {
    0.3
}
fn default_attract() -> f32 {
    10.0
}
fn default_bounce() -> f32 {
    0.4
}
fn default_friction() -> f32 {
    0.2
}
fn default_up() -> [f32; 3] {
    [0.0, 1.0, 0.0]
}

impl UpdateModule {
    /// The kind number `vfx.wesl` switches on.
    pub fn code(&self) -> u32 {
        match self {
            UpdateModule::Force { .. } => 1,
            UpdateModule::Drag { .. } => 2,
            UpdateModule::CurlNoise { .. } => 3,
            UpdateModule::Turbulence { .. } => 4,
            UpdateModule::Attractor { .. } => 5,
            UpdateModule::Collide { .. } => 6,
            UpdateModule::KillPlane { .. } => 7,
        }
    }

    pub fn is_collide(&self) -> bool {
        matches!(self, UpdateModule::Collide { .. })
    }

    fn normalize(&mut self) {
        let f = |v: &mut f32, lo: f32, hi: f32, fallback: f32| {
            *v = finite_or(*v, fallback).clamp(lo, hi);
        };
        match self {
            UpdateModule::Force { force } => {
                for v in force {
                    f(v, -1.0e5, 1.0e5, 0.0);
                }
            }
            UpdateModule::Drag { amount } => f(amount, 0.0, 100.0, 1.0),
            UpdateModule::CurlNoise {
                strength,
                scale,
                speed,
            } => {
                f(strength, -1.0e4, 1.0e4, 2.0);
                f(scale, 1.0e-4, 100.0, 0.5);
                f(speed, 0.0, 100.0, 0.3);
            }
            UpdateModule::Turbulence { strength, scale } => {
                f(strength, -1.0e4, 1.0e4, 2.0);
                f(scale, 1.0e-4, 100.0, 0.5);
            }
            UpdateModule::Attractor {
                strength,
                radius,
                kill_radius,
                ..
            } => {
                f(strength, -1.0e5, 1.0e5, 10.0);
                f(radius, 0.0, 1.0e5, 0.0);
                f(kill_radius, 0.0, 1.0e5, 0.0);
            }
            UpdateModule::Collide {
                bounce,
                friction,
                lifetime_loss,
                ..
            } => {
                f(bounce, 0.0, 1.0, 0.4);
                f(friction, 0.0, 1.0, 0.2);
                f(lifetime_loss, 0.0, 1.0, 0.0);
            }
            UpdateModule::KillPlane { point, normal } => {
                for v in point.iter_mut() {
                    f(v, -1.0e6, 1.0e6, 0.0);
                }
                let n = Vec3::from_array(*normal);
                *normal = if n.is_finite() && n.length_squared() > 1e-8 {
                    n.normalize().to_array()
                } else {
                    default_up()
                };
            }
        }
    }
}

// ─── Render ──────────────────────────────────────────────────────────────

/// How a particle's color lands on what is behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ParticleBlend {
    /// Adds light: sparks, fire, magic. Order doesn't matter.
    #[default]
    Additive,
    /// Covers: smoke, dust, leaves.
    Alpha,
}

/// Which way a billboard faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ParticleFacing {
    /// Always square to the camera.
    #[default]
    Camera,
    /// Stretched along its velocity: rain, sparks, tracers.
    Velocity,
    /// Lying flat, facing up: ripples, ground dust.
    Flat,
}

/// A flipbook: an image cut into `columns` x `rows` frames, played left to
/// right, top to bottom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flipbook {
    #[serde(default)]
    pub image: String,
    #[serde(default = "one_cell")]
    pub columns: u32,
    #[serde(default = "one_cell")]
    pub rows: u32,
    /// Frames per second, or 0 to play the sheet once over the particle's
    /// life.
    #[serde(default)]
    pub fps: f32,
    /// Start each particle on a random frame.
    #[serde(default)]
    pub random_start: bool,
    /// Cross-fade between frames rather than cutting.
    #[serde(default = "yes")]
    pub blend: bool,
}

fn one_cell() -> u32 {
    1
}
fn yes() -> bool {
    true
}

impl Default for Flipbook {
    fn default() -> Self {
        Flipbook {
            image: String::new(),
            columns: 1,
            rows: 1,
            fps: 0.0,
            random_start: false,
            blend: true,
        }
    }
}

impl Flipbook {
    pub fn frames(&self) -> u32 {
        self.columns.max(1) * self.rows.max(1)
    }
}

/// How the particles draw, and what changes over their lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleRender {
    #[serde(default)]
    pub blend: ParticleBlend,
    #[serde(default)]
    pub facing: ParticleFacing,
    /// How long a velocity-facing particle stretches: seconds of travel.
    #[serde(default = "default_stretch")]
    pub stretch: f32,
    /// Lit by the scene's lights (3D) instead of glowing by itself.
    #[serde(default)]
    pub lit: bool,
    /// Fade out this close to what is behind, in world units, so a quad
    /// doesn't show a hard line where it cuts into the ground (3D). 0 is off.
    #[serde(default)]
    pub soft: f32,
    /// Brightness multiplier. Past 1 glows in the HDR frame and blooms.
    #[serde(default = "one")]
    pub intensity: f32,
    #[serde(default)]
    pub flipbook: Flipbook,
    /// Multiplies the start-to-end size over life.
    #[serde(default)]
    pub size: Curve,
    /// Multiplies the start-to-end tint over life.
    #[serde(default)]
    pub color: Gradient,
    /// Opacity over life. Defaults to fading out.
    #[serde(default = "fade_out")]
    pub alpha: Curve,
    /// Degrees added to the particle's turn over life.
    #[serde(default = "zero_curve")]
    pub rotation: Curve,
    /// Degrees a second each particle spins, with a random sign.
    #[serde(default)]
    pub spin: f32,
    /// Start each particle at a random turn.
    #[serde(default)]
    pub random_rotation: bool,
    /// 0-1: how much smaller than its size a particle may randomly be.
    #[serde(default)]
    pub size_random: f32,
    /// How much light from behind a lit particle lets through, like thin
    /// smoke. 0 is opaque to it.
    #[serde(default)]
    pub translucency: f32,
}

fn default_stretch() -> f32 {
    0.05
}
fn fade_out() -> Curve {
    Curve::linear(1.0, 0.0)
}
fn zero_curve() -> Curve {
    Curve::constant(0.0)
}

impl Default for ParticleRender {
    fn default() -> Self {
        ParticleRender {
            blend: ParticleBlend::default(),
            facing: ParticleFacing::default(),
            stretch: default_stretch(),
            lit: false,
            soft: 0.0,
            intensity: 1.0,
            flipbook: Flipbook::default(),
            size: Curve::default(),
            color: Gradient::default(),
            alpha: fade_out(),
            rotation: zero_curve(),
            spin: 0.0,
            random_rotation: false,
            size_random: 0.0,
            translucency: 0.0,
        }
    }
}

/// What a ribbon follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RibbonSource {
    /// Every particle drags its own ribbon.
    #[default]
    Particles,
    /// The emitting actor drags one: a sword swing, a comet.
    Actor,
}

/// Ribbons: a strip through the last `points` places each particle (or the
/// actor) has been.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RibbonSpec {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub source: RibbonSource,
    /// Points of history, head included.
    #[serde(default = "default_points")]
    pub points: u32,
    /// World units travelled before a new point is laid.
    #[serde(default = "default_step")]
    pub step: f32,
    /// Extra segments between two points, smoothed along a Catmull-Rom
    /// spline.
    #[serde(default = "default_tessellation")]
    pub tessellation: u32,
    /// Full width at the head, in world units.
    #[serde(default = "default_width")]
    pub width: f32,
    /// Width along the ribbon, head (0) to tail (1).
    #[serde(default = "taper")]
    pub width_curve: Curve,
    /// Color along the ribbon, head to tail, over the particle's own tint.
    #[serde(default)]
    pub color: Gradient,
    /// Twist every segment to face the camera; otherwise the strip lies
    /// across the world's up.
    #[serde(default = "yes")]
    pub face_camera: bool,
    /// Brightness multiplier: past 1 makes glow trails in the HDR frame.
    #[serde(default = "one")]
    pub intensity: f32,
    /// Draw the particles' billboards as well as their ribbons.
    #[serde(default = "yes")]
    pub heads: bool,
}

fn default_points() -> u32 {
    16
}
fn default_step() -> f32 {
    0.25
}
fn default_tessellation() -> u32 {
    2
}
fn default_width() -> f32 {
    0.2
}
fn taper() -> Curve {
    Curve::linear(1.0, 0.0)
}

impl Default for RibbonSpec {
    fn default() -> Self {
        RibbonSpec {
            enabled: false,
            source: RibbonSource::default(),
            points: default_points(),
            step: default_step(),
            tessellation: default_tessellation(),
            width: default_width(),
            width_curve: taper(),
            color: Gradient::default(),
            face_camera: true,
            intensity: 1.0,
            heads: true,
        }
    }
}

/// Where an emitter simulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SimMode {
    /// The GPU where the world can (3D with compute shaders), else the CPU.
    #[default]
    Auto,
    /// Always the CPU pool: deterministic, and collides with bodies.
    Cpu,
}

/// Project-wide particle settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VfxSettings {
    /// Live particles the whole world may hold. Emitters past it stop
    /// spawning until some die.
    #[serde(default = "default_budget")]
    pub budget: u32,
    /// Forces every emitter onto the CPU pool: headless and low-end targets.
    #[serde(default)]
    pub cpu_only: bool,
}

fn default_budget() -> u32 {
    200_000
}

impl Default for VfxSettings {
    fn default() -> Self {
        VfxSettings {
            budget: default_budget(),
            cpu_only: false,
        }
    }
}

impl VfxSettings {
    pub fn normalize(&mut self) {
        self.budget = self.budget.clamp(1, 4_000_000);
    }
}

/// What an actor's particles did, as the reporters and scripts read it: the
/// live count, how many spawned, died and collided this frame, and where the
/// last of each happened (GPU emitters report a frame or two late).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParticleSense {
    pub alive: u32,
    pub spawned: u32,
    pub died: u32,
    pub collided: u32,
    pub spawn_at: Option<[f32; 3]>,
    pub die_at: Option<[f32; 3]>,
    pub collide_at: Option<[f32; 3]>,
}

impl ParticleSense {
    /// This frame's count of one event.
    pub fn count(&self, event: ParticleEvent) -> u32 {
        match event {
            ParticleEvent::Spawn => self.spawned,
            ParticleEvent::Die => self.died,
            ParticleEvent::Collide => self.collided,
        }
    }

    /// Where the last one of an event happened, this run.
    pub fn at(&self, event: ParticleEvent) -> Option<[f32; 3]> {
        match event {
            ParticleEvent::Spawn => self.spawn_at,
            ParticleEvent::Die => self.die_at,
            ParticleEvent::Collide => self.collide_at,
        }
    }

    /// Folds one step's events in: counts replace, positions stick.
    pub fn record(&mut self, alive: u32, events: &StepEvents) {
        self.alive = alive;
        self.spawned = events.spawned;
        self.died = events.died;
        self.collided = events.collided;
        for (at, new) in [
            (&mut self.spawn_at, events.spawn_at),
            (&mut self.die_at, events.die_at),
            (&mut self.collide_at, events.collide_at),
        ] {
            if new.is_some() {
                *at = new;
            }
        }
    }
}

/// Which particle event a `when my particles` hat waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ParticleEvent {
    #[default]
    Spawn,
    Die,
    Collide,
}

impl ParticleEvent {
    pub const ALL: [ParticleEvent; 3] = [
        ParticleEvent::Spawn,
        ParticleEvent::Die,
        ParticleEvent::Collide,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ParticleEvent::Spawn => "Spawn",
            ParticleEvent::Die => "Die",
            ParticleEvent::Collide => "Collide",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|event| event.name().eq_ignore_ascii_case(name.trim()))
    }
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// Clamp the graph half of an emitter into its live ranges.
pub fn normalize(spec: &mut ParticleSpec) {
    spec.shape.normalize();
    spec.bursts.truncate(16);
    for burst in &mut spec.bursts {
        burst.time = finite_or(burst.time, 0.0).clamp(0.0, 3600.0);
        burst.interval = finite_or(burst.interval, 1.0).clamp(0.01, 3600.0);
        burst.count = burst.count.min(GPU_MAX);
    }
    spec.modules.truncate(MAX_MODULES);
    for module in &mut spec.modules {
        module.normalize();
    }
    let render = &mut spec.render;
    render.stretch = finite_or(render.stretch, 0.05).clamp(0.0, 10.0);
    render.soft = finite_or(render.soft, 0.0).clamp(0.0, 100.0);
    render.intensity = finite_or(render.intensity, 1.0).clamp(0.0, 1000.0);
    render.spin = finite_or(render.spin, 0.0).clamp(-7200.0, 7200.0);
    render.size_random = finite_or(render.size_random, 0.0).clamp(0.0, 1.0);
    render.translucency = finite_or(render.translucency, 0.0).clamp(0.0, 4.0);
    render.flipbook.columns = render.flipbook.columns.clamp(1, 64);
    render.flipbook.rows = render.flipbook.rows.clamp(1, 64);
    render.flipbook.fps = finite_or(render.flipbook.fps, 0.0).clamp(0.0, 240.0);
    render.size.normalize();
    render.color.normalize();
    render.alpha.normalize();
    render.rotation.normalize();
    let ribbon = &mut spec.ribbon;
    ribbon.points = ribbon.points.clamp(2, MAX_RIBBON_POINTS);
    ribbon.step = finite_or(ribbon.step, 0.25).clamp(1e-3, 1.0e4);
    ribbon.tessellation = ribbon.tessellation.min(8);
    ribbon.width = finite_or(ribbon.width, 0.2).clamp(0.0, 1.0e4);
    ribbon.intensity = finite_or(ribbon.intensity, 1.0).clamp(0.0, 1000.0);
    ribbon.width_curve.normalize();
    ribbon.color.normalize();
    spec.duration = finite_or(spec.duration, 2.0).clamp(0.1, 3600.0);
}

// ─── The shared particle ─────────────────────────────────────────────────

/// Flag: the slot is the actor's own ribbon anchor, not a particle.
pub const FLAG_PINNED: u32 = 1;

/// One particle slot, byte for byte `Particle` in `vfx.wesl`. `life <= 0`
/// is an empty slot.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Particle {
    pub position: [f32; 3],
    pub age: f32,
    pub velocity: [f32; 3],
    pub life: f32,
    /// Degrees.
    pub rotation: f32,
    /// Degrees a second.
    pub spin: f32,
    /// Size factor from `size_random`.
    pub size: f32,
    pub seed: u32,
    /// Newest ribbon point in the low 16 bits, points held in the high 16.
    pub ribbon: u32,
    pub flags: u32,
    /// Times it has hit something.
    pub hits: u32,
    /// First flipbook frame.
    pub frame: f32,
}

impl Particle {
    pub const BYTES: usize = 64;

    pub fn alive(&self) -> bool {
        self.life > 0.0
    }

    pub fn to_bytes(particles: &[Particle], out: &mut Vec<u8>) {
        out.clear();
        out.reserve(particles.len() * Self::BYTES);
        for p in particles {
            for v in p.position {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&p.age.to_le_bytes());
            for v in p.velocity {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&p.life.to_le_bytes());
            out.extend_from_slice(&p.rotation.to_le_bytes());
            out.extend_from_slice(&p.spin.to_le_bytes());
            out.extend_from_slice(&p.size.to_le_bytes());
            out.extend_from_slice(&p.seed.to_le_bytes());
            out.extend_from_slice(&p.ribbon.to_le_bytes());
            out.extend_from_slice(&p.flags.to_le_bytes());
            out.extend_from_slice(&p.hits.to_le_bytes());
            out.extend_from_slice(&p.frame.to_le_bytes());
        }
    }
}

/// Bytes one ribbon point takes: position and the time it was laid.
pub const RIBBON_POINT_BYTES: usize = 16;

// ─── Randomness and noise, as `blockloom::hash` and `noise` spell them ───

pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277803737);
    (word >> 22) ^ word
}

/// The top 24 bits as a float in [0, 1).
pub fn unit(x: u32) -> f32 {
    (x >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Next float in [0, 1) off a stream.
pub fn rand(state: &mut u32) -> f32 {
    *state = pcg(*state);
    unit(*state)
}

fn pcg3(v: [u32; 3]) -> [u32; 3] {
    let mut h = v.map(|x| x.wrapping_mul(1664525).wrapping_add(1013904223));
    h[0] = h[0].wrapping_add(h[1].wrapping_mul(h[2]));
    h[1] = h[1].wrapping_add(h[2].wrapping_mul(h[0]));
    h[2] = h[2].wrapping_add(h[0].wrapping_mul(h[1]));
    h = h.map(|x| x ^ (x >> 16));
    h[0] = h[0].wrapping_add(h[1].wrapping_mul(h[2]));
    h[1] = h[1].wrapping_add(h[2].wrapping_mul(h[0]));
    h[2] = h[2].wrapping_add(h[0].wrapping_mul(h[1]));
    h
}

fn gradient_of3(cell: [i32; 3]) -> Vec3 {
    let h = pcg3(cell.map(|c| c as u32));
    let z = unit(h[0]) * 2.0 - 1.0;
    let angle = unit(h[1]) * std::f32::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(r * angle.cos(), r * angle.sin(), z)
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Gradient noise, roughly in [-1, 1]: `blockloom::noise::gradient3`.
pub fn gradient3(p: Vec3) -> f32 {
    let cell = p.floor();
    let t = p - cell;
    let cell = [cell.x as i32, cell.y as i32, cell.z as i32];
    let mut corners = [0.0f32; 8];
    for (i, corner) in corners.iter_mut().enumerate() {
        let o = [(i & 1) as i32, ((i >> 1) & 1) as i32, ((i >> 2) & 1) as i32];
        let g = gradient_of3([cell[0] + o[0], cell[1] + o[1], cell[2] + o[2]]);
        *corner = g.dot(t - Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32));
    }
    let (fx, fy, fz) = (fade(t.x), fade(t.y), fade(t.z));
    let lerp = |a: f32, b: f32, f: f32| a + (b - a) * f;
    let near = lerp(
        lerp(corners[0], corners[1], fx),
        lerp(corners[2], corners[3], fx),
        fy,
    );
    let far = lerp(
        lerp(corners[4], corners[5], fx),
        lerp(corners[6], corners[7], fx),
        fy,
    );
    lerp(near, far, fz) * 1.154_700_5
}

/// Three decorrelated noise channels.
fn noise3(p: Vec3) -> Vec3 {
    Vec3::new(
        gradient3(p),
        gradient3(p + Vec3::new(31.4, -17.2, 9.7)),
        gradient3(p + Vec3::new(-8.3, 23.9, 41.1)),
    )
}

/// The curl of a noise potential: `vfx.wesl`'s `curl`. Flat worlds take the
/// curl of one channel, which stays in the plane.
pub fn curl(p: Vec3, flat: bool) -> Vec3 {
    let e = 0.05;
    if flat {
        let f = |q: Vec3| gradient3(q);
        let dx = (f(p + Vec3::X * e) - f(p - Vec3::X * e)) / (2.0 * e);
        let dy = (f(p + Vec3::Y * e) - f(p - Vec3::Y * e)) / (2.0 * e);
        return Vec3::new(dy, -dx, 0.0);
    }
    let dx = (noise3(p + Vec3::X * e) - noise3(p - Vec3::X * e)) / (2.0 * e);
    let dy = (noise3(p + Vec3::Y * e) - noise3(p - Vec3::Y * e)) / (2.0 * e);
    let dz = (noise3(p + Vec3::Z * e) - noise3(p - Vec3::Z * e)) / (2.0 * e);
    Vec3::new(dy.z - dz.y, dz.x - dx.z, dx.y - dy.x)
}

// ─── The CPU pool ────────────────────────────────────────────────────────

/// A triangle of the actor's mesh, in the emitter's frame, for
/// `MeshSurface`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
}

impl Triangle {
    pub fn area(&self) -> f32 {
        (self.b - self.a).cross(self.c - self.a).length() * 0.5
    }

    pub fn normal(&self) -> Vec3 {
        (self.b - self.a)
            .cross(self.c - self.a)
            .try_normalize()
            .unwrap_or(Vec3::Y)
    }
}

/// A mesh's triangles with their running area, for area-weighted picks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Surface {
    pub triangles: Vec<Triangle>,
    /// Running total of area up to and including each triangle.
    pub cdf: Vec<f32>,
}

impl Surface {
    /// At most this many triangles are kept; a denser mesh is thinned evenly.
    pub const MAX_TRIANGLES: usize = 4096;

    pub fn new(mut triangles: Vec<Triangle>) -> Self {
        if triangles.len() > Self::MAX_TRIANGLES {
            let step = triangles.len() as f32 / Self::MAX_TRIANGLES as f32;
            triangles = (0..Self::MAX_TRIANGLES)
                .map(|i| triangles[(i as f32 * step) as usize])
                .collect();
        }
        let mut total = 0.0;
        let cdf = triangles
            .iter()
            .map(|t| {
                total += t.area();
                total
            })
            .collect();
        Surface { triangles, cdf }
    }

    /// A point and its normal, area weighted, off two uniforms and a third
    /// for the barycentrics.
    pub fn pick(&self, u: f32, v: f32, w: f32) -> Option<(Vec3, Vec3)> {
        let total = *self.cdf.last()?;
        if total <= 0.0 {
            return None;
        }
        let want = u * total;
        let index = self
            .cdf
            .partition_point(|&c| c < want)
            .min(self.cdf.len() - 1);
        let t = self.triangles[index];
        let (mut s, mut r) = (v, w);
        if s + r > 1.0 {
            s = 1.0 - s;
            r = 1.0 - r;
        }
        Some((t.a + (t.b - t.a) * s + (t.c - t.a) * r, t.normal()))
    }
}

/// A particle hitting something: where it crossed, and the surface's
/// normal there.
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
}

/// What one pool step needs from the world.
pub struct StepInput<'a> {
    pub dt: f32,
    /// The pool's own clock, which the noise fields drift on.
    pub time: f32,
    /// The emitter's pose: turn and position, no scale.
    pub emitter: Mat4,
    /// The world's gravity, before the emitter's scale.
    pub gravity: Vec3,
    pub flat: bool,
    /// Particles to start this step (rate plus bursts), before the pool cap.
    pub spawn: u32,
    /// Each module's attractor target, where it has one (the same order as
    /// the spec's modules).
    pub targets: &'a [Option<Vec3>],
    pub surface: Option<&'a Surface>,
    /// The air at a point, already scaled by nothing: the pool applies the
    /// emitter's `wind`.
    pub wind: &'a dyn Fn(Vec3) -> Vec3,
    /// A segment against the world, for `Collide`.
    pub collide: &'a dyn Fn(Vec3, Vec3) -> Option<Hit>,
}

/// What one pool step reports.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StepEvents {
    pub spawned: u32,
    pub died: u32,
    pub collided: u32,
    pub spawn_at: Option<[f32; 3]>,
    pub die_at: Option<[f32; 3]>,
    pub collide_at: Option<[f32; 3]>,
}

impl StepEvents {
    pub fn count(&self, event: ParticleEvent) -> u32 {
        match event {
            ParticleEvent::Spawn => self.spawned,
            ParticleEvent::Die => self.died,
            ParticleEvent::Collide => self.collided,
        }
    }

    pub fn at(&self, event: ParticleEvent) -> Option<[f32; 3]> {
        match event {
            ParticleEvent::Spawn => self.spawn_at,
            ParticleEvent::Die => self.die_at,
            ParticleEvent::Collide => self.collide_at,
        }
    }
}

/// Seconds the wind takes to mostly carry a new particle along.
const WIND_CATCH: f32 = 0.25;

/// The CPU half of the VFX graph: a fixed ring of slots, stepped in order,
/// deterministic for a seed. Ribbons live beside the slots in `trail`.
#[derive(Debug, Clone)]
pub struct Pool {
    pub particles: Vec<Particle>,
    /// `points` ribbon points per slot: position and the time laid.
    pub trail: Vec<[f32; 4]>,
    pub points: u32,
    seed: u32,
    /// Next slot to look in for a free one.
    cursor: usize,
    /// The air each particle has picked up, beside its slot.
    air: Vec<Vec3>,
}

impl Pool {
    pub fn new(capacity: u32, points: u32, seed: u32) -> Self {
        let capacity = capacity.clamp(1, CPU_MAX) as usize;
        let points = points.clamp(2, MAX_RIBBON_POINTS);
        Pool {
            particles: vec![Particle::default(); capacity],
            trail: vec![[0.0; 4]; capacity * points as usize],
            points,
            seed: pcg(seed ^ 0x9E37_79B9),
            cursor: 0,
            air: vec![Vec3::ZERO; capacity],
        }
    }

    pub fn capacity(&self) -> u32 {
        self.particles.len() as u32
    }

    pub fn alive(&self) -> u32 {
        self.particles
            .iter()
            .filter(|p| p.alive() && p.flags & FLAG_PINNED == 0)
            .count() as u32
    }

    pub fn clear(&mut self) {
        self.particles.fill(Particle::default());
        self.cursor = 0;
    }

    /// Keep slot 0 as the actor's ribbon anchor, or give it back.
    fn pin(&mut self, on: bool, input: &StepInput) {
        let at = input.emitter.transform_point3(Vec3::ZERO);
        let slot = &mut self.particles[0];
        if on {
            if slot.flags & FLAG_PINNED == 0 {
                *slot = Particle {
                    position: at.to_array(),
                    life: f32::MAX,
                    size: 1.0,
                    flags: FLAG_PINNED,
                    ..Default::default()
                };
                let base = 0;
                self.trail[base] = [at.x, at.y, at.z, input.time];
                slot.ribbon = 1 << 16;
            }
            slot.velocity =
                ((at - Vec3::from_array(slot.position)) / input.dt.max(1e-4)).to_array();
            slot.position = at.to_array();
        } else if slot.flags & FLAG_PINNED != 0 {
            *slot = Particle::default();
        }
    }

    /// Advance every particle one step, then start `input.spawn` new ones.
    pub fn step(&mut self, spec: &ParticleSpec, input: &StepInput) -> StepEvents {
        let mut events = StepEvents::default();
        let dt = input.dt.max(0.0);
        let pinned = spec.ribbon.enabled && spec.ribbon.source == RibbonSource::Actor;
        self.pin(pinned, input);
        let emitter_at = input.emitter.transform_point3(Vec3::ZERO);
        let points = self.points as usize;
        let ribbons = spec.ribbon.enabled;
        let modules: Vec<&UpdateModule> = spec.modules.iter().take(MAX_MODULES).collect();
        for i in 0..self.particles.len() {
            let mut p = self.particles[i];
            if !p.alive() {
                continue;
            }
            if p.flags & FLAG_PINNED != 0 {
                if ribbons {
                    self.lay_point(i, &mut p, spec.ribbon.step, input.time);
                }
                self.particles[i] = p;
                continue;
            }
            p.age += dt;
            let mut dead = p.age >= p.life;
            let mut pos = Vec3::from_array(p.position);
            let mut vel = Vec3::from_array(p.velocity);
            if !dead {
                vel += input.gravity * spec.gravity_scale * dt;
                if spec.wind > 0.0 {
                    let carried = (input.wind)(pos) * spec.wind;
                    let catch = (dt / WIND_CATCH).min(1.0);
                    self.air[i] = self.air[i].lerp(carried, catch);
                }
                let mut hit = None;
                for (index, module) in modules.iter().enumerate() {
                    match module {
                        UpdateModule::Force { force } => vel += Vec3::from_array(*force) * dt,
                        UpdateModule::Drag { amount } => vel *= (-amount * dt).exp(),
                        UpdateModule::CurlNoise {
                            strength,
                            scale,
                            speed,
                        } => {
                            let q = pos * *scale + Vec3::splat(input.time * speed);
                            vel += curl(q, input.flat) * *strength * dt;
                        }
                        UpdateModule::Turbulence { strength, scale } => {
                            let q = pos * *scale + Vec3::splat(p.age * 1.7);
                            let mut push = noise3(q + Vec3::splat((p.seed & 1023) as f32));
                            if input.flat {
                                push.z = 0.0;
                            }
                            vel += push * *strength * dt;
                        }
                        UpdateModule::Attractor {
                            strength,
                            radius,
                            kill_radius,
                            ..
                        } => {
                            let target = input
                                .targets
                                .get(index)
                                .copied()
                                .flatten()
                                .unwrap_or(emitter_at);
                            let to = target - pos;
                            let d = to.length();
                            if d < *kill_radius {
                                dead = true;
                            } else if *radius <= 0.0 || d < *radius {
                                vel += to.normalize_or_zero() * *strength * dt;
                            }
                        }
                        UpdateModule::Collide { .. } => hit = Some(*module),
                        UpdateModule::KillPlane { .. } => {}
                    }
                }
                let next = pos + (vel + self.air[i]) * dt;
                if let Some(UpdateModule::Collide {
                    bounce,
                    friction,
                    lifetime_loss,
                    kill,
                }) = hit
                    && let Some(contact) = (input.collide)(pos, next)
                {
                    p.hits += 1;
                    events.collided += 1;
                    events.collide_at = Some(contact.point.to_array());
                    let n = contact.normal;
                    let vn = n * vel.dot(n);
                    let vt = vel - vn;
                    vel = vt * (1.0 - friction) - vn * *bounce;
                    pos = contact.point + n * 1e-3;
                    p.age += p.life * lifetime_loss;
                    if *kill || p.age >= p.life {
                        dead = true;
                    }
                } else {
                    pos = next;
                }
                for module in &modules {
                    if let UpdateModule::KillPlane { point, normal } = module
                        && (pos - Vec3::from_array(*point)).dot(Vec3::from_array(*normal)) < 0.0
                    {
                        dead = true;
                    }
                }
            }
            if dead {
                events.died += 1;
                events.die_at = Some(pos.to_array());
                p.life = 0.0;
            } else {
                p.position = pos.to_array();
                p.velocity = vel.to_array();
                p.rotation += p.spin * dt;
                if ribbons {
                    self.lay_point(i, &mut p, spec.ribbon.step, input.time);
                }
            }
            self.particles[i] = p;
        }
        debug_assert_eq!(self.trail.len(), self.particles.len() * points);
        self.spawn(spec, input, &mut events);
        events
    }

    /// Keep the head point on the particle, and lay it down once it has
    /// moved a step from the one before.
    fn lay_point(&mut self, i: usize, p: &mut Particle, step: f32, time: f32) {
        let points = self.points;
        let base = i * points as usize;
        let head = p.ribbon & 0xFFFF;
        let held = (p.ribbon >> 16).max(1);
        let pos = Vec3::from_array(p.position);
        let prev = (head + points - 1) % points;
        let before = self.trail[base + prev as usize];
        let moved = held > 1 && pos.distance(Vec3::new(before[0], before[1], before[2])) >= step;
        if held == 1 || moved {
            // The head point becomes fixed; a new head starts on the particle.
            let next = (head + 1) % points;
            self.trail[base + next as usize] = [pos.x, pos.y, pos.z, time];
            p.ribbon = next | ((held + 1).min(points) << 16);
        } else {
            self.trail[base + head as usize] = [pos.x, pos.y, pos.z, time];
        }
    }

    fn spawn(&mut self, spec: &ParticleSpec, input: &StepInput, events: &mut StepEvents) {
        let capacity = self.particles.len();
        let cap = (spec.max as usize).min(capacity);
        let mut alive = self.particles.iter().filter(|p| p.alive()).count();
        let mut want = input.spawn as usize;
        let mut looked = 0;
        while want > 0 && alive < cap && looked < capacity {
            let i = self.cursor % capacity;
            self.cursor = (self.cursor + 1) % capacity;
            looked += 1;
            if self.particles[i].alive() {
                continue;
            }
            let p = self.born(spec, input);
            let at = p.position;
            let base = i * self.points as usize;
            self.trail[base] = [at[0], at[1], at[2], input.time];
            self.particles[i] = Particle {
                ribbon: 1 << 16,
                ..p
            };
            self.air[i] = Vec3::ZERO;
            events.spawned += 1;
            events.spawn_at = Some(at);
            alive += 1;
            want -= 1;
        }
    }

    fn born(&mut self, spec: &ParticleSpec, input: &StepInput) -> Particle {
        let seed = &mut self.seed;
        let particle_seed = pcg(*seed ^ 0xB5AD_4ECE);
        let draw = spawn_point(
            spec,
            input.flat,
            input.surface,
            [rand(seed), rand(seed), rand(seed), rand(seed)],
        );
        let (local, outward) = draw;
        let dir_local = launch_direction(spec, input.flat, outward, [rand(seed), rand(seed)]);
        let turn = glam::Mat3::from_mat4(input.emitter);
        let position = input.emitter.transform_point3(local);
        let dir = (turn * dir_local).normalize_or_zero();
        let speed = spec.speed * (0.5 + rand(seed));
        let life = spec.lifetime * (0.6 + 0.8 * rand(seed));
        let render = &spec.render;
        let rotation = if render.random_rotation {
            rand(seed) * 360.0
        } else {
            0.0
        };
        let spin = render.spin * if rand(seed) < 0.5 { -1.0 } else { 1.0 };
        let size = 1.0 - render.size_random * rand(seed);
        let frames = render.flipbook.frames();
        let frame = if render.flipbook.random_start {
            (rand(seed) * frames as f32).floor()
        } else {
            0.0
        };
        Particle {
            position: position.to_array(),
            age: 0.0,
            velocity: (dir * speed).to_array(),
            life: life.max(1e-3),
            rotation,
            spin,
            size,
            seed: particle_seed,
            ribbon: 0,
            flags: 0,
            hits: 0,
            frame,
        }
    }

    /// Screens' worth of particle area over a `pixels`-tall view `fov_y`
    /// radians wide, from `eye`: 1.0 is every pixel drawn once. Perspective
    /// only; `ortho` gives the view's height in world units instead.
    pub fn overdraw(&self, spec: &ParticleSpec, eye: Vec3, view: ViewSize) -> f32 {
        let mut total = 0.0;
        let size_lut = spec.render.size.bake();
        for p in &self.particles {
            if !p.alive() || p.flags & FLAG_PINNED != 0 {
                continue;
            }
            let t = (p.age / p.life).clamp(0.0, 1.0);
            let size = particle_size(spec, &size_lut, t) * p.size;
            total += view.coverage(size, Vec3::from_array(p.position).distance(eye));
        }
        total
    }
}

/// How big a view is, for [`Pool::overdraw`].
#[derive(Debug, Clone, Copy)]
pub enum ViewSize {
    Perspective { fov_y: f32, aspect: f32 },
    Orthographic { height: f32, aspect: f32 },
}

impl ViewSize {
    /// The fraction of the screen a `size`-wide square covers at `distance`.
    pub fn coverage(self, size: f32, distance: f32) -> f32 {
        match self {
            ViewSize::Perspective { fov_y, aspect } => {
                let height = 2.0 * distance.max(1e-3) * (fov_y * 0.5).tan();
                let side = (size / height.max(1e-6)).min(4.0);
                side * side / aspect.max(1e-3)
            }
            ViewSize::Orthographic { height, aspect } => {
                let side = (size / height.max(1e-6)).min(4.0);
                side * side / aspect.max(1e-3)
            }
        }
    }
}

/// A particle's full size at life fraction `t`, as the renderer draws it.
pub fn particle_size(spec: &ParticleSpec, size_lut: &[f32; LUT], t: f32) -> f32 {
    let base = spec.size_start + (spec.size_end - spec.size_start) * t;
    let i = ((t.clamp(0.0, 1.0) * (LUT - 1) as f32).round() as usize).min(LUT - 1);
    (base * size_lut[i]).max(0.0)
}

/// A newborn's position in the emitter's frame, and the outward direction
/// there. `u` is four uniforms.
pub fn spawn_point(
    spec: &ParticleSpec,
    flat: bool,
    surface: Option<&Surface>,
    u: [f32; 4],
) -> (Vec3, Vec3) {
    let ball = |u: [f32; 4]| {
        if flat {
            let angle = u[0] * std::f32::consts::TAU;
            Vec3::new(angle.cos(), angle.sin(), 0.0)
        } else {
            let z = u[0] * 2.0 - 1.0;
            let angle = u[1] * std::f32::consts::TAU;
            let r = (1.0 - z * z).max(0.0).sqrt();
            Vec3::new(r * angle.cos(), z, r * angle.sin())
        }
    };
    match &spec.shape {
        SpawnShape::Point => (Vec3::ZERO, ball(u)),
        SpawnShape::Sphere { radius, surface } => {
            let dir = ball(u);
            let reach = if *surface {
                1.0
            } else if flat {
                u[2].sqrt()
            } else {
                u[2].cbrt()
            };
            (dir * *radius * reach, dir)
        }
        SpawnShape::Box { size } => {
            let mut p = Vec3::new(u[0] - 0.5, u[1] - 0.5, u[2] - 0.5) * Vec3::from_array(*size);
            if flat {
                p.z = 0.0;
            }
            (p, p.normalize_or(Vec3::Y))
        }
        SpawnShape::Cone { radius } => {
            // A disk across the facing: +x in 2D's plane, +z's in 3D.
            let angle = u[0] * std::f32::consts::TAU;
            let r = u[1].sqrt() * *radius;
            let p = if flat {
                Vec3::new(0.0, (u[0] * 2.0 - 1.0) * *radius, 0.0)
            } else {
                Vec3::new(angle.cos() * r, angle.sin() * r, 0.0)
            };
            (p, p.normalize_or(Vec3::Y))
        }
        SpawnShape::MeshSurface => surface
            .and_then(|s| s.pick(u[0], u[1], u[2]))
            .unwrap_or((Vec3::ZERO, ball(u))),
    }
}

/// A launch direction in the emitter's frame: the actor faces +x in 2D and
/// -z in 3D, the same as `forward_of`.
pub fn launch_direction(spec: &ParticleSpec, flat: bool, outward: Vec3, u: [f32; 2]) -> Vec3 {
    if spec.direction == LaunchDirection::Outward {
        return outward.normalize_or(if flat { Vec3::X } else { Vec3::NEG_Z });
    }
    let half = (spec.spread.to_radians() * 0.5).clamp(0.0, std::f32::consts::PI);
    if flat {
        let jitter = (u[0] * 2.0 - 1.0) * half;
        return Vec3::new(jitter.cos(), jitter.sin(), 0.0);
    }
    // Uniform over the spherical cap around -z.
    let cos_t = 1.0 - u[0] * (1.0 - half.cos());
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let phi = u[1] * std::f32::consts::TAU;
    Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), -cos_t)
}

/// Particles the emitter's rate and bursts start over one step, carrying the
/// fractional remainder in `acc`. `clock` is the emitter's own time before
/// the step.
pub fn due(spec: &ParticleSpec, acc: &mut f32, clock: f64, dt: f32) -> u32 {
    *acc += spec.rate * dt.max(0.0);
    let whole = acc.floor().max(0.0);
    *acc -= whole;
    let mut count = whole as u32;
    let to = clock + dt.max(0.0) as f64;
    for burst in &spec.bursts {
        count = count.saturating_add(burst.due(clock, to));
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calm(_: Vec3) -> Vec3 {
        Vec3::ZERO
    }
    fn nothing(_: Vec3, _: Vec3) -> Option<Hit> {
        None
    }

    fn input<'a>(spawn: u32, dt: f32) -> StepInput<'a> {
        StepInput {
            dt,
            time: 0.0,
            emitter: Mat4::IDENTITY,
            gravity: Vec3::new(0.0, -10.0, 0.0),
            flat: false,
            spawn,
            targets: &[],
            surface: None,
            wind: &calm,
            collide: &nothing,
        }
    }

    #[test]
    fn curves_interpolate_and_hold_past_their_ends() {
        let curve = Curve {
            keys: vec![CurveKey { t: 0.2, v: 1.0 }, CurveKey { t: 0.6, v: 3.0 }],
        };
        assert_eq!(curve.sample(0.0), 1.0);
        assert_eq!(curve.sample(0.4), 2.0);
        assert_eq!(curve.sample(1.0), 3.0);
        assert_eq!(Curve { keys: vec![] }.sample(0.5), 1.0);
        let lut = Curve::linear(0.0, 1.0).bake();
        assert_eq!(lut[0], 0.0);
        assert_eq!(lut[LUT - 1], 1.0);
    }

    #[test]
    fn gradients_mix_in_linear_light() {
        let gradient = Gradient::linear("#000000", "#FFFFFF");
        let mid = gradient.sample(0.5);
        assert!((mid[0] - 0.5).abs() < 1e-5);
        assert_eq!(Gradient::default().sample(0.3), [1.0; 4]);
    }

    #[test]
    fn a_burst_fires_once_per_cycle() {
        let burst = Burst {
            time: 0.5,
            count: 10,
            cycles: 3,
            interval: 1.0,
        };
        assert_eq!(burst.due(0.0, 0.4), 0);
        assert_eq!(burst.due(0.4, 0.6), 10);
        assert_eq!(burst.due(0.6, 1.4), 0);
        assert_eq!(burst.due(0.0, 10.0), 30);
        assert_eq!(burst.due(2.4, 2.6), 10);
        assert_eq!(burst.due(3.4, 3.6), 0);
        let forever = Burst {
            cycles: 0,
            ..burst.clone()
        };
        assert_eq!(forever.due(99.4, 99.6), 10);
    }

    #[test]
    fn rate_and_bursts_add_up() {
        let spec = ParticleSpec {
            rate: 10.0,
            bursts: vec![Burst {
                time: 0.0,
                count: 5,
                cycles: 1,
                interval: 1.0,
            }],
            ..ParticleSpec::default()
        };
        let mut acc = 0.0;
        // The burst at t = 0 lands in the first window only if it is open at 0.
        assert_eq!(due(&spec, &mut acc, -0.01, 0.25), 2 + 5);
        // Half a particle carried over makes the next window three.
        assert_eq!(due(&spec, &mut acc, 0.24, 0.25), 3);
        assert!(acc.abs() < 1e-5);
    }

    #[test]
    fn the_pool_spawns_up_to_its_cap_and_reaps_the_dead() {
        let spec = ParticleSpec {
            max: 8,
            lifetime: 0.1,
            ..ParticleSpec::default()
        };
        let mut pool = Pool::new(16, 4, 1);
        let events = pool.step(&spec, &input(20, 0.0));
        assert_eq!(events.spawned, 8);
        assert_eq!(pool.alive(), 8);
        let events = pool.step(&spec, &input(0, 1.0));
        assert_eq!(events.died, 8);
        assert_eq!(pool.alive(), 0);
    }

    #[test]
    fn the_same_seed_sprays_the_same() {
        let spec = ParticleSpec::default();
        let run = |seed| {
            let mut pool = Pool::new(64, 4, seed);
            pool.step(&spec, &input(32, 0.016));
            pool.step(&spec, &input(0, 0.016));
            pool.particles.clone()
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
    }

    #[test]
    fn drag_slows_and_a_kill_plane_kills() {
        let mut spec = ParticleSpec {
            gravity_scale: 0.0,
            lifetime: 10.0,
            speed: 10.0,
            spread: 0.0,
            modules: vec![UpdateModule::Drag { amount: 2.0 }],
            ..ParticleSpec::default()
        };
        let mut pool = Pool::new(4, 4, 3);
        pool.step(&spec, &input(1, 0.0));
        let before = Vec3::from_array(pool.particles[0].velocity).length();
        pool.step(&spec, &input(0, 0.5));
        let after = Vec3::from_array(pool.particles[0].velocity).length();
        assert!((after / before - (-1.0f32).exp()).abs() < 1e-4);
        // Launched along -z: a plane at z = -1 facing +z is crossed.
        spec.modules = vec![UpdateModule::KillPlane {
            point: [0.0, 0.0, -1.0],
            normal: [0.0, 0.0, 1.0],
        }];
        let events = pool.step(&spec, &input(0, 1.0));
        assert_eq!(events.died, 1);
    }

    #[test]
    fn a_collider_bounces_a_particle_and_reports_it() {
        let spec = ParticleSpec {
            gravity_scale: 1.0,
            lifetime: 10.0,
            speed: 0.0,
            modules: vec![UpdateModule::Collide {
                bounce: 0.5,
                friction: 0.0,
                lifetime_loss: 0.0,
                kill: false,
            }],
            ..ParticleSpec::default()
        };
        let ground = |from: Vec3, to: Vec3| {
            (to.y < -1.0 && from.y >= -1.0).then(|| Hit {
                point: Vec3::new(to.x, -1.0, to.z),
                normal: Vec3::Y,
            })
        };
        let mut pool = Pool::new(4, 4, 5);
        pool.step(&spec, &input(1, 0.0));
        let mut hits = 0;
        for _ in 0..60 {
            let step = StepInput {
                collide: &ground,
                ..input(0, 1.0 / 30.0)
            };
            hits += pool.step(&spec, &step).collided;
        }
        assert!(hits >= 1);
        assert!(pool.particles[0].position[1] >= -1.0);
    }

    #[test]
    fn an_attractor_pulls_and_swallows() {
        let spec = ParticleSpec {
            gravity_scale: 0.0,
            speed: 0.0,
            lifetime: 10.0,
            shape: SpawnShape::Sphere {
                radius: 5.0,
                surface: true,
            },
            modules: vec![UpdateModule::Attractor {
                target: String::new(),
                strength: 50.0,
                radius: 0.0,
                kill_radius: 0.5,
            }],
            ..ParticleSpec::default()
        };
        let mut pool = Pool::new(8, 4, 9);
        pool.step(&spec, &input(8, 0.0));
        let mut died = 0;
        for _ in 0..200 {
            died += pool.step(&spec, &input(0, 1.0 / 60.0)).died;
        }
        assert!(died > 0);
    }

    #[test]
    fn curl_noise_has_no_divergence() {
        let p = Vec3::new(0.3, 1.7, -2.2);
        let e = 0.01;
        let div = (curl(p + Vec3::X * e, false).x - curl(p - Vec3::X * e, false).x
            + curl(p + Vec3::Y * e, false).y
            - curl(p - Vec3::Y * e, false).y
            + curl(p + Vec3::Z * e, false).z
            - curl(p - Vec3::Z * e, false).z)
            / (2.0 * e);
        let size = curl(p, false).length();
        assert!(div.abs() < size.max(1.0) * 0.1, "div {div}, size {size}");
    }

    #[test]
    fn ribbons_lay_points_as_the_particle_travels() {
        let spec = ParticleSpec {
            gravity_scale: 0.0,
            speed: 0.0,
            lifetime: 100.0,
            modules: vec![UpdateModule::Force {
                force: [0.0, 100.0, 0.0],
            }],
            ribbon: RibbonSpec {
                enabled: true,
                points: 4,
                step: 0.5,
                ..RibbonSpec::default()
            },
            ..ParticleSpec::default()
        };
        let mut pool = Pool::new(1, 4, 2);
        pool.step(&spec, &input(1, 0.0));
        for _ in 0..30 {
            pool.step(&spec, &input(0, 0.05));
        }
        let held = pool.particles[0].ribbon >> 16;
        assert_eq!(held, 4);
        // The head point rides the particle.
        let head = (pool.particles[0].ribbon & 0xFFFF) as usize;
        assert_eq!(pool.trail[head][1], pool.particles[0].position[1]);
    }

    #[test]
    fn an_actor_ribbon_pins_slot_zero() {
        let spec = ParticleSpec {
            rate: 0.0,
            ribbon: RibbonSpec {
                enabled: true,
                source: RibbonSource::Actor,
                ..RibbonSpec::default()
            },
            ..ParticleSpec::default()
        };
        let mut pool = Pool::new(4, 8, 2);
        let mut step = input(0, 0.1);
        let moved = Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0));
        step.emitter = moved;
        pool.step(&spec, &step);
        assert_eq!(pool.particles[0].flags & FLAG_PINNED, FLAG_PINNED);
        assert_eq!(pool.particles[0].position, [3.0, 0.0, 0.0]);
        assert_eq!(pool.alive(), 0);
    }

    #[test]
    fn mesh_surfaces_pick_by_area() {
        let big = Triangle {
            a: Vec3::ZERO,
            b: Vec3::new(10.0, 0.0, 0.0),
            c: Vec3::new(0.0, 10.0, 0.0),
        };
        let small = Triangle {
            a: Vec3::new(0.0, 0.0, 5.0),
            b: Vec3::new(0.1, 0.0, 5.0),
            c: Vec3::new(0.0, 0.1, 5.0),
        };
        let surface = Surface::new(vec![small, big]);
        let mut state = 1;
        let on_big = (0..1000)
            .filter(|_| {
                let (p, _) = surface
                    .pick(rand(&mut state), rand(&mut state), rand(&mut state))
                    .unwrap();
                p.z == 0.0
            })
            .count();
        assert!(on_big > 990);
    }

    #[test]
    fn the_particle_layout_is_64_bytes() {
        let mut bytes = Vec::new();
        Particle::to_bytes(&[Particle::default(); 3], &mut bytes);
        assert_eq!(bytes.len(), 3 * Particle::BYTES);
    }

    #[test]
    fn old_emitters_load_as_graphs() {
        let old = r##"{"rate":24,"lifetime":0.8,"speed":120,"spread":60,"gravity_scale":0.5,"size_start":6,"size_end":1,"color_start":"#FFFFFF","color_end":"#FFAB19","max":128}"##;
        let spec: ParticleSpec = serde_json::from_str(old).unwrap();
        assert_eq!(spec.shape, SpawnShape::Point);
        assert!(spec.modules.is_empty());
        assert_eq!(spec.render.alpha.sample(1.0), 0.0);
        assert!(!spec.ribbon.enabled);
    }

    #[test]
    fn the_vfx_module_compiles() {
        crate::shader_lib::validate(include_str!("shaders/vfx.wesl"), &[])
            .unwrap_or_else(|error| panic!("{error}"));
    }
}
