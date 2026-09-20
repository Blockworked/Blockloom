//! Everything the editor holds, and the one snapshot it hands the frontend.

use blockloom_core::project::Project;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub(crate) use blockstitch_core::editor::{
    EditSession, History, InstrPath, ValueBuffers, ValueLocation,
};

pub(crate) type SharedState = Arc<Mutex<AppState>>;

/// How many undo steps the editor keeps.
pub(crate) const UNDO_STACK_LIMIT: usize = 50;

/// How many log lines are kept from a run.
const LOG_LIMIT: usize = 200;

pub(crate) struct AppState {
    /// Every saved project, name-sorted, as `load_projects` returns them.
    pub(crate) projects: Vec<Project>,
    pub(crate) selected: Option<usize>,
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
}

impl AppState {
    pub(crate) fn project(&self) -> Option<&Project> {
        self.selected.and_then(|index| self.projects.get(index))
    }

    pub(crate) fn project_mut(&mut self) -> Option<&mut Project> {
        match self.selected {
            Some(index) => self.projects.get_mut(index),
            None => None,
        }
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

// ─── The snapshot the frontend gets ────────────────────────────────────────

#[derive(Serialize, Clone)]
pub(crate) struct StateDto {
    pub(crate) project_names: Vec<String>,
    pub(crate) selected: Option<usize>,
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
    /// False when no runtime binary sits next to this one, which is the one
    /// install mistake that would otherwise look like "Play does nothing".
    pub(crate) runtime_available: bool,
    /// Whether a game window is open at all - it outlives a run, so Play is
    /// instant the second time.
    pub(crate) runtime_open: bool,
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
        project_names: s.projects.iter().map(|p| p.name.clone()).collect(),
        selected: s.selected,
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
        runtime_available: blockloom_protocol::runtime_path().exists(),
        runtime_open: s.runtime.is_some(),
    }
}
