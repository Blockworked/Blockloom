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
                Text { text: "Blockloom"; color: Theme.text; font.pixelSize: 30; font.weight: Font.Bold }
                Text { text: "Build a game out of blocks."; color: Theme.textDim; font.pixelSize: 14 }
            }
            Item { Layout.fillWidth: true }
            Text { text: app.appVersion; color: Theme.textDim; font.pixelSize: 12 }
        }
        Rectangle {
            visible: !root.appState.runtime_available
            Layout.fillWidth: true; implicitHeight: runtimeNote.implicitHeight + 20; radius: 8; color: "#4a3a1c"; border.color: Theme.warning
            Text { id: runtimeNote; anchors.fill: parent; anchors.margins: 10; wrapMode: Text.WordWrap; color: "#ffd89a"; font.pixelSize: 13
                text: "The game runtime is missing, so Play will have nothing to open. Build the whole workspace (just build), not only the editor." }
        }
        RowLayout {
            spacing: 10
            BwButton { primary: true; iconName: "plus"; text: "Create project"; onClicked: newProject.open() }
            BwButton { iconName: "folder-open"; text: "Open a folder"; onClicked: openFolder.open() }
            BwButton { iconName: "upload"; text: "Import a file"; onClicked: importFile.open() }
        }
        Text { visible: root.error.length > 0; text: root.error; color: Theme.danger; font.pixelSize: 13; Layout.fillWidth: true; wrapMode: Text.WordWrap }
        ColumnLayout {
            visible: root.appState.library.length === 0
            Layout.fillWidth: true; Layout.topMargin: 40; spacing: 6
            Text { Layout.alignment: Qt.AlignHCenter; text: "No projects yet."; color: Theme.text; font.pixelSize: 16; font.weight: Font.DemiBold }
            Text { Layout.alignment: Qt.AlignHCenter; text: "Create one and it gets a folder of its own, assets and all."; color: Theme.textDim; font.pixelSize: 13 }
        }
        GridView {
            id: grid
            visible: root.appState.library.length > 0
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            cellWidth: 300; cellHeight: 128
            model: root.appState.library
            ScrollBar.vertical: ScrollBar {}
            delegate: Item {
                id: card
                required property var modelData
                width: grid.cellWidth; height: grid.cellHeight
                Rectangle {
                    anchors.fill: parent; anchors.margins: 6; radius: 10
                    color: cardHover.hovered ? "#303134" : Theme.panel; border.color: cardHover.hovered ? Theme.accent : Theme.borderSoft
                    HoverHandler { id: cardHover; cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: root.report("open_project", { path: card.modelData.path }) }
                    ColumnLayout {
                        anchors.fill: parent; anchors.margins: 14; spacing: 6
                        RowLayout {
                            Layout.fillWidth: true
                            Text { Layout.fillWidth: true; text: card.modelData.name; color: Theme.text; font.pixelSize: 15; font.weight: Font.DemiBold; elide: Text.ElideRight }
                            LucideIcon { name: card.modelData.mode === "ThreeD" ? "box" : "square"; color: Theme.textDim; width: 14; height: 14 }
                            Text { text: card.modelData.mode === "ThreeD" ? "3D" : "2D"; color: Theme.textDim; font.pixelSize: 12 }
                        }
                        Text { Layout.fillWidth: true; text: root.where(card.modelData.path); color: Theme.textDim; font.pixelSize: 12; elide: Text.ElideMiddle
                            ToolTip.visible: pathHover.hovered; ToolTip.text: card.modelData.path; ToolTip.delay: 600
                            HoverHandler { id: pathHover } }
                        Item { Layout.fillHeight: true }
                        RowLayout {
                            Layout.fillWidth: true
                            Text { Layout.fillWidth: true; text: root.opened(card.modelData); color: Theme.textDim; font.pixelSize: 11 }
                            IconButton { iconName: "x"; tip: "Remove from this list"; implicitWidth: 26; implicitHeight: 26; onClicked: root.report("forget_project", { path: card.modelData.path }) }
                            IconButton { iconName: "trash-2"; danger: true; tip: "Delete the project folder"; implicitWidth: 26; implicitHeight: 26; onClicked: { deleteDialog.entry = card.modelData; deleteDialog.open(); } }
                        }
                    }
                }
            }
        }
    }

    NewProjectDialog { id: newProject; app: root.app }
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
    BwDialog {
        id: deleteDialog
        property var entry: null
        title: "Delete project?"
        standardButtons: Dialog.Yes | Dialog.Cancel
        Text {
            width: 420; wrapMode: Text.WordWrap; color: Theme.text
            text: deleteDialog.entry ? "Delete “" + deleteDialog.entry.name + "” and everything in its folder?\n\n" + deleteDialog.entry.path + "\n\nThis cannot be undone." : ""
        }
        onAccepted: if (entry) root.report("delete_project", { path: entry.path })
    }
}
