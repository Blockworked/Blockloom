use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};
use std::path::PathBuf;

fn example() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../plugins/examples/com.example.health")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_plugin_installs_owns_records_and_leaves_them_when_removed() {
    let root = std::env::temp_dir().join(format!("blockloom-plugins-{}", uuid::Uuid::new_v4()));
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", root.join("data"));
        std::env::set_var("BLOCKLOOM_PLUGINS_OFFLINE", "0");
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    let call = |cmd: &str, args: Value| backend.dispatch(cmd, args);
    let invoke = |cmd: &str, args: Value| call(cmd, args).unwrap();
    invoke(
        "create_project",
        json!({"name": "Plugged", "mode": "TwoD", "location": root.join("projects")}),
    );
    let ball = invoke("add_actor", json!({"shape": "Circle", "name": "Ball"}));
    let ball = ball.as_str().unwrap_or_default().to_owned();

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
