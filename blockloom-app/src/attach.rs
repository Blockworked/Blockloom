//! Attach mode: a shell or MCP session driving the editor's own backend.
//!
//! By default every shell boots a second in-memory backend, which forks a
//! silent copy of whatever project the editor has open. Against an editor
//! that serves attach mode, `blockloom-shell --attach` instead forwards each
//! command line to that backend over a local socket, so there is ever one
//! copy of the project and no merge problem by construction.
//!
//! The wire is one JSON object per line each way: the client sends
//! `{"cmd", "args"}` and the server answers `{"ok", "result", "error",
//! "state"}`, the shell's own response shape. Unix only: elsewhere
//! `--attach` says so instead of pretending.

use crate::Backend;
use serde_json::{Value, json};
use std::path::PathBuf;

/// Where the editor serves attach mode: one socket per user, since one
/// editor holds one backend.
pub fn socket_path() -> PathBuf {
    blockloom_core::project::data_dir().join("attach.sock")
}

/// Serves `backend` on [`socket_path`] in a background thread. A second
/// editor stands down with an error rather than stealing the socket, and the
/// folder locks still keep two owners from writing as one.
#[cfg(unix)]
pub fn serve_attach(backend: Backend) -> Result<(), String> {
    use std::os::unix::net::{UnixListener, UnixStream};

    let path = socket_path();
    if path.exists() && UnixStream::connect(&path).is_ok() {
        return Err("Another Blockloom is already serving attach mode".to_string());
    }
    let _ = std::fs::remove_file(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::thread::Builder::new()
        .name("blockloom-attach".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(stream) = conn else { continue };
                let backend = backend.clone();
                std::thread::Builder::new()
                    .name("blockloom-attach-conn".into())
                    .spawn(move || serve_conn(backend, stream))
                    .ok();
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Attach mode has no socket to serve on this platform.
#[cfg(not(unix))]
pub fn serve_attach(_backend: Backend) -> Result<(), String> {
    Err("Attach mode needs a local socket, so it isn't available on this platform".to_string())
}

/// One command arriving over the socket.
#[cfg(unix)]
fn serve_conn(backend: Backend, stream: std::os::unix::net::UnixStream) {
    use std::io::{BufRead, BufReader, Write};
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let mut lines = BufReader::new(reader).lines();
    let mut out = stream;
    while let Some(Ok(line)) = lines.next() {
        let response = serve_line(&backend, &line);
        if writeln!(out, "{response}").is_err() {
            return;
        }
    }
}

/// Runs one attach line and renders the shell-shaped response.
#[cfg(unix)]
fn serve_line(backend: &Backend, line: &str) -> String {
    let request: Value = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(e) => {
            return json!({"ok": false, "result": null, "error": format!("Bad attach request: {e}"), "state": null})
                .to_string();
        }
    };
    let cmd = request.get("cmd").and_then(Value::as_str).unwrap_or("");
    let args = request.get("args").cloned().unwrap_or(Value::Null);
    let args = if args.is_null() { json!({}) } else { args };
    let (ok, result, error) = match backend.dispatch(cmd, args) {
        Ok(result) => (true, result, Value::Null),
        Err(error) => (false, Value::Null, Value::String(error)),
    };
    let state = backend
        .state_json()
        .map(Value::String)
        .unwrap_or(Value::Null);
    let state: Value = match state {
        Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
        other => other,
    };
    json!({"ok": ok, "result": result, "error": error, "state": state}).to_string()
}

/// A shell driving the editor's backend instead of booting its own.
#[cfg(unix)]
pub struct AttachClient {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
    writer: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl AttachClient {
    /// Connects to the editor's attach socket, or says why there is none.
    pub fn connect() -> Result<Self, String> {
        let path = socket_path();
        let writer = std::os::unix::net::UnixStream::connect(&path)
            .map_err(|_| format!("No Blockloom editor is serving attach mode ({}). Open the editor first, or drop --attach to work on your own copy.", path.display()))?;
        let reader = std::io::BufReader::new(
            writer
                .try_clone()
                .map_err(|e| format!("Couldn't talk to the editor: {e}"))?,
        );
        Ok(Self { reader, writer })
    }

    /// Forwards one command and answers the server's whole response object.
    pub fn roundtrip(&mut self, cmd: &str, args: Value) -> Result<Value, String> {
        use std::io::{BufRead, Write};
        let args = if args.is_null() { json!({}) } else { args };
        writeln!(self.writer, "{}", json!({"cmd": cmd, "args": args}))
            .map_err(|e| format!("Couldn't reach the editor: {e}"))?;
        self.writer
            .flush()
            .map_err(|e| format!("Couldn't reach the editor: {e}"))?;
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| format!("Couldn't hear back from the editor: {e}"))?;
        if line.trim().is_empty() {
            return Err("The editor closed the attach connection".to_string());
        }
        serde_json::from_str(&line).map_err(|e| format!("Bad attach reply: {e}"))
    }
}

/// No socket to connect to on this platform.
#[cfg(not(unix))]
pub struct AttachClient;

#[cfg(not(unix))]
impl AttachClient {
    pub fn connect() -> Result<Self, String> {
        Err("Attach mode needs a local socket, so it isn't available on this platform".to_string())
    }

    pub fn roundtrip(&mut self, _cmd: &str, _args: Value) -> Result<Value, String> {
        Err("Attach mode needs a local socket, so it isn't available on this platform".to_string())
    }
}
