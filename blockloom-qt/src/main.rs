#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! Blockloom's editor window: a Qt Quick app.
//!
//! This process owns the backend (`blockloom-app`) directly - there's no
//! daemon - and every command from QML goes through `AppBridge` into
//! `Backend::dispatch`. On Linux the game world runs here too, on a thread
//! of its own, drawing into GPU images the Game view shows; on Windows it
//! shares Vulkan images through Win32 handles with the Game view
//! (see `game_view`). Elsewhere it is still a child process
//! (`blockloom-runtime`).

mod app_bridge;
mod app_icon;
mod game_view;
mod preview;
mod qt_diagnostics;

use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQmlEngine, QUrl};
use std::pin::Pin;

fn main() {
    // Every control is drawn from the app's own theme; pin the Basic style so
    // a platform style doesn't add native chrome or palettes underneath it.
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Basic") };
    }
    game_view::prefer_renderer();
    qt_diagnostics::install();
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();

    // Both QML modules are linked in rather than loaded as plugins, so their
    // resources and Rust QObjects are registered by hand.
    blockstitch_qml::init();
    cxx_qt::init_qml_module!("com.blockworked.Blockloom");

    let mut app = QGuiApplication::new();
    app_icon::apply();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        engine
            .as_mut()
            .on_object_creation_failed(|_, url| {
                eprintln!("failed to create QML root object from {url}");
            })
            .release();
        let entry = "qrc:/qt/qml/com/blockworked/Blockloom/qml/Main.qml".to_string();
        // Development harnesses use the real backend with a smaller QML root.
        #[cfg(feature = "qml-preview")]
        let entry = std::env::var("BLOCKLOOM_QML_ENTRY").unwrap_or(entry);
        engine.load(&QUrl::from(entry.as_str()));
        app_icon::apply_to_windows();
        game_view::configure_windows();
    }
    if let Some(engine) = engine.as_mut() {
        let engine: Pin<&mut QQmlEngine> = engine.upcast_pin();
        engine.on_quit(|_| {}).release();
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
