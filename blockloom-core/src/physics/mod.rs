//! The physics document: Rigidbody, Collider and the ownership that joins them.
//!
//! Phase 1 of `docs/physics-and-character-controller-plan.md`. This module is the
//! saved model and its rules and nothing else: specs ([`spec`]), reusable surfaces
//! ([`material`]), property metadata ([`meta`]), validation ([`validate`]), who
//! owns which shape ([`ownership`]), editing transactions ([`edit`]) and the
//! conversion of legacy `Body` components ([`migrate`]).
//!
//! The runtime does not read any of it yet (Phase 2 does): an actor's physics
//! still comes from its `Body` until a project is deliberately migrated, so opening,
//! playing and saving an existing project is unchanged. Run state (velocity,
//! contacts, sleeping) never appears here.

pub mod contacts;
pub mod edit;
pub mod geometry;
pub mod ids;
pub mod layers;
pub mod material;
pub mod meta;
pub mod migrate;
pub mod ops;
pub mod ownership;
pub mod plan;
pub mod spec;
pub mod validate;

pub use contacts::{
    ContactEvent, ContactKind, ContactPayload, ContactPhase, ContactPoint, ContactScope,
    ContactTracker, Endpoint, ExitReason,
};
pub use ids::{ColliderId, ComponentId};
pub use layers::{ColliderFilter, LayerSettings, layer_bit, needs_exact, pair_collides};
pub use material::{
    CombineMode, MaterialBody, MaterialDef, MaterialLibrary, MaterialOverrides, MaterialRef,
    PhysicsMaterial, PhysicsMaterial2D,
};
pub use migrate::{ActorMigration, shape_from_look};
pub use ops::{ForceMode, world_inertia};
pub use ownership::{BodyEntry, ColliderOwnership, LocalPose, PhysicsOwnership};
pub use plan::{BodyPlan, ColliderPlan, ExtraMass, PhysicsPlan, PlannedMaterial, PlannedShape};
pub use spec::{
    Axis, BodyType, ColliderGeometry, ColliderShape, ColliderSpec, CollisionDetection, Constraints,
    Interpolation, LayerOverrides, MassSource, RigidbodySpec,
};
pub use validate::{PhysicsIssue, Severity, validate_scene};

use serde::{Deserialize, Serialize};

/// The physics schema this build writes. A project with no physics components
/// and no stored materials is written exactly as before and reads as schema 0.
pub const SCHEMA_VERSION: u32 = 1;

/// Which collision behavior a project asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompatibilityProfile {
    /// The behavior projects had before this system: the runtime keeps the old
    /// trigger and static-touch quirks.
    #[default]
    Legacy,
    /// The documented Unity-style matrix. New projects use it once the runtime
    /// reads these components.
    Unity,
}

/// Project-wide physics settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PhysicsSettings {
    /// 0 for a project that has none of this yet.
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub profile: CompatibilityProfile,
    #[serde(default)]
    pub materials: MaterialLibrary,
    #[serde(default)]
    pub layers: LayerSettings,
}

impl PhysicsSettings {
    /// True when nothing here differs from a project that never heard of it, so
    /// the project file is written without the section.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Marks the project as using the current schema.
    pub fn stamp(&mut self) {
        self.schema_version = SCHEMA_VERSION;
    }

    /// An error for a document written by a newer Blockloom.
    pub fn check_version(&self) -> Result<(), String> {
        if self.schema_version > SCHEMA_VERSION {
            return Err(format!(
                "This project uses physics schema {}, but this Blockloom understands up to {SCHEMA_VERSION}. Update Blockloom to open it.",
                self.schema_version
            ));
        }
        Ok(())
    }
}
