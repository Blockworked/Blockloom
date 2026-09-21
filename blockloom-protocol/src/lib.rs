//! The wire format between the editor and the game world.
//!
//! The world runs in its own process (`blockloom-runtime`) because Bevy needs
//! its own window and event loop, which the editor's CEF runtime already owns.
//! The editor spawns it as a child and the two talk newline-delimited JSON over
//! its stdin and stdout - no sockets, no ports, and the pipe closing is all the
//! shutdown handshake either side needs.

use blockloom_core::project::Project;
use blockloom_core::value::Evaluated;
use serde::{Deserialize, Serialize};

/// Bumped when a message changes shape. The runtime reports the version it
/// was built with in [`RuntimeMessage::Ready`]; a mismatch means a stale
/// binary next to a fresh editor.
pub const PROTOCOL_VERSION: u32 = 1;

/// Editor -> runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum EditorMessage {
    /// Builds (or rebuilds) the world from this project. Any running scripts
    /// are dropped.
    Load {
        project: Box<Project>,
    },
    /// The green flag.
    Start,
    /// Stops every script and puts each actor back where the project says.
    Stop,
    Pause {
        paused: bool,
    },
    /// Close the window and exit.
    Shutdown,
}

/// Runtime -> editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeMessage {
    /// Sent once, as soon as the window is up.
    Ready { protocol: u32 },
    /// A `say` block, or anything else worth showing in the editor's log.
    Say { actor: String, text: String },
    /// A block failed to evaluate. The script carried on regardless.
    Error { actor: String, message: String },
    /// Where everything is, a few times a second - what the editor's actor
    /// inspector and variable watchers display while a project runs.
    Status(Status),
    /// The play session ended through Stop or a `stop all` block.
    Stopped,
    /// The runtime is giving up (a fatal renderer or physics error).
    Fatal { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    pub paused: bool,
    /// Seconds since the green flag.
    pub time: f64,
    pub fps: f32,
    pub actors: Vec<ActorStatus>,
    /// Project-wide variables, by name.
    pub globals: Vec<VariableValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorStatus {
    pub id: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableValue {
    pub name: String,
    pub value: Evaluated,
}

/// One message as a line on the wire, newline included.
pub fn encode<T: Serialize>(message: &T) -> String {
    match serde_json::to_string(message) {
        Ok(json) => format!("{json}\n"),
        // A message that can't be serialized is a bug, not a wire error; the
        // other side would rather see a parse failure than a dropped line.
        Err(e) => format!("{{\"event\":\"fatal\",\"message\":\"{e}\"}}\n"),
    }
}

/// Parses one line. Blank lines are ignored by returning `None`.
pub fn decode<T: for<'de> Deserialize<'de>>(line: &str) -> Option<Result<T, String>> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(serde_json::from_str(line).map_err(|e| format!("{e}: {line}")))
}

/// Name of the runtime binary, which every install ships next to the editor.
pub const RUNTIME_BINARY: &str = if cfg!(windows) {
    "blockloom-runtime.exe"
} else {
    "blockloom-runtime"
};

/// The runtime binary that belongs to this build: the one sitting next to the
/// running executable.
pub fn runtime_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(RUNTIME_BINARY)))
        .unwrap_or_else(|| std::path::PathBuf::from(RUNTIME_BINARY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_round_trips_as_one_line() {
        let message = RuntimeMessage::Say {
            actor: "a1".to_string(),
            text: "hello".to_string(),
        };
        let line = encode(&message);
        assert!(line.ends_with('\n'));
        assert!(!line[..line.len() - 1].contains('\n'));
        assert_eq!(decode::<RuntimeMessage>(&line), Some(Ok(message)));
    }

    #[test]
    fn a_blank_line_is_not_an_error() {
        assert_eq!(decode::<EditorMessage>("  \n"), None);
        assert!(matches!(decode::<EditorMessage>("{oops}"), Some(Err(_))));
    }
}
