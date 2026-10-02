use blockloom_app::{AppHandle, Backend};
use serde_json::{Value, json};
use std::path::PathBuf;

fn data_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("blockloom-physics-{}", uuid::Uuid::new_v4()));
        unsafe {
            std::env::set_var("BLOCKLOOM_DATA_DIR", root.join("data"));
        }
        root
    })
    .clone()
}

fn backend(name: &str) -> (Backend, PathBuf) {
    let root = data_root().join(name);
    let backend = Backend::start(AppHandle::new(|_| {}));
    backend
        .dispatch(
            "create_project",
            json!({"name": name, "mode": "ThreeD", "location": root.join("projects")}),
        )
        .unwrap();
    (backend, root)
}

fn actor(backend: &Backend, shape: &str, name: &str) -> String {
    backend
        .dispatch("add_actor", json!({"shape": shape, "name": name}))
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

fn colliders(backend: &Backend, actor: &str) -> Vec<Value> {
    let state = backend.dispatch("get_state", json!({})).unwrap();
    let found = state["project"]["actors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == actor)
        .cloned()
        .unwrap();
    found["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["component"] == "Collider")
        .cloned()
        .collect()
}

fn sphere() -> Value {
    json!({"geometry": {"kind": "Shape", "shape": {"kind": "Sphere", "radius": 0.5}}})
}

#[test]
fn colliders_are_added_edited_and_removed_by_id_with_undo() {
    let (b, _root) = backend("ids");
    let rock = actor(&b, "Sphere", "Rock");
    let id = b
        .dispatch(
            "add_collider",
            json!({"actorId": rock, "collider": sphere()}),
        )
        .unwrap();
    let id = id.as_str().unwrap().to_owned();
    assert_eq!(colliders(&b, &rock).len(), 1);

    let mut edited = colliders(&b, &rock)[0]["collider"].clone();
    edited["trigger"] = json!(true);
    b.dispatch("set_collider", json!({"collider": edited}))
        .unwrap();
    assert_eq!(colliders(&b, &rock)[0]["collider"]["id"], id.as_str());
    assert_eq!(colliders(&b, &rock)[0]["collider"]["trigger"], true);

    // Undo goes back to the pre-edit snapshot with the same identity.
    b.dispatch("undo", json!({})).unwrap();
    assert_eq!(colliders(&b, &rock)[0]["collider"]["id"], id.as_str());
    assert_eq!(colliders(&b, &rock)[0]["collider"]["trigger"], false);
    b.dispatch("redo", json!({})).unwrap();
    assert_eq!(colliders(&b, &rock)[0]["collider"]["trigger"], true);

    b.dispatch("remove_collider", json!({"colliderId": id}))
        .unwrap();
    assert!(colliders(&b, &rock).is_empty());
    assert!(
        b.dispatch("remove_collider", json!({"colliderId": id}))
            .is_err()
    );
}

#[test]
fn a_refused_edit_leaves_no_undo_step() {
    let (b, _root) = backend("refused");
    let rock = actor(&b, "Sphere", "Rock");
    b.dispatch(
        "add_collider",
        json!({"actorId": rock, "collider": sphere()}),
    )
    .unwrap();
    let bad = json!({"geometry": {"kind": "Shape", "shape": {"kind": "Sphere", "radius": -1.0}}});
    assert!(
        b.dispatch("add_collider", json!({"actorId": rock, "collider": bad}))
            .is_err()
    );
    assert_eq!(colliders(&b, &rock).len(), 1);
    // One undo removes the one collider that was added, not a phantom step.
    b.dispatch("undo", json!({})).unwrap();
    assert!(colliders(&b, &rock).is_empty());
}

#[test]
fn duplicating_an_actor_gives_its_colliders_new_ids() {
    let (b, _root) = backend("clone");
    let rock = actor(&b, "Sphere", "Rock");
    b.dispatch(
        "add_collider",
        json!({"actorId": rock, "collider": sphere()}),
    )
    .unwrap();
    b.dispatch("set_rigidbody", json!({"actorId": rock, "rigidbody": {}}))
        .unwrap();
    let copy = b
        .dispatch("duplicate_actor", json!({"actorId": rock}))
        .unwrap();
    let copy = copy.as_str().unwrap().to_owned();
    let original = colliders(&b, &rock)[0]["collider"]["id"].clone();
    let copied = colliders(&b, &copy)[0]["collider"]["id"].clone();
    assert_ne!(original, copied);
    let check = b.dispatch("physics_check", json!({})).unwrap();
    assert_eq!(check["errors"], 0, "{check}");
}

#[test]
fn the_generic_component_commands_send_callers_to_the_typed_ones() {
    let (b, _root) = backend("generic");
    let rock = actor(&b, "Sphere", "Rock");
    let component =
        json!({"component": "Collider", "collider": {"geometry": {"kind": "FromLook"}}});
    let error = b
        .dispatch(
            "add_actor_component",
            json!({"actorId": rock, "component": component}),
        )
        .unwrap_err();
    assert!(error.contains("add-collider"), "{error}");
}

#[test]
fn a_stored_material_is_used_by_id_and_kept_while_in_use() {
    let (b, _root) = backend("materials");
    let rock = actor(&b, "Sphere", "Rock");
    let id = b
        .dispatch(
            "add_physics_material",
            json!({"name": "Slick", "material": {"dimension": "Three", "material": {
                "static_friction": 0.05, "dynamic_friction": 0.05, "bounciness": 0.0}}}),
        )
        .unwrap();
    let id = id.as_str().unwrap().to_owned();
    let mut spec = sphere();
    spec["material"] = json!({"kind": "Asset", "id": id});
    b.dispatch("add_collider", json!({"actorId": rock, "collider": spec}))
        .unwrap();
    let error = b
        .dispatch("remove_physics_material", json!({"id": id}))
        .unwrap_err();
    assert!(error.contains("still used"), "{error}");
    // A bad value is refused with the field named.
    let error = b
        .dispatch(
            "set_physics_material",
            json!({"id": id, "material": {"dimension": "Three", "material": {
                "static_friction": -1.0, "dynamic_friction": 0.1, "bounciness": 0.0}}}),
        )
        .unwrap_err();
    assert!(error.contains("friction"), "{error}");
    // A 2D material can't take a 3D one's place.
    assert!(b
        .dispatch(
            "set_physics_material",
            json!({"id": id, "material": {"dimension": "Two", "material": {"friction": 0.4, "bounciness": 0.0}}}),
        )
        .is_err());
}

#[test]
fn the_migration_preview_changes_nothing() {
    let (b, _root) = backend("preview");
    // The starter project's Player and Ground still carry the legacy Body.
    let ownership = b.dispatch("physics_ownership", json!({})).unwrap();
    assert!(!ownership["legacyBodies"].as_array().unwrap().is_empty());
    let before = b.dispatch("get_state", json!({})).unwrap()["project"].clone();
    let preview = b.dispatch("physics_migration_preview", json!({})).unwrap();
    assert_eq!(preview["applied"], false);
    assert!(!preview["actors"].as_array().unwrap().is_empty());
    assert_eq!(
        before,
        b.dispatch("get_state", json!({})).unwrap()["project"]
    );
    assert!(b.dispatch("physics_properties", json!({})).unwrap()["collider"].is_array());
}
