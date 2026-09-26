//! Supervising the game world.
//!
//! The runtime is started on Play and kept for as long as the editor lives (or
//! until the project switches dimension, which needs a different plugin set and
//! so a fresh world). It is either a child process, whose stdout is read on a
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
            RuntimeMessage::Ready { protocol } => {
                if protocol != blockloom_protocol::PROTOCOL_VERSION {
                    s.push_log(LogLine {
                        kind: "error".to_string(),
                        actor: "Blockloom".to_string(),
                        text: format!(
                            "The game runtime speaks protocol {protocol}, this editor speaks {}. Rebuild the workspace.",
                            blockloom_protocol::PROTOCOL_VERSION
                        ),
                    });
                }
            }
            RuntimeMessage::Say { actor, text } => {
                let line = LogLine {
                    kind: "say".to_string(),
                    actor: name_of(&actor),
                    text,
                };
                self.publish_log(s, line);
                return;
            }
            RuntimeMessage::Error { actor, message } => {
                let line = LogLine {
                    kind: "error".to_string(),
                    actor: name_of(&actor),
                    text: message,
                };
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
                    s.push_log(LogLine {
                        kind: "error".to_string(),
                        actor: "Blockloom".to_string(),
                        text: message,
                    });
                }
            }
            RuntimeMessage::Fatal { message } => {
                s.running = false;
                s.push_log(LogLine {
                    kind: "error".to_string(),
                    actor: "Blockloom".to_string(),
                    text: message,
                });
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
        s.preview_port = None;
        s.pointer_locked = false;
        s.ray_tracing = None;
        let dto = crate::state::state_dto(&s);
        drop(s);
        self.app.send(Event::RuntimeClosed);
        self.app.emit_state(&dto);
    }
}
