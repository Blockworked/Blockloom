#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! Blockloom's editor window: a Qt Quick app.
//!
//! This process owns the backend (`blockloom-app`) directly - there's no
//! daemon - and every command from QML goes through `AppBridge` into
//! `Backend::dispatch`. The one other process is the game world
//! (`blockloom-runtime`), spawned on Play, because Bevy needs an event loop
//! of its own and this one belongs to Qt.

mod app_bridge;
mod app_icon;
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
        engine.load(&QUrl::from("qrc:/qt/qml/com/blockworked/Blockloom/qml/Main.qml"));
        app_icon::apply_to_windows();
    }
    if let Some(engine) = engine.as_mut() {
        let engine: Pin<&mut QQmlEngine> = engine.upcast_pin();
        engine.on_quit(|_| {}).release();
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
