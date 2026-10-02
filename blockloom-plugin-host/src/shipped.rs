//! A built game's plugins, loaded from what the build shipped.
//!
//! The player has no project lock or cache: it has `game/plugins/<id>/`, the
//! pack's record of what each plugin was built as, and its own target.
//! [`shipped_loadout`] verifies each plugin against that record and says what
//! the world should host, exactly as the editor's [`ActivePlugins::loadout`]
//! does for Play.
//!
//! [`ActivePlugins::loadout`]: crate::active::ActivePlugins::loadout

use crate::active::{code_runtime_of, loadout_plugin};
use crate::package::Package;
use blockloom_plugin_api::loadout::Loadout;
use blockloom_plugin_api::manifest::Tier;
use std::path::Path;

/// One plugin as the pack records it.
pub struct Shipped<'a> {
    pub id: &'a str,
    /// The folder the build copied its files to.
    pub dir: &'a Path,
    /// The content hash the game was built with.
    pub hash: &'a str,
    /// The package paths that were copied.
    pub files: &'a [String],
}

/// What a world hosts for the plugins a game shipped, for `target`. A plugin
/// with no code is left out; one with code that is damaged, or that has no
/// artifact for this target, fails the whole load, since a game that quietly
/// ran without its plugin would be wrong rather than slow.
pub fn shipped_loadout(plugins: &[Shipped], target: &str) -> Result<Loadout, Vec<String>> {
    let mut loaded = Vec::new();
    let mut shaders = Vec::new();
    let mut kernels = Vec::new();
    let mut errors = Vec::new();
    for shipped in plugins {
        let package = match Package::load_shipped(shipped.dir, shipped.files, shipped.hash) {
            Ok(package) => package,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        match package.shader_modules() {
            Ok(modules) => shaders.extend(modules),
            Err(e) => errors.push(e),
        }
        match package.kernels() {
            Ok(found) => kernels.extend(found),
            Err(e) => errors.push(e),
        }
        if package.manifest.tier == Tier::Declarative {
            continue;
        }
        match code_runtime_of(shipped.id, &package, target) {
            Ok(runtime) => loaded.push(loadout_plugin(shipped.id, &package, runtime)),
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(Loadout {
            plugins: loaded,
            shaders,
            kernels,
        })
    } else {
        Err(errors)
    }
}
