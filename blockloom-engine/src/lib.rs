//! The one crate an engine-tier (adapter) plugin depends on. A plugin is a
//! `BlockloomModule` named with `export_module!`; a project runtime links
//! it and calls `run`. Unstable: it moves with each engine build.

pub use blockloom_runtime::api::*;
pub use blockloom_runtime::module::BlockloomModule;
pub use {blockloom_plugin_api as plugin_api, blockloom_plugin_sdk as sdk};

/// What a plugin's `runtime.adapter.engine_build` must name.
pub const ENGINE_BUILD: &str = env!("CARGO_PKG_VERSION");

pub mod prelude {
    pub use crate::bevy::prelude::*;
    pub use crate::{
        ActorId, AddModule, BlockloomModule, PluginComponents, PluginRecord, RunState,
        SceneDocument, WorldRebuilt,
    };
}

/// Names a plugin's module, so a generated project runtime can find it as
/// `<crate>::blockloom_module()`.
#[macro_export]
macro_rules! export_module {
    ($module:expr) => {
        pub fn blockloom_module() -> impl $crate::BlockloomModule {
            $module
        }
    };
}

/// Starts the world process with these modules added: a project runtime's
/// whole `main`.
pub fn run(modules: impl FnOnce()) {
    modules();
    blockloom_runtime::run_process();
}
