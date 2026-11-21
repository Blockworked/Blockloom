//! Material assets end to end: actors following one file take its changes
//! except where they override them, through undo and a reopen.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};

fn material(state: &Value, id: &str) -> Value {
    state["project"]["actors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id)
        .unwrap()["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == "Material")
        .unwrap()
        .clone()
}

#[test]
fn actors_follow_their_material_file_but_keep_overrides() {
    let data = std::env::temp_dir().join(format!("blockloom-material-{}", uuid::Uuid::new_v4()));
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
        json!({"name": "Mats", "location": projects, "mode": "ThreeD"}),
    );
    let dir = projects.join("Mats");
    let a = run("add_actor", json!({"shape": "Cuboid", "name": "Wall"}));
    let b = run("add_actor", json!({"shape": "Cuboid", "name": "Floor"}));
    let (a, b) = (
        a.as_str().unwrap().to_string(),
        b.as_str().unwrap().to_string(),
    );
    let path = run("create_material_asset", json!({"actorId": a}));
    let path = path.as_str().unwrap().to_string();
    assert_eq!(path, "assets/materials/Wall.blockmaterial");
    run("assign_material", json!({"actorId": b, "path": path}));

    let set = |id: &str, at: &str, value: Value| {
        run(
            "set_property",
            json!({"target": format!("actor:{id}/Material"), "path": format!("material.{at}"), "value": value}),
        );
    };
    set(&b, "roughness", json!(0.25));
    let state = run("get_state", json!({}));
    assert_eq!(
        material(&state, &b)["source"]["overrides"],
        json!(["roughness"])
    );

    set(&a, "metallic", json!(1.0));
    set(&a, "roughness", json!(0.75));
    assert_eq!(run("apply_material", json!({"actorId": a})), json!(1));
    let state = run("get_state", json!({}));
    assert_eq!(
        material(&state, &b)["material"]["metallic"],
        1.0,
        "b followed"
    );
    assert_eq!(
        material(&state, &b)["material"]["roughness"],
        0.25,
        "b's override stays"
    );

    run("revert_material", json!({"actorId": b}));
    let state = run("get_state", json!({}));
    assert_eq!(material(&state, &b)["material"]["roughness"], 0.75);
    run("undo", json!({}));
    let state = run("get_state", json!({}));
    assert_eq!(material(&state, &b)["material"]["roughness"], 0.25);

    // An edit to the file outside the editor reaches both on open.
    run("save_project", json!({}));
    run("close_project", json!({}));
    let file = dir.join(&path);
    let text = std::fs::read_to_string(&file).unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    doc["material"]["metallic"] = json!(0.5);
    std::fs::write(&file, doc.to_string()).unwrap();
    run("open_project", json!({"path": dir}));
    let state = run("get_state", json!({}));
    assert_eq!(material(&state, &a)["material"]["metallic"], 0.5);
    assert_eq!(material(&state, &b)["material"]["metallic"], 0.5);
    assert_eq!(material(&state, &b)["material"]["roughness"], 0.25);
    let used = run("asset_used_by", json!({"path": path}));
    assert_eq!(used.as_array().unwrap().len(), 2, "{used}");

    run("unlink_material", json!({"actorId": b}));
    let state = run("get_state", json!({}));
    assert!(material(&state, &b).get("source").is_none());
    run("close_project", json!({}));
    let _ = std::fs::remove_dir_all(&data);
}
