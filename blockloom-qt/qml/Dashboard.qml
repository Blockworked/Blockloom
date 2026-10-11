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

    property string search: ""
    // Projects matching the search box, by name or folder.
    readonly property var shown: {
        const q = search.trim().toLowerCase();
        const all = root.appState.library || [];
        return q === "" ? all : all.filter(e => e.name.toLowerCase().indexOf(q) >= 0 || e.path.toLowerCase().indexOf(q) >= 0);
    }
    // Why a project can't be opened, or "" when it can.
    function blocked(entry) {
        if (!entry.available) return entry.error || "The folder is unavailable.";
        return "";
    }
    function editorLabel(entry) {
        return entry.installation_label || "No editor set";
    }
    function showDelete(entry) { deleteDialog.entry = entry; deleteDialog.open(); }
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
            Item { Layout.fillWidth: true }
            StitchTextField {
                visible: (root.appState.library || []).length > 4
                objectName: "project-search"; placeholderText: "Search projects"; implicitWidth: 220
                onTextChanged: root.search = text
            }
        }
        Text { visible: root.error.length > 0; text: root.error; color: Theme.danger; font.pixelSize: 13; Layout.fillWidth: true; wrapMode: Text.WordWrap }
        ColumnLayout {
            visible: root.appState.library.length === 0
            Layout.fillWidth: true; Layout.topMargin: 24; spacing: 6
            Text { text: "No projects yet."; color: Theme.text; font.pixelSize: 16; font.weight: Font.DemiBold }
            Text { text: "Create one and it gets a folder of its own, assets and all."; color: Theme.textDim; font.pixelSize: 13 }
        }
        Text {
            visible: root.appState.library.length > 0 && root.shown.length === 0
            objectName: "no-search-match"
            Layout.alignment: Qt.AlignHCenter; Layout.topMargin: 30
            text: "No project matches “" + root.search + "”."; color: Theme.textDim; font.pixelSize: 14
        }
        Text { visible: root.shown.length > 0; text: "Projects"; color: Theme.textDim; font.pixelSize: 13; font.weight: Font.DemiBold }
        ListView {
            id: list
            objectName: "project-list"
            visible: root.appState.library.length > 0
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: root.shown
            ScrollBar.vertical: ScrollBar {}
            delegate: Rectangle {
                id: row
                required property var modelData
                readonly property string unavailable: root.blocked(modelData)
                width: list.width; height: 64
                color: rowHover.hovered && !unavailable ? Theme.hover : "transparent"
                border.color: activeFocus ? Theme.accent : "transparent"
                activeFocusOnTab: true
                Keys.onReturnPressed: if (!unavailable) root.report("open_project", { path: modelData.path })
                Keys.onEnterPressed: if (!unavailable) root.report("open_project", { path: modelData.path })
                HoverHandler { id: rowHover; cursorShape: row.unavailable ? Qt.ArrowCursor : Qt.PointingHandCursor }
                TapHandler { enabled: !row.unavailable; onTapped: root.report("open_project", { path: row.modelData.path }) }
                Rectangle { anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom; height: 1; color: Theme.borderSoft }
                RowLayout {
                    anchors.fill: parent; anchors.leftMargin: 12; anchors.rightMargin: 8; spacing: 12
                    LucideIcon { visible: !row.unavailable; name: row.modelData.mode === "ThreeD" ? "box" : "square"; color: Theme.textDim; width: 18; height: 18 }
                    ColumnLayout {
                        Layout.fillWidth: true; spacing: 2
                        Text { Layout.fillWidth: true; text: row.modelData.name; color: Theme.text; font.pixelSize: 14; font.weight: Font.DemiBold; elide: Text.ElideRight }
                        Text { Layout.fillWidth: true; text: root.where(row.modelData.path); color: Theme.textDim; font.pixelSize: 12; elide: Text.ElideMiddle
                            ToolTip.visible: pathHover.hovered; ToolTip.text: row.modelData.path; ToolTip.delay: 600
                            HoverHandler { id: pathHover } }
                        Text {
                            Layout.fillWidth: true; elide: Text.ElideRight; font.pixelSize: 11
                            color: row.unavailable || row.modelData.in_use || row.modelData.wrong_editor ? Theme.warning : Theme.textDim
                            text: row.unavailable ? row.unavailable
                                : row.modelData.wrong_editor ? "Set to " + root.editorLabel(row.modelData) + ", not this editor."
                                : row.modelData.in_use ? "Open in another editor. Opening it here joins that session."
                                : root.editorLabel(row.modelData)
                        }
                    }
                    Text { visible: !row.unavailable; text: row.modelData.mode === "ThreeD" ? "3D" : "2D"; color: Theme.textDim; font.pixelSize: 12 }
                    Text { visible: !row.unavailable; Layout.preferredWidth: 140; horizontalAlignment: Text.AlignRight; text: root.opened(row.modelData); color: Theme.textDim; font.pixelSize: 12; elide: Text.ElideRight }
                    StitchButton { visible: row.unavailable.length > 0; objectName: "locate-folder"; text: "Locate folder"; onClicked: { locateDialog.entry = row.modelData; locateDialog.open(); } }
                    StitchButton { visible: row.unavailable.length > 0; objectName: "remove-from-list"; text: "Remove from list"; onClicked: root.report("forget_project", { path: row.modelData.path }) }
                    IconButton { visible: row.unavailable.length === 0; objectName: "project-menu"; iconName: "ellipsis"; tip: "Project options"; implicitWidth: 26; implicitHeight: 26
                        onClicked: { projectMenu.entry = row.modelData; projectMenu.popup(); } }
                }
            }
        }
        Item { visible: root.appState.library.length === 0; Layout.fillHeight: true }
    }

    Menu {
        id: projectMenu
        property var entry: null
        MenuItem { text: "Remove from list"; onTriggered: if (projectMenu.entry) root.report("forget_project", { path: projectMenu.entry.path }) }
        MenuSeparator {}
        MenuItem { text: "Delete project folder…"; onTriggered: { deleteDialog.entry = projectMenu.entry; deleteDialog.open(); } }
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
    FolderDialog {
        id: locateDialog
        property var entry: null
        title: "Locate the project folder"
        currentFolder: root.app.toFileUrl(root.appState.default_project_location)
        onAccepted: if (entry) root.report("locate_project", { path: entry.path, folder: root.app.fromFileUrl(selectedFolder) })
    }
    StitchDialog {
        id: deleteDialog
        property var entry: null
        readonly property bool confirmed: entry && confirmField.text === entry.name
        title: "Delete project?"
        readonly property bool forever: foreverBox.checked
        standardButtons: Dialog.NoButton
        onOpened: { confirmField.text = ""; foreverBox.checked = false }
        ColumnLayout {
            width: 440; spacing: 10
            Text {
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text
                text: deleteDialog.entry ? "Delete “" + deleteDialog.entry.name + "” and everything in its folder?\n\n" + deleteDialog.entry.path
                    + (deleteDialog.forever ? "\n\nThis cannot be undone." : "\n\nThe folder moves to the trash, so you can restore it.") : ""
            }
            Text {
                visible: deleteDialog.entry && deleteDialog.entry.in_use
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.warning; font.pixelSize: 12
                text: "This project is open in another editor, so it can't be deleted until that editor closes it."
            }
            StitchCheckBox { id: foreverBox; objectName: "delete-permanently"; text: "Delete permanently instead of moving to the trash" }
            Text { text: "Type the project name to confirm"; color: Theme.textDim; font.pixelSize: 12 }
            StitchTextField { id: confirmField; objectName: "delete-confirm"; Layout.fillWidth: true }
            RowLayout {
                Layout.alignment: Qt.AlignRight; spacing: 8
                StitchButton { text: "Cancel"; onClicked: deleteDialog.close() }
                StitchButton { objectName: "delete-project"; text: deleteDialog.forever ? "Delete permanently" : "Move to trash"; enabled: deleteDialog.confirmed && !(deleteDialog.entry && deleteDialog.entry.in_use)
                    onClicked: { root.report("delete_project", { path: deleteDialog.entry.path, permanent: deleteDialog.forever }); deleteDialog.close(); } }
            }
        }
    }
}
