//! Blockloom's engine library: everything about a project that neither the
//! editor window nor the renderer owns.
//!
//! - [`scene`] is the world a project describes - actors, their looks, their
//!   bodies, the camera, 2D or 3D.
//! - [`components`] is what an actor is made of: its place, look, body and
//!   camera as separate components the runtime turns into Bevy ones, plus the
//!   custom components a project invents.
//! - [`blocks`] is the block vocabulary ([`blocks::InstructionKind`]) and its
//!   `BlockKind` impl, the hinge onto `blockstitch-core`'s document model.
//! - [`project`] is the saved document: a [`scene::World`] plus one canvas
//!   per actor, and the folder it is saved in.
//! - [`library`] is the set of project folders the Dashboard lists.
//! - [`vm`] compiles those canvases into a flat program and runs every script
//!   cooperatively, one slice per rendered frame, emitting [`vm::Effect`]s for
//!   a host to apply. `blockloom-runtime` is that host.
//! - [`sense`] is the world state reporter blocks read, published by the host
//!   once a frame.
//! - [`wire`] converts documents to and from the flat JSON shape the
//!   blockstitch frontend speaks.

pub mod blocks;
pub mod components;
pub mod fields;
pub mod library;
pub mod project;
pub mod scene;
pub mod script;
pub mod sense;
pub mod value;
pub mod vm;
pub mod wire;

/// Registers Blockloom's own reporter blocks with blockstitch, so a saved
/// project using them evaluates. Every entry point calls it; twice is fine.
pub fn init() {
    value::register_blockloom_operators();
}
