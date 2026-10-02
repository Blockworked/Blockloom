//! Installs, resolves and runs plugin packages.
//!
//! - [`package`] reads and verifies a package folder; [`source`] and
//!   [`registry`] say where one comes from.
//! - [`resolver`] picks one version of each plugin for a project, and
//!   [`install`] applies the result as a transaction over the shared
//!   [`cache`] and the project's [`lock`] files.
//! - [`active`] is what a project's locked plugins contribute once loaded,
//!   and how a document's plugin records are checked against them.
//! - [`lifecycle`] and [`hooks`] order what happens when, and [`native`]
//!   loads a native module behind the C ABI. [`portable`] runs a WebAssembly
//!   module under the same contract with a memory and work budget, and
//!   [`module`] is either of them. [`world`] hosts a project's modules in a
//!   running game: lifecycle calls, hooks by stage and blocks that are ops.
//!   [`shipped`] loads what a built game carries, for its player. [`imports`]
//!   runs importers and build hooks over a project's files. [`trust`] is the
//!   per-user list of plugins allowed to run code in the editor. [`services`]
//!   answers a module's `blockloom.call`s: [`storage`] for its bytes and
//!   [`diagnostics`] for what it costs. [`isolated`] hosts a module in a
//!   worker process of its own when asked to.

pub mod active;
pub mod cache;
pub mod diagnostics;
pub mod files;
pub mod generation;
pub mod hooks;
pub mod imports;
pub mod install;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub mod isolated;
pub mod jobs;
pub mod lifecycle;
pub mod lock;
pub mod module;
pub mod native;
pub mod package;
pub mod portable;
pub mod registry;
pub mod resolver;
pub mod services;
pub mod shipped;
pub mod source;
pub mod storage;
pub mod trust;
pub mod world;
