use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};
use qt_build_utils::{QResource, QResourceFile, QResources};

fn main() {
    #[cfg(windows)]
    {
        // Explorer and the taskbar pick the exe's embedded icon; the runtime
        // title-bar icon comes from the Qt resource below.
        let mut res = winres::WindowsResource::new();
        res.set_icon("../src-tauri/icons/icon.ico");
        res.compile().expect("failed to embed Windows icon");
    }

    let builder = CxxQtBuilder::new_qml_module(
        QmlModule::new("com.blockworked.Blockloom")
            .version(1, 0)
            .qml_file(QmlFile::from("qml/Blocks.qml").singleton(true))
            .qml_files([
                "qml/Main.qml",
                "qml/Dashboard.qml",
                "qml/NewProjectDialog.qml",
                "qml/EditorPage.qml",
                "qml/TopBar.qml",
                "qml/ActorList.qml",
                "qml/BlockSidebar.qml",
                "qml/InspectorPanel.qml",
                "qml/InspectorRow.qml",
                "qml/NumberField.qml",
                "qml/ColorField.qml",
                "qml/AssetField.qml",
                "qml/ChoiceField.qml",
                "qml/SwitchField.qml",
                "qml/SliderField.qml",
                "qml/AssetTray.qml",
                "qml/RunLog.qml",
                "qml/SoundPreview.qml",
                "qml/PreviewPanel.qml",
                "qml/ProjectSettingsDialog.qml",
                "qml/BuildDialog.qml",
                "qml/ScriptDialog.qml",
                "qml/MakeBlockDialog.qml",
                "qml/SectionLabel.qml",
                "qml/IconButton.qml",
            ]),
    )
    .file("src/app_bridge.rs")
    .file("src/app_icon.rs")
    .file("src/qt_diagnostics.rs")
    // Runtime window icon (see src/app_icon.cpp, addressed as ":/icons/...").
    .qrc_resources(QResources::new().resource(
        QResource::new()
            .prefix("/icons")
            .file(QResourceFile::new("../res/icons/blockloom.png").alias("blockloom.png")),
    ))
    .qt_module("QuickControls2")
    .qt_module("QuickDialogs2");

    unsafe {
        builder
            .cc_builder(|cc| {
                cc.include("src");
                cc.file("src/qt_diagnostics.cpp");
                cc.file("src/app_icon.cpp");
            })
            .build();
    }
}
