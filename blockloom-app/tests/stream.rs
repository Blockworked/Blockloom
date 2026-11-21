//! The window's change stream: replaying every message the backend sent,
//! the way `Main.qml`'s `applyMessage` does, lands on the same state a whole
//! snapshot would give, edit after edit.

use blockloom_app::{AppHandle, Backend, Event};
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

/// `Main.qml`'s `applyMessage`, without the object reuse.
fn apply(state: &mut Value, message: &Value) {
    if let Some(full) = message.get("full") {
        *state = full.clone();
        return;
    }
    let patch = &message["patch"];
    if let Some(set) = patch["set"].as_object() {
        for (key, value) in set {
            state[key] = value.clone();
        }
    }
    let Some(project) = patch.get("project") else {
        return;
    };
    if let Some(set) = project["set"].as_object() {
        for (key, value) in set {
            state["project"][key] = value.clone();
        }
    }
    let Some(actors) = project.get("actors") else {
        return;
    };
    let mut by_id: Map<String, Value> = state["project"]["actors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["id"].as_str().unwrap().to_string(), a.clone()))
        .collect();
    if let Some(set) = actors["set"].as_object() {
        for (id, actor) in set {
            by_id.insert(id.clone(), actor.clone());
        }
    }
    let removed: Vec<&str> = actors["remove"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let order: Vec<String> = match actors["order"].as_array() {
        Some(order) => order
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect(),
        None => state["project"]["actors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect(),
    };
    state["project"]["actors"] = Value::Array(
        order
            .iter()
            .filter(|id| !removed.contains(&id.as_str()))
            .filter_map(|id| by_id.get(id).cloned())
            .collect(),
    );
}

/// Paths where `got` and `want` differ. Numbers compare loosely: text
/// parsing doesn't round-trip a float's last bit.
fn differences(path: &str, got: &Value, want: &Value, out: &mut Vec<String>) {
    match (got, want) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let null = Value::Null;
                let at = format!("{path}/{key}");
                differences(
                    &at,
                    a.get(key).unwrap_or(&null),
                    b.get(key).unwrap_or(&null),
                    out,
                );
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}/{i}"), x, y, out);
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            if (a - b).abs() > 1e-9 * a.abs().max(1.0) {
                out.push(format!("{path}: {a} vs {b}"));
            }
        }
        (a, b) if a != b => {
            let short = |v: &Value| v.to_string().chars().take(200).collect::<String>();
            out.push(format!("{path}: {} vs {}", short(a), short(b)));
        }
        _ => {}
    }
}

/// The whole snapshot, shaped as the window holds it: scenes listed
/// without their actors.
fn expected(backend: &Backend) -> Value {
    let mut state = backend.dispatch("get_state", json!({})).unwrap();
    if let Some(scenes) = state["project"]["scenes"].as_array_mut() {
        for scene in scenes {
            scene.as_object_mut().unwrap().remove("actors");
        }
    }
    state
}

#[test]
fn replayed_patches_match_the_whole_snapshot() {
    let data = std::env::temp_dir().join(format!("blockloom-stream-{}", uuid::Uuid::new_v4()));
    let projects = data.join("projects");
    std::fs::create_dir_all(&projects).unwrap();
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", &data);
    }
    let messages: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = messages.clone();
    let backend = Backend::start(AppHandle::new(move |event| {
        if let Event::State(json) = event {
            sink.lock().unwrap().push(json.to_string());
        }
    }));
    let mut state: Value = serde_json::from_str(&backend.frontend_state().unwrap()).unwrap();
    let mut state = state["full"].take();
    let mut check = |backend: &Backend, what: &str, state: &mut Value| {
        for message in messages.lock().unwrap().drain(..) {
            apply(state, &serde_json::from_str(&message).unwrap());
        }
        let want = expected(backend);
        let mut diffs = Vec::new();
        differences("", state, &want, &mut diffs);
        assert!(diffs.is_empty(), "after {what}: {diffs:#?}");
    };

    let steps: Vec<(&str, Value)> = vec![
        (
            "create_project",
            json!({"name": "Stream", "location": projects}),
        ),
        ("add_actor", json!({"shape": "Circle", "name": "Ball"})),
        ("add_actor", json!({"shape": "Rect", "name": "Wall"})),
        ("undo", json!({})),
        ("redo", json!({})),
        ("add_scene", json!({"name": "Two"})),
        ("add_actor", json!({"shape": "Circle", "name": "Other"})),
    ];
    for (command, args) in steps {
        backend.dispatch(command, args).unwrap();
        check(&backend, command, &mut state);
    }
    // Edits that name actors, and a switch back to the first scene.
    let first = state["project"]["scenes"][0]["id"].clone();
    backend
        .dispatch("set_active_scene", json!({"sceneId": first}))
        .unwrap();
    check(&backend, "set_active_scene", &mut state);
    let ball = state["project"]["actors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "Ball")
        .unwrap()["id"]
        .clone();
    for (command, args) in [
        ("rename_actor", json!({"actorId": ball, "name": "Renamed"})),
        ("duplicate_actor", json!({"actorId": ball})),
        ("select_actor", json!({"actorId": ball})),
        ("remove_actor", json!({"actorId": ball})),
        ("undo", json!({})),
        ("set_project_name", json!({"name": "Streamed"})),
        ("undo", json!({})),
    ] {
        backend.dispatch(command, args).unwrap();
        check(&backend, command, &mut state);
    }
    // A rename sends the one actor, not the project.
    backend
        .dispatch("rename_actor", json!({"actorId": ball, "name": "Again"}))
        .unwrap();
    let sent = messages.lock().unwrap().clone();
    let patch: Value = serde_json::from_str(sent.last().unwrap()).unwrap();
    let actors = &patch["patch"]["project"]["actors"]["set"];
    assert_eq!(actors.as_object().unwrap().len(), 1, "{patch}");
    assert!(patch["patch"]["project"].get("set").is_none(), "{patch}");
    check(&backend, "rename again", &mut state);
    let _ = std::fs::remove_dir_all(&data);
}
