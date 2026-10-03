//! Phase 1 gate: save, load, clone and reparent keep identities; old projects load
//! and re-save untouched; invisible static colliders and bodies without a shape are
//! representable.

use blockloom_core::components::ActorComponent;
use blockloom_core::physics::{
    ColliderShape, ColliderSpec, MassSource, MaterialBody, MaterialRef, PhysicsMaterial,
    PhysicsOwnership, RigidbodySpec, SCHEMA_VERSION,
};
use blockloom_core::project::{
    Actor, Project, create_project, project_file, read_project_dir, save_project,
};
use blockloom_core::scene::{BodyKind, Mode, Physics, Visual};
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("blockloom-gate-{}", uuid_like()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{n}-{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

fn cube() -> Visual {
    Visual::Cuboid {
        color: "#ffffff".into(),
        size: [1.0; 3],
    }
}

fn add(project: &mut Project, name: &str) -> String {
    project.add_actor(Actor::new(name, cube()))
}

fn scene_bytes(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e == "blockloom" || e == "blockscene")
            {
                files.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn a_legacy_project_loads_and_resaves_byte_for_byte() {
    let temp = TempDir::new();
    let mut project = Project::starter("Old", Mode::ThreeD);
    let id = add(&mut project, "Crate");
    project
        .actor_mut(&id)
        .unwrap()
        .components
        .set_physics(Physics {
            body: BodyKind::Dynamic,
            ..Default::default()
        });
    let dir = create_project(&project, &temp.0).unwrap();
    let before = scene_bytes(&dir);
    let index = std::fs::read_to_string(project_file(&dir)).unwrap();
    assert!(
        !index.contains("physics"),
        "a legacy project has no section: {index}"
    );

    let loaded = read_project_dir(&dir).unwrap();
    assert!(loaded.physics.is_default());
    let actor = loaded.actor(&id).unwrap();
    assert!(
        actor.components.get("Body").is_some(),
        "the legacy Body stays"
    );
    assert!(actor.components.rigidbody().is_none());
    assert_eq!(actor.components.colliders().count(), 0);

    save_project(&loaded, &dir).unwrap();
    assert_eq!(
        before,
        scene_bytes(&dir),
        "an unmigrated project re-saves identically"
    );
}

#[test]
fn ids_and_stored_materials_survive_a_disk_round_trip() {
    let temp = TempDir::new();
    let mut project = Project::starter("New", Mode::ThreeD);
    let id = add(&mut project, "Rock");
    let material = project.physics.materials.find_or_add(
        "Slick",
        MaterialBody::Three {
            material: PhysicsMaterial {
                static_friction: 0.05,
                dynamic_friction: 0.05,
                ..Default::default()
            },
        },
    );
    project.physics.stamp();
    let library = project.physics.materials.clone();
    project
        .active_scene_mut()
        .set_rigidbody(&id, RigidbodySpec::default(), &library)
        .unwrap();
    let mut spec = ColliderSpec::new(ColliderShape::Sphere { radius: 0.5 });
    spec.material = MaterialRef::Asset {
        id: material.clone(),
    };
    let collider = project
        .active_scene_mut()
        .add_collider(&id, spec, &library)
        .unwrap();
    let body = project
        .actor(&id)
        .unwrap()
        .components
        .rigidbody()
        .unwrap()
        .id
        .clone();

    let dir = create_project(&project, &temp.0).unwrap();
    let loaded = read_project_dir(&dir).unwrap();
    assert_eq!(loaded.physics.schema_version, SCHEMA_VERSION);
    assert!(loaded.physics.materials.get(&material).is_some());
    let actor = loaded.actor(&id).unwrap();
    assert_eq!(actor.components.rigidbody().unwrap().id, body);
    let found = actor.components.collider(&collider).expect("the same id");
    assert_eq!(found.material, MaterialRef::Asset { id: material });
    assert_eq!(loaded, project);
}

#[test]
fn a_cloned_actor_gets_new_ids_but_keeps_what_it_refers_to() {
    let mut project = Project::starter("Clone", Mode::ThreeD);
    let id = add(&mut project, "Rock");
    let library = project.physics.materials.clone();
    let mut spec = ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] });
    spec.material = MaterialRef::BuiltIn { name: "Ice".into() };
    let first = project
        .active_scene_mut()
        .add_collider(&id, spec, &library)
        .unwrap();
    project
        .active_scene_mut()
        .set_rigidbody(&id, RigidbodySpec::default(), &library)
        .unwrap();

    let mut copy = project.actor(&id).unwrap().clone();
    copy.id = "copy".into();
    copy.refresh_physics_ids();
    project.active_scene_mut().actors.push(copy);

    let original = project.actor(&id).unwrap();
    let copied = project.actor("copy").unwrap();
    let copied_collider = copied.components.colliders().next().unwrap();
    assert_ne!(copied_collider.id, first);
    assert_eq!(
        copied_collider.material,
        MaterialRef::BuiltIn { name: "Ice".into() }
    );
    assert_ne!(
        copied.components.rigidbody().unwrap().id,
        original.components.rigidbody().unwrap().id
    );
    assert!(project.active_scene().physics_issues(&library).is_empty());
}

#[test]
fn reparenting_keeps_ids_and_moves_the_collider_to_the_new_body() {
    let mut project = Project::starter("Reparent", Mode::ThreeD);
    let car = add(&mut project, "Car");
    let wheel = add(&mut project, "Wheel");
    let library = project.physics.materials.clone();
    let scene = project.active_scene_mut();
    scene
        .set_rigidbody(&car, RigidbodySpec::default(), &library)
        .unwrap();
    let wheel_collider = scene
        .add_collider(
            &wheel,
            ColliderSpec::new(ColliderShape::Sphere { radius: 0.3 }),
            &library,
        )
        .unwrap();
    scene
        .actors
        .iter_mut()
        .find(|a| a.id == wheel)
        .unwrap()
        .components
        .placement_mut()
        .position = [2.0, 0.0, 0.0];

    let table = PhysicsOwnership::resolve(&project.active_scene().actors);
    assert_eq!(
        table.collider(&wheel_collider).unwrap().body_actor,
        None,
        "static scenery"
    );

    assert!(
        project
            .active_scene_mut()
            .move_actor(&wheel, &car, "")
            .unwrap_or(false)
    );
    let table = PhysicsOwnership::resolve(&project.active_scene().actors);
    let owner = table
        .collider(&wheel_collider)
        .expect("same id after the move");
    assert_eq!(owner.body_actor.as_deref(), Some(car.as_str()));
    assert!(
        (owner.local_pose.position[0] - 2.0).abs() < 1e-4,
        "{:?}",
        owner.local_pose
    );

    assert!(
        project
            .active_scene_mut()
            .move_actor(&wheel, "", "")
            .unwrap_or(false)
    );
    let table = PhysicsOwnership::resolve(&project.active_scene().actors);
    assert_eq!(table.collider(&wheel_collider).unwrap().body_actor, None);
}

#[test]
fn a_newer_schema_is_refused_with_a_clear_error() {
    let temp = TempDir::new();
    let mut project = Project::starter("Future", Mode::ThreeD);
    project.physics.stamp();
    let dir = create_project(&project, &temp.0).unwrap();
    let path = project_file(&dir);
    let text = std::fs::read_to_string(&path).unwrap();
    let edited = text.replace(
        &format!("\"schema_version\": {SCHEMA_VERSION}"),
        &format!("\"schema_version\": {}", SCHEMA_VERSION + 1),
    );
    assert_ne!(text, edited);
    std::fs::write(&path, edited).unwrap();
    let error = read_project_dir(&dir).unwrap_err();
    assert!(error.contains("physics schema"), "{error}");
}

#[test]
fn an_invisible_static_collider_and_a_body_without_a_shape_are_representable() {
    let mut project = Project::starter("Fixtures", Mode::ThreeD);
    let wall = add(&mut project, "Invisible wall");
    let lonely = add(&mut project, "Lonely body");
    let library = project.physics.materials.clone();
    let scene = project.active_scene_mut();
    // No Look at all, and no body: a static collider.
    scene
        .actors
        .iter_mut()
        .find(|a| a.id == wall)
        .unwrap()
        .components
        .remove("Look");
    let id = scene
        .add_collider(
            &wall,
            ColliderSpec::new(ColliderShape::Box {
                size: [10.0, 4.0, 0.2],
            }),
            &library,
        )
        .unwrap();
    // A body with nothing to collide as.
    let body = RigidbodySpec {
        mass: MassSource::Explicit { mass: 2.0 },
        ..Default::default()
    };
    scene.set_rigidbody(&lonely, body, &library).unwrap();

    let table = PhysicsOwnership::resolve(&scene.actors);
    assert!(table.static_colliders().any(|c| c.collider == id));
    assert!(table.bodies_without_shape().any(|b| b.actor == lonely));
    assert!(
        scene
            .physics_issues(&library)
            .iter()
            .all(|i| i.severity != blockloom_core::physics::Severity::Error),
        "{:?}",
        scene.physics_issues(&library)
    );
    let json = serde_json::to_string(&scene.actors).unwrap();
    let back: Vec<Actor> = serde_json::from_str(&json).unwrap();
    assert_eq!(back.len(), scene.actors.len());
    assert!(back.iter().any(|a| {
        a.components
            .iter()
            .find(|c| matches!(c, ActorComponent::Collider { .. }))
            .is_some()
    }));
}
