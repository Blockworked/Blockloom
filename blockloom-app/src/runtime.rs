//! Supervising the game world's process.
//!
//! The runtime is spawned on Play and kept for as long as the editor lives (or
//! until the project switches dimension, which needs a different plugin set and
//! so a fresh process). Its stdout is read on a thread that folds every message
//! straight into app state.

use crate::state::LogLine;
use crate::{Backend, Event};
use blockloom_core::scene::Mode;
use blockloom_protocol::{EditorMessage, RuntimeMessage, decode, encode, runtime_path};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

pub(crate) struct RuntimeHandle {
    child: Child,
    stdin: ChildStdin,
    /// Which dimension this process was started for.
    pub(crate) mode: Mode,
}

impl RuntimeHandle {
    /// Starts the runtime binary sitting next to this one and begins reading
    /// its messages into `backend`'s state.
    pub(crate) fn spawn(mode: Mode, backend: Backend) -> Result<Self, String> {
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
                        Some(Ok(message)) => backend.on_runtime_message(message),
                        Some(Err(e)) => tracing::warn!("Bad message from the runtime: {e}"),
                        None => {}
                    }
                }
                // The pipe closed: the window is gone, whether it was asked to
                // go or crashed.
                backend.on_runtime_exit();
            })
            .map_err(|e| e.to_string())?;

        Ok(Self { child, stdin, mode })
    }

    /// Sends a message. `false` means the pipe is gone and the handle should
    /// be dropped.
    pub(crate) fn send(&mut self, message: &EditorMessage) -> bool {
        let line = encode(message);
        self.stdin.write_all(line.as_bytes()).is_ok() && self.stdin.flush().is_ok()
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        // Ask first, then insist: a clean exit closes the window without
        // Chromium-style orphans, but a wedged renderer still has to go.
        let _ = self.send(&EditorMessage::Shutdown);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Backend {
    /// Folds one runtime message into app state and tells the frontend.
    pub(crate) fn on_runtime_message(&self, message: RuntimeMessage) {
        let Ok(mut s) = self.state.lock() else {
            return;
        };
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
                s.push_log(line);
            }
            RuntimeMessage::Error { actor, message } => {
                let line = LogLine {
                    kind: "error".to_string(),
                    actor: name_of(&actor),
                    text: message,
                };
                s.push_log(line);
            }
            RuntimeMessage::Status(status) => {
                s.running = status.running;
                s.paused = status.paused;
                s.status = Some(status);
            }
            RuntimeMessage::Stopped => {
                s.running = false;
                s.paused = false;
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

    /// The runtime's window closed. Its handle is dropped so the next Play
    /// starts a fresh one.
    pub(crate) fn on_runtime_exit(&self) {
        let Ok(mut s) = self.state.lock() else {
            return;
        };
        s.running = false;
        s.paused = false;
        s.status = None;
        s.runtime = None;
        let dto = crate::state::state_dto(&s);
        drop(s);
        self.app.send(Event::RuntimeClosed);
        self.app.emit_state(&dto);
    }
}
