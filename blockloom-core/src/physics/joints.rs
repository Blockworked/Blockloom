//! Constraints: repeatable joints between two bodies, or a body and the world.
//!
//! A [`ConstraintSpec`] is the saved description (several per actor, each with a
//! [`ConstraintId`]). [`resolve_frames`] turns it into the two joint frames the
//! backend needs, [`plan_constraints`] checks endpoints and gathers the lot for
//! the runtime, and [`should_break`] is the break rule. Run state (a motor's
//! speed, whether a joint broke) lives in the thread-local registry at the end,
//! which the runtime fills and blocks and scripts read and command.
//!
//! Units are document units: metres in 3D, pixels in 2D (the runtime converts),
//! and degrees for angles.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::ids::ConstraintId;
use super::ownership::actor_worlds;
use super::validate::PhysicsIssue;
use crate::project::Actor;
use crate::scene::Mode;

/// What a constraint holds fixed and what it leaves free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ConstraintKind {
    /// No relative motion at all.
    #[default]
    Fixed,
    /// Turns about one axis (a pivot in 2D).
    Hinge,
    /// Turns freely about the anchor. 3D only.
    Ball,
    /// Slides along one axis.
    Slider,
    /// A spring along the line between the anchors.
    Spring,
    /// Keeps the anchors between a minimum and a maximum distance (a rope).
    Distance,
    /// A slider for the suspension and a hinge for the spin, on one axis.
    Wheel,
    /// Each of the six axes chosen as locked, limited or free.
    Configurable,
}

impl ConstraintKind {
    pub const ALL: [ConstraintKind; 8] = [
        Self::Fixed,
        Self::Hinge,
        Self::Ball,
        Self::Slider,
        Self::Spring,
        Self::Distance,
        Self::Wheel,
        Self::Configurable,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Hinge => "hinge",
            Self::Ball => "ball",
            Self::Slider => "slider",
            Self::Spring => "spring",
            Self::Distance => "distance",
            Self::Wheel => "wheel",
            Self::Configurable => "configurable",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let word = text.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.name() == word)
    }

    /// Whether the kind exists in `mode`.
    pub fn in_mode(self, mode: Mode) -> bool {
        !(self == Self::Ball && mode == Mode::TwoD)
    }

    /// The axis the kind is built around, when the author gives none.
    pub fn default_axis(self, mode: Mode) -> [f32; 3] {
        match (self, mode) {
            (Self::Hinge | Self::Wheel, Mode::ThreeD) => [0.0, 0.0, 1.0],
            (Self::Slider, Mode::TwoD) => [1.0, 0.0, 0.0],
            (_, _) => [1.0, 0.0, 0.0],
        }
    }
}

/// A motion limit on the joint's free axis: degrees for a turn, document units
/// for a slide or a distance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub min: f32,
    #[serde(default)]
    pub max: f32,
}

impl Default for Limit {
    fn default() -> Self {
        Self {
            enabled: false,
            min: 0.0,
            max: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MotorMode {
    #[default]
    Off,
    /// Drives the free axis at `target` (degrees or units a second).
    Velocity,
    /// Drives the free axis towards `target` with a spring.
    Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Motor {
    #[serde(default)]
    pub mode: MotorMode,
    /// Velocity (degrees or units per second) or position (degrees or units).
    #[serde(default)]
    pub target: f32,
    /// The most force or torque the motor may use; 0 means no limit.
    #[serde(default)]
    pub max_force: f32,
    /// Position mode: how hard it pulls towards the target.
    #[serde(default = "default_stiffness")]
    pub stiffness: f32,
    #[serde(default = "default_damping")]
    pub damping: f32,
}

fn default_stiffness() -> f32 {
    100.0
}

fn default_damping() -> f32 {
    10.0
}

impl Default for Motor {
    fn default() -> Self {
        Self {
            mode: MotorMode::Off,
            target: 0.0,
            max_force: 0.0,
            stiffness: default_stiffness(),
            damping: default_damping(),
        }
    }
}

/// A spring's softness (the `Spring` kind) or a soft limit's.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpringSpec {
    #[serde(default = "default_stiffness")]
    pub stiffness: f32,
    #[serde(default = "default_damping")]
    pub damping: f32,
    /// The distance the spring settles at.
    #[serde(default = "default_rest")]
    pub rest_length: f32,
}

fn default_rest() -> f32 {
    1.0
}

impl Default for SpringSpec {
    fn default() -> Self {
        Self {
            stiffness: default_stiffness(),
            damping: default_damping(),
            rest_length: default_rest(),
        }
    }
}

/// How one axis of a configurable constraint behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AxisMode {
    #[default]
    Locked,
    Limited,
    Free,
}

/// One authored constraint on an actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintSpec {
    #[serde(default)]
    pub id: ConstraintId,
    /// What blocks and scripts call it; unique on its actor. Empty is addressed
    /// by its place (`1`, `2`, ...).
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub kind: ConstraintKind,
    /// The actor at the other end. Empty anchors the constraint to the world.
    #[serde(default)]
    pub target: String,
    /// Where it attaches, in this actor's own frame.
    #[serde(default)]
    pub anchor: [f32; 3],
    /// Where it attaches on the target (in the target's frame, or the world's
    /// when there is no target). Used when `auto_configure` is off.
    #[serde(default)]
    pub connected_anchor: [f32; 3],
    /// Work the other end's anchor and axis out from where the two actors
    /// stand, so nothing snaps when Play starts.
    #[serde(default = "yes")]
    pub auto_configure: bool,
    /// The hinge, slide or wheel axis in this actor's frame.
    #[serde(default = "default_axis")]
    pub axis: [f32; 3],
    #[serde(default = "default_axis")]
    pub connected_axis: [f32; 3],
    /// Limits the free axis (hinge angle, slide travel, spring travel).
    #[serde(default)]
    pub limit: Limit,
    #[serde(default)]
    pub motor: Motor,
    #[serde(default)]
    pub spring: SpringSpec,
    /// A distance constraint's range.
    #[serde(default)]
    pub min_distance: f32,
    #[serde(default = "default_rest")]
    pub max_distance: f32,
    /// The six axes of a configurable constraint: x, y, z then the three turns.
    #[serde(default)]
    pub axes: [AxisMode; 6],
    #[serde(default)]
    pub axis_limits: [[f32; 2]; 6],
    /// The joint snaps when it carries more than this force or torque.
    #[serde(default)]
    pub break_force: Option<f32>,
    #[serde(default)]
    pub break_torque: Option<f32>,
    /// Whether the two bodies still collide with each other.
    #[serde(default)]
    pub enable_collision: bool,
    /// Broadcast when the constraint breaks; empty says nothing.
    #[serde(default)]
    pub break_message: String,
}

fn yes() -> bool {
    true
}

fn default_axis() -> [f32; 3] {
    [1.0, 0.0, 0.0]
}

impl Default for ConstraintSpec {
    fn default() -> Self {
        Self {
            id: ConstraintId::generate(),
            name: String::new(),
            enabled: true,
            kind: ConstraintKind::Fixed,
            target: String::new(),
            anchor: [0.0; 3],
            connected_anchor: [0.0; 3],
            auto_configure: true,
            axis: default_axis(),
            connected_axis: default_axis(),
            limit: Limit::default(),
            motor: Motor::default(),
            spring: SpringSpec::default(),
            min_distance: 0.0,
            max_distance: default_rest(),
            axes: [AxisMode::Locked; 6],
            axis_limits: [[0.0; 2]; 6],
            break_force: None,
            break_torque: None,
            enable_collision: false,
            break_message: String::new(),
        }
    }
}

impl ConstraintSpec {
    /// A constraint of `kind` with the axis that kind is built around.
    pub fn of(kind: ConstraintKind, mode: Mode) -> Self {
        let axis = kind.default_axis(mode);
        Self {
            kind,
            axis,
            connected_axis: axis,
            ..Self::default()
        }
    }

    /// Why a setting is wrong, as (field, message) pairs.
    pub fn validate(&self, mode: Mode) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut bad = |field: &str, why: &str| out.push((field.to_string(), why.to_string()));
        if !self.kind.in_mode(mode) {
            bad("kind", "a ball constraint needs a 3D project");
        }
        let numbers = self
            .anchor
            .iter()
            .chain(&self.connected_anchor)
            .chain(&self.axis)
            .chain(&self.connected_axis)
            .chain(self.axis_limits.iter().flatten());
        if numbers.into_iter().any(|v| !v.is_finite()) {
            bad("anchor", "anchors, axes and limits must be finite numbers");
        }
        if matches!(
            self.kind,
            ConstraintKind::Hinge
                | ConstraintKind::Slider
                | ConstraintKind::Wheel
                | ConstraintKind::Configurable
        ) && mode == Mode::ThreeD
            && Vec3::from(self.axis).length() < 1e-4
        {
            bad("axis", "the axis can't be zero length");
        }
        if self.limit.enabled {
            if !self.limit.min.is_finite() || !self.limit.max.is_finite() {
                bad("limit", "limits must be finite numbers");
            } else if self.limit.min > self.limit.max {
                bad("limit", "the lower limit is above the upper limit");
            } else if matches!(self.kind, ConstraintKind::Hinge | ConstraintKind::Ball)
                && (self.limit.min < -180.0 || self.limit.max > 180.0)
            {
                bad("limit", "a turn limit stays within -180 to 180 degrees");
            }
        }
        if self.kind == ConstraintKind::Distance
            && (self.min_distance < 0.0
                || self.max_distance <= 0.0
                || self.min_distance > self.max_distance)
        {
            bad(
                "max_distance",
                "a distance range needs 0 <= minimum <= maximum, maximum above 0",
            );
        }
        if self.kind == ConstraintKind::Spring
            && (self.spring.stiffness < 0.0
                || self.spring.damping < 0.0
                || self.spring.rest_length < 0.0)
        {
            bad(
                "spring",
                "stiffness, damping and rest length can't be negative",
            );
        }
        if self.motor.mode != MotorMode::Off {
            if !matches!(
                self.kind,
                ConstraintKind::Hinge | ConstraintKind::Slider | ConstraintKind::Wheel
            ) {
                bad("motor", "only a hinge, slider or wheel has a motor");
            }
            if self.motor.max_force < 0.0
                || self.motor.stiffness < 0.0
                || self.motor.damping < 0.0
                || !self.motor.target.is_finite()
            {
                bad(
                    "motor",
                    "motor force, stiffness and damping can't be negative",
                );
            }
        }
        for (field, value) in [
            ("break_force", self.break_force),
            ("break_torque", self.break_torque),
        ] {
            if value.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
                bad(field, "a break threshold is a number above zero, or none");
            }
        }
        out
    }

    /// The name blocks use: its own, else its place in the actor's list.
    pub fn handle(&self, index: usize) -> String {
        if self.name.trim().is_empty() {
            (index + 1).to_string()
        } else {
            self.name.trim().to_string()
        }
    }
}

/// The two frames a joint is built from: a point and a rotation on each body,
/// with the joint's own X axis along the hinge, slide or wheel axis. Aligned in
/// the world at the pose the scene stands in, so Play starts without a snap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Frames {
    pub position_a: [f32; 3],
    pub rotation_a: [f32; 4],
    pub position_b: [f32; 3],
    pub rotation_b: [f32; 4],
}

fn rigid(world: &Mat4) -> (Quat, Vec3) {
    let (_, rotation, position) = world.to_scale_rotation_translation();
    (rotation.normalize(), position)
}

/// Works out the frames of `spec` for an actor standing at `a` and a target
/// standing at `b` (the world itself when `None`).
pub fn resolve_frames(spec: &ConstraintSpec, a: &Mat4, b: Option<&Mat4>) -> Frames {
    let (a_rot, a_pos) = rigid(a);
    let axis = Vec3::from(spec.axis);
    let axis = if axis.length() < 1e-6 {
        Vec3::X
    } else {
        axis.normalize()
    };
    let frame_a = Quat::from_rotation_arc(Vec3::X, axis);
    let world_anchor = a.transform_point3(Vec3::from(spec.anchor));
    let position_a = a_rot.inverse() * (world_anchor - a_pos);
    let (b_rot, b_pos) = b.map_or((Quat::IDENTITY, Vec3::ZERO), rigid);
    if spec.auto_configure {
        let world_frame = a_rot * frame_a;
        return Frames {
            position_a: position_a.to_array(),
            rotation_a: frame_a.to_array(),
            position_b: (b_rot.inverse() * (world_anchor - b_pos)).to_array(),
            rotation_b: (b_rot.inverse() * world_frame).normalize().to_array(),
        };
    }
    let connected = Vec3::from(spec.connected_axis);
    let connected = if connected.length() < 1e-6 {
        Vec3::X
    } else {
        connected.normalize()
    };
    Frames {
        position_a: position_a.to_array(),
        rotation_a: frame_a.to_array(),
        position_b: spec.connected_anchor,
        rotation_b: Quat::from_rotation_arc(Vec3::X, connected).to_array(),
    }
}

/// Whether the load on a joint snaps it. `force` and `torque` are what the
/// backend measured over one step of `dt` seconds (impulses divided by `dt`).
pub fn should_break(spec: &ConstraintSpec, force: f32, torque: f32) -> bool {
    spec.break_force.is_some_and(|limit| force > limit)
        || spec.break_torque.is_some_and(|limit| torque > limit)
}

/// One constraint, planned.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConstraintPlan {
    pub actor: String,
    /// `None` when anchored to the world.
    pub target: Option<String>,
    pub handle: String,
    pub spec: ConstraintSpec,
    pub frames: Frames,
}

/// Checks every constraint's endpoints and gathers the runnable ones.
pub fn plan_constraints(
    actors: &[Actor],
    mode: Mode,
    issues: &mut Vec<PhysicsIssue>,
) -> Vec<ConstraintPlan> {
    let by_id: HashMap<&str, &Actor> = actors.iter().map(|a| (a.id.as_str(), a)).collect();
    let worlds = actor_worlds(actors);
    let mut planned = Vec::new();
    let mut seen_ids: HashSet<&ConstraintId> = HashSet::new();
    for actor in actors {
        let mut handles: HashSet<String> = HashSet::new();
        for (index, spec) in actor.components.constraints().enumerate() {
            let id = actor.id.as_str();
            let issue = |message: String| PhysicsIssue::error(message).on(id).of(spec.id.as_str());
            if !seen_ids.insert(&spec.id) {
                issues.push(issue(format!(
                    "\"{}\" repeats constraint id {}",
                    actor.name, spec.id
                )));
                continue;
            }
            let handle = spec.handle(index);
            if !handles.insert(handle.to_ascii_lowercase()) {
                issues.push(issue(format!(
                    "\"{}\" has two constraints called \"{handle}\"",
                    actor.name
                )));
                continue;
            }
            let problems = spec.validate(mode);
            if !problems.is_empty() {
                for (field, why) in problems {
                    issues.push(issue(format!("{} \"{handle}\": {why}", actor.name)).field(&field));
                }
                continue;
            }
            if !spec.enabled {
                continue;
            }
            if actor.components.rigidbody().is_none() {
                issues.push(issue(format!(
                    "\"{}\" has a constraint but no Rigidbody to hold it",
                    actor.name
                )));
                continue;
            }
            let target = if spec.target.is_empty() {
                None
            } else {
                let Some(other) = by_id.get(spec.target.as_str()) else {
                    issues.push(issue(format!(
                        "constraint \"{handle}\" on \"{}\" joins an actor that doesn't exist",
                        actor.name
                    )));
                    continue;
                };
                if other.id == actor.id {
                    issues.push(issue(format!(
                        "constraint \"{handle}\" on \"{}\" joins the actor to itself",
                        actor.name
                    )));
                    continue;
                }
                if other.components.rigidbody().is_none() {
                    issues.push(issue(format!(
                        "constraint \"{handle}\" on \"{}\" joins \"{}\", which has no Rigidbody",
                        actor.name, other.name
                    )));
                    continue;
                }
                Some(other.id.clone())
            };
            let Some(a_world) = worlds.get(&actor.id) else {
                continue;
            };
            let b_world = target.as_ref().and_then(|t| worlds.get(t));
            planned.push(ConstraintPlan {
                actor: actor.id.clone(),
                target,
                handle,
                spec: spec.clone(),
                frames: resolve_frames(spec, a_world, b_world),
            });
        }
    }
    planned
}

// ---------------------------------------------------------------------------
// The backend-neutral recipe.

/// One of the joint's six degrees of freedom. 2D uses `LinX`, `LinY` and `AngX`
/// (the single turn).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum JointAxis {
    LinX,
    LinY,
    LinZ,
    AngX,
    AngY,
    AngZ,
}

impl JointAxis {
    pub fn is_angular(self) -> bool {
        matches!(self, Self::AngX | Self::AngY | Self::AngZ)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AxisLimit {
    pub axis: JointAxis,
    /// Radians for a turn, document units for a slide.
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MotorRecipe {
    pub axis: JointAxis,
    /// A velocity motor chases a speed, a position motor a place.
    pub velocity: bool,
    /// Radians (a second) for a turn, document units (a second) for a slide.
    pub target: f32,
    pub stiffness: f32,
    pub damping: f32,
    /// `None` is unlimited.
    pub max_force: Option<f32>,
}

/// What to ask the backend for: which axes are locked, which are limited,
/// which have motors, and whether the linear axes act as one distance.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Blueprint {
    pub locked: Vec<JointAxis>,
    pub limits: Vec<AxisLimit>,
    pub motors: Vec<MotorRecipe>,
    /// The three linear axes measure one distance (spring and rope).
    pub coupled_linear: bool,
    pub contacts: bool,
}

impl Blueprint {
    pub fn is_locked(&self, axis: JointAxis) -> bool {
        self.locked.contains(&axis)
    }
}

/// The motor of `spec` on `axis`, if it has one switched on.
pub fn motor_recipe(spec: &ConstraintSpec, axis: JointAxis) -> Option<MotorRecipe> {
    let m = spec.motor;
    if m.mode == MotorMode::Off {
        return None;
    }
    let target = if axis.is_angular() {
        m.target.to_radians()
    } else {
        m.target
    };
    let velocity = m.mode == MotorMode::Velocity;
    Some(MotorRecipe {
        axis,
        velocity,
        target,
        // A velocity motor is all damping: it pushes until the speed is met.
        stiffness: if velocity { 0.0 } else { m.stiffness },
        damping: m.damping.max(1e-3),
        max_force: (m.max_force > 0.0).then_some(m.max_force),
    })
}

/// The recipe for `spec` in `mode`.
pub fn blueprint(spec: &ConstraintSpec, mode: Mode) -> Blueprint {
    use JointAxis::*;
    let three = mode == Mode::ThreeD;
    let linear: &[JointAxis] = if three {
        &[LinX, LinY, LinZ]
    } else {
        &[LinX, LinY]
    };
    let angular: &[JointAxis] = if three { &[AngX, AngY, AngZ] } else { &[AngX] };
    let mut b = Blueprint {
        contacts: spec.enable_collision,
        ..Default::default()
    };
    let limit_on = |axis: JointAxis| -> Option<AxisLimit> {
        spec.limit.enabled.then(|| {
            let (min, max) = (spec.limit.min, spec.limit.max);
            if axis.is_angular() {
                AxisLimit {
                    axis,
                    min: min.to_radians(),
                    max: max.to_radians(),
                }
            } else {
                AxisLimit { axis, min, max }
            }
        })
    };
    match spec.kind {
        ConstraintKind::Fixed => {
            b.locked.extend(linear);
            b.locked.extend(angular);
        }
        ConstraintKind::Hinge => {
            b.locked.extend(linear);
            b.locked.extend(angular.iter().filter(|a| **a != AngX));
            b.limits.extend(limit_on(AngX));
            b.motors.extend(motor_recipe(spec, AngX));
        }
        ConstraintKind::Ball => {
            b.locked.extend(linear);
            for axis in angular {
                b.limits.extend(limit_on(*axis));
            }
        }
        ConstraintKind::Slider => {
            b.locked.extend(linear.iter().filter(|a| **a != LinX));
            b.locked.extend(angular);
            b.limits.extend(limit_on(LinX));
            b.motors.extend(motor_recipe(spec, LinX));
        }
        ConstraintKind::Spring => {
            b.coupled_linear = true;
            b.limits.extend(limit_on(LinX));
            b.motors.push(MotorRecipe {
                axis: LinX,
                velocity: false,
                target: spec.spring.rest_length,
                stiffness: spec.spring.stiffness,
                damping: spec.spring.damping,
                max_force: None,
            });
        }
        ConstraintKind::Distance => {
            b.coupled_linear = true;
            b.limits.push(AxisLimit {
                axis: LinX,
                min: spec.min_distance,
                max: spec.max_distance,
            });
        }
        ConstraintKind::Wheel => {
            b.locked.extend(linear.iter().filter(|a| **a != LinX));
            b.locked.extend(angular.iter().filter(|a| **a != AngX));
            b.limits.extend(limit_on(LinX));
            if spec.spring.stiffness > 0.0 {
                b.motors.push(MotorRecipe {
                    axis: LinX,
                    velocity: false,
                    target: 0.0,
                    stiffness: spec.spring.stiffness,
                    damping: spec.spring.damping,
                    max_force: None,
                });
            }
            b.motors.extend(motor_recipe(spec, AngX));
        }
        ConstraintKind::Configurable => {
            let order: &[(usize, JointAxis)] = if three {
                &[
                    (0, LinX),
                    (1, LinY),
                    (2, LinZ),
                    (3, AngX),
                    (4, AngY),
                    (5, AngZ),
                ]
            } else {
                &[(0, LinX), (1, LinY), (5, AngX)]
            };
            for (index, axis) in order {
                match spec.axes[*index] {
                    AxisMode::Locked => b.locked.push(*axis),
                    AxisMode::Limited => {
                        let [min, max] = spec.axis_limits[*index];
                        let (min, max) = if axis.is_angular() {
                            (min.to_radians(), max.to_radians())
                        } else {
                            (min, max)
                        };
                        b.limits.push(AxisLimit {
                            axis: *axis,
                            min,
                            max,
                        });
                    }
                    AxisMode::Free => {}
                }
            }
        }
    }
    b
}

/// Where the free axis of a constraint stands now, from the two bodies'
/// rigid poses as (rotation, position) arrays (`a` is the actor holding the
/// constraint, `b` its target or the world): degrees for a turn, document units for a slide or a distance.
pub fn measure(
    spec: &ConstraintSpec,
    frames: &Frames,
    a: ([f32; 4], [f32; 3]),
    b: ([f32; 4], [f32; 3]),
) -> f32 {
    let (a_rot, a_pos) = (Quat::from_array(a.0).normalize(), Vec3::from(a.1));
    let (b_rot, b_pos) = (Quat::from_array(b.0).normalize(), Vec3::from(b.1));
    let frame_a = a_rot * Quat::from_array(frames.rotation_a);
    let frame_b = b_rot * Quat::from_array(frames.rotation_b);
    let point_a = a_pos + a_rot * Vec3::from(frames.position_a);
    let point_b = b_pos + b_rot * Vec3::from(frames.position_b);
    match spec.kind {
        ConstraintKind::Hinge | ConstraintKind::Ball => {
            let relative = (frame_b.inverse() * frame_a).normalize();
            let angle = 2.0 * relative.x.atan2(relative.w);
            let wrapped = (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            wrapped.to_degrees()
        }
        ConstraintKind::Slider | ConstraintKind::Wheel => {
            (point_a - point_b).dot(frame_b * Vec3::X)
        }
        _ => (point_a - point_b).length(),
    }
}

// ---------------------------------------------------------------------------
// Run state: what the runtime publishes and what blocks command.

/// What the runtime measured on one constraint last tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JointStatus {
    pub broken: bool,
    pub enabled: bool,
    /// The free axis now: degrees for a turn, units for a slide or a distance.
    pub position: f32,
    /// How fast the free axis is moving.
    pub speed: f32,
    pub force: f32,
    pub torque: f32,
}

/// A change a block or script asked for, applied by the runtime on the next
/// fixed tick.
#[derive(Debug, Clone, PartialEq)]
pub struct JointCommand {
    pub actor: String,
    pub handle: String,
    pub verb: JointVerb,
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum JointVerb {
    Enable,
    Disable,
    Break,
    #[default]
    MotorSpeed,
    MotorTarget,
    MotorForce,
    MotorOff,
    Stiffness,
    Damping,
}

impl JointVerb {
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word.trim().to_ascii_lowercase().as_str() {
            "enable" => Self::Enable,
            "disable" => Self::Disable,
            "break" => Self::Break,
            "motor speed" => Self::MotorSpeed,
            "motor target" => Self::MotorTarget,
            "motor force" => Self::MotorForce,
            "motor off" => Self::MotorOff,
            "stiffness" => Self::Stiffness,
            "damping" => Self::Damping,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Break => "break",
            Self::MotorSpeed => "motor speed",
            Self::MotorTarget => "motor target",
            Self::MotorForce => "motor force",
            Self::MotorOff => "motor off",
            Self::Stiffness => "stiffness",
            Self::Damping => "damping",
        }
    }
}

#[derive(Default)]
struct Registry {
    /// Actor -> handles in list order.
    names: HashMap<String, Vec<String>>,
    status: HashMap<(String, String), JointStatus>,
    queue: Vec<JointCommand>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Forgets every joint: a world is being rebuilt or a run ended.
pub fn reset() {
    REGISTRY.with(|r| *r.borrow_mut() = Registry::default());
}

/// Makes the plan's constraints addressable by name.
pub fn register_plan(plan: &[ConstraintPlan]) {
    REGISTRY.with(|r| *r.borrow_mut() = Registry::default());
    extend_plan(plan);
}

/// Adds constraints that came into the run after it began (a clone's).
pub fn extend_plan(plan: &[ConstraintPlan]) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        for c in plan {
            r.names
                .entry(c.actor.clone())
                .or_default()
                .push(c.handle.clone());
            r.status.insert(
                (c.actor.clone(), c.handle.to_ascii_lowercase()),
                JointStatus {
                    enabled: true,
                    ..Default::default()
                },
            );
        }
    });
}

/// Names constraints an actor carries without a plan behind them, for a host or
/// test that answers joint blocks itself.
pub fn register_handles(actor: &str, handles: &[&str]) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        for handle in handles {
            r.names
                .entry(actor.to_string())
                .or_default()
                .push((*handle).to_string());
            r.status.insert(
                (actor.to_string(), handle.to_ascii_lowercase()),
                JointStatus {
                    enabled: true,
                    ..Default::default()
                },
            );
        }
    });
}

pub fn publish(actor: &str, handle: &str, status: JointStatus) {
    REGISTRY.with(|r| {
        r.borrow_mut()
            .status
            .insert((actor.to_string(), handle.to_ascii_lowercase()), status);
    });
}

pub fn status(actor: &str, handle: &str) -> Option<JointStatus> {
    REGISTRY.with(|r| {
        r.borrow()
            .status
            .get(&(actor.to_string(), handle.trim().to_ascii_lowercase()))
            .copied()
    })
}

/// The commands blocks and scripts queued since the last call.
pub fn take_commands() -> Vec<JointCommand> {
    REGISTRY.with(|r| std::mem::take(&mut r.borrow_mut().queue))
}

/// Runs `joint <verb>|<name>` for `actor`: the op a block lowers to.
pub fn run_op(actor: &str, op: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() {
        return Err("a joint statement needs a finite number".to_string());
    }
    let (verb, handle) = op
        .split_once('|')
        .ok_or_else(|| format!("a joint statement is \"verb|name\", not \"{op}\""))?;
    let verb = JointVerb::parse(verb)
        .ok_or_else(|| format!("a joint has no statement called \"{}\"", verb.trim()))?;
    let handle = handle.trim();
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let Some(names) = r.names.get(actor) else {
            return Err("this actor has no constraints".to_string());
        };
        let found = names
            .iter()
            .find(|n| n.eq_ignore_ascii_case(handle))
            .cloned()
            .ok_or_else(|| format!("this actor has no constraint called \"{handle}\""))?;
        r.queue.push(JointCommand {
            actor: actor.to_string(),
            handle: found,
            verb,
            value,
        });
        Ok(())
    })
}

/// A number from a constraint, by the name a reporter gives it.
pub fn read_number(actor: &str, handle: &str, field: &str) -> f64 {
    let count = REGISTRY.with(|r| r.borrow().names.get(actor).map_or(0, Vec::len));
    let field = field.trim().to_ascii_lowercase();
    if field == "count" {
        return count as f64;
    }
    let found = status(actor, handle);
    if field == "exists" {
        return f64::from(found.is_some());
    }
    let Some(s) = found else {
        return 0.0;
    };
    match field.as_str() {
        "broken" => f64::from(s.broken),
        "enabled" => f64::from(s.enabled && !s.broken),
        "position" | "angle" => f64::from(s.position),
        "speed" => f64::from(s.speed),
        "force" => f64::from(s.force),
        "torque" => f64::from(s.torque),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ActorComponent;
    use crate::physics::RigidbodySpec;
    use crate::scene::Visual;

    fn body(name: &str, x: f32) -> Actor {
        let mut a = Actor::new(
            name,
            Visual::Cuboid {
                color: "#fff".into(),
                size: [1.0; 3],
            },
        );
        a.id = name.to_ascii_lowercase();
        a.components.insert(ActorComponent::Rigidbody {
            rigidbody: RigidbodySpec::default(),
        });
        if let Some(ActorComponent::Place { placement }) = a.components.get_mut("Place") {
            placement.position = [x, 0.0, 0.0];
        }
        a
    }

    fn with(actor: &mut Actor, spec: ConstraintSpec) {
        actor
            .components
            .insert(ActorComponent::Constraint { constraint: spec });
    }

    #[test]
    fn a_hinge_between_two_bodies_plans_aligned_frames() {
        let mut door = body("Door", 2.0);
        let frame = body("Frame", 0.0);
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.target = "frame".into();
        spec.anchor = [-1.0, 0.0, 0.0];
        spec.axis = [0.0, 1.0, 0.0];
        with(&mut door, spec);
        let mut issues = Vec::new();
        let planned = plan_constraints(&[door, frame], Mode::ThreeD, &mut issues);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(planned.len(), 1);
        let f = planned[0].frames;
        // The anchor stands at world x = 1: -1 from the door, +1 from the frame.
        assert!((f.position_a[0] + 1.0).abs() < 1e-4);
        assert!((f.position_b[0] - 1.0).abs() < 1e-4);
        // Both frames' X axes point along world Y.
        let a = Quat::from_array(f.rotation_a) * Vec3::X;
        let b = Quat::from_array(f.rotation_b) * Vec3::X;
        assert!((a - Vec3::Y).length() < 1e-4 && (b - Vec3::Y).length() < 1e-4);
    }

    #[test]
    fn a_world_anchored_constraint_needs_no_target() {
        let mut lamp = body("Lamp", 0.0);
        let mut spec = ConstraintSpec::of(ConstraintKind::Distance, Mode::ThreeD);
        spec.anchor = [0.0, 1.0, 0.0];
        with(&mut lamp, spec);
        let mut issues = Vec::new();
        let planned = plan_constraints(&[lamp], Mode::ThreeD, &mut issues);
        assert!(issues.is_empty());
        assert_eq!(planned[0].target, None);
        assert!((planned[0].frames.position_b[1] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn bad_endpoints_are_named() {
        let mut a = body("A", 0.0);
        let mut missing = ConstraintSpec::default();
        missing.target = "ghost".into();
        with(&mut a, missing);
        let mut b = body("B", 1.0);
        let mut to_self = ConstraintSpec::default();
        to_self.target = "b".into();
        with(&mut b, to_self);
        let mut c = body("C", 2.0);
        let mut no_body = ConstraintSpec::default();
        no_body.target = "plain".into();
        with(&mut c, no_body);
        let mut plain = Actor::new(
            "Plain",
            Visual::Cuboid {
                color: "#fff".into(),
                size: [1.0; 3],
            },
        );
        plain.id = "plain".into();
        let mut issues = Vec::new();
        let planned = plan_constraints(&[a, b, c, plain], Mode::ThreeD, &mut issues);
        assert!(planned.is_empty());
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert!(issues.iter().all(|i| i.is_error()));
    }

    #[test]
    fn several_constraints_on_one_actor_need_distinct_names() {
        let mut a = body("A", 0.0);
        let mut one = ConstraintSpec::default();
        one.name = "top".into();
        let mut two = ConstraintSpec::default();
        two.name = "TOP".into();
        with(&mut a, one);
        with(&mut a, two);
        let mut issues = Vec::new();
        let planned = plan_constraints(&[a], Mode::ThreeD, &mut issues);
        assert_eq!(planned.len(), 1);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn fields_are_checked_with_their_names() {
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.limit = Limit {
            enabled: true,
            min: 30.0,
            max: -30.0,
        };
        assert_eq!(spec.validate(Mode::ThreeD)[0].0, "limit");
        let ball = ConstraintSpec::of(ConstraintKind::Ball, Mode::TwoD);
        assert_eq!(ball.validate(Mode::TwoD)[0].0, "kind");
        let mut motor = ConstraintSpec::of(ConstraintKind::Fixed, Mode::ThreeD);
        motor.motor.mode = MotorMode::Velocity;
        assert_eq!(motor.validate(Mode::ThreeD)[0].0, "motor");
        let mut brk = ConstraintSpec::default();
        brk.break_force = Some(0.0);
        assert_eq!(brk.validate(Mode::ThreeD)[0].0, "break_force");
        assert!(ConstraintSpec::default().validate(Mode::ThreeD).is_empty());
    }

    #[test]
    fn a_joint_breaks_past_either_threshold() {
        let mut spec = ConstraintSpec::default();
        assert!(!should_break(&spec, 1e9, 1e9));
        spec.break_force = Some(100.0);
        spec.break_torque = Some(10.0);
        assert!(!should_break(&spec, 99.0, 9.0));
        assert!(should_break(&spec, 101.0, 0.0));
        assert!(should_break(&spec, 0.0, 11.0));
    }

    #[test]
    fn statements_queue_for_the_runtime_and_reporters_read_its_status() {
        let mut a = body("A", 0.0);
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.name = "Wheel".into();
        with(&mut a, spec);
        let mut issues = Vec::new();
        let planned = plan_constraints(&[a], Mode::ThreeD, &mut issues);
        register_plan(&planned);
        assert!(run_op("a", "motor speed|wheel", 90.0).is_ok());
        assert!(run_op("a", "motor speed|nope", 1.0).is_err());
        assert!(run_op("a", "spin|wheel", 1.0).is_err());
        let queued = take_commands();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].handle, "Wheel");
        assert_eq!(queued[0].verb, JointVerb::MotorSpeed);
        assert_eq!(read_number("a", "wheel", "count"), 1.0);
        assert_eq!(read_number("a", "wheel", "broken"), 0.0);
        publish(
            "a",
            "Wheel",
            JointStatus {
                broken: true,
                ..Default::default()
            },
        );
        assert_eq!(read_number("a", "wheel", "broken"), 1.0);
        reset();
        assert_eq!(read_number("a", "wheel", "exists"), 0.0);
    }

    #[test]
    fn each_kind_locks_and_frees_the_right_axes() {
        use JointAxis::*;
        let make = |kind| ConstraintSpec::of(kind, Mode::ThreeD);
        let fixed = blueprint(&make(ConstraintKind::Fixed), Mode::ThreeD);
        assert_eq!(fixed.locked.len(), 6);
        let hinge = blueprint(&make(ConstraintKind::Hinge), Mode::ThreeD);
        assert!(!hinge.is_locked(AngX) && hinge.is_locked(AngY) && hinge.is_locked(LinX));
        let ball = blueprint(&make(ConstraintKind::Ball), Mode::ThreeD);
        assert_eq!(ball.locked.len(), 3);
        let slider = blueprint(&make(ConstraintKind::Slider), Mode::ThreeD);
        assert!(!slider.is_locked(LinX) && slider.is_locked(LinY) && slider.is_locked(AngX));
        let spring = blueprint(&make(ConstraintKind::Spring), Mode::ThreeD);
        assert!(spring.coupled_linear && spring.locked.is_empty());
        assert_eq!(spring.motors[0].target, 1.0);
        let rope = blueprint(&make(ConstraintKind::Distance), Mode::ThreeD);
        assert!(rope.coupled_linear);
        assert_eq!(rope.limits[0].max, 1.0);
        let wheel = blueprint(&make(ConstraintKind::Wheel), Mode::ThreeD);
        assert!(!wheel.is_locked(LinX) && !wheel.is_locked(AngX) && wheel.is_locked(AngY));
        assert_eq!(wheel.motors.len(), 1, "the suspension spring");
    }

    #[test]
    fn a_hinge_limit_and_motor_are_in_radians() {
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.limit = Limit {
            enabled: true,
            min: -90.0,
            max: 90.0,
        };
        spec.motor = Motor {
            mode: MotorMode::Velocity,
            target: 180.0,
            max_force: 50.0,
            ..Motor::default()
        };
        let b = blueprint(&spec, Mode::ThreeD);
        assert!((b.limits[0].max - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let m = b.motors[0];
        assert!(m.velocity && (m.target - std::f32::consts::PI).abs() < 1e-5);
        assert_eq!(m.stiffness, 0.0);
        assert_eq!(m.max_force, Some(50.0));
    }

    #[test]
    fn a_2d_configurable_constraint_maps_z_turn_to_the_one_angle() {
        use JointAxis::*;
        let mut spec = ConstraintSpec::of(ConstraintKind::Configurable, Mode::TwoD);
        spec.axes = [
            AxisMode::Free,
            AxisMode::Locked,
            AxisMode::Locked,
            AxisMode::Locked,
            AxisMode::Locked,
            AxisMode::Limited,
        ];
        spec.axis_limits[5] = [-45.0, 45.0];
        let b = blueprint(&spec, Mode::TwoD);
        assert_eq!(b.locked, vec![LinY]);
        assert_eq!(b.limits[0].axis, AngX);
        assert!((b.limits[0].max - 45f32.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn a_hinge_measures_its_turn_and_a_slider_its_travel() {
        let hinge = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        let a_world = Mat4::IDENTITY;
        let frames = resolve_frames(&hinge, &a_world, Some(&Mat4::IDENTITY));
        let quarter = Quat::from_axis_angle(
            Vec3::from(hinge.axis).normalize(),
            std::f32::consts::FRAC_PI_2,
        );
        let angle = measure(
            &hinge,
            &frames,
            (quarter.to_array(), [0.0; 3]),
            (Quat::IDENTITY.to_array(), [0.0; 3]),
        );
        assert!((angle - 90.0).abs() < 1e-3, "{angle}");
        let slider = ConstraintSpec::of(ConstraintKind::Slider, Mode::ThreeD);
        let frames = resolve_frames(&slider, &a_world, Some(&Mat4::IDENTITY));
        let travel = measure(
            &slider,
            &frames,
            (Quat::IDENTITY.to_array(), [2.5, 0.0, 0.0]),
            (Quat::IDENTITY.to_array(), [0.0; 3]),
        );
        assert!((travel - 2.5).abs() < 1e-4);
        let rope = ConstraintSpec::of(ConstraintKind::Distance, Mode::ThreeD);
        let frames = resolve_frames(&rope, &a_world, None);
        let gap = measure(
            &rope,
            &frames,
            (Quat::IDENTITY.to_array(), [0.0, 3.0, 4.0]),
            (Quat::IDENTITY.to_array(), [0.0; 3]),
        );
        assert!((gap - 5.0).abs() < 1e-4);
    }
}
