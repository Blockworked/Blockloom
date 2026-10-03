//! Blockloom's backend: the project document, every command the editor issues,
//! and the game runtime's leash. It has no dependency on Qt, so the editor
//! window and the shell host the same code.

pub mod attach;
mod build_jobs;
mod commands;
mod dispatch;
mod runtime;
mod scaffold;
pub mod screen;
pub mod shell;
mod state;

pub use runtime::EmbeddedRuntime;

use crate::state::{AppState, SharedState, UNDO_STACK_LIMIT, state_dto};
use blockloom_core::library;
use std::sync::{Arc, Mutex};

/// Something the backend tells whoever is hosting it.
#[derive(Clone, Debug)]
pub enum Event {
    /// The whole snapshot, already serialized, after any change.
    State(Arc<str>),
    /// A running world's periodic status alone, serialized. It arrives
    /// several times a second, so a frontend can apply it without re-reading
    /// the whole snapshot; the next snapshot carries it too.
    Status(Arc<str>),
    /// The run log alone, serialized as `{total, lines}` - a `say` in a loop
    /// would otherwise re-send the whole snapshot every frame.
    Log(Arc<str>),
    /// The game window closed on its own.
    RuntimeClosed,
}

/// Where the backend publishes [`Event`]s. The host supplies the sink: the
/// editor forwards them to QML.
#[derive(Clone)]
pub struct AppHandle {
    sink: Arc<dyn Fn(Event) + Send + Sync>,
}

impl AppHandle {
    pub fn new(sink: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self {
            sink: Arc::new(sink),
        }
    }

    pub(crate) fn emit_state<T: serde::Serialize>(&self, dto: &T) {
        match serde_json::to_string(dto) {
            Ok(json) => self.send(Event::State(json.into())),
            Err(e) => tracing::warn!("Couldn't serialize the app state: {e}"),
        }
    }

    pub(crate) fn send(&self, event: Event) {
        (self.sink)(event);
    }
}

/// The running backend. Cloning shares the same state.
#[derive(Clone)]
pub struct Backend {
    pub(crate) state: SharedState,
    pub(crate) app: AppHandle,
    pub(crate) builds: build_jobs::BuildJobs,
}

impl Backend {
    /// Lists the projects the Dashboard offers and opens none of them - the
    /// editor starts on the Dashboard.
    pub fn start(app: AppHandle) -> Backend {
        Self::start_with(app, None)
    }

    /// [`start`](Self::start), with the game world run inside this process
    /// by `embedded` rather than spawned as a child.
    pub fn start_embedded(app: AppHandle, embedded: Arc<dyn EmbeddedRuntime>) -> Backend {
        Self::start_with(app, Some(embedded))
    }

    fn start_with(app: AppHandle, embedded: Option<Arc<dyn EmbeddedRuntime>>) -> Backend {
        // Registers Blockloom's reporter blocks before any project loads.
        blockloom_core::init();

        library::migrate_legacy_projects();
        let state = AppState {
            library: library::list(),
            open: None,
            session_id: blockloom_core::sync::new_session(),
            selected_actor: None,
            history: state::History::new(UNDO_STACK_LIMIT),
            invalid_field_buffers: Default::default(),
            runtime: None,
            running: false,
            paused: false,
            status: None,
            interface_edit: None,
            interface_design: None,
            interface_layout: None,
            log: Vec::new(),
            log_total: 0,
            // An embedded world is always shown, so it is always previewing.
            preview_enabled: embedded.is_some(),
            preview_headless: false,
            preview_port: None,
            pointer_locked: false,
            ray_tracing: None,
            plugin_diagnostics: serde_json::Value::Null,
            embedded,
            scene_view: Default::default(),
            picked_tile: None,
        };
        Backend {
            state: Arc::new(Mutex::new(state)),
            app,
            builds: Default::default(),
        }
    }

    /// The current snapshot as JSON - what a newly connected frontend asks for.
    pub fn state_json(&self) -> Result<String, String> {
        let s = self.state.lock().map_err(|e| e.to_string())?;
        serde_json::to_string(&state_dto(&s)).map_err(|e| e.to_string())
    }

    /// Closes the game window, if one is open. Called as the editor exits so
    /// the runtime never outlives it. Also drops our project folder lock, so
    /// a backend that goes away without closing its project doesn't look
    /// live to the next opener until its heartbeat runs out.
    pub fn shutdown(&self) {
        self.builds.shutdown();
        if let Ok(mut s) = self.state.lock() {
            if let Some(open) = &s.open
                && open.owns_lock
            {
                blockloom_core::sync::release_lock(&open.dir, &s.session_id);
            }
            s.runtime = None;
        }
    }
}
