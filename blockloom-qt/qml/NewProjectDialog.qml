import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Naming a new project, choosing where it goes and which dimension it is.
// The folder name is not a field: the app names the folder after the project.
BwDialog {
    id: root
    required property var app
    property string mode: "TwoD"
    property string error: ""
    property bool busy: false
    title: "New project"
    standardButtons: Dialog.NoButton

    onOpened: {
        nameField.text = "My Game";
        locationField.text = app.appState.default_project_location;
        mode = "TwoD"; error = ""; busy = false;
        nameField.forceActiveFocus(); nameField.selectAll();
    }
    // What the project's own folder will be called: anything a path can't
    // hold is shown as the `-` it becomes.
    readonly property string folder: {
        const cleaned = nameField.text.replace(/[\/\\:*?"<>|]/g, "-").trim().replace(/^\.+|\.+$/g, "").trim();
        return cleaned === "" ? "Project" : cleaned;
    }
    readonly property string fullPath: {
        const loc = locationField.text;
        const sep = loc.indexOf("\\") >= 0 ? "\\" : "/";
        return loc.replace(/[\/\\]+$/, "") + sep + folder;
    }
    function submit() {
        if (busy) return;
        if (!nameField.text.trim().length) { error = "Give the project a name"; return; }
        busy = true;
        app.invoke("create_project", { name: nameField.text.trim(), location: locationField.text.trim(), mode: mode },
            () => { busy = false; root.close(); }, e => { busy = false; error = String(e); });
    }

    ColumnLayout {
        width: 460; spacing: 8
        Text { visible: root.error.length > 0; text: root.error; color: Theme.danger; Layout.fillWidth: true; wrapMode: Text.WordWrap }
        Text { text: "Name"; color: Theme.textDim; font.pixelSize: 12 }
        BwTextField { id: nameField; Layout.fillWidth: true; placeholderText: "My Game"; onAccepted: root.submit() }
        Text { text: "Where to keep it"; color: Theme.textDim; font.pixelSize: 12 }
        RowLayout {
            Layout.fillWidth: true
            BwTextField { id: locationField; Layout.fillWidth: true; onAccepted: root.submit() }
            IconButton { iconName: "folder-open"; tip: "Browse for a folder"; flat: false; implicitWidth: 34; implicitHeight: 34; onClicked: browse.open() }
        }
        Text { Layout.fillWidth: true; wrapMode: Text.WrapAnywhere; color: Theme.textDim; font.pixelSize: 12; text: "The project gets a folder of its own: " + root.fullPath }
        Text { text: "Dimension"; color: Theme.textDim; font.pixelSize: 12 }
        RowLayout {
            spacing: 6
            BwButton { text: "2D"; iconName: "square"; primary: root.mode === "TwoD"; onClicked: root.mode = "TwoD" }
            BwButton { text: "3D"; iconName: "box"; primary: root.mode === "ThreeD"; onClicked: root.mode = "ThreeD" }
        }
        Text {
            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: (root.mode === "TwoD" ? "Sprites and flat physics, measured in pixels." : "Meshes and 3D physics, measured in metres.") + " A project can switch later."
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight; Layout.topMargin: 8; spacing: 8
            BwButton { text: "Cancel"; onClicked: root.close() }
            BwButton { text: "Create"; primary: true; enabled: !root.busy; onClicked: root.submit() }
        }
    }
    FolderDialog {
        id: browse
        title: "Where to keep the project"
        currentFolder: root.app.toFileUrl(locationField.text)
        onAccepted: locationField.text = root.app.fromFileUrl(selectedFolder)
    }
}
