//! The voxel plugin through the real boundary: the C entry point in this
//! process, the world host that applies its effects, and the sealed package
//! with its WebAssembly module.
//!
//! The wasm tests build the crate with the toolchain running them. A missing
//! `wasm32-unknown-unknown` target skips them unless `BLOCKLOOM_REQUIRE_WASM`
//! is set, which CI does.

use blockloom_plugin_api::loadout::LoadoutBlock;
use blockloom_plugin_api::manifest::PortableEntry;
use blockloom_plugin_api::schema::{BlockKind, CommandAction, Contributions};
use blockloom_plugin_host::module::CodeModule;
use blockloom_plugin_host::native::{NativeModule, default_services};
use blockloom_plugin_host::package::{self, Package};
use blockloom_plugin_host::portable::PortableModule;
use blockloom_plugin_host::world::{Effect, Outcome, Preloaded, WorldPlugins};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const ID: &str = "com.blockworked.voxel";
const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn package_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("package")
}

fn contributions() -> Contributions {
    let text = std::fs::read_to_string(package_dir().join("schemas/voxel.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn blocks() -> Vec<LoadoutBlock> {
    let contributions = contributions();
    contributions
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Statement | BlockKind::Reporter))
        .map(|b| {
            let command = contributions
                .command(b.command.as_deref().unwrap())
                .unwrap();
            let CommandAction::Module { op } = &command.action else {
                panic!("the voxel blocks run module ops");
            };
            LoadoutBlock {
                type_id: b.type_id.clone(),
                op: op.clone(),
                slots: b.slots.clone(),
                wants_actor: false,
                returns: b.returns.clone(),
            }
        })
        .collect()
}

fn module() -> NativeModule {
    unsafe {
        NativeModule::from_entry(
            blockloom_voxel::blockloom_plugin_entry_v1,
            BTreeSet::new(),
            default_services("0.0.1".to_string()),
        )
    }
    .unwrap()
}

fn hosted(module: CodeModule) -> WorldPlugins {
    WorldPlugins::with_modules(vec![Preloaded {
        id: ID.to_string(),
        module,
        hooks: vec![],
        blocks: blocks(),
    }])
    .unwrap()
}

/// What a game hands the plugin when a run starts: the project's `world`
/// resource.
fn resources(payload: Value) -> impl Fn(&str) -> Value {
    move |_| {
        json!({"records": [], "resources": [{
            "type_id": "world", "schema_version": 1, "payload": payload.clone(),
        }]})
    }
}

/// The meshes a stream of outcomes leaves standing, by name.
#[derive(Default)]
struct Scene {
    meshes: BTreeMap<String, blockloom_plugin_api::mesh::MeshData>,
    said: Vec<String>,
}

impl Scene {
    fn apply(&mut self, outcomes: Vec<Outcome>) {
        for outcome in outcomes {
            match outcome {
                Outcome::Effect {
                    effect: Effect::Mesh(mesh),
                    ..
                } => {
                    self.meshes.insert(mesh.name.clone(), mesh);
                }
                Outcome::Effect {
                    effect: Effect::RemoveMesh { name },
                    ..
                } => {
                    self.meshes.remove(&name);
                }
                Outcome::Effect {
                    effect: Effect::Say { text },
                    ..
                } => self.said.push(text),
                Outcome::Log { .. } => {}
                other => panic!("unexpected outcome {other:?}"),
            }
        }
    }

    fn triangles(&self) -> usize {
        self.meshes.values().map(|m| m.indices.len() / 3).sum()
    }
}

fn small_flat() -> Value {
    json!({"preset": "flat", "seed": 1, "size": [32, 16, 32], "voxel_size": 0.5, "origin": [10, 0, -4]})
}

#[test]
fn the_package_schema_is_valid_and_its_blocks_resolve() {
    let contributions = contributions();
    contributions.check_definition().unwrap();
    assert_eq!(contributions.resources.len(), 1);
    assert_eq!(blocks().len(), 6);
    // Every statement and reporter has the op it names.
    let ops: BTreeSet<_> = blocks().into_iter().map(|b| b.op).collect();
    for op in ["set", "fill", "sphere", "generate", "get", "height"] {
        assert!(ops.contains(op), "{op}");
    }
}

#[test]
fn a_run_draws_the_world_and_edits_redraw_only_what_they_touch() {
    let mut world = hosted(CodeModule::Native(module()));
    let mut scene = Scene::default();
    scene.apply(world.start(&resources(small_flat())));
    // A flat 32x16x32 world is two chunks wide, one tall: four chunks, the
    // ground in each, and a few triangles apiece.
    assert_eq!(scene.meshes.len(), 4, "{:?}", scene.meshes.keys());
    assert!(scene.said[0].contains("32x16x32"), "{:?}", scene.said);
    let ground = scene.triangles();
    assert!(ground < 100, "{ground} triangles for a flat floor");
    let first = &scene.meshes["chunk/0/0/0"];
    assert_eq!(first.origin, [10.0, 0.0, -4.0]);
    assert!(first.collider);
    // A chunk is 16 cells of half a unit: positions stay within 8.
    assert!(first.positions.iter().all(|&p| (0.0..=8.0).contains(&p)));

    // A flat world of height 16 is ground up to y 4, grass on top.
    assert_eq!(
        world
            .read(ID, "ground_height", &[json!(5), json!(5)], "me")
            .unwrap(),
        json!(4)
    );
    assert_eq!(
        world
            .read(ID, "voxel_at", &[json!(5), json!(4), json!(5)], "me")
            .unwrap(),
        json!(3)
    );

    // A tower inside one chunk redraws that chunk alone.
    let before = scene.meshes.clone();
    let outcomes = world.run_block(
        ID,
        "fill_voxels",
        &[
            json!(2),
            json!(5),
            json!(2),
            json!(3),
            json!(8),
            json!(3),
            json!("wood"),
        ],
        "me",
    );
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    scene.apply(outcomes);
    assert_eq!(scene.meshes.len(), 4);
    assert_ne!(scene.meshes["chunk/0/0/0"], before["chunk/0/0/0"]);
    assert_eq!(scene.meshes["chunk/1/0/1"], before["chunk/1/0/1"]);

    // A cell on a chunk's edge redraws its neighbour too.
    let outcomes = world.run_block(
        ID,
        "set_voxel",
        &[json!(15), json!(5), json!(4), json!("stone")],
        "me",
    );
    assert_eq!(outcomes.len(), 2, "{outcomes:?}");

    // Digging a hole through the floor shows in the reporters.
    let outcomes = world.run_block(
        ID,
        "voxel_sphere",
        &[json!(20), json!(4), json!(20), json!(3), json!("air")],
        "me",
    );
    scene.apply(outcomes);
    assert_eq!(
        world
            .read(ID, "voxel_at", &[json!(20), json!(4), json!(20)], "me")
            .unwrap(),
        json!(0)
    );
    assert_eq!(
        world
            .read(ID, "ground_height", &[json!(20), json!(20)], "me")
            .unwrap(),
        json!(0)
    );

    // Glowing ore is its own mesh with emission.
    scene.apply(world.run_block(
        ID,
        "set_voxel",
        &[json!(1), json!(5), json!(1), json!("glow")],
        "me",
    ));
    let glow = &scene.meshes["chunk/0/0/0/glow7"];
    assert!(glow.emission.unwrap()[0] > 1.0);
    // Taking it away again retires the mesh.
    scene.apply(world.run_block(
        ID,
        "set_voxel",
        &[json!(1), json!(5), json!(1), json!("air")],
        "me",
    ));
    assert!(!scene.meshes.contains_key("chunk/0/0/0/glow7"));

    // Generating again replaces the world.
    scene.apply(world.run_block(ID, "generate_terrain", &[json!("empty"), json!(1)], "me"));
    assert!(scene.meshes.is_empty(), "{:?}", scene.meshes.keys());
    assert_eq!(
        world
            .read(ID, "ground_height", &[json!(5), json!(5)], "me")
            .unwrap(),
        json!(-1)
    );
}

#[test]
fn bad_settings_and_edits_are_refused_with_a_reason() {
    let mut world = hosted(CodeModule::Native(module()));
    // The reason is logged; the call itself reports that it failed.
    let outcomes = world.start(&resources(json!({"size": [4096, 64, 4096]})));
    let text = format!("{outcomes:?}");
    assert!(
        text.contains("1 to 1024") && text.contains("world.start"),
        "{text}"
    );

    let mut world = hosted(CodeModule::Native(module()));
    // Nothing is built until the game starts.
    let early = world.run_block(
        ID,
        "set_voxel",
        &[json!(0), json!(0), json!(0), json!("stone")],
        "me",
    );
    assert!(
        format!("{early:?}").contains("press Play first"),
        "{early:?}"
    );
    world.start(&resources(small_flat()));
    let unknown = world.run_block(
        ID,
        "set_voxel",
        &[json!(0), json!(0), json!(0), json!("lava")],
        "me",
    );
    let text = format!("{unknown:?}");
    assert!(text.contains("no material called lava"), "{text}");
    let preset = world.run_block(ID, "generate_terrain", &[json!("moon"), json!(1)], "me");
    assert!(format!("{preset:?}").contains("island"), "{preset:?}");
}

#[test]
fn a_default_world_is_what_an_absent_resource_means() {
    let mut world = hosted(CodeModule::Native(module()));
    let mut scene = Scene::default();
    scene.apply(world.start(&|_| json!({"records": [], "resources": []})));
    assert!(scene.said[0].contains("64x32x64"), "{:?}", scene.said);
    // 4 x 2 x 4 chunks, the island's air above and below left out.
    assert!(!scene.meshes.is_empty() && scene.meshes.len() <= 32);
    let again = {
        let mut other = hosted(CodeModule::Native(module()));
        let mut scene = Scene::default();
        scene.apply(other.start(&|_| json!({"records": [], "resources": []})));
        scene
    };
    // The same seed, the same meshes.
    assert_eq!(scene.meshes, again.meshes);
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
}

fn build_wasm() -> Option<PathBuf> {
    let installed = Command::new("rustc")
        .args(["--print", "target-libdir", "--target", WASM_TARGET])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .is_some_and(|dir| dir.exists());
    if !installed {
        assert!(
            std::env::var_os("BLOCKLOOM_REQUIRE_WASM").is_none(),
            "{WASM_TARGET} is required but not installed"
        );
        eprintln!("skipping: {WASM_TARGET} is not installed (rustup target add {WASM_TARGET})");
        return None;
    }
    let output = cargo()
        .args([
            "build",
            "--release",
            "--package",
            "blockloom-voxel",
            "--target",
            WASM_TARGET,
            "--message-format=json",
        ])
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "blockloom_voxel")
        .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|name| name.as_str().map(PathBuf::from))
        .find(|p| p.extension().is_some_and(|e| e == "wasm"))
}

fn portable(path: &Path, entry: &PortableEntry) -> PortableModule {
    PortableModule::load(
        &std::fs::read(path).unwrap(),
        entry,
        BTreeSet::new(),
        default_services("0.0.1".to_string()),
    )
    .unwrap()
}

/// The sealed package's module draws the same default world as the native
/// one, inside the manifest's own call budget.
#[test]
fn the_sealed_package_runs_in_the_portable_executor() {
    let Some(wasm) = build_wasm() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(ID);
    package::copy_dir(&package_dir(), &root).unwrap();
    std::fs::create_dir_all(root.join("portable")).unwrap();
    std::fs::copy(&wasm, root.join("portable/voxel.wasm")).unwrap();
    package::seal(&root).unwrap();
    let package = Package::load(&root).unwrap();
    assert_eq!(package.manifest.id, ID);
    assert_eq!(package.contributions.blocks.len(), 6);

    let entry = package.manifest.runtime.portable.clone().unwrap();
    let wasm_module = portable(&root.join(&entry.module), &entry);
    let mut world = hosted(CodeModule::Portable(Box::new(wasm_module)));
    let mut scene = Scene::default();
    let began = std::time::Instant::now();
    scene.apply(world.start(&|_| json!({"records": [], "resources": []})));
    println!(
        "voxel.wasm is {} bytes; the default world took {:?}",
        std::fs::metadata(&wasm).unwrap().len(),
        began.elapsed()
    );
    assert!(scene.said[0].contains("64x32x64"), "{:?}", scene.said);

    let mut native = hosted(CodeModule::Native(module()));
    let mut expected = Scene::default();
    expected.apply(native.start(&|_| json!({"records": [], "resources": []})));
    assert_eq!(scene.meshes, expected.meshes);
}
