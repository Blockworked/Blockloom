//! Environment assets end to end: scenes following one `.blockenv` take its
//! changes except where they override them, through undo and a reopen.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};

fn world(backend: &Backend) -> Value {
    backend.dispatch("get_state", json!({})).unwrap()["project"]["world"].clone()
}

#[test]
fn scenes_follow_their_environment_but_keep_overrides() {
    let data = std::env::temp_dir().join(format!("blockloom-env-{}", uuid::Uuid::new_v4()));
    let projects = data.join("projects");
    std::fs::create_dir_all(&projects).unwrap();
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", &data);
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    let run = |command: &str, args: Value| {
        backend
            .dispatch(command, args.clone())
            .unwrap_or_else(|e| panic!("{command} {args}: {e}"))
    };
    run(
        "create_project",
        json!({"name": "Envs", "location": projects}),
    );
    let dir = projects.join("Envs");
    let first = run("get_state", json!({}))["project"]["active_scene"].clone();
    run("set_background", json!({"color": "#102030"}));
    let path = run(
        "create_environment_asset",
        json!({"path": "assets/look.blockenv"}),
    );
    assert_eq!(path, "assets/look.blockenv");
    let second = run("add_scene", json!({"name": "Cave"}));
    run("set_active_scene", json!({"sceneId": second}));
    run(
        "assign_environment",
        json!({"path": "assets/look.blockenv"}),
    );
    assert_eq!(world(&backend)["background"], "#102030");
    run(
        "set_property",
        json!({"target": "world", "path": "wind.speed", "value": 7.5}),
    );
    let w = world(&backend);
    assert_eq!(
        w["environment"]["overrides"],
        json!(["wind.speed"]),
        "{}",
        w["environment"]
    );

    // The first scene changes its background and pushes it to the file.
    run("set_active_scene", json!({"sceneId": first}));
    run("set_background", json!({"color": "#AA0000"}));
    assert_eq!(run("apply_environment", json!({})), json!(1));
    run("set_active_scene", json!({"sceneId": second}));
    let w = world(&backend);
    assert_eq!(w["background"], "#AA0000", "the cave followed");
    assert_eq!(w["wind"]["speed"], 7.5, "its override stays");

    run("revert_environment", json!({"path": "wind.speed"}));
    assert_ne!(world(&backend)["wind"]["speed"], 7.5);
    run("undo", json!({}));
    assert_eq!(world(&backend)["wind"]["speed"], 7.5);

    // An edit to the file outside the editor reaches both scenes on open.
    run("save_project", json!({}));
    run("close_project", json!({}));
    let file = dir.join("assets/look.blockenv");
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("#AA0000", "#00AA00");
    std::fs::write(&file, text).unwrap();
    run("open_project", json!({"path": dir}));
    run("set_active_scene", json!({"sceneId": second}));
    let w = world(&backend);
    assert_eq!(w["background"], "#00AA00");
    assert_eq!(w["wind"]["speed"], 7.5);
    let used = run("asset_used_by", json!({"path": "assets/look.blockenv"}));
    assert_eq!(used.as_array().unwrap().len(), 2, "{used}");

    run("unlink_environment", json!({}));
    assert!(world(&backend).get("environment").is_none());
    run("close_project", json!({}));
    let _ = std::fs::remove_dir_all(&data);
}
