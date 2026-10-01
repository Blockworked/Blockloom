//! The editor's view of a device screen: one session per watched device
//! that pushes frames to its host and takes touch and keys back, off the
//! backend's command queue so a slow command never delays a tap.
//!
//! A session streams from the best source it can reach: an emulator's own
//! gRPC service (frames scaled by the emulator, multi-touch back), or else
//! a persistent `screencap` loop over adb (any device). Both feed the same
//! `Event`s; the host decides what to do with them.

mod frames;
mod grpc;
mod shell;
mod stream;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The widest frame a session sends, in pixels.
pub const MIN_WIDTH: u32 = 144;
pub const MAX_WIDTH: u32 = 1080;
pub const DEFAULT_WIDTH: u32 = 540;

/// One pointer's state: pressed, moved while pressed, released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Down,
    Move,
    Up,
}

/// What the viewer's user does to the screen. Points are fractions (0..1)
/// across the frame, so the host needn't know the device's pixels.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Input {
    Touch { phase: Phase, x: f32, y: f32 },
    Key { code: String },
}

/// One frame of the screen: a JPEG data URL and the sizes input maps with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Frame {
    pub serial: String,
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub device_width: u32,
    pub device_height: u32,
    pub seq: u64,
    /// Where the frames come from: `grpc` or `adb`.
    pub transport: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Frame(Frame),
    /// The session ended on its own and says why.
    Failed(String),
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// The device's own pixel size as the latest frame saw it, shared with
/// whatever turns fractions into pixels.
#[derive(Default)]
pub(crate) struct DeviceSize {
    width: AtomicU32,
    height: AtomicU32,
}

impl DeviceSize {
    pub fn set(&self, width: u32, height: u32) {
        self.width.store(width, Ordering::Relaxed);
        self.height.store(height, Ordering::Relaxed);
    }

    pub fn get(&self) -> (u32, u32) {
        (
            self.width.load(Ordering::Relaxed),
            self.height.load(Ordering::Relaxed),
        )
    }

    /// A fractional point on the screen as device pixels, once a size is known.
    pub fn point(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        let (w, h) = self.get();
        (w > 0 && h > 0).then(|| {
            (
                (x.clamp(0.0, 1.0) * (w - 1) as f32).round() as u32,
                (y.clamp(0.0, 1.0) * (h - 1) as f32).round() as u32,
            )
        })
    }
}

/// Whatever turns a pointer into touches on the device. Swapped for the
/// emulator's own once its gRPC connection is up.
pub(crate) trait TouchSink: Send {
    fn touch(&mut self, phase: Phase, x: f32, y: f32);
}

/// The ways a touch can reach the device. The emulator's own, when its
/// gRPC stream is up, wins over the adb shell's.
#[derive(Default)]
pub(crate) struct Routes {
    pub shell: Option<Box<dyn TouchSink>>,
    pub grpc: Option<Box<dyn TouchSink>>,
}

impl Routes {
    fn current(&mut self) -> Option<&mut Box<dyn TouchSink>> {
        self.grpc.as_mut().or(self.shell.as_mut())
    }
}

pub(crate) type Touch = Arc<Mutex<Routes>>;

/// What a transport needs from its session.
pub(crate) struct Context {
    pub adb: std::path::PathBuf,
    pub serial: String,
    pub max_width: u32,
    pub stop: Arc<AtomicBool>,
    pub sink: Sink,
    pub size: Arc<DeviceSize>,
    pub touch: Touch,
}

impl Context {
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// A watched device. Dropping it ends the stream.
pub struct Session {
    stop: Arc<AtomicBool>,
    input: mpsc::Sender<Input>,
}

impl Session {
    /// Starts watching `serial`, sending frames (at most `max_width` wide)
    /// to `sink` until dropped. Never blocks: connecting happens on the
    /// session's own threads, and a failure arrives as `Event::Failed`.
    pub fn start(serial: &str, max_width: u32, sink: Sink) -> Session {
        let stop = Arc::new(AtomicBool::new(false));
        let (input, inbox) = mpsc::channel();
        let serial = serial.to_string();
        let max_width = max_width.clamp(MIN_WIDTH, MAX_WIDTH);
        let session = Session {
            stop: stop.clone(),
            input,
        };
        let spawned = std::thread::Builder::new()
            .name("blockloom-screen".into())
            .spawn(move || {
                let adb = match blockloom_core::android::adb_binary() {
                    Ok(adb) => adb,
                    Err(error) => return sink(Event::Failed(error)),
                };
                let size = Arc::new(DeviceSize::default());
                let touch: Touch = Arc::new(Mutex::new(Routes::default()));
                let ctx = Context {
                    adb,
                    serial,
                    max_width,
                    stop: stop.clone(),
                    sink: sink.clone(),
                    size,
                    touch,
                };
                pump_input(inbox, &ctx);
                transport(&ctx);
            });
        if spawned.is_err() {
            tracing::warn!("Couldn't start the screen session thread");
        }
        session
    }

    /// Queues one input for the device. Never blocks.
    pub fn send(&self, input: Input) {
        let _ = self.input.send(input);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Feeds queued inputs to the device on a thread of its own, so a swipe
/// waiting on adb never holds up the frames. It also opens the adb shell
/// keys (and, absent a better route, touch) go through, off the path of
/// the first frame.
fn pump_input(inbox: mpsc::Receiver<Input>, ctx: &Context) {
    let stop = ctx.stop.clone();
    let sink = ctx.sink.clone();
    let touch = ctx.touch.clone();
    let size = ctx.size.clone();
    let (adb, serial) = (ctx.adb.clone(), ctx.serial.clone());
    let spawned = std::thread::Builder::new()
        .name("blockloom-screen-input".into())
        .spawn(move || {
            let shell = match shell::Shell::start(&adb, &serial) {
                Ok(shell) => Arc::new(Mutex::new(shell)),
                Err(error) => return sink(Event::Failed(error)),
            };
            if let Ok(mut routes) = touch.lock() {
                routes.shell = Some(Box::new(shell::ShellTouch::new(shell.clone(), size)));
            }
            while !stop.load(Ordering::Relaxed) {
                match inbox.recv_timeout(Duration::from_millis(100)) {
                    Ok(Input::Touch { phase, x, y }) => {
                        if let Ok(mut routes) = touch.lock()
                            && let Some(route) = routes.current()
                        {
                            route.touch(phase, x, y);
                        }
                    }
                    Ok(Input::Key { code }) => {
                        let sent = shell.lock().map(|mut s| s.key(&code));
                        if let Ok(Err(error)) = sent {
                            tracing::debug!("{error}");
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    if spawned.is_err() {
        tracing::warn!("Couldn't start the screen input thread");
    }
}

/// Runs the best transport the device offers until the session stops.
fn transport(ctx: &Context) {
    if ctx.serial.starts_with("emulator-") {
        match grpc::run(ctx) {
            Ok(()) => return,
            Err(error) => tracing::debug!("{}: no gRPC screen ({error}), using adb", ctx.serial),
        }
    }
    if ctx.stopped() {
        return;
    }
    if let Err(error) = stream::run(ctx) {
        (ctx.sink)(Event::Failed(error));
    }
}

#[cfg(all(test, unix))]
pub(crate) mod testing {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Writing then executing a script races other threads forking (ETXTBSY);
    /// every stub is written under one lock.
    static WRITING: Mutex<()> = Mutex::new(());

    pub fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let _held = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    pub fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("blockloom-screen-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A context whose frames and failures land in the returned lists.
    pub fn context(adb: PathBuf) -> (Context, Arc<Mutex<Vec<Frame>>>) {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let seen = frames.clone();
        let sink: Sink = Arc::new(move |event| {
            if let Event::Frame(frame) = event {
                seen.lock().unwrap().push(frame);
            }
        });
        let ctx = Context {
            adb,
            serial: "phone-1".to_string(),
            max_width: 8,
            stop: Arc::new(AtomicBool::new(false)),
            sink,
            size: Arc::new(DeviceSize::default()),
            touch: Arc::new(Mutex::new(Routes::default())),
        };
        (ctx, frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_parse_from_the_json_the_panel_sends() {
        let touch: Input =
            serde_json::from_str(r#"{"type":"touch","phase":"down","x":0.25,"y":0.5}"#).unwrap();
        assert_eq!(
            touch,
            Input::Touch {
                phase: Phase::Down,
                x: 0.25,
                y: 0.5
            }
        );
        let key: Input = serde_json::from_str(r#"{"type":"key","code":"back"}"#).unwrap();
        assert_eq!(
            key,
            Input::Key {
                code: "back".into()
            }
        );
        assert!(serde_json::from_str::<Input>(r#"{"type":"nope"}"#).is_err());
    }

    #[test]
    fn points_land_on_pixels_inside_the_screen() {
        let size = DeviceSize::default();
        assert_eq!(size.point(0.5, 0.5), None);
        size.set(1080, 2400);
        assert_eq!(size.point(0.0, 0.0), Some((0, 0)));
        assert_eq!(size.point(1.0, 1.0), Some((1079, 2399)));
        assert_eq!(size.point(-3.0, 9.0), Some((0, 2399)));
    }
}
