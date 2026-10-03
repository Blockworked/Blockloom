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
use blockloom_core::physics::joints::ConstraintSpec;
use blockloom_core::physics::motor::CharacterMotorSpec;
use blockloom_core::physics::presets::{PlayerPreset, PlayerProfile};
use blockloom_core::physics::{
    ColliderId, ColliderSpec, CompatibilityProfile, ConstraintId, MaterialBody, MaterialLibrary,
    MaterialRef, PhysicsOwnership, RigidbodySpec, Severity, meta,
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

pub(crate) fn add_constraint(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    constraint: ConstraintSpec,
) -> Result<String, String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene
                .add_constraint(&actor_id, constraint, library)
                .map(|id| id.to_string())
        })
    })
}

pub(crate) fn set_constraint(
    state: &SharedState,
    app: &AppHandle,
    constraint: ConstraintSpec,
) -> Result<(), String> {
    let session = Some(format!("physics-constraint:{}", constraint.id));
    edit(state, app, session, |project| {
        with_scene(project, |scene, library| {
            scene.set_constraint(constraint, library)
        })
    })
}

pub(crate) fn remove_constraint(
    state: &SharedState,
    app: &AppHandle,
    constraint_id: String,
) -> Result<String, String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene.remove_constraint(&ConstraintId::from(constraint_id.as_str()), library)
        })
    })
}

/// `list-constraints`: every constraint of the active scene (or of one actor)
/// with the name blocks use for it.
pub(crate) fn list_constraints(
    state: &SharedState,
    actor_id: Option<String>,
) -> Result<Value, String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let scene = project.active_scene();
    let mut out = Vec::new();
    for actor in &scene.actors {
        if actor_id.as_deref().is_some_and(|id| id != actor.id) {
            continue;
        }
        for (index, constraint) in actor.components.constraints().enumerate() {
            out.push(json!({
                "actorId": actor.id,
                "actor": actor.name,
                "handle": constraint.handle(index),
                "constraint": constraint,
            }));
        }
    }
    Ok(Value::Array(out))
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

pub(crate) fn set_character_motor(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    motor: CharacterMotorSpec,
) -> Result<String, String> {
    let session = Some(format!("physics-motor:{actor_id}"));
    edit(state, app, session, |project| {
        with_scene(project, |scene, library| {
            scene
                .set_character_motor(&actor_id, motor, library)
                .map(|id| id.to_string())
        })
    })
}

pub(crate) fn remove_character_motor(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene.remove_character_motor(&actor_id, library)
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

fn preset_named(word: &str) -> Result<PlayerPreset, String> {
    PlayerPreset::parse(word).ok_or_else(|| {
        let all: Vec<&str> = PlayerPreset::ALL.iter().map(|p| p.name()).collect();
        format!("Unknown player preset \"{word}\"; use {}", all.join(", "))
    })
}

/// `preview-player-preset`: what a preset would add, replace and convert on
/// an actor. Nothing is changed.
pub(crate) fn preview_player_preset(
    state: &SharedState,
    actor_id: String,
    preset: String,
) -> Result<Value, String> {
    let preset = preset_named(&preset)?;
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let preview = project
        .active_scene()
        .preview_player_preset(&actor_id, preset);
    serde_json::to_value(preview).map_err(|e| e.to_string())
}

/// `apply-player-preset`: installs a preset as one undo step. An actor that
/// already moves some other way needs `convert`.
pub(crate) fn apply_player_preset(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    preset: String,
    convert: bool,
) -> Result<Value, String> {
    let preset = preset_named(&preset)?;
    let preview = edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            scene.apply_player_preset(&actor_id, preset, convert, library)
        })
    })?;
    serde_json::to_value(preview).map_err(|e| e.to_string())
}

/// Where a project keeps its saved player profiles.
fn profile_dir(state: &SharedState) -> Result<std::path::PathBuf, String> {
    let s = lock(state)?;
    let dir = s.project_dir().ok_or("No project is open")?;
    Ok(dir.join("assets").join("profiles"))
}

/// A profile's file stem: letters, digits, spaces, dashes and underscores.
fn profile_stem(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
    {
        return Err(
            "A profile name uses letters, digits, spaces, dashes and underscores".to_string(),
        );
    }
    Ok(name.to_string())
}

fn profile_path(state: &SharedState, name: &str) -> Result<std::path::PathBuf, String> {
    Ok(profile_dir(state)?.join(format!("{}.profile.json", profile_stem(name)?)))
}

/// `save-player-profile`: writes an actor's controller, motor, camera and
/// input actions to `assets/profiles/<name>.profile.json`.
pub(crate) fn save_player_profile(
    state: &SharedState,
    actor_id: String,
    name: String,
) -> Result<String, String> {
    let path = profile_path(state, &name)?;
    let profile = {
        let s = lock(state)?;
        let project = s.project().ok_or("No project is open")?;
        PlayerProfile::capture(project.active_scene(), &actor_id, name.trim())?
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(format!(
        "assets/profiles/{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
    ))
}

/// `list-player-profiles`: the saved profiles, with the dimension each suits.
pub(crate) fn list_player_profiles(state: &SharedState) -> Result<Value, String> {
    let dir = profile_dir(state)?;
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".profile.json"))
            else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match serde_json::from_str::<PlayerProfile>(&text) {
                Ok(profile) => out.push(json!({
                    "name": stem,
                    "mode": profile.mode,
                    "version": profile.version,
                    "valid": true,
                })),
                Err(why) => {
                    out.push(json!({"name": stem, "valid": false, "error": why.to_string()}))
                }
            }
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(Value::Array(out))
}

/// `apply-player-profile`: installs a saved profile on an actor as one undo
/// step.
pub(crate) fn apply_player_profile(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    name: String,
    convert: bool,
) -> Result<Value, String> {
    let path = profile_path(state, &name)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("No saved profile called \"{}\"", name.trim()))?;
    let profile: PlayerProfile =
        serde_json::from_str(&text).map_err(|e| format!("The profile is unreadable: {e}"))?;
    let preview = edit(state, app, None, |project| {
        with_scene(project, |scene, library| {
            profile.apply(scene, &actor_id, convert, library)
        })
    })?;
    serde_json::to_value(preview).map_err(|e| e.to_string())
}

/// `import-player-profile`: copies a profile file from another project into
/// this one, checking it reads first.
pub(crate) fn import_player_profile(state: &SharedState, path: String) -> Result<String, String> {
    let source = std::path::PathBuf::from(&path);
    let text = std::fs::read_to_string(&source).map_err(|e| format!("Can't read {path}: {e}"))?;
    let profile: PlayerProfile =
        serde_json::from_str(&text).map_err(|e| format!("That is not a player profile: {e}"))?;
    let target = profile_path(state, &profile.name)?;
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if target.exists() {
        return Err(format!(
            "This project already has a profile called \"{}\"; rename one first",
            profile.name
        ));
    }
    std::fs::write(&target, text).map_err(|e| e.to_string())?;
    Ok(profile.name)
}

/// `physics-migration-preview`: what converting each legacy `Body` would
/// store, in every scene or for one actor. Nothing is changed.
pub(crate) fn physics_migration_preview(
    state: &SharedState,
    actor: Option<String>,
) -> Result<Value, String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    let mut scenes = Vec::new();
    let mut total = 0;
    for scene in &project.scenes {
        let mut preview = scene.clone();
        let mut library = project.physics.materials.clone();
        let migrations = match &actor {
            Some(id) => preview.migrate_actor_physics(id, &mut library),
            None => preview.migrate_physics(&mut library),
        };
        if migrations.is_empty() {
            continue;
        }
        total += migrations.len();
        scenes.push(json!({ "scene": scene.name, "actors": migrations }));
    }
    if let Some(id) = &actor
        && scenes.is_empty()
        && !project
            .scenes
            .iter()
            .any(|scene| scene.actors.iter().any(|a| &a.id == id))
    {
        return Err(format!("No actor \"{id}\""));
    }
    Ok(json!({ "applied": false, "total": total, "scenes": scenes }))
}

/// `migrate-physics`: converts legacy `Body` components to Rigidbody and Collider
/// in every scene, or on one actor. One undo step; the project file as it was
/// is copied to `.blockloom/backups` first. A second call converts nothing.
pub(crate) fn migrate_physics(
    state: &SharedState,
    app: &AppHandle,
    actor: Option<String>,
) -> Result<Value, String> {
    // Nothing to convert: no backup, no undo step.
    let preview = physics_migration_preview(state, actor.clone())?;
    if preview["total"] == 0 {
        return Ok(json!({ "applied": true, "total": 0, "scenes": [] }));
    }
    {
        let s = lock(state)?;
        if let Some(open) = &s.open {
            backup_project(&open.dir);
        }
    }
    edit(state, app, None, |project| {
        let mut library = project.physics.materials.clone();
        let mut scenes = Vec::new();
        let mut total = 0;
        for scene in &mut project.scenes {
            let migrations = match &actor {
                Some(id) => scene.migrate_actor_physics(id, &mut library),
                None => scene.migrate_physics(&mut library),
            };
            if migrations.is_empty() {
                continue;
            }
            total += migrations.len();
            scenes.push(json!({ "scene": scene.name, "actors": migrations }));
        }
        project.physics.materials = library;
        Ok(json!({ "applied": true, "total": total, "scenes": scenes }))
    })
}

/// Copies the project file aside before an upgrade rewrites it. Best effort: a
/// failure is logged and the undo history still holds the old project.
fn backup_project(dir: &std::path::Path) {
    let from = dir.join(blockloom_core::project::PROJECT_FILE);
    if !from.is_file() {
        return;
    }
    let backups = dir.join(".blockloom").join("backups");
    let name = format!(
        "project.before-physics-upgrade-{}.blockloom",
        blockloom_core::sync::now_secs()
    );
    if let Err(e) =
        std::fs::create_dir_all(&backups).and_then(|_| std::fs::copy(&from, backups.join(name)))
    {
        tracing::warn!("Couldn't back up the project before the physics upgrade: {e}");
    }
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
        ActorComponent::CharacterMotor { .. } => {
            Err("Use set-character-motor for a CharacterMotor".to_string())
        }
        ActorComponent::Constraint { .. } => Err(
            "Use add-constraint and set-constraint for constraints: they are addressed by id"
                .to_string(),
        ),
        _ => Ok(()),
    }
}
