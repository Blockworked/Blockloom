use cxx_qt_build::{CxxQtBuilder, QmlModule};
use qt_build_utils::{QResource, QResourceFile, QResources};

fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.blockworked.BlockloomHub")
            .version(1, 0)
            .qml_files(["qml/Main.qml"]),
    )
    .file("src/application.rs")
    .include_dir("../qt-common")
    .cpp_file("../qt-common/blockloom_application.h")
    .cpp_file("../qt-common/blockloom_application.cpp")
    .cpp_file("src/hub_service.h")
    .cpp_file("src/hub_service.cpp")
    .qrc_resources(QResources::new().resource(fonts()))
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .qt_module("QuickDialogs2")
    .qt_module("Widgets")
    .build();
}

// The editor's font, so the Hub reads the same.
fn fonts() -> QResource {
    let mut fonts = QResource::new().prefix("/fonts");
    for weight in ["Regular", "Medium", "SemiBold", "Bold"] {
        let name = format!("AtkinsonHyperlegibleNext-{weight}.ttf");
        fonts = fonts.file(QResourceFile::new(format!("../res/fonts/{name}")).alias(&name));
    }
    fonts
}
