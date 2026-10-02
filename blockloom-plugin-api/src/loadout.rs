//! What a running world needs to host its plugins' code.
//!
//! The editor (or a built game's pack) turns the active plugins into a
//! [`Loadout`]: for each plugin that runs code, which library or module to
//! open, what it may do, the hooks it registered and the blocks whose commands
//! are module ops, which the world runs itself instead of asking the editor.
//! Paths are whatever the sender could resolve; the world opens them as given.

use crate::generation::NodeSchema;
use crate::manifest::{Capability, PortableEntry};
use crate::schema::{FieldSchema, FieldType, HookSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// A native plugin's library, ready for loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeLibrary {
    pub path: PathBuf,
    /// The package's content hash; a changed package is a different module.
    pub hash: String,
    pub capabilities: BTreeSet<Capability>,
}

/// A portable plugin's verified module, ready for loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableLibrary {
    pub path: PathBuf,
    /// The package's content hash; a changed package is a different module.
    pub hash: String,
    pub capabilities: BTreeSet<Capability>,
    pub entry: PortableEntry,
}

/// The code a plugin runs on this machine: its native library when it has
/// one for the target, else its portable module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeRuntime {
    Native(NativeLibrary),
    Portable(PortableLibrary),
}

impl CodeRuntime {
    pub fn hash(&self) -> &str {
        match self {
            CodeRuntime::Native(n) => &n.hash,
            CodeRuntime::Portable(p) => &p.hash,
        }
    }
}

/// A block whose command is a module op: the world calls the op itself, with
/// the block's slots as the arguments. A reporter also says what it answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadoutBlock {
    pub type_id: String,
    pub op: String,
    /// The slots in the order an instruction's args follow.
    pub slots: Vec<FieldSchema>,
    /// Whether the command takes the running actor as `actor`.
    #[serde(default)]
    pub wants_actor: bool,
    /// Set for a reporter: what its answer is read as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<FieldType>,
}

/// One plugin as the world loads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadoutPlugin {
    pub id: String,
    pub runtime: CodeRuntime,
    #[serde(default)]
    pub hooks: Vec<HookSchema>,
    #[serde(default)]
    pub blocks: Vec<LoadoutBlock>,
    /// The plugin draws a preview in the scene view while nothing plays.
    #[serde(default)]
    pub preview: bool,
    /// The graph nodes its modules compute (see [`crate::generation`]).
    #[serde(default)]
    pub nodes: Vec<NodeSchema>,
}

/// Every plugin the world hosts, in dependency order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Loadout {
    pub plugins: Vec<LoadoutPlugin>,
}

impl Loadout {
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

/// The ops a module answers when a world hosts it, beyond the ops its own
/// commands name. A module that does not know one answers `Unsupported`, which
/// the world treats as "nothing to do".
pub mod ops {
    /// The world was built and the run began. Input: `{"plugin", "records",
    /// "resources", "preview"}`, the plugin's own records from the project.
    /// `preview` is true when the scene view hosts the module while nothing
    /// plays: it may draw, but hooks and blocks do not run.
    pub const START: &str = "world.start";
    /// The run ended; the module stays loaded until the world is dropped.
    pub const STOP: &str = "world.stop";
    /// Prefix of a hook's op: a hook named `tick` is called as `hook.tick`
    /// with `{"stage", "hook", "tick", "dt"}`.
    pub const HOOK_PREFIX: &str = "hook.";
}
