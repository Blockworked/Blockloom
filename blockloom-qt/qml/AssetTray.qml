import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The asset tray: a small file manager over the project folder, along the
// bottom of the editor. A project is a folder, so this is that folder -
// `project.blockloom` beside the `assets/` everything else lives in. Dragging
// a file out of here onto a path box is what puts it in the document.
//
// The listing isn't part of the app snapshot - a folder on disk changes for
// reasons the editor never hears about - so the tray asks for it after every
// change of its own, and has a refresh button for everyone else's.
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    implicitHeight: header.height + (remembered.open ? remembered.height : 0)
    color: Theme.panel
    border.color: Theme.borderSoft

    Settings { id: remembered; category: "assets"; property bool open: true; property real height: 150 }
    property string path: ""
    property string selected: ""
    property var entries: []
    property var reports: ({})
    property string error: ""
    property bool busy: false
    property var menuEntry: null
    property var draft: null        // { mode: "folder"|"file"|"rename", path, name }
    property string previewing: ""
    property var dropTarget: null  // A folder path while an asset drag hovers one, else null - null rather than "" so the project root ("") stays hoverable without reading as hovered.

    readonly property var icons: ({ folder: "folder", image: "image", audio: "music", font: "file-type", model: "box", script: "file-code", shader: "sparkles", text: "file-text", hdr: "sun", volume: "layers", light: "zap", height: "trending-up", scene: "map", lighting: "sun", other: "file" })
    function parentOf(p) { const cut = p.lastIndexOf("/"); return cut === -1 ? "" : p.slice(0, cut); }
    readonly property var crumbs: { const parts = path.split("/").filter(p => p.length); return parts.map((name, i) => ({ name: name, path: parts.slice(0, i + 1).join("/") })); }
    function fileSize(bytes) {
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1024 * 1024) return Math.round(bytes / 1024) + " KB";
        return (bytes / (1024 * 1024)).toFixed(1) + " MB";
    }

    function refresh() {
        if (!appState.project_path) { entries = []; return; }
        busy = true;
        app.invoke("list_assets", { path: path }, list => {
            busy = false; entries = list; error = "";
            app.invoke("pipeline_status", {}, statuses => { const next = {}; for (const r of statuses) next[r.path] = r; reports = next; }, () => {});
        }, e => {
            busy = false; entries = [];
            // A folder deleted from under us drops back to the top.
            if (path) path = ""; else error = String(e);
        });
    }
    function run(command, args) {
        error = "";
        app.invoke(command, args, result => {
            if (command === "create_asset" && /\.blocklighting$/i.test(String(result))) {
                app.selectLighting(result); selected = result;
            } else if (command === "create_asset" && /\.blockscene$/i.test(String(result))) {
                app.selectScene(root.appState.project.active_scene); selected = result;
            } else if ((command === "rename_asset" || command === "move_asset") && app.inspectedLighting
                && (app.inspectedLighting === args.path || app.inspectedLighting.startsWith(args.path + "/"))) {
                app.inspectedLighting = result + app.inspectedLighting.slice(args.path.length);
            } else if (command === "delete_asset" && app.inspectedLighting
                && (app.inspectedLighting === args.path || app.inspectedLighting.startsWith(args.path + "/"))) {
                app.selectScene(root.appState.project.active_scene);
            }
            refresh();
        }, e => { error = String(e); refresh(); });
    }
    function goTo(p) { path = p; selected = ""; }
    onPathChanged: refresh()
    Connections { target: root.app; function onAppStateChanged() { if (root.lastProject !== root.appState.project_path) { root.lastProject = root.appState.project_path; root.path = ""; root.refresh(); } } }
    Connections { target: root.app; function onInspectedLightingChanged() { root.refresh(); } }
    property var lastProject: null
    Component.onCompleted: { lastProject = appState.project_path; refresh(); }

    function startDraft(mode) {
        const name = mode === "folder" ? "New folder" : mode === "scene" ? "New Scene" : mode === "lighting" ? "New Lighting" : "notes.txt";
        draft = { mode: mode, path: "", name: name };
    }
    function startRename(entry) { if (!entry.protected) draft = { mode: "rename", path: entry.path, name: entry.name }; }
    function commitDraft(name) {
        const d = draft;
        draft = null;
        if (!d || !name.trim().length) return;
        if (d.mode === "folder") run("create_asset_folder", { parent: path, name: name });
        else if (d.mode === "file") run("create_asset", { parent: path, name: name });
        else if (d.mode === "lighting") {
            let file = name.trim();
            if (!/\.blocklighting$/i.test(file)) file += ".blocklighting";
            run("create_asset", { parent: path, name: file });
        }
        else if (d.mode === "scene") {
            let file = name.trim();
            if (!/\.blockscene$/i.test(file)) file += ".blockscene";
            run("create_asset", { parent: path, name: file });
        }
        else if (name !== d.name) run("rename_asset", { path: d.path, name: name });
    }
    function report(entry) { return reports[entry.path] || null; }
    // Double-clicking a scene file opens it; its filename is the scene name.
    function openSceneAsset(entry) {
        const scenes = root.appState.project ? root.appState.project.scenes : [];
        const active = root.appState.project ? root.appState.project.active_scene : "";
        let found = scenes.find(s => s.path === entry.path);
        if (!found) {
            const stem = entry.name.replace(/\.blockscene$/i, "");
            found = scenes.find(s => s.name === stem);
        }
        if (found) {
            root.app.selectScene(found.id);
            if (found.id !== active) root.app.invoke("set_active_scene", { sceneId: found.id });
        } else root.app.invoke("import_scene", { path: entry.path }, id => root.app.selectScene(id));
    }
    function selectEntry(entry) {
        if (entry.kind === "lighting") app.selectLighting(entry.path);
        else if (entry.kind === "scene") {
            const scenes = appState.project ? appState.project.scenes : [];
            const found = scenes.find(s => s.path === entry.path);
            if (found) app.selectScene(found.id);
            else openSceneAsset(entry);
        }
    }
    // An image can be re-roled; offer every role it isn't already.
    function canRole(role) {
        if (!menuEntry || menuEntry.kind !== "image") return false;
        const r = report(menuEntry);
        return (r && r.role ? r.role : "texture") !== role;
    }
    function togglePreview(entry) {
        if (!sound.item) sound.source = "SoundPreview.qml";
        if (!sound.item) return;
        if (previewing === entry.path) { sound.item.stop(); previewing = ""; return; }
        sound.item.play(app.assetUrl(entry.path));
        previewing = entry.path;
    }

    // ─── Dragging a tile ───────────────────────────────────────────────────
    function canDropIn(entry, target) {
        return !!entry && !entry.protected && entry.path !== target && parentOf(entry.path) !== target && !target.startsWith(entry.path + "/");
    }
    function folderAt(sceneX, sceneY) {
        const p = tiles.mapFromItem(null, sceneX, sceneY);
        const tile = tiles.childAt(p.x, p.y);
        if (tile && tile.folderPath !== undefined) return tile.folderPath;
        for (let i = 0; i < crumbRow.children.length; ++i) {
            const c = crumbRow.children[i];
            if (c.folderPath !== undefined && c.contains(c.mapFromItem(null, sceneX, sceneY))) return c.folderPath;
        }
        return null;
    }
    function moveDrag(entry, sceneX, sceneY) {
        const g = ghostLayer.mapFromItem(null, sceneX, sceneY);
        ghost.x = g.x + 8; ghost.y = g.y + 8;
        const folder = folderAt(sceneX, sceneY);
        dropTarget = folder !== null && canDropIn(entry, folder) ? folder : null;
    }
    function endDrag(entry, sceneX, sceneY) {
        root.app.assetDrag = null;
        const target = dropTarget;
        dropTarget = null;
        for (const box of root.app.assetTargets.slice()) if (box.takeDrop(entry, sceneX, sceneY)) return;
        const folder = folderAt(sceneX, sceneY);
        if (folder !== null && canDropIn(entry, folder)) run("move_asset", { path: entry.path, parent: folder });
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        // Drag the top edge to resize.
        Rectangle {
            visible: remembered.open
            Layout.fillWidth: true; Layout.preferredHeight: 4; color: resizer.containsMouse || resizer.pressed ? Theme.accent : "transparent"
            MouseArea {
                id: resizer
                anchors.fill: parent; anchors.topMargin: -3; anchors.bottomMargin: -3; hoverEnabled: true; cursorShape: Qt.SizeVerCursor
                property real startHeight: 0; property real startY: 0
                onPressed: mouse => { startHeight = remembered.height; startY = mapToItem(null, 0, mouse.y).y; }
                onPositionChanged: mouse => { if (pressed) remembered.height = Math.max(120, Math.min(520, startHeight + startY - mapToItem(null, 0, mouse.y).y)); }
            }
        }
        RowLayout {
            id: header
            Layout.fillWidth: true; Layout.preferredHeight: 36; Layout.leftMargin: 6; Layout.rightMargin: 6; spacing: 4
            BwButton {
                flat: true; implicitHeight: 28; font.pixelSize: 12
                iconName: remembered.open ? "chevron-down" : "chevron-up"; text: "Assets"
                onClicked: { remembered.open = !remembered.open; if (remembered.open) root.refresh(); }
            }
            Row {
                id: crumbRow
                visible: remembered.open; spacing: 2
                Layout.fillWidth: true
                BwButton {
                    readonly property string folderPath: ""
                    flat: root.dropTarget !== ""; implicitHeight: 26; font.pixelSize: 12; iconName: "house"
                    text: root.appState.project ? root.appState.project.name : "Project"
                    onClicked: root.goTo("")
                }
                Repeater {
                    model: root.crumbs
                    delegate: BwButton {
                        required property var modelData
                        readonly property string folderPath: modelData.path
                        flat: root.dropTarget !== modelData.path; implicitHeight: 26; font.pixelSize: 12; text: "› " + modelData.name
                        onClicked: root.goTo(modelData.path)
                    }
                }
            }
            Item { Layout.fillWidth: true; visible: !remembered.open }
            IconButton { visible: remembered.open; iconName: "folder-plus"; tip: "New folder"; implicitWidth: 28; implicitHeight: 28; onClicked: root.startDraft("folder") }
            IconButton { visible: remembered.open; iconName: "file-plus"; tip: "New asset"; implicitWidth: 28; implicitHeight: 28; onClicked: createMenu.popup() }
            IconButton { visible: remembered.open; iconName: "download"; tip: "Import files into this folder"; implicitWidth: 28; implicitHeight: 28; onClicked: importDialog.open() }
            IconButton { visible: remembered.open; iconName: "refresh-cw"; tip: "Re-read this folder"; enabled: !root.busy; implicitWidth: 28; implicitHeight: 28; onClicked: root.refresh() }
        }
        Rectangle {
            visible: remembered.open
            Layout.fillWidth: true; Layout.preferredHeight: remembered.height
            color: fileDrop.containsDrag ? "#1a1597ff" : Theme.canvas
            // Files and folders dropped from the OS file manager import here.
            DropArea {
                id: fileDrop
                anchors.fill: parent
                onEntered: drag => drag.accepted = drag.hasUrls
                onDropped: drop => {
                    if (!drop.hasUrls) return;
                    const folder = root.folderAt(drop.x + mapToItem(null, 0, 0).x, drop.y + mapToItem(null, 0, 0).y);
                    root.run("import_assets", { parent: folder !== null ? folder : root.path, paths: drop.urls.map(u => root.app.fromFileUrl(u)) });
                    drop.accept();
                }
            }
            Text { visible: root.error.length > 0; anchors.top: parent.top; anchors.left: parent.left; anchors.margins: 8; text: root.error; color: Theme.danger; font.pixelSize: 12; z: 2 }
            Flickable {
                id: flick
                anchors.fill: parent; anchors.margins: 8; anchors.topMargin: root.error.length ? 26 : 8
                contentHeight: tiles.height; clip: true
                ScrollBar.vertical: ScrollBar {}
                // The empty tray: deselects, and right-click acts on the folder listed.
                MouseArea {
                    width: flick.width; height: Math.max(tiles.height, flick.height)
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onClicked: mouse => { root.selected = ""; if (mouse.button === Qt.RightButton) { root.menuEntry = null; trayMenu.popup(); } }
                }
                Flow {
                    id: tiles
                    width: parent.width; spacing: 6
                    component Tile: Rectangle {
                        property bool active: false
                        width: 88; height: 86; radius: 6
                        color: active ? "#2b4a6b" : (tileHover.hovered ? "#303134" : "transparent")
                        border.color: active ? Theme.accent : "transparent"
                        HoverHandler { id: tileHover }
                    }
                    Tile {
                        visible: root.path.length > 0
                        readonly property string folderPath: root.parentOf(root.path)
                        active: root.dropTarget === folderPath && root.path.length > 0
                        LucideIcon { anchors.horizontalCenter: parent.horizontalCenter; y: 14; name: "chevron-up"; width: 28; height: 28; color: Theme.textDim }
                        Text { anchors.horizontalCenter: parent.horizontalCenter; y: 58; text: ".."; color: Theme.text; font.pixelSize: 12 }
                        MouseArea { anchors.fill: parent; onClicked: root.goTo(parent.folderPath) }
                    }
                    Tile {
                        visible: !!root.draft && root.draft.mode !== "rename"
                        LucideIcon { anchors.horizontalCenter: parent.horizontalCenter; y: 10; name: root.draft && root.draft.mode === "folder" ? "folder" : root.draft && root.draft.mode === "scene" ? "map" : "file"; width: 28; height: 28; color: Theme.textDim }
                        BwTextField {
                            id: draftField
                            anchors.left: parent.left; anchors.right: parent.right; y: 50; implicitHeight: 26; font.pixelSize: 11
                            text: root.draft ? root.draft.name : ""
                            onVisibleChanged: if (visible) { forceActiveFocus(); selectAll(); }
                            onAccepted: root.commitDraft(text)
                            onActiveFocusChanged: if (!activeFocus && root.draft && root.draft.mode !== "rename") root.commitDraft(text)
                            Keys.onEscapePressed: root.draft = null
                        }
                    }
                    Repeater {
                        model: root.entries
                        delegate: Tile {
                            id: tile
                            objectName: "asset-" + modelData.path
                            required property var modelData
                            readonly property var folderPath: modelData.kind === "folder" ? modelData.path : undefined
                            readonly property bool renaming: !!root.draft && root.draft.mode === "rename" && root.draft.path === modelData.path
                            active: root.selected === modelData.path || root.dropTarget === modelData.path
                            ToolTip.visible: tileHoverTip.hovered && !renaming
                            ToolTip.delay: 700
                            ToolTip.text: modelData.path + (modelData.kind === "folder" ? "" : " · " + root.fileSize(modelData.size)) + (root.report(modelData) ? "\n" + root.report(modelData).summary : "")
                            HoverHandler { id: tileHoverTip }
                            Image {
                                id: thumb
                                visible: tile.modelData.kind === "image" && status === Image.Ready
                                anchors.horizontalCenter: parent.horizontalCenter; y: 8; width: 44; height: 44
                                fillMode: Image.PreserveAspectFit; asynchronous: true; cache: false
                                sourceSize.width: 88; sourceSize.height: 88
                                source: tile.modelData.kind === "image" ? root.app.assetUrl(tile.modelData.path) + "?" + tile.modelData.modified : ""
                            }
                            LucideIcon { visible: !thumb.visible; anchors.horizontalCenter: parent.horizontalCenter; y: 14; name: root.icons[tile.modelData.kind] || "file"; width: 30; height: 30; color: tile.modelData.kind === "folder" ? Theme.accent : Theme.textDim }
                            Rectangle { visible: !!root.report(tile.modelData) && root.report(tile.modelData).dirty; x: parent.width - 18; y: 8; width: 8; height: 8; radius: 4; color: Theme.warning }
                            Text {
                                visible: !tile.renaming
                                anchors.left: parent.left; anchors.right: parent.right; anchors.margins: 4; y: 58
                                text: tile.modelData.name; color: Theme.text; font.pixelSize: 11
                                horizontalAlignment: Text.AlignHCenter; elide: Text.ElideMiddle
                            }
                            BwTextField {
                                visible: tile.renaming
                                anchors.left: parent.left; anchors.right: parent.right; y: 54; implicitHeight: 26; font.pixelSize: 11
                                text: tile.modelData.name
                                onVisibleChanged: if (visible) { forceActiveFocus(); const dot = text.lastIndexOf("."); select(0, dot > 0 ? dot : text.length); }
                                onAccepted: root.commitDraft(text)
                                onActiveFocusChanged: if (!activeFocus && tile.renaming) root.commitDraft(text)
                                Keys.onEscapePressed: root.draft = null
                            }
                            MouseArea {
                                anchors.fill: parent; enabled: !tile.renaming
                                acceptedButtons: Qt.LeftButton | Qt.RightButton
                                property point press
                                property bool dragging: false
                                onPressed: mouse => {
                                    root.selected = tile.modelData.path;
                                    press = mapToItem(null, mouse.x, mouse.y); dragging = false;
                                    if (mouse.button === Qt.RightButton && !tile.modelData.protected) { root.menuEntry = tile.modelData; trayMenu.popup(); }
                                }
                                onPositionChanged: mouse => {
                                    if (!(mouse.buttons & Qt.LeftButton)) return;
                                    const p = mapToItem(null, mouse.x, mouse.y);
                                    if (!dragging && Math.hypot(p.x - press.x, p.y - press.y) > 6) {
                                        dragging = true;
                                        root.app.assetDrag = tile.modelData;
                                        ghost.entry = tile.modelData;
                                    }
                                    if (dragging) root.moveDrag(tile.modelData, p.x, p.y);
                                }
                                onReleased: mouse => {
                                    if (!dragging) {
                                        if (mouse.button === Qt.LeftButton) root.selectEntry(tile.modelData);
                                        return;
                                    }
                                    dragging = false; ghost.entry = null;
                                    const p = mapToItem(null, mouse.x, mouse.y);
                                    root.endDrag(tile.modelData, p.x, p.y);
                                }
                                onCanceled: { dragging = false; ghost.entry = null; root.app.assetDrag = null; root.dropTarget = null; }
                                onDoubleClicked: {
                                    if (tile.modelData.kind === "folder") root.goTo(tile.modelData.path);
                                    else if (tile.modelData.kind === "scene") root.openSceneAsset(tile.modelData);
                                }
                            }
                            BwButton {
                                visible: tile.modelData.kind === "audio"
                                x: 4; y: 4; implicitWidth: 24; implicitHeight: 24; flat: true; text: ""
                                iconName: root.previewing === tile.modelData.path ? "pause" : "play"
                                onClicked: root.togglePreview(tile.modelData)
                            }
                        }
                    }
                }
                Text {
                    visible: !root.entries.length && !root.error.length
                    anchors.centerIn: parent; width: Math.min(parent.width - 40, 520); wrapMode: Text.WordWrap; horizontalAlignment: Text.AlignHCenter
                    color: Theme.textDim; font.pixelSize: 12
                    text: "Nothing here yet. Import a sprite or a sound, or make a folder to put one in - then drag it onto a component that takes it."
                }
            }
        }
    }

    BwMenu {
        id: createMenu
        BwMenuItem { iconName: "file-plus"; text: "File"; onTriggered: root.startDraft("file") }
        BwMenuItem { iconName: "map"; text: "Scene"; onTriggered: root.startDraft("scene") }
        BwMenuItem { iconName: "sun"; text: "Lighting"; onTriggered: root.startDraft("lighting") }
    }
    BwMenu {
        id: trayMenu
        BwMenuItem { iconName: "external-link"; text: "Open File Location"; onTriggered: root.app.invoke("open_asset_location", { path: root.menuEntry ? root.menuEntry.path : root.path }) }
        BwMenuItem { visible: !root.menuEntry; iconName: "folder-plus"; text: "New folder"; onTriggered: root.startDraft("folder") }
        BwMenuItem { visible: !root.menuEntry; iconName: "file-plus"; text: "New file"; onTriggered: root.startDraft("file") }
        BwMenuItem { visible: !root.menuEntry; iconName: "map"; text: "New scene"; onTriggered: root.startDraft("scene") }
        BwMenuItem { visible: !root.menuEntry; iconName: "sun"; text: "New Lighting"; onTriggered: root.startDraft("lighting") }
        BwMenuItem { visible: !root.menuEntry; iconName: "download"; text: "Import files here"; onTriggered: importDialog.open() }
        BwMenuItem { visible: !root.menuEntry; iconName: "refresh-cw"; text: "Re-read this folder"; onTriggered: root.refresh() }
        BwMenuItem {
            visible: !root.menuEntry && Object.values(root.reports).some(r => r.dirty)
            iconName: "sparkles"; text: "Reimport changed files"
            onTriggered: root.run("reimport_assets", { paths: Object.values(root.reports).filter(r => r.dirty).map(r => r.path) })
        }
        BwMenuItem { visible: !!root.menuEntry && !!root.report(root.menuEntry) && root.report(root.menuEntry).dirty; iconName: "sparkles"; text: "Reimport this file"; onTriggered: root.run("reimport_assets", { paths: [root.menuEntry.path] }) }
        BwMenuItem { visible: root.canRole("texture"); iconName: "image"; text: "Import as texture"; onTriggered: root.run("set_import_role", { path: root.menuEntry.path, role: "auto" }) }
        BwMenuItem { visible: root.canRole("heightmap"); iconName: "trending-up"; text: "Import as heightmap"; onTriggered: root.run("set_import_role", { path: root.menuEntry.path, role: "heightmap" }) }
        BwMenuItem { visible: root.canRole("cookie"); iconName: "sun"; text: "Import as light cookie"; onTriggered: root.run("set_import_role", { path: root.menuEntry.path, role: "cookie" }) }
        BwMenuItem { visible: root.canRole("volume"); iconName: "layers"; text: "Import as volume strip"; onTriggered: root.run("set_import_role", { path: root.menuEntry.path, role: "volume" }) }
        BwMenuItem { visible: !!root.menuEntry && root.menuEntry.kind === "hdr"; iconName: "sun"; text: "Exposure bias…"
            onTriggered: { biasDialog.entry = root.menuEntry; const r = root.report(root.menuEntry); biasDialog.ev = r && r.exposure_bias ? r.exposure_bias : 0; biasDialog.open(); } }
        BwMenuItem { visible: !!root.menuEntry && root.menuEntry.kind === "folder"; iconName: "folder"; text: "Open"; onTriggered: root.goTo(root.menuEntry.path) }
        BwMenuItem { visible: !!root.menuEntry && root.menuEntry.kind === "scene"; iconName: "map"; text: "Open scene"; onTriggered: root.openSceneAsset(root.menuEntry) }
        BwMenuItem { visible: !!root.menuEntry; iconName: "pencil"; text: "Rename"; onTriggered: root.startRename(root.menuEntry) }
        BwMenuItem { visible: !!root.menuEntry && root.path.length > 0; iconName: "upload"; text: "Move up one folder"; onTriggered: root.run("move_asset", { path: root.menuEntry.path, parent: root.parentOf(root.parentOf(root.menuEntry.path)) }) }
        BwMenuItem { visible: !!root.menuEntry; iconName: "trash-2"; danger: true; text: root.menuEntry ? "Delete “" + root.menuEntry.name + "”" : ""; onTriggered: { deleteDialog.entry = root.menuEntry; deleteDialog.open(); } }
    }
    BwDialog {
        id: deleteDialog
        property var entry: null
        title: "Delete?"
        standardButtons: Dialog.Yes | Dialog.Cancel
        Text { color: Theme.text; text: deleteDialog.entry ? "Delete " + (deleteDialog.entry.kind === "folder" ? "“" + deleteDialog.entry.name + "” and everything in it" : "“" + deleteDialog.entry.name + "”") + "?\n\nThis cannot be undone." : "" }
        onAccepted: if (entry) root.run("delete_asset", { path: entry.path })
    }
    BwDialog {
        id: biasDialog
        property var entry: null
        property real ev: 0
        title: "Exposure bias"
        standardButtons: Dialog.Ok | Dialog.Cancel
        ColumnLayout {
            spacing: 8
            Text { color: Theme.textDim; font.pixelSize: 12; text: biasDialog.entry ? "Stops to brighten (or, below zero, darken) “" + biasDialog.entry.name + "” by, wherever it is used." : "" }
            NumberField { Layout.preferredWidth: 120; value: biasDialog.ev; fallback: 0; onCommitted: n => biasDialog.ev = Math.min(Math.max(n, -16), 16) }
        }
        onAccepted: if (entry) root.run("set_exposure_bias", { path: entry.path, ev: ev })
    }
    FileDialog {
        id: importDialog
        title: "Import into the project"
        fileMode: FileDialog.OpenFiles
        onAccepted: root.run("import_assets", { parent: root.path, paths: selectedFiles.map(u => root.app.fromFileUrl(u)) })
    }
    Loader { id: sound }
    Connections { target: sound.item; ignoreUnknownSignals: true; function onPlayingChanged() { if (!sound.item.playing) root.previewing = ""; } }

    // The tile following the pointer while an asset is dragged.
    Item {
        id: ghostLayer
        parent: Overlay.overlay; anchors.fill: parent; z: 1000; enabled: false
        Rectangle {
            id: ghost
            property var entry: null
            visible: !!entry
            width: ghostText.implicitWidth + 36; height: 28; radius: 6; color: Theme.panelRaised; border.color: Theme.accent; opacity: 0.92
            Row {
                anchors.centerIn: parent; spacing: 6
                LucideIcon { name: ghost.entry ? (root.icons[ghost.entry.kind] || "file") : "file"; width: 14; height: 14; color: Theme.textDim; anchors.verticalCenter: parent.verticalCenter }
                Text { id: ghostText; text: ghost.entry ? ghost.entry.name : ""; color: Theme.text; font.pixelSize: 12; anchors.verticalCenter: parent.verticalCenter }
            }
        }
    }
}
