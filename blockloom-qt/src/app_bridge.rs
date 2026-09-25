//! The one QObject QML talks to. It hosts the backend in this process: each
//! command runs on a worker thread, in the order it was sent, and its answer
//! comes back to the Qt thread as `replied`. State snapshots the backend
//! publishes arrive the same way, coalesced so a burst parses once.

use crate::preview;
use blockloom_app::{AppHandle as BackendHandle, Backend, Event};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value, json};
use std::pin::Pin;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, state_json, cxx_name = "stateJson")]
        #[qproperty(QString, status_json, cxx_name = "statusJson")]
        #[qproperty(QString, log_json, cxx_name = "logJson")]
        #[qproperty(QString, preview_frame, cxx_name = "previewFrame")]
        #[qproperty(QString, app_version, cxx_name = "appVersion")]
        type AppBridge = super::AppBridgeRust;

        /// Starts the backend. Safe to call more than once.
        #[qinvokable]
        fn start(self: Pin<&mut AppBridge>);

        /// Queues `command` with JSON `arguments`; the answer comes back as
        /// `replied(token, {ok, result, error})`.
        #[qinvokable]
        #[cxx_name = "invokeCommand"]
        fn invoke_command(self: Pin<&mut AppBridge>, token: i32, command: &QString, arguments: &QString);

        /// Follows the runtime's preview stream on `port`, or stops following
        /// it when `port` is 0.
        #[qinvokable]
        #[cxx_name = "watchPreview"]
        fn watch_preview(self: Pin<&mut AppBridge>, port: i32);

        /// Closes the game window, if one is open. Called as the editor exits.
        #[qinvokable]
        fn shutdown(self: Pin<&mut AppBridge>);

        /// The Bevy `KeyCode` name of the key behind a key event's
        /// `nativeScanCode`, or "" where the platform gives none.
        #[qinvokable]
        #[cxx_name = "physicalKey"]
        fn physical_key(self: &AppBridge, scan_code: i32) -> QString;
    }

    unsafe extern "RustQt" {
        #[qsignal]
        fn replied(self: Pin<&mut AppBridge>, token: i32, response: QString);
    }

    impl cxx_qt::Threading for AppBridge {}
}

struct Job {
    token: i32,
    command: String,
    args: Value,
}

pub struct AppBridgeRust {
    state_json: QString,
    status_json: QString,
    log_json: QString,
    preview_frame: QString,
    app_version: QString,
    backend: Option<Backend>,
    jobs: Option<mpsc::Sender<Job>>,
    preview: Option<preview::Watch>,
}

impl Default for AppBridgeRust {
    fn default() -> Self {
        Self {
            state_json: QString::from("{}"),
            status_json: QString::default(),
            log_json: QString::default(),
            preview_frame: QString::default(),
            app_version: QString::from(env!("CARGO_PKG_VERSION")),
            backend: None,
            jobs: None,
            preview: None,
        }
    }
}

/// The latest value waiting to be handed to the Qt thread. Only one closure
/// is queued per burst, and it takes whatever is newest when it runs.
type Latest = Arc<Mutex<Option<String>>>;

fn post_latest(
    slot: &Latest,
    thread: &cxx_qt::CxxQtThread<qobject::AppBridge>,
    value: String,
    apply: fn(Pin<&mut qobject::AppBridge>, String),
) {
    let Ok(mut held) = slot.lock() else { return };
    let already_queued = held.is_some();
    *held = Some(value);
    if already_queued {
        return;
    }
    drop(held);
    let slot = slot.clone();
    let _ = thread.queue(move |bridge| {
        let taken = slot.lock().ok().and_then(|mut held| held.take());
        if let Some(value) = taken {
            apply(bridge, value);
        }
    });
}

fn apply_state(bridge: Pin<&mut qobject::AppBridge>, json: String) {
    bridge.set_state_json(QString::from(&json));
}

fn apply_status(bridge: Pin<&mut qobject::AppBridge>, json: String) {
    bridge.set_status_json(QString::from(&json));
}

fn apply_log(bridge: Pin<&mut qobject::AppBridge>, json: String) {
    bridge.set_log_json(QString::from(&json));
}

impl qobject::AppBridge {
    pub fn start(mut self: Pin<&mut Self>) {
        if self.rust().backend.is_some() {
            return;
        }
        let thread = self.qt_thread();
        let pending: Latest = Arc::new(Mutex::new(None));
        let status: Latest = Arc::new(Mutex::new(None));
        let log: Latest = Arc::new(Mutex::new(None));
        let sink_thread = thread.clone();
        let sink_pending = pending.clone();
        let handle = BackendHandle::new(move |event| match event {
            Event::State(json) => {
                post_latest(&sink_pending, &sink_thread, json.to_string(), apply_state);
            }
            Event::Status(json) => {
                post_latest(&status, &sink_thread, json.to_string(), apply_status);
            }
            Event::Log(json) => {
                post_latest(&log, &sink_thread, json.to_string(), apply_log);
            }
            Event::RuntimeClosed => {}
        });
        let backend = match crate::game_view::host() {
            Some(host) => Backend::start_embedded(handle, host),
            None => Backend::start(handle),
        };

        let (tx, rx) = mpsc::channel::<Job>();
        let worker = backend.clone();
        std::thread::Builder::new()
            .name("blockloom-backend".into())
            .spawn(move || {
                for job in rx {
                    let response = match worker.dispatch(&job.command, job.args) {
                        Ok(result) => json!({ "ok": true, "result": result }),
                        Err(error) => json!({ "ok": false, "error": error }),
                    };
                    let token = job.token;
                    let text = response.to_string();
                    let _ = thread.queue(move |bridge| {
                        bridge.replied(token, QString::from(&text));
                    });
                }
            })
            .expect("failed to start the backend thread");

        if let Ok(json) = backend.state_json() {
            self.as_mut().set_state_json(QString::from(&json));
        }
        self.as_mut().rust_mut().backend = Some(backend);
        self.as_mut().rust_mut().jobs = Some(tx);
    }

    pub fn invoke_command(self: Pin<&mut Self>, token: i32, command: &QString, arguments: &QString) {
        let args = match serde_json::from_str::<Value>(&arguments.to_string()) {
            Ok(args) => args,
            Err(error) => {
                let text = json!({ "ok": false, "error": format!("Invalid command arguments: {error}") });
                self.replied(token, QString::from(&text.to_string()));
                return;
            }
        };
        let job = Job { token, command: command.to_string(), args };
        let sent = self.rust().jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok());
        if !sent {
            let text = json!({ "ok": false, "error": "The backend isn't running" });
            self.replied(token, QString::from(&text.to_string()));
        }
    }

    pub fn watch_preview(mut self: Pin<&mut Self>, port: i32) {
        // Dropping the old watch stops its thread.
        self.as_mut().rust_mut().preview = None;
        if port <= 0 {
            self.as_mut().set_preview_frame(QString::default());
            return;
        }
        let thread = self.qt_thread();
        let latest: Latest = Arc::new(Mutex::new(None));
        let watch = preview::Watch::start(port as u16, move |frame| {
            post_latest(&latest, &thread, frame, |bridge, frame| {
                bridge.set_preview_frame(QString::from(&frame));
            });
        });
        self.as_mut().rust_mut().preview = Some(watch);
    }

    pub fn physical_key(&self, scan_code: i32) -> QString {
        let name = u32::try_from(scan_code)
            .ok()
            .and_then(blockloom_protocol::keys::physical_key);
        QString::from(name.unwrap_or_default())
    }

    pub fn shutdown(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().preview = None;
        if let Some(backend) = &self.rust().backend {
            backend.shutdown();
        }
    }
}
