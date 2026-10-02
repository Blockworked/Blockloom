use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};
use std::path::PathBuf;

/// One data dir for every test here: it is a process-wide env var.
fn data_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("blockloom-plugins-{}", uuid::Uuid::new_v4()));
        unsafe {
            std::env::set_var("BLOCKLOOM_DATA_DIR", root.join("data"));
            std::env::set_var("BLOCKLOOM_PLUGINS_OFFLINE", "0");
        }
        root
    })
    .clone()
}

fn example() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../plugins/examples/com.example.health")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_plugin_installs_owns_records_and_leaves_them_when_removed() {
    let root = data_root().join("plugged");
    let backend = Backend::start(AppHandle::new(|_| {}));
    let call = |cmd: &str, args: Value| backend.dispatch(cmd, args);
    let invoke = |cmd: &str, args: Value| call(cmd, args).unwrap();
    invoke(
        "create_project",
        json!({"name": "Plugged", "mode": "TwoD", "location": root.join("projects")}),
    );
    let ball = invoke("add_actor", json!({"shape": "Circle", "name": "Ball"}));
    let ball = ball.as_str().unwrap_or_default().to_owned();

    // Inspecting a folder verifies it and names it without installing.
    let seen = invoke("plugin_inspect", json!({"path": example()}));
    assert_eq!(seen["id"], "com.example.health");
    assert_eq!(seen["blocks"], 1);
    assert!(
        invoke("plugin_list", json!({}))["installed"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(call("plugin_inspect", json!({"path": root.join("nowhere")})).is_err());

    // The example is sealed, so the dry run resolves and verifies it.
    let dry = invoke(
        "plugin_install",
        json!({"id": "com.example.health", "source": format!("path:{}", example()), "dryRun": true}),
    );
    assert_eq!(dry["dryRun"], true);
    assert!(
        invoke("plugin_list", json!({}))["installed"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    invoke(
        "plugin_install",
        json!({"id": "com.example.health", "source": format!("path:{}", example())}),
    );
    let list = invoke("plugin_list", json!({}));
    assert_eq!(list["installed"][0]["id"], "com.example.health");

    // A command the plugin declares is reachable by name and checked by its schema.
    let commands = invoke("plugin_commands", json!({}));
    assert!(commands.to_string().contains("com.example.health/set_hp"));
    invoke(
        "plugin_call",
        json!({"command": "com.example.health/give_health", "args": {"actor": ball, "hp": 40}}),
    );
    invoke(
        "plugin_call",
        json!({"command": "com.example.health/set_hp", "args": {"actor": ball, "value": 25}}),
    );
    assert!(
        call(
            "plugin_call",
            json!({"command": "com.example.health/set_hp", "args": {"actor": ball, "value": 5000}})
        )
        .is_err()
    );
    invoke(
        "plugin_call",
        json!({"command": "com.example.health/set_difficulty", "args": {"damage_scale": 2}}),
    );
    assert_eq!(invoke("plugin_check", json!({}))["canRun"], true);

    // Removing the plugin keeps what it owned, and says so.
    let removal = invoke("plugin_remove", json!({"id": "com.example.health"}));
    assert!(!removal["keptRecords"].as_array().unwrap().is_empty());
    let check = invoke("plugin_check", json!({}));
    assert_eq!(check["canRun"], false);
    assert!(call("run_project", json!({})).is_err());

    // The records survive a save and reload untouched, and rolling back heals them.
    let state = invoke("get_state", json!({}));
    let dir = PathBuf::from(state["project_path"].as_str().unwrap());
    let saved = blockloom_core::project::read_project_dir(&dir).unwrap();
    assert_eq!(saved.plugin_ids().len(), 1);
    invoke("plugin_rollback", json!({}));
    assert_eq!(invoke("plugin_check", json!({}))["canRun"], true);
}

#[test]
fn a_native_plugin_command_runs_in_its_module() {
    use blockloom_plugin_host::native::fixture;
    let root = data_root().join("native");
    let triple = blockloom_core::build::host()
        .expect("a supported host")
        .triple
        .to_string();
    let pkg = root.join("pkg");
    std::fs::create_dir_all(pkg.join("lib")).unwrap();
    std::fs::create_dir_all(pkg.join("schemas")).unwrap();
    let built = fixture::build(&root, fixture::SOURCE);
    let library = format!("lib/{}", built.file_name().unwrap().to_string_lossy());
    std::fs::copy(&built, pkg.join(&library)).unwrap();
    std::fs::write(
        pkg.join("schemas/commands.json"),
        json!({"commands": [{
            "name": "echo", "summary": "Answer with the arguments.",
            "args": [{"name": "word", "type": "text", "default": "hi"}],
            "action": {"do": "module", "op": "echo"}
        }, {
            "name": "boom", "summary": "A module call that fails.",
            "action": {"do": "module", "op": "panic"}
        }]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pkg.join("plugin.json"),
        json!({
            "format": 1, "id": "com.example.native", "name": "Native", "version": "1.0.0",
            "engine": ">=0.0.1", "tier": "native", "abi": 1, "sdk": "^0.1",
            "capabilities": ["native-execution"],
            "runtime": {"native": {triple: {"library": library}}},
            "contributions": ["schemas/commands.json"]
        })
        .to_string(),
    )
    .unwrap();

    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd: &str, args: Value| backend.dispatch(cmd, args).unwrap();
    invoke("plugin_seal", json!({"path": pkg.to_string_lossy()}));
    invoke(
        "create_project",
        json!({"name": "Native", "mode": "TwoD", "location": root.join("projects")}),
    );
    invoke(
        "plugin_install",
        json!({"id": "com.example.native", "source": format!("path:{}", pkg.display())}),
    );
    let answer = invoke(
        "plugin_call",
        json!({"command": "com.example.native/echo", "args": {"word": "loom"}}),
    );
    assert_eq!(answer["word"], "loom");
    // A panic in the module comes back as an error, not a dead editor.
    assert!(
        backend
            .dispatch("plugin_call", json!({"command": "com.example.native/boom"}))
            .is_err()
    );
    // And the module is still usable afterwards.
    let again = invoke("plugin_call", json!({"command": "com.example.native/echo"}));
    assert_eq!(again["word"], "hi");
}

#[test]
fn a_plugin_block_runs_its_command_with_its_slots() {
    let root = data_root().join("blocks");
    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd: &str, args: Value| backend.dispatch(cmd, args).unwrap();
    invoke(
        "create_project",
        json!({"name": "Blocks", "mode": "TwoD", "location": root.join("projects")}),
    );
    let ball = invoke("add_actor", json!({"shape": "Circle", "name": "Ball"}));
    let ball = ball.as_str().unwrap_or_default().to_owned();
    // No plugin yet: a block with nothing behind it is an error, not a no-op.
    assert!(
        backend
            .dispatch(
                "plugin_run_block",
                json!({"plugin": "com.example.health", "block": "set_hp", "args": ["Ball", 25]}),
            )
            .is_err()
    );
    invoke(
        "plugin_install",
        json!({"id": "com.example.health", "source": format!("path:{}", example())}),
    );
    // The palette draws blocks from this shape (`Blocks.qml`'s plugin section).
    let shown = invoke("get_state", json!({}))["plugins"]["blocks"].clone();
    let block = &shown[0]["block"];
    assert_eq!(shown[0]["plugin"], "com.example.health");
    assert_eq!(block["type_id"], "set_hp");
    assert_eq!(block["kind"], "statement");
    assert_eq!(block["category"], "Health");
    assert_eq!(block["label"], "set {actor} health to {value}");
    assert_eq!(block["slots"][0]["name"], "actor");
    assert_eq!(block["slots"][0]["type"], "actor");
    assert_eq!(block["slots"][1]["type"], "int");
    assert_eq!(block["slots"][1]["default"], 100);
    assert_eq!(block["slots"][1]["min"], 0);
    // The actor slot takes a name; a whole number satisfies an int field.
    invoke(
        "plugin_run_block",
        json!({"plugin": "com.example.health", "block": "set_hp", "args": ["Ball", 25], "actor": ball}),
    );
    let state = invoke("get_state", json!({}));
    let dir = PathBuf::from(state["project_path"].as_str().unwrap());
    let saved = blockloom_core::project::read_project_dir(&dir).unwrap();
    let hp = saved
        .plugin_records()
        .into_iter()
        .find(|(_, r)| r.type_id == "Health")
        .map(|(_, r)| r.payload["hp"].clone());
    assert_eq!(hp, Some(json!(25)));
    // The wrong number of slots, and an unknown block, are refused by name.
    let few = backend.dispatch(
        "plugin_run_block",
        json!({"plugin": "com.example.health", "block": "set_hp", "args": ["Ball"]}),
    );
    assert!(few.unwrap_err().contains("2 slots"));
    assert!(
        backend
            .dispatch(
                "plugin_run_block",
                json!({"plugin": "com.example.health", "block": "nope", "args": []}),
            )
            .is_err()
    );
}

#[test]
fn a_portable_plugin_command_runs_in_its_wasm_module() {
    let root = data_root().join("portable");
    let pkg = root.join("pkg");
    std::fs::create_dir_all(pkg.join("portable")).unwrap();
    std::fs::create_dir_all(pkg.join("schemas")).unwrap();
    std::fs::write(
        pkg.join("portable/m.wasm"),
        blockloom_plugin_host::portable::fixture::wasm(),
    )
    .unwrap();
    std::fs::write(
        pkg.join("schemas/commands.json"),
        json!({"commands": [{
            "name": "echo", "summary": "Answer with the arguments.",
            "args": [{"name": "word", "type": "text", "default": "hi"}],
            "action": {"do": "module", "op": "echo"}
        }, {
            "name": "spin", "summary": "A module call that never returns.",
            "action": {"do": "module", "op": "spin"}
        }, {
            "name": "chatter", "summary": "A module call that logs.",
            "action": {"do": "module", "op": "log"}
        }]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pkg.join("plugin.json"),
        json!({
            "format": 1, "id": "com.example.portable", "name": "Portable", "version": "1.0.0",
            "engine": ">=0.0.1", "tier": "portable", "abi": 1, "sdk": "^0.1",
            "runtime": {"portable": {"module": "portable/m.wasm", "call_limit_ms": 10}},
            "contributions": ["schemas/commands.json"]
        })
        .to_string(),
    )
    .unwrap();

    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd: &str, args: Value| backend.dispatch(cmd, args).unwrap();
    invoke("plugin_seal", json!({"path": pkg.to_string_lossy()}));
    invoke(
        "create_project",
        json!({"name": "Portable", "mode": "TwoD", "location": root.join("projects")}),
    );
    invoke(
        "plugin_install",
        json!({"id": "com.example.portable", "source": format!("path:{}", pkg.display())}),
    );
    let answer = invoke(
        "plugin_call",
        json!({"command": "com.example.portable/echo", "args": {"word": "loom"}}),
    );
    assert_eq!(answer["word"], "loom");
    // A runaway call is stopped by its budget and says so.
    let error = backend
        .dispatch(
            "plugin_call",
            json!({"command": "com.example.portable/spin"}),
        )
        .unwrap_err();
    assert!(error.contains("10 ms of work"), "{error}");
    // The stopped module is replaced by a fresh one for the next call.
    let again = invoke(
        "plugin_call",
        json!({"command": "com.example.portable/echo"}),
    );
    assert_eq!(again["word"], "hi");
    // What it logs reaches the run log.
    invoke(
        "plugin_call",
        json!({"command": "com.example.portable/chatter"}),
    );
    assert!(
        invoke("get_state", json!({}))
            .to_string()
            .contains("hello from wasm")
    );
}

/// The palette example's wasm module, or `None` when the target is missing.
fn palette_wasm() -> Option<PathBuf> {
    use std::process::Command;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let installed = Command::new("rustc")
        .args([
            "--print",
            "target-libdir",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .is_some_and(|dir| dir.exists());
    if !installed {
        assert!(
            std::env::var_os("BLOCKLOOM_REQUIRE_WASM").is_none(),
            "wasm32-unknown-unknown is required but not installed"
        );
        return None;
    }
    let output = Command::new(cargo)
        .args([
            "build",
            "--release",
            "--package",
            "blockloom-example-palette",
            "--target",
            "wasm32-unknown-unknown",
            "--message-format=json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|name| name.as_str().map(PathBuf::from))
        .find(|path| path.extension().is_some_and(|e| e == "wasm"))
}

#[test]
fn an_importer_runs_when_a_file_is_imported_and_its_outputs_follow_the_source() {
    let Some(wasm) = palette_wasm() else { return };
    let root = data_root().join("importer");
    let pkg = root.join("pkg");
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins/examples/palette/package");
    blockloom_plugin_host::package::copy_dir(&source, &pkg).unwrap();
    std::fs::create_dir_all(pkg.join("portable")).unwrap();
    std::fs::copy(&wasm, pkg.join("portable/palette.wasm")).unwrap();

    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd: &str, args: Value| backend.dispatch(cmd, args).unwrap();
    invoke("plugin_seal", json!({"path": pkg.to_string_lossy()}));
    invoke(
        "create_project",
        json!({"name": "Importer", "mode": "TwoD", "location": root.join("projects")}),
    );
    let dir = PathBuf::from(
        invoke("get_state", json!({}))["project_path"]
            .as_str()
            .unwrap(),
    );
    // With no importer installed a palette is just a file.
    let gpl = root.join("sunset.gpl");
    std::fs::write(
        &gpl,
        "GIMP Palette\nName: Sunset\n255 0 0 Red\n0 0 255 Blue\n",
    )
    .unwrap();
    invoke(
        "import_assets",
        json!({"parent": "assets", "paths": [gpl.to_string_lossy()]}),
    );
    assert!(!dir.join("assets/sunset.gpl.imported").exists());

    invoke(
        "plugin_install",
        json!({"id": "com.example.palette", "source": format!("path:{}", pkg.display())}),
    );
    let listed = invoke("plugin_importers", json!({}));
    assert_eq!(listed["importers"][0]["extensions"], json!(["gpl"]));
    assert_eq!(listed["buildHooks"][0]["name"], "cook");

    // Importing a file that an installed importer takes imports it as well.
    std::fs::remove_file(dir.join("assets/sunset.gpl")).unwrap();
    invoke(
        "import_assets",
        json!({"parent": "assets", "paths": [gpl.to_string_lossy()]}),
    );
    assert!(dir.join("assets/sunset.gpl.imported/palette.png").is_file());
    let imports = invoke("plugin_imports", json!({}));
    assert_eq!(imports["imports"][0]["source"], "assets/sunset.gpl");
    assert_eq!(imports["imports"][0]["state"], "fresh");

    // The shell-side command re-runs one by name, and refuses what no
    // importer takes.
    let again = invoke(
        "plugin_import",
        json!({"path": "assets/sunset.gpl", "importer": "com.example.palette/gpl"}),
    );
    assert_eq!(again["outputs"].as_array().unwrap().len(), 2);
    std::fs::write(dir.join("assets/a.txt"), "x").unwrap();
    assert!(
        backend
            .dispatch("plugin_import", json!({"path": "assets/a.txt"}))
            .is_err()
    );

    // Deleting the source takes what it made with it.
    invoke("delete_asset", json!({"path": "assets/sunset.gpl"}));
    assert!(!dir.join("assets/sunset.gpl.imported").exists());
    assert!(
        invoke("plugin_imports", json!({}))["imports"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
