//! GPU compute for plugins.
//!
//! [`check`] decides whether a kernel may run at all: it parses and validates
//! the WGSL, pins its bindings to the schema and proves every loop bounded.
//! The `engine` feature adds [`engine::ComputeEngine`], which owns the buffers
//! and runs queued commands on a device the caller hands in.

pub mod check;
#[cfg(feature = "engine")]
pub mod engine;

pub use check::{KernelInfo, check_kernel};
