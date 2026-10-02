//! The example run both ways: as the C entry point in this process, as a real
//! native library, and as the WebAssembly module a package ships.
//!
//! The wasm and cdylib tests build the crate with the toolchain running them.
//! A missing `wasm32-unknown-unknown` target skips the wasm ones unless
//! `BLOCKLOOM_REQUIRE_WASM` is set, which CI does.

use blockloom_plugin_api::manifest::PortableEntry;
use blockloom_plugin_host::native::{NativeModule, default_services};
use blockloom_plugin_host::package::{self, Package};
use blockloom_plugin_host::portable::PortableModule;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
}

/// Builds this crate (for `target`, or the host) and returns the artifact
/// that is not an rlib.
fn build(target: Option<&str>) -> PathBuf {
    let mut command = cargo();
    command.args([
        "build",
        "--release",
        "--package",
        "blockloom-example-tally",
        "--message-format=json",
    ]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().expect("cargo runs");
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| {
            message["reason"] == "compiler-artifact"
                && message["target"]["name"] == "blockloom_example_tally"
        })
        .flat_map(|message| message["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|name| name.as_str().map(PathBuf::from))
        .find(|path| path.extension().is_some_and(|e| e != "rlib" && e != "d"))
        .expect("the build reported a library")
}

/// The wasm module, or `None` when the target is not installed here.
fn wasm() -> Option<PathBuf> {
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
    Some(build(Some(WASM_TARGET)))
}

fn entry() -> blockloom_plugin_api::abi::EntryFn {
    blockloom_example_tally::blockloom_plugin_entry_v1
}

fn portable(path: &Path) -> PortableModule {
    PortableModule::load(
        &std::fs::read(path).unwrap(),
        &PortableEntry {
            module: "portable/tally.wasm".to_string(),
            memory_limit_mib: 16,
            call_limit_ms: 20,
        },
        BTreeSet::new(),
        default_services("0.0.1".to_string()),
    )
    .unwrap()
}

/// What every way of running the plugin has to agree on.
fn exercise(mut call: impl FnMut(&str, Value) -> Result<Value, String>) {
    assert_eq!(
        call("add", json!({"name": "coins", "by": 3})).unwrap(),
        json!({"count": 3})
    );
    // `by` defaults to 1, and the state is kept between calls.
    assert_eq!(
        call("add", json!({"name": "coins"})).unwrap(),
        json!({"count": 4})
    );
    call("add", json!({"name": "lives", "by": 2})).unwrap();
    assert_eq!(
        call("get", json!({"name": "coins"})).unwrap(),
        json!({"count": 4, "value": 4})
    );
    assert_eq!(
        call("get", json!({"name": "nothing"})).unwrap(),
        json!({"count": 0, "value": 0})
    );
    assert_eq!(
        call("all", Value::Null).unwrap(),
        json!({"counts": {"coins": 4, "lives": 2}})
    );
    // A host service, answered through the boundary.
    assert_eq!(
        call("engine", Value::Null).unwrap(),
        json!({"engine": "0.0.1", "abi": 1})
    );
    // Refusals carry their status.
    assert!(
        call("add", json!({"by": 1}))
            .unwrap_err()
            .contains("BadArgument")
    );
    assert!(
        call("fly", Value::Null)
            .unwrap_err()
            .contains("Unsupported")
    );
    assert_eq!(call("reset", Value::Null).unwrap(), Value::Null);
    assert_eq!(call("all", Value::Null).unwrap(), json!({"counts": {}}));
    // As a hosted world module: a run's end says what it counted.
    call("add", json!({"name": "wins", "by": 2})).unwrap();
    assert_eq!(
        call("world.stop", Value::Null).unwrap(),
        json!({"effects": [{"effect": "say", "text": "final tally: wins = 2"}]})
    );
    assert_eq!(call("world.start", Value::Null).unwrap(), Value::Null);
    assert_eq!(call("all", Value::Null).unwrap(), json!({"counts": {}}));
    // Once a run hosts it, a change fires the event the hat listens for.
    assert_eq!(
        call("add", json!({"name": "wins", "by": 5})).unwrap(),
        json!({"count": 5, "effects": [
            {"effect": "event", "name": "changed", "args": ["wins", 5]}
        ]})
    );
    assert!(call("world.stop", Value::Null).unwrap()["effects"].is_array());
}

#[test]
fn the_entry_point_runs_in_this_process() {
    let module = unsafe {
        NativeModule::from_entry(
            entry(),
            BTreeSet::new(),
            default_services("0.0.1".to_string()),
        )
    }
    .unwrap();
    exercise(|op, args| module.call_json(op, &args));
    let logs = module.take_logs();
    assert_eq!(logs[0], (3, "tally started".to_string()));
    // A refusal is logged with its reason as well as returned.
    assert!(
        logs.iter()
            .any(|(level, text)| *level == 1 && text.contains("name must be some text")),
        "{logs:?}"
    );
}

#[test]
fn the_built_native_library_loads_and_runs() {
    let library = build(None);
    let module = NativeModule::load(
        &library,
        BTreeSet::new(),
        default_services("0.0.1".to_string()),
    )
    .unwrap();
    exercise(|op, args| module.call_json(op, &args));
}

#[test]
fn the_wasm_module_runs_in_the_portable_executor() {
    let Some(path) = wasm() else { return };
    let mut module = portable(&path);
    exercise(|op, args| module.call_json(op, &args));
    let logs = module.take_logs();
    assert_eq!(logs[0], (3, "tally started".to_string()));
    assert!(!module.is_stopped());
    let size = std::fs::metadata(&path).unwrap().len();
    println!("tally.wasm is {size} bytes");
}

#[test]
fn the_package_seals_and_verifies_with_its_wasm() {
    let Some(wasm) = wasm() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("com.example.tally");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("package");
    package::copy_dir(&source, &root).unwrap();
    std::fs::create_dir_all(root.join("portable")).unwrap();
    std::fs::copy(&wasm, root.join("portable/tally.wasm")).unwrap();
    package::seal(&root).unwrap();
    let package = Package::load(&root).unwrap();
    assert_eq!(package.manifest.id, "com.example.tally");
    assert_eq!(package.contributions.commands.len(), 4);
    assert_eq!(package.contributions.blocks.len(), 3);
    assert!(package.target_hashes().contains_key("portable"));
    // The sealed module is the one the manifest names, and it runs.
    let entry = package.manifest.runtime.portable.as_ref().unwrap();
    let mut module = portable(&root.join(&entry.module));
    assert_eq!(
        module.call_json("get", &json!({"name": "x"})).unwrap(),
        json!({"count": 0, "value": 0})
    );
}

/// The package's own schema, run the way a game hosts it: the reporter is
/// asked on demand and the hat's event comes back from `add`.
#[test]
fn the_reporter_and_hat_work_in_a_hosted_world() {
    use blockloom_plugin_api::loadout::LoadoutBlock;
    use blockloom_plugin_api::schema::{BlockKind, CommandAction, Contributions};
    use blockloom_plugin_host::module::CodeModule;
    use blockloom_plugin_host::world::{Effect, Outcome, Preloaded, WorldPlugins};

    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("package/schemas/tally.json");
    let contributions: Contributions =
        serde_json::from_str(&std::fs::read_to_string(schema).unwrap()).unwrap();
    contributions.check_definition().unwrap();
    let blocks: Vec<LoadoutBlock> = contributions
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Statement | BlockKind::Reporter))
        .map(|b| {
            let command = contributions
                .command(b.command.as_deref().unwrap())
                .unwrap();
            let CommandAction::Module { op } = &command.action else {
                panic!("the tally's commands are module ops");
            };
            LoadoutBlock {
                type_id: b.type_id.clone(),
                op: op.clone(),
                slots: b.slots.clone(),
                wants_actor: false,
                returns: b.returns.clone(),
            }
        })
        .collect();
    let hat = contributions
        .blocks
        .iter()
        .find(|b| b.type_id == "changed")
        .unwrap();
    assert_eq!(hat.event.as_deref(), Some("changed"));

    let module = unsafe {
        NativeModule::from_entry(
            entry(),
            BTreeSet::new(),
            default_services("0.0.1".to_string()),
        )
    }
    .unwrap();
    let mut world = WorldPlugins::with_modules(vec![Preloaded {
        id: "com.example.tally".to_string(),
        module: CodeModule::Native(module),
        hooks: vec![],
        blocks,
    }])
    .unwrap();
    let id = "com.example.tally";
    world.start(&|_| json!({"records": [], "resources": []}));

    // The reporter answers an integer, and the same question twice is one call.
    assert_eq!(
        world.read(id, "count", &[json!("coins")], "me").unwrap(),
        json!(0)
    );
    let outcomes = world.run_block(id, "add", &[json!(4), json!("coins")], "me");
    assert_eq!(
        outcomes,
        [Outcome::Effect {
            plugin: id.to_string(),
            effect: Effect::Event {
                name: "changed".to_string(),
                actor: None,
                args: vec![json!("coins"), json!(4)],
            }
        }]
    );
    // The add forgot what had been read.
    assert_eq!(
        world.read(id, "count", &[json!("coins")], "me").unwrap(),
        json!(4)
    );
}

/// What a build does and the player undoes: the sealed package's files and
/// manifest copied under a game folder, then loaded from there by the pack's
/// record of them and hosted like a run.
#[test]
fn a_shipped_copy_loads_into_a_world_and_a_damaged_one_does_not() {
    use blockloom_plugin_host::shipped::{Shipped, shipped_loadout};
    use blockloom_plugin_host::world::WorldPlugins;

    let Some(wasm) = wasm() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sealed");
    package::copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("package"),
        &root,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("portable")).unwrap();
    std::fs::copy(&wasm, root.join("portable/tally.wasm")).unwrap();
    package::seal(&root).unwrap();
    let sealed = Package::load(&root).unwrap();

    // The game folder, as `build::copy_plugins` lays it out.
    let game = dir.path().join("game/plugins/com.example.tally");
    let files: Vec<String> = sealed.manifest.files.keys().cloned().collect();
    for file in files.iter().map(String::as_str).chain(["plugin.json"]) {
        let to = game.join(file);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(root.join(file), to).unwrap();
    }
    fn shipped<'a>(game: &'a Path, files: &'a [String], hash: &'a str) -> Vec<Shipped<'a>> {
        vec![Shipped {
            id: "com.example.tally",
            dir: game,
            hash,
            files,
        }]
    }
    let target = "x86_64-unknown-linux-gnu";
    let loadout = shipped_loadout(&shipped(&game, &files, &sealed.content_hash), target).unwrap();
    assert_eq!(loadout.plugins.len(), 1);
    let blocks: Vec<_> = loadout.plugins[0]
        .blocks
        .iter()
        .map(|b| b.type_id.as_str())
        .collect();
    assert_eq!(
        blocks,
        ["add", "count"],
        "statements and reporters that are module ops"
    );

    let mut world = WorldPlugins::load(&loadout, "0.0.1");
    let id = "com.example.tally";
    world.start(&|_| json!({"records": [], "resources": []}));
    world.run_block(id, "add", &[json!(3), json!("coins")], "me");
    assert_eq!(
        world.read(id, "count", &[json!("coins")], "me").unwrap(),
        json!(3)
    );
    assert!(world.drain().is_empty(), "nothing went wrong");

    // Not the package the game was built with.
    let wrong = shipped_loadout(&shipped(&game, &files, "0000"), target).unwrap_err();
    assert!(
        wrong[0].contains("not the package the game was built with"),
        "{wrong:?}"
    );
    // A file changed after the build.
    std::fs::write(game.join("portable/tally.wasm"), b"\0asm tampered").unwrap();
    let damaged =
        shipped_loadout(&shipped(&game, &files, &sealed.content_hash), target).unwrap_err();
    assert!(
        damaged[0].contains("does not match its declared hash"),
        "{damaged:?}"
    );
}
