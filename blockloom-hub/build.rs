use cxx_qt_build::{CxxQtBuilder, QmlModule};
use qt_build_utils::{QResource, QResourceFile, QResources};

fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.blockworked.BlockloomHub")
            .version(1, 0)
            .qml_files(["qml/Main.qml"]),
    )
    .cpp_file("src/hub_service.h")
    .cpp_file("src/hub_service.cpp")
    .qrc_resources(
        QResources::new().resource(
            QResource::new()
                .prefix("/hub-service")
                .file(QResourceFile::new("../scripts/hub.py").alias("hub.py"))
                .file(QResourceFile::new("../scripts/hub_install.py").alias("hub_install.py"))
                .file(QResourceFile::new("../scripts/hub_process.py").alias("hub_process.py"))
                .file(QResourceFile::new("../scripts/hub_download.py").alias("hub_download.py"))
                .file(QResourceFile::new("../scripts/hub_github.py").alias("hub_github.py"))
                .file(QResourceFile::new("../scripts/hub_backup.py").alias("hub_backup.py"))
                .file(QResourceFile::new("../scripts/hub_tools.py").alias("hub_tools.py"))
                .file(QResourceFile::new("../scripts/replace.py").alias("replace.py")),
        ).resource(fonts()),
    )
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .qt_module("QuickDialogs2")
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
