//! Supervising the game world.
//!
//! The runtime is started on Play and kept for as long as the editor lives.
//! Both dimensions' pipelines live side by side, so a project or scene switch
//! across dimensions swaps live under the next rebuild. It is either a child
//! process, whose stdout is read on a
//! thread, or - when the host supplies an [`EmbeddedRuntime`] - a world inside
//! this process talking over channels. Either way every message is folded
//! straight into app state.

use crate::state::LogLine;
use crate::{Backend, Event};
use blockloom_core::scene::Mode;
use blockloom_protocol::{EditorMessage, RuntimeMessage, decode, encode, runtime_path};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

/// The script file an actor runs, when it runs exactly one. Enough to link a
/// run-log line back to its file; an actor with several scripts names none,
/// since no single file owns the line.
fn lone_script_path(s: &crate::state::AppState, actor_id: &str) -> Option<String> {
    let mut scripts = s.project()?.actor(actor_id)?.components.scripts();
    let first = scripts.next()?.to_string();
    scripts.next().is_none().then_some(first)
}

/// Starts a game world inside this process instead of as a child. The Qt
/// editor supplies one, so its Game view can show the world's frames.
pub trait EmbeddedRuntime: Send + Sync {
    /// Starts a world for `mode` reading `incoming`. It reports through
    /// `outgoing`, and dropping that is how it says it has gone. Dropping
    /// the returned guard waits for the world to finish.
    fn start(
        &self,
        mode: Mode,
        incoming: Receiver<EditorMessage>,
        outgoing: Sender<RuntimeMessage>,
    ) -> Result<Box<dyn Send>, String>;
}

enum Link {
    Child {
        child: Child,
        stdin: ChildStdin,
    },
    Embedded {
        incoming: Sender<EditorMessage>,
        world: Option<Box<dyn Send>>,
    },
}

pub(crate) struct RuntimeHandle {
    link: Link,
    pub(crate) id: u64,
    /// Which dimension this process was started for.
    pub(crate) mode: Mode,
}

impl RuntimeHandle {
    /// Starts the runtime binary sitting next to this one and begins reading
    /// its messages into `backend`'s state.
    pub(crate) fn spawn(
        mode: Mode,
        backend: Backend,
        embedded: Option<Arc<dyn EmbeddedRuntime>>,
    ) -> Result<Self, String> {
        let id = NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed);
        if let Some(host) = embedded {
            return Self::embed(id, mode, backend, host.as_ref());
        }
        let path = runtime_path();
        if !path.exists() {
            return Err(format!(
                "The game runtime is missing: expected {}. Build the whole workspace, not just the editor.",
                path.display()
            ));
        }
        let mut child = Command::new(&path)
            .arg("--mode")
            .arg(if mode.is_3d() { "3d" } else { "2d" })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The runtime's own logs belong in the terminal, not in the editor.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Couldn't start {}: {e}", path.display()))?;
        let stdin = child.stdin.take().ok_or("the runtime has no stdin")?;
        let stdout = child.stdout.take().ok_or("the runtime has no stdout")?;

        std::thread::Builder::new()
            .name("runtime-reader".to_string())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    match decode::<RuntimeMessage>(&line) {
                        Some(Ok(message)) => backend.on_runtime_message(id, message),
                        Some(Err(e)) => tracing::warn!("Bad message from the runtime: {e}"),
                        None => {}
                    }
                }
                // The pipe closed: the window is gone, whether it was asked to
                // go or crashed.
                backend.on_runtime_exit(id);
            })
            .map_err(|e| e.to_string())?;

        Ok(Self {
            link: Link::Child { child, stdin },
            id,
            mode,
        })
    }

    /// Starts the host's in-process world, and reads what it reports the way
    /// a child's stdout is read.
    fn embed(
        id: u64,
        mode: Mode,
        backend: Backend,
        host: &dyn EmbeddedRuntime,
    ) -> Result<Self, String> {
        let (incoming, from_editor) = channel();
        let (to_editor, outgoing) = channel();
        let world = host.start(mode, from_editor, to_editor)?;
        std::thread::Builder::new()
            .name("runtime-reader".to_string())
            .spawn(move || {
                for message in outgoing {
                    backend.on_runtime_message(id, message);
                }
                backend.on_runtime_exit(id);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            link: Link::Embedded {
                incoming,
                world: Some(world),
            },
            id,
            mode,
        })
    }

    /// Sends a message. `false` means the link is gone and the handle should
    /// be dropped.
    pub(crate) fn send(&mut self, message: &EditorMessage) -> bool {
        match &mut self.link {
            Link::Child { stdin, .. } => {
                let line = encode(message);
                stdin.write_all(line.as_bytes()).is_ok() && stdin.flush().is_ok()
            }
            // An embedded world always draws into the Game view, so there is
            // no sidecar to start or stop.
            Link::Embedded { .. } if matches!(message, EditorMessage::Preview { .. }) => true,
            Link::Embedded { incoming, .. } => incoming.send(message.clone()).is_ok(),
        }
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        let _ = self.send(&EditorMessage::Shutdown);
        match &mut self.link {
            // Ask first, then insist: a clean exit closes the window without
            // orphans, but a wedged renderer still has to go.
            Link::Child { child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            // Closing the channel stops the world even if it missed the
            // Shutdown; then wait, so two worlds never share the GPU.
            Link::Embedded { incoming, world } => {
                *incoming = channel().0;
                drop(world.take());
            }
        }
    }
}

impl Backend {
    /// Folds one runtime message into app state and tells the frontend.
    pub(crate) fn on_runtime_message(&self, runtime_id: u64, message: RuntimeMessage) {
        let Ok(mut s) = self.state.lock() else {
            return;
        };
        if s.runtime
            .as_ref()
            .is_none_or(|runtime| runtime.id != runtime_id)
        {
            return;
        }
        let name_of = |id: &str| {
            s.project()
                .and_then(|project| project.actor(id))
                .map(|actor| actor.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        match message {
            RuntimeMessage::ScriptLoaded { actor, path, error } => {
                if s.project()
                    .and_then(|project| project.actor(&actor))
                    .is_none_or(|actor| !actor.components.scripts().any(|other| other == path))
                {
                    return;
                }
                let name = name_of(&actor);
                let build_failed = s
                    .open
                    .as_ref()
                    .and_then(|open| open.script_statuses.get(&path))
                    .is_some_and(|status| status.stage == "build_failed");
                if !build_failed {
                    s.record_script_status(
                        &path,
                        if error.is_some() {
                            "load_failed"
                        } else {
                            "loaded"
                        },
                        error.clone(),
                    );
                }
                if !build_failed && let Some(error) = error {
                    s.push_log(LogLine::script_error(
                        name,
                        &path,
                        format!("{path} couldn't load:\n{error}"),
                    ));
                }
            }
            RuntimeMessage::LanSession(status) => {
                s.lan_session = Some(status);
            }
            RuntimeMessage::InterfaceLayout(layout) => {
                if s.interface_design.as_ref().is_some_and(|d| {
                    d.revision == layout.revision && d.generation == layout.generation
                }) {
                    s.interface_layout = Some(layout);
                }
                return;
            }
            RuntimeMessage::Ready { protocol } => {
                s.lan_session = None;
                if protocol != blockloom_protocol::PROTOCOL_VERSION {
                    s.push_log(LogLine::error("Blockloom".to_string(), format!(
                            "The game runtime speaks protocol {protocol}, this editor speaks {}. Rebuild the workspace.",
                            blockloom_protocol::PROTOCOL_VERSION
                        )));
                }
            }
            RuntimeMessage::Say { actor, text } => {
                let mut line = LogLine::say(name_of(&actor), text);
                line.path = lone_script_path(&s, &actor);
                self.publish_log(s, line);
                return;
            }
            RuntimeMessage::Error { actor, message } => {
                let mut line = LogLine::error(name_of(&actor), message);
                line.path = lone_script_path(&s, &actor);
                self.publish_log(s, line);
                return;
            }
            RuntimeMessage::Status(status) => {
                let unchanged = s.running == status.running && s.paused == status.paused;
                s.running = status.running;
                s.paused = status.paused;
                // Only positions and values moved: send those on their own.
                if unchanged {
                    let json = serde_json::to_string(&status);
                    s.status = Some(status);
                    drop(s);
                    match json {
                        Ok(json) => self.app.send(Event::Status(json.into())),
                        Err(e) => tracing::warn!("Couldn't serialize the run status: {e}"),
                    }
                    return;
                }
                s.status = Some(status);
            }
            RuntimeMessage::Stopped => {
                s.lan_session = None;
                s.running = false;
                s.paused = false;
                s.pointer_locked = false;
            }
            RuntimeMessage::PreviewReady { port } => {
                s.preview_port = Some(port);
            }
            RuntimeMessage::PreviewStopped => {
                s.preview_port = None;
            }
            RuntimeMessage::PointerLock { locked } => {
                s.pointer_locked = locked;
            }
            RuntimeMessage::PluginDiagnostics { snapshot } => {
                s.plugin_diagnostics = snapshot;
            }
            RuntimeMessage::RayTracing(status) => {
                s.ray_tracing = Some(status);
            }
            RuntimeMessage::Picked { actor } => {
                if s.project()
                    .and_then(|project| project.actor(&actor))
                    .is_none()
                {
                    return;
                }
                s.selected_actor = Some(actor);
                s.history.end_session();
            }
            RuntimeMessage::Placed {
                actor,
                placement,
                offset,
                volume,
            } => {
                crate::commands::place_from_view(&mut s, &actor, placement, offset, volume);
            }
            RuntimeMessage::TerrainStroke { actor, stroke } => {
                if let Err(message) = crate::commands::terrain_stroke(&mut s, &actor, stroke) {
                    s.push_log(LogLine::error("Blockloom".to_string(), message));
                }
            }
            RuntimeMessage::TileStroke {
                actor,
                brush,
                segments,
            } => {
                if let Err(message) =
                    crate::commands::tile_stroke(&mut s, &actor, &brush, &segments)
                {
                    s.push_log(LogLine::error("Blockloom".to_string(), message));
                }
            }
            RuntimeMessage::TilePicked { actor, tile } => {
                let serial = s.picked_tile.as_ref().map_or(1, |pick| pick.serial + 1);
                s.picked_tile = Some(crate::state::PickedTile {
                    actor,
                    tile,
                    serial,
                });
            }
            RuntimeMessage::PluginTool {
                plugin,
                tool,
                hits,
                options,
            } => {
                drop(s);
                // Run outside the lock: the command takes it itself.
                if let Err(message) = crate::commands::plugins::run_tool(
                    &self.state,
                    &self.app,
                    &plugin,
                    &tool,
                    &hits,
                    &options,
                ) {
                    let line = LogLine::error("Blockloom".to_string(), message);
                    if let Ok(s) = self.state.lock() {
                        self.publish_log(s, line);
                    }
                }
                return;
            }
            RuntimeMessage::PluginCall {
                actor,
                plugin,
                block,
                args,
            } => {
                let who = name_of(&actor);
                drop(s);
                // Run outside the lock: the command takes it itself.
                if let Err(message) = crate::commands::plugins::run_block(
                    &self.state,
                    &self.app,
                    &actor,
                    &plugin,
                    &block,
                    args,
                ) {
                    let line = LogLine::error(who, message);
                    if let Ok(s) = self.state.lock() {
                        self.publish_log(s, line);
                    }
                }
                return;
            }
            RuntimeMessage::Fatal { message } => {
                s.running = false;
                s.push_log(LogLine::error("Blockloom".to_string(), message));
            }
        }
        let dto = crate::state::state_dto(&s);
        drop(s);
        self.app.emit_state(&dto);
    }

    /// Adds a line and sends the log on its own, leaving the snapshot alone.
    fn publish_log(&self, mut s: std::sync::MutexGuard<'_, crate::state::AppState>, line: LogLine) {
        s.push_log(line);
        let json = serde_json::to_string(&crate::state::LogDto {
            total: s.log_total,
            lines: &s.log,
        });
        drop(s);
        match json {
            Ok(json) => self.app.send(Event::Log(json.into())),
            Err(e) => tracing::warn!("Couldn't serialize the run log: {e}"),
        }
    }

    /// The runtime's window closed. Its handle is dropped so the next Play
    /// starts a fresh one.
    pub(crate) fn on_runtime_exit(&self, runtime_id: u64) {
        let Ok(mut s) = self.state.lock() else {
            return;
        };
        if s.runtime
            .as_ref()
            .is_none_or(|runtime| runtime.id != runtime_id)
        {
            return;
        }
        s.running = false;
        s.paused = false;
        s.status = None;
        s.runtime = None;
        s.interface_design = None;
        s.interface_layout = None;
        s.preview_port = None;
        s.pointer_locked = false;
        s.ray_tracing = None;
        let dto = crate::state::state_dto(&s);
        drop(s);
        self.app.send(Event::RuntimeClosed);
        self.app.emit_state(&dto);
    }
}

#[cfg(test)]
mod script_status_tests {
    use super::*;
    use blockloom_core::{components::ActorComponent, project::Project};
    use serde_json::json;

    #[test]
    fn scripts_stack_run_in_order_and_shared_paths_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Backend::start(crate::AppHandle::new(|_| {}));
        let project = Project::starter("Scripts", Mode::TwoD);
        let id = project.actors.first().unwrap().id.clone();
        {
            let mut state = backend.state.lock().unwrap();
            state.open = Some(crate::state::OpenProject::new(
                project,
                temp.path().into(),
                0,
                false,
                false,
            ));
        }
        let scripts_of = |backend: &Backend| -> Vec<String> {
            let state = backend.state.lock().unwrap();
            let project = state.project().unwrap();
            project
                .actor(&id)
                .unwrap()
                .components
                .scripts()
                .map(str::to_string)
                .collect()
        };
        // First script via the legacy entry point.
        let first: String = backend
            .dispatch("create_script", json!({"actorId": id}))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        // A second one appends rather than replacing the first.
        let second: String = backend
            .dispatch("add_script", json!({"actorId": id}))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(first, second);
        assert_eq!(scripts_of(&backend), vec![first.clone(), second.clone()]);
        // Shared library code is not runnable.
        assert!(
            backend
                .dispatch(
                    "add_script",
                    json!({"actorId": id, "path": "assets/scripts/shared/util.rs"}),
                )
                .is_err()
        );
        // Duplicates are refused.
        assert!(
            backend
                .dispatch("add_script", json!({"actorId": id, "path": first}))
                .is_err()
        );
        // Order is the run order: moving the second to the front sticks.
        backend
            .dispatch(
                "move_script",
                json!({"actorId": id, "path": second, "index": 0}),
            )
            .unwrap();
        assert_eq!(scripts_of(&backend), vec![second.clone(), first.clone()]);
        // One card edits one slot: rename the front slot by its old path.
        let renamed = "assets/scripts/renamed.rs".to_string();
        backend
            .dispatch(
                "set_actor_component",
                json!({"actorId": id, "name": second, "component": {"component": "Script", "path": renamed}}),
            )
            .unwrap();
        assert_eq!(scripts_of(&backend), vec![renamed.clone(), first.clone()]);
        // Detaching one leaves the other running.
        backend
            .dispatch("remove_script", json!({"actorId": id, "path": renamed}))
            .unwrap();
        assert_eq!(scripts_of(&backend), vec![first.clone()]);
    }

    #[test]
    fn load_results_publish_details_and_preserve_build_failures() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Backend::start(crate::AppHandle::new(|_| {}));
        let mut project = Project::starter("Scripts", Mode::TwoD);
        let actor = project.actors.first_mut().unwrap();
        let id = actor.id.clone();
        let path = "assets/scripts/test.rs";
        actor
            .components
            .insert(ActorComponent::Script { path: path.into() });
        let (incoming, _receiver) = channel();
        {
            let mut state = backend.state.lock().unwrap();
            state.open = Some(crate::state::OpenProject::new(
                project,
                temp.path().into(),
                0,
                false,
                false,
            ));
            state.runtime = Some(RuntimeHandle {
                id: 77,
                mode: Mode::TwoD,
                link: Link::Embedded {
                    incoming,
                    world: None,
                },
            });
        }
        let send = |error: Option<&str>| {
            backend.on_runtime_message(
                77,
                RuntimeMessage::ScriptLoaded {
                    actor: id.clone(),
                    path: path.into(),
                    error: error.map(str::to_string),
                },
            )
        };
        send(Some("script ABI mismatch"));
        let state = backend.dispatch("get_state", json!({})).unwrap();
        assert_eq!(state["script_statuses"][path]["stage"], "load_failed");
        assert_eq!(
            state["script_statuses"][path]["error"],
            "script ABI mismatch"
        );
        send(None);
        let state = backend.dispatch("get_state", json!({})).unwrap();
        assert_eq!(state["script_statuses"][path]["stage"], "loaded");
        assert!(state["script_statuses"][path]["error"].is_null());
        backend.state.lock().unwrap().record_script_status(
            path,
            "build_failed",
            Some("compiler error".into()),
        );
        send(None);
        let state = backend.dispatch("get_state", json!({})).unwrap();
        assert_eq!(state["script_statuses"][path]["stage"], "build_failed");
        assert_eq!(state["script_statuses"][path]["error"], "compiler error");
        backend
            .dispatch(
                "write_script",
                json!({"actorId": id, "source": "invalid rust"}),
            )
            .unwrap();
        let state = backend.dispatch("get_state", json!({})).unwrap();
        assert!(state["script_statuses"][path].is_null());
        backend
            .dispatch("check_script", json!({"actorId": id}))
            .unwrap();
        let state = backend.dispatch("get_state", json!({})).unwrap();
        assert_eq!(state["script_statuses"][path]["stage"], "build_failed");
        assert!(
            !state["script_statuses"][path]["error"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        if blockloom_core::script::toolchain_version().is_ok() {
            backend
                .dispatch(
                    "write_script",
                    json!({"actorId": id, "source": "blockloom::export!();"}),
                )
                .unwrap();
            backend
                .dispatch("check_script", json!({"actorId": id}))
                .unwrap();
            let state = backend.dispatch("get_state", json!({})).unwrap();
            assert_eq!(state["script_statuses"][path]["stage"], "built");
        }
    }
}

#[cfg(test)]
mod editor_phase4_tests {
    use super::*;
    use blockloom_core::{components::ActorComponent, project::Project};
    use serde_json::json;

    fn open_backend(project: Project, temp: &tempfile::TempDir) -> Backend {
        let backend = Backend::start(crate::AppHandle::new(|_| {}));
        {
            let mut state = backend.state.lock().unwrap();
            state.open = Some(crate::state::OpenProject::new(
                project,
                temp.path().into(),
                0,
                false,
                false,
            ));
            let (incoming, _receiver) = channel();
            state.runtime = Some(RuntimeHandle {
                id: 77,
                mode: Mode::TwoD,
                link: Link::Embedded {
                    incoming,
                    world: None,
                },
            });
        }
        backend
    }

    #[test]
    fn completions_and_hover_answer_from_the_backend() {
        let temp = tempfile::tempdir().unwrap();
        let backend = open_backend(Project::starter("Scripts", Mode::TwoD), &temp);
        let found = backend
            .dispatch("script_completions", json!({"prefix": "go_t"}))
            .unwrap();
        assert!(
            found
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["insert"] == "go_to"),
            "{found}"
        );
        let hovered = backend
            .dispatch("script_hover", json!({"symbol": "say"}))
            .unwrap();
        assert!(hovered.as_str().unwrap().contains("Actor"), "{hovered}");
        assert!(
            backend
                .dispatch("script_hover", json!({"symbol": "no_such_thing_here"}))
                .unwrap()
                .is_null()
        );
    }

    #[test]
    fn format_script_rewrites_through_rustfmt() {
        let temp = tempfile::tempdir().unwrap();
        let backend = open_backend(Project::starter("Scripts", Mode::TwoD), &temp);
        let id = backend
            .state
            .lock()
            .unwrap()
            .project()
            .unwrap()
            .actors
            .first()
            .unwrap()
            .id
            .clone();
        backend
            .dispatch("create_script", json!({"actorId": id}))
            .unwrap();
        if blockloom_core::script::ide::rustfmt_version().is_err() {
            assert!(
                backend
                    .dispatch("format_script", json!({"actorId": id}))
                    .is_err()
            );
            return;
        }
        backend
            .dispatch(
                "write_script",
                json!({"actorId": id, "source": "use blockloom::*;\nfn start(me: &Actor) {\nlet x=1;\n}\nblockloom::export!(start = start);\n"}),
            )
            .unwrap();
        let formatted = backend
            .dispatch("format_script", json!({"actorId": id}))
            .unwrap();
        assert!(
            formatted.as_str().unwrap().contains("let x = 1;"),
            "{formatted}"
        );
        // Formatting twice is stable, and the file holds the formatted text.
        let again = backend
            .dispatch("format_script", json!({"actorId": id}))
            .unwrap();
        assert_eq!(formatted, again);
    }

    #[test]
    fn clippy_script_answers_without_crashing() {
        let temp = tempfile::tempdir().unwrap();
        let backend = open_backend(Project::starter("Scripts", Mode::TwoD), &temp);
        let id = backend
            .state
            .lock()
            .unwrap()
            .project()
            .unwrap()
            .actors
            .first()
            .unwrap()
            .id
            .clone();
        backend
            .dispatch("create_script", json!({"actorId": id}))
            .unwrap();
        // Missing clippy is an empty answer, not an error; present clippy
        // answers a diagnostics-shaped list.
        let found = backend
            .dispatch("clippy_script", json!({"actorId": id}))
            .unwrap();
        assert!(found.is_array(), "{found}");
    }

    #[test]
    fn runtime_lines_link_back_to_the_script_file() {
        let temp = tempfile::tempdir().unwrap();
        let mut project = Project::starter("Scripts", Mode::TwoD);
        let id = project.actors.first().unwrap().id.clone();
        let path = "assets/scripts/test.rs";
        project
            .actors
            .first_mut()
            .unwrap()
            .components
            .insert(ActorComponent::Script { path: path.into() });
        let backend = open_backend(project, &temp);
        backend.on_runtime_message(
            77,
            RuntimeMessage::Error {
                actor: id.clone(),
                message: "the script panicked in tick".to_string(),
            },
        );
        let state = backend.dispatch("get_state", json!({})).unwrap();
        let last = state["log"].as_array().unwrap().last().unwrap();
        assert_eq!(last["path"], path);
        assert_eq!(last["level"], "error");
    }
}
