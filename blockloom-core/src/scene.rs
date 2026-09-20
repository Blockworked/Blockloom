//! The world a project describes: actors, how they look, how they collide,
//! and how it's viewed. Shared verbatim between the editor (which edits it)
//! and the runtime (which renders it).

use serde::{Deserialize, Serialize};

/// Which dimension a project runs in. Blocks are written once and mean the
/// obvious thing in both: a Z coordinate is simply ignored in `TwoD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    TwoD,
    ThreeD,
}

impl Mode {
    pub fn is_3d(self) -> bool {
        matches!(self, Mode::ThreeD)
    }
}

/// One axis of a position, rotation or velocity. Travels as its name, which
/// is what an in-place dropdown on the block writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Axis {
    #[default]
    X,
    Y,
    Z,
}

impl Axis {
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

/// What an actor looks like. The first three are 2D; the rest are 3D. An
/// actor's visual also decides the collider its body gets, so a project
/// never has to describe the same shape twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape")]
pub enum Visual {
    Rect {
        color: String,
        size: [f32; 2],
    },
    Circle {
        color: String,
        radius: f32,
    },
    /// A 2D image loaded from `path`, relative to the project's folder.
    Image {
        path: String,
        size: [f32; 2],
    },
    Cuboid {
        color: String,
        size: [f32; 3],
    },
    Sphere {
        color: String,
        radius: f32,
    },
    Capsule {
        color: String,
        radius: f32,
        height: f32,
    },
    /// A flat ground plane, the one 3D visual meant to be left static.
    Plane {
        color: String,
        size: [f32; 2],
    },
}

impl Visual {
    /// True for the 3D visuals - a project in the wrong mode still loads,
    /// and the runtime just won't have anything to draw it with.
    pub fn is_3d(&self) -> bool {
        matches!(
            self,
            Visual::Cuboid { .. }
                | Visual::Sphere { .. }
                | Visual::Capsule { .. }
                | Visual::Plane { .. }
        )
    }

    pub fn color(&self) -> Option<&str> {
        match self {
            Visual::Rect { color, .. }
            | Visual::Circle { color, .. }
            | Visual::Cuboid { color, .. }
            | Visual::Sphere { color, .. }
            | Visual::Capsule { color, .. }
            | Visual::Plane { color, .. } => Some(color),
            Visual::Image { .. } => None,
        }
    }

    pub fn set_color(&mut self, next: String) {
        match self {
            Visual::Rect { color, .. }
            | Visual::Circle { color, .. }
            | Visual::Cuboid { color, .. }
            | Visual::Sphere { color, .. }
            | Visual::Capsule { color, .. }
            | Visual::Plane { color, .. } => *color = next,
            Visual::Image { .. } => {}
        }
    }
}

/// How the physics engine treats an actor. `None` means the actor is moved
/// only by its own blocks - no collider, no gravity, nothing to push it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BodyKind {
    #[default]
    None,
    /// Immovable, but everything else collides with it - ground and walls.
    Static,
    /// Simulated: gravity, impulses, and collision response all apply.
    Dynamic,
    /// Moved by blocks, but still pushes dynamic bodies out of the way.
    Kinematic,
}

/// An actor's physics settings. The collider shape isn't here: it's derived
/// from [`Visual`], so the two can't disagree.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Physics {
    #[serde(default)]
    pub body: BodyKind,
    /// Multiplies the world's gravity for this actor alone - `0` for a
    /// floating dynamic body, `2` for a heavy one.
    #[serde(default = "one")]
    pub gravity_scale: f32,
    /// Stops a dynamic body from tipping over, the usual want for a
    /// platformer character.
    #[serde(default)]
    pub lock_rotation: bool,
    #[serde(default = "default_restitution")]
    pub restitution: f32,
    #[serde(default = "default_friction")]
    pub friction: f32,
}

fn one() -> f32 {
    1.0
}

fn default_restitution() -> f32 {
    0.0
}

fn default_friction() -> f32 {
    0.5
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            body: BodyKind::None,
            gravity_scale: 1.0,
            lock_rotation: false,
            restitution: 0.0,
            friction: 0.5,
        }
    }
}

/// An actor's place in the world. Rotation is Euler degrees per axis, which
/// is what the blocks talk in; the runtime converts once on the way to Bevy.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default)]
    pub rotation: [f32; 3],
    #[serde(default = "unit_scale")]
    pub scale: f32,
}

fn unit_scale() -> f32 {
    1.0
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
        }
    }
}

/// How the world is viewed. In `TwoD` only `zoom` and `follow` matter; in
/// `ThreeD` the camera sits at `position` looking at `look_at`, or trails
/// `follow` at that same offset when one is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    #[serde(default = "default_camera_position")]
    pub position: [f32; 3],
    #[serde(default)]
    pub look_at: [f32; 3],
    #[serde(default = "unit_scale")]
    pub zoom: f32,
    /// Actor id the camera keeps centered, if any.
    #[serde(default)]
    pub follow: Option<String>,
}

fn default_camera_position() -> [f32; 3] {
    [0.0, 6.0, 14.0]
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: default_camera_position(),
            look_at: [0.0; 3],
            zoom: 1.0,
            follow: None,
        }
    }
}

/// Everything about the world that isn't an actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct World {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default = "default_background")]
    pub background: String,
    /// Metres per second squared. The default is 2D-friendly: -981 units,
    /// since a 2D unit is a pixel.
    #[serde(default = "default_gravity_2d")]
    pub gravity: [f32; 3],
    #[serde(default)]
    pub camera: Camera,
}

fn default_background() -> String {
    "#1B2431".to_string()
}

fn default_gravity_2d() -> [f32; 3] {
    [0.0, -981.0, 0.0]
}

impl Default for World {
    fn default() -> Self {
        Self {
            mode: Mode::TwoD,
            background: default_background(),
            gravity: default_gravity_2d(),
            camera: Camera::default(),
        }
    }
}

impl World {
    /// The gravity a fresh project of this mode starts with - 2D works in
    /// pixels, 3D in metres, so the two differ by roughly a hundredfold.
    pub fn default_gravity(mode: Mode) -> [f32; 3] {
        match mode {
            Mode::TwoD => default_gravity_2d(),
            Mode::ThreeD => [0.0, -9.81, 0.0],
        }
    }
}
