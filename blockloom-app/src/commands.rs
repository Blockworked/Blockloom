//! Every command the editor can issue. Each one locks state, checkpoints undo
//! where an edit is undoable, changes the document, saves it, and publishes a
//! fresh snapshot - in that order, so the frontend never sees a half-applied
//! edit.

use crate::runtime::RuntimeHandle;
use crate::state::{
    AppState, EditSession, InstrPath, LogLine, OpenProject, SharedState, StateDto, SyncDto,
    ValueLocation, state_dto, sync_dto,
};
use crate::{AppHandle, Backend};
use blockloom_core::android;
use blockloom_core::assets;
use blockloom_core::blocks::{
    ActorGraph, BlockPiece, BlockShape, DictEntry, Instruction, InstructionKind, ListItem,
    normalize_block_color, resolve_dict_reporters, resolve_list_reporters,
};
use blockloom_core::build;
use blockloom_core::codegen;
use blockloom_core::components::{ActorComponent, Components};
use blockloom_core::fog::Fog;
use blockloom_core::library;
use blockloom_core::lightning::Lightning;
use blockloom_core::material::GraphEffect;
use blockloom_core::nav::NavSettings;
use blockloom_core::pipeline;
use blockloom_core::project::{self, Actor, Project};
use blockloom_core::scene::{
    Camera, DisplayOutput, Lighting, Mode, Physics, Placement, PostProcess, Visual,
};
use blockloom_core::scene_components::{SceneComponent, SceneComponents};
use blockloom_core::script;
use blockloom_core::sky::{Sky, SkyKind};
use blockloom_core::sound::SoundMixer;
use blockloom_core::sync;
use blockloom_core::sync::LockInfo;
use blockloom_core::value::{Evaluated, Value};
use blockloom_core::wind::Wind;
use blockstitch_core::editor::{ValueEdit, prune_value_buffers};
use blockstitch_core::value::operator_kind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;
use std::sync::atomic::Ordering;

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

/// Writes the open project to its folder, tracking the new revision. Failures
/// are logged, not surfaced: an unwritable folder shouldn't stop the editor
/// working. A failed save marks the copy dirty, which is what makes the next
/// reload a conflict with a choice instead of a quiet overwrite.
fn auto_save(s: &AppState) {
    if let Some(open) = &s.open {
        match project::save_project(&open.project, &open.dir) {
            Ok(revision) => {
                open.revision.store(revision, Ordering::SeqCst);
                open.dirty.store(false, Ordering::SeqCst);
                open.touched.store(sync::now_secs(), Ordering::SeqCst);
                sync::refresh_heartbeat(&open.dir, &s.session_id, "");
            }
            Err(e) => {
                tracing::warn!("Couldn't save the project: {e}");
                open.dirty.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// Touches our owner lock's heartbeat, throttled so chatty commands (a key
/// press, a forwarded mouse move) don't rewrite the file each time. Runs on
/// every dispatch, so an idle-but-alive backend keeps looking alive.
pub(crate) fn note_activity(state: &SharedState) {
    let Ok(s) = state.lock() else {
        return;
    };
    let Some(open) = &s.open else {
        return;
    };
    if !open.owns_lock {
        return;
    }
    let now = sync::now_secs();
    if now.saturating_sub(open.touched.load(Ordering::SeqCst)) < sync::HEARTBEAT_TOUCH_EVERY_SECS {
        return;
    }
    if sync::refresh_heartbeat(&open.dir, &s.session_id, "") {
        open.touched.store(now, Ordering::SeqCst);
    }
}

/// Reloads the open project straight off disk when another side saved since
/// this backend last wrote or loaded. Every edit already hits disk on
/// landing, so an idle backend is never dirty and just follows along; a
/// dirty one keeps its unsaved work and reports the conflict instead. The
/// reload checkpoints first, so it stays undoable, and the undo history
/// itself is never merged - each side keeps its own.
pub(crate) fn poll_live_reload(backend: &Backend) {
    let mut s = match backend.state.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    let (dir, loaded, dirty) = match &s.open {
        Some(open) => (
            open.dir.clone(),
            open.loaded_revision(),
            open.dirty.load(Ordering::SeqCst),
        ),
        None => return,
    };
    if dirty {
        return;
    }
    let disk = sync::read_revision(&dir);
    if disk <= loaded {
        return;
    }
    let project = match project::read_project_dir(&dir) {
        Ok(project) => project,
        Err(e) => {
            tracing::warn!("Couldn't reload the project: {e}");
            return;
        }
    };
    push_undo(&mut s);
    if let Some(open) = s.open.as_mut() {
        open.project = project;
        open.revision.store(disk, Ordering::SeqCst);
    }
    sync_runtime(&mut s);
    emit(&backend.app, &s);
}

/// Regenerates the analysis project rust-analyzer opens: the root `Cargo.toml`
/// plus the `blockloom` crate under `.blockloom/ide/`. Analysis-only, so a
/// failure is a warning rather than a refused edit.
fn sync_ide(dir: &Path) {
    if let Err(e) = script::ide::sync_ide_project(dir) {
        tracing::warn!("Couldn't sync the script IDE project: {e}");
    }
}

/// Whether `path` is a script file or lives under the scripts folder, which is
/// when an asset change means the analysis project needs regenerating.
fn touches_scripts(path: &str) -> bool {
    let path = path.replace('\\', "/");
    path == script::SCRIPTS_DIR
        || path.starts_with(&format!("{}/", script::SCRIPTS_DIR))
        || (path.ends_with(".rs") && script::is_valid_path(&path))
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

/// The list contents a reporter preview reads: the open actor's own, over
/// the project's shared ones - the same shadowing rule a run uses.
fn lists_env(s: &AppState) -> HashMap<String, Vec<ListItem>> {
    match (s.project(), s.actor_id()) {
        (Some(project), Some(actor)) => {
            let mut merged: HashMap<String, Vec<ListItem>> = project
                .global_lists
                .iter()
                .map(|list| (list.name.clone(), list.items.clone()))
                .collect();
            merged.extend(
                project
                    .actor(&actor)
                    .map(|actor| actor.graph.list_values())
                    .unwrap_or_default(),
            );
            merged
        }
        _ => HashMap::new(),
    }
}

/// The dict contents a reporter preview reads: the open actor's own, over
/// the project's shared ones - the same shadowing rule a run uses.
fn dicts_env(s: &AppState) -> HashMap<String, Vec<DictEntry>> {
    match (s.project(), s.actor_id()) {
        (Some(project), Some(actor)) => {
            let mut merged: HashMap<String, Vec<DictEntry>> = project
                .global_dicts
                .iter()
                .map(|dict| (dict.name.clone(), dict.entries.clone()))
                .collect();
            merged.extend(
                project
                    .actor(&actor)
                    .map(|actor| actor.graph.dict_values())
                    .unwrap_or_default(),
            );
            merged
        }
        _ => HashMap::new(),
    }
}

// ─── State ─────────────────────────────────────────────────────────────────

pub(crate) fn get_state(state: &SharedState) -> Result<StateDto, String> {
    let s = lock(state)?;
    Ok(state_dto(&s))
}

/// Every block and value slot, as one JSON document an agent can author
/// against. No state: the vocabulary is the same in any project.
pub(crate) fn block_vocabulary() -> Result<serde_json::Value, String> {
    Ok(blockloom_core::vocabulary::block_vocabulary())
}

// ─── Projects ──────────────────────────────────────────────────────────────

/// What opening a folder decided about sharing it.
#[derive(serde::Serialize)]
pub(crate) struct OpenReport {
    /// Opened while a live owner held the folder: this copy shares the files
    /// and reloads their saves. Only one copy that way, no merge problem.
    pub(crate) attached: bool,
    /// Took the owner lock off a live backend explicitly (`force`).
    pub(crate) took_over: bool,
    /// Who owns the folder, when someone else got there first.
    pub(crate) owner: Option<LockInfo>,
    /// Why this open deserves a second thought, if it does.
    pub(crate) warning: Option<String>,
}

/// Our claim on a folder we are about to own.
fn owner_lock(session: &str) -> LockInfo {
    LockInfo {
        pid: sync::own_pid(),
        session: session.to_string(),
        heartbeat: sync::now_secs(),
        app: "blockloom".to_string(),
    }
}

/// Opens the project in `dir`, replacing whatever was open. The Dashboard's
/// cards and its "Open a folder" both land here. When a live backend owns the
/// folder this one attaches instead of forking a silent second copy - unless
/// `force` takes the lock off it explicitly, never silently.
pub(crate) fn open_project(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    force: bool,
) -> Result<OpenReport, String> {
    let dir = std::path::PathBuf::from(path);
    let mut project = project::read_project_dir(&dir)?;
    // Booting a project loads its default scene, not wherever the last
    // editor left off.
    project.active_scene = project.boot_scene_id();
    let disk_revision = sync::read_revision(&dir);
    let mut s = lock(state)?;
    let session = s.session_id.clone();
    close_open_project(&mut s, true);
    library::remember(&dir);
    let owner = sync::live_owner(&dir, &session);
    let (owns_lock, attached, took_over, warning) = match &owner {
        Some(live) if !force => (
            false,
            true,
            false,
            Some(format!(
                "That folder is open in another Blockloom (session {}), so this copy shares its files and follows its saves. Pass force=true to take it over, or drive the editor directly with blockloom-shell --attach.",
                live.session
            )),
        ),
        other => {
            let held = sync::write_lock(&dir, &owner_lock(&session)).is_ok();
            if !held {
                tracing::warn!("Couldn't write the owner lock for {}", dir.display());
            }
            (held, false, other.is_some(), None)
        }
    };
    s.open = Some(OpenProject::new(
        project,
        dir.clone(),
        disk_revision,
        owns_lock,
        attached,
    ));
    s.library = library::list();
    if let Some(dir) = s.project_dir().map(Path::to_path_buf) {
        sync_ide(&dir);
        // Undo is empty on open, so grids nothing names can go. Another
        // owner's history might still name some.
        if owns_lock
            && !attached
            && let Some(project) = s.project()
        {
            let keep = blockloom_core::terrain::store::project_names(project);
            if let Err(e) = blockloom_core::terrain::store::prune(&dir, &keep) {
                tracing::warn!("Couldn't tidy the terrain store: {e}");
            }
        }
    }
    emit(app, &s);
    Ok(OpenReport {
        attached,
        took_over,
        owner,
        warning,
    })
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
    let session = s.session_id.clone();
    close_open_project(&mut s, true);
    library::remember(&dir);
    let revision = sync::read_revision(&dir);
    let owns_lock = sync::write_lock(&dir, &owner_lock(&session)).is_ok();
    s.open = Some(OpenProject::new(project, dir, revision, owns_lock, false));
    s.library = library::list();
    if let Some(dir) = s.project_dir().map(Path::to_path_buf) {
        sync_ide(&dir);
    }
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
    match project::save_project(&open.project, &from) {
        Ok(revision) => {
            open.revision.store(revision, Ordering::SeqCst);
            open.dirty.store(false, Ordering::SeqCst);
        }
        Err(e) => {
            tracing::warn!("Couldn't save the project: {e}");
            open.dirty.store(true, Ordering::SeqCst);
        }
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

pub(crate) fn set_project_icon(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let normalized = if path.trim().is_empty() {
        String::new()
    } else {
        let path = assets::normalize(&path)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
        if assets::kind_of(&path) != assets::AssetKind::Image {
            return Err("The game icon must be an image asset".to_string());
        }
        let file = assets::resolve(&project_dir(&s)?, &path)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
        if !file.is_file() {
            return Err(format!("{} does not exist", file.display()));
        }
        path
    };
    if s.project()
        .is_some_and(|project| project.icon == normalized)
    {
        return Ok(());
    }
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.icon = normalized;
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn save_open_project(state: &SharedState, app: &AppHandle) -> Result<u64, String> {
    let s = lock(state)?;
    if let Some(open) = &s.open {
        let revision = project::save_project(&open.project, &open.dir)?;
        open.revision.store(revision, Ordering::SeqCst);
        open.dirty.store(false, Ordering::SeqCst);
    }
    emit(app, &s);
    Ok(s.open
        .as_ref()
        .map(|open| open.loaded_revision())
        .unwrap_or(0))
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
    let session = s.session_id.clone();
    close_open_project(&mut s, true);
    library::remember(&dir);
    let revision = sync::read_revision(&dir);
    let owns_lock = sync::write_lock(&dir, &owner_lock(&session)).is_ok();
    s.open = Some(OpenProject::new(project, dir, revision, owns_lock, false));
    s.library = library::list();
    if let Some(dir) = s.project_dir().map(Path::to_path_buf) {
        sync_ide(&dir);
    }
    emit(app, &s);
    Ok(())
}

/// What reloading the folder decided.
#[derive(serde::Serialize)]
pub(crate) struct ReloadReport {
    pub(crate) reloaded: bool,
    pub(crate) revision: u64,
}

/// Where the open project stands against its folder: revisions, staleness,
/// lock owner and attach mode. What agents poll instead of the document.
pub(crate) fn sync_status(state: &SharedState) -> Result<SyncDto, String> {
    let s = lock(state)?;
    Ok(sync_dto(&s))
}

/// Loads the open project back off disk. Idle backends reload on their own
/// before every command; this is the explicit form, and the conflict prompt:
/// when unsaved in-memory edits would be lost it refuses unless `take_theirs`
/// says to take the folder's side. Saving first keeps mine instead. Undo
/// history stays per side and is never merged - the reload checkpoints, so it
/// can itself be undone.
pub(crate) fn reload_project(
    state: &SharedState,
    app: &AppHandle,
    take_theirs: bool,
) -> Result<ReloadReport, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let loaded = s
        .open
        .as_ref()
        .map(|open| open.loaded_revision())
        .unwrap_or(0);
    let dirty = s
        .open
        .as_ref()
        .is_some_and(|open| open.dirty.load(Ordering::SeqCst));
    let disk = sync::read_revision(&dir);
    if dirty && !take_theirs {
        return Err("This copy holds unsaved edits a reload would throw away. Run reload-project take_theirs=true to take the folder's side, or save-project to keep mine.".to_string());
    }
    if disk == loaded && !dirty {
        return Ok(ReloadReport {
            reloaded: false,
            revision: loaded,
        });
    }
    let project = project::read_project_dir(&dir)?;
    push_undo(&mut s);
    if let Some(open) = s.open.as_mut() {
        open.project = project;
        open.revision.store(disk, Ordering::SeqCst);
        open.dirty.store(false, Ordering::SeqCst);
    }
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(ReloadReport {
        reloaded: true,
        revision: disk,
    })
}

/// Takes the open folder's owner lock explicitly, so headless edits stop
/// deferring to whoever held it. The loud form of what `open_project`
/// refuses to do silently.
pub(crate) fn take_over_lock(state: &SharedState, app: &AppHandle) -> Result<LockInfo, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let info = owner_lock(&s.session_id);
    sync::write_lock(&dir, &info)?;
    if let Some(open) = s.open.as_mut() {
        open.owns_lock = true;
        open.attached = false;
    }
    emit(app, &s);
    Ok(info)
}

/// Lets go of whatever is open, leaving the editor on the Dashboard. The game
/// window and the run log belong to the project, so they go too. `save` is
/// false only when the project is on its way to being deleted. Releases our
/// owner lock, but only when it is still ours.
fn close_open_project(s: &mut AppState, save: bool) {
    s.interface_design = None;
    s.interface_layout = None;
    if save {
        auto_save(s);
    }
    if let Some(open) = &s.open
        && open.owns_lock
    {
        sync::release_lock(&open.dir, &s.session_id);
    }
    s.open = None;
    s.selected_actor = None;
    s.history.clear();
    s.invalid_field_buffers.clear();
    s.runtime = None;
    s.running = false;
    s.paused = false;
    s.status = None;
    s.preview_enabled = s.embedded.is_some();
    s.preview_headless = false;
    s.preview_port = None;
    s.log.clear();
}

// ─── The world ─────────────────────────────────────────────────────────────

/// Switches a project between 2D and 3D. Gravity follows the mode unless the
/// project set its own, and a running world is restarted, since the two
/// dimensions are different processes.
pub(crate) fn set_mode(
    _backend: &Backend,
    state: &SharedState,
    app: &AppHandle,
    mode: Mode,
) -> Result<(), String> {
    let mut s = lock(state)?;
    if s.project().is_none_or(|project| project.world.mode == mode) {
        return Ok(());
    }
    push_undo(&mut s);
    {
        let Some(project) = s.project_mut() else {
            return Ok(());
        };
        project.switch_mode(mode);
    }
    auto_save(&s);
    // Both pipelines live side by side now, so a dimension change is just a
    // reload: the runtime swaps live under the next rebuild.
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

// ─── Scenes ────────────────────────────────────────────────────────────────
// Each scene has its own actors and `World` settings (including dimension),
// so v1 supports mixed 2D/3D scenes. All five are one undo step each; the
// whole project snapshot is what `push_undo` checkpoints.

pub(crate) fn add_scene(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    mode: Option<Mode>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let Some(project) = s.project_mut() else {
        return Err("No project is open".to_string());
    };
    let id = project.add_scene(&name, mode);
    s.selected_actor = None;
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn duplicate_scene(
    state: &SharedState,
    app: &AppHandle,
    scene_id: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let Some(project) = s.project_mut() else {
        return Err("No project is open".to_string());
    };
    let id = project.duplicate_scene(&scene_id)?;
    s.selected_actor = None;
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(id)
}

pub(crate) fn rename_scene(
    state: &SharedState,
    app: &AppHandle,
    scene_id: String,
    name: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let Some(project) = s.project_mut() else {
        return Err("No project is open".to_string());
    };
    let old_path = project
        .scene(&scene_id)
        .map(|scene| scene.asset_path())
        .ok_or("Scene not found".to_string())?;
    let renamed = project.rename_scene(&scene_id, &name)?;
    let new_path = project
        .scene(&scene_id)
        .map(|scene| scene.asset_path())
        .unwrap_or(old_path.clone());
    // The file follows the name, so the tray never shows a stale twin.
    if new_path != old_path
        && let Some(dir) = s.project_dir().map(Path::to_path_buf)
    {
        let from = dir.join(&old_path);
        let to = dir.join(&new_path);
        if from.is_file() {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::rename(&from, &to)
                .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
        }
    }
    auto_save(&s);
    emit(app, &s);
    Ok(renamed)
}

pub(crate) fn remove_scene(
    state: &SharedState,
    app: &AppHandle,
    scene_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let path = {
        let Some(project) = s.project_mut() else {
            return Err("No project is open".to_string());
        };
        let path = project
            .scene(&scene_id)
            .map(|scene| scene.asset_path())
            .ok_or("Scene not found".to_string())?;
        project.remove_scene(&scene_id)?;
        path
    };
    s.selected_actor = None;
    // The scene asset goes with it; anything else on disk is left alone, so
    // a staged import never vanishes on save.
    if let Some(dir) = s.project_dir().map(Path::to_path_buf) {
        let file = dir.join(&path);
        if file.is_file() {
            std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        }
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the boot scene: what a fresh open - and a built game - loads. The
/// editor keeps editing wherever it is.
pub(crate) fn set_default_scene(
    state: &SharedState,
    app: &AppHandle,
    scene_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let Some(project) = s.project_mut() else {
        return Err("No project is open".to_string());
    };
    project.set_default_scene(&scene_id)?;
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

/// Makes `scene_id` the edited scene. Actor selection is per scene, so it
/// clears. Across dimensions the runtime swaps its dim2/dim3 pipeline live
/// (see `world::rebuild_world`), so no fresh process is needed.
pub(crate) fn set_active_scene(
    _backend: &Backend,
    state: &SharedState,
    app: &AppHandle,
    scene_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    if s.project().is_some_and(|p| p.active_scene == scene_id) {
        return Ok(());
    }
    push_undo(&mut s);
    {
        let Some(project) = s.project_mut() else {
            return Ok(());
        };
        project.set_active_scene(&scene_id)?;
    }
    s.selected_actor = Some(String::new());
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

// ─── Scene assets and components ─────────────────────────────────────────
// A scene is its own `.blockscene` asset; its settings are components on the
// scene the way `Place` is a component on an actor. The `set_*` world
// commands above keep editing the active scene field by field; these speak
// components, so the inspector can show lighting, sky, fog, wind, post and
// physics the way it shows an actor's parts.

/// Lists a scene's settings as components, active scene when `scene_id` is
/// empty. What the inspector draws for the scene itself.
pub(crate) fn scene_components(
    state: &SharedState,
    scene_id: Option<String>,
) -> Result<Vec<SceneComponent>, String> {
    let s = lock(state)?;
    let Some(project) = s.project() else {
        return Err("No project is open".to_string());
    };
    let scene = match scene_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => project.scene(id).ok_or("Scene not found".to_string())?,
        None => project.active_scene(),
    };
    Ok(SceneComponents::from_world(&scene.world).0)
}

/// Sets one scene component, replacing the one of the same name. Answers its
/// name. Even a `Dimension` change is just a reload now: both pipelines live
/// side by side and the runtime swaps live.
pub(crate) fn set_scene_component(
    _backend: &Backend,
    state: &SharedState,
    app: &AppHandle,
    scene_id: Option<String>,
    component: SceneComponent,
) -> Result<String, String> {
    let mut component = component;
    match &mut component {
        SceneComponent::Quality { settings } => settings.normalize(),
        SceneComponent::Physics { fixed_rate, .. } => *fixed_rate = fixed_rate.clamp(1.0, 1000.0),
        SceneComponent::Background { color } => {
            *color = normalize_block_color(color).ok_or("Choose a valid color")?;
        }
        SceneComponent::Lighting { lighting } => *lighting = sanitize_lighting(lighting.clone())?,
        SceneComponent::Navigation { settings } => validate_navigation(settings)?,
        SceneComponent::Sky { sky } => {
            sky.normalize();
            for (name, color) in sky.colors_mut() {
                *color =
                    normalize_block_color(color).ok_or(format!("Choose a valid {name} color"))?;
            }
        }
        SceneComponent::Fog { fog } => {
            fog.normalize();
            for (name, color) in fog.colors_mut() {
                *color =
                    normalize_block_color(color).ok_or(format!("Choose a valid {name} color"))?;
            }
        }
        SceneComponent::Clouds { clouds } => clouds.normalize(),
        SceneComponent::CloudLayers { layers } => blockloom_core::cloud_layers::normalize(layers),
        SceneComponent::Lightning { lightning } => {
            lightning.normalize();
            lightning.color =
                normalize_block_color(&lightning.color).ok_or("Choose a valid lightning color")?;
        }
        SceneComponent::Wind { wind } => wind.normalize(),
        SceneComponent::Director { director } => director.normalize(),
        SceneComponent::Vfx { settings } => settings.normalize(),
        SceneComponent::Post { post } => post.normalize(),
        SceneComponent::Display { display } => display.normalize(),
        SceneComponent::Sound { mixer } => {
            mixer.master_volume = blockloom_core::sound::clamp_gain(mixer.master_volume);
            mixer.music_volume = blockloom_core::sound::clamp_gain(mixer.music_volume);
            mixer.sfx_volume = blockloom_core::sound::clamp_gain(mixer.sfx_volume);
        }
        _ => {}
    }
    let mut s = lock(state)?;
    let target = {
        let project = s.project().ok_or("No project is open".to_string())?;
        match scene_id.clone().filter(|id| !id.is_empty()) {
            Some(id) => {
                if project.scene(&id).is_none() {
                    return Err("Scene not found".to_string());
                }
                id
            }
            None => project.active_scene.clone(),
        }
    };
    let old_mode = s.project().map(|p| p.world.mode);
    push_undo(&mut s);
    let name = component.name().to_string();
    let new_mode = {
        let Some(project) = s.project_mut() else {
            return Err("No project is open".to_string());
        };
        let Some(scene) = project.scene_mut(&target) else {
            return Err("Scene not found".to_string());
        };
        if let SceneComponent::Dimension { mode } = component {
            scene.switch_mode(mode);
        } else {
            SceneComponents(vec![component]).apply_to_world(&mut scene.world);
        }
        scene.world.mode
    };
    auto_save(&s);
    {
        let _ = (old_mode, new_mode);
        sync_runtime(&mut s);
    }
    emit(app, &s);
    Ok(name)
}

/// Drops one scene component; reads of it fall back to its default. Removing
/// `Dimension` returns the scene to 2D, rebuilding the runtime when it is
/// the active scene.
pub(crate) fn remove_scene_component(
    state: &SharedState,
    app: &AppHandle,
    scene_id: Option<String>,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let target = {
        let project = s.project().ok_or("No project is open".to_string())?;
        match scene_id.clone().filter(|id| !id.is_empty()) {
            Some(id) => {
                if project.scene(&id).is_none() {
                    return Err("Scene not found".to_string());
                }
                id
            }
            None => project.active_scene.clone(),
        }
    };
    let _old_mode = s.project().map(|p| p.world.mode);
    let _is_active = s.project().is_some_and(|p| p.active_scene == target);
    push_undo(&mut s);
    let _new_mode = {
        let Some(project) = s.project_mut() else {
            return Err("No project is open".to_string());
        };
        let Some(scene) = project.scene_mut(&target) else {
            return Err("Scene not found".to_string());
        };
        let mut components = SceneComponents::from_world(&scene.world);
        if !components.remove(&name) {
            return Err(format!("This scene has no \"{name}\" component"));
        }
        scene.world = components.to_world();
        scene.world.mode
    };
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Brings a `.blockscene` file from anywhere on disk into this project: it is
/// read, given a fresh id when one collides, and made active - the next save
/// writes it under `assets/scenes/`. `path` may be absolute or
/// project-relative, so a file staged in `assets/scenes/` imports by naming
/// it. What sharing a scene as a file means.
pub(crate) fn import_scene(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    // Project-relative first, so a staged file imports by its tray path.
    let source: std::path::PathBuf = blockloom_core::assets::normalize(&path)
        .and_then(|relative| blockloom_core::assets::resolve(&dir, &relative))
        .filter(|full| full.is_file())
        .unwrap_or_else(|| Path::new(&path).to_path_buf());
    let mut scene =
        project::read_scene_file(&source).map_err(|e| format!("{}: {e}", source.display()))?;
    push_undo(&mut s);
    let Some(project) = s.project_mut() else {
        return Err("No project is open".to_string());
    };
    // Fresh ids keep cross-scene references from leaking between the copy
    // and whatever project it came from.
    if project.scene(&scene.id).is_some() {
        use std::collections::HashMap;
        scene.id = uuid::Uuid::new_v4().simple().to_string();
        let mut remap = HashMap::new();
        for actor in &mut scene.actors {
            let next = uuid::Uuid::new_v4().simple().to_string();
            remap.insert(actor.id.clone(), next.clone());
            actor.id = next;
        }
        for actor in &mut scene.actors {
            if let Some(parent) = actor.parent()
                && let Some(next) = remap.get(parent).cloned()
            {
                actor.components.set_parent(&next);
            }
        }
    }
    scene.name = project.unique_scene_name(&scene.name);
    scene.path = project.unused_scene_path(&scene.name);
    let id = scene.id.clone();
    let dest_relative = scene.asset_path();
    project.scenes.push(scene);
    project.resolve_lighting_assets(&dir);
    project.active_scene = id.clone();
    s.selected_actor = None;
    // A staged file under a different name served its turn; the save below
    // writes the name-based asset, so remove the staging copy.
    if let Ok(relative) = source
        .strip_prefix(&dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        && relative != dest_relative
        && source.is_file()
    {
        let _ = std::fs::remove_file(&source);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(id)
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

pub(crate) fn set_fixed_rate(
    state: &SharedState,
    app: &AppHandle,
    fixed_rate: f32,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.world.fixed_rate = fixed_rate.clamp(1.0, 1000.0);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

fn validate_navigation(navigation: &NavSettings) -> Result<(), String> {
    if navigation.links.len() > 16 || navigation.areas.len() > 8 {
        return Err("Too many navigation links or areas".into());
    }
    let valid_point = |point: [f32; 2]| point.into_iter().all(f32::is_finite);
    if navigation.areas.iter().any(|a| {
        !valid_point(a.center) || !valid_point(a.size) || !a.cost.is_finite() || a.cost < 0.0
    }) || navigation
        .links
        .iter()
        .any(|l| !valid_point(l.from) || !valid_point(l.to) || !l.cost.is_finite() || l.cost < 0.0)
    {
        return Err("Navigation coordinates and costs must be finite and costs nonnegative".into());
    }
    Ok(())
}

pub(crate) fn set_navigation(
    state: &SharedState,
    app: &AppHandle,
    navigation: NavSettings,
) -> Result<(), String> {
    validate_navigation(&navigation)?;
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.world.navigation = navigation;
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

pub(crate) fn read_lighting_asset(state: &SharedState, path: String) -> Result<Lighting, String> {
    let s = lock(state)?;
    assets::read_lighting(&project_dir(&s)?, &path)
}

pub(crate) fn write_lighting_asset(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    lighting: Lighting,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let lighting = sanitize_lighting(lighting)?;
    assets::write_lighting(&dir, &path, &lighting)?;
    if let Some(project) = s.project_mut() {
        project.resolve_lighting_assets(&dir);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_scene_lighting_asset(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    scene_id: Option<String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let target = scene_id.filter(|id| !id.is_empty()).unwrap_or_else(|| {
        s.project()
            .map(|p| p.active_scene.clone())
            .unwrap_or_default()
    });
    let current = s
        .project()
        .and_then(|p| p.scene(&target))
        .ok_or("Scene not found")?;
    let mut lighting = if path.is_empty() {
        current.world.lighting.clone()
    } else {
        assets::read_lighting(&project_dir(&s)?, &path)?
    };
    lighting.asset = assets::normalize(&path).ok_or("Invalid asset path")?;
    push_undo(&mut s);
    s.project_mut()
        .and_then(|p| p.scene_mut(&target))
        .ok_or("Scene not found")?
        .world
        .lighting = lighting;
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

fn sanitize_lighting(lighting: Lighting) -> Result<Lighting, String> {
    let light_color =
        normalize_block_color(&lighting.light_color).ok_or("Choose a valid light color")?;
    let ambient_color =
        normalize_block_color(&lighting.ambient_color).ok_or("Choose a valid ambient color")?;
    Ok(Lighting {
        light_color,
        ambient_color,
        illuminance: lighting.illuminance.clamp(0.0, 200_000.0),
        ambient_brightness: lighting.ambient_brightness.clamp(0.0, 1000.0),
        shadow_map_size: lighting.shadow_map_size.clamp(512, 8192),
        shadow_bias: lighting.shadow_bias.clamp(0.0, 0.5),
        sky: String::new(),
        shadows: lighting.shadows.clone().sanitized(),
        ray_tracing: lighting.ray_tracing.clone().sanitized(),
        sun_cookie: lighting.sun_cookie.trim().replace('\\', "/"),
        sun_cookie_size: if lighting.sun_cookie_size.is_finite() {
            lighting.sun_cookie_size.clamp(0.01, 100_000.0)
        } else {
            20.0
        },
        ..lighting
    })
}

pub(crate) fn set_lighting(
    state: &SharedState,
    app: &AppHandle,
    lighting: Lighting,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let sanitized = sanitize_lighting(lighting.clone())?;
    if let Some(project) = s.project_mut() {
        // A sky path here is the old spelling of an HDRI sky.
        if !lighting.sky.trim().is_empty() {
            let sky = &mut project.world.sky;
            sky.kind = SkyKind::Hdri;
            sky.hdri.path = lighting.sky.clone();
            sky.hdri.brightness = lighting.sky_brightness;
            sky.normalize();
        }
        project.world.lighting = sanitized;
        // Editing inline settings detaches them from the shared asset.
        project.world.lighting.asset.clear();
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the 3D sky: its kind, the sun's position, each kind's settings and
/// what it lights. Edited on the scene's Sky component.
pub(crate) fn set_sky(state: &SharedState, app: &AppHandle, sky: Sky) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut sky = sky;
    sky.normalize();
    for (name, color) in sky.colors_mut() {
        *color = normalize_block_color(color).ok_or(format!("Choose a valid {name} color"))?;
    }
    if let Some(project) = s.project_mut() {
        project.world.sky = sky;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the air: height fog, volumetric fog and aerial perspective. What
/// the project settings dialog's Fog section edits.
pub(crate) fn set_fog(state: &SharedState, app: &AppHandle, fog: Fog) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut fog = fog;
    fog.normalize();
    for (name, color) in fog.colors_mut() {
        *color = normalize_block_color(color).ok_or(format!("Choose a valid {name} color"))?;
    }
    if let Some(project) = s.project_mut() {
        project.world.fog = fog;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_clouds(
    state: &SharedState,
    app: &AppHandle,
    clouds: blockloom_core::clouds::Clouds,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut clouds = clouds;
    clouds.normalize();
    if let Some(project) = s.project_mut() {
        project.world.clouds = clouds;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn set_cloud_layers(
    state: &SharedState,
    app: &AppHandle,
    layers: Vec<blockloom_core::cloud_layers::CloudLayer>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut layers = layers;
    blockloom_core::cloud_layers::normalize(&mut layers);
    if let Some(project) = s.project_mut() {
        project.world.cloud_layers = layers;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Paints a stroke into a cloud layer's coverage file and points the layer
/// at it, bumping its revision so the runtime rereads it.
pub(crate) fn paint_cloud_layer(
    state: &SharedState,
    app: &AppHandle,
    layer: usize,
    brush: blockloom_core::cloud_layers::Brush,
    points: Vec<[f32; 2]>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let current = s
        .project()
        .ok_or("No project is open")?
        .world
        .cloud_layers
        .get(layer)
        .cloned()
        .ok_or_else(|| format!("There is no cloud layer {}", layer + 1))?;
    let path = blockloom_core::cloud_layers::paint_file(&dir, &current, layer, &brush, &points)?;
    let _ = pipeline::note_imported(&dir, &path, "cloud layer coverage");
    push_undo(&mut s);
    if let Some(l) = s
        .project_mut()
        .and_then(|p| p.world.cloud_layers.get_mut(layer))
    {
        l.coverage_texture = path.clone();
        l.revision = l.revision.wrapping_add(1);
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(path)
}

/// Bakes the clouds' shape and erosion noise from the cloud seed into volume
/// assets and points the clouds at them, so they can be edited or replaced.
pub(crate) fn bake_cloud_noise(
    state: &SharedState,
    app: &AppHandle,
) -> Result<Vec<String>, String> {
    use blockloom_core::clouds::{CloudNoise, noise_strip};
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let seed = s
        .project()
        .ok_or("No project is open")?
        .world
        .wind
        .clouds
        .seed;
    let mut written = Vec::new();
    for kind in [CloudNoise::Shape, CloudNoise::Detail] {
        let path = kind.asset_path().to_string();
        let full = assets::resolve(&dir, &path)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        noise_strip(kind, kind.size(), seed)
            .save(&full)
            .map_err(|e| format!("{}: {e}", full.display()))?;
        // A PNG reads as a texture by its extension; this one is a volume.
        let _ = pipeline::set_role(&dir, &path, Some(pipeline::ImportRole::Volume));
        let _ = pipeline::note_imported(&dir, &path, "cloud noise");
        written.push(path);
    }
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.world.clouds.shape_volume = written[0].clone();
        project.world.clouds.detail_volume = written[1].clone();
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(written)
}

/// Sets how lightning looks and sounds, and the storm that throws it.
pub(crate) fn set_lightning(
    state: &SharedState,
    app: &AppHandle,
    lightning: Lightning,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut lightning = lightning;
    lightning.normalize();
    lightning.color = normalize_block_color(&lightning.color)
        .ok_or("Choose a valid lightning color".to_string())?;
    if let Some(project) = s.project_mut() {
        project.world.lightning = lightning;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the wind: direction, speed, gusts, the profile near the ground, the
/// storm dial and how the clouds ride it.
pub(crate) fn set_wind(state: &SharedState, app: &AppHandle, wind: Wind) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut wind = wind;
    wind.normalize();
    if let Some(project) = s.project_mut() {
        project.world.wind = wind;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the time-of-day and weather director: the 24h clock, its curve
/// tracks and the project's weather presets. Disabled by default, so old
/// projects keep exactly the look they shipped.
pub(crate) fn set_director(
    state: &SharedState,
    app: &AppHandle,
    director: blockloom_core::director::Director,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut director = director;
    director.normalize();
    if let Some(project) = s.project_mut() {
        project.world.director = director;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Replaces the director with one of its keyframe presets
/// (dawn, noon, dusk or midnight): a sun track through that moment plus
/// matching exposure, fog and cloud tracks. Answers the preset's name.
pub(crate) fn apply_director_preset(
    state: &SharedState,
    app: &AppHandle,
    preset: String,
) -> Result<String, String> {
    let director = blockloom_core::director::Director::keyframe_preset(&preset)
        .ok_or_else(|| "Pick dawn, noon, dusk or midnight".to_string())?;
    let name = preset.trim().to_string();
    set_director(state, app, director)?;
    Ok(name)
}

/// Copies a weather preset - a project one, or a built-in like Storm - to a
/// project preset under a new name, replacing the project preset of that
/// name when one exists. What the preset gallery's Save copy button writes
/// through; answers the saved name.
pub(crate) fn save_director_preset(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    from: String,
) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Name the preset first".to_string());
    }
    let s = lock(state)?;
    let director = s
        .project()
        .map(|project| project.world.director.clone())
        .ok_or_else(|| "No project is open".to_string())?;
    let mut preset = director
        .preset(&from)
        .ok_or_else(|| format!("There's no weather called \"{}\"", from.trim()))?;
    preset.name = name.clone();
    preset.normalize();
    let mut next: Vec<blockloom_core::director::WeatherPreset> = director
        .presets
        .iter()
        .filter(|p| !p.name.eq_ignore_ascii_case(&name))
        .cloned()
        .collect();
    if next.len() >= 32 {
        return Err("A project holds at most 32 presets".to_string());
    }
    next.push(preset);
    drop(s);
    let mut full = director;
    full.presets = next;
    set_director(state, app, full)?;
    Ok(name)
}

/// Sets the project's particle budget and whether emitters stay on the CPU.
pub(crate) fn set_vfx(
    state: &SharedState,
    app: &AppHandle,
    vfx: blockloom_core::vfx::VfxSettings,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut vfx = vfx;
    vfx.normalize();
    if let Some(project) = s.project_mut() {
        project.world.vfx = vfx;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Saves the shared rendering budgets and frame-time feedback settings.
pub(crate) fn set_quality(
    state: &SharedState,
    app: &AppHandle,
    mut quality: blockloom_core::quality::Settings,
) -> Result<(), String> {
    quality.normalize();
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.world.quality = quality;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the post-process on the world camera: the whole chain from exposure
/// to grain. What the project settings dialog edits; the runtime seeds
/// its camera components from it on every rebuild.
pub(crate) fn set_post_process(
    state: &SharedState,
    app: &AppHandle,
    post: PostProcess,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut post = post;
    post.normalize();
    if let Some(project) = s.project_mut() {
        project.world.post = post;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the output signal, peak brightness and paper white. Takes effect
/// where the display offers HDR; SDR everywhere else.
pub(crate) fn set_display_output(
    state: &SharedState,
    app: &AppHandle,
    display: DisplayOutput,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let mut display = display;
    display.normalize();
    if let Some(project) = s.project_mut() {
        project.world.display = display;
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

/// Sets the saved mix: one gain per bus, in linear 0-2. What the project
/// settings dialog edits, and what a rebuilt world reseeds its live gains
/// from. Clamped like a live write, so a stored mix can never blow out.
pub(crate) fn set_sound_mixer(
    state: &SharedState,
    app: &AppHandle,
    mixer: SoundMixer,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.world.sound = SoundMixer {
            master_volume: blockloom_core::sound::clamp_gain(mixer.master_volume),
            music_volume: blockloom_core::sound::clamp_gain(mixer.music_volume),
            sfx_volume: blockloom_core::sound::clamp_gain(mixer.sfx_volume),
        };
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
    send_selection(&mut s);
    emit(app, &s);
    Ok(())
}

/// A scene view drag landed: writes where the actor now stands, as one undo
/// step. A child placed in its parent's frame gets its offset written too,
/// since that, not its `Place`, is what puts it there.
pub(crate) fn place_from_view(
    s: &mut AppState,
    actor_id: &str,
    placement: Placement,
    offset: Option<[f32; 3]>,
    volume: Option<blockloom_protocol::VolumeBounds>,
) {
    if s.running || s.project().and_then(|p| p.actor(actor_id)).is_none() {
        return;
    }
    push_undo(s);
    // Drags land on float noise; the inspector shouldn't show it.
    let tidy = |value: f32, step: f32| (value / step).round() * step;
    let placement = Placement {
        position: placement.position.map(|v| tidy(v, 1e-4)),
        rotation: placement.rotation.map(|v| tidy(v, 1e-3)),
        scale: tidy(placement.scale, 1e-4),
        stretch: placement.stretch.map(|v| tidy(v, 1e-4)),
    };
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(actor_id)) {
        actor.components.set_placement(placement);
        if let Some(offset) = offset {
            actor
                .components
                .set_parent_offset(Some(offset.map(|v| tidy(v, 1e-4))));
        }
        if let Some(bounds) = volume
            && let Some(mut spec) = actor.components.volume().cloned()
        {
            spec.half_extents = bounds.half_extents.map(|v| tidy(v, 1e-4));
            spec.radius = tidy(bounds.radius, 1e-4);
            spec.blend_distance = tidy(bounds.blend_distance, 1e-4);
            actor
                .components
                .insert(ActorComponent::Volume { volume: spec });
        }
    }
    s.selected_actor = Some(actor_id.to_string());
    auto_save(s);
    sync_runtime(s);
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

/// Moves an actor within the list and optionally under another one: `parent`
/// is the id it hangs off afterwards (empty for the top level) and `before`
/// the level-mate it lands in front of (empty for the end of the level).
/// Resolves to whether anything changed. Loops and strangers are refused.
pub(crate) fn move_actor(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    parent: String,
    before: String,
) -> Result<bool, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let moved = match s.project_mut() {
        Some(project) => project.move_actor(&actor_id, &parent, &before)?,
        None => return Err("No project is open".to_string()),
    };
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(moved)
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
    check_parent(s.project(), &actor_id, &component)?;
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
    mut component: ActorComponent,
) -> Result<(), String> {
    let mut s = lock(state)?;
    check_parent(s.project(), &actor_id, &component)?;
    if let ActorComponent::Material { material } = &mut component {
        material.normalize();
    }
    if let ActorComponent::Terrain { terrain } = &mut component {
        terrain.normalize();
    }
    if let ActorComponent::Water { water } = &mut component {
        water.normalize();
    }
    if let ActorComponent::Buoyancy { buoyancy } = &mut component {
        buoyancy.normalize();
    }
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

/// A `Parent` has to name another actor that isn't already hanging off this
/// one. `apply_parenting` walks the chain once a step from the roots down,
/// and a loop has no root to start at.
fn check_parent(
    project: Option<&Project>,
    actor_id: &str,
    component: &ActorComponent,
) -> Result<(), String> {
    let ActorComponent::Parent { parent, .. } = component else {
        return Ok(());
    };
    if parent.is_empty() {
        return Ok(());
    }
    let Some(project) = project else {
        return Ok(());
    };
    if parent == actor_id {
        return Err("An actor can't hang off itself".to_string());
    }
    let Some(other) = project.actor(parent) else {
        return Err("No such actor to hang off".to_string());
    };
    let parents: HashMap<String, String> = project
        .actors
        .iter()
        .filter_map(|actor| Some((actor.id.clone(), actor.parent()?.to_string())))
        .collect();
    if project::reaches(&parents, parent, actor_id) {
        return Err(format!(
            "\"{}\" already hangs off this actor, so it can't be its parent",
            other.name
        ));
    }
    Ok(())
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
        "Model" => Visual::Model {
            path: String::new(),
            tint: "#4C97FF".to_string(),
            scale: [1.0, 1.0, 1.0],
            animation: String::new(),
        },
        "Tilemap" => Visual::Tilemap {
            tilemap: blockloom_core::material::Tilemap::default(),
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
        "Model" => "Model",
        "Tilemap" => "Tiles",
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
    let _ = build_scripts(&mut s);
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
        s.runtime = Some(RuntimeHandle::spawn(
            project.world.mode,
            backend.clone(),
            s.embedded.clone(),
        )?);
        greet(&mut s);
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
    // The sidecar belongs to the process, so a fresh runtime re-enables it.
    if s.preview_enabled {
        let headless = s.preview_headless;
        let runtime = s.runtime.as_mut().expect("checked above");
        if !runtime.send(&blockloom_protocol::EditorMessage::Preview {
            enabled: true,
            headless,
        }) {
            s.preview_port = None;
        }
    }
    s.interface_design = None;
    s.interface_layout = None;
    s.log.clear();
    s.running = true;
    s.paused = false;
    emit(app, &s);
    Ok(())
}

pub(crate) fn stop_project(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    s.interface_design = None;
    s.interface_layout = None;
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
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::Pause { paused })
    {
        s.runtime = None;
        s.running = false;
        s.paused = false;
        s.status = None;
        emit(app, &s);
        return Err("Lost the connection to the game runtime".to_string());
    }
    s.paused = paused && s.running;
    emit(app, &s);
    Ok(())
}

/// Brings up a world for the scene view to edit: the project loaded but not
/// started. Does nothing while one is already up for this dimension.
pub(crate) fn open_world(
    backend: &Backend,
    state: &SharedState,
    app: &AppHandle,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let Some(project) = s.project().cloned() else {
        return Err("No project is open".to_string());
    };
    if s.runtime
        .as_ref()
        .is_some_and(|runtime| runtime.mode == project.world.mode)
    {
        return Ok(());
    }
    s.runtime = None;
    s.runtime = Some(RuntimeHandle::spawn(
        project.world.mode,
        backend.clone(),
        s.embedded.clone(),
    )?);
    let dir = s
        .project_dir()
        .map(|dir| dir.to_string_lossy().into_owned());
    let alive = greet(&mut s)
        && s.runtime.as_mut().is_some_and(|runtime| {
            runtime.send(&blockloom_protocol::EditorMessage::Load {
                project: Box::new(project),
                dir,
            })
        });
    if !alive {
        s.runtime = None;
        emit(app, &s);
        return Err("Lost the connection to the game runtime".to_string());
    }
    if s.preview_enabled {
        let headless = s.preview_headless;
        let shown = s.runtime.as_mut().is_some_and(|runtime| {
            runtime.send(&blockloom_protocol::EditorMessage::Preview {
                enabled: true,
                headless,
            })
        });
        if !shown {
            s.preview_port = None;
        }
    }
    s.running = false;
    s.paused = false;
    emit(app, &s);
    Ok(())
}

/// How the scene view edits: the tool, snapping and its steps. An editor
/// preference, so it isn't saved with the project.
pub(crate) fn set_scene_view(
    state: &SharedState,
    view: blockloom_protocol::SceneView,
) -> Result<(), String> {
    let mut s = lock(state)?;
    s.scene_view = view.clone();
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::SceneView(view))
    {
        s.runtime = None;
    }
    Ok(())
}

/// Points the scene view's camera at the selected actor.
pub(crate) fn frame_selected(state: &SharedState) -> Result<(), String> {
    let mut s = lock(state)?;
    send_selection(&mut s);
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::FrameSelected)
    {
        s.runtime = None;
    }
    Ok(())
}

/// Saves the Game view's next frame, linear and before tonemapping, as an
/// OpenEXR file under the project's `screenshots/`. Answers with its path;
/// the world says in the run log once it is written.
pub(crate) fn capture_exr(state: &SharedState) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or("No project is open")?;
    let name: String = s
        .project()
        .map(|project| project.name.as_str())
        .unwrap_or("Screenshot")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    let path = dir
        .join("screenshots")
        .join(format!("{} {stamp}.exr", name.trim()));
    let path = path.to_string_lossy().into_owned();
    let Some(runtime) = s.runtime.as_mut() else {
        return Err("Open the Game view to capture it".to_string());
    };
    if !runtime.send(&blockloom_protocol::EditorMessage::CaptureExr { path: path.clone() }) {
        s.runtime = None;
        return Err("The game world has stopped".to_string());
    }
    Ok(path)
}

/// Asks the world to bake light probes where they stand - `actors`, or every
/// probe when empty - into `.blockloom/probes`. Answers with how many were
/// asked for; each bake says so in the run log once it is written.
pub(crate) fn bake_probes(state: &SharedState, actors: Vec<String>) -> Result<usize, String> {
    let mut s = lock(state)?;
    let project = s.project().ok_or("No project is open")?;
    if !project.world.mode.is_3d() {
        return Err("Light probes need a 3D world".to_string());
    }
    let probes: Vec<String> = project
        .actors
        .iter()
        .filter(|actor| actor.components.probe().is_some())
        .map(|actor| actor.id.clone())
        .collect();
    let wanted: Vec<String> = if actors.is_empty() {
        probes
    } else {
        if let Some(missing) = actors.iter().find(|id| !probes.contains(id)) {
            return Err(format!("{missing} has no Probe component"));
        }
        actors
    };
    if wanted.is_empty() {
        return Err("Nothing carries a Probe component".to_string());
    }
    let count = wanted.len();
    let Some(runtime) = s.runtime.as_mut() else {
        return Err("Open the Game view to bake probes".to_string());
    };
    if !runtime.send(&blockloom_protocol::EditorMessage::BakeProbes { actors: wanted }) {
        s.runtime = None;
        return Err("The game world has stopped".to_string());
    }
    Ok(count)
}

/// Every light probe's bake: whether one is on disk, and whether the probe
/// or the scene it sees has moved on since.
pub(crate) fn probe_status(
    state: &SharedState,
) -> Result<Vec<blockloom_core::probe::ProbeStatus>, String> {
    let s = lock(state)?;
    let dir = s.project_dir().ok_or("No project is open")?;
    let project = s.project().ok_or("No project is open")?;
    Ok(blockloom_core::probe::statuses(project, dir))
}

/// Closes the game window without touching the project.
pub(crate) fn close_runtime(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    s.runtime = None;
    s.running = false;
    s.paused = false;
    s.status = None;
    s.preview_port = None;
    s.preview_enabled = s.embedded.is_some();
    emit(app, &s);
    Ok(())
}

/// Turns the embedded preview sidecar on or off. The runtime serves MJPEG on
/// loopback and reports the port, which the viewport reads directly. The
/// window stays up unless headless mode hides it.
pub(crate) fn set_preview_enabled(
    state: &SharedState,
    app: &AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut s = lock(state)?;
    // An embedded world has nowhere else to be seen.
    let enabled = enabled || s.embedded.is_some();
    s.preview_enabled = enabled;
    if !enabled {
        s.preview_port = None;
    }
    let headless = s.preview_headless;
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::Preview { enabled, headless })
    {
        s.runtime = None;
        s.running = false;
        s.paused = false;
        s.status = None;
        s.preview_port = None;
        emit(app, &s);
        return Err("Lost the connection to the game runtime".to_string());
    }
    emit(app, &s);
    Ok(())
}

/// Hides the runtime's OS window while the preview stream runs, or brings it
/// back. Re-sends the preview state when the sidecar is up, which applies
/// live: starting the sidecar is idempotent, so the stream keeps serving.
pub(crate) fn set_preview_headless(
    state: &SharedState,
    app: &AppHandle,
    headless: bool,
) -> Result<(), String> {
    let mut s = lock(state)?;
    s.preview_headless = headless;
    let (enabled, headless) = (s.preview_enabled, s.preview_headless);
    if enabled
        && let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::Preview { enabled, headless })
    {
        s.runtime = None;
        s.running = false;
        s.paused = false;
        s.status = None;
        s.preview_port = None;
        emit(app, &s);
        return Err("Lost the connection to the game runtime".to_string());
    }
    emit(app, &s);
    Ok(())
}

/// Forwards one viewport input event to the runtime. Sent often, so this
/// skips the state broadcast: nothing in the snapshot changed.
pub(crate) fn preview_input(
    state: &SharedState,
    input: blockloom_protocol::PreviewInput,
) -> Result<(), String> {
    let mut s = lock(state)?;
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::PreviewInput { input })
    {
        s.runtime = None;
        s.running = false;
        s.paused = false;
        s.status = None;
        s.preview_port = None;
        return Err("Lost the connection to the game runtime".to_string());
    }
    Ok(())
}

/// Advances a paused world by one fixed tick. Ignored unless paused.
pub(crate) fn step_project(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::Step)
    {
        s.runtime = None;
        s.running = false;
        s.paused = false;
        s.status = None;
        s.preview_port = None;
        emit(app, &s);
        return Err("Lost the connection to the game runtime".to_string());
    }
    emit(app, &s);
    Ok(())
}

/// [`build_scripts_for`] aimed at this machine, which is what Play needs.
fn build_scripts(s: &mut AppState) -> usize {
    build_scripts_for(s, None)
}

/// Compiles every actor's script for `target` (`None` being this machine),
/// logging whatever rustc has to say about the ones that fail and answering
/// how many did. A failed script just doesn't load, so Play carries on - a
/// build can't, since the game would ship without it. Without a toolchain
/// there is one log line, not one per script: blocks still run, only scripts
/// stay quiet.
fn build_scripts_for(s: &mut AppState, target: Option<&str>) -> usize {
    let Some(dir) = s.project_dir().map(Path::to_path_buf) else {
        return 0;
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
    if scripts.is_empty() {
        return 0;
    }
    if target.is_none()
        && let Err(error) = script::toolchain_version()
    {
        s.push_log(LogLine {
            kind: "error".to_string(),
            actor: "Scripts".to_string(),
            text: format!(
                "Scripts need a Rust toolchain and this machine doesn't have one, so {} script(s) won't run. Blocks still run. {error}",
                scripts.len()
            ),
        });
        return scripts.len();
    }
    let mut failed = 0;
    // Android links through the NDK wrapper, which a direct rustc call only
    // takes as `-C linker=`. Anything else links the way Play does.
    let linker = target
        .filter(|triple| android::is_android(triple))
        .and_then(android::ndk_linker_for);
    for (actor, path) in scripts {
        if let Err(error) = script::compile_for_with_linker(&dir, &path, target, linker.as_deref())
        {
            failed += 1;
            s.push_log(LogLine {
                kind: "error".to_string(),
                actor,
                text: format!("{path} didn't compile:\n{error}"),
            });
        }
    }
    failed
}

// ─── Building ──────────────────────────────────────────────────────────────

/// Every platform the Build dialog offers, this machine's first, each with
/// whether it could be built for right now and why not when it couldn't.
pub(crate) fn list_build_targets(state: &SharedState) -> Result<Vec<build::TargetStatus>, String> {
    let s = lock(state)?;
    let project = s.project().cloned();
    drop(s);
    let has_scripts = project.as_ref().is_some_and(|project| {
        project
            .actors
            .iter()
            .any(|actor| actor.components.script().is_some())
    });
    let fast_source = project
        .as_ref()
        .ok_or_else(|| "No project is open".to_string())
        .and_then(|project| {
            codegen::compile(project)
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
    Ok(build::targets(
        has_scripts,
        fast_source,
        &blockloom_protocol::runtime_path(),
    ))
}

pub(crate) struct BuildRequest {
    project: Project,
    dir: PathBuf,
    target: &'static build::Target,
    params: crate::build_jobs::BuildParams,
}

pub(crate) fn prepare_build_game(
    state: &SharedState,
    params: crate::build_jobs::BuildParams,
) -> Result<BuildRequest, String> {
    let s = lock(state)?;
    let project = s
        .project()
        .cloned()
        .ok_or_else(|| "No project is open".to_string())?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "This project has no folder to build from".to_string())?;
    let target = match params.target.as_deref() {
        Some(triple) => build::target(triple)
            .ok_or_else(|| format!("Blockloom doesn't know how to build for {triple}"))?,
        None => build::host()
            .ok_or("Blockloom has no name for this platform, so it can't build for it")?,
    };
    if params.device.is_some() && !target.is_android() {
        return Err("Deploying to a device requires an Android target".to_string());
    }
    auto_save(&s);
    Ok(BuildRequest {
        project,
        dir,
        target,
        params,
    })
}

pub(crate) fn run_build_game(
    state: &SharedState,
    app: &AppHandle,
    request: BuildRequest,
) -> Result<build::Build, String> {
    let BuildRequest {
        project,
        dir,
        target,
        params,
    } = request;
    let crate::build_jobs::BuildParams {
        path,
        fast,
        hdr,
        store_pass,
        key_pass,
        remember_passwords,
        ..
    } = params;
    blockloom_core::build_control::step("Checking build requirements")?;
    let player = if target.is_android() {
        // Fail fast on a release row with no password, before the long NDK
        // cross-build below: the build would refuse it anyway.
        android::resolve_signing(&project.android, store_pass.as_deref(), key_pass.as_deref())?;
        // No staged player: the desktop NDK cross-builds the runtime into
        // the APK's `lib/<abi>/`, so `build` takes its path as the player.
        // A prebuilt one wins when the env names it (see
        // `android::runtime_so_override`); otherwise this compiles one,
        // which is the long step of an Android build.
        android::runtime_so_for(target.triple)?
    } else {
        build::player_for(target, &blockloom_protocol::runtime_path()).ok_or_else(|| {
            format!(
                "There is no player for {}. Stage one in players/{}/ beside Blockloom.",
                target.label, target.triple
            )
        })?
    };
    let fast_source = codegen::compile(&project)
        .map(|_| ())
        .map_err(|error| error.to_string());
    let fast_toolchain = match build::script_target(target) {
        None => script::toolchain_version().map(|_| ()),
        Some(triple) => script::target_installed(triple),
    };
    let fast = match fast {
        Some(false) => false,
        // A browser runs the blocks on the VM; native logic is a library
        // nothing there can open.
        Some(true) if target.is_web() => {
            return Err("A web build runs its blocks on the VM, so it can't compile them".into());
        }
        None if target.is_web() => false,
        Some(true) => {
            fast_source?;
            fast_toolchain?;
            true
        }
        None => fast_source.is_ok() && fast_toolchain.is_ok(),
    };

    // A script that won't compile can't be shipped around: the built game
    // would load an actor whose behaviour silently isn't there. Cross builds
    // compile their own copy, since a script is native code like the player.
    blockloom_core::build_control::step("Compiling actor scripts")?;
    let script_target = build::script_target(target);
    let linker = script_target
        .filter(|triple| android::is_android(triple))
        .and_then(android::ndk_linker_for);
    let mut failed = false;
    for actor in &project.actors {
        blockloom_core::build_control::check()?;
        if let Some(path) = actor.components.script()
            && let Err(error) =
                script::compile_for_with_linker(&dir, path, script_target, linker.as_deref())
        {
            blockloom_core::build_control::check()?;
            failed = true;
            let mut s = lock(state)?;
            s.push_log(LogLine {
                kind: "error".to_string(),
                actor: actor.name.clone(),
                text: format!("{path} didn't compile:\n{error}"),
            });
            emit(app, &s);
        }
    }
    if failed {
        return Err("A script didn't compile, so the game wasn't built - see the log".to_string());
    }

    if fast {
        blockloom_core::build_control::step("Compiling blocks to native code")?;
        match build::script_target(target) {
            Some(triple) if target.is_android() => {
                codegen::compile_for_with_linker(
                    &project,
                    &dir,
                    Some(triple),
                    android::ndk_linker_for(triple).as_deref(),
                )?;
            }
            other => {
                codegen::compile_for(&project, &dir, other)?;
            }
        }
    }

    let options = build::BuildOptions {
        fast,
        sdr_only: target.is_web() || !hdr.unwrap_or(target.hdr_default().0),
        store_pass,
        key_pass,
        remember_passwords,
    };
    let built = build::build(
        &project,
        &dir,
        target,
        &player,
        Path::new(&path),
        options.clone(),
    )?;
    blockloom_core::build_control::check()?;
    let mut s = lock(state)?;
    // A cached Android build reused the previous APK, so say so instead of
    // claiming a fresh compile.
    let action = if built.cached { "Reusing" } else { "Built" };
    s.push_log(LogLine {
        kind: "say".to_string(),
        actor: "Blockloom".to_string(),
        text: format!(
            "{} {} for {}: {} asset(s), {} script(s), {} shader(s), {} blocks, {}{}{}{}, {} -> {} and {}{}",
            action,
            project.name,
            target.label,
            built.assets,
            built.scripts,
            built.shaders,
            if built.compiled { "native" } else { "VM" },
            if options.sdr_only { "SDR only" } else { "HDR" },
            if built.sky { ", sky baked to BC6H" } else { "" },
            if built.dlss {
                ", DLSS DLLs included"
            } else {
                ""
            },
            if target.is_android() {
                format!(", {}-signed", built.signed)
            } else {
                String::new()
            },
            build::size_text(built.size),
            built.dir.display(),
            built.archive.display(),
            if built.cached {
                " (nothing changed)"
            } else {
                ""
            },
        ),
    });
    emit(app, &s);
    Ok(built)
}

// ─── Android ─────────────────────────────────────────────────────────────
// Player-only target, so these need no open project: they report the
// desktop toolchain (SDK/NDK/JDK/Rust targets) the APK build will use.

/// The Android toolchain as it stands: SDK/NDK paths, license stamp, and
/// one probe row per piece `just android-check` checks. Needs no device.
pub(crate) fn android_status() -> Result<android::AndroidStatus, String> {
    Ok(android::status())
}

/// What `adb devices` sees through the installed platform-tools, or why
/// there is no adb to ask.
pub(crate) fn android_device_status() -> Result<Vec<android::Device>, String> {
    android::device_status()
}

/// Downloads the cmdline-tools bootstrap when the SDK row has none, then
/// installs the pinned platform, build-tools, platform-tools and NDK.
/// Licenses stay unaccepted; that is `android_accept_licenses`.
pub(crate) fn android_install_sdk() -> Result<android::InstallReport, String> {
    android::install_sdk()
}

/// Shows the SDK license texts, or accepts them when `accept` is true and
/// records the stamp in the app config.
pub(crate) fn android_accept_licenses(accept: bool) -> Result<android::LicenseReport, String> {
    android::accept_licenses(accept)
}

/// Installs `apk` on `device` (or the only device when unset) and launches
/// it, answering the started component. An emulator counts as a device.
pub(crate) fn android_install(
    apk: String,
    app: String,
    device: Option<String>,
) -> Result<android::ApkInstall, String> {
    android::install_apk(std::path::Path::new(&apk), &app, device.as_deref())
}

/// Dumps the device log and keeps the runtime's `blockloom:` markers plus
/// any Rust panic: what `just android-smoke` checks, and what a developer
/// reads when a game misbehaves on device. One shot, not a stream.
pub(crate) fn android_logcat(device: Option<String>) -> Result<android::Logcat, String> {
    android::logcat(device.as_deref(), "blockloom:")
}

/// Polls the device log the way the Build dialog streams it: dumps, clears
/// the buffer for the next poll, and appends every kept line to the RunLog
/// (markers as `say`, panics as `error`, both from `Android`), then answers
/// the same dump. The install step clears the buffer on launch, so the
/// first poll after it reads only the fresh run.
pub(crate) fn android_logcat_tail(
    state: &SharedState,
    app: &AppHandle,
    device: Option<String>,
) -> Result<android::Logcat, String> {
    let dumped = android::logcat_tail(device.as_deref(), "blockloom:")?;
    if !dumped.lines.is_empty() || !dumped.panics.is_empty() {
        let mut s = lock(state)?;
        for line in &dumped.lines {
            s.push_log(LogLine {
                kind: "say".to_string(),
                actor: "Android".to_string(),
                text: line.trim().to_string(),
            });
        }
        for line in &dumped.panics {
            s.push_log(LogLine {
                kind: "error".to_string(),
                actor: "Android".to_string(),
                text: line.trim().to_string(),
            });
        }
        emit(app, &s);
    }
    Ok(dumped)
}

/// What the OS keyring keeps for the open project's release key, so the
/// Build dialog can say when typing passwords is optional. Needs no device.
pub(crate) fn android_keyring_status(
    state: &SharedState,
) -> Result<android::KeyringStatus, String> {
    let s = lock(state)?;
    let Some(project) = s.project() else {
        return Err("Open a project first.".to_string());
    };
    Ok(android::keyring_status_for(&project.android))
}

/// Forgets whatever the OS keyring keeps for the open project's release
/// key. Answers whether anything was there. Needs no device.
pub(crate) fn android_forget_passwords(state: &SharedState) -> Result<bool, String> {
    let s = lock(state)?;
    let Some(project) = s.project().cloned() else {
        return Err("Open a project first.".to_string());
    };
    drop(s);
    android::forget_signing(&project.android)
}

/// The emulator rows as they stand: binary, image and every AVD with its
/// run state. Needs no open project and no device.
pub(crate) fn android_emulator_status() -> Result<android::EmulatorStatus, String> {
    Ok(android::emulator_status())
}

/// Makes an AVD on the pinned image, answering its name. Empty names the
/// managed default. Needs no open project and no device.
pub(crate) fn android_create_avd(name: Option<String>) -> Result<String, String> {
    android::create_avd(name.as_deref())
}

/// Renames a stopped AVD.
pub(crate) fn android_rename_avd(name: String, new_name: String) -> Result<String, String> {
    android::rename_avd(&name, &new_name)
}

/// Deletes a stopped AVD and its saved data.
pub(crate) fn android_delete_avd(name: String) -> Result<String, String> {
    android::delete_avd(&name)
}

/// Boots `avd` (the managed default when unset, created on the spot when no
/// AVDs exist at all) and waits up to `wait_secs` (5 minutes when unset, 0
/// to return right after spawning) for adb to see it booted. Needs no open
/// project. `headless` hides the host window: the editor shows the screen
/// itself through `android_mirror_frame`, like Android Studio's embedded
/// emulator tool window.
pub(crate) fn android_start_emulator(
    avd: Option<String>,
    wait_secs: Option<u64>,
    headless: bool,
) -> Result<android::EmulatorBoot, String> {
    android::start_emulator_with_options(avd.as_deref(), wait_secs, headless)
}

/// Stops the running emulator on `serial` (`adb emu kill`). Empty stops the
/// only running emulator; a physical serial is refused. Needs no project.
pub(crate) fn android_stop_emulator(serial: Option<String>) -> Result<String, String> {
    android::stop_emulator(serial.as_deref())
}

/// Grabs the device's screen as a downscaled PNG data URL for the embedded
/// emulator view. `device` names a serial (empty means the only device);
/// `max_width` caps the frame width (default 360). Needs no open project.
pub(crate) fn android_mirror_frame(
    device: Option<String>,
    max_width: Option<u32>,
) -> Result<android::MirrorFrame, String> {
    android::mirror_frame(device.as_deref(), max_width)
}

/// Taps the device at the fractional point `x, y` (0..1 across the mirror
/// image). What a click on the embedded screen becomes.
pub(crate) fn android_mirror_tap(device: Option<String>, x: f32, y: f32) -> Result<String, String> {
    android::mirror_tap(device.as_deref(), x, y)
}

/// Swipes the device from one fractional point to another over
/// `duration_ms` (default 300). What a drag on the embedded screen becomes.
pub(crate) fn android_mirror_swipe(
    device: Option<String>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    duration_ms: Option<u64>,
) -> Result<String, String> {
    android::mirror_swipe(device.as_deref(), x1, y1, x2, y2, duration_ms)
}

/// Presses a named key on the device: back, home, recents, enter, delete,
/// tab, power, volume_up, volume_down, volume_mute. The embedded screen's
/// hardware buttons.
pub(crate) fn android_mirror_key(device: Option<String>, code: String) -> Result<String, String> {
    android::mirror_key(device.as_deref(), &code)
}

/// Points the SDK row at `path` (empty clears back to the default) and
/// answers the resolved dir. Needs no open project: the row is app-level.
pub(crate) fn android_set_sdk_path(path: String) -> Result<String, String> {
    Ok(android::set_sdk_path(&path)?.to_string_lossy().into_owned())
}

/// Points the NDK row at `path` (empty clears back to the pinned NDK inside
/// the SDK) and answers the resolved dir.
pub(crate) fn android_set_ndk_path(path: String) -> Result<String, String> {
    Ok(android::set_ndk_path(&path)?.to_string_lossy().into_owned())
}

/// Writes the per-project Android rows (applicationId override, version
/// code and name, release keystore plus alias). Each is optional so a caller
/// can change one row without resending the rest; an invalid id refuses the
/// edit, empty clears back to the default id from the project name. A
/// keystore row must name a file that exists; passwords are never stored
/// here, each build asks for them.
pub(crate) fn set_android_settings(
    state: &SharedState,
    app: &AppHandle,
    application_id: Option<String>,
    version_code: Option<u32>,
    version_name: Option<String>,
    keystore: Option<String>,
    key_alias: Option<String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    if s.open.is_none() {
        return Err("Open a project first.".to_string());
    }
    if let Some(id) = &application_id {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            android::validate_application_id(trimmed)?;
        }
    }
    if let Some(path) = &keystore {
        let trimmed = path.trim();
        if !trimmed.is_empty() && !std::path::Path::new(trimmed).is_file() {
            return Err(format!(
                "{trimmed} isn't a key file. Create one first, or pick the file again."
            ));
        }
    }
    let current = s.project().map(|project| project.android.clone());
    let mut next = current.clone().unwrap_or_default();
    if let Some(id) = application_id {
        next.application_id = id.trim().to_string();
    }
    if let Some(code) = version_code {
        next.version_code = code.max(1);
    }
    if let Some(name) = version_name {
        next.version_name = if name.trim().is_empty() {
            "1.0.0".to_string()
        } else {
            name.trim().to_string()
        };
    }
    if let Some(path) = keystore {
        next.keystore = path.trim().to_string();
    }
    if let Some(alias) = key_alias {
        next.key_alias = alias.trim().to_string();
    }
    if current.is_some_and(|current| current == next) {
        return Ok(());
    }
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.android = next;
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

/// Makes a release key: a new RSA keypair under `alias` in the keystore at
/// `path`, creating the file when it names nothing yet. Answers the aliases
/// the file holds afterwards. Passwords come from the args or the env (see
/// `android::STORE_PASS_ENV`); nothing is stored. Needs no open project.
pub(crate) fn android_create_keystore(
    path: String,
    alias: String,
    store_pass: Option<String>,
    key_pass: Option<String>,
    remember_passwords: bool,
) -> Result<Vec<String>, String> {
    if path.trim().is_empty() {
        return Err("Name the key file first.".to_string());
    }
    let aliases = android::create_keystore(
        std::path::Path::new(path.trim()),
        &alias,
        store_pass.as_deref(),
        key_pass.as_deref(),
    )?;
    if remember_passwords {
        // The key just proved these passwords work, so they are worth
        // keeping. A keyring that won't keep them only affects the next
        // build's typing, never the key just made.
        let _ = android::remember_signing(
            &android::AndroidSettings {
                keystore: path.trim().to_string(),
                key_alias: alias.trim().to_string(),
                ..android::AndroidSettings::default()
            },
            store_pass.as_deref(),
            key_pass.as_deref(),
        );
    }
    Ok(aliases)
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
        assets::AssetKind::Lighting => serde_json::to_string_pretty(&Lighting::default()).unwrap(),
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

/// Makes an asset - a text file, a script with the starter template, or a
/// scene when the name ends in `.blockscene`. A scene is a normal tray file:
/// its filename is the scene name, and making one loads it.
pub(crate) fn create_asset(
    state: &SharedState,
    app: &AppHandle,
    parent: String,
    name: String,
) -> Result<String, String> {
    let trimmed = name.trim().to_string();
    if project::is_scene_asset(&trimmed) {
        let mut s = lock(state)?;
        let dir = project_dir(&s)?;
        // The tray dedupes like any other file; the scene follows the file.
        let file = if trimmed.to_lowercase().ends_with(".blockscene") {
            trimmed.clone()
        } else {
            format!("{trimmed}.blockscene")
        };
        let made = assets::create_file(&dir, &parent, &file, "")?;
        // `create_file` wrote a placeholder; the save below writes the real
        // scene over it.
        push_undo(&mut s);
        let id = {
            let Some(project) = s.project_mut() else {
                return Err("No project is open".to_string());
            };
            project.add_scene_at(&made, None)?
        };
        s.selected_actor = None;
        auto_save(&s);
        sync_runtime(&mut s);
        emit(app, &s);
        let _ = id;
        return Ok(made);
    }
    let s = lock(state)?;
    let dir = project_dir(&s)?;
    let template = if assets::kind_of(&name) == assets::AssetKind::Lighting {
        let mut lighting = s
            .project()
            .ok_or("No project is open")?
            .world
            .lighting
            .clone();
        lighting.asset.clear();
        serde_json::to_string_pretty(&lighting).map_err(|e| e.to_string())?
    } else {
        asset_template(&name)
    };
    let made = assets::create_file(&dir, &parent, &name, &template)?;
    if touches_scripts(&made) {
        sync_ide(&dir);
    }
    Ok(made)
}

/// Copies files from anywhere on the machine into the project folder. A
/// `.blockscene` among them becomes a scene, named for its filename.
pub(crate) fn import_assets(
    state: &SharedState,
    app: &AppHandle,
    parent: String,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let sources: Vec<std::path::PathBuf> =
        paths.into_iter().map(std::path::PathBuf::from).collect();
    let made = assets::import(&dir, &parent, &sources)?;
    for path in &made {
        // Best-effort: the pipeline learns the file's shape at import so the
        // tray can show it and reimport tracking starts clean.
        if let Ok(report) = pipeline::inspect_asset(&dir, path) {
            let _ = pipeline::note_imported(&dir, path, &report.summary);
        }
    }
    if made.iter().any(|path| touches_scripts(path)) {
        sync_ide(&dir);
    }
    // Scenes register by filename; a file that won't parse becomes a fresh
    // scene under that name rather than failing the whole import.
    let scenes: Vec<String> = made
        .iter()
        .filter(|path| project::is_scene_asset(path))
        .cloned()
        .collect();
    if !scenes.is_empty() {
        push_undo(&mut s);
        for relative in &scenes {
            if let Err(e) = register_scene_file(&mut s, &dir, relative) {
                tracing::warn!("Couldn't register scene {relative}: {e}");
            }
        }
        s.selected_actor = None;
        auto_save(&s);
        sync_runtime(&mut s);
        emit(app, &s);
    }
    Ok(made)
}

/// Registers the `.blockscene` file at `relative` as a scene, named for its
/// filename. A file that won't parse starts a fresh scene there instead, so
/// a tray rename onto `.blockscene` never fails the edit.
fn register_scene_file(
    s: &mut AppState,
    dir: &std::path::Path,
    relative: &str,
) -> Result<String, String> {
    let full = dir.join(relative);
    let mut scene = project::read_scene_file(&full).unwrap_or_else(|_| {
        let stem = project::scene_name_for_path(relative);
        let mode = s
            .project()
            .map(|p| p.world.mode)
            .unwrap_or(blockloom_core::scene::Mode::TwoD);
        let mut fresh = blockloom_core::project::Scene::new(
            if stem.is_empty() { "Scene" } else { &stem },
            mode,
        );
        if fresh.world.input.actions.is_empty() {
            fresh.world.input = blockloom_core::input::InputConfig::starter();
        }
        fresh
    });
    let project = s.project_mut().ok_or("No project is open".to_string())?;
    if project.scene(&scene.id).is_some() || scene.id.trim().is_empty() {
        scene.id = uuid::Uuid::new_v4().simple().to_string();
        let mut remap = std::collections::HashMap::new();
        for actor in &mut scene.actors {
            let next = uuid::Uuid::new_v4().simple().to_string();
            remap.insert(actor.id.clone(), next.clone());
            actor.id = next;
        }
        for actor in &mut scene.actors {
            if let Some(parent) = actor.parent()
                && let Some(next) = remap.get(parent).cloned()
            {
                actor.components.set_parent(&next);
            }
        }
    }
    let stem = project::scene_name_for_path(relative);
    scene.name = project.unique_scene_name(if stem.is_empty() { &scene.name } else { &stem });
    scene.path = relative.to_string();
    let id = scene.id.clone();
    project.scenes.push(scene);
    project.active_scene = id.clone();
    Ok(id)
}

pub(crate) fn rename_asset(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    name: String,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let was_scene = s
        .project()
        .is_some_and(|project| project.scene_for_asset(&path).is_some());
    let dir = project_dir(&s)?;
    let moved = assets::rename(&dir, &path, &name)?;
    pipeline::note_moved(&dir, &path, &moved);
    if touches_scripts(&path) || touches_scripts(&moved) {
        sync_ide(&dir);
    }
    if was_scene {
        // The filename is the scene name: a rename renames the scene, and
        // dropping the extension un-scenes the file.
        push_undo(&mut s);
        if project::is_scene_asset(&moved) {
            let stem = project::scene_name_for_path(&moved);
            let id = s
                .project()
                .and_then(|p| p.scene_for_asset(&path))
                .map(|scene| scene.id.clone())
                .ok_or("Scene not found".to_string())?;
            if let Some(scene) = s.project_mut().and_then(|p| p.scene_mut(&id)) {
                scene.path = moved.clone();
            }
            let _renamed = {
                let Some(project) = s.project_mut() else {
                    return Err("No project is open".to_string());
                };
                project.rename_scene_to_stem(&id, &stem)?
            };
            // `rename_scene_to_stem` recomputes a name-based path in the old
            // folder; the file already moved, so keep its real path.
            if let Some(scene) = s.project_mut().and_then(|p| p.scene_mut(&id)) {
                scene.path = moved.clone();
            }
        } else {
            let id = s
                .project()
                .and_then(|p| p.scene_for_asset(&path))
                .map(|scene| scene.id.clone());
            if let Some(id) = id {
                let Some(project) = s.project_mut() else {
                    return Err("No project is open".to_string());
                };
                if project.scenes.len() <= 1 {
                    return Err("A project needs at least one scene".to_string());
                }
                project.remove_scene(&id)?;
                if s.selected_actor.is_some() {
                    s.selected_actor = None;
                }
            }
        }
        auto_save(&s);
        sync_runtime(&mut s);
        emit(app, &s);
        return Ok(moved);
    }
    // A plain file renamed onto `.blockscene` becomes a scene.
    if !was_scene && project::is_scene_asset(&moved) && s.project().is_some() {
        push_undo(&mut s);
        let dir = project_dir(&s)?;
        match register_scene_file(&mut s, &dir, &moved) {
            Ok(_) => {
                s.selected_actor = None;
                auto_save(&s);
                sync_runtime(&mut s);
                emit(app, &s);
            }
            Err(e) => {
                tracing::warn!("Couldn't register scene {moved}: {e}");
            }
        }
        return Ok(moved);
    }
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
    // Scenes under the moved file or folder follow it, keeping the index in
    // step with the tray.
    let affected: Vec<String> = s
        .project()
        .map(|project| {
            project
                .scenes
                .iter()
                .filter(|scene| {
                    let at = scene.asset_path();
                    at == path || at.starts_with(&format!("{path}/"))
                })
                .map(|scene| scene.id.clone())
                .collect()
        })
        .unwrap_or_default();
    if !affected.is_empty() {
        push_undo(&mut s);
    }
    let dir = project_dir(&s)?;
    let moved = assets::move_to(&dir, &path, &parent)?;
    pipeline::note_moved(&dir, &path, &moved);
    if touches_scripts(&path) || touches_scripts(&moved) {
        sync_ide(&dir);
    }
    if !affected.is_empty() {
        for id in &affected {
            let old = s
                .project()
                .and_then(|p| p.scene(id))
                .map(|scene| scene.asset_path())
                .unwrap_or_default();
            let new = if old == path {
                moved.clone()
            } else {
                moved.clone() + &old[path.len()..]
            };
            if let Some(scene) = s.project_mut().and_then(|p| p.scene_mut(id)) {
                scene.path = new;
            }
        }
        auto_save(&s);
        sync_runtime(&mut s);
        emit(app, &s);
        return Ok(moved);
    }
    repoint_assets(&mut s, app, &path, &moved);
    Ok(moved)
}

pub(crate) fn delete_asset(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    // Deleting a scene file deletes its scene; a folder takes the scenes
    // under it. The last scene stays.
    let condemned: Vec<String> = s
        .project()
        .map(|project| {
            project
                .scenes
                .iter()
                .filter(|scene| {
                    let at = scene.asset_path();
                    at == path || at.starts_with(&format!("{path}/"))
                })
                .map(|scene| scene.id.clone())
                .collect()
        })
        .unwrap_or_default();
    if !condemned.is_empty() {
        let remaining = s
            .project()
            .map(|p| p.scenes.len().saturating_sub(condemned.len()))
            .unwrap_or(0);
        if remaining == 0 {
            return Err("A project needs at least one scene".to_string());
        }
        push_undo(&mut s);
        if let Some(project) = s.project_mut() {
            for id in &condemned {
                let _ = project.remove_scene(id);
            }
        }
        s.selected_actor = None;
        let dir = project_dir(&s)?;
        assets::delete(&dir, &path)?;
        pipeline::note_removed(&dir, &path);
        auto_save(&s);
        sync_runtime(&mut s);
        emit(app, &s);
        return Ok(());
    }
    let dir = project_dir(&s)?;
    assets::delete(&dir, &path)?;
    pipeline::note_removed(&dir, &path);
    if touches_scripts(&path) {
        sync_ide(&dir);
    }
    Ok(())
}

/// A file's bytes as a `data:` URL - the only way a web page can show a
/// thumbnail of a file on disk.
pub(crate) fn read_asset(state: &SharedState, path: String) -> Result<String, String> {
    let s = lock(state)?;
    assets::data_url(&project_dir(&s)?, &path)
}

/// Opens the native file manager to where this asset lives: a file gets
/// spotlighted inside its parent folder, a folder opens itself.
pub(crate) fn open_asset_location(state: &SharedState, path: String) -> Result<(), String> {
    let s = lock(state)?;
    let target = assets::resolve(&project_dir(&s)?, &path)
        .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
    reveal_in_file_manager(&target)
}

// ─── Asset pipeline ────────────────────────────────────────────────────────

/// One asset through the pipeline: what it is, what import would do, and
/// whether it changed since import.
pub(crate) fn inspect_asset(
    state: &SharedState,
    path: String,
) -> Result<pipeline::PipelineReport, String> {
    let s = lock(state)?;
    pipeline::inspect_asset(&project_dir(&s)?, &path)
}

/// Every importable file with its pipeline report, for the tray's badges.
pub(crate) fn pipeline_status(
    state: &SharedState,
) -> Result<Vec<pipeline::PipelineReport>, String> {
    let s = lock(state)?;
    Ok(pipeline::scan_project(&project_dir(&s)?))
}

/// What an asset imports as: `hdr`, `volume`, `heightmap`, `ies`, `cookie`,
/// `texture`, or `auto` to follow its extension. The asset reads dirty until
/// it is reimported.
pub(crate) fn set_import_role(
    state: &SharedState,
    path: String,
    role: String,
) -> Result<pipeline::PipelineReport, String> {
    let s = lock(state)?;
    pipeline::set_role(
        &project_dir(&s)?,
        &path,
        pipeline::ImportRole::parse(&role)?,
    )
}

/// Scales an HDR file by some stops wherever it is decoded; the world picks
/// it up on the reload this sends.
pub(crate) fn set_exposure_bias(
    state: &SharedState,
    path: String,
    ev: f32,
) -> Result<pipeline::PipelineReport, String> {
    let mut s = lock(state)?;
    let report = pipeline::set_exposure_bias(&project_dir(&s)?, &path, ev)?;
    sync_runtime(&mut s);
    Ok(report)
}

/// Re-inspect files and refresh their fingerprints. Empty means everything
/// dirty; naming paths forces those even when clean.
pub(crate) fn reimport_assets(
    state: &SharedState,
    paths: Vec<String>,
) -> Result<Vec<pipeline::PipelineReport>, String> {
    let s = lock(state)?;
    pipeline::reimport(&project_dir(&s)?, &paths)
}

/// Lay images into one atlas sheet. Without `output` it is a plan the tray
/// can preview; with one it is baked into `<output>.png` plus the layout as
/// `<output>.json`, both project-relative. A build bakes its own sheet of
/// Image looks either way.
pub(crate) fn pack_atlas(
    state: &SharedState,
    paths: Vec<String>,
    max_size: Option<u32>,
    padding: Option<u32>,
    output: Option<String>,
) -> Result<pipeline::AtlasLayout, String> {
    let s = lock(state)?;
    let dir = project_dir(&s)?;
    if let Some(output) = output.filter(|output| !output.trim().is_empty()) {
        let relative = assets::normalize(&output)
            .ok_or_else(|| format!("\"{output}\" isn't a path in this project"))?;
        let stem = relative
            .strip_suffix(".png")
            .or_else(|| relative.strip_suffix(".json"))
            .unwrap_or(&relative);
        let resolve = |path: String| {
            assets::resolve(&dir, &path)
                .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))
        };
        let image = resolve(format!("{stem}.png"))?;
        let layout = resolve(format!("{stem}.json"))?;
        let atlas =
            pipeline::bake_atlas(&dir, &paths, max_size.unwrap_or(2048), padding.unwrap_or(1))?;
        pipeline::write_atlas(&atlas, &image, &layout)?;
        return Ok(atlas.layout);
    }
    let mut inputs = Vec::new();
    for path in paths {
        let relative = assets::normalize(&path)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
        let full = assets::resolve(&dir, &relative)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
        let bytes = std::fs::read(&full).map_err(|e| format!("{}: {e}", full.display()))?;
        let info = pipeline::inspect_texture(&relative, &bytes)?;
        inputs.push(pipeline::AtlasInput {
            name: relative,
            width: info.width,
            height: info.height,
        });
    }
    pipeline::pack_atlas(&inputs, max_size.unwrap_or(2048), padding.unwrap_or(1))
}

/// Writes an actor's custom effect out as a `.wesl` asset and points the
/// effect at it, so from then on the file draws the surface and editing it
/// changes what Play shows. It exports the effect's own graph, so nothing
/// on screen moves. Answers the file's path. Never overwrites a file: an
/// effect that already reads one needs a fresh `path` to export again.
pub(crate) fn export_shader(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: Option<String>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let actor = s
        .project()
        .and_then(|p| p.actor(&actor_id))
        .ok_or("Actor not found")?;
    let mut material = actor
        .components
        .material()
        .cloned()
        .ok_or("This actor has no Material component")?;
    let effect = material
        .shader
        .as_mut()
        .ok_or("Switch the material's custom effect on first")?;
    let path = match path.filter(|path| !path.trim().is_empty()) {
        Some(path) => assets::normalize(&path)
            .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?,
        None if !effect.source.is_empty() => {
            return Err(format!(
                "The effect already draws with {}; name a new file to export again",
                effect.source
            ));
        }
        None => {
            let stem = format!("assets/shaders/{}", project::folder_name(&actor.name));
            std::iter::once(format!("{stem}.wesl"))
                .chain((2..).map(|n| format!("{stem} {n}.wesl")))
                .find(|candidate| {
                    assets::resolve(&dir, candidate).is_some_and(|full| !full.exists())
                })
                .expect("an unused name always exists")
        }
    };
    let full = assets::resolve(&dir, &path)
        .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
    if full.exists() {
        return Err(format!("{path} already exists"));
    }
    let wesl = GraphEffect {
        source: String::new(),
        ..effect.clone()
    }
    .starter_graph()
    .to_wesl_asset()?;
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&full, wesl).map_err(|e| format!("{}: {e}", full.display()))?;
    effect.source = path.clone();
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor
            .components
            .insert(ActorComponent::Material { material });
    }
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(path)
}

/// Checks the `.wesl` file an actor's effect draws with, the way Play will,
/// and logs the verdict against the actor.
pub(crate) fn check_shader(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let actor = s
        .project()
        .and_then(|p| p.actor(&actor_id))
        .ok_or("Actor not found")?;
    let name = actor.name.clone();
    let source = actor
        .components
        .material()
        .and_then(|material| material.shader.as_ref())
        .map(|effect| effect.source.clone())
        .filter(|source| !source.is_empty())
        .ok_or("This actor's effect doesn't read a .wesl file")?;
    let dim3 = s.project().is_some_and(|p| p.world.mode.is_3d());
    let full = assets::resolve(&dir, &source)
        .ok_or_else(|| format!("\"{source}\" isn't a path in this project"))?;
    let verdict = std::fs::read_to_string(&full)
        .map_err(|e| format!("couldn't read {source}: {e}"))
        .and_then(|text| blockloom_core::material::check_surface_wesl(&text, dim3));
    let line = match verdict {
        Ok(()) => LogLine {
            kind: "say".to_string(),
            actor: name,
            text: format!("{source} compiles"),
        },
        Err(error) => LogLine {
            kind: "error".to_string(),
            actor: name,
            text: format!("{source} doesn't compile:\n{error}"),
        },
    };
    s.push_log(line);
    emit(app, &s);
    Ok(())
}

/// Hand the OS a path: reveal a file inside its parent in the file manager,
/// or open a directory in it. Spawned detached - the file manager is a GUI
/// app that must outlive this process.
fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    let is_dir = path.is_dir();
    let mut command = std::process::Command::new(if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    });
    if cfg!(target_os = "windows") {
        if is_dir {
            command.arg(path);
        } else {
            // One token: `/select,<path>` is Explorer's contract; the split
            // form opens the parent folder without highlighting anything.
            command.arg(format!("/select,{}", path.display()));
        }
    } else if cfg!(target_os = "macos") {
        if is_dir {
            command.arg(path);
        } else {
            // -R selects the item instead of launching whatever claims it.
            command.arg("-R").arg(path);
        }
    } else {
        // No portable "select this file" on Linux - open its parent folder.
        command.arg(path.parent().unwrap_or(path));
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the file manager: {e}"))
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
    sync_ide(&dir);
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
    sync_ide(&dir);
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
    sync_ide(&dir);
    emit(app, &s);
    Ok(())
}

/// Whether this machine can compile scripts, and what it would use. Missing is
/// a status, not an error: blocks still run, only scripts need a toolchain.
pub(crate) fn script_toolchain(
    state: &SharedState,
) -> Result<script::ide::ToolchainStatus, String> {
    drop(lock(state)?);
    Ok(script::ide::toolchain_status())
}

/// One actor's script errors pinned to their lines, for the editor to show
/// inline. `cargo check` over the analysis project when Cargo is here, else
/// one `rustc` run. Empty means it compiled, or there is nothing to compile
/// with - the run log says which.
pub(crate) fn script_diagnostics(
    state: &SharedState,
    actor_id: String,
) -> Result<Vec<script::ide::ScriptDiagnostic>, String> {
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
    drop(s);
    sync_ide(&dir);
    Ok(script::ide::diagnostics_for(&dir, &path))
}

/// Regenerates the analysis project rust-analyzer opens and answers what it
/// holds. The editor calls this implicitly on every scripted edit; this is
/// the explicit spelling for agents and for the "Open in editor" button.
pub(crate) fn sync_script_ide(state: &SharedState) -> Result<script::ide::SyncReport, String> {
    let s = lock(state)?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or("No project is open")?;
    drop(s);
    let report = script::ide::sync_ide_project(&dir)?;
    Ok(report)
}

/// Points the user's own editor at the project: syncs the analysis project,
/// then tries VS Code / Zed / the file manager, in that order. Returns the
/// folder and what opened it.
pub(crate) fn open_script_ide(state: &SharedState) -> Result<serde_json::Value, String> {
    let s = lock(state)?;
    let dir = s
        .project_dir()
        .map(Path::to_path_buf)
        .ok_or("No project is open")?;
    drop(s);
    sync_ide(&dir);
    let path = dir.to_string_lossy().into_owned();
    for (binary, editor) in [("code", "VS Code"), ("zed", "Zed")] {
        if std::process::Command::new(binary).arg(&dir).spawn().is_ok() {
            return Ok(serde_json::json!({"path": path, "openedWith": editor}));
        }
    }
    reveal_in_file_manager(&dir)?;
    Ok(serde_json::json!({"path": path, "openedWith": "file manager"}))
}

/// Pushes the edited project to an idle runtime, so a scene edit shows in the
/// game window straight away. While a run is going the edit waits for the next
/// Play - reloading mid-run would throw the world away under the user.
fn sync_runtime(s: &mut AppState) {
    s.interface_design = None;
    s.interface_layout = None;
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
    send_selection(s);
}

/// Tells the world which actor the scene view should outline.
fn send_selection(s: &mut AppState) {
    let actor = s.actor_id();
    if let Some(runtime) = s.runtime.as_mut()
        && !runtime.send(&blockloom_protocol::EditorMessage::Select { actor })
    {
        s.runtime = None;
    }
}

/// What a fresh world needs beyond the project: how the scene view edits and
/// what is selected. `false` means the link is already gone.
fn greet(s: &mut AppState) -> bool {
    let view = blockloom_protocol::EditorMessage::SceneView(s.scene_view.clone());
    let select = blockloom_protocol::EditorMessage::Select {
        actor: s.actor_id(),
    };
    match s.runtime.as_mut() {
        Some(runtime) => runtime.send(&view) && runtime.send(&select),
        None => true,
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

/// Moves the tail at and after `path` in `strand_id` into `target_id` at
/// `target_path` in one step - the Qt canvas's attach-on-drop for part of a
/// stack, with no split-then-merge round trip.
pub(crate) fn merge_tail(
    state: &SharedState,
    app: &AppHandle,
    strand_id: String,
    path: InstrPath,
    target_id: String,
    target_path: InstrPath,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match graph_mut(&mut s) {
        Some(graph) => graph.merge_tail(&strand_id, &path, &target_id, &target_path),
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
    let lists = lists_env(&s);
    let dicts = dicts_env(&s);
    let resolved = value.resolve_vars(&env);
    let resolved = resolve_list_reporters(&resolved, &lists).unwrap_or(resolved);
    let resolved = resolve_dict_reporters(&resolved, &dicts).unwrap_or(resolved);
    Ok(resolved
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
    // `~t0` holds a suspendable reporter's value while it sleeps, so no
    // project variable may take the prefix.
    if name.trim().starts_with('~') {
        return Err("Variable name can't start with \"~\"".to_string());
    }
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
    if new_name.trim().starts_with('~') {
        return Err("Variable name can't start with \"~\"".to_string());
    }
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

// ─── Input actions ─────────────────────────────────────────────────────────

pub(crate) fn create_input_action(
    state: &SharedState,
    app: &AppHandle,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match s.project_mut() {
        Some(project) => project.create_input_action(&name).map(|_| ()),
        None => Ok(()),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn rename_input_action(
    state: &SharedState,
    app: &AppHandle,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match s.project_mut() {
        Some(project) => project
            .rename_input_action(&old_name, &new_name)
            .map(|_| ()),
        None => Ok(()),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn delete_input_action(
    state: &SharedState,
    app: &AppHandle,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    if let Some(project) = s.project_mut() {
        project.remove_input_action(&name);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn add_input_binding(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    binding: String,
) -> Result<bool, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match s.project_mut() {
        Some(project) => project.world.input.add_binding(&name, &binding),
        None => Ok(false),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn remove_input_binding(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    binding: String,
) -> Result<bool, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match s.project_mut() {
        Some(project) => project.world.input.remove_binding(&name, &binding),
        None => Ok(false),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn clear_input_bindings(
    state: &SharedState,
    app: &AppHandle,
    name: String,
) -> Result<bool, String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let result = match s.project_mut() {
        Some(project) => project.world.input.clear_bindings(&name),
        None => Ok(false),
    };
    auto_save(&s);
    emit(app, &s);
    result
}

// ─── Lists ───────────────────────────────────────────────────────────────────

/// `scope` is `"global"` for a project-wide list, anything else for one
/// private to the open actor.
pub(crate) fn create_list(
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
            Some(project) => project.create_global_list(&name).map(|_| ()),
            None => Ok(()),
        }
    } else {
        match graph_mut(&mut s) {
            Some(graph) => match graph.create_list(&name) {
                Ok(trimmed) => {
                    if let Some(list) = graph.lists.iter_mut().find(|list| list.name == trimmed) {
                        list.editor_x = 36;
                        list.editor_y = 36;
                    }
                    Ok(())
                }
                Err(message) => Err(message),
            },
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn rename_list(
    state: &SharedState,
    app: &AppHandle,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.lists.iter().any(|l| l.name == old_name));
    let result = if owned_by_actor {
        match graph_mut(&mut s) {
            Some(graph) => graph.rename_list(&old_name, &new_name).map(|_| ()),
            None => Ok(()),
        }
    } else {
        match s.project_mut() {
            Some(project) => project.rename_global_list(&old_name, &new_name).map(|_| ()),
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn delete_list(
    state: &SharedState,
    app: &AppHandle,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.lists.iter().any(|l| l.name == name));
    if owned_by_actor {
        if let Some(graph) = graph_mut(&mut s) {
            graph.remove_list(&name);
        }
    } else if let Some(project) = s.project_mut() {
        project.remove_global_list(&name);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

/// Replaces a list's items, wherever it lives. `ListItem` is literal-only by
/// construction, which enforces the literal-only list contract. Not undoable,
/// the way typing into a canvas monitor isn't - the items themselves are the
/// edit.
pub(crate) fn set_list_items(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    items: Vec<ListItem>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.lists.iter().any(|l| l.name == name));
    let result = if owned_by_actor {
        match graph_mut(&mut s) {
            Some(graph) => graph.set_list_items(&name, items),
            None => Ok(()),
        }
    } else {
        match s.project_mut() {
            Some(project) => match project
                .global_lists
                .iter_mut()
                .find(|list| list.name == name)
            {
                Some(list) => {
                    list.items = items;
                    Ok(())
                }
                None => Err("List not found".to_string()),
            },
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

/// Saves whether a list's editable canvas monitor is open and where it sits.
/// A presentation preference rather than an undoable edit.
pub(crate) fn set_list_editor_state(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    visible: bool,
    x: i32,
    y: i32,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.lists.iter().any(|l| l.name == name));
    let result = if owned_by_actor {
        match graph_mut(&mut s) {
            Some(graph) => graph.set_list_editor_state(&name, visible, x, y),
            None => Ok(()),
        }
    } else {
        match s.project_mut() {
            Some(project) => match project
                .global_lists
                .iter_mut()
                .find(|list| list.name == name)
            {
                Some(list) => {
                    list.editor_visible = visible;
                    list.editor_x = x.max(0);
                    list.editor_y = y.max(0);
                    Ok(())
                }
                None => Err("List not found".to_string()),
            },
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

// ─── Dicts ───────────────────────────────────────────────────────────────────

/// `scope` is `"global"` for a project-wide dict, anything else for one
/// private to the open actor.
pub(crate) fn create_dict(
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
            Some(project) => project.create_global_dict(&name).map(|_| ()),
            None => Ok(()),
        }
    } else {
        match graph_mut(&mut s) {
            Some(graph) => match graph.create_dict(&name) {
                Ok(trimmed) => {
                    if let Some(dict) = graph.dicts.iter_mut().find(|dict| dict.name == trimmed) {
                        dict.editor_x = 36;
                        dict.editor_y = 36;
                    }
                    Ok(())
                }
                Err(message) => Err(message),
            },
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn rename_dict(
    state: &SharedState,
    app: &AppHandle,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.dicts.iter().any(|d| d.name == old_name));
    let result = if owned_by_actor {
        match graph_mut(&mut s) {
            Some(graph) => graph.rename_dict(&old_name, &new_name).map(|_| ()),
            None => Ok(()),
        }
    } else {
        match s.project_mut() {
            Some(project) => project.rename_global_dict(&old_name, &new_name).map(|_| ()),
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

pub(crate) fn delete_dict(
    state: &SharedState,
    app: &AppHandle,
    name: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.dicts.iter().any(|d| d.name == name));
    if owned_by_actor {
        if let Some(graph) = graph_mut(&mut s) {
            graph.remove_dict(&name);
        }
    } else if let Some(project) = s.project_mut() {
        project.remove_global_dict(&name);
    }
    auto_save(&s);
    emit(app, &s);
    Ok(())
}

/// Replaces a dict's entries, wherever it lives. `DictEntry` values are
/// literal-only by construction, which enforces the literal-only dict
/// contract. Not undoable, the way typing into a canvas monitor isn't - the
/// entries themselves are the edit.
pub(crate) fn set_dict_entries(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    entries: Vec<DictEntry>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.dicts.iter().any(|d| d.name == name));
    let result = if owned_by_actor {
        match graph_mut(&mut s) {
            Some(graph) => graph.set_dict_entries(&name, entries),
            None => Ok(()),
        }
    } else {
        match s.project_mut() {
            Some(project) => match project
                .global_dicts
                .iter_mut()
                .find(|dict| dict.name == name)
            {
                Some(dict) => {
                    dict.entries = entries;
                    Ok(())
                }
                None => Err("Dict not found".to_string()),
            },
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

/// Saves whether a dict's editable canvas monitor is open and where it sits.
/// A presentation preference rather than an undoable edit.
pub(crate) fn set_dict_editor_state(
    state: &SharedState,
    app: &AppHandle,
    name: String,
    visible: bool,
    x: i32,
    y: i32,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let owned_by_actor =
        graph_mut(&mut s).is_some_and(|graph| graph.dicts.iter().any(|d| d.name == name));
    let result = if owned_by_actor {
        match graph_mut(&mut s) {
            Some(graph) => graph.set_dict_editor_state(&name, visible, x, y),
            None => Ok(()),
        }
    } else {
        match s.project_mut() {
            Some(project) => match project
                .global_dicts
                .iter_mut()
                .find(|dict| dict.name == name)
            {
                Some(dict) => {
                    dict.editor_visible = visible;
                    dict.editor_x = x.max(0);
                    dict.editor_y = y.max(0);
                    Ok(())
                }
                None => Err("Dict not found".to_string()),
            },
            None => Ok(()),
        }
    };
    auto_save(&s);
    emit(app, &s);
    result
}

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
        blockloom_core::cloud_layers::restore_painted(&open.dir, &open.project.world.cloud_layers);
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

/// Sends a temporary UI document through the existing idle world transport.
pub(crate) fn preview_interface(
    backend: &Backend,
    state: &SharedState,
    app: &AppHandle,
    design: Option<blockloom_protocol::InterfaceDesign>,
) -> Result<(), String> {
    {
        let s = lock(state)?;
        if s.running {
            return Err("Stop the game before previewing the interface".into());
        }
        if let Some(design) = &design {
            design.validate()?;
            if let Some(dir) = s.project_dir() {
                design.document.with_stylesheets(dir)?;
            }
            if s.interface_design.as_ref().is_some_and(|old| {
                (design.generation, design.revision) <= (old.generation, old.revision)
            }) {
                return Err("Stale interface design revision or viewport generation".into());
            }
        }
    }
    open_world(backend, state, app)?;
    let mut s = lock(state)?;
    let message = blockloom_protocol::EditorMessage::InterfaceDesign {
        design: design.clone(),
    };
    if !s
        .runtime
        .as_mut()
        .is_some_and(|runtime| runtime.send(&message))
    {
        return Err("Lost the connection to the game runtime".into());
    }
    s.interface_design = design;
    s.interface_layout = None;
    Ok(())
}

pub(crate) fn interface_layout(
    state: &SharedState,
) -> Result<Option<blockloom_protocol::InterfaceLayout>, String> {
    Ok(lock(state)?.interface_layout.clone())
}

/// Saves the designer document as one undoable edit.
pub(crate) fn set_interface(
    state: &SharedState,
    app: &AppHandle,
    document: blockloom_core::ui::UiDocument,
) -> Result<(), String> {
    document.validate()?;
    let mut s = lock(state)?;
    if s.project().is_none() {
        return Err("No project is open".into());
    }
    push_undo(&mut s);
    s.project_mut().unwrap().world.interface = document;
    auto_save(&s);
    sync_runtime(&mut s);
    emit(app, &s);
    Ok(())
}

pub(crate) fn save_interface_asset(state: &SharedState, name: String) -> Result<String, String> {
    let s = lock(state)?;
    let document = &s.project().ok_or("No project is open")?.world.interface;
    let text = serde_json::to_string_pretty(document).map_err(|e| e.to_string())?;
    assets::create_file(&project_dir(&s)?, "assets/ui", &name, &text)
}
pub(crate) fn load_interface_asset(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let document = {
        let s = lock(state)?;
        let full =
            assets::resolve(&project_dir(&s)?, &path).ok_or("Invalid interface asset path")?;
        let text = std::fs::read_to_string(full).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("Invalid interface: {e}"))?
    };
    set_interface(state, app, document)
}

// ─── Tilemaps ──────────────────────────────────────────────────────────────

fn tilemap_of(s: &AppState, actor_id: &str) -> Result<blockloom_core::material::Tilemap, String> {
    match s
        .project()
        .ok_or("No project is open")?
        .actor(actor_id)
        .ok_or("Actor not found")?
        .visual()
    {
        Some(Visual::Tilemap { tilemap }) => Ok(tilemap.clone()),
        _ => Err("That actor's look isn't a tilemap".to_string()),
    }
}

fn store_tilemap(s: &mut AppState, actor_id: &str, mut tilemap: blockloom_core::material::Tilemap) {
    tilemap.normalize();
    push_undo(s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(actor_id)) {
        actor.components.set_visual(Visual::Tilemap { tilemap });
    }
    auto_save(s);
    sync_runtime(s);
}

/// Runs a tile stroke's segments on a map's saved cells, as one undo step.
/// The scene view has already drawn it, from the same brush code.
pub(crate) fn tile_stroke(
    s: &mut AppState,
    actor_id: &str,
    brush: &blockloom_core::tilemap::TileBrush,
    segments: &[[i32; 4]],
) -> Result<usize, String> {
    if s.running {
        return Err("Stop the game to paint tiles".to_string());
    }
    let mut map = tilemap_of(s, actor_id)?;
    let mut changed = std::collections::HashSet::new();
    for segment in segments {
        changed.extend(map.apply_brush(brush, (segment[0], segment[1]), (segment[2], segment[3])));
    }
    if !changed.is_empty() {
        store_tilemap(s, actor_id, map);
    }
    Ok(changed.len())
}

/// [`tile_stroke`] for the shell and the frontend: how many cells changed.
pub(crate) fn paint_tiles(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    brush: blockloom_core::tilemap::TileBrush,
    segments: Vec<[i32; 4]>,
) -> Result<usize, String> {
    let mut s = lock(state)?;
    let changed = tile_stroke(&mut s, &actor_id, &brush, &segments)?;
    emit(app, &s);
    Ok(changed)
}

/// A path relative to `base` (a folder in the project), `..` allowed as
/// long as it stays inside the project.
fn join_in_project(base: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in relative.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    assets::normalize(&parts.join("/"))
}

/// Takes a Tiled JSON tileset (`.tsj`/`.json`) onto a tilemap: its image,
/// tile size, collision and passable tiles, animations, region tiles and
/// edge or mixed wang sets. Answers what didn't come across.
pub(crate) fn import_tileset(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: String,
) -> Result<Vec<String>, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let full = assets::resolve(&dir, &path).ok_or("Invalid tileset path")?;
    let text = std::fs::read_to_string(&full).map_err(|e| format!("Couldn't read {path}: {e}"))?;
    let mut import = blockloom_core::tilemap::import_tiled_tileset(&text)?;
    let folder = assets::normalize(&path)
        .and_then(|p| p.rsplit_once('/').map(|(folder, _)| folder.to_string()))
        .unwrap_or_default();
    import.image = join_in_project(&folder, &import.image).ok_or_else(|| {
        format!(
            "The tileset's image \"{}\" is outside the project",
            import.image
        )
    })?;
    let mut map = tilemap_of(&s, &actor_id)?;
    map.apply_import(&import);
    store_tilemap(&mut s, &actor_id, map);
    emit(app, &s);
    Ok(import.skipped)
}

/// What a tilemap is made of, for the inspector's stats line.
pub(crate) fn tilemap_stats(
    state: &SharedState,
    actor_id: String,
) -> Result<blockloom_core::tilemap::TileStats, String> {
    let s = lock(state)?;
    Ok(tilemap_of(&s, &actor_id)?.stats())
}

/// Adds an autotile set laid out as a strip of consecutive sheet cells from
/// `first`: 16 for an edge set, 47 for a blob set.
pub(crate) fn add_autotile(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    name: String,
    mode: blockloom_core::tilemap::AutotileMode,
    first: i32,
) -> Result<(), String> {
    use blockloom_core::tilemap::{AutotileMode, AutotileSet};
    let mut s = lock(state)?;
    let mut map = tilemap_of(&s, &actor_id)?;
    let name = name.trim();
    if name.is_empty() {
        return Err("An autotile set needs a name".to_string());
    }
    let set = match mode {
        AutotileMode::Edge => AutotileSet::edge_strip(name, first.max(0)),
        AutotileMode::Blob => AutotileSet::blob_strip(name, first.max(0)),
    };
    let cells = (map.sheet_columns * map.sheet_rows) as i32;
    let last = set.rules.iter().map(|rule| rule.tile).max().unwrap_or(0);
    if last >= cells {
        return Err(format!(
            "That set needs tiles {} to {last}, but the sheet only has {cells}",
            first.max(0)
        ));
    }
    map.autotiles
        .retain(|other| !other.name.eq_ignore_ascii_case(name));
    map.autotiles.push(set);
    store_tilemap(&mut s, &actor_id, map);
    emit(app, &s);
    Ok(())
}

// ─── Terrain ───────────────────────────────────────────────────────────────

fn terrain_of(
    s: &AppState,
    actor_id: &str,
) -> Result<blockloom_core::terrain::TerrainSpec, String> {
    s.project()
        .ok_or("No project is open")?
        .actor(actor_id)
        .ok_or("Actor not found")?
        .components
        .terrain()
        .cloned()
        .ok_or_else(|| "That actor has no Terrain component".to_string())
}

fn store_terrain(s: &mut AppState, actor_id: &str, terrain: blockloom_core::terrain::TerrainSpec) {
    push_undo(s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(actor_id)) {
        actor.components.insert(ActorComponent::Terrain { terrain });
    }
    auto_save(s);
    sync_runtime(s);
}

/// Applies a brush stroke to a terrain's saved grids, as one undo step. The
/// scene view has already drawn it, from the same functions.
pub(crate) fn terrain_stroke(
    s: &mut AppState,
    actor_id: &str,
    mut stroke: blockloom_core::terrain::sculpt::Stroke,
) -> Result<(), String> {
    use blockloom_core::terrain::{sculpt, store};
    if s.running {
        return Err("Stop the game to edit terrain".to_string());
    }
    let dir = s.project_dir().ok_or("No project is open")?.to_path_buf();
    let mut spec = terrain_of(s, actor_id)?;
    let shape = spec.shape();
    let mut grids = sculpt::Editable::load(Some(&dir), &spec);
    sculpt::settle_level(&mut stroke, &grids.heights, &shape);
    if grids.apply(&shape, &stroke).is_none() {
        return Ok(());
    }
    let target = stroke.brush.target;
    let grid = grids
        .grid(target)
        .ok_or("That brush has nothing to paint")?;
    let name = store::save(&dir, &grid)?;
    sculpt::set_target(&mut spec, target, name);
    store_terrain(s, actor_id, spec);
    Ok(())
}

/// [`terrain_stroke`] for the shell and the frontend.
pub(crate) fn paint_terrain(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    stroke: blockloom_core::terrain::sculpt::Stroke,
) -> Result<(), String> {
    let mut s = lock(state)?;
    terrain_stroke(&mut s, &actor_id, stroke)?;
    emit(app, &s);
    Ok(())
}

/// Replaces a terrain's heights with a heightmap asset (16-bit PNG, RAW
/// `.r16`/`.r32`, or any image marked as a heightmap), stretched to the
/// terrain's resolution.
pub(crate) fn import_terrain_heightmap(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: String,
) -> Result<(), String> {
    use blockloom_core::terrain::{Heightfield, store};
    let mut s = lock(state)?;
    let dir = s.project_dir().ok_or("No project is open")?.to_path_buf();
    let mut spec = terrain_of(&s, &actor_id)?;
    let map = pipeline::load_heightmap(&dir, &path)?;
    let field = Heightfield::from_heightmap(&map, spec.resolution);
    spec.heights = store::save(&dir, &store::Grid::from_heights(&field))?;
    store_terrain(&mut s, &actor_id, spec);
    emit(app, &s);
    Ok(())
}

/// Runs an erosion filter over a terrain's heights, as one undo step.
pub(crate) fn erode_terrain(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    erosion: blockloom_core::terrain::sculpt::Erosion,
) -> Result<(), String> {
    use blockloom_core::terrain::store;
    let mut s = lock(state)?;
    let dir = s.project_dir().ok_or("No project is open")?.to_path_buf();
    let mut spec = terrain_of(&s, &actor_id)?;
    let mut field = store::heights_for(Some(&dir), &spec);
    erosion.apply(&mut field, &spec.shape());
    spec.heights = store::save(&dir, &store::Grid::from_heights(&field))?;
    store_terrain(&mut s, &actor_id, spec);
    // The preview, if any, is now the document.
    if let Some(runtime) = s.runtime.as_mut() {
        runtime.send(&blockloom_protocol::EditorMessage::PreviewErosion {
            actor: actor_id,
            erosion: None,
        });
    }
    emit(app, &s);
    Ok(())
}

/// Shows an erosion filter on a terrain in the Game view without saving
/// it; no filter puts the saved ground back.
pub(crate) fn preview_terrain_erosion(
    state: &SharedState,
    actor_id: String,
    erosion: Option<blockloom_core::terrain::sculpt::Erosion>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    terrain_of(&s, &actor_id)?;
    let Some(runtime) = s.runtime.as_mut() else {
        return Err("Open the Game view to preview erosion".to_string());
    };
    if !runtime.send(&blockloom_protocol::EditorMessage::PreviewErosion {
        actor: actor_id,
        erosion,
    }) {
        s.runtime = None;
        return Err("The game world has stopped".to_string());
    }
    Ok(())
}
