//! The saved specifications: what a Rigidbody and a Collider *are*.
//!
//! These hold authoring intent only. Velocity, contacts, sleeping and ground
//! support are run state and live in the world, never here, so Play and Stop can
//! restore the document exactly. Every number is finite (see `validate`).

use serde::{Deserialize, Serialize};

use super::ids::{ColliderId, ComponentId};
use super::material::{MaterialOverrides, MaterialRef};
use crate::scene::Mode;

// ─── Rigidbody ──────────────────────────────────────────────────────────────

/// How a body moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BodyType {
    #[default]
    Dynamic,
    /// Moved by its owner; pushes dynamic bodies but is not pushed back.
    Kinematic,
    /// 2D only (Rigidbody2D has a Static body type; a 3D static collider has no
    /// Rigidbody at all).
    Static,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Interpolation {
    #[default]
    None,
    Interpolate,
    Extrapolate,
}

/// Unity's four collision detection names. What each one switches on in the
/// backend, and where it stops short, is the compatibility ledger's CCD table;
/// the runtime (Phase 2) reports an unsupported combination rather than
/// quietly aliasing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CollisionDetection {
    #[default]
    Discrete,
    Continuous,
    ContinuousDynamic,
    ContinuousSpeculative,
}

/// Where the body's mass comes from. Mass is body-owned: collider edits never
/// multiply an explicit mass by the number of shapes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum MassSource {
    /// A fixed mass in kilograms.
    Explicit { mass: f32 },
    /// Mass is the density times the volume (3D) or area (2D) of the attached
    /// solid colliders. Triggers add nothing.
    Density { density: f32 },
}

impl Default for MassSource {
    fn default() -> Self {
        MassSource::Explicit { mass: 1.0 }
    }
}

/// Axes the solver may not move or turn the body along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Constraints {
    #[serde(default)]
    pub freeze_position: [bool; 3],
    #[serde(default)]
    pub freeze_rotation: [bool; 3],
}

impl Constraints {
    pub fn is_free(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigidbodySpec {
    #[serde(default)]
    pub id: ComponentId,
    #[serde(default)]
    pub body_type: BodyType,
    /// 2D only: a body that is not simulated is left out of the world with its
    /// colliders.
    #[serde(default = "yes")]
    pub simulated: bool,
    #[serde(default)]
    pub mass: MassSource,
    /// Local centre of mass; `None` computes it from the colliders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_of_mass: Option<[f32; 3]>,
    /// Principal inertia (3D) or the scalar moment (2D, first element); `None`
    /// computes it from the colliders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inertia: Option<[f32; 3]>,
    #[serde(default)]
    pub linear_damping: f32,
    #[serde(default = "default_angular_damping")]
    pub angular_damping: f32,
    /// Whether the world's gravity applies at all (Unity's Use Gravity).
    #[serde(default = "yes")]
    pub use_gravity: bool,
    /// Blockloom's multiplier on top of `use_gravity`.
    #[serde(default = "one")]
    pub gravity_scale: f32,
    #[serde(default)]
    pub interpolation: Interpolation,
    #[serde(default)]
    pub collision_detection: CollisionDetection,
    #[serde(default)]
    pub constraints: Constraints,
    /// Metres per second.
    #[serde(default = "default_max_linear")]
    pub max_linear_velocity: f32,
    /// Radians per second.
    #[serde(default = "default_max_angular")]
    pub max_angular_velocity: f32,
    /// Mass-normalised kinetic energy below which the body may sleep.
    #[serde(default = "default_sleep")]
    pub sleep_threshold: f32,
    /// Extra solver iterations for this body; `None` uses the world's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solver_iterations: Option<u32>,
    /// Metres per second a penetrating contact may push this body out at.
    #[serde(default = "default_depenetration")]
    pub max_depenetration_velocity: f32,
    /// A migrated `Body` that had its character-control switch on: the runtime
    /// keeps the old kinematic controller on this body until it is converted to a
    /// CharacterController.
    #[serde(default, skip_serializing_if = "is_false")]
    pub legacy_character_control: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

fn default_angular_damping() -> f32 {
    0.05
}

/// Blockloom's default; Unity's documentation does not state one (ledger row 41).
pub const DEFAULT_MAX_LINEAR_VELOCITY: f32 = 10_000.0;

fn default_max_linear() -> f32 {
    DEFAULT_MAX_LINEAR_VELOCITY
}

fn default_max_angular() -> f32 {
    7.0
}

fn default_sleep() -> f32 {
    0.005
}

fn default_depenetration() -> f32 {
    10.0
}

impl Default for RigidbodySpec {
    fn default() -> Self {
        Self {
            id: ComponentId::generate(),
            body_type: BodyType::Dynamic,
            simulated: true,
            mass: MassSource::default(),
            center_of_mass: None,
            inertia: None,
            linear_damping: 0.0,
            angular_damping: default_angular_damping(),
            use_gravity: true,
            gravity_scale: 1.0,
            interpolation: Interpolation::None,
            collision_detection: CollisionDetection::Discrete,
            constraints: Constraints::default(),
            max_linear_velocity: default_max_linear(),
            max_angular_velocity: default_max_angular(),
            sleep_threshold: default_sleep(),
            solver_iterations: None,
            max_depenetration_velocity: default_depenetration(),
            legacy_character_control: false,
        }
    }
}

// ─── Collider ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Axis {
    X,
    #[default]
    Y,
    Z,
}

/// An authored shape. Sizes are full extents, in metres (3D) or world units (2D);
/// a capsule's `height` is end to end, caps included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ColliderShape {
    // 3D
    Box {
        size: [f32; 3],
    },
    Sphere {
        radius: f32,
    },
    Capsule {
        radius: f32,
        height: f32,
        #[serde(default)]
        axis: Axis,
    },
    /// The convex hull of a mesh asset; required for solid shapes on a dynamic body.
    ConvexHull {
        mesh: String,
    },
    /// A mesh asset as-is. Static (or kinematic) scenery only.
    TriangleMesh {
        mesh: String,
    },
    /// The heightfield of the actor's own Terrain component.
    Terrain,
    // 2D
    Rect {
        size: [f32; 2],
    },
    Circle {
        radius: f32,
    },
    Capsule2d {
        size: [f32; 2],
        #[serde(default)]
        horizontal: bool,
    },
    /// A convex polygon, counter-clockwise.
    Polygon {
        points: Vec<[f32; 2]>,
    },
    Edge {
        a: [f32; 2],
        b: [f32; 2],
    },
    Chain {
        points: Vec<[f32; 2]>,
        #[serde(default)]
        closed: bool,
    },
    /// The solid cells of the actor's own tilemap.
    Tilemap,
}

impl ColliderShape {
    /// Whether this shape can exist in a world of `mode`. A tilemap collides in
    /// both; `Terrain` and the mesh shapes are 3D, the rest of the flat shapes 2D.
    pub fn fits(&self, mode: Mode) -> bool {
        match self {
            ColliderShape::Tilemap => true,
            ColliderShape::Box { .. }
            | ColliderShape::Sphere { .. }
            | ColliderShape::Capsule { .. }
            | ColliderShape::ConvexHull { .. }
            | ColliderShape::TriangleMesh { .. }
            | ColliderShape::Terrain => mode == Mode::ThreeD,
            _ => mode == Mode::TwoD,
        }
    }

    /// "3D" or "2D" for messages about a shape in the wrong world.
    pub fn world_word(&self) -> &'static str {
        if self.fits(Mode::ThreeD) { "3D" } else { "2D" }
    }

    /// True for shapes with no inside: a solid dynamic body cannot be made of them.
    /// (A tilemap is a compound of convex cells, so it is fine.)
    pub fn is_concave(&self) -> bool {
        matches!(
            self,
            ColliderShape::TriangleMesh { .. }
                | ColliderShape::Terrain
                | ColliderShape::Edge { .. }
                | ColliderShape::Chain { .. }
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            ColliderShape::Box { .. } => "Box",
            ColliderShape::Sphere { .. } => "Sphere",
            ColliderShape::Capsule { .. } => "Capsule",
            ColliderShape::ConvexHull { .. } => "Convex hull",
            ColliderShape::TriangleMesh { .. } => "Triangle mesh",
            ColliderShape::Terrain => "Terrain",
            ColliderShape::Rect { .. } => "Rectangle",
            ColliderShape::Circle { .. } => "Circle",
            ColliderShape::Capsule2d { .. } => "Capsule 2D",
            ColliderShape::Polygon { .. } => "Polygon",
            ColliderShape::Edge { .. } => "Edge",
            ColliderShape::Chain { .. } => "Chain",
            ColliderShape::Tilemap => "Tilemap",
        }
    }
}

/// Where a collider's shape comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ColliderGeometry {
    /// A saved shape. The default for new colliders.
    Shape { shape: ColliderShape },
    /// Derived from the actor's Look every time it changes. The opt-in provider
    /// legacy projects migrate to, so they keep following their visual.
    FromLook,
}

/// Include and exclude layers laid over a collider's own layer and the project
/// matrix. A set bit is a layer slot (bit `n - 1` for layer `n`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LayerOverrides {
    #[serde(default)]
    pub include: u32,
    #[serde(default)]
    pub exclude: u32,
    /// Higher wins when two overrides disagree on a pair.
    #[serde(default)]
    pub priority: i32,
}

impl LayerOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Layer slots a project has.
pub const LAYER_SLOTS: u8 = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColliderSpec {
    #[serde(default)]
    pub id: ColliderId,
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub geometry: ColliderGeometry,
    /// Local offset of the shape from the actor's origin.
    #[serde(default)]
    pub center: [f32; 3],
    /// Local rotation of the shape, Euler degrees.
    #[serde(default)]
    pub rotation: [f32; 3],
    #[serde(default)]
    pub material: MaterialRef,
    #[serde(default, skip_serializing_if = "MaterialOverrides::is_empty")]
    pub material_overrides: MaterialOverrides,
    /// Senses overlap without pushing back.
    #[serde(default)]
    pub trigger: bool,
    /// Layer slot, 1 to [`LAYER_SLOTS`].
    #[serde(default = "default_layer")]
    pub layer: u8,
    #[serde(default, skip_serializing_if = "LayerOverrides::is_empty")]
    pub layer_overrides: LayerOverrides,
    /// The pre-matrix per-object mask a migrated collider keeps (bit `n - 1`
    /// for layer `n`) so asymmetric masks are not lost. `None` for new colliders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_mask: Option<u32>,
    /// Extra distance, in world units, at which contacts are generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_offset: Option<f32>,
    /// Whether queries (rays, overlaps, casts) see this collider.
    #[serde(default = "yes")]
    pub queryable: bool,
    /// 2D: supports actors from above and lets them through from below and the
    /// sides.
    #[serde(default)]
    pub one_way: bool,
}

fn default_layer() -> u8 {
    1
}

impl ColliderSpec {
    /// A solid, enabled collider of `shape` with a fresh id and every other
    /// setting at its default.
    pub fn new(shape: ColliderShape) -> Self {
        Self::with_geometry(ColliderGeometry::Shape { shape })
    }

    /// A collider that follows the actor's Look.
    pub fn from_look() -> Self {
        Self::with_geometry(ColliderGeometry::FromLook)
    }

    fn with_geometry(geometry: ColliderGeometry) -> Self {
        Self {
            id: ColliderId::generate(),
            name: String::new(),
            enabled: true,
            geometry,
            center: [0.0; 3],
            rotation: [0.0; 3],
            material: MaterialRef::Default,
            material_overrides: MaterialOverrides::default(),
            trigger: false,
            layer: 1,
            layer_overrides: LayerOverrides::default(),
            legacy_mask: None,
            contact_offset: None,
            queryable: true,
            one_way: false,
        }
    }

    /// The saved shape, if the geometry is authored.
    pub fn shape(&self) -> Option<&ColliderShape> {
        match &self.geometry {
            ColliderGeometry::Shape { shape } => Some(shape),
            ColliderGeometry::FromLook => None,
        }
    }
}

impl Default for ColliderSpec {
    fn default() -> Self {
        Self::from_look()
    }
}
