//! The resource database end to end: files get ids in `assets.index`, a
//! rename keeps the id and repoints the document, and "used by" finds the
//! actor naming a file.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};

#[test]
fn a_renamed_asset_keeps_its_id_and_its_users() {
    let data = std::env::temp_dir().join(format!("blockloom-res-{}", uuid::Uuid::new_v4()));
    let projects = data.join("projects");
    std::fs::create_dir_all(&projects).unwrap();
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", &data);
    }
    let source = data.join("hero.png");
    std::fs::write(&source, b"not really a png").unwrap();
    let backend = Backend::start(AppHandle::new(|_| {}));
    let run = |command: &str, args: Value| backend.dispatch(command, args).unwrap();
    run("create_project", json!({"name": "Res", "location": projects}));
    let made = run(
        "import_assets",
        json!({"parent": "assets", "paths": [source.to_string_lossy()]}),
    );
    let path = made[0].as_str().unwrap().to_string();
    let index = run("resource_index", json!({}));
    let id = index["resources"][&path]["id"].as_str().unwrap().to_string();

    let actor = run("add_actor", json!({"shape": "Rect", "name": "Hero"}));
    let actor = actor.as_str().map(str::to_string).unwrap_or_else(|| {
        let state = run("get_state", json!({}));
        state["project"]["actors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "Hero")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    });
    run(
        "set_property",
        json!({
            "target": format!("actor:{actor}/Look"),
            "path": "visual",
            "value": {"shape": "Image", "path": path, "size": [32, 32]},
        }),
    );
    let used = run("asset_used_by", json!({"path": path}));
    assert_eq!(used.as_array().unwrap().len(), 1, "{used}");
    assert_eq!(used[0]["actor"], "Hero");
    let listed = run("list_assets", json!({"path": "assets"}));
    let entry = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == path.as_str())
        .unwrap()
        .clone();
    assert_eq!(entry["id"], id.as_str());
    assert_eq!(entry["uses"], 1);

    let moved = run("rename_asset", json!({"path": path, "name": "player.png"}));
    let moved = moved.as_str().unwrap().to_string();
    let index = run("resource_index", json!({}));
    assert_eq!(index["resources"][&moved]["id"], id.as_str());
    assert!(index["resources"].get(&path).is_none());
    let used = run("asset_used_by", json!({"path": moved}));
    assert_eq!(used.as_array().unwrap().len(), 1, "the actor followed: {used}");

    run("close_project", json!({}));
    let _ = std::fs::remove_dir_all(&data);
}
