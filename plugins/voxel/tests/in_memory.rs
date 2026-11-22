//! A shipped engine plugin read only through the file reader: the package is
//! gone from disk, its manifest and schemas live in a table, its code is
//! linked into this process, and the loadout and the world still come up.
//!
//! Its own test binary, since the reader is installed once per process.

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
const HOST: &str = "x86_64-unknown-linux-gnu";

static FILES: OnceLock<BTreeMap<PathBuf, Vec<u8>>> = OnceLock::new();

fn from_table(path: &Path) -> io::Result<Vec<u8>> {
    FILES
        .get()
        .and_then(|table| table.get(path))
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not in the table"))
}

#[test]
fn a_linked_plugin_loads_from_memory_files() {
    blockloom_plugin_host::linked::register(ID, blockloom_voxel::blockloom_plugin_entry_v1);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(ID);
    package::copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("package"),
        &root,
    )
    .unwrap();
    package::seal(&root).unwrap();
    let sealed = Package::load(&root).unwrap();

    // What a build carries: the manifest and every file but the engine crate,
    // which is linked into the player.
    let home = Path::new("plugins").join(ID);
    let shipped: Vec<String> = sealed
        .manifest
        .files
        .keys()
        .filter(|f| !f.starts_with("engine/"))
        .cloned()
        .collect();
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
        HOST,
    )
    .unwrap();
    assert_eq!(loadout.plugins.len(), 1);
    let plugin = &loadout.plugins[0];
    assert!(matches!(plugin.runtime, CodeRuntime::Linked(_)));
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
        HOST,
    );
    assert!(refused.is_err());

    // The linked module opens and draws its world.
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
