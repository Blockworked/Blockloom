#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod application;

use cxx_qt_lib::{QQmlApplicationEngine, QUrl};

fn main() {
    // The Hub window runs its project and editor logic as itself with this
    // flag, so there is one tool to ship and no Qt in the command path.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--hubctl") {
        std::process::exit(blockloom_hub_state::cli::run(&args[1..]));
    }
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        // No worker exists yet, so setting the environment is safe.
        unsafe { std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Basic") };
    }
    blockstitch_qml::init();
    cxx_qt::init_qml_module!("com.blockworked.BlockloomHub");
    let mut app = application::new();
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
    let code = app.as_mut().map_or(1, |app| app.exec());
    drop(engine);
    drop(app);
    std::process::exit(code);
}
