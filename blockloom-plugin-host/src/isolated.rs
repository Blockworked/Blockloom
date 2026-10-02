//! A plugin hosted in a child process of its own.
//!
//! A native library runs in whatever process loads it, so one that spins
//! stalls the game and one that crashes takes it down. With isolation on, the
//! library is loaded by `blockloom-plugin-worker` instead, and the host talks
//! to it over the worker's stdin and stdout: length-prefixed JSON frames, the
//! same ops and bytes as in-process. The host services stay on this side: a
//! call the plugin makes to the host crosses the pipe, is answered here
//! against the plugin's capability gate and service hub, and goes back. A call
//! that outlasts its limit, or a worker that dies, stops the module (the owner
//! reloads it) and the game goes on.

use crate::native::ServiceFn;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use blockloom_plugin_api::abi::Status;
use blockloom_plugin_api::loadout::CodeRuntime;
use serde_json::{Value, json};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

/// The environment variable that turns isolation on: `process` hosts native
/// libraries out of process, `all` hosts portable modules there too.
pub const ENV_MODE: &str = "BLOCKLOOM_PLUGIN_ISOLATION";
/// Names the worker binary, when it is not beside the running executable.
pub const ENV_WORKER: &str = "BLOCKLOOM_PLUGIN_WORKER";

/// The most one frame may hold.
const MAX_FRAME: usize = 256 * 1024 * 1024;
/// How long a call may take when its caller gave no limit.
const DEFAULT_LIMIT_MS: u32 = 10_000;

pub fn write_frame(out: &mut impl Write, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    out.write_all(&(bytes.len() as u32).to_le_bytes())?;
    out.write_all(&bytes)?;
    out.flush()
}

/// The next frame, or `None` when the other side closed the pipe.
pub fn read_frame(input: &mut impl Read) -> io::Result<Option<Value>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::other(format!("a {len} byte frame is too large")));
    }
    let mut bytes = vec![0u8; len];
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

/// Whether `runtime` should be hosted out of process, given the mode.
pub fn wants_isolation(runtime: &CodeRuntime) -> bool {
    match std::env::var(ENV_MODE).as_deref() {
        Ok("process") => matches!(runtime, CodeRuntime::Native(_)),
        Ok("all") => true,
        _ => false,
    }
}

/// The worker binary: named by [`ENV_WORKER`], else beside the executable.
pub fn worker_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(ENV_WORKER)
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "blockloom-plugin-worker.exe"
    } else {
        "blockloom-plugin-worker"
    };
    let beside = exe.parent()?.join(name);
    if beside.is_file() {
        return Some(beside);
    }
    // A test binary lives one folder down, in deps/.
    let up = exe.parent()?.parent()?.join(name);
    up.is_file().then_some(up)
}

pub struct IsolatedModule {
    child: Child,
    stdin: ChildStdin,
    frames: Receiver<io::Result<Option<Value>>>,
    services: Box<ServiceFn>,
    logs: Vec<(u32, String)>,
    stopped: bool,
    plugin: String,
}

impl IsolatedModule {
    /// Starts the worker and has it load `runtime`.
    pub fn open(
        runtime: &CodeRuntime,
        plugin: &str,
        services: Box<ServiceFn>,
    ) -> Result<IsolatedModule, String> {
        let worker = worker_path().ok_or_else(|| {
            format!(
                "plugin isolation is on, but no blockloom-plugin-worker was found ({ENV_WORKER})"
            )
        })?;
        Self::open_with(&worker, runtime, plugin, services)
    }

    pub fn open_with(
        worker: &Path,
        runtime: &CodeRuntime,
        plugin: &str,
        services: Box<ServiceFn>,
    ) -> Result<IsolatedModule, String> {
        let mut child = Command::new(worker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("{}: {e}", worker.display()))?;
        let stdin = child.stdin.take().ok_or("the worker has no stdin")?;
        let stdout = child.stdout.take().ok_or("the worker has no stdout")?;
        let (tx, frames) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let frame = read_frame(&mut reader);
                let end = !matches!(frame, Ok(Some(_)));
                if tx.send(frame).is_err() || end {
                    break;
                }
            }
        });
        let mut module = IsolatedModule {
            child,
            stdin,
            frames,
            services,
            logs: Vec::new(),
            stopped: false,
            plugin: plugin.to_string(),
        };
        module
            .send(&json!({"t": "load", "runtime": runtime}))
            .map_err(|e| format!("could not reach the worker: {e}"))?;
        match module.wait(Duration::from_secs(30)) {
            Ok(frame) if frame["ok"] == json!(true) => Ok(module),
            Ok(frame) => Err(frame["error"]
                .as_str()
                .unwrap_or("the worker could not load the plugin")
                .to_string()),
            Err(e) => Err(e),
        }
    }

    fn send(&mut self, frame: &Value) -> io::Result<()> {
        write_frame(&mut self.stdin, frame)
    }

    /// The next `loaded`/`done` frame, answering the plugin's service calls on
    /// the way, within `limit`.
    fn wait(&mut self, limit: Duration) -> Result<Value, String> {
        let deadline = std::time::Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let frame = match self.frames.recv_timeout(left) {
                Ok(Ok(Some(frame))) => frame,
                Ok(Ok(None)) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.lost("its process ended"));
                }
                Ok(Err(e)) => return Err(self.lost(&format!("its pipe failed: {e}"))),
                Err(RecvTimeoutError::Timeout) => {
                    let _ = self.child.kill();
                    return Err(self.lost(&format!(
                        "ran past {} ms in one call and its process was stopped",
                        limit.as_millis()
                    )));
                }
            };
            match frame["t"].as_str() {
                Some("service") => {
                    let name = frame["name"].as_str().unwrap_or("");
                    let input = B64
                        .decode(frame["input"].as_str().unwrap_or(""))
                        .unwrap_or_default();
                    let reply = match (self.services)(name, &input) {
                        Ok(bytes) => {
                            json!({"t": "service_result", "ok": true, "output": B64.encode(bytes)})
                        }
                        Err(status) => {
                            json!({"t": "service_result", "ok": false, "status": status as i32})
                        }
                    };
                    if let Err(e) = self.send(&reply) {
                        return Err(self.lost(&format!("its pipe failed: {e}")));
                    }
                }
                Some("log") => self.take_log_frame(&frame),
                _ => return Ok(frame),
            }
        }
    }

    fn take_log_frame(&mut self, frame: &Value) {
        if let Some(lines) = frame["logs"].as_array() {
            for line in lines {
                if let (Some(level), Some(text)) = (line[0].as_u64(), line[1].as_str()) {
                    self.logs.push((level as u32, text.to_string()));
                }
            }
        }
    }

    fn lost(&mut self, why: &str) -> String {
        self.stopped = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
        format!("{} {why}", self.plugin)
    }

    /// Calls `op`; `limit_ms` bounds the wall time of the whole call, which
    /// is what keeps a spinning native library from stalling the game.
    pub fn call(&mut self, op: &str, input: &[u8], limit_ms: u32) -> Result<Vec<u8>, String> {
        if self.stopped {
            return Err(format!(
                "{op}: the plugin's process was stopped by an earlier fault"
            ));
        }
        let limit_ms = if limit_ms == 0 {
            DEFAULT_LIMIT_MS
        } else {
            limit_ms
        };
        let sent = self.send(&json!({
            "t": "call", "op": op, "input": B64.encode(input), "limit_ms": limit_ms,
        }));
        if let Err(e) = sent {
            return Err(format!(
                "{op}: {}",
                self.lost(&format!("its pipe failed: {e}"))
            ));
        }
        let done = self
            .wait(Duration::from_millis(u64::from(limit_ms)))
            .map_err(|e| format!("{op}: {e}"))?;
        self.take_log_frame(&done);
        if done["ok"] == json!(true) {
            B64.decode(done["output"].as_str().unwrap_or(""))
                .map_err(|e| format!("{op}: {e}"))
        } else {
            Err(done["error"]
                .as_str()
                .map_or_else(|| format!("{op}: Error"), str::to_string))
        }
    }

    pub fn call_json(&mut self, op: &str, input: &Value) -> Result<Value, String> {
        let bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
        let out = self.call(op, &bytes, 0)?;
        if out.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&out).map_err(|e| format!("{op}: {e}"))
    }

    pub fn take_logs(&mut self) -> Vec<(u32, String)> {
        std::mem::take(&mut self.logs)
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped
    }
}

impl Drop for IsolatedModule {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = write_frame(&mut self.stdin, &json!({"t": "shutdown"}));
            // Give the plugin its shutdown call, then make sure it is gone.
            for _ in 0..50 {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What the worker process runs: loads one module and serves calls on
/// stdin/stdout until told to shut down or the host goes away.
pub fn serve() -> i32 {
    use crate::module::CodeModule;
    use std::sync::{Arc, Mutex};

    struct Pipes {
        input: BufReader<io::Stdin>,
        output: Box<dyn Write + Send>,
    }
    // Frames go out on a copy of stdout, and stdout itself is pointed at
    // stderr, so a plugin that prints cannot corrupt the protocol.
    #[cfg(unix)]
    let output: Box<dyn Write + Send> = {
        use std::os::fd::FromRawFd;
        // SAFETY: fd 1 is open; the copy is owned by the File from here on.
        unsafe {
            let copy = libc::dup(1);
            libc::dup2(2, 1);
            Box::new(std::fs::File::from_raw_fd(copy))
        }
    };
    #[cfg(not(unix))]
    let output: Box<dyn Write + Send> = Box::new(io::stdout());
    let pipes = Arc::new(Mutex::new(Pipes {
        input: BufReader::new(io::stdin()),
        output,
    }));
    let send = |pipes: &Mutex<Pipes>, frame: &Value| {
        let mut p = pipes.lock().unwrap_or_else(|e| e.into_inner());
        write_frame(&mut p.output, frame)
    };
    let recv = |pipes: &Mutex<Pipes>| {
        let mut p = pipes.lock().unwrap_or_else(|e| e.into_inner());
        read_frame(&mut p.input)
    };

    let load = match recv(&pipes) {
        Ok(Some(frame)) if frame["t"] == "load" => frame,
        _ => return 2,
    };
    let services: Box<ServiceFn> = {
        let pipes = pipes.clone();
        Box::new(move |name, input| {
            let frame = json!({"t": "service", "name": name, "input": B64.encode(input)});
            {
                let mut p = pipes.lock().unwrap_or_else(|e| e.into_inner());
                write_frame(&mut p.output, &frame).map_err(|_| Status::Error)?;
            }
            let reply = {
                let mut p = pipes.lock().unwrap_or_else(|e| e.into_inner());
                read_frame(&mut p.input).map_err(|_| Status::Error)?
            };
            let reply = reply.ok_or(Status::Error)?;
            if reply["ok"] == json!(true) {
                B64.decode(reply["output"].as_str().unwrap_or(""))
                    .map_err(|_| Status::Error)
            } else {
                Err(Status::from_code(
                    reply["status"].as_i64().unwrap_or(1) as i32
                ))
            }
        })
    };
    let runtime: CodeRuntime = match serde_json::from_value(load["runtime"].clone()) {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = send(
                &pipes,
                &json!({"t": "loaded", "ok": false, "error": e.to_string()}),
            );
            return 2;
        }
    };
    let mut module = match CodeModule::open_in_process(&runtime, services) {
        Ok(module) => module,
        Err(error) => {
            let _ = send(&pipes, &json!({"t": "loaded", "ok": false, "error": error}));
            return 1;
        }
    };
    let _ = send(&pipes, &json!({"t": "loaded", "ok": true}));
    loop {
        let frame = match recv(&pipes) {
            Ok(Some(frame)) => frame,
            _ => return 0,
        };
        match frame["t"].as_str() {
            Some("shutdown") => return 0,
            Some("call") => {
                let op = frame["op"].as_str().unwrap_or("").to_string();
                let input = B64
                    .decode(frame["input"].as_str().unwrap_or(""))
                    .unwrap_or_default();
                let limit = frame["limit_ms"].as_u64().unwrap_or(0) as u32;
                let answer = module.call_bytes(&op, &input, limit);
                let logs: Vec<Value> = module
                    .take_logs()
                    .into_iter()
                    .map(|(level, text)| json!([level, text]))
                    .collect();
                let reply = match answer {
                    Ok(bytes) => {
                        json!({"t": "done", "ok": true, "output": B64.encode(bytes), "logs": logs})
                    }
                    Err(error) => json!({"t": "done", "ok": false, "error": error, "logs": logs}),
                };
                if send(&pipes, &reply).is_err() {
                    return 0;
                }
            }
            _ => {}
        }
    }
}
