//! Blockloom's editor window: a Tauri app on a CEF runtime.
//!
//! Unlike Blockwork, there's no daemon - this process owns the backend
//! (`blockloom-app`) directly, and every frontend command goes straight into
//! `Backend::dispatch`. The one other process is the game world
//! (`blockloom-runtime`), spawned on Play, because Bevy needs an event loop of
//! its own and this one belongs to CEF.

mod theme;

use blockloom_app::{AppHandle as BackendHandle, Backend, Event};
use serde_json::Value;
use tauri::{Emitter, Manager, State};

/// The event the frontend listens on for state snapshots.
const STATE_EVENT: &str = "state-updated";

pub fn run() {
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();

    // CEF re-execs this same binary for its helper processes (renderer, GPU,
    // zygote, ...), tagged with a `--type=` switch. Those must fall straight
    // through to `tauri::Builder::run`, which hands them to
    // `cef::execute_process` and exits - no backend, no window, no runtime.
    let is_cef_subprocess = std::env::args().any(|arg| arg.starts_with("--type="));

    // `--ozone-platform=x11` forces the X11 Ozone platform. The CEF runtime
    // picks Wayland whenever `WAYLAND_DISPLAY` is set, so clear it to keep CEF
    // and winit agreeing on X11; the switch below covers Chromium itself.
    let ozone_platform = if is_cef_subprocess {
        None
    } else {
        ozone_platform_arg()
    };
    if ozone_platform.as_deref() == Some("x11") {
        // SAFETY: cleared before any thread reading the env starts; the CEF
        // runtime only reads it during init, on this thread.
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
        }
    }

    let mut cef = tauri_runtime_cef::Cef::default()
        .command_line_args([("--use-mock-keychain", None::<String>)]);
    if let Some(platform) = &ozone_platform {
        cef = cef.command_line_arg("--ozone-platform", Some(platform.clone()));
    }
    let mut builder = tauri::Builder::default().runtime(cef);

    if !is_cef_subprocess {
        builder = builder.setup(|app| {
            let handle = app.handle().clone();
            // Every state change is re-emitted to the page as one event; the
            // frontend keeps no other copy of the truth.
            let backend = Backend::start(BackendHandle::new(move |event| {
                if let Event::State(json) = event {
                    let _ = handle.emit_str(STATE_EVENT, json.to_string());
                }
            }));
            app.manage(backend);
            theme::create_main_window(app.handle())?;
            Ok(())
        });
    }

    builder
        .invoke_handler(tauri::generate_handler![
            call,
            reset_zoom,
            pick_project_file,
            pick_files,
            pick_folder,
            theme::set_theme_background
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Blockloom window")
        .run(|app, event| {
            // The game window is this process's child: it goes when we go.
            if let tauri::RunEvent::Exit = event
                && let Some(backend) = app.try_state::<Backend>()
            {
                backend.shutdown();
            }
        });
}

#[derive(serde::Serialize)]
struct CallResponse {
    result: Value,
    state: Value,
}

/// Runs a backend command and returns the resulting state with it. The direct
/// snapshot keeps CEF command updates reliable even if an event is delayed.
#[tauri::command]
fn call(backend: State<'_, Backend>, cmd: String, args: Value) -> Result<CallResponse, String> {
    let result = backend.dispatch(&cmd, args)?;
    let state = backend.dispatch("get_state", serde_json::json!({}))?;
    Ok(CallResponse { result, state })
}

/// Resets Chromium page zoom to 100%. Handled here rather than by the browser's
/// own Ctrl+0 accelerator, which this CEF runtime can't be relied on to deliver
/// (opt-in per webview; absent entirely on Alloy-style webviews).
#[tauri::command]
fn reset_zoom(app: tauri::AppHandle) -> Result<(), String> {
    let windows = app.webview_windows();
    if windows.is_empty() {
        return Err("no app window".to_string());
    }
    for window in windows.values() {
        window.set_zoom(1.0).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Shows a file dialog for a `.blockloom` file - a save dialog pre-filled with
/// `default_name` when `save` is set, an open dialog otherwise. Runs here rather
/// than in the backend so the dialog belongs to the editor window. Returns
/// `None` if the user cancelled.
#[tauri::command]
async fn pick_project_file(save: bool, default_name: Option<String>) -> Option<String> {
    let dialog = rfd::AsyncFileDialog::new().add_filter(
        "Blockloom project",
        &[blockloom_core::project::PROJECT_EXTENSION],
    );
    let file = if save {
        dialog
            .set_title("Export project")
            .set_file_name(default_name.unwrap_or_default())
            .save_file()
            .await
    } else {
        dialog.set_title("Import project").pick_file().await
    };
    file.map(|file| file.path().to_string_lossy().into_owned())
}

/// Shows a file dialog for importing assets, which can take several at once.
/// Returns `None` if the user cancelled.
#[tauri::command]
async fn pick_files(title: Option<String>) -> Option<Vec<String>> {
    let files = rfd::AsyncFileDialog::new()
        .set_title(title.unwrap_or_else(|| "Import assets".to_string()))
        .pick_files()
        .await?;
    Some(
        files
            .into_iter()
            .map(|file| file.path().to_string_lossy().into_owned())
            .collect(),
    )
}

/// Shows a folder dialog - where a new project should go, or which project
/// folder to open. Starts at `start` when that folder exists. Returns `None`
/// if the user cancelled.
#[tauri::command]
async fn pick_folder(title: Option<String>, start: Option<String>) -> Option<String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title.unwrap_or_default());
    if let Some(start) = start.filter(|start| std::path::Path::new(start).is_dir()) {
        dialog = dialog.set_directory(start);
    }
    dialog
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned())
}

/// This process's `--ozone-platform` choice, if it was given one.
fn ozone_platform_arg() -> Option<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut chosen = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if let Some(value) = arg.strip_prefix("--ozone-platform=") {
            if !value.is_empty() {
                chosen = Some(value.to_string());
            }
        } else if arg == "--ozone-platform"
            && let Some(next) = args.get(index + 1)
            && !next.is_empty()
            && !next.starts_with('-')
        {
            chosen = Some(next.clone());
            index += 1;
        }
        index += 1;
    }
    chosen
}
