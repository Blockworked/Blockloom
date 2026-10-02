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

    // The schemas an inspector builds its forms from come with the listing.
    let types = list["types"].as_array().unwrap();
    let health = types
        .iter()
        .find(|t| t["name"] == "com.example.health/Health")
        .unwrap();
    assert_eq!(health["kind"], "component");
    assert_eq!(health["fields"][0]["name"], "hp");
    assert_eq!(health["fields"][0]["type"], "int");
    assert_eq!(health["defaults"]["hp"], 100);
    assert!(
        types
            .iter()
            .any(|t| t["name"] == "com.example.health/Difficulty" && t["kind"] == "resource")
    );

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

    // A source edited outside the editor is imported again on request, and
    // an output edited by hand is only replaced when asked for by name.
    std::fs::write(
        dir.join("assets/sunset.gpl"),
        "GIMP Palette\nName: Sunset\n255 0 0 Red\n0 255 0 Green\n0 0 255 Blue\n",
    )
    .unwrap();
    assert_eq!(
        invoke("plugin_imports", json!({}))["imports"][0]["state"],
        "source_changed"
    );
    let done = invoke("plugin_reimport", json!({}));
    assert_eq!(done["reimported"], json!(["assets/sunset.gpl"]));
    assert_eq!(
        invoke("plugin_imports", json!({}))["imports"][0]["state"],
        "fresh"
    );
    let json_out = dir.join("assets/sunset.gpl.imported/palette.json");
    std::fs::write(&json_out, "{}").unwrap();
    let skipped = invoke("plugin_reimport", json!({}));
    assert!(skipped["reimported"].as_array().unwrap().is_empty());
    assert_eq!(skipped["skipped"][0]["path"], "assets/sunset.gpl");
    assert_eq!(std::fs::read_to_string(&json_out).unwrap(), "{}");
    let forced = invoke("plugin_reimport", json!({"path": "assets/sunset.gpl"}));
    assert_eq!(forced["reimported"], json!(["assets/sunset.gpl"]));
    assert_ne!(std::fs::read_to_string(&json_out).unwrap(), "{}");
    assert!(
        backend
            .dispatch("plugin_reimport", json!({"path": "assets/other.gpl"}))
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

#[test]
fn an_attached_copy_follows_the_owners_package_changes() {
    let root = data_root().join("attached");
    let owner = Backend::start(AppHandle::new(|_| {}));
    let own = |cmd: &str, args: Value| owner.dispatch(cmd, args).unwrap();
    own(
        "create_project",
        json!({"name": "Shared", "mode": "TwoD", "location": root.join("projects")}),
    );
    let dir = own("get_state", json!({}))["project_path"]
        .as_str()
        .unwrap()
        .to_string();

    let follower = Backend::start(AppHandle::new(|_| {}));
    let report = follower
        .dispatch("open_project", json!({"path": dir}))
        .unwrap();
    assert_eq!(report["attached"], true);
    let installed = |b: &Backend| {
        b.dispatch("plugin_list", json!({})).unwrap()["installed"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(installed(&follower), 0);

    // An install touches only plugins.json and the lock, yet the follower
    // hears about it and loads the package.
    own(
        "plugin_install",
        json!({"id": "com.example.health", "source": format!("path:{}", example())}),
    );
    assert_eq!(installed(&follower), 1);
    // The owner doesn't re-read its own change.
    assert_eq!(installed(&owner), 1);

    own("plugin_remove", json!({"id": "com.example.health"}));
    assert_eq!(installed(&follower), 0);
    // And a follower may not change packages.
    assert!(
        follower
            .dispatch(
                "plugin_install",
                json!({"id": "com.example.health", "source": format!("path:{}", example())}),
            )
            .is_err()
    );
}

#[test]
fn a_command_can_set_or_append_to_a_resource_field() {
    let root = data_root().join("resfield");
    let pkg = root.join("pkg");
    std::fs::create_dir_all(pkg.join("schemas")).unwrap();
    std::fs::write(
        pkg.join("schemas/log.json"),
        json!({
            "resources": [{"type_id": "journal", "fields": [
                {"name": "title", "type": "text", "default": "none"},
                {"name": "lines", "type": "list", "item": {"type": "text"}, "max_len": 2, "default": []},
                {"name": "trail", "type": "list", "item": {"type": "text"}, "max_len": 8, "default": []}
            ]}],
            "tools": [{"name": "walk", "title": "Walk", "cast": "cast", "command": "trail_at",
                "drag": true, "outline": "box", "args": {"x": "$hit.cell.0"}},
                {"name": "stamp", "title": "Stamp", "cast": "cast", "command": "title_at",
                "args": {"who": "$option.who", "x": "$hit.cell.0"},
                "options": [{"name": "who", "type": "text", "default": "me"}]}],
            "panels": [{"name": "main", "title": "Journal", "items": [
                {"kind": "resource", "resource": "journal"},
                {"kind": "command", "command": "add_line", "label": "Add"}
            ]}],
            "commands": [
                {"name": "add_line", "summary": "Append a line.",
                 "args": [{"name": "value", "type": "text", "default": ""}],
                 "action": {"do": "set_resource_field", "resource": "journal", "field": "lines", "append": true}},
                {"name": "set_title", "summary": "Set the title.",
                 "args": [{"name": "value", "type": "text", "default": ""}],
                 "action": {"do": "set_resource_field", "resource": "journal", "field": "title"}},
                {"name": "trail_at", "summary": "Append a trail step.",
                 "args": [{"name": "x", "type": "int", "default": 0}],
                 "action": {"do": "set_resource_field", "resource": "journal", "field": "trail", "append": true, "template": "step {x}"}},
                {"name": "title_at", "summary": "Set the title from a template.",
                 "args": [{"name": "who", "type": "text", "default": "me"}, {"name": "x", "type": "int", "default": 0}],
                 "action": {"do": "set_resource_field", "resource": "journal", "field": "title", "template": "{who} at {x}"}}
            ]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pkg.join("plugin.json"),
        json!({
            "format": 1, "id": "com.example.journal", "name": "Journal", "version": "1.0.0",
            "engine": ">=0.0.1", "tier": "declarative", "contributions": ["schemas/log.json"]
        })
        .to_string(),
    )
    .unwrap();
    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd: &str, args: Value| backend.dispatch(cmd, args).unwrap();
    invoke("plugin_seal", json!({"path": pkg.to_string_lossy()}));
    invoke(
        "create_project",
        json!({"name": "Journal", "mode": "TwoD", "location": root.join("projects")}),
    );
    invoke(
        "plugin_install",
        json!({"id": "com.example.journal", "source": format!("path:{}", pkg.display())}),
    );
    let shown = invoke("get_state", json!({}))["plugins"]["panels"].clone();
    assert_eq!(shown[0]["plugin"], "com.example.journal");
    assert_eq!(shown[0]["panel"]["title"], "Journal");
    assert_eq!(shown[0]["commands"]["add_line"]["args"][0]["name"], "value");
    let add = |text: &str| {
        backend.dispatch(
            "plugin_call",
            json!({"command": "com.example.journal/add_line", "args": {"value": text}}),
        )
    };
    add("one").unwrap();
    add("two").unwrap();
    let resource = || {
        let state = backend.dispatch("get_state", json!({})).unwrap();
        state["project"]["plugin_resources"]
            .as_array()
            .and_then(|all| all.iter().find(|r| r["type_id"] == "journal").cloned())
            .unwrap()
    };
    assert_eq!(resource()["payload"]["lines"], json!(["one", "two"]));
    // The list's own bound still holds, and a refused append changes nothing.
    assert!(add("three").is_err());
    assert_eq!(resource()["payload"]["lines"], json!(["one", "two"]));
    // Setting one field keeps the others.
    invoke(
        "plugin_call",
        json!({"command": "com.example.journal/set_title", "args": {"value": "Day 1"}}),
    );
    let payload = resource()["payload"].clone();
    assert_eq!(payload["title"], "Day 1");
    assert_eq!(payload["lines"], json!(["one", "two"]));
    // One undo takes back one command.
    invoke("undo", json!({}));
    assert_eq!(resource()["payload"]["title"], "none");
    // A template builds the value from the arguments.
    invoke(
        "plugin_call",
        json!({"command": "com.example.journal/title_at", "args": {"who": "cat", "x": 7}}),
    );
    assert_eq!(resource()["payload"]["title"], "cat at 7");
    // A scene tool's click resolves its command's arguments from the hit.
    let tools = invoke("get_state", json!({}))["plugins"]["tools"].clone();
    let stamp = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["tool"]["name"] == "stamp")
        .expect("the stamp tool is listed");
    assert_eq!(stamp["tool"]["name"], "stamp");
    invoke(
        "plugin_run_tool",
        json!({"plugin": "com.example.journal", "tool": "stamp",
               "hit": {"hit": true, "cell": [3, 0, 0]}, "options": {"who": "dog"}}),
    );
    assert_eq!(resource()["payload"]["title"], "dog at 3");
    assert!(
        backend
            .dispatch(
                "plugin_run_tool",
                json!({"plugin": "com.example.journal", "tool": "stamp", "hit": {"hit": true}}),
            )
            .is_err()
    );

    // A stroke is every hit at once: one write, one undo step, repeats dropped.
    let walk = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["tool"]["name"] == "walk")
        .expect("the walk tool is listed");
    assert_eq!(walk["tool"]["drag"], true);
    assert_eq!(walk["tool"]["outline"], "box");
    let hit = |x: i32| json!({"hit": true, "cell": [x, 0, 0]});
    let stroke = invoke(
        "plugin_run_tool",
        json!({"plugin": "com.example.journal", "tool": "walk",
               "hits": [hit(1), hit(2), hit(2), hit(3)]}),
    );
    assert_eq!(stroke["applied"], 3);
    assert_eq!(
        resource()["payload"]["trail"],
        json!(["step 1", "step 2", "step 3"])
    );
    invoke("undo", json!({}));
    assert_eq!(resource()["payload"]["trail"], json!([]));
    assert_eq!(resource()["payload"]["title"], "dog at 3");
    // Too long for the list's bound: refused whole.
    let long: Vec<Value> = (0..9).map(hit).collect();
    assert!(
        backend
            .dispatch(
                "plugin_run_tool",
                json!({"plugin": "com.example.journal", "tool": "walk", "hits": long}),
            )
            .is_err()
    );
    assert_eq!(resource()["payload"]["trail"], json!([]));
}

#[test]
fn editor_modules_load_only_after_the_user_trusts_that_exact_package() {
    let root = data_root().join("trusted");
    let pkg = root.join("pkg");
    std::fs::create_dir_all(pkg.join("editor")).unwrap();
    std::fs::write(
        pkg.join("editor/StampPanel.qml"),
        "import QtQuick\nItem { property var host }\n",
    )
    .unwrap();
    let manifest = |version: &str| {
        std::fs::write(
            pkg.join("plugin.json"),
            json!({
                "format": 1, "id": "com.example.stamp", "name": "Stamp", "version": version,
                "engine": ">=0.0.1", "tier": "declarative", "contributions": [],
                "capabilities": ["trusted-editor"],
                "editor": {"modules": ["editor/StampPanel.qml"]}
            })
            .to_string(),
        )
        .unwrap();
    };
    manifest("1.0.0");
    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd: &str, args: Value| backend.dispatch(cmd, args).unwrap();
    invoke("plugin_seal", json!({"path": pkg.to_string_lossy()}));
    invoke(
        "create_project",
        json!({"name": "Trusted", "mode": "TwoD", "location": root.join("projects")}),
    );
    invoke(
        "plugin_install",
        json!({"id": "com.example.stamp", "source": format!("path:{}", pkg.display())}),
    );
    let modules = || invoke("get_state", json!({}))["plugins"]["editorModules"].clone();

    // Installed but not trusted: the editor is told what it ships, not where it is.
    let listed = modules();
    assert_eq!(listed[0]["plugin"], "com.example.stamp");
    assert_eq!(listed[0]["trusted"], false);
    assert_eq!(listed[0]["modules"][0]["title"], "Stamp panel");
    assert!(listed[0]["modules"][0]["file"].is_null());

    invoke("plugin_trust", json!({"id": "com.example.stamp"}));
    let trusted = modules();
    assert_eq!(trusted[0]["trusted"], true);
    let file = trusted[0]["modules"][0]["file"].as_str().unwrap();
    assert!(file.ends_with("editor/StampPanel.qml") && std::path::Path::new(file).exists());

    // Taking it back unloads it.
    invoke("plugin_untrust", json!({"id": "com.example.stamp"}));
    assert_eq!(modules()[0]["trusted"], false);

    // Trust names the package's content: a new version asks again.
    invoke("plugin_trust", json!({"id": "com.example.stamp"}));
    manifest("1.1.0");
    invoke("plugin_seal", json!({"path": pkg.to_string_lossy()}));
    invoke(
        "plugin_update",
        json!({"id": "com.example.stamp", "source": format!("path:{}", pkg.display())}),
    );
    let after = modules();
    assert_eq!(after[0]["trusted"], false, "{after}");
    assert_eq!(after[0]["changed"], true);

    // A plugin with no editor modules has nothing to trust.
    assert!(
        backend
            .dispatch("plugin_trust", json!({"id": "com.example.nothing"}))
            .is_err()
    );
}

#[test]
fn the_notes_example_ships_a_screen_and_saves_through_its_command() {
    let root = data_root().join("notes");
    let notes =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins/examples/com.example.notes");
    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd: &str, args: Value| backend.dispatch(cmd, args).unwrap();
    invoke(
        "create_project",
        json!({"name": "Notes", "mode": "TwoD", "location": root.join("projects")}),
    );
    invoke(
        "plugin_install",
        json!({"id": "com.example.notes", "source": format!("path:{}", notes.display())}),
    );
    invoke("plugin_trust", json!({"id": "com.example.notes"}));
    let modules = invoke("get_state", json!({}))["plugins"]["editorModules"].clone();
    let file = modules[0]["modules"][0]["file"].as_str().unwrap();
    assert!(
        std::fs::read_to_string(file)
            .unwrap()
            .contains("host.call(\"set_notes\"")
    );
    // The sticky note's inspector section is listed too, with its file once trusted.
    let section = &modules[0]["inspectors"][0];
    assert_eq!(section["component"], "sticky");
    let section_file = section["file"].as_str().unwrap();
    assert!(
        std::fs::read_to_string(section_file)
            .unwrap()
            .contains("host.write(next)")
    );
    let types = invoke("get_state", json!({}))["plugins"]["types"].clone();
    assert!(
        types
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "com.example.notes/sticky"),
        "{types}"
    );
    // The screen's Save button is this command.
    invoke(
        "plugin_call",
        json!({"command": "com.example.notes/set_notes", "args": {"value": "first draft"}}),
    );
    let state = invoke("get_state", json!({}));
    let saved = state["project"]["plugin_resources"]
        .as_array()
        .and_then(|all| all.iter().find(|r| r["type_id"] == "notes"))
        .unwrap();
    assert_eq!(saved["payload"]["text"], "first draft");
}
