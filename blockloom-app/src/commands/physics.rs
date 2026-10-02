//! Physics commands: the Rigidbody and Collider components, the project's
//! material library and the checks over them.
//!
//! The runtime still plays the legacy `Body` until the physics plan's phase 2, so
//! these edit and validate the document only. Every edit goes through the scene's
//! transactional operations (`blockloom_core::physics::edit`), which refuse a
//! change that would leave its actor invalid, and a refused edit leaves no undo
//! step behind.

use super::{auto_save, emit, lock, sync_runtime};
use crate::AppHandle;
use crate::state::{EditSession, SharedState};
use blockloom_core::build::ExtraFile;
use blockloom_core::components::ActorComponent;
use blockloom_core::physics::controller::CharacterControllerSpec;
use blockloom_core::physics::cook::{
    CollisionLookup, CookControl, Decompose, FolderCollision, NoCollisionData, Source, cook_project,
};
use blockloom_core::physics::{
    ColliderId, ColliderSpec, CompatibilityProfile, MaterialBody, MaterialLibrary, MaterialRef,
    PhysicsOwnership, RigidbodySpec, Severity, meta,
};
use blockloom_core::project::{Project, Scene};
use blockloom_core::scene::Mode;
use serde_json::{Value, json};

/// Runs `edit` on the open project; on success files an undo step (coalesced
/// under `session` when given), saves and tells the editor. A failed edit
/// changes nothing and files nothing.
fn edit<R>(
    state: &SharedState,
    app: &AppHandle,
    session: Option<String>,
    edit: impl FnOnce(&mut Project) -> Result<R, String>,
) -> Result<R, String> {
    let mut s = lock(state)?;
    let snapshot = s.project().cloned().ok_or("No project is open")?;
    let project = s.project_mut().ok_or("No project is open")?;
    let result = match edit(project) {
        Ok(result) => {
            project.physics.stamp();
            result
        }
        Err(error) => {
            *project = snapshot;
            return Err(error);
        }
    };
    match session {
        Some(comment_id) => {
            s.history
                .push_for_session(snapshot, EditSession::Comment { comment_id });
        }
        None => s.history.push(snapshot),
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(result)
}

/// The active scene with the library its materials resolve against.
fn with_scene<R>(
    project: &mut Project,
    f: impl FnOnce(&mut Scene, &MaterialLibrary) -> Result<R, String>,
) -> Result<R, String> {
    let library = project.physics.materials.clone();
    f(project.active_scene_mut(), &library)
}

/// Whether any collider in the project refers to material `id`.
fn material_in_use(project: &Project, id: &str) -> Option<String> {
    for scene in &project.scenes {
        for actor in &scene.actors {
            for collider in actor.components.colliders() {
                if collider.material == (MaterialRef::Asset { id: id.to_string() }) {
                    return Some(format!("\"{}\" on \"{}\"", collider.name, actor.name));
                }
            }
        }
    }
    None
}

pub(crate) fn add_collider(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    collider: ColliderSpec,
) -> Result<String, String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene
                .add_collider(&actor_id, collider, library)
                .map(|id| id.to_string())
        })
    })
}

pub(crate) fn set_collider(
    state: &SharedState,
    app: &AppHandle,
    collider: ColliderSpec,
) -> Result<(), String> {
    let session = Some(format!("physics-collider:{}", collider.id));
    edit(state, app, session, |project| {
        with_scene(project, |scene, library| {
            scene.set_collider(collider, library)
        })
    })
}

pub(crate) fn remove_collider(
    state: &SharedState,
    app: &AppHandle,
    collider_id: String,
) -> Result<String, String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene.remove_collider(&ColliderId::from(collider_id.as_str()), library)
        })
    })
}

pub(crate) fn fit_collider_to_look(
    state: &SharedState,
    app: &AppHandle,
    collider_id: String,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene.fit_collider_to_look(&ColliderId::from(collider_id.as_str()), library)
        })
    })
}

pub(crate) fn set_rigidbody(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    rigidbody: RigidbodySpec,
) -> Result<String, String> {
    let session = Some(format!("physics-rigidbody:{actor_id}"));
    edit(state, app, session, |project| {
        with_scene(project, |scene, library| {
            scene
                .set_rigidbody(&actor_id, rigidbody, library)
                .map(|id| id.to_string())
        })
    })
}

pub(crate) fn remove_rigidbody(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene.remove_rigidbody(&actor_id, library)
        })
    })
}

pub(crate) fn set_character_controller(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    controller: CharacterControllerSpec,
) -> Result<String, String> {
    let session = Some(format!("physics-controller:{actor_id}"));
    edit(state, app, session, |project| {
        with_scene(project, |scene, library| {
            scene
                .set_character_controller(&actor_id, controller, library)
                .map(|id| id.to_string())
        })
    })
}

pub(crate) fn remove_character_controller(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene.remove_character_controller(&actor_id, library)
        })
    })
}

pub(crate) fn set_physics_profile(
    state: &SharedState,
    app: &AppHandle,
    profile: CompatibilityProfile,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        project.physics.profile = profile;
        Ok(())
    })
}

/// Names a collision layer (1 to 32); an empty name goes back to "Layer N".
pub(crate) fn set_physics_layer_name(
    state: &SharedState,
    app: &AppHandle,
    layer: u8,
    name: String,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        project.physics.layers.set_name(layer, &name)
    })
}

/// Switches collisions between two layers on or off for one dimension.
pub(crate) fn set_layer_collision(
    state: &SharedState,
    app: &AppHandle,
    mode: Mode,
    a: u8,
    b: u8,
    collides: bool,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        project.physics.layers.set_collides(mode, a, b, collides)
    })
}

pub(crate) fn add_physics_material(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    material: MaterialBody,
) -> Result<String, String> {
    edit(state, app, None, |project| {
        material.validate()?;
        project.physics.materials.add(&name, material)
    })
}

pub(crate) fn set_physics_material(
    state: &SharedState,
    app: &AppHandle,
    id: String,
    name: Option<String>,
    material: MaterialBody,
) -> Result<(), String> {
    let session = Some(format!("physics-material:{id}"));
    edit(state, app, session, |project| {
        material.validate()?;
        // A stored material keeps its dimension: colliders resolve it per world.
        if let Some(current) = project.physics.materials.get(&id)
            && current.body.mode() != material.mode()
        {
            return Err("A material can't change between 2D and 3D".to_string());
        }
        project
            .physics
            .materials
            .update(&id, name.as_deref(), material)
    })
}

pub(crate) fn remove_physics_material(
    state: &SharedState,
    app: &AppHandle,
    id: String,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        if let Some(user) = material_in_use(project, &id) {
            return Err(format!("This material is still used by collider {user}"));
        }
        project.physics.materials.remove(&id)
    })
}

// ─── Cooking ───────────────────────────────────────────────────────────────

/// Where the plan finds cooked collision: cooked (and cached) from the model
/// files of the open project, or nothing without a folder.
fn lookup_for(dir: Option<&std::path::Path>) -> Box<dyn CollisionLookup> {
    match dir {
        Some(dir) => Box::new(FolderCollision::new(dir, Source::Cook)),
        None => Box::new(NoCollisionData),
    }
}

/// The collision files a build of `project` ships: every cooked mesh and the
/// manifest the player finds them by, staged in the project's cooked folder.
/// A mesh that won't cook fails the build rather than shipping a body that
/// falls through its floor.
pub(crate) fn collision_extras(
    project: &Project,
    dir: &std::path::Path,
) -> Result<Vec<ExtraFile>, String> {
    let (lookup, report) = cook_project(project, dir, CookControl::new());
    if let Some(error) = report.errors.first() {
        return Err(format!("Collision data could not be made: {error}"));
    }
    let mut extras = Vec::new();
    for (path, bytes) in lookup.manifest_files()? {
        let from = dir.join(&path);
        if path.ends_with(blockloom_core::physics::cook::lookup::MANIFEST) {
            if let Some(parent) = from.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&from, bytes).map_err(|e| format!("{}: {e}", from.display()))?;
        }
        extras.push(ExtraFile { to: path, from });
    }
    Ok(extras)
}

/// `physics-cook`: makes (or reuses) the collision data of every mesh collider
/// in every scene and reports what each came to.
pub(crate) fn physics_cook(state: &SharedState) -> Result<Value, String> {
    let (project, dir) = {
        let s = lock(state)?;
        let project = s.project().cloned().ok_or("No project is open")?;
        let dir = s.project_dir().map(|dir| dir.to_path_buf());
        (project, dir)
    };
    let dir = dir.ok_or("The project has no folder to cook collision in")?;
    let (_, report) = cook_project(&project, &dir, CookControl::new());
    Ok(json!({
        "ok": report.errors.is_empty(),
        "errors": report.errors,
        "meshes": report.entries.iter().map(|entry| json!({
            "mesh": entry.mesh,
            "kind": entry.kind.name(),
            "stats": entry.stats,
        })).collect::<Vec<_>>(),
    }))
}

/// `set-physics-cooking`: the project's cooking limits, and which meshes cook
/// to several hulls. `decompose` of null forgets the mesh's decomposition.
pub(crate) fn set_physics_cooking(
    state: &SharedState,
    app: &AppHandle,
    weld: Option<f32>,
    max_hull_vertices: Option<u16>,
    mesh: Option<String>,
    decompose: Option<Value>,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        let cooking = &mut project.physics.cooking;
        if let Some(weld) = weld {
            cooking.weld = weld;
        }
        if let Some(max) = max_hull_vertices {
            cooking.max_hull_vertices = max;
        }
        match (mesh, decompose) {
            (Some(mesh), Some(Value::Null)) => {
                cooking.decompose.remove(&mesh);
            }
            (Some(mesh), Some(settings)) => {
                let settings: Decompose = serde_json::from_value(settings)
                    .map_err(|e| format!("invalid decompose settings: {e}"))?;
                cooking.decompose.insert(mesh, settings);
            }
            (None, Some(_)) => return Err("Name the mesh to decompose".into()),
            _ => {}
        }
        cooking.check()
    })
}

// ─── Reads ─────────────────────────────────────────────────────────────────

/// `physics-check`: every physics problem in every scene.
pub(crate) fn physics_check(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let mut issues = Vec::new();
    let lookup = lookup_for(s.project_dir());
    for scene in &project.scenes {
        for issue in scene.physics_plan_with(&project.physics, &*lookup).issues {
            let mut value = serde_json::to_value(&issue).map_err(|e| e.to_string())?;
            value["scene"] = json!(scene.name);
            issues.push((issue.severity, value));
        }
    }
    let errors = issues
        .iter()
        .filter(|(severity, _)| *severity == Severity::Error)
        .count();
    let warnings = issues.len() - errors;
    Ok(json!({
        "ok": errors == 0,
        "errors": errors,
        "warnings": warnings,
        "schemaVersion": project.physics.schema_version,
        "profile": project.physics.profile,
        "issues": issues.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
    }))
}

/// Refuses Play or Build while the scene that would run has a physics error.
pub(crate) fn preflight(
    project: &Project,
    dir: Option<&std::path::Path>,
    action: &str,
) -> Result<(), String> {
    let lookup = lookup_for(dir);
    let plan = project
        .active_scene()
        .physics_plan_with(&project.physics, &*lookup);
    let errors: Vec<String> = plan
        .errors()
        .map(|issue| match &issue.actor {
            Some(actor) => format!("{actor}: {}", issue.message),
            None => issue.message.clone(),
        })
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Physics problems stop {action}:\n- {}",
        errors.join("\n- ")
    ))
}

/// `physics-plan`: what Play would install for the active scene (bodies with
/// their mass split, shapes with their poses, materials and filter groups).
pub(crate) fn physics_plan(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let scene = project.active_scene();
    let lookup = lookup_for(s.project_dir());
    let plan = scene.physics_plan_with(&project.physics, &*lookup);
    let mode = scene.world.mode;
    Ok(json!({
        "scene": scene.name,
        "runnable": plan.is_runnable(),
        "exactFiltering": plan.exact_filtering,
        "bodies": plan.bodies,
        "colliders": plan.colliders,
        "layers": {
            "names": (1..=32u8).map(|n| project.physics.layers.name(n)).collect::<Vec<_>>(),
            "disabled": match mode {
                Mode::ThreeD => &project.physics.layers.disabled_3d,
                Mode::TwoD => &project.physics.layers.disabled_2d,
            },
        },
        "issues": plan.issues,
    }))
}

/// `physics-ownership`: which body carries each collider in the active scene,
/// and where in the body's frame the shape sits.
pub(crate) fn physics_ownership(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let scene = project.active_scene();
    let table = PhysicsOwnership::resolve(&scene.actors);
    let legacy: Vec<&str> = scene
        .actors
        .iter()
        .filter(|actor| actor.components.get("Body").is_some())
        .map(|actor| actor.id.as_str())
        .collect();
    Ok(json!({
        "colliders": table.colliders,
        "bodies": table.bodies,
        "staticColliders": table.static_colliders().map(|c| c.collider.clone()).collect::<Vec<_>>(),
        "bodiesWithoutShape": table.bodies_without_shape().map(|b| b.actor.clone()).collect::<Vec<_>>(),
        "legacyBodies": legacy,
    }))
}

/// `physics-properties`: the units, bounds and visibility of every field.
pub(crate) fn physics_properties() -> Value {
    json!({
        "rigidbody": meta::RIGIDBODY,
        "collider": meta::COLLIDER,
        "material": meta::MATERIAL,
    })
}

/// `physics-migration-preview`: what converting each legacy `Body` would
/// store. Nothing is changed; the runtime still reads `Body`.
pub(crate) fn physics_migration_preview(state: &SharedState) -> Result<Value, String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let scene = project.active_scene();
    let migrations = scene.physics_migration_preview(&project.physics.materials);
    Ok(json!({
        "scene": scene.name,
        "applied": false,
        "actors": migrations,
    }))
}

/// The generic component commands address a component by name, which is
/// ambiguous for repeated colliders and skips the physics checks, so they send
/// callers to the typed commands.
pub(crate) fn refuse_generic(component: &ActorComponent) -> Result<(), String> {
    match component {
        ActorComponent::Collider { .. } => Err(
            "Use add-collider and set-collider for colliders: they are addressed by id".to_string(),
        ),
        ActorComponent::Rigidbody { .. } => Err("Use set-rigidbody for a Rigidbody".to_string()),
        ActorComponent::CharacterController { .. } => {
            Err("Use set-character-controller for a CharacterController".to_string())
        }
        _ => Ok(()),
    }
}
