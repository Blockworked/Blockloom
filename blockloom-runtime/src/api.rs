//! What an engine-tier (adapter) plugin builds against, re-exported by the
//! `blockloom-engine` facade. Unstable until Voxel and one more plugin use it.

use crate::module::BlockloomModule;
use std::sync::Mutex;

/// The Bevy this engine build is on, so a plugin never picks its own.
pub use bevy;
pub use blockloom_core;

pub use crate::engine::{ActorId, Dimension, PendingEffects, PluginComponents, SceneDocument};
pub use crate::module::{AddModule, Modules, RunState, WorldRebuilt};
pub use crate::world::{SimulationSet, WorldCamera, apply_common, clear_effects};
pub use blockloom_plugin_api::record::PluginRecord;

static ENGINE_MODULES: Mutex<Vec<Box<dyn BlockloomModule>>> = Mutex::new(Vec::new());

/// Queues a plugin's module for every world this process builds. A project
/// runtime calls it before `run_process`.
pub fn add_engine_module(module: impl BlockloomModule) {
    if let Ok(mut modules) = ENGINE_MODULES.lock() {
        modules.push(Box::new(module));
    }
}

/// Builds the queued modules into `app`, after the built-in ones.
pub(crate) fn build_engine_modules(app: &mut bevy::prelude::App) {
    let Ok(modules) = ENGINE_MODULES.lock() else {
        return;
    };
    for module in modules.iter() {
        module.build(app);
        app.world_mut()
            .get_resource_or_init::<Modules>()
            .0
            .push(module.name());
    }
}

/// Makes `entry` (a crate's `export_plugin!` entry) the code of `plugin` in
/// this process: its blocks, hooks, scene tools and preview run in-process
/// with no library or wasm module, through the same C ABI.
#[cfg(feature = "plugins")]
pub fn link_plugin(plugin: &str, entry: blockloom_plugin_api::abi::EntryFn) {
    blockloom_plugin_host::linked::register(plugin, entry);
}

/// A line in the editor's run log, from `actor` (empty for none).
pub fn say(actor: &str, text: &str) {
    crate::bridge::send(&blockloom_protocol::RuntimeMessage::Say {
        actor: actor.to_string(),
        text: text.to_string(),
    });
}
