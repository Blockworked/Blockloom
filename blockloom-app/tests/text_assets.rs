use blockloom_app::{AppHandle, Backend};
use serde_json::json;

#[test]
fn text_assets_write_whole_files_under_assets_only() {
    let root = std::env::temp_dir().join(format!("blockloom-text-{}", uuid::Uuid::new_v4()));
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", root.join("data"));
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd, args| backend.dispatch(cmd, args);
    invoke(
        "create_project",
        json!({"name": "Text", "mode": "ThreeD", "location": root.join("projects")}),
    )
    .unwrap();
    let dir = std::path::PathBuf::from(
        invoke("get_state", json!({})).unwrap()["project_path"]
            .as_str()
            .unwrap(),
    );
    let path = "assets/blocks/stone.blockdef";
    let made = invoke(
        "write_text_asset",
        json!({"path": path, "text": "{\"name\":\"stone\"}"}),
    );
    assert_eq!(made.unwrap(), json!(path));
    assert_eq!(
        invoke("read_text_asset", json!({"path": path})).unwrap(),
        json!("{\"name\":\"stone\"}")
    );
    // The new file is indexed like any other asset.
    let index = invoke("resource_index", json!({})).unwrap().to_string();
    assert!(index.contains(path), "{index}");
    for bad in [
        "project.blockloom",
        "assets/../project.blockloom",
        "assets/x.blockscene",
    ] {
        assert!(
            invoke("write_text_asset", json!({"path": bad, "text": "{}"})).is_err(),
            "{bad}"
        );
    }
    assert!(invoke("read_text_asset", json!({"path": "assets/missing.txt"})).is_err());
    assert!(dir.join("project.blockloom").is_file());
    let _ = std::fs::remove_dir_all(root);
}
