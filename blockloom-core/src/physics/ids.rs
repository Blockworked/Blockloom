//! Identities for repeatable physics components.
//!
//! A collider is addressed by its [`ColliderId`], never by its name or its place in
//! the actor's list, so a rename, a reorder, an undo or a save keeps pointing at the
//! same shape. Ids are unique within a scene (actor ids work the same way).

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// A fresh random id.
            pub fn generate() -> Self {
                Self(uuid::Uuid::new_v4().simple().to_string())
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            /// A new value gets a fresh id, so a spec written without one (a hand-made
            /// file, a shell command) is still addressable.
            fn default() -> Self {
                Self::generate()
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_string())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// Names one collider on one actor for the life of the document.
    ColliderId
);
id_type!(
    /// Names a single-instance physics component (a Rigidbody) so references to it
    /// survive the same edits a [`ColliderId`] does.
    ComponentId
);
id_type!(
    /// Names one constraint on one actor for the life of the document.
    ConstraintId
);
