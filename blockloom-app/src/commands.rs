//! Every command the editor can issue. Each one locks state, checkpoints undo
//! where an edit is undoable, changes the document, saves it, and publishes a
//! fresh snapshot - in that order, so the frontend never sees a half-applied
//! edit.

use crate::runtime::RuntimeHandle;
use crate::state::{
    AppState, EditSession, InstrPath, LogLine, OpenProject, SharedState, StateDto, ValueLocation,
    state_dto,
};
use crate::{AppHandle, Backend};
use blockloom_core::assets;
use blockloom_core::blocks::{
    ActorGraph, BlockPiece, BlockShape, Instruction, InstructionKind, normalize_block_color,
};
use blockloom_core::components::{ActorComponent, Components};
use blockloom_core::library;
use blockloom_core::project::{self, Actor, Project};
use blockloom_core::scene::{Camera, Mode, Physics, Placement, Visual};
use blockloom_core::script;
use blockloom_core::value::{Evaluated, Value};
use blockstitch_core::editor::{ValueEdit, prune_value_buffers};
use blockstitch_core::value::operator_kind;
use std::collections::HashMap;
use std::path::Path;
use std::sync::MutexGuard;

type Guard<'a> = MutexGuard<'a, AppState>;

fn lock(state: &SharedState) -> Result<Guard<'_>, String> {
    state.lock().map_err(|e| e.to_string())
}

/// Checkpoints the whole project for undo.
fn push_undo(s: &mut AppState) {
    push_undo_for(s, None);
}

/// [`push_undo`] for an edit that coalesces with the keystrokes already in
/// progress at `session` - only the first one checkpoints.
fn push_undo_for(s: &mut AppState, session: Option<EditSession>) {
    let snapshot = s.project().cloned();
    match (snapshot, session) {
        (Some(snapshot), Some(session)) => {
            s.history.push_for_session(snapshot, session);
        }
        (Some(snapshot), None) => s.history.push(snapshot),
        (None, _) => s.history.end_session(),
    }
}

/// Writes the open project to its folder. Failures are logged, not surfaced:
/// an unwritable folder shouldn't stop the editor working.
fn auto_save(s: &AppState) {
    if let Some(open) = &s.open
        && let Err(e) = project::save_project(&open.project, &open.dir)
    {
        tracing::warn!("Couldn't save the project: {e}");
    }
}

fn emit(app: &AppHandle, s: &AppState) {
    app.emit_state(&state_dto(s));
}

/// The open actor's canvas - what almost every canvas command edits.
fn graph_mut(s: &mut AppState) -> Option<&mut ActorGraph> {
    let id = s.actor_id()?;
    Some(&mut s.project_mut()?.actor_mut(&id)?.graph)
}

/// The variable values a reporter or a dropped value block is evaluated
/// against: the open actor's own, over the project's globals.
fn env(s: &AppState) -> HashMap<String, Evaluated> {
    match (s.project(), s.actor_id()) {
        (Some(project), Some(actor)) => project.env_for(&actor),
        _ => HashMap::new(),
    }
}

// ─── State ─────────────────────────────────────────────────────────────────

pub(crate) fn get_state(state: &SharedState) -> Result<StateDto, String> {
    let s = lock(state)?;
    Ok(state_dto(&s))
}

// ─── Projects ──────────────────────────────────────────────────────────────

/// Opens the project in `dir`, replacing whatever was open. The Dashboard's
/// cards and its "Open a folder" both land here.
pub(crate) fn open_project(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let dir = std::path::PathBuf::from(path);
    let project = project::read_project_dir(&dir)?;
    let mut s = lock(state)?;
    close_open_project(&mut s, true);
    library::remember(&dir);
    s.open = Some(OpenProject { project, dir });
    s.library = library::list();
    emit(app, &s);
    Ok(())
}

/// Makes a project folder under `location` - named after the project, since
/// the app owns the folder name - and opens it.
pub(crate) fn create_project(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    location: Option<String>,
    mode: Mode,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the project a name".to_string());
    }
    let parent = location
        .map(|location| std::path::PathBuf::from(location.trim()))
        .filter(|location| !location.as_os_str().is_empty())
        .unwrap_or_else(project::default_projects_dir);

    let project = Project::starter(name, mode);
    let dir = project::create_project(&project, &parent)?;

    let mut s = lock(state)?;
    close_open_project(&mut s, true);
    library::remember(&dir);
    s.open = Some(OpenProject { project, dir });
    s.library = library::list();
    emit(app, &s);
    Ok(())
}

/// Saves the open project and goes back to the Dashboard.
pub(crate) fn close_project(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    close_open_project(&mut s, true);
    s.library = library::list();
    emit(app, &s);
    Ok(())
}

/// Drops a project from the Dashboard without touching the folder, so it can
/// be opened again later from wherever it is.
pub(crate) fn forget_project(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let dir = std::path::PathBuf::from(path);
    let mut s = lock(state)?;
    if s.project_dir() == Some(dir.as_path()) {
        close_open_project(&mut s, true);
    }
    library::forget(&dir);
    s.library = library::list();
    emit(app, &s);
    Ok(())
}

/// Deletes a project folder outright. Refuses a folder that isn't one, so a
/// stale entry can't take something else with it.
pub(crate) fn delete_project(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let dir = std::path::PathBuf::from(path);
    let mut s = lock(state)?;
    // Discard rather than save: the folder is about to go.
    if s.project_dir() == Some(dir.as_path()) {
        close_open_project(&mut s, false);
    }
    project::delete_project_dir(&dir)?;
    library::forget(&dir);
    s.library = library::list();
    emit(app, &s);
    Ok(())
}

/// Renames the open project, and its folder with it.
pub(crate) fn set_project_name(
    state: &SharedState,
    app: &AppHandle,
    name: String,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the project a name".to_string());
    }
    let mut s = lock(state)?;
    let Some(open) = &mut s.open else {
        return Ok(());
    };
    open.project.name = name.clone();
    let from = open.dir.clone();
    // Save first, so the rename moves a folder that already says the new name.
    if let Err(e) = project::save_project(&open.project, &from) {
        tracing::warn!("Couldn't save the project: {e}");
    }
    match project::rename_project_dir(&from, &name) {
        Ok(to) => {
            if to != from {
                library::moved(&from, &to);
                open.dir = to;
            }
        }
        // A folder that won't move isn't worth refusing the rename over - the
        // project is saved either way.
        Err(e) => tracing::warn!("Couldn't rename the project folder: {e}"),
    }
    s.library = library::list();
    emit(app, &s);
    Ok(())
}

pub(crate) fn save_open_project(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let s = lock(state)?;
    if let Some(open) = &s.open {
        project::save_project(&open.project, &open.dir)?;
    }
    emit(app, &s);
    Ok(())
}

/// Suggested file name for the export dialog.
pub(crate) fn export_file_name(state: &SharedState) -> Result<String, String> {
    let s = lock(state)?;
    let name = s.project().map(|p| p.name.clone()).unwrap_or_default();
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    Ok(format!("{safe}.{}", project::PROJECT_EXTENSION))
}

pub(crate) fn export_project(state: &SharedState, path: String) -> Result<(), String> {
    let s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    project::export_project(project, std::path::Path::new(&path))
}

/// Reads an exported `.blockloom` file into a folder of its own under the
/// default location, and opens it.
pub(crate) fn import_project(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let mut project = project::read_project(std::path::Path::new(&path))?;
    // A fresh id, so importing the same file twice gives two projects rather
    // than two folders claiming to be one.
    project.id = uuid::Uuid::new_v4().simple().to_string();
    let dir = project::create_project(&project, &project::default_projects_dir())?;

    let mut s = lock(state)?;
    close_open_project(&mut s, true);
    library::remember(&dir);
    s.open = Some(OpenProject { project, dir });
    s.library = library::list();
    emit(app, &s);
    Ok(())
}

/// Lets go of whatever is open, leaving the editor on the Dashboard. The game
/// window and the run log belong to the project, so they go too. `save` is
/// false only when the project is on its way to being deleted.
fn close_open_project(s: &mut AppState, save: bool) {
    if save {
        auto_save(s);
    }
    s.open = None;
    s.selected_actor = None;
    s.history.clear();
    s.invalid_field_buffers.clear();
    s.runtime = None;
    s.running = false;
    s.paused = false;
    s.status = None;
    s.log.clear();
}

// ─── The world ─────────────────────────────────────────────────────────────

/// Switches a project between 2D and 3D. Gravity follows the mode unless the
/// project set its own, and a running world is restarted, since the two
/// dimensions are different processes.
pub(crate) fn set_mode(
    backend: &Backend,
    state: &SharedState,
    app: &AppHandle,
    mode: Mode,
) -> Result<(), String> {
    let mut s = lock(state)?;
    if s.project().is_none_or(|project| project.world.mode == mode) {
        return Ok(());
    }
    push_undo(&mut s);
    let runtime_was_open = s.runtime.is_some();
    let was_running = s.running;
    let was_paused = s.paused;
    let Some(project) = s.project_mut() else {
        return Ok(());
    };
    project.switch_mode(mode);
    auto_save(&s);

    // A dimension uses a different Bevy plugin set, so replace the process
    // and restore whether it was idle, running, or paused.
    s.runtime = None;
    s.status = None;
    let mut restart_error = None;
    if runtime_was_open {
        let project = s.project().cloned().expect("checked above");
        let dir = s
            .project_dir()
            .map(|dir| dir.to_string_lossy().into_owned());
        match RuntimeHandle::spawn(mode, backend.clone()) {
            Ok(mut runtime) => {
                let loaded = runtime.send(&blockloom_protocol::EditorMessage::Load {
                    project: Box::new(project),
                    dir: dir.clone(),
                });
                let started = !was_running
                    || (runtime.send(&blockloom_protocol::EditorMessage::Start)
                        && (!was_paused
                            || runtime
                                .send(&blockloom_protocol::EditorMessage::Pause { paused: true })));
                if loaded && started {
                    s.runtime = Some(runtime);
                } else {
                    restart_error = Some("Lost the connection to the game runtime".to_string());
                }
            }
            Err(error) => restart_error = Some(error),
        }
    }
    s.running = runtime_was_open && was_running && s.runtime.is_some();
    s.paused = s.running && was_paused;
    emit(app, &s);
    restart_error.map_or(Ok(()), Err)
}

pub(crate) fn set_background(
    state: &SharedState,
    app: &AppHandle,
    color: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let color = normalize_block_color(&color).ok_or("Choose a valid color")?;
    if let Some(project) = s.project_mut() {
        project.world.background = color;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_gravity(
    state: &SharedState,
    app: &AppHandle,
    gravity: [f32; 3],
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.world.gravity = gravity;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_camera(
    state: &SharedState,
    app: &AppHandle,
    camera: Camera,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.world.camera = camera;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

// ─── Actors ────────────────────────────────────────────────────────────────

pub(crate) fn select_actor(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    s.selected_actor = Some(actor_id);
    s.history.end_session();
    emit(app, &s);
    Ok(())
}

/// Adds an actor with a default look for `shape` - see [`default_visual`].
pub(crate) fn add_actor(
    state: &SharedState,
    app: &AppHandle,
    shape: String,
    name: Option<String>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let Some(project) = s.project_mut() else {
        return Err("No project is open".to_string());
    };
    let visual = default_visual(&shape).ok_or_else(|| format!("Unknown shape \"{shape}\""))?;
    let name = name.unwrap_or_else(|| shape_label(&shape).to_string());
    let id = project.add_actor(Actor::new(name, visual));
    s.selected_actor = Some(id.clone());
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn duplicate_actor(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let Some(project) = s.project_mut() else {
        return Err("No project is open".to_string());
    };
    let mut copy = project.actor(&actor_id).cloned().ok_or("Actor not found")?;
    copy.id = uuid::Uuid::new_v4().simple().to_string();
    // Fresh ids for everything on the canvas, so the copy's blocks are its own.
    for strand in &mut copy.graph.strands {
        strand.id = uuid::Uuid::new_v4().simple().to_string();
        for instruction in &mut strand.instructions {
            instruction.walk_mut(&mut |ins| {
                ins.id = uuid::Uuid::new_v4().simple().to_string();
            });
        }
    }
    copy.graph.comments.clear();
    // There is one camera, so the copy doesn't get to keep it.
    copy.components.remove("Camera");
    let id = project.add_actor(copy);
    s.selected_actor = Some(id.clone());
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn remove_actor(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.remove_actor(&actor_id);
    }
    if s.selected_actor.as_deref() == Some(actor_id.as_str()) {
        s.selected_actor = None;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn rename_actor(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo_for(
        &mut s,
        Some(EditSession::Comment {
            comment_id: format!("actor-name:{actor_id}"),
        }),
    );
    let result = match s.project_mut() {
        Some(project) => project.rename_actor(&actor_id, &name).map(|_| ()),
        None => Ok(()),
    };
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    result
}

pub(crate) fn set_actor_visual(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    visual: Visual,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.set_visual(visual);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_actor_placement(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    placement: Placement,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo_for(
        &mut s,
        Some(EditSession::Comment {
            comment_id: format!("actor-placement:{actor_id}"),
        }),
    );
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.set_placement(placement);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_actor_physics(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    physics: Physics,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.set_physics(physics);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_actor_visible(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    visible: bool,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.set_visible(visible);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

// ─── Components ────────────────────────────────────────────────────────────

/// Adds a component to an actor, or replaces the one of the same name. A
/// custom component is given a name nothing else on the actor is using.
pub(crate) fn add_actor_component(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    mut component: ActorComponent,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) else {
        return Err("Actor not found".to_string());
    };
    if let ActorComponent::Custom { name, .. } = &mut component {
        *name = actor.components.unique_custom_name(name);
    }
    let name = component.name().to_string();
    actor.components.insert(component);
    if name == "Camera" {
        s.project_mut()
            .expect("checked above")
            .claim_camera(&actor_id);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(name)
}

/// Replaces a component in place, keyed by the name it already has - what
/// every field in the inspector writes through.
pub(crate) fn set_actor_component(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    name: String,
    component: ActorComponent,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo_for(
        &mut s,
        Some(EditSession::Comment {
            comment_id: format!("actor-component:{actor_id}:{name}"),
        }),
    );
    let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) else {
        return Err("Actor not found".to_string());
    };
    let result = rename_or_replace(&mut actor.components, &name, component);
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    result
}

/// Writes `component` over the one called `name`. A custom component that
/// came back under a different name has been renamed, so it takes the old
/// one's place in the list rather than being appended.
fn rename_or_replace(
    components: &mut Components,
    name: &str,
    component: ActorComponent,
) -> Result<(), String> {
    if component.name() == name {
        components.insert(component);
        return Ok(());
    }
    if components.contains(component.name()) {
        return Err(format!(
            "This actor already has a \"{}\" component",
            component.name()
        ));
    }
    let Some(index) = components.0.iter().position(|slot| slot.name() == name) else {
        return Err(format!("No \"{name}\" component to change"));
    };
    components.0[index] = component;
    Ok(())
}

pub(crate) fn remove_actor_component(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.remove(&name);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// The look a freshly added actor gets. 2D sizes are in pixels and 3D ones in
/// metres, which is why the numbers are so different.
fn default_visual(shape: &str) -> Option<Visual> {
    Some(match shape {
        "Rect" => Visual::Rect {
            color: "#4C97FF".to_string(),
            size: [60.0, 60.0],
        },
        "Circle" => Visual::Circle {
            color: "#FFAB19".to_string(),
            radius: 30.0,
        },
        "Image" => Visual::Image {
            path: String::new(),
            size: [80.0, 80.0],
        },
        "Cuboid" => Visual::Cuboid {
            color: "#4C97FF".to_string(),
            size: [1.0, 1.0, 1.0],
        },
        "Sphere" => Visual::Sphere {
            color: "#FFAB19".to_string(),
            radius: 0.5,
        },
        "Capsule" => Visual::Capsule {
            color: "#40BF4A".to_string(),
            radius: 0.4,
            height: 1.0,
        },
        "Plane" => Visual::Plane {
            color: "#3E4A5B".to_string(),
            size: [20.0, 20.0],
        },
        _ => return None,
    })
}

fn shape_label(shape: &str) -> &str {
    match shape {
        "Rect" => "Square",
        "Circle" => "Ball",
        "Image" => "Sprite",
        "Cuboid" => "Box",
        "Sphere" => "Ball",
        "Capsule" => "Body",
        "Plane" => "Ground",
        other => other,
    }
}

// ─── Running ───────────────────────────────────────────────────────────────

/// Play: brings up a runtime for this project's dimension if there isn't one,
/// hands it the project, and starts it.
pub(crate) fn run_project(
    backend: &Backend,
    state: &SharedState,
    app: &AppHandle,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let Some(project) = s.project().cloned() else {
        return Err("No project is open".to_string());
    };
    auto_save(&s);
    // Built before the world is handed over, so a script that won't compile
    // shows its errors in the log instead of silently doing nothing.
    build_scripts(&mut s);
    let dir = s
        .project_dir()
        .map(|dir| dir.to_string_lossy().into_owned());

    let wrong_dimension = s
        .runtime
        .as_ref()
        .is_some_and(|runtime| runtime.mode != project.world.mode);
    if wrong_dimension {
        s.runtime = None;
    }
    if s.runtime.is_none() {
        s.runtime = Some(RuntimeHandle::spawn(project.world.mode, backend.clone())?);
    }

    let Some(runtime) = s.runtime.as_mut() else {
        return Err("The game runtime isn't running".to_string());
    };
    let alive = runtime.send(&blockloom_protocol::EditorMessage::Load {
        project: Box::new(project),
        dir,
    }) && runtime.send(&blockloom_protocol::EditorMessage::Start);
    if !alive {
        s.runtime = None;
        return Err("Lost the connection to the game runtime".to_string());
    }
    s.log.clear();
    s.running = true;
    s.paused = false;
    emit(app, &s);
    Ok(())
}

pub(crate) fn stop_project(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::Stop)
    {
        s.runtime = None;
    }
    s.running = false;
    s.paused = false;
    emit(app, &s);
    Ok(())
}

pub(crate) fn pause_project(
    state: &SharedState,
    app: &AppHandle,
    paused: bool,
) -> Result<(), String> {
    let mut s = lock(state)?;
    if let Some(runtime) = s.runtime.as_mut() {
        if !runtime.send(&blockloom_protocol::EditorMessage::Pause { paused }) {
            s.runtime = None;
            s.running = false;
            s.paused = false;
            s.status = None;
            emit(app, &s);
            return Err("Lost the connection to the game runtime".to_string());
        }
    }
    s.paused = paused && s.running;
    emit(app, &s);
    Ok(())
}

/// Closes the game window without touching the project.
pub(crate) fn close_runtime(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    s.runtime = None;
    s.running = false;
    s.paused = false;
    s.status = None;
    emit(app, &s);
    Ok(())
}

/// Compiles every actor's script, logging whatever rustc has to say about
/// the ones that fail. A failed script just doesn't load: the rest of the
/// project still plays.
fn build_scripts(s: &mut AppState) {
    let Some(dir) = s.project_dir().map(Path::to_path_buf) else {
        return;
    };
    let scripts: Vec<(String, String)> = s
        .project()
        .into_iter()
        .flat_map(|project| project.actors.iter())
        .filter_map(|actor| {
            actor
                .components
                .script()
                .map(|path| (actor.name.clone(), path.to_string()))
        })
        .collect();
    for (actor, path) in scripts {
        if let Err(error) = script::compile(&dir, &path) {
            s.push_log(LogLine {
                kind: "error".to_string(),
                actor,
                text: format!("{path} didn't compile:\n{error}"),
            });
        }
    }
}

// ─── Assets ────────────────────────────────────────────────────────────────
//
// A project is a folder, so its assets are files in it and the asset tray is a
// small file manager over that folder. None of this touches the document, so
// none of it checkpoints undo; the tray asks for a fresh listing after every
// change instead of waiting for a state snapshot.

/// What a new file of each kind starts out holding. A script gets the same
/// starter template the Script component makes, so one dragged onto an actor
/// compiles as it is.
fn asset_template(name: &str) -> String {
    match assets::kind_of(name) {
        assets::AssetKind::Script => script::starter("this actor"),
        _ => String::new(),
    }
}

fn project_dir(s: &Guard<'_>) -> Result<std::path::PathBuf, String> {
    s.project_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "No project is open".to_string())
}

/// What one folder of the project holds.
pub(crate) fn list_assets(
    state: &SharedState,
    path: String,
) -> Result<Vec<assets::AssetEntry>, String> {
    let s = lock(state)?;
    assets::list(&project_dir(&s)?, &path)
}

pub(crate) fn create_asset_folder(
    state: &SharedState,
    parent: String,
    name: String,
) -> Result<String, String> {
    let s = lock(state)?;
    assets::create_folder(&project_dir(&s)?, &parent, &name)
}

/// Makes an empty asset - a text file, or a script with the starter template.
pub(crate) fn create_asset(
    state: &SharedState,
    parent: String,
    name: String,
) -> Result<String, String> {
    let s = lock(state)?;
    assets::create_file(&project_dir(&s)?, &parent, &name, &asset_template(&name))
}

/// Copies files from anywhere on the machine into the project folder.
pub(crate) fn import_assets(
    state: &SharedState,
    parent: String,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let s = lock(state)?;
    let sources: Vec<std::path::PathBuf> =
        paths.into_iter().map(std::path::PathBuf::from).collect();
    assets::import(&project_dir(&s)?, &parent, &sources)
}

pub(crate) fn rename_asset(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    name: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let moved = assets::rename(&project_dir(&s)?, &path, &name)?;
    repoint_assets(&mut s, app, &path, &moved);
    Ok(moved)
}

pub(crate) fn move_asset(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    parent: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let moved = assets::move_to(&project_dir(&s)?, &path, &parent)?;
    repoint_assets(&mut s, app, &path, &moved);
    Ok(moved)
}

pub(crate) fn delete_asset(state: &SharedState, path: String) -> Result<(), String> {
    let s = lock(state)?;
    assets::delete(&project_dir(&s)?, &path)
}

/// A file's bytes as a `data:` URL - the only way a web page can show a
/// thumbnail of a file on disk.
pub(crate) fn read_asset(state: &SharedState, path: String) -> Result<String, String> {
    let s = lock(state)?;
    assets::data_url(&project_dir(&s)?, &path)
}

/// Follows a renamed or moved asset through the document, so an actor whose
/// image or script just moved still points at it. Nothing to do in the usual
/// case, and then nothing is saved or published either.
fn repoint_assets(s: &mut AppState, app: &AppHandle, from: &str, to: &str) {
    if from == to {
        return;
    }
    let changed = s
        .project_mut()
        .is_some_and(|project| project.repoint_asset(from, to));
    if !changed {
        return;
    }
    auto_save(s);
    sync_runtime(s);
    emit(app, s);
}

// ─── Scripts ───────────────────────────────────────────────────────────────

/// Gives an actor a script: makes the file from the starter template if it
/// isn't there, and attaches the component that names it.
pub(crate) fn create_script(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or("No project is open")?;
    push_undo(&mut s);
    let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) else {
        return Err("Actor not found".to_string());
    };
    // An actor that already names a script keeps it, so this is also the
    // "make the file I deleted" button.
    let path = match actor.components.script() {
        Some(path) => path.to_string(),
        None => script::unused_path(&dir, &actor.name),
    };
    let name = actor.name.clone();
    script::create(&dir, &path, &name)?;
    actor
        .components
        .insert(ActorComponent::Script { path: path.clone() });
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(path)
}

/// Compiles one actor's script without playing, so the editor can show what
/// rustc thinks of it. The message is the success line or the errors.
pub(crate) fn check_script(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or("No project is open")?;
    let Some(actor) = s.project().and_then(|p| p.actor(&actor_id)) else {
        return Err("Actor not found".to_string());
    };
    let name = actor.name.clone();
    let path = actor
        .components
        .script()
        .ok_or("This actor has no script")?
        .to_string();
    let line = match script::compile(&dir, &path) {
        Ok(_) => LogLine {
            kind: "say".to_string(),
            actor: name,
            text: format!("{path} compiled"),
        },
        Err(error) => LogLine {
            kind: "error".to_string(),
            actor: name,
            text: format!("{path} didn't compile:\n{error}"),
        },
    };
    s.push_log(line);
    emit(app, &s);
    Ok(())
}

/// The script's source, for the editor to show. Missing is empty, not an
/// error: a project can name a file somebody deleted.
pub(crate) fn read_script(state: &SharedState, actor_id: String) -> Result<String, String> {
    let s = lock(state)?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or("No project is open")?;
    let Some(path) = s
        .project()
        .and_then(|p| p.actor(&actor_id))
        .and_then(|actor| actor.components.script())
    else {
        return Ok(String::new());
    };
    Ok(std::fs::read_to_string(script::source_path(&dir, path)).unwrap_or_default())
}

/// Writes a script's source back. The file is the document here - it isn't
/// part of the project JSON - so this doesn't touch undo.
pub(crate) fn write_script(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    source: String,
) -> Result<(), String> {
    let s = lock(state)?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or("No project is open")?;
    let path = s
        .project()
        .and_then(|p| p.actor(&actor_id))
        .and_then(|actor| actor.components.script())
        .ok_or("This actor has no script")?
        .to_string();
    if !script::is_valid_path(&path) {
        return Err(format!("\"{path}\" isn't a script path"));
    }
    let file = script::source_path(&dir, &path);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&file, source).map_err(|e| format!("{}: {e}", file.display()))?;
    emit(app, &s);
    Ok(())
}

/// Pushes the edited project to an idle runtime, so a scene edit shows in the
/// game window straight away. While a run is going the edit waits for the next
/// Play - reloading mid-run would throw the world away under the user.
fn sync_runtime(s: &mut AppState) {
    let Some(project) = s.project().cloned() else {
        return;
    };
    if s.running {
        return;
    }
    let dir = s
        .project_dir()
        .map(|dir| dir.to_string_lossy().into_owned());
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::Load {
            project: Box::new(project),
            dir,
        })
    {
        s.runtime = None;
    }
}

// ─── Canvas: instructions ──────────────────────────────────────────────────

pub(crate) fn add_instruction(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
    path: InstrPath,
    instruction: Instruction,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match graph_mut(&mut s) {
        Some(graph) => graph
            .insert_instruction(&strand_id, &path, instruction)
            .map(|_| ()),
        None => Ok(()),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn edit_instruction(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
    path: InstrPath,
    instruction: Instruction,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo_for(
        &mut s,
        Some(EditSession::Instruction {
            strand_id: strand_id.clone(),
            index: path.clone(),
        }),
    );
    if let Some(graph) = graph_mut(&mut s) {
        graph.replace_instruction(&strand_id, &path, instruction);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn remove_instruction(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
    path: InstrPath,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.remove_instruction(&strand_id, &path);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

/// Deletes one block, splitting whatever was below it into its own strand at
/// `(x, y)` - returns that strand's id, if one was made.
pub(crate) fn delete_instruction(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
    path: InstrPath,
    x: i32,
    y: i32,
) -> Result<Option<String>, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match graph_mut(&mut s) {
        Some(graph) => graph.delete_instruction(&strand_id, &path, x, y),
        None => Ok(None),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn paste_instructions(
    state: &SharedState,
    app: &AppHandle,
    x: i32,
    y: i32,
    instructions: Vec<Instruction>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let id = match graph_mut(&mut s) {
        Some(graph) => graph.add_strand(x, y, instructions),
        None => return Err("No actor is open".to_string()),
    };
    auto_save(&s);
    emit(app, &s);
    Ok(id)
}

// ─── Canvas: strands ───────────────────────────────────────────────────────

pub(crate) fn add_strand(
    state: &SharedState,
    app: &AppHandle,
    x: Option<i32>,
    y: Option<i32>,
    instruction: Option<Instruction>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let id = match graph_mut(&mut s) {
        Some(graph) => {
            let (default_x, default_y) = graph.next_strand_position();
            let instructions = instruction.map(|i| vec![i]).unwrap_or_default();
            graph.add_strand(x.unwrap_or(default_x), y.unwrap_or(default_y), instructions)
        }
        None => return Err("No actor is open".to_string()),
    };
    auto_save(&s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn remove_strand(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.remove_strand(&strand_id);
    }
    blockstitch_core::editor::drop_strand_buffers(&mut s.invalid_field_buffers, &strand_id);
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn move_strand(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
    x: i32,
    y: i32,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.move_strand(&strand_id, x, y);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn split_strand(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
    path: InstrPath,
    x: i32,
    y: i32,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match graph_mut(&mut s) {
        Some(graph) => graph.split_strand(&strand_id, &path, x, y),
        None => Err("No actor is open".to_string()),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn merge_strand(
    state: &SharedState,
    app: &AppHandle,
    dragged_id: String,
    target_id: String,
    path: InstrPath,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match graph_mut(&mut s) {
        Some(graph) => graph.merge_strand(&dragged_id, &target_id, &path),
        None => Ok(()),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

// ─── Canvas: values ────────────────────────────────────────────────────────

pub(crate) fn edit_value_field(
    state: &SharedState,
    app: &AppHandle,
    location: ValueLocation,
    text: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    // Coalesce a run of keystrokes into this field into a single undo step.
    push_undo_for(&mut s, Some(EditSession::Value(location.clone())));
    if let Some(graph) = graph_mut(&mut s) {
        match graph.edit_value_text(&location, text.clone()) {
            // Text slots are always valid - no invalid-buffer bookkeeping.
            ValueEdit::Text => auto_save(&s),
            // A numeric field keeps the raw text either way, so a half-written
            // number doesn't snap back mid-edit.
            ValueEdit::Number { parsed } => {
                s.invalid_field_buffers.insert(location, text);
                if parsed {
                    auto_save(&s);
                }
            }
            ValueEdit::Missing => {}
        }
    }
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_value_kind(
    state: &SharedState,
    app: &AppHandle,
    location: ValueLocation,
    kind: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let env = env(&s);
    let result = match graph_mut(&mut s) {
        Some(graph) => graph.set_value_kind(&location, &kind, &env),
        None => Ok(()),
    };
    prune_value_buffers(&mut s.invalid_field_buffers, &location);
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn take_value(
    state: &SharedState,
    app: &AppHandle,
    location: ValueLocation,
) -> Result<Value, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let taken = graph_mut(&mut s).and_then(|graph| graph.take_value(&location));
    prune_value_buffers(&mut s.invalid_field_buffers, &location);
    auto_save(&s);
    emit(app, &s);
    taken.ok_or_else(|| "Nothing to take at that location".to_string())
}

pub(crate) fn put_value(
    state: &SharedState,
    app: &AppHandle,
    location: ValueLocation,
    value: Value,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.put_value(&location, value);
    }
    prune_value_buffers(&mut s.invalid_field_buffers, &location);
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

/// What a value block would report right now, for the canvas's hover preview.
/// Anything that can't be worked out without a running world (a parameter, a
/// reporter block, "my x position") previews as empty rather than an error.
pub(crate) fn preview_value(state: &SharedState, value: Value) -> Result<String, String> {
    let s = lock(state)?;
    let env = env(&s);
    Ok(value
        .resolve_vars(&env)
        .eval()
        .map(|evaluated| evaluated.as_text())
        .unwrap_or_default())
}

pub(crate) fn create_floating_value(
    state: &SharedState,
    app: &AppHandle,
    x: i32,
    y: i32,
    value: Value,
    origin_block_id: Option<String>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let id = match graph_mut(&mut s) {
        Some(graph) => graph.add_floating_value(x, y, value, origin_block_id),
        None => return Err("No actor is open".to_string()),
    };
    auto_save(&s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn move_floating_value(
    state: &SharedState,
    app: &AppHandle,
    floating_id: String,
    x: i32,
    y: i32,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.move_floating_value(&floating_id, x, y);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn remove_floating_value(
    state: &SharedState,
    app: &AppHandle,
    floating_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.remove_floating_value(&floating_id);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

// ─── Canvas: comments ──────────────────────────────────────────────────────

pub(crate) fn create_comment(
    state: &SharedState,
    app: &AppHandle,
    x: i32,
    y: i32,
    text: String,
    attached_to: Option<String>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let id = match graph_mut(&mut s) {
        Some(graph) => graph.add_comment(x, y, text, attached_to),
        None => return Err("No actor is open".to_string()),
    };
    auto_save(&s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn move_comment(
    state: &SharedState,
    app: &AppHandle,
    comment_id: String,
    x: i32,
    y: i32,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.move_comment(&comment_id, x, y);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn edit_comment_text(
    state: &SharedState,
    app: &AppHandle,
    comment_id: String,
    text: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo_for(
        &mut s,
        Some(EditSession::Comment {
            comment_id: comment_id.clone(),
        }),
    );
    if let Some(graph) = graph_mut(&mut s) {
        graph.set_comment_text(&comment_id, text);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_comment_collapsed(
    state: &SharedState,
    app: &AppHandle,
    comment_id: String,
    collapsed: bool,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.set_comment_collapsed(&comment_id, collapsed);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn remove_comment(
    state: &SharedState,
    app: &AppHandle,
    comment_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.remove_comment(&comment_id);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

// ─── Variables ─────────────────────────────────────────────────────────────

/// `scope` is `"global"` for a project-wide variable, anything else for one
/// private to the open actor.
pub(crate) fn create_variable(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    scope: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let global = scope == "global";
    let result = if global {
        match s.project_mut() {
            Some(project) => project.create_global(&name).map(|_| ()),
            None => Ok(()),
        }
    } else {
        match graph_mut(&mut s) {
            Some(graph) => graph.create_variable(&name).map(|_| ()),
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn rename_variable(
    state: &SharedState,
    app: &AppHandle,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.variables.iter().any(|v| v.name == old_name));
    let result = if owned_by_actor {
        match graph_mut(&mut s) {
            Some(graph) => graph.rename_variable(&old_name, &new_name).map(|_| ()),
            None => Ok(()),
        }
    } else {
        match s.project_mut() {
            Some(project) => project.rename_global(&old_name, &new_name).map(|_| ()),
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn delete_variable(
    state: &SharedState,
    app: &AppHandle,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.variables.iter().any(|v| v.name == name));
    if owned_by_actor {
        if let Some(graph) = graph_mut(&mut s) {
            graph.remove_variable(&name);
        }
    } else if let Some(project) = s.project_mut() {
        project.remove_global(&name);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

// ─── Custom blocks ─────────────────────────────────────────────────────────

pub(crate) fn create_block(
    state: &SharedState,
    app: &AppHandle,
    pieces: Vec<BlockPiece>,
    shape: BlockShape,
    color: String,
) -> Result<String, String> {
    blockloom_core::blocks::BlockDef::validate_pieces(&pieces)?;
    let color = normalize_block_color(&color).ok_or("Choose a valid block color")?;
    let mut s = lock(state)?;
    push_undo(&mut s);
    let id = match graph_mut(&mut s) {
        Some(graph) => {
            let (x, y) = graph.next_strand_position();
            graph.create_block(pieces, shape, color, x, y, |block_id| {
                InstructionKind::BlockHeader {
                    block_id: block_id.to_string(),
                }
            })
        }
        None => return Err("No actor is open".to_string()),
    };
    auto_save(&s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn edit_block(
    state: &SharedState,
    app: &AppHandle,
    block_id: String,
    pieces: Vec<BlockPiece>,
    shape: BlockShape,
    color: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match graph_mut(&mut s) {
        Some(graph) => graph.update_block(&block_id, pieces, shape, &color),
        None => Ok(()),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn delete_block(
    state: &SharedState,
    app: &AppHandle,
    block_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(graph) = graph_mut(&mut s) {
        graph.remove_block(&block_id);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

// ─── Undo/redo ─────────────────────────────────────────────────────────────

pub(crate) fn undo(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    step_history(state, app, crate::state::History::undo)
}

pub(crate) fn redo(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    step_history(state, app, crate::state::History::redo)
}

fn step_history(
    state: &SharedState,
    app: &AppHandle,
    step: fn(&mut crate::state::History<Project>, Project) -> Option<Project>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let Some(current) = s.project().cloned() else {
        return Ok(());
    };
    if let Some(previous) = step(&mut s.history, current) {
        let open = s.open.as_mut().ok_or("No project is open")?;
        open.project = previous;
        s.invalid_field_buffers.clear();
        auto_save(&s);
        sync_runtime(&mut s);
    }
    emit(app, &s);
    Ok(())
}

/// The palette entry a value kind names, so the frontend can ask whether a
/// kind exists before dropping it. Used by the sidebar's own validation.
pub(crate) fn value_kind_exists(kind: String) -> Result<bool, String> {
    Ok(operator_kind(&kind).is_some())
}

/// A log line the frontend wants to add itself (an import failure, say).
pub(crate) fn push_log(
    state: &SharedState,
    app: &AppHandle,
    kind: String,
    text: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    s.push_log(LogLine {
        kind,
        actor: "Blockloom".to_string(),
        text,
    });
    emit(app, &s);
    Ok(())
}

pub(crate) fn clear_log(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    s.log.clear();
    emit(app, &s);
    Ok(())
}
