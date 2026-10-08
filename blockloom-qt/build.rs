use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};
use qt_build_utils::{QResource, QResourceFile, QResources};

fn main() {
    #[cfg(windows)]
    {
        // Explorer and the taskbar pick the exe's embedded icon; the runtime
        // title-bar icon comes from the Qt resource below.
        let mut res = winres::WindowsResource::new();
        res.set_icon("../res/icons/icon.ico");
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
                "qml/UiDesigner.qml",
                "qml/UiLayoutInspector.qml",
                "qml/InterfaceGeometry.qml",
                "qml/TopBar.qml",
                "qml/ActorList.qml",
                "qml/BlockSidebar.qml",
                "qml/InspectorPanel.qml",
                "qml/InspectorComponentCard.qml",
                "qml/RigidbodyForm.qml",
                "qml/ColliderForm.qml",
                "qml/CharacterControllerForm.qml",
                "qml/CharacterMotorForm.qml",
                "qml/ConstraintForm.qml",
                "qml/PhysicsUpgradeCard.qml",
                "qml/PlayerCameraForm.qml",
                "qml/PlayerSetupCard.qml",
                "qml/FlipbookStrip.qml",
                "qml/InspectorRow.qml",
                "qml/SurfaceDetailRows.qml",
                "qml/EmitterGraphRows.qml",
                "qml/CurveField.qml",
                "qml/DirectorTrackField.qml",
                "qml/NumberField.qml",
                "qml/ColorField.qml",
                "qml/HdrColorField.qml",
                "qml/AssetField.qml",
                "qml/ChoiceField.qml",
                "qml/SwitchField.qml",
                "qml/SliderField.qml",
                "qml/AssetTray.qml",
                "qml/RunLog.qml",
                "qml/SoundPreview.qml",
                "qml/PreviewPanel.qml",
                "qml/ProjectSettingsDialog.qml",
                "qml/SettingsFields.qml",
                "qml/AppSettingsDialog.qml",
                "qml/BuildDialog.qml",
                "qml/PluginManagerDialog.qml",
                "qml/PluginPanelDialog.qml",
                "qml/PluginEditorsDialog.qml",
                "qml/PluginRecordForm.qml",
                "qml/PluginValueEditor.qml",
                "qml/BuildProgress.qml",
                "qml/DevicePanel.qml",
                "qml/DevicesPanel.qml",
                "qml/ScriptDialog.qml",
                "qml/MakeBlockDialog.qml",
                "qml/SectionLabel.qml",
                "qml/IconButton.qml",
            ]),
    )
    .file("src/app_bridge.rs")
    .file("src/app_icon.rs")
    .file("src/qt_diagnostics.rs")
    .file("src/game_view.rs")
    // The Game view item: moc'd into this QML module, then compiled.
    .cpp_file("src/game_view.h")
    .cpp_file("src/game_view.cpp")
    // Runtime window icon (see src/app_icon.cpp, addressed as ":/icons/...").
    .qrc_resources(
        QResources::new().resource(
            QResource::new()
                .prefix("/icons")
                .file(QResourceFile::new("../res/icons/blockloom.png").alias("blockloom.png")),
        ),
    )
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .qt_module("QuickDialogs2");

    // Frames arrive as dma-bufs imported through EGL, and the pointer is
    // locked through Wayland's own protocols.
    let linux = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    if linux {
        println!("cargo:rustc-link-lib=EGL");
        println!("cargo:rustc-link-lib=wayland-client");
    }
    let qpa = qt_private_headers("QtGui");

    unsafe {
        builder
            .cc_builder(|cc| {
                cc.include("src");
                if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
                    let headers = std::env::var_os("VULKAN_SDK")
                        .map(|sdk| std::path::PathBuf::from(sdk).join("Include"))
                        .filter(|path| path.join("vulkan/vulkan.h").is_file())
                        .unwrap_or_else(|| "../.patched-deps/vulkan-headers/include".into());
                    assert!(
                        headers.join("vulkan/vulkan.h").is_file(),
                        "Vulkan headers missing; run just prepare-patched-deps"
                    );
                    cc.include(headers);
                    cc.file("src/game_view_vulkan.cpp");
                }
                if std::env::var_os("CARGO_FEATURE_QML_PREVIEW").is_some() {
                    cc.define("QT_QML_DEBUG", None);
                }
                cc.file("src/qt_diagnostics.cpp");
                cc.file("src/app_icon.cpp");
                cc.file("src/pointer_lock.cpp");
                if let Some(qpa) = &qpa {
                    cc.include(qpa);
                }
                if linux {
                    cc.file("src/wayland/pointer-constraints-unstable-v1-protocol.cpp");
                    cc.file("src/wayland/relative-pointer-unstable-v1-protocol.cpp");
                    cc.file("src/wayland/viewporter-protocol.cpp");
                }
            })
            .build();
    }
}

/// `<headers>/<module>/<version>`, where Qt keeps the QPA headers
/// (`QtGui/qpa/...`) the Wayland surface is reached through.
fn qt_private_headers(module: &str) -> Option<std::path::PathBuf> {
    let qmake = std::env::var("QMAKE").ok();
    let query = |key: &str| {
        qmake
            .iter()
            .map(String::as_str)
            .chain(["qmake6", "qmake"])
            .find_map(|qmake| {
                let out = std::process::Command::new(qmake)
                    .args(["-query", key])
                    .output()
                    .ok()?;
                out.status
                    .success()
                    .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
            })
    };
    let dir = std::path::Path::new(&query("QT_INSTALL_HEADERS")?)
        .join(module)
        .join(query("QT_VERSION")?);
    dir.is_dir().then_some(dir)
}
