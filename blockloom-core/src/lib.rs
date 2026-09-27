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
//! - [`material`] is the look beyond flat colors: surface materials, shader
//!   graphs, particles, trails and tilemaps.
//! - [`assets`] is the files inside one of those folders, which the editor's
//!   asset tray lists and the runtime loads images and fonts from.
//! - [`pack`] is that document again, as a built game carries it, and
//!   [`build`] is what lays one out: the player binary, the pack, the assets
//!   and the compiled scripts, in a folder that runs on its own.
//! - [`codegen`] compiles those same canvases into Rust instead, for a built
//!   game that runs native code rather than walking the tree.
//! - [`vm`] compiles those canvases into a flat program and runs every script
//!   cooperatively, one slice per rendered frame, emitting [`vm::Effect`]s for
//!   a host to apply. `blockloom-runtime` is that host.
//! - [`sense`] is the world state reporter blocks read, published by the host
//!   once a frame.
//! - [`sky`] is the 3D sky: its kind, where the sun and moon stand, and how
//!   much sunlight the atmosphere lets through.
//! - [`ui`] is the screen-space overlay a game builds out of blocks: what
//!   kinds of element there are, where one sits, and what a block can change.
//! - [`pipeline`] is the asset pipeline: model rigs, atlases, texture and
//!   audio compression plans, and reimport tracking.
//! - [`wire`] converts documents to and from the flat JSON shape the
//!   blockstitch frontend speaks.

pub mod ai;
pub mod animation;
pub mod assets;
pub mod blocks;
pub mod build;
pub mod codegen;
pub mod components;
pub mod decals;
mod distribution;
pub mod fields;
pub mod fog;
pub mod input;
pub mod library;
pub mod lightning;
pub mod material;
pub mod nav;
pub mod pack;
pub mod physics_query;
pub mod pipeline;
pub mod probe;
pub mod project;
pub mod rig2d;
pub mod save;
pub mod scene;
pub mod scene_components;
pub mod script;
pub mod sense;
pub mod shader_lib;
pub mod sky;
pub mod sound;
pub mod sprite2d;
pub mod sync;
pub mod terrain;
pub mod tilemap;
pub mod ui;
pub mod value;
pub mod vfs;
pub mod vfx;
pub mod vm;
pub mod vocabulary;
pub mod volume;
pub mod water;
pub mod web_build;
pub mod wind;
pub mod wire;

/// Registers Blockloom's own reporter blocks with blockstitch, so a saved
/// project using them evaluates. Every entry point calls it; twice is fine.
pub fn init() {
    value::register_blockloom_operators();
}

pub mod cloud_layers;
pub mod clouds;
