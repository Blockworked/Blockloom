//! Blockloom's backend: the project document, every command the editor issues,
//! and the game runtime's leash. It has no dependency on Tauri or CEF, so the
//! editor window and the browser dev bridge host the same code.

mod commands;
mod dispatch;
mod runtime;
mod state;

use crate::state::{AppState, SharedState, UNDO_STACK_LIMIT, state_dto};
use blockloom_core::library;
use std::sync::{Arc, Mutex};

/// Something the backend tells whoever is hosting it.
#[derive(Clone, Debug)]
pub enum Event {
    /// The whole snapshot, already serialized, after any change.
    State(Arc<str>),
    /// The game window closed on its own.
    RuntimeClosed,
}

/// Where the backend publishes [`Event`]s. The host supplies the sink: the
/// editor re-emits them as a Tauri event, the dev bridge as a WebSocket frame.
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
}

impl Backend {
    /// Lists the projects the Dashboard offers and opens none of them - the
    /// editor starts on the Dashboard.
    pub fn start(app: AppHandle) -> Backend {
        // Registers Blockloom's reporter blocks before any project loads.
        blockloom_core::init();

        library::migrate_legacy_projects();
        let state = AppState {
            library: library::list(),
            open: None,
            selected_actor: None,
            history: state::History::new(UNDO_STACK_LIMIT),
            invalid_field_buffers: Default::default(),
            runtime: None,
            running: false,
            paused: false,
            status: None,
            log: Vec::new(),
        };
        Backend {
            state: Arc::new(Mutex::new(state)),
            app,
        }
    }

    /// The current snapshot as JSON - what a newly connected frontend asks for.
    pub fn state_json(&self) -> Result<String, String> {
        let s = self.state.lock().map_err(|e| e.to_string())?;
        serde_json::to_string(&state_dto(&s)).map_err(|e| e.to_string())
    }

    /// Closes the game window, if one is open. Called as the editor exits so
    /// the runtime never outlives it.
    pub fn shutdown(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.runtime = None;
        }
    }
}
