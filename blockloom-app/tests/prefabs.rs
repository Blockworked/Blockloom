//! Prefabs end to end: an edit to one instance applied to the prefab
//! reaches the others, except where they override it, and survives undo,
//! a reopen and an edit made to the file outside the editor.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};

fn actor<'a>(state: &'a Value, id: &str) -> &'a Value {
    state["project"]["actors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id)
        .unwrap()
}

fn color(state: &Value, id: &str) -> String {
    actor(state, id)["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == "Look")
        .unwrap()["visual"]["color"]
        .as_str()
        .unwrap()
        .to_string()
}

fn overrides(state: &Value, id: &str) -> Vec<String> {
    actor(state, id)["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == "Prefab")
        .unwrap()["prefab"]["overrides"]
        .as_array()
        .map(|o| o.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

#[test]
fn instances_follow_their_prefab_but_keep_overrides() {
    let data = std::env::temp_dir().join(format!("blockloom-prefab-{}", uuid::Uuid::new_v4()));
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
        json!({"name": "Prefabs", "location": projects}),
    );
    let dir = projects.join("Prefabs");
    let crate_id = run("add_actor", json!({"shape": "Rect", "name": "Crate"}));
    let crate_id = crate_id.as_str().unwrap().to_string();
    let path = run("create_prefab", json!({"actorId": crate_id}));
    let path = path.as_str().unwrap().to_string();
    assert_eq!(path, "assets/prefabs/Crate.blockprefab");
    let a = run(
        "instantiate_prefab",
        json!({"path": path, "position": [100, 0, 0]}),
    );
    let b = run(
        "instantiate_prefab",
        json!({"path": path, "position": [200, 0, 0]}),
    );
    let (a, b) = (
        a.as_str().unwrap().to_string(),
        b.as_str().unwrap().to_string(),
    );

    // b overrides its color; moving it is not an override.
    let set_color = |id: &str, color: &str| {
        run(
            "set_property",
            json!({"target": format!("actor:{id}/Look"), "path": "visual.color", "value": color}),
        );
    };
    set_color(&b, "#00FF00");
    run(
        "set_property",
        json!({"target": format!("actor:{b}/Place"), "path": "placement.position", "value": [250, 0, 0]}),
    );
    let state = run("get_state", json!({}));
    assert_eq!(overrides(&state, &b), ["Look.visual"]);

    // a changes size and color and pushes them into the prefab.
    set_color(&a, "#FF0000");
    run(
        "set_property",
        json!({"target": format!("actor:{a}/Look"), "path": "visual.size", "value": [80, 80]}),
    );
    run("apply_prefab", json!({"actorId": a}));
    let state = run("get_state", json!({}));
    assert!(overrides(&state, &a).is_empty());
    assert_eq!(
        color(&state, &crate_id),
        "#FF0000",
        "the first instance followed"
    );
    assert_eq!(color(&state, &b), "#00FF00", "b's override stays");
    let b_look = &actor(&state, &b)["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == "Place")
        .unwrap()["placement"]["position"];
    assert_eq!(b_look[0], 250.0);

    // b drops its override and takes the prefab's color.
    run("revert_prefab", json!({"actorId": b}));
    let state = run("get_state", json!({}));
    assert_eq!(color(&state, &b), "#FF0000");
    run("undo", json!({}));
    let state = run("get_state", json!({}));
    assert_eq!(
        color(&state, &b),
        "#00FF00",
        "undo brings the override back"
    );
    assert_eq!(overrides(&state, &b), ["Look.visual"]);

    // An edit to the file outside the editor reaches the instances on open.
    run("save_project", json!({}));
    run("close_project", json!({}));
    let file = dir.join(&path);
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("#FF0000", "#123456");
    std::fs::write(&file, text).unwrap();
    run("open_project", json!({"path": dir}));
    let state = run("get_state", json!({}));
    assert_eq!(color(&state, &a), "#123456");
    assert_eq!(color(&state, &b), "#00FF00");

    let used = run("asset_used_by", json!({"path": path}));
    assert_eq!(used.as_array().unwrap().len(), 3, "{used}");
    run("close_project", json!({}));
    let _ = std::fs::remove_dir_all(&data);
}

fn actors(state: &Value) -> &Vec<Value> {
    state["project"]["actors"].as_array().unwrap()
}

fn link(state: &Value, id: &str) -> Value {
    actor(state, id)["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == "Prefab")
        .map(|c| c["prefab"].clone())
        .unwrap_or(Value::Null)
}

fn parent(state: &Value, id: &str) -> String {
    actor(state, id)["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == "Parent")
        .and_then(|c| c["parent"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// The instance members of `root`, keyed by member id.
fn members(state: &Value, root: &str) -> std::collections::BTreeMap<String, String> {
    actors(state)
        .iter()
        .filter_map(|a| {
            let id = a["id"].as_str()?.to_string();
            let l = link(state, &id);
            if id == root {
                return Some((String::new(), id));
            }
            (l["instance"] == root).then(|| (l["member"].as_str().unwrap().to_string(), id))
        })
        .collect()
}

#[test]
fn a_prefab_carries_its_children_and_nests_another() {
    let data = std::env::temp_dir().join(format!("blockloom-prefab-{}", uuid::Uuid::new_v4()));
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
    let text = |v: Value| v.as_str().unwrap().to_string();
    run(
        "create_project",
        json!({"name": "Nest", "location": projects}),
    );
    let dir = projects.join("Nest");
    let car = text(run("add_actor", json!({"shape": "Rect", "name": "Car"})));
    let wheel = text(run("add_actor", json!({"shape": "Rect", "name": "Wheel"})));
    run("move_actor", json!({"actorId": wheel, "parent": car}));
    let path = text(run("create_prefab", json!({"actorId": car})));
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(&path)).unwrap()).unwrap();
    assert_eq!(file["children"].as_array().unwrap().len(), 1, "{file}");

    // A placed copy brings its wheel, hung off the copy.
    let copy = text(run(
        "instantiate_prefab",
        json!({"path": path, "position": [300, 0, 0]}),
    ));
    let state = run("get_state", json!({}));
    let copy_members = members(&state, &copy);
    assert_eq!(copy_members.len(), 2, "{copy_members:?}");
    let copy_wheel = copy_members[&wheel].clone();
    assert_eq!(parent(&state, &copy_wheel), copy);

    // Recolor the copy's wheel and apply from it: the first car's wheel follows.
    run(
        "set_property",
        json!({"target": format!("actor:{copy_wheel}/Look"), "path": "visual.color", "value": "#ABCDEF"}),
    );
    run("apply_prefab", json!({"actorId": copy_wheel}));
    let state = run("get_state", json!({}));
    assert_eq!(color(&state, &wheel), "#ABCDEF");

    // A child hung off the copy becomes a member everywhere on apply.
    let light = text(run("add_actor", json!({"shape": "Rect", "name": "Light"})));
    run("move_actor", json!({"actorId": light, "parent": copy}));
    run("apply_prefab", json!({"actorId": copy}));
    let state = run("get_state", json!({}));
    let first = members(&state, &car);
    assert_eq!(first.len(), 3, "{first:?}");
    assert_eq!(parent(&state, &first[&light]), car);

    // Deleting a member is the instance's own: a later sync leaves it out.
    run("remove_actor", json!({"actorId": first[&light]}));
    run(
        "set_property",
        json!({"target": format!("actor:{copy}/Look"), "path": "visual.color", "value": "#111111"}),
    );
    run("apply_prefab", json!({"actorId": copy}));
    let state = run("get_state", json!({}));
    assert_eq!(
        members(&state, &car).len(),
        2,
        "the deleted light stays deleted"
    );
    assert_eq!(color(&state, &car), "#111111");
    run("revert_prefab", json!({"actorId": car}));
    let state = run("get_state", json!({}));
    assert_eq!(
        members(&state, &car).len(),
        3,
        "reverting all brings it back"
    );

    // A garage holding a car nests the car prefab by reference.
    let garage = text(run("add_actor", json!({"shape": "Rect", "name": "Garage"})));
    run("move_actor", json!({"actorId": copy, "parent": garage}));
    let garage_path = text(run("create_prefab", json!({"actorId": garage})));
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(&garage_path)).unwrap()).unwrap();
    let kids = file["children"].as_array().unwrap();
    assert_eq!(
        kids.len(),
        1,
        "only the nested car's root is written: {file}"
    );
    assert_eq!(
        kids[0]["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["component"] == "Prefab")
            .unwrap()["prefab"]["source"]["path"],
        path
    );
    let second = text(run(
        "instantiate_prefab",
        json!({"path": garage_path, "position": [0, 500, 0]}),
    ));
    let state = run("get_state", json!({}));
    let inside = members(&state, &second);
    assert_eq!(inside.len(), 4, "garage, car, wheel, light: {inside:?}");

    // Changing the car prefab reaches the car inside the placed garage.
    run(
        "set_property",
        json!({"target": format!("actor:{car}/Look"), "path": "visual.color", "value": "#222222"}),
    );
    run("apply_prefab", json!({"actorId": car}));
    let state = run("get_state", json!({}));
    let nested_car = inside
        .iter()
        .find(|(m, _)| !m.is_empty() && !m.contains('/'))
        .unwrap()
        .1;
    assert_eq!(color(&state, nested_car), "#222222");

    // Reopening keeps every instance whole.
    run("save_project", json!({}));
    run("close_project", json!({}));
    run("open_project", json!({"path": dir}));
    let state = run("get_state", json!({}));
    assert_eq!(members(&state, &second).len(), 4);
    assert_eq!(members(&state, &car).len(), 3);
    run("close_project", json!({}));
    let _ = std::fs::remove_dir_all(&data);
}
