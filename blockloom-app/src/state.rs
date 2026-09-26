//! Everything the editor holds, and the one snapshot it hands the frontend.

use blockloom_core::library::ProjectEntry;
use blockloom_core::project::Project;
use blockloom_core::sync::LockInfo;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) use blockstitch_core::editor::{
    EditSession, History, InstrPath, ValueBuffers, ValueLocation,
};

pub(crate) type SharedState = Arc<Mutex<AppState>>;

/// How many undo steps the editor keeps.
pub(crate) const UNDO_STACK_LIMIT: usize = 50;

/// How many log lines are kept from a run.
const LOG_LIMIT: usize = 200;

/// The project the editor has open, and the folder it came from.
pub(crate) struct OpenProject {
    pub(crate) project: Project,
    pub(crate) dir: PathBuf,
    /// Revision this backend last wrote or loaded. The folder's counter moves
    /// on every save from any side, so a smaller number here than on disk
    /// means another backend saved since - time to reload.
    pub(crate) revision: AtomicU64,
    /// True when in-memory edits never reached disk because a save failed.
    /// Set under a shared borrow too, since every edit auto-saves.
    pub(crate) dirty: AtomicBool,
    /// Whether this backend holds the folder's owner lock.
    pub(crate) owns_lock: bool,
    /// Opened while a live owner held the folder: this copy shares the files
    /// and reloads their saves, rather than forking a silent second truth.
    pub(crate) attached: bool,
    /// Last heartbeat write, so the lock file is touched at most every few
    /// seconds no matter how chatty the command stream is.
    pub(crate) touched: AtomicU64,
}

impl OpenProject {
    pub(crate) fn new(
        project: Project,
        dir: PathBuf,
        revision: u64,
        owns_lock: bool,
        attached: bool,
    ) -> Self {
        Self {
            project,
            dir,
            revision: AtomicU64::new(revision),
            dirty: AtomicBool::new(false),
            owns_lock,
            attached,
            touched: AtomicU64::new(0),
        }
    }

    pub(crate) fn loaded_revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }
}

pub(crate) struct AppState {
    /// What the Dashboard lists, most recently opened first.
    pub(crate) library: Vec<ProjectEntry>,
    /// This backend instance, naming its owner lock and heartbeats.
    pub(crate) session_id: String,
    /// The open project, or `None` while the Dashboard is showing.
    pub(crate) open: Option<OpenProject>,
    /// Actor whose canvas the editor is showing.
    pub(crate) selected_actor: Option<String>,
    /// Undo/redo over whole-project snapshots: an edit can touch a canvas, an
    /// actor's placement or the world, and all three belong to one step.
    pub(crate) history: History<Project>,
    pub(crate) invalid_field_buffers: ValueBuffers,
    /// The game world's process, while one is up.
    pub(crate) runtime: Option<crate::runtime::RuntimeHandle>,
    pub(crate) running: bool,
    pub(crate) paused: bool,
    /// The last status the runtime reported.
    pub(crate) status: Option<blockloom_protocol::Status>,
    pub(crate) log: Vec<LogLine>,
    /// Lines ever pushed, so a frontend can tell which ones it hasn't seen.
    pub(crate) log_total: u64,
    /// Whether the embedded preview viewport wants the sidecar stream.
    pub(crate) preview_enabled: bool,
    /// Hides the runtime's OS window while the stream runs.
    pub(crate) preview_headless: bool,
    /// The sidecar's loopback port, while it is serving.
    pub(crate) preview_port: Option<u16>,
    /// The embedded world asked for the pointer locked.
    pub(crate) pointer_locked: bool,
    /// What ray tracing can do in the open world, and is doing.
    pub(crate) ray_tracing: Option<blockloom_protocol::RayTracingStatus>,
    /// Runs the game world inside this process, when the host supplies one.
    pub(crate) embedded: Option<Arc<dyn crate::runtime::EmbeddedRuntime>>,
    /// How the scene view edits, re-sent to every world that comes up.
    pub(crate) scene_view: blockloom_protocol::SceneView,
}

impl AppState {
    pub(crate) fn project(&self) -> Option<&Project> {
        self.open.as_ref().map(|open| &open.project)
    }

    pub(crate) fn project_mut(&mut self) -> Option<&mut Project> {
        self.open.as_mut().map(|open| &mut open.project)
    }

    /// The open project's folder.
    pub(crate) fn project_dir(&self) -> Option<&std::path::Path> {
        self.open.as_ref().map(|open| open.dir.as_path())
    }

    /// The actor whose canvas is open, falling back to the first one so the
    /// editor is never left with nothing to show.
    pub(crate) fn actor_id(&self) -> Option<String> {
        let project = self.project()?;
        match &self.selected_actor {
            Some(id) if project.actor(id).is_some() => Some(id.clone()),
            _ => project.actors.first().map(|actor| actor.id.clone()),
        }
    }

    pub(crate) fn push_log(&mut self, line: LogLine) {
        self.log.push(line);
        self.log_total += 1;
        let overflow = self.log.len().saturating_sub(LOG_LIMIT);
        self.log.drain(..overflow);
    }
}

/// One line of the editor's run log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogLine {
    /// `"say"` or `"error"`.
    pub(crate) kind: String,
    /// Actor name, resolved when the line was made.
    pub(crate) actor: String,
    pub(crate) text: String,
}

/// The run log as [`crate::Event::Log`] carries it.
#[derive(Serialize)]
pub(crate) struct LogDto<'a> {
    pub(crate) total: u64,
    pub(crate) lines: &'a [LogLine],
}

// ─── The snapshot the frontend gets ────────────────────────────────────────

#[derive(Serialize, Clone)]
pub(crate) struct StateDto {
    /// Every project the Dashboard offers to open.
    pub(crate) library: Vec<ProjectEntryDto>,
    /// Where the New Project dialog points by default.
    pub(crate) default_project_location: String,
    /// Where the Export dialog points by default.
    pub(crate) default_export_location: String,
    /// Where the Build dialog puts games by default.
    pub(crate) default_build_location: String,
    /// The open project's folder, for the editor's title and Reveal.
    pub(crate) project_path: Option<String>,
    /// The open project, with every instruction flattened into the shape
    /// blockstitch's canvas reads (see `blockloom_core::wire`).
    pub(crate) project: Option<serde_json::Value>,
    pub(crate) selected_actor: Option<String>,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
    pub(crate) invalid_field_buffers: Vec<InvalidFieldDto>,
    pub(crate) running: bool,
    pub(crate) paused: bool,
    pub(crate) status: Option<blockloom_protocol::Status>,
    pub(crate) log: Vec<LogLine>,
    pub(crate) log_total: u64,
    /// False when no runtime binary sits next to this one, which is the one
    /// install mistake that would otherwise look like "Play does nothing".
    pub(crate) runtime_available: bool,
    /// Whether a game window is open at all - it outlives a run, so Play is
    /// instant the second time.
    pub(crate) runtime_open: bool,
    /// Whether the embedded preview viewport wants the sidecar stream.
    pub(crate) preview_enabled: bool,
    /// Hides the runtime's OS window while the stream runs.
    pub(crate) preview_headless: bool,
    /// The sidecar's loopback port, while it is serving. The viewport reads
    /// `http://127.0.0.1:{port}/preview.mjpg` directly.
    pub(crate) preview_port: Option<u16>,
    /// The size a game draws at, which the Game view scales to fit.
    pub(crate) game_size: (u32, u32),
    /// The world runs inside the editor and draws into its Game view, so
    /// there is no window, sidecar or headless mode to offer.
    pub(crate) runtime_embedded: bool,
    /// The Game view should hold the pointer while it has the keyboard.
    pub(crate) pointer_locked: bool,
    /// What ray tracing can do in the open world, and is doing. Absent until
    /// a 3D world has come up.
    pub(crate) ray_tracing: Option<blockloom_protocol::RayTracingStatus>,
    /// How this copy relates to the project folder on disk, for agents and
    /// the editor to tell a stale copy from a live one.
    pub(crate) sync: SyncDto,
}

/// Where the open project stands against its folder: revisions, lock owner,
/// and whether this backend attached to someone else's folder.
#[derive(Serialize, Clone)]
pub(crate) struct SyncDto {
    /// Revision this backend last wrote or loaded.
    pub(crate) revision: u64,
    /// Revision the folder is at now.
    pub(crate) disk_revision: u64,
    /// True when another side saved since this backend last wrote or loaded.
    pub(crate) stale: bool,
    /// True when in-memory edits never reached disk because a save failed.
    pub(crate) dirty: bool,
    /// Opened while a live owner held the folder; shares the files.
    pub(crate) attached: bool,
    /// Holds the folder's owner lock.
    pub(crate) owns_lock: bool,
    /// This backend instance.
    pub(crate) session: String,
    /// Who owns the folder now, if anyone's lock is on it.
    pub(crate) owner: Option<LockInfo>,
}

/// One Dashboard card.
#[derive(Serialize, Clone)]
pub(crate) struct ProjectEntryDto {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) mode: blockloom_core::scene::Mode,
    pub(crate) opened_at: u64,
}

/// A numeric field holding text that doesn't parse yet - the frontend keeps
/// showing it rather than snapping the value back mid-edit.
#[derive(Serialize, Clone)]
pub(crate) struct InvalidFieldDto {
    pub(crate) location: ValueLocation,
    pub(crate) text: String,
}

pub(crate) fn state_dto(s: &AppState) -> StateDto {
    let project = s.project().and_then(|project| {
        blockloom_core::wire::to_wire(project)
            .map_err(|e| tracing::warn!("Couldn't serialize the project: {e}"))
            .ok()
    });
    StateDto {
        library: s
            .library
            .iter()
            .map(|entry| ProjectEntryDto {
                path: entry.path.to_string_lossy().into_owned(),
                name: entry.name.clone(),
                mode: entry.mode,
                opened_at: entry.opened_at,
            })
            .collect(),
        default_project_location: blockloom_core::project::default_projects_dir()
            .to_string_lossy()
            .into_owned(),
        default_export_location: blockloom_core::project::default_exports_dir()
            .to_string_lossy()
            .into_owned(),
        default_build_location: blockloom_core::project::default_builds_dir()
            .to_string_lossy()
            .into_owned(),
        project_path: s
            .project_dir()
            .map(|dir| dir.to_string_lossy().into_owned()),
        project,
        selected_actor: s.actor_id(),
        can_undo: s.history.can_undo(),
        can_redo: s.history.can_redo(),
        invalid_field_buffers: s
            .invalid_field_buffers
            .iter()
            .map(|(location, text)| InvalidFieldDto {
                location: location.clone(),
                text: text.clone(),
            })
            .collect(),
        running: s.running,
        paused: s.paused,
        status: s.status.clone(),
        log: s.log.clone(),
        log_total: s.log_total,
        runtime_available: s.embedded.is_some() || blockloom_protocol::runtime_path().exists(),
        runtime_open: s.runtime.is_some(),
        preview_enabled: s.preview_enabled,
        preview_headless: s.preview_headless,
        preview_port: s.preview_port,
        game_size: blockloom_protocol::GAME_SIZE,
        runtime_embedded: s.embedded.is_some(),
        pointer_locked: s.pointer_locked && s.running && s.runtime.is_some(),
        ray_tracing: s.runtime.as_ref().and(s.ray_tracing.clone()),
        sync: sync_dto(s),
    }
}

/// The open folder's sync standing: this copy's revision against the disk's,
// plus who owns the folder. Two tiny file reads; cheap enough to ride every
// snapshot so agents can poll `sync.disk_revision` instead of the document.
pub(crate) fn sync_dto(s: &AppState) -> SyncDto {
    let Some(open) = &s.open else {
        return SyncDto {
            revision: 0,
            disk_revision: 0,
            stale: false,
            dirty: false,
            attached: false,
            owns_lock: false,
            session: s.session_id.clone(),
            owner: None,
        };
    };
    let revision = open.loaded_revision();
    let disk_revision = blockloom_core::sync::read_revision(&open.dir);
    SyncDto {
        revision,
        disk_revision,
        stale: disk_revision > revision,
        dirty: open.dirty.load(Ordering::SeqCst),
        attached: open.attached,
        owns_lock: open.owns_lock,
        session: s.session_id.clone(),
        owner: blockloom_core::sync::read_lock(&open.dir),
    }
}
