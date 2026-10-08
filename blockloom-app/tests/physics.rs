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
    assert!(preview["total"].as_u64().unwrap() > 0);
    assert!(
        !preview["scenes"][0]["actors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        before,
        b.dispatch("get_state", json!({})).unwrap()["project"]
    );
    assert!(b.dispatch("physics_properties", json!({})).unwrap()["collider"].is_array());
}

#[test]
fn a_sample_project_opens_runnable_in_both_dimensions() {
    for (mode, name) in [("ThreeD", "sample3"), ("TwoD", "sample2")] {
        let root = data_root().join(name);
        let backend = Backend::start(AppHandle::new(|_| {}));
        backend
            .dispatch(
                "create_project",
                json!({"name": name, "mode": mode, "location": root.join("projects"),
                    "sample": "physics-playground"}),
            )
            .unwrap();
        let plan = backend.dispatch("physics_plan", json!({})).unwrap();
        assert_eq!(plan["runnable"], true, "{mode}: {plan}");
        assert!(plan["constraints"].as_array().is_none_or(|c| c.len() >= 3));
        let ownership = backend.dispatch("physics_ownership", json!({})).unwrap();
        assert!(ownership["legacyBodies"].as_array().unwrap().is_empty());
    }
    let backend = Backend::start(AppHandle::new(|_| {}));
    let error = backend
        .dispatch(
            "create_project",
            json!({"name": "nope", "location": data_root().join("nope").join("projects"),
                "sample": "missing"}),
        )
        .unwrap_err();
    assert!(error.contains("physics-playground"), "{error}");
}

#[test]
fn the_physics_upgrade_converts_backs_up_and_undoes() {
    let (b, root) = backend("upgrade");
    let legacy = |b: &Backend| {
        b.dispatch("physics_ownership", json!({})).unwrap()["legacyBodies"]
            .as_array()
            .unwrap()
            .len()
    };
    let before = legacy(&b);
    assert!(before >= 2);
    // One actor first: only it converts.
    let first = b.dispatch("physics_migration_preview", json!({})).unwrap()["scenes"][0]["actors"]
        [0]["actor"]
        .as_str()
        .unwrap()
        .to_string();
    let one = b
        .dispatch("physics_migration_preview", json!({"actorId": first}))
        .unwrap();
    assert_eq!(one["total"], 1);
    let done = b
        .dispatch("migrate_physics", json!({"actorId": first}))
        .unwrap();
    assert_eq!(done["applied"], true);
    assert_eq!(done["total"], 1);
    assert_eq!(legacy(&b), before - 1);
    // Then the rest, and a third call has nothing left.
    let rest = b.dispatch("migrate_physics", json!({})).unwrap();
    assert_eq!(rest["total"].as_u64().unwrap() as usize, before - 1);
    assert_eq!(legacy(&b), 0);
    assert_eq!(
        b.dispatch("migrate_physics", json!({})).unwrap()["total"],
        0
    );
    // The project plays from components now.
    assert_eq!(
        b.dispatch("physics_plan", json!({})).unwrap()["runnable"],
        true
    );
    // The old file was kept aside.
    let backups = root
        .join("projects")
        .join("upgrade")
        .join(".blockloom")
        .join("backups");
    assert!(std::fs::read_dir(&backups).map(|d| d.count()).unwrap_or(0) >= 1);
    // Undo walks it back one step at a time.
    b.dispatch("undo", json!({})).unwrap();
    b.dispatch("undo", json!({})).unwrap();
    assert_eq!(legacy(&b), before);
}

#[test]
fn layers_are_named_and_switched_with_undo_and_the_plan_reports_them() {
    let (b, _root) = backend("layers");
    b.dispatch(
        "set_physics_layer_name",
        json!({"layer": 3, "name": "Enemies"}),
    )
    .unwrap();
    b.dispatch(
        "set_layer_collision",
        json!({"mode": "ThreeD", "a": 4, "b": 3, "collides": false}),
    )
    .unwrap();
    let plan = b.dispatch("physics_plan", json!({})).unwrap();
    assert_eq!(plan["layers"]["names"][2], "Enemies");
    assert_eq!(plan["layers"]["disabled"], json!([[3, 4]]));
    assert_eq!(plan["runnable"], true);

    assert!(
        b.dispatch(
            "set_layer_collision",
            json!({"mode": "ThreeD", "a": 0, "b": 3, "collides": false}),
        )
        .is_err()
    );
    b.dispatch("undo", json!({})).unwrap();
    let plan = b.dispatch("physics_plan", json!({})).unwrap();
    assert_eq!(plan["layers"]["disabled"], json!([]));
    assert_eq!(plan["layers"]["names"][2], "Enemies");
}

#[test]
fn the_plan_lists_bodies_and_refuses_what_it_cannot_build() {
    let (b, _root) = backend("plan");
    let id = actor(&b, "Sphere", "Crate");
    b.dispatch(
        "set_rigidbody",
        json!({"actorId": id, "rigidbody": {"mass": {"mode": "Explicit", "mass": 3.0}}}),
    )
    .unwrap();
    b.dispatch("add_collider", json!({"actorId": id, "collider": sphere()}))
        .unwrap();
    let plan = b.dispatch("physics_plan", json!({})).unwrap();
    let body = plan["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|body| body["actor"] == id.as_str())
        .unwrap();
    assert!((body["total_mass"].as_f64().unwrap() - 3.0).abs() < 1e-3);

    b.dispatch(
        "add_collider",
        json!({"actorId": id, "collider": {"geometry": {"kind": "Shape", "shape": {"kind": "ConvexHull", "mesh": "rock.glb"}}}}),
    )
    .unwrap();
    let plan = b.dispatch("physics_plan", json!({})).unwrap();
    assert_eq!(plan["runnable"], false);
    let check = b.dispatch("physics_check", json!({})).unwrap();
    assert_eq!(check["ok"], false);
    assert!(
        check["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["message"].as_str().unwrap().contains("rock.glb"))
    );
}

const CUBE_OBJ: &str = "v -0.5 -0.5 -0.5\nv 0.5 -0.5 -0.5\nv 0.5 0.5 -0.5\nv -0.5 0.5 -0.5\n\
v -0.5 -0.5 0.5\nv 0.5 -0.5 0.5\nv 0.5 0.5 0.5\nv -0.5 0.5 0.5\n\
f 1 3 2\nf 1 4 3\nf 5 6 7\nf 5 7 8\nf 1 2 6\nf 1 6 5\nf 4 7 3\nf 4 8 7\nf 1 5 8\nf 1 8 4\nf 2 3 7\nf 2 7 6\n";

fn project_folder(root: &std::path::Path, name: &str) -> PathBuf {
    root.join("projects").join(name)
}

#[test]
fn cooking_makes_collision_from_a_model_and_reports_it() {
    let (b, root) = backend("cook");
    std::fs::write(
        project_folder(&root, "cook").join("assets/crate.obj"),
        CUBE_OBJ,
    )
    .unwrap();
    let id = actor(&b, "Sphere", "Crate");
    b.dispatch(
        "set_rigidbody",
        json!({"actorId": id, "rigidbody": {"mass": {"mode": "Explicit", "mass": 3.0}}}),
    )
    .unwrap();
    b.dispatch(
        "add_collider",
        json!({"actorId": id, "collider": {"geometry": {"kind": "Shape", "shape": {"kind": "ConvexHull", "mesh": "assets/crate.obj"}}}}),
    )
    .unwrap();

    let plan = b.dispatch("physics_plan", json!({})).unwrap();
    assert_eq!(plan["runnable"], true, "{plan}");
    let cook = b.dispatch("physics_cook", json!({})).unwrap();
    assert_eq!(cook["ok"], true, "{cook}");
    let meshes = cook["meshes"].as_array().unwrap();
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0]["kind"], "hull");
    assert_eq!(meshes[0]["stats"]["vertices"], 8);

    // A concave mesh can only be solid on a dynamic body once it decomposes.
    b.dispatch(
        "set_physics_cooking",
        json!({"mesh": "assets/crate.obj", "decompose": {"max_hulls": 4}}),
    )
    .unwrap();
    let state = b.dispatch("get_state", json!({})).unwrap();
    assert_eq!(
        state["project"]["physics"]["cooking"]["decompose"]["assets/crate.obj"]["max_hulls"],
        4
    );
    let bad = b.dispatch("set_physics_cooking", json!({"maxHullVertices": 2}));
    assert!(bad.is_err());
}

#[test]
fn a_mesh_that_cannot_cook_stops_the_check_and_the_cook() {
    let (b, _root) = backend("cookfail");
    let id = actor(&b, "Sphere", "Rock");
    b.dispatch(
        "add_collider",
        json!({"actorId": id, "collider": {"geometry": {"kind": "Shape", "shape": {"kind": "TriangleMesh", "mesh": "assets/missing.obj"}}}}),
    )
    .unwrap();
    let cook = b.dispatch("physics_cook", json!({})).unwrap();
    assert_eq!(cook["ok"], false);
    assert!(cook["errors"][0].as_str().unwrap().contains("missing.obj"));
    let check = b.dispatch("physics_check", json!({})).unwrap();
    assert_eq!(check["ok"], false);
}
