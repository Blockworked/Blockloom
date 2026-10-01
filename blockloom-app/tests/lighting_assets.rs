use blockloom_app::{AppHandle, Backend};
use serde_json::json;

#[test]
fn lighting_assets_are_shared_saved_and_keep_a_missing_file_fallback() {
    let root = std::env::temp_dir().join(format!("blockloom-lighting-{}", uuid::Uuid::new_v4()));
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", root.join("data"));
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd, args| backend.dispatch(cmd, args).unwrap();
    invoke(
        "create_project",
        json!({"name": "Lighting", "mode": "ThreeD", "location": root.join("projects")}),
    );
    let state = invoke("get_state", json!({}));
    let dir = std::path::PathBuf::from(state["project_path"].as_str().unwrap());
    let first = state["project"]["active_scene"]
        .as_str()
        .unwrap()
        .to_owned();
    invoke("set_lighting", json!({"lighting": {"illuminance": 321}}));
    let path = invoke(
        "create_asset",
        json!({"parent": "assets", "name": "Sun.blocklighting"}),
    );
    let path = path.as_str().unwrap();
    assert_eq!(
        invoke("read_lighting_asset", json!({"path": path}))["illuminance"].as_f64(),
        Some(321.0)
    );
    invoke("set_scene_lighting_asset", json!({"path": path}));
    invoke(
        "create_asset",
        json!({"parent": "assets", "name": "Second.blockscene"}),
    );
    invoke("set_scene_lighting_asset", json!({"path": path}));
    invoke(
        "write_lighting_asset",
        json!({"path": path, "lighting": {"illuminance": 456, "shadows": {"contact": true}}}),
    );
    let state = invoke("get_state", json!({}));
    for scene in state["project"]["scenes"].as_array().unwrap() {
        assert_eq!(
            scene["world"]["lighting"]["illuminance"].as_f64(),
            Some(456.0)
        );
        assert_eq!(scene["world"]["lighting"]["asset"], path);
    }
    assert!(
        backend
            .dispatch(
                "write_lighting_asset",
                json!({"path": path, "lighting": {"light_color": "invalid"}})
            )
            .is_err()
    );
    assert!(
        backend
            .dispatch(
                "set_scene_lighting_asset",
                json!({"path": "../Sun.blocklighting"})
            )
            .is_err()
    );
    invoke(
        "rename_asset",
        json!({"path": path, "name": "Sunset.blocklighting"}),
    );
    let renamed = "assets/Sunset.blocklighting";
    let loaded = blockloom_core::project::read_project_dir(&dir).unwrap();
    assert!(
        loaded
            .scenes
            .iter()
            .all(|s| s.world.lighting.asset == renamed)
    );
    assert!(
        loaded
            .scenes
            .iter()
            .all(|s| s.world.lighting.illuminance == 456.0)
    );
    invoke("set_active_scene", json!({"sceneId": first}));
    invoke("set_scene_lighting_asset", json!({"path": ""}));
    assert_eq!(
        invoke("get_state", json!({}))["project"]["world"]["lighting"]["illuminance"].as_f64(),
        Some(456.0)
    );
    invoke("undo", json!({}));
    assert_eq!(
        invoke("get_state", json!({}))["project"]["world"]["lighting"]["asset"],
        renamed
    );
    invoke("close_project", json!({}));
    std::fs::remove_file(dir.join(renamed)).unwrap();
    let loaded = blockloom_core::project::read_project_dir(&dir).unwrap();
    assert!(
        loaded
            .scenes
            .iter()
            .all(|s| s.world.lighting.illuminance == 456.0)
    );
    std::fs::remove_dir_all(root).unwrap();
}
