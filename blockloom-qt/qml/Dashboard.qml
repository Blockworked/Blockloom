import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Where the app starts: every project Blockloom knows about, and the ways to
// get another one. Opening a project swaps this page for the editor.
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    property string error: ""
    color: Theme.window

    function report(command, args) { root.error = ""; app.invoke(command, args, null, e => root.error = String(e)); }
    // The tail of a path, since the front of a long one says the least.
    function where(path) {
        const parts = path.split(/[\/\\]/).filter(p => p.length);
        return parts.length <= 3 ? path : "…/" + parts.slice(-3).join("/");
    }
    // "today", "3 days ago" - enough to sort the pile by eye.
    function opened(entry) {
        if (!entry.opened_at) return "never opened";
        const days = Math.floor((Date.now() / 1000 - entry.opened_at) / 86400);
        if (days <= 0) return "opened today";
        if (days === 1) return "opened yesterday";
        if (days < 30) return "opened " + days + " days ago";
        return "opened " + new Date(entry.opened_at * 1000).toLocaleDateString();
    }

    ColumnLayout {
        anchors.fill: parent; anchors.margins: 36; spacing: 18
        RowLayout {
            Layout.fillWidth: true
            ColumnLayout {
                spacing: 2
                Text { text: "Blockloom"; color: Theme.text; font.pixelSize: 24; font.weight: Font.Bold }
                Text { text: "Build a game out of blocks."; color: Theme.textDim; font.pixelSize: 14 }
            }
            Item { Layout.fillWidth: true }
            Text { text: app.appVersion; color: Theme.textDim; font.pixelSize: 12 }
            IconButton { iconName: "settings"; tip: "App settings"; implicitWidth: 30; implicitHeight: 30; onClicked: appSettings.open() }
        }
        Rectangle {
            visible: !root.appState.runtime_available
            Layout.fillWidth: true; implicitHeight: runtimeNote.implicitHeight + 20; radius: 8; color: Theme.warningBackground; border.color: Theme.warningBorder
            LucideIcon { id: runtimeIcon; name: "triangle-alert"; color: Theme.text; width: 16; height: 16; anchors.left: parent.left; anchors.leftMargin: 12; anchors.verticalCenter: parent.verticalCenter }
            Text { id: runtimeNote; anchors.fill: parent; anchors.margins: 10; anchors.leftMargin: 38; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 13
                text: "The game runtime is missing, so Play will have nothing to open. Build the whole workspace (just build), not only the editor." }
        }
        RowLayout {
            spacing: 10
            StitchButton { primary: true; iconName: "plus"; text: "Create project"; onClicked: newProject.open() }
            StitchButton { iconName: "folder-open"; text: "Open a folder"; onClicked: openFolder.open() }
            StitchButton { iconName: "upload"; text: "Import a file"; onClicked: importFile.open() }
        }
        Text { visible: root.error.length > 0; text: root.error; color: Theme.danger; font.pixelSize: 13; Layout.fillWidth: true; wrapMode: Text.WordWrap }
        ColumnLayout {
            visible: root.appState.library.length === 0
            Layout.fillWidth: true; Layout.topMargin: 24; spacing: 6
            Text { text: "No projects yet."; color: Theme.text; font.pixelSize: 16; font.weight: Font.DemiBold }
            Text { text: "Create one and it gets a folder of its own, assets and all."; color: Theme.textDim; font.pixelSize: 13 }
        }
        Text { visible: root.appState.library.length > 0; text: "Projects"; color: Theme.textDim; font.pixelSize: 13; font.weight: Font.DemiBold }
        ListView {
            id: list
            visible: root.appState.library.length > 0
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: root.appState.library
            ScrollBar.vertical: ScrollBar {}
            delegate: Rectangle {
                id: row
                required property var modelData
                width: list.width; height: 56
                color: rowHover.hovered ? Theme.hover : "transparent"
                HoverHandler { id: rowHover; cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: root.report("open_project", { path: row.modelData.path }) }
                Rectangle { anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom; height: 1; color: Theme.borderSoft }
                RowLayout {
                    anchors.fill: parent; anchors.leftMargin: 12; anchors.rightMargin: 8; spacing: 12
                    LucideIcon { name: row.modelData.mode === "ThreeD" ? "box" : "square"; color: Theme.textDim; width: 18; height: 18 }
                    ColumnLayout {
                        Layout.fillWidth: true; spacing: 2
                        Text { Layout.fillWidth: true; text: row.modelData.name; color: Theme.text; font.pixelSize: 14; font.weight: Font.DemiBold; elide: Text.ElideRight }
                        Text { Layout.fillWidth: true; text: root.where(row.modelData.path); color: Theme.textDim; font.pixelSize: 12; elide: Text.ElideMiddle
                            ToolTip.visible: pathHover.hovered; ToolTip.text: row.modelData.path; ToolTip.delay: 600
                            HoverHandler { id: pathHover } }
                    }
                    Text { text: row.modelData.mode === "ThreeD" ? "3D" : "2D"; color: Theme.textDim; font.pixelSize: 12 }
                    Text { Layout.preferredWidth: 140; horizontalAlignment: Text.AlignRight; text: root.opened(row.modelData); color: Theme.textDim; font.pixelSize: 12; elide: Text.ElideRight }
                    IconButton { iconName: "x"; tip: "Remove from this list"; implicitWidth: 28; implicitHeight: 28; onClicked: root.report("forget_project", { path: row.modelData.path }) }
                    IconButton { iconName: "trash-2"; danger: true; tip: "Delete the project folder"; implicitWidth: 28; implicitHeight: 28; onClicked: { deleteDialog.entry = row.modelData; deleteDialog.open(); } }
                }
            }
        }
        Item { visible: root.appState.library.length === 0; Layout.fillHeight: true }
    }

    NewProjectDialog { id: newProject; app: root.app }
    AppSettingsDialog { id: appSettings; app: root.app }
    FolderDialog {
        id: openFolder
        title: "Open project"
        currentFolder: root.app.toFileUrl(root.appState.default_project_location)
        onAccepted: root.report("open_project", { path: root.app.fromFileUrl(selectedFolder) })
    }
    FileDialog {
        id: importFile
        title: "Import project"
        nameFilters: ["Blockloom project (*.blockloom)"]
        onAccepted: root.report("import_project", { path: root.app.fromFileUrl(selectedFile) })
    }
    StitchDialog {
        id: deleteDialog
        property var entry: null
        title: "Delete project?"
        standardButtons: Dialog.Yes | Dialog.Cancel
        Text {
            width: 420; wrapMode: Text.WordWrap; color: Theme.text
            text: deleteDialog.entry ? "Delete \"" + deleteDialog.entry.name + "\" and everything in its folder?\n\n" + deleteDialog.entry.path + "\n\nThis cannot be undone." : ""
        }
        onAccepted: if (entry) root.report("delete_project", { path: entry.path })
    }
}
