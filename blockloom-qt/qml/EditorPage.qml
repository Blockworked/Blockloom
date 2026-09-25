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

    readonly property bool running: appState.running === true
    // Play shows the game it started.
    onRunningChanged: if (running) panels.tab = 1
    function report(command, args) { invoke(command, args, null, e => invoke("push_log", { kind: "error", text: String(e) })); }

    function invoke(command, args, done, failed) { app.invoke(command, args, done, failed); }
    function strandById(id) { return actor ? (actor.strands.find(s => s.id === id) || null) : null; }
    function nextPath(path) { const p = JSON.parse(JSON.stringify(path)); p[p.length - 1].index++; return p; }
    function instructionAt(strand, path) {
        let list = strand ? strand.instructions : [];
        for (let i = 0; i < path.length; ++i) {
            const block = list[path[i].index];
            if (!block || i === path.length - 1) return block || null;
            list = BlockRegistry.body(block, path[i].slot || 0);
        }
        return null;
    }
    function listAt(strand, path) {
        let list = strand ? strand.instructions : [];
        for (let i = 0; i < path.length - 1; ++i) {
            const block = list[path[i].index];
            if (!block) return [];
            list = BlockRegistry.body(block, path[i].slot || 0);
        }
        return list;
    }

    // ─── The editor's own copy buffer for blocks (not the OS clipboard) ────
    property var copied: null
    function onBlockMenu(action, strandId, path, instruction) {
        const strand = strandById(strandId);
        if (action === "copy") { if (instruction) copied = [instruction]; }
        else if (action === "copy-stack") { copied = listAt(strand, path).slice(path[path.length - 1].index); }
        else if (action === "note") {
            const note = actor.comments.find(c => c.attached_to === instruction.id);
            if (note) invoke("set_comment_collapsed", { commentId: note.id, collapsed: false });
            else invoke("create_comment", { x: 220, y: 0, text: "", attachedTo: instruction.id });
        }
    }
    function onCanvasMenu(action, x, y) {
        if (action === "paste" && copied && copied.length)
            invoke("paste_instructions", { x: x, y: y, instructions: copied.map(i => Blocks.regenerateIds(i)) });
    }

    // ─── Dragging a palette entry (or a header's parameter) onto the canvas ─
    property var paletteDrag: null   // { spec, offsetX, offsetY }
    function beginPaletteDrag(spec, ox, oy, sx, sy) {
        paletteDrag = { spec: spec, offsetX: ox, offsetY: oy };
        ghost.spec = spec;
        movePaletteDrag(sx, sy);
        ghost.visible = true;
    }
    function movePaletteDrag(sx, sy) {
        if (!paletteDrag) return;
        const p = ghostLayer.mapFromItem(null, sx, sy);
        ghost.x = p.x - paletteDrag.offsetX; ghost.y = p.y - paletteDrag.offsetY;
        const spec = paletteDrag.spec;
        if (spec.kind === "value") {
            canvas.clearSnap();
            const g1 = ghostLayer.mapToItem(null, ghost.x, ghost.y);
            const g2 = ghostLayer.mapToItem(null, ghost.x + ghost.width, ghost.y + ghost.height);
            canvas.updatePaletteValueTargetRect(g1.x, g1.y, g2.x, g2.y, sx, sy);
            return;
        }
        canvas.updatePaletteValueTarget(null, null);
        const at = canvas.workspacePoint(sx, sy);
        if (at && spec.instruction) canvas.updatePaletteSnap(at.x - paletteDrag.offsetX / canvas.zoom, at.y - paletteDrag.offsetY / canvas.zoom, spec.instruction, ghost.width, ghost.height);
        else canvas.clearSnap();
    }
    function endPaletteDrag(sx, sy) {
        const drag = paletteDrag;
        const snapValid = canvas.paletteSnapValid, snapTargetId = canvas.paletteSnapTargetId;
        const snapPath = JSON.parse(JSON.stringify(canvas.paletteSnapPath || []));
        const valueTarget = canvas.paletteValueTarget ? JSON.parse(JSON.stringify(canvas.paletteValueTarget)) : null;
        cancelPaletteDrag();
        if (!drag) return;
        const spec = drag.spec;
        if (spec.kind === "value" && valueTarget) { invoke("put_value", { location: valueTarget, value: spec.value }); return; }
        const at = canvas.workspacePoint(sx, sy);
        if (!at) return;
        const x = Math.round(at.x - drag.offsetX / canvas.zoom), y = Math.round(at.y - drag.offsetY / canvas.zoom);
        if (spec.kind === "value") {
            invoke("create_floating_value", { x: x, y: y, value: spec.value, originBlockId: spec.originBlockId || null });
            return;
        }
        const instruction = JSON.parse(JSON.stringify(spec.instruction));
        instruction.id = Blocks.uuid();
        if (snapValid && snapTargetId) invoke("add_instruction", { strandId: snapTargetId, path: snapPath, instruction: instruction });
        else invoke("add_strand", { x: x, y: y, instruction: instruction });
    }
    function cancelPaletteDrag() { paletteDrag = null; ghost.visible = false; ghost.spec = null; canvas.clearSnap(); canvas.updatePaletteValueTarget(null, null); }
    // A double-click on a palette entry drops it near the top of the view.
    function addAtView(spec) {
        const at = canvas.workspacePoint(canvas.mapToItem(null, 80, 80).x, canvas.mapToItem(null, 80, 80).y) || { x: 0, y: 0 };
        if (spec.kind === "value") invoke("create_floating_value", { x: Math.round(at.x), y: Math.round(at.y), value: spec.value, originBlockId: null });
        else { const i = JSON.parse(JSON.stringify(spec.instruction)); i.id = Blocks.uuid(); invoke("add_strand", { x: Math.round(at.x), y: Math.round(at.y), instruction: i }); }
    }
    // A canvas block dropped on the sidebar deletes it and everything below it.
    function trashDraggedBlocks(strandId, path, tailCount, sx, sy) {
        if (!sidebar.contains(sidebar.mapFromItem(null, sx, sy))) return;
        if (path.length === 1 && path[0].index === 0) { invoke("remove_strand", { strandId: strandId }); return; }
        for (let i = 0; i < tailCount; ++i) invoke("remove_instruction", { strandId: strandId, path: path });
    }
    // A value dropped on the sidebar resets its slot, or deletes a whole floating block.
    function trashDraggedValue(location, value, sx, sy) {
        if (!sidebar.contains(sidebar.mapFromItem(null, sx, sy))) return;
        if (location.kind === "Floating" && !(location.path && location.path.length)) invoke("remove_floating_value", { floatingId: location.floating_id });
        else invoke("take_value", { location: location });
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
                            model: [{ label: "Code", icon: "blocks" }, { label: "Game", icon: "gamepad-2" }, { label: "Interface", icon: "layout-grid" }]
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
                                onDragStarted: (spec, sx, sy, ox, oy) => root.beginPaletteDrag(spec, ox, oy, sx, sy)
                                onDragMoved: (sx, sy) => root.movePaletteDrag(sx, sy)
                                onDragEnded: (sx, sy) => root.endPaletteDrag(sx, sy)
                                onDragCanceled: root.cancelPaletteDrag()
                                onEntryActivated: spec => root.addAtView(spec)
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
                                onBlockDragOutside: (strandId, path, tailCount, sx, sy) => root.trashDraggedBlocks(strandId, path, tailCount, sx, sy)
                                onInstructionRemoved: (strandId, path) => {
                                    const strand = root.strandById(strandId);
                                    root.invoke("delete_instruction", { strandId: strandId, path: path, x: strand ? strand.x + 40 : 0, y: strand ? strand.y + 40 : 0 });
                                }
                                onInstructionDuplicated: (strandId, path, instruction) => root.invoke("add_instruction", { strandId: strandId, path: root.nextPath(path), instruction: Blocks.regenerateIds(instruction) })
                                onInstructionEdited: (strandId, path, instruction) => root.invoke("edit_instruction", { strandId: strandId, path: path, instruction: instruction })
                                onValueEdited: (location, text) => root.invoke("edit_value_field", { location: location, text: text })
                                onValueTakeRequested: location => root.invoke("take_value", { location: location })
                                onValuePutRequested: (location, value) => root.invoke("put_value", { location: location, value: value })
                                onValueCreateRequested: (x, y, value) => root.invoke("create_floating_value", { x: x, y: y, value: value, originBlockId: null })
                                onValueDragOutside: (location, value, sx, sy) => root.trashDraggedValue(location, value, sx, sy)
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
                                onPaletteDragStarted: (spec, sx, sy, ox, oy) => root.beginPaletteDrag(spec, ox, oy, sx, sy)
                                onPaletteDragMoved: (sx, sy) => root.movePaletteDrag(sx, sy)
                                onPaletteDragEnded: (sx, sy) => root.endPaletteDrag(sx, sy)
                                onPaletteDragCanceled: root.cancelPaletteDrag()
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
                    PreviewPanel { app: root.app }
                    UiDesigner { app: root.app }
                }
            }
            InspectorPanel {
                Layout.fillHeight: true
                Layout.preferredWidth: panels.rightOpen ? panels.rightWidth : 34
                app: root.app
                open: panels.rightOpen
                onOpenRequested: open => panels.rightOpen = open
                onResizeRequested: w => panels.rightWidth = Math.max(220, Math.min(480, w))
            }
        }
        AssetTray { Layout.fillWidth: true; app: root.app }
        RunLog { Layout.fillWidth: true; app: root.app }
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

    // Follows the pointer while a palette entry is dragged.
    Item {
        id: ghostLayer
        anchors.fill: parent; z: 500; enabled: false
        Item {
            id: ghost
            property var spec: null
            visible: false; opacity: 0.92
            width: ghostBlock.active ? ghostBlock.item.width : (ghostValue.active ? ghostValue.item.width : 0)
            height: ghostBlock.active ? ghostBlock.item.height : (ghostValue.active ? ghostValue.item.height : 0)
            Loader {
                id: ghostBlock
                active: ghost.visible && !!ghost.spec && ghost.spec.kind !== "value"
                sourceComponent: InstructionBlock {
                    instruction: ghost.spec.instruction; paletteMode: true; locked: true
                    blockColor: ghost.spec.color || Theme.block
                    blockDefinitions: root.actor ? root.actor.block_defs : []
                }
            }
            Loader {
                id: ghostValue
                active: ghost.visible && !!ghost.spec && ghost.spec.kind === "value"
                sourceComponent: ValueChip {
                    valueData: ghost.spec.value; boxed: true; editable: false
                    forceBoolean: !!ghost.spec.forceBoolean; callDisplayLabel: ghost.spec.label || "custom block"
                    blockDefinitions: root.actor ? root.actor.block_defs : []
                }
            }
        }
    }
}
