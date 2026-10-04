//! The plugin contract: everything a package, the host and an author's
//! tooling must agree on, with no Qt, Bevy or file-system dependency.
//!
//! - [`manifest`] is `plugin.json`: identity, compatibility, dependencies,
//!   capabilities, targets and the hash of every payload file.
//! - [`schema`] is what a package contributes - component, resource, block
//!   and command definitions - and the validation of a payload against one.
//! - [`record`] is a plugin-owned record as a project document stores it,
//!   kept losslessly even when its plugin is missing.
//! - [`abi`] is the native C boundary: fixed-width, versioned, no Rust types.
//! - [`loadout`] is what a running world needs to host a plugin's code.
//! - [`mesh`] is a mesh a plugin submits for the world to draw.
//! - [`assets`] is what an importer or a build hook is asked and answers.
//! - [`compute`] is the GPU kernels a plugin ships and the buffer commands it may ask for.
//! - [`generation`] is typed graph nodes a plugin offers and the graphs built from them.
//! - [`surfaces`] is menu items, shortcuts and scene-view overlays.
//! - [`data`] is where a plugin's project data lives and how a build lists it.
//! - [`wasm`] is the same contract over a WebAssembly module's linear memory.
//!
//! Engine, SDK, plugin ABI, editor API, schema and shader API versions are
//! separate numbers (see [`versions`]) so a change to one does not look like a
//! change to the rest.

pub mod abi;
pub mod assets;
pub mod compute;
pub mod data;
pub mod generation;
pub mod id;
pub mod loadout;
pub mod lod;
pub mod manifest;
pub mod mesh;
pub mod record;
pub mod rendering;
pub mod schema;
pub mod surfaces;
pub mod wasm;

pub use semver::{self, Version, VersionReq};

/// The independent version axes a package is checked against.
pub mod versions {
    /// The C ABI in [`crate::abi`]. Bump only when that boundary changes.
    pub const PLUGIN_ABI: u32 = 1;
    /// The SDK a plugin was built against (manifest `sdk` range).
    pub const SDK: &str = "0.1.0";
    /// What editor contributions (inspector sections, panels) may declare.
    pub const EDITOR_API: u32 = 1;
    /// The shader import/pass contract a package's shaders are written to.
    pub const SHADER_API: u32 = 1;
    /// The `plugin.json` / `plugins.json` / `plugins.lock` layouts.
    pub const MANIFEST_FORMAT: u32 = 1;
}
