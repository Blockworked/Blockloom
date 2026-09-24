import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The open project's name, the way back to the Dashboard, the door to the
// project settings, and the run controls.
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    implicitHeight: 52
    color: Theme.panel
    border.color: Theme.borderSoft

    // A command that fails has nowhere else to say so - the run log is where
    // the user is already looking for what went wrong.
    function report(command, args) { app.invoke(command, args, null, e => app.invoke("push_log", { kind: "error", text: String(e) })); }

    RowLayout {
        anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 10; spacing: 4
        IconButton { iconName: "layout-grid"; tip: "All projects"; onClicked: root.report("close_project") }
        BwTextField {
            Layout.preferredWidth: 240
            text: root.appState.project ? root.appState.project.name : ""
            placeholderText: "Project name"
            ToolTip.visible: hovered && !!root.appState.project_path; ToolTip.text: root.appState.project_path || ""; ToolTip.delay: 700
            onEditingFinished: if (root.appState.project && text !== root.appState.project.name) root.report("set_project_name", { name: text })
        }
        IconButton { iconName: "save"; tip: "Save now"; onClicked: root.app.invoke("save_project") }
        IconButton { iconName: "upload"; tip: "Import a project"; onClicked: importFile.open() }
        IconButton { iconName: "download"; tip: "Export this project"; onClicked: root.app.invoke("export_file_name", {}, name => { exportFile.currentFile = root.app.toFileUrl(root.appState.default_project_location + "/" + name); exportFile.open(); }) }
        IconButton { iconName: "package"; tip: "Build a standalone game"; onClicked: buildDialog.open() }
        IconButton { iconName: "settings"; tip: "Project settings"; onClicked: settingsDialog.open() }
        Item { Layout.fillWidth: true }
        IconButton { iconName: "undo-2"; tip: "Undo"; enabled: root.appState.can_undo; onClicked: root.app.invoke("undo") }
        IconButton { iconName: "redo-2"; tip: "Redo"; enabled: root.appState.can_redo; onClicked: root.app.invoke("redo") }
        Rectangle {
            visible: root.appState.running
            implicitWidth: fpsText.implicitWidth + 16; implicitHeight: 24; radius: 12; color: Theme.panelRaised; border.color: Theme.border
            Text { id: fpsText; anchors.centerIn: parent; text: Math.round(root.appState.status ? root.appState.status.fps : 0) + " fps"; color: Theme.textDim; font.pixelSize: 11 }
        }
        IconButton { visible: root.appState.runtime_open; iconName: "monitor-x"; tip: "Close the game window"; onClicked: root.app.invoke("close_runtime") }
        IconButton {
            visible: root.appState.running
            iconName: root.appState.paused ? "play" : "pause"; tip: root.appState.paused ? "Resume" : "Pause"
            onClicked: root.app.invoke("pause_project", { paused: !root.appState.paused })
        }
        IconButton { visible: root.appState.running && root.appState.paused; iconName: "step-forward"; tip: "Advance one tick"; onClicked: root.report("step_project") }
        BwButton {
            Layout.leftMargin: 6
            primary: !root.appState.running; danger: root.appState.running
            iconName: root.appState.running ? "square" : "play"
            text: root.appState.running ? "Stop" : "Play"
            onClicked: root.report(root.appState.running ? "stop_project" : "run_project")
        }
    }

    FileDialog {
        id: importFile
        title: "Import project"
        nameFilters: ["Blockloom project (*.blockloom)"]
        onAccepted: root.report("import_project", { path: root.app.fromFileUrl(selectedFile) })
    }
    FileDialog {
        id: exportFile
        title: "Export project"
        fileMode: FileDialog.SaveFile
        nameFilters: ["Blockloom project (*.blockloom)"]
        onAccepted: root.report("export_project", { path: root.app.fromFileUrl(selectedFile) })
    }
    BuildDialog { id: buildDialog; app: root.app }
    ProjectSettingsDialog { id: settingsDialog; app: root.app }
}
