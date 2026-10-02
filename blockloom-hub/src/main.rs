#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        // No worker exists yet, so setting the environment is safe.
        unsafe { std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Fusion") };
    }
    cxx_qt::init_qml_module!("com.blockworked.BlockloomHub");
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        engine
            .as_mut()
            .on_object_creation_failed(|_, url| {
                eprintln!("failed to create Hub window from {url}");
                std::process::exit(1);
            })
            .release();
        engine.load(&QUrl::from(
            "qrc:/qt/qml/com/blockworked/BlockloomHub/qml/Main.qml",
        ));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
