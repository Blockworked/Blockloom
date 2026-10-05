use blockloom_app::{AppHandle, Backend};
use serde_json::json;

#[test]
fn settings_persist_are_undoable_and_invalid_edits_leave_no_history() {
    let root = std::env::temp_dir().join(format!(
        "blockloom-multiplayer-app-{}",
        uuid::Uuid::new_v4()
    ));
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", root.join("data"));
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    backend
        .dispatch(
            "create_project",
            json!({"name":"LAN", "mode":"TwoD", "location":root.join("projects")}),
        )
        .unwrap();
    let get =
        || backend.dispatch("get_state", json!({})).unwrap()["project"]["multiplayer"].clone();
    assert!(get().is_null());
    let settings = json!({"enabled":true,"max_guests":3});
    backend
        .dispatch("set_multiplayer", json!({"settings":settings}))
        .unwrap();
    assert_eq!(get(), settings);
    assert!(
        backend
            .dispatch(
                "set_multiplayer",
                json!({"settings":{"enabled":true,"max_guests":17}})
            )
            .is_err()
    );
    assert_eq!(get(), settings);
    backend.dispatch("undo", json!({})).unwrap();
    assert!(get().is_null());
    backend.dispatch("redo", json!({})).unwrap();
    assert_eq!(get(), settings);
    let dir = root.join("projects/LAN");
    assert_eq!(
        blockloom_core::project::read_project_dir(&dir)
            .unwrap()
            .multiplayer
            .max_guests,
        3
    );
    drop(backend);
    std::fs::remove_dir_all(root).unwrap();
}
