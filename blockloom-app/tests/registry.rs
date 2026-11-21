//! The type registry end to end: listing it, `add_component` from its
//! defaults, `set_property` checked against its schemas, and undo.

use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};

fn component<'a>(state: &'a Value, actor: &str, name: &str) -> &'a Value {
    let actors = state["project"]["actors"].as_array().unwrap();
    let actor = actors.iter().find(|a| a["id"] == actor).unwrap();
    actor["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == name)
        .unwrap_or(&Value::Null)
}

#[test]
fn components_and_settings_are_added_and_set_through_the_registry() {
    let root = std::env::temp_dir().join(format!("blockloom-registry-{}", uuid::Uuid::new_v4()));
    unsafe {
        std::env::set_var("BLOCKLOOM_DATA_DIR", root.join("data"));
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    let invoke = |cmd, args| backend.dispatch(cmd, args);
    invoke(
        "create_project",
        json!({"name": "Registry", "mode": "ThreeD", "location": root.join("projects")}),
    )
    .unwrap();
    let actor = invoke("add_actor", json!({"shape": "Sphere", "name": "Lamp"})).unwrap();
    let actor = actor.as_str().unwrap();

    let types = invoke("type_registry", json!({})).unwrap();
    let light = types["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "Light")
        .expect("Light is registered");
    assert_eq!(light["default"]["light"]["intensity"], 800.0);
    assert!(
        types["world"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "fog")
    );

    // Added with the registry's defaults.
    invoke("add_component", json!({"actorId": actor, "type": "Light"})).unwrap();
    let state = invoke("get_state", json!({})).unwrap();
    assert_eq!(component(&state, actor, "Light")["light"]["range"], 20.0);
    assert!(invoke("add_component", json!({"actorId": actor, "type": "Script"})).is_err());
    assert!(invoke("add_component", json!({"actorId": actor, "type": "Nope"})).is_err());

    // One checked field at a time.
    let target = format!("actor:{actor}/Light");
    invoke(
        "set_property",
        json!({"target": target, "path": "light.intensity", "value": 1500}),
    )
    .unwrap();
    invoke(
        "set_property",
        json!({"target": target, "path": "light.beam.motes.count", "value": 300}),
    )
    .unwrap();
    let err = invoke(
        "set_property",
        json!({"target": target, "path": "light.intensity", "value": -1}),
    )
    .unwrap_err();
    assert!(err.contains("light.intensity"), "{err}");
    assert!(
        invoke(
            "set_property",
            json!({"target": target, "path": "light.nonsense", "value": 1})
        )
        .is_err()
    );
    let state = invoke("get_state", json!({})).unwrap();
    let light = &component(&state, actor, "Light")["light"];
    assert_eq!(light["intensity"], 1500.0);
    assert_eq!(light["beam"]["motes"]["count"], 300);

    // World settings, colors upper-cased like a picker writes them.
    invoke(
        "set_property",
        json!({"target": "world", "path": "fog.height.day_color", "value": "#a0b0c0"}),
    )
    .unwrap();
    invoke(
        "set_property",
        json!({"target": "world", "path": "fog.height.distance", "value": 120}),
    )
    .unwrap();
    let state = invoke("get_state", json!({})).unwrap();
    let height = &state["project"]["world"]["fog"]["height"];
    assert_eq!(height["day_color"], "#A0B0C0");
    assert_eq!(height["distance"], 120.0);

    // Two writes to one path are one undo step; the color before it another.
    invoke("undo", json!({})).unwrap();
    let state = invoke("get_state", json!({})).unwrap();
    let height = &state["project"]["world"]["fog"]["height"];
    assert_eq!(height["day_color"], "#A0B0C0");
    assert_ne!(height["distance"], 120.0);
    let _ = std::fs::remove_dir_all(&root);
}
