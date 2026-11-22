//! VOXEL as an engine-tier plugin: the project runtime links this crate, and
//! the module makes the voxel code the plugin's in-process module, so blocks,
//! hooks, scene tools and the preview run natively with no wasm in between.

use blockloom_engine::prelude::*;

pub const ID: &str = "com.blockworked.voxel";

pub struct VoxelModule;

impl BlockloomModule for VoxelModule {
    fn name(&self) -> &'static str {
        ID
    }

    fn build(&self, _app: &mut App) {
        blockloom_engine::link_plugin(ID, crate::blockloom_plugin_entry_v1);
    }
}

blockloom_engine::export_module!(VoxelModule);
