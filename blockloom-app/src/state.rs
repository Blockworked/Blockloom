//! Everything the editor holds, and the one snapshot it hands the frontend.

use blockloom_core::library::ProjectEntry;
use blockloom_core::project::Project;
use blockloom_core::sync::LockInfo;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) use blockstitch_core::editor::{EditSession, InstrPath, ValueBuffers, ValueLocation};

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
    /// What the project's locked plugins contribute, loaded when it opens
    /// and again after every package change.
    pub(crate) plugins: blockloom_plugin_host::active::ActivePlugins,
    pub(crate) modules: crate::commands::plugins::Modules,
    pub(crate) script_statuses: std::collections::BTreeMap<String, ScriptStatus>,
    /// Undo, and what the disk and the frontend haven't seen yet.
    pub(crate) tracker: crate::history::Tracker,
    /// When the pending edits are saved, and when the first of them landed.
    pub(crate) save_due: Option<(std::time::Instant, std::time::Instant)>,
}

impl OpenProject {
    pub(crate) fn new(
        project: Project,
        dir: PathBuf,
        revision: u64,
        owns_lock: bool,
        attached: bool,
    ) -> Self {
        let plugins = crate::commands::plugins::load_active(&dir);
        let mut tracker = crate::history::Tracker::default();
        tracker.reset(&project);
        Self {
            project,
            dir,
            revision: AtomicU64::new(revision),
            dirty: AtomicBool::new(false),
            owns_lock,
            attached,
            touched: AtomicU64::new(0),
            plugins,
            modules: Default::default(),
            script_statuses: Default::default(),
            tracker,
            save_due: None,
        }
    }

    pub(crate) fn loaded_revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }
}

/// The latest native build or load result for one script path.
#[derive(Clone, Serialize)]
pub(crate) struct ScriptStatus {
    pub(crate) stage: String,
    pub(crate) error: Option<String>,
}

impl AppState {
    pub(crate) fn record_script_status(&mut self, path: &str, stage: &str, error: Option<String>) {
        if let Some(open) = &mut self.open {
            open.script_statuses.insert(
                path.to_string(),
                ScriptStatus {
                    stage: stage.to_string(),
                    error,
                },
            );
        }
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
    pub(crate) invalid_field_buffers: ValueBuffers,
    /// The game world's process, while one is up.
    pub(crate) runtime: Option<crate::runtime::RuntimeHandle>,
    pub(crate) running: bool,
    pub(crate) paused: bool,
    /// The last status the runtime reported.
    pub(crate) status: Option<blockloom_protocol::Status>,
    pub(crate) lan_session: Option<blockloom_protocol::LanSessionStatus>,
    pub(crate) interface_edit: Option<InterfaceEditTransaction>,
    pub(crate) interface_design: Option<blockloom_protocol::InterfaceDesign>,
    pub(crate) interface_layout: Option<blockloom_protocol::InterfaceLayout>,
    pub(crate) log: Vec<LogLine>,
    /// Lines ever pushed, repeats included, so a frontend can tell the log
    /// moved. Each kept line's `id` is the total it was first pushed at.
    pub(crate) log_total: u64,
    /// When the runtime's lines last went to the frontend, and whether some
    /// are waiting: a chatty world sends the log a few times a second, not
    /// once a line.
    pub(crate) log_sent_at: Option<std::time::Instant>,
    pub(crate) log_pending: bool,
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
    /// The plugins' diagnostics the world last reported.
    pub(crate) plugin_diagnostics: serde_json::Value,
    /// The `choices` lists each previewed plugin last answered.
    pub(crate) plugin_choices: std::collections::BTreeMap<String, serde_json::Value>,
    /// Runs the game world inside this process, when the host supplies one.
    pub(crate) embedded: Option<Arc<dyn crate::runtime::EmbeddedRuntime>>,
    /// How the scene view edits, re-sent to every world that comes up.
    pub(crate) scene_view: blockloom_protocol::SceneView,
    /// What the Tiles tool's pick last read, for the tile palette to take up.
    pub(crate) picked_tile: Option<PickedTile>,
    /// What the window was last sent, so the next message is a patch.
    pub(crate) publisher: crate::stream::Publisher,
    /// How long edits wait for a quiet spell before they are saved. None
    /// saves every edit at once (the shell, tests).
    pub(crate) save_delay: Option<std::time::Duration>,
}

/// One pick of the Tiles tool. `serial` counts picks, so picking the same
/// tile twice still reads as news.
#[derive(Serialize, Clone)]
pub(crate) struct PickedTile {
    pub(crate) actor: String,
    pub(crate) tile: i32,
    pub(crate) serial: u64,
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
            Some(id) if id.is_empty() => None,
            Some(id) if project.actor(id).is_some() => Some(id.clone()),
            _ => project.actors.first().map(|actor| actor.id.clone()),
        }
    }

    /// Ends the run of keystrokes undo is coalescing, if any.
    pub(crate) fn end_session(&mut self) {
        if let Some(open) = &mut self.open {
            open.tracker.history.end_session();
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.tracker.history.can_undo())
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.tracker.history.can_redo())
    }

    /// Adds a line, or counts it on the last one when it says the same thing
    /// again, so a `say` in a loop is one line that ticks up.
    pub(crate) fn push_log(&mut self, mut line: LogLine) {
        self.log_total += 1;
        if line.time.is_none() && self.running {
            line.time = self.status.as_ref().map(|status| status.time);
        }
        fold_into(&mut self.log, line, self.log_total);
    }
}

fn fold_into(log: &mut Vec<LogLine>, mut line: LogLine, id: u64) {
    if let Some(last) = log.last_mut()
        && last.repeats(&line)
    {
        last.count += 1;
        last.time = line.time;
        return;
    }
    line.id = id;
    line.count = 1;
    log.push(line);
    let overflow = log.len().saturating_sub(LOG_LIMIT);
    log.drain(..overflow);
}

/// One line of the editor's run log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogLine {
    /// `"say"` or `"error"`.
    pub(crate) kind: String,
    /// `"info"` for a `say`, `"error"` for an error. Kept beside `kind` so
    /// the log can grow severities without renaming what QML already reads.
    #[serde(default = "LogLine::info_level")]
    pub(crate) level: String,
    /// Actor name, resolved when the line was made.
    pub(crate) actor: String,
    pub(crate) text: String,
    /// Script file this line is about, when one names it. The log offers it
    /// back to the editor, so a failure lands on its file and line.
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) line: Option<u32>,
    /// Stable for as long as the line is kept: the log total it arrived at.
    #[serde(default)]
    pub(crate) id: u64,
    /// How many times in a row it was said.
    #[serde(default = "LogLine::one")]
    pub(crate) count: u32,
    /// Seconds into the run of its latest repeat, while a world was running.
    #[serde(default)]
    pub(crate) time: Option<f64>,
}

impl LogLine {
    fn info_level() -> String {
        "info".to_string()
    }

    fn one() -> u32 {
        1
    }

    fn repeats(&self, next: &LogLine) -> bool {
        self.kind == next.kind
            && self.level == next.level
            && self.actor == next.actor
            && self.text == next.text
            && self.path == next.path
            && self.line == next.line
    }

    fn new(kind: &str, level: &str, actor: String, text: String) -> Self {
        LogLine {
            kind: kind.to_string(),
            level: level.to_string(),
            actor,
            text,
            path: None,
            line: None,
            id: 0,
            count: 1,
            time: None,
        }
    }

    pub(crate) fn say(actor: String, text: String) -> Self {
        LogLine::new("say", "info", actor, text)
    }

    pub(crate) fn error(actor: String, text: String) -> Self {
        LogLine::new("error", "error", actor, text)
    }

    /// A line with a caller-chosen kind (plugin log levels, the frontend's
    /// own pushes). Anything but `"error"` reads as info.
    pub(crate) fn with_kind(kind: String, actor: String, text: String) -> Self {
        let level = if kind == "error" { "error" } else { "info" };
        LogLine::new(&kind, level, actor, text)
    }

    /// An error about one script file: carries its path and the first
    /// `path:line` the compiler or runtime message names, so the log can
    /// offer the failure back to the editor.
    pub(crate) fn script_error(actor: String, path: &str, text: String) -> Self {
        let line = first_error_line(&text, path);
        LogLine {
            path: Some(path.to_string()),
            line,
            ..LogLine::new("error", "error", actor, text)
        }
    }
}

/// The first line number named as `path:line` (or `path:line:col`) in `text`,
/// as rustc writes its spans. `None` when the message names none.
fn first_error_line(text: &str, path: &str) -> Option<u32> {
    for line in text.lines() {
        let mut rest = line;
        while let Some(found) = rest.find(path) {
            rest = &rest[found + path.len()..];
            let digits: String = rest
                .strip_prefix(':')
                .unwrap_or("")
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(number) = digits.parse::<u32>()
                && number > 0
            {
                return Some(number);
            }
        }
    }
    None
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
    pub(crate) script_statuses: std::collections::BTreeMap<String, ScriptStatus>,
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
    pub(crate) lan_session: Option<blockloom_protocol::LanSessionStatus>,
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
    /// The Tiles tool's last pick.
    pub(crate) picked_tile: Option<PickedTile>,
    /// Installed plugins, the ones that failed to load and the records that
    /// need attention. Null for a project that has never used a plugin.
    pub(crate) plugins: serde_json::Value,
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
#[derive(Serialize, Clone, PartialEq)]
pub(crate) struct ProjectEntryDto {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) mode: blockloom_core::scene::Mode,
    pub(crate) opened_at: u64,
    pub(crate) available: bool,
    pub(crate) error: Option<String>,
    /// The installation the Hub bound this project to, if any.
    pub(crate) installation: Option<String>,
    /// That installation as a person would name it.
    pub(crate) installation_label: Option<String>,
    /// The Hub set this project to an editor other than this one.
    pub(crate) wrong_editor: bool,
    /// Another editor holds this project's folder right now.
    pub(crate) in_use: bool,
}

/// A numeric field holding text that doesn't parse yet - the frontend keeps
/// showing it rather than snapping the value back mid-edit.
#[derive(Serialize, Clone)]
pub(crate) struct InvalidFieldDto {
    pub(crate) location: ValueLocation,
    pub(crate) text: String,
}

/// The Dashboard's cards, with what the Hub and other editors say about each.
pub(crate) fn library_dtos(s: &AppState) -> Vec<ProjectEntryDto> {
    s.library
        .iter()
        .map(|entry| ProjectEntryDto {
            path: entry.path.to_string_lossy().into_owned(),
            name: entry.name.clone(),
            mode: entry.mode,
            opened_at: entry.opened_at,
            available: entry.available,
            error: entry.error.clone(),
            installation: blockloom_core::hub::bound_installation(&entry.path),
            installation_label: blockloom_core::hub::bound_installation(&entry.path)
                .map(|id| blockloom_core::hub::friendly_name(&id)),
            wrong_editor: blockloom_core::hub::conflict(&entry.path).is_some(),
            in_use: blockloom_core::sync::live_owner(&entry.path, &s.session_id).is_some(),
        })
        .collect()
}

pub(crate) fn state_dto(s: &AppState) -> StateDto {
    let project = s.project().and_then(|project| {
        blockloom_core::wire::to_wire(project)
            .map_err(|e| tracing::warn!("Couldn't serialize the project: {e}"))
            .ok()
    });
    state_dto_with(s, project)
}

/// [`state_dto`] carrying `project` as the document.
pub(crate) fn state_dto_with(s: &AppState, project: Option<serde_json::Value>) -> StateDto {
    StateDto {
        script_statuses: s
            .open
            .as_ref()
            .map(|open| open.script_statuses.clone())
            .unwrap_or_default(),
        library: library_dtos(s),
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
        can_undo: s.can_undo(),
        can_redo: s.can_redo(),
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
        // A project runtime only shows there when its view took the socket.
        runtime_embedded: s.embedded.is_some()
            && s.runtime.as_ref().is_none_or(|runtime| runtime.in_view),
        pointer_locked: s.pointer_locked && s.running && s.runtime.is_some(),
        ray_tracing: s.runtime.as_ref().and(s.ray_tracing.clone()),
        lan_session: s.runtime.as_ref().and(s.lan_session.clone()),
        sync: sync_dto(s),
        picked_tile: s.picked_tile.clone(),
        plugins: crate::commands::plugins::summary(s),
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

/// A draft belongs to one project and one saved revision. It holds only the
/// interface document it started from, not a copy of the project.
pub(crate) struct InterfaceEditTransaction {
    pub(crate) token: String,
    /// The `.blockui` being edited, or `None` for the scene's own interface.
    pub(crate) screen: Option<(String, blockloom_core::ui_tree::UiScreen)>,
    pub(crate) original: blockloom_core::ui::UiDocument,
    pub(crate) revision: u64,
    pub(crate) document: blockloom_core::ui::UiDocument,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_lines_point_at_the_named_span() {
        let text = "error: cannot find value `foo`\n --> assets/scripts/player.rs:12:5\n  |\n12 |     foo();";
        assert_eq!(first_error_line(text, "assets/scripts/player.rs"), Some(12));
        assert_eq!(first_error_line("boom", "assets/scripts/player.rs"), None);
        let line = LogLine::script_error(
            "Player".to_string(),
            "assets/scripts/player.rs",
            text.to_string(),
        );
        assert_eq!(line.level, "error");
        assert_eq!(line.path.as_deref(), Some("assets/scripts/player.rs"));
        assert_eq!(line.line, Some(12));
        assert_eq!(
            LogLine::say("A".to_string(), "hi".to_string()).level,
            "info"
        );
    }

    #[test]
    fn repeats_fold_into_one_line() {
        let mut log = Vec::new();
        let say = |text: &str, time: f64| LogLine {
            time: Some(time),
            ..LogLine::say("Player".to_string(), text.to_string())
        };
        fold_into(&mut log, say("score: 1", 0.5), 1);
        fold_into(&mut log, say("score: 1", 0.6), 2);
        fold_into(&mut log, say("score: 1", 0.7), 3);
        assert_eq!(log.len(), 1);
        assert_eq!((log[0].id, log[0].count, log[0].time), (1, 3, Some(0.7)));
        fold_into(&mut log, say("score: 2", 0.8), 4);
        fold_into(
            &mut log,
            LogLine::error("Player".to_string(), "score: 2".to_string()),
            5,
        );
        assert_eq!(
            log.iter().map(|l| (l.id, l.count)).collect::<Vec<_>>(),
            [(1, 3), (4, 1), (5, 1)]
        );
        for id in 6..6 + LOG_LIMIT as u64 {
            fold_into(&mut log, say(&id.to_string(), 1.0), id);
        }
        assert_eq!(log.len(), LOG_LIMIT);
        assert_eq!(log[0].id, 6, "the oldest lines go first");
    }
}
