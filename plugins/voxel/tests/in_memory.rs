//! A shipped plugin read only through the file reader, as a browser player
//! has it: the package is gone from disk, its bytes live in a table, and the
//! loadout and the world still come up.
//!
//! Its own test binary, since the reader is installed once per process.

mod common;

use blockloom_plugin_api::loadout::CodeRuntime;
use blockloom_plugin_host::files;
use blockloom_plugin_host::package::{self, Package};
use blockloom_plugin_host::shipped::{Shipped, shipped_loadout};
use blockloom_plugin_host::world::{Effect, Outcome, WorldPlugins};
use serde_json::json;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const ID: &str = "com.blockworked.voxel";

static FILES: OnceLock<BTreeMap<PathBuf, Vec<u8>>> = OnceLock::new();

fn from_table(path: &Path) -> io::Result<Vec<u8>> {
    FILES
        .get()
        .and_then(|table| table.get(path))
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not in the table"))
}

#[test]
fn a_plugin_loads_from_memory_files_for_the_web_target() {
    let Some(wasm) = common::build_wasm() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(ID);
    package::copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("package"),
        &root,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("portable")).unwrap();
    std::fs::copy(&wasm, root.join("portable/voxel.wasm")).unwrap();
    package::seal(&root).unwrap();
    let sealed = Package::load(&root).unwrap();

    // What a web build puts in the page: the manifest and every shipped file
    // under the plugin's folder.
    let home = Path::new("plugins").join(ID);
    let shipped: Vec<String> = sealed.manifest.files.keys().cloned().collect();
    let mut table = BTreeMap::new();
    for name in std::iter::once("plugin.json").chain(shipped.iter().map(String::as_str)) {
        table.insert(home.join(name), std::fs::read(root.join(name)).unwrap());
    }
    drop(dir);
    assert!(FILES.set(table).is_ok());
    files::set_reader(from_table);

    let loadout = shipped_loadout(
        &[Shipped {
            id: ID,
            dir: &home,
            hash: &sealed.content_hash,
            files: &shipped,
        }],
        "wasm32-unknown-unknown",
    )
    .unwrap();
    assert_eq!(loadout.plugins.len(), 1);
    let plugin = &loadout.plugins[0];
    assert!(matches!(plugin.runtime, CodeRuntime::Portable(_)));
    assert!(plugin.preview, "the manifest asks for a preview");

    // A damaged table is refused, not run.
    let mut broken = shipped.clone();
    broken.push("schemas/missing.json".to_string());
    let refused = shipped_loadout(
        &[Shipped {
            id: ID,
            dir: &home,
            hash: &sealed.content_hash,
            files: &broken,
        }],
        "wasm32-unknown-unknown",
    );
    assert!(refused.is_err());

    // The module opens through the same reader and draws its world.
    let mut world = WorldPlugins::load(&loadout, "0.0.1");
    let outcomes = world.start(&|_| json!({"records": [], "resources": [], "preview": false}));
    let meshes = outcomes
        .iter()
        .filter(|o| {
            matches!(
                o,
                Outcome::Effect {
                    effect: Effect::Mesh(_),
                    ..
                }
            )
        })
        .count();
    assert!(meshes > 4, "{meshes} meshes, outcomes: {outcomes:?}");
}
