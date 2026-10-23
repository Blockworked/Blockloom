import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The editor a project opens into: a top bar, the actor list, a Code tab
// (block palette and canvas) and a Game tab, the inspector, the asset tray
// and the run log.
Item {
    id: root
    required property var app
    readonly property var appState: app.appState
    readonly property var project: appState.project
    readonly property var actor: app.openActor
    // An actor's own lists and dicts shadow the project's shared ones.
    readonly property var lists: actor ? Blocks.visibleCollections(actor.lists, project.global_lists) : []
    readonly property var dicts: actor ? Blocks.visibleCollections(actor.dicts, project.global_dicts) : []

    Settings {
        id: panels
        category: "panels"
        property bool leftOpen: true
        property real leftWidth: 220
        property bool rightOpen: true
        property real rightWidth: 290
        property bool blocksOpen: true
        property real blocksWidth: 340
        property int tab: 0
    }

    Connections {
        target: root.app
        function onInspectedLightingChanged() { if (root.app.inspectedLighting) panels.rightOpen = true; }
        function onInspectedSceneChanged() { if (root.app.inspectedScene) panels.rightOpen = true; }
        function onInspectSceneChanged() { if (root.app.inspectScene) panels.rightOpen = true; }
    }
    readonly property bool running: appState.running === true
    // Play shows the game it started.
    onRunningChanged: if (running) panels.tab = 1
    function report(command, args) { invoke(command, args, null, e => invoke("push_log", { kind: "error", text: String(e) })); }

    function invoke(command, args, done, failed) { app.invoke(command, args, done, failed); }
    function strandById(id) { return actor ? (actor.strands.find(s => s.id === id) || null) : null; }
    function instructionAt(strand, path) {
        let list = strand ? strand.instructions : [];
        for (let i = 0; i < path.length; ++i) {
            const block = list[path[i].index];
            if (!block || i === path.length - 1) return block || null;
            list = BlockRegistry.body(block, path[i].slot || 0);
        }
        return null;
    }
    // ─── The editor's own copy buffer for blocks (not the OS clipboard) ────
    // A fragment from copy_instructions: the blocks plus the notes on them.
    property var copied: null
    function onBlockMenu(action, strandId, path, instruction) {
        if (action === "copy" || action === "copy-stack")
            invoke("copy_instructions", { strandId: strandId, path: path, count: action === "copy" ? 1 : null }, fragment => copied = fragment);
        else if (action === "note") {
            const note = actor.comments.find(c => c.attached_to === instruction.id);
            if (note) invoke("set_comment_collapsed", { commentId: note.id, collapsed: false });
            else invoke("create_comment", { x: 220, y: 0, text: "", attachedTo: instruction.id });
        }
    }
    function onCanvasMenu(action, x, y) {
        if (action === "paste" && copied && copied.instructions.length)
            invoke("paste_fragment", { x: x, y: y, fragment: copied });
    }

    // ─── Palette drops: the shared controller's edits as backend commands ──
    function submitEdit(edit, reply) {
        const call = (command, args) => invoke(command, args, () => reply(true), error => { app.showError(error); reply(false, error); });
        if (edit.kind === "insert_instruction") call("add_instruction", { strandId: edit.strandId, path: edit.path, instruction: edit.instruction });
        else if (edit.kind === "add_strand") call("add_strand", { x: edit.x, y: edit.y, instruction: edit.instruction });
        else if (edit.kind === "put_value") call("put_value", { location: edit.location, value: edit.value });
        else if (edit.kind === "create_floating_value") call("create_floating_value", { x: edit.x, y: edit.y, value: edit.value, originBlockId: edit.originBlockId || null });
        else if (edit.kind === "remove_instructions") call("remove_instruction", { strandId: edit.strandId, path: edit.path, count: edit.count });
        else if (edit.kind === "remove_strand") call("remove_strand", { strandId: edit.strandId });
        else if (edit.kind === "take_value") call("take_value", { location: edit.location });
        else if (edit.kind === "remove_floating_value") call("remove_floating_value", { floatingId: edit.floatingId });
        else reply(false, "Unknown edit " + edit.kind);
    }
    // A value drag is one move and one undo step.
    function moveValue(from, to, x, y, originBlockId) {
        invoke("move_value", { from: from, to: to || null, x: Math.round(x || 0), y: Math.round(y || 0), originBlockId: originBlockId || null });
    }
    ColumnLayout {
        anchors.fill: parent; spacing: 0
        TopBar { Layout.fillWidth: true; app: root.app }
        Rectangle {
            visible: !root.appState.runtime_available
            Layout.fillWidth: true; implicitHeight: 34; color: "#4a3a1c"
            Text { anchors.centerIn: parent; color: "#ffd89a"; font.pixelSize: 12
                text: "The game runtime is missing, so Play has nothing to open. Build the whole workspace (just build), not only the editor." }
        }
        RowLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; spacing: 0
            ActorList {
                visible: panels.tab !== 2
                Layout.fillHeight: true
                Layout.preferredWidth: panels.leftOpen ? panels.leftWidth : 34
                app: root.app
                open: panels.leftOpen
                onOpenRequested: open => panels.leftOpen = open
                onResizeRequested: w => panels.leftWidth = Math.max(180, Math.min(420, w))
            }
            ColumnLayout {
                Layout.fillWidth: true; Layout.fillHeight: true; spacing: 0
                Rectangle {
                    Layout.fillWidth: true; implicitHeight: 36
                    color: Theme.panel
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.borderSoft }
                    RowLayout {
                        anchors.fill: parent; anchors.leftMargin: 6; anchors.rightMargin: 6; spacing: 2
                        Repeater {
                            model: [{ label: "Code", icon: "blocks" }, { label: "Game", icon: "gamepad-2" }, { label: "Interface", icon: "layout-grid" }, { label: "Devices", icon: "smartphone" }]
                            delegate: Rectangle {
                                id: tab
                                required property var modelData
                                required property int index
                                readonly property bool current: panels.tab === index
                                Layout.fillHeight: true
                                implicitWidth: tabRow.implicitWidth + 24
                                color: tabMouse.containsMouse && !current ? Theme.panelRaised : "transparent"
                                Row {
                                    id: tabRow
                                    anchors.centerIn: parent; spacing: 6
                                    LucideIcon { name: tab.modelData.icon; width: 14; height: 14; anchors.verticalCenter: parent.verticalCenter; color: tab.current ? Theme.text : Theme.textDim }
                                    Text { text: tab.modelData.label; color: tab.current ? Theme.text : Theme.textDim; font.pixelSize: 12; font.weight: tab.current ? Font.DemiBold : Font.Normal; anchors.verticalCenter: parent.verticalCenter }
                                }
                                Rectangle { visible: tab.current; anchors.bottom: parent.bottom; width: parent.width; height: 2; color: Theme.accent }
                                MouseArea { id: tabMouse; anchors.fill: parent; hoverEnabled: true; onClicked: panels.tab = tab.index }
                            }
                        }
                        Item { Layout.fillWidth: true }
                        IconButton {
                            visible: root.appState.preview_enabled
                            iconName: root.running ? "square" : "play"; tip: root.running ? "Stop" : "Play"
                            onClicked: root.report(root.running ? "stop_project" : "run_project")
                        }
                        IconButton {
                            visible: root.appState.preview_enabled && root.running
                            iconName: root.appState.paused ? "play" : "pause"; tip: root.appState.paused ? "Resume" : "Pause"
                            onClicked: root.report("pause_project", { paused: !root.appState.paused })
                        }
                        IconButton { visible: root.appState.preview_enabled && root.running && root.appState.paused; iconName: "step-forward"; tip: "Advance one tick"; onClicked: root.report("step_project") }
                    }
                }
                StackLayout {
                    Layout.fillWidth: true; Layout.fillHeight: true
                    currentIndex: panels.tab
                    Item {
                        RowLayout {
                            anchors.fill: parent; spacing: 0
                            visible: !!root.actor
                            BlockSidebar {
                                id: sidebar
                                Layout.fillHeight: true
                                Layout.preferredWidth: panels.blocksOpen ? panels.blocksWidth : 34
                                app: root.app; editor: root
                                open: panels.blocksOpen
                                trashArmed: canvas.dragging && sidebar.contains(sidebar.mapFromItem(null, canvas.dragSceneX, canvas.dragSceneY))
                                onOpenRequested: open => panels.blocksOpen = open
                                onResizeRequested: w => panels.blocksWidth = Math.max(220, Math.min(640, w))
                                onDragStarted: (spec, sx, sy, ox, oy) => dragController.begin(spec, sx, sy, ox, oy)
                                onDragMoved: (sx, sy) => dragController.move(sx, sy)
                                onDragEnded: (sx, sy) => dragController.end(sx, sy)
                                onDragCanceled: dragController.cancel()
                                onEntryActivated: spec => dragController.addAtView(spec)
                                onDetailsRequested: (name, identifier, explainer) => details.show(name, identifier, explainer)
                            }
                            BlockCanvas {
                                id: canvas
                                Layout.fillWidth: true; Layout.fillHeight: true
                                offerClear: false
                                strands: root.actor ? root.actor.strands : []
                                comments: root.actor ? root.actor.comments : []
                                floatingValues: root.actor ? root.actor.floating_values : []
                                variables: Blocks.variableNames()
                                lists: root.lists
                                dicts: root.dicts
                                blockDefinitions: root.actor ? root.actor.block_defs : []
                                onStrandMoved: (strandId, x, y) => root.invoke("move_strand", { strandId: strandId, x: x, y: y })
                                onInstructionSplit: (strandId, path, x, y) => root.invoke("split_strand", { strandId: strandId, path: path, x: x, y: y })
                                onStrandsMerged: (draggedId, targetId, path) => root.invoke("merge_strand", { draggedId: draggedId, targetId: targetId, path: path })
                                onTailMerged: (strandId, path, targetId, targetPath) => root.invoke("merge_tail", { strandId: strandId, path: path, targetId: targetId, targetPath: targetPath })
                                onInstructionRemoved: (strandId, path) => {
                                    const strand = root.strandById(strandId);
                                    root.invoke("delete_instruction", { strandId: strandId, path: path, x: strand ? strand.x + 40 : 0, y: strand ? strand.y + 40 : 0 });
                                }
                                onInstructionDuplicated: (strandId, path) => root.invoke("duplicate_instruction", { strandId: strandId, path: path })
                                onInstructionEdited: (strandId, path, instruction) => root.invoke("edit_instruction", { strandId: strandId, path: path, instruction: instruction })
                                onValueEdited: (location, text) => root.invoke("edit_value_field", { location: location, text: text })
                                onValueTakeRequested: location => root.invoke("take_value", { location: location })
                                onValuePutRequested: (location, value) => root.invoke("put_value", { location: location, value: value })
                                onValueCreateRequested: (x, y, value) => root.invoke("create_floating_value", { x: x, y: y, value: value, originBlockId: null })
                                onValueMoveRequested: (from, to, x, y, originBlockId) => root.moveValue(from, to, x, y, originBlockId)
                                onCommentForInstructionRequested: instruction => root.onBlockMenu("note", "", [], instruction)
                                onDetailsRequested: type => details.show(Blocks.labels[type] || type, type, "")
                                onCanvasNoteRequested: (x, y) => root.invoke("create_comment", { x: x, y: y, text: "" })
                                onCommentMoved: (commentId, x, y) => root.invoke("move_comment", { commentId: commentId, x: x, y: y })
                                onCommentEdited: (commentId, text) => root.invoke("edit_comment_text", { commentId: commentId, text: text })
                                onCommentCollapseChanged: (commentId, collapsed) => root.invoke("set_comment_collapsed", { commentId: commentId, collapsed: collapsed })
                                onCommentRemoved: commentId => root.invoke("remove_comment", { commentId: commentId })
                                onFloatingValueMoved: (floatingId, x, y) => root.invoke("move_floating_value", { floatingId: floatingId, x: x, y: y })
                                onFloatingValueRemoved: floatingId => root.invoke("remove_floating_value", { floatingId: floatingId })
                                onListItemsEdited: (name, items) => root.invoke("set_list_items", { name: name, items: items })
                                onListEditorStateChanged: (name, visible, x, y) => root.invoke("set_list_editor_state", { name: name, visible: visible, x: x, y: y })
                                onDictEntriesEdited: (name, entries) => root.invoke("set_dict_entries", { name: name, entries: entries })
                                onDictEditorStateChanged: (name, visible, x, y) => root.invoke("set_dict_editor_state", { name: name, visible: visible, x: x, y: y })
                                onBlockMenuAction: (action, strandId, path, instruction) => root.onBlockMenu(action, strandId, path, instruction)
                                onCanvasMenuAction: (action, x, y) => root.onCanvasMenu(action, x, y)
                            }
                        }
                        Item {
                            visible: !root.actor
                            anchors.fill: parent
                            Column {
                                anchors.centerIn: parent; spacing: 8
                                LucideIcon { anchors.horizontalCenter: parent.horizontalCenter; name: "blocks"; color: "#777980"; width: 48; height: 48 }
                                Text { anchors.horizontalCenter: parent.horizontalCenter; text: "No actor selected"; color: Theme.text; font.pixelSize: 16; font.weight: Font.DemiBold }
                                Text { text: "Pick an actor on the left, or add one, to see its blocks."; color: Theme.textDim; font.pixelSize: 13 }
                            }
                        }
                    }
                    Loader {
                        active: panels.tab === 1
                        sourceComponent: Component { PreviewPanel { app: root.app } }
                    }
                    UiDesigner { app: root.app }
                    DevicesPanel { app: root.app }
                }
            }
            InspectorPanel {
                id: inspector
                visible: panels.tab !== 2
                Layout.fillHeight: true
                Layout.preferredWidth: panels.rightOpen ? panels.rightWidth : 34
                app: root.app
                open: panels.rightOpen
                onOpenRequested: open => panels.rightOpen = open
                onResizeRequested: w => panels.rightWidth = Math.max(220, Math.min(480, w))
            }
        }
        AssetTray { visible: panels.tab !== 2; Layout.fillWidth: true; app: root.app }
        RunLog { Layout.fillWidth: true; app: root.app; onOpenScriptRequested: (path, line) => inspector.openScript(path, line) }
    }

    BwDialog {
        id: details
        standardButtons: Dialog.NoButton; width: 430; padding: 0; topPadding: 0; bottomPadding: 0; showClose: false
        property string detailName: ""; property string detailIdentifier: ""; property string detailExplainer: ""
        function show(name, identifier, explainer) { detailName = name; detailIdentifier = identifier; detailExplainer = explainer; open(); }
        contentItem: Column {
            spacing: 12; padding: 18
            Row { spacing: 9
                LucideIcon { name: "info"; width: 20; height: 20; color: Theme.accent; anchors.verticalCenter: parent.verticalCenter }
                Text { text: details.detailName; color: Theme.text; font.pixelSize: 17; font.weight: Font.Bold; anchors.verticalCenter: parent.verticalCenter } }
            Rectangle { width: 394; height: 30; radius: 5; color: Theme.field; border.color: Theme.border
                Text { anchors.centerIn: parent; text: details.detailIdentifier; color: Theme.accent; font.family: "monospace"; font.pixelSize: 12 } }
            Text { visible: details.detailExplainer.length > 0; width: 394; text: details.detailExplainer; color: Theme.textDim; font.pixelSize: 13; wrapMode: Text.WordWrap }
            Row { anchors.right: parent.right; anchors.rightMargin: 18; BwButton { text: "Close"; onClicked: details.close() } }
        }
    }

    PaletteDragController {
        id: dragController
        anchors.fill: parent
        canvas: canvas
        variables: Blocks.variableNames()
        lists: root.lists
        blockDefinitions: root.actor ? root.actor.block_defs : []
        // Switching actors cancels a drag and drops a held ghost.
        documentId: root.actor ? root.actor.id : ""
        document: root.actor ? [root.actor.strands, root.actor.floating_values] : null
        trashContains: (sx, sy) => sidebar.contains(sidebar.mapFromItem(null, sx, sy))
        submit: (edit, reply) => root.submitEdit(edit, reply)
    }
}
