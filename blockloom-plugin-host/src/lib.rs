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
//!   loads a native module behind the C ABI.

pub mod active;
pub mod cache;
pub mod hooks;
pub mod install;
pub mod lifecycle;
pub mod lock;
pub mod native;
pub mod package;
pub mod registry;
pub mod resolver;
pub mod source;
