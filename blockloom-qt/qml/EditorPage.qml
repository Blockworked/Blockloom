import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The editor a project opens into: the top bar and its menus, then three
// dock regions (actors on the left, the inspector on the right, assets, run
// log and problems along the bottom) around the Code and Game views, which
// can sit side by side.
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
        property real blocksWidth: 380
        property int tab: 0
        property bool split: false
        property real splitRatio: 0.5
        property bool bottomOpen: true
        property real bottomHeight: 180
        property int bottomTab: 0
        // The panel shown in each region, and where the user moved panels to
        // ({panel id: "left" | "right" | "bottom"}, as JSON).
        property string leftPanel: "actors"
        property string rightPanel: "inspector"
        property string bottomPanel: "assets"
        property string layout: "{}"
    }

    // Every dockable panel: the editor's own and one per plugin panel.
    readonly property var builtinPanels: [
        { id: "actors", title: "Actors", iconName: "user", edge: "left" },
        { id: "inspector", title: "Inspector", iconName: "sliders-horizontal", edge: "right" },
        { id: "assets", title: "Assets", iconName: "folder", edge: "bottom" },
        { id: "runLog", title: "Run log", iconName: "scroll", edge: "bottom" },
        { id: "problems", title: "Problems", iconName: "triangle-alert", edge: "bottom" },
        { id: "profiler", title: "Profiler", iconName: "gauge", edge: "bottom" }
    ]
    readonly property var pluginPanels: appState.plugins && appState.plugins.panels ? appState.plugins.panels : []
    function pluginPanelId(p) { return "plugin:" + p.plugin + "/" + p.panel.name; }
    readonly property var panelDefs: builtinPanels.concat(pluginPanels.map(p =>
        ({ id: root.pluginPanelId(p), title: p.panel.title, iconName: "plug-zap", edge: "bottom" })))
    readonly property var placed: { try { return JSON.parse(panels.layout || "{}"); } catch (e) { return {}; } }
    function edgeOf(id) {
        const def = root.panelDefs.find(d => d.id === id);
        return root.placed[id] || (def ? def.edge : "bottom");
    }
    function idsAt(edge) { return root.panelDefs.filter(d => root.edgeOf(d.id) === edge).map(d => d.id); }
    function tabOf(id) {
        const def = root.panelDefs.find(d => d.id === id) || { title: id, iconName: "square" };
        const badge = id === "problems" && problems.problems.length ? String(problems.problems.length) : "";
        return { title: def.title, iconName: def.iconName, badge: badge };
    }
    function regionOpen(edge) { return edge === "left" ? panels.leftOpen : edge === "right" ? panels.rightOpen : panels.bottomOpen; }
    function setOpen(edge, open) {
        if (edge === "left") panels.leftOpen = open;
        else if (edge === "right") panels.rightOpen = open;
        else panels.bottomOpen = open;
    }
    function currentAt(edge) { return edge === "left" ? panels.leftPanel : edge === "right" ? panels.rightPanel : panels.bottomPanel; }
    function setCurrent(edge, id) {
        if (edge === "left") panels.leftPanel = id;
        else if (edge === "right") panels.rightPanel = id;
        else panels.bottomPanel = id;
    }
    function showPanel(id) { const edge = root.edgeOf(id); root.setCurrent(edge, id); root.setOpen(edge, true); }
    function togglePanel(id) {
        const edge = root.edgeOf(id);
        if (root.regionOpen(edge) && root.currentAt(edge) === id) root.setOpen(edge, false);
        else root.showPanel(id);
    }
    function panelShown(id) { const edge = root.edgeOf(id); return root.regionOpen(edge) && root.currentAt(edge) === id; }
    // Moves a panel to another region and shows it there.
    function movePanel(id, edge) {
        const next = Object.assign({}, root.placed);
        next[id] = edge;
        panels.layout = JSON.stringify(next);
        root.showPanel(id);
    }
    function panelItem(id) {
        const own = { actors: actorsPanel, inspector: inspector, assets: assetsPanel, runLog: runLogPanel, problems: problems, profiler: profilerPanel };
        if (own[id]) return own[id];
        for (let i = 0; i < pluginPanelItems.count; ++i) {
            const item = pluginPanelItems.objectAt(i);
            if (item && root.pluginPanelId(item.panel) === id) return item;
        }
        return null;
    }

    EditorActions { id: actions; app: root.app; page: root }
    // Actions that show or fold one panel, wherever it docks.
    readonly property var panelActions: ({ "editor.assets": "assets", "editor.runLog": "runLog", "editor.problems": "problems", "editor.profiler": "profiler" })
    // The window's own actions, which EditorActions hands back here.
    function editorAction(id) {
        const views = ["editor.showCode", "editor.showGame", "editor.showInterface", "editor.showDevices"];
        const dialogs = { "editor.build": "build", "editor.plugins": "plugins",
                          "editor.pluginEditors": "pluginEditors", "editor.projectSettings": "projectSettings",
                          "editor.appSettings": "appSettings", "editor.import": "import", "editor.export": "export" };
        if (id === "editor.palette") palette.open();
        else if (id === "editor.shortcuts") keymapDialog.open();
        else if (views.indexOf(id) >= 0) panels.tab = views.indexOf(id);
        else if (id === "editor.split") { panels.split = !panels.split; if (panels.split && panels.tab > 1) panels.tab = 0; }
        else if (root.panelActions[id]) root.togglePanel(root.panelActions[id]);
        else if (id === "editor.pluginPanels" && root.pluginPanels.length) root.showPanel(root.pluginPanelId(root.pluginPanels[0]));
        else if (dialogs[id]) topBar.openDialog(dialogs[id]);
    }
    function actionChecked(id) {
        if (id === "editor.split") return panels.split;
        return !!root.panelActions[id] && root.panelShown(root.panelActions[id]);
    }
    // The keymap's shortcuts, one per bound action.
    Instantiator {
        model: Object.keys(actions.keymap)
        delegate: Shortcut {
            required property string modelData
            sequences: actions.keys(modelData)
            enabled: actions.enabled(modelData)
            onActivated: actions.run(modelData)
        }
    }
    CommandPalette { id: palette; actions: actions }
    KeymapDialog { id: keymapDialog; actions: actions }

    Connections {
        target: root.app
        function onInspectedLightingChanged() { if (root.app.inspectedLighting) root.showPanel("inspector"); }
        function onInspectedSceneChanged() { if (root.app.inspectedScene) root.showPanel("inspector"); }
        function onInspectSceneChanged() { if (root.app.inspectScene) root.showPanel("inspector"); }
    }
    readonly property bool running: appState.running === true
    // Play shows the game it started.
    onRunningChanged: if (running) { if (panels.tab > 1 || !panels.split) panels.tab = 1; if (panels.bottomOpen && panels.bottomTab === 0) panels.bottomTab = 1; }
    readonly property bool codeShown: panels.tab === 0 || (panels.split && panels.tab === 1)
    readonly property bool gameShown: panels.tab === 1 || (panels.split && panels.tab === 0)
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
        const instruction = Blocks.realInstruction(edit.instruction);
        const value = Blocks.realValue(edit.value);
        if (edit.kind === "insert_instruction") call("add_instruction", { strandId: edit.strandId, path: edit.path, instruction: instruction, targetY: edit.targetY === undefined ? null : edit.targetY });
        else if (edit.kind === "add_strand") call("add_strand", { x: edit.x, y: edit.y, instruction: instruction });
        else if (edit.kind === "put_value") call("put_value", { location: edit.location, value: value });
        else if (edit.kind === "create_floating_value") call("create_floating_value", { x: edit.x, y: edit.y, value: value, originBlockId: edit.originBlockId || null });
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
        TopBar { id: topBar; Layout.fillWidth: true; app: root.app; actions: actions }
        Rectangle {
            visible: !root.appState.runtime_available
            Layout.fillWidth: true; implicitHeight: 36; color: Theme.warningBackground
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.warningBorder }
            Row {
                anchors.centerIn: parent; spacing: 8
                LucideIcon { name: "triangle-alert"; width: 16; height: 16; color: Theme.warning; anchors.verticalCenter: parent.verticalCenter }
                Text { anchors.verticalCenter: parent.verticalCenter; color: Theme.text; font.pixelSize: 13
                    text: "The game runtime is missing, so Play has nothing to open. Build the whole workspace (just build), not only the editor." }
            }
        }
        RowLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; spacing: 0
            PanelDock {
                page: root
                objectName: "dock-left"
                visible: panels.tab !== 2
                edge: "left"
                Layout.fillHeight: true
                Layout.preferredWidth: implicitWidth
                open: panels.leftOpen; size: panels.leftWidth; minSize: 180; maxSize: 420
                onOpenRequested: open => panels.leftOpen = open
                onResizeRequested: w => panels.leftWidth = w
            }
            ColumnLayout {
                Layout.fillWidth: true; Layout.fillHeight: true; spacing: 0
                Rectangle {
                    Layout.fillWidth: true; implicitHeight: 36
                    color: Theme.panel
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.borderSoft }
                    RowLayout {
                        anchors.fill: parent; anchors.leftMargin: 6; anchors.rightMargin: 6; spacing: 2
                        IconButton {
                            visible: root.codeShown && !!root.actor
                            iconName: "panel-left"; tip: panels.blocksOpen ? "Hide the blocks" : "Show the blocks"
                            implicitWidth: 28; implicitHeight: 28
                            iconColor: panels.blocksOpen ? Theme.accent : "transparent"
                            onClicked: panels.blocksOpen = !panels.blocksOpen
                        }
                        Repeater {
                            model: [{ label: "Code", icon: "blocks" }, { label: "Game", icon: "gamepad-2" }, { label: "Interface", icon: "layout-grid" }, { label: "Devices", icon: "smartphone" }]
                            delegate: Rectangle {
                                id: tab
                                required property var modelData
                                required property int index
                                readonly property bool current: panels.tab === index || (panels.split && index <= 1 && panels.tab <= 1)
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
                        IconButton {
                            objectName: "split-button"
                            visible: panels.tab <= 1
                            iconName: "columns-2"; tip: panels.split ? "Show one view" : "Show the game beside the code"
                            implicitWidth: 28; implicitHeight: 28
                            iconColor: panels.split ? Theme.accent : "transparent"
                            onClicked: actions.run("editor.split")
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
                    currentIndex: panels.tab <= 1 ? 0 : panels.tab - 1
                    // Code and Game, one at a time or side by side.
                    Item {
                    id: codeGame
                    readonly property bool both: root.codeShown && root.gameShown
                    readonly property real codeWidth: both ? Math.round(width * Math.max(0.2, Math.min(0.8, panels.splitRatio))) : width
                    Item {
                        id: codePane
                        objectName: "code-pane"
                        visible: root.codeShown
                        width: codeGame.codeWidth; height: parent.height
                        RowLayout {
                            anchors.fill: parent; spacing: 0
                            visible: !!root.actor
                            BlockSidebar {
                                id: sidebar
                                visible: panels.blocksOpen
                                Layout.fillHeight: true
                                Layout.preferredWidth: panels.blocksWidth
                                app: root.app
                                trashActive: canvas.dragging
                                trashArmed: canvas.dragging && sidebar.contains(sidebar.mapFromItem(null, canvas.dragSceneX, canvas.dragSceneY))
                                onResizeRequested: w => panels.blocksWidth = Math.max(280, Math.min(640, w))
                                onDragStarted: (spec, sx, sy, ox, oy) => dragController.begin(spec, sx, sy, ox, oy)
                                onDragMoved: (sx, sy) => dragController.move(sx, sy)
                                onDragEnded: (sx, sy) => dragController.end(sx, sy)
                                onDragCanceled: dragController.cancel()
                                onEntryActivated: spec => dragController.addAtView(spec)
                                onDetailsShown: (name, identifier, explainer) => details.show(name, identifier, explainer)
                            }
                            BlockCanvas {
                                id: canvas
                                Layout.fillWidth: true; Layout.fillHeight: true
                                offerClear: false
                                documentId: root.actor ? root.actor.id : ""
                                strands: root.actor ? root.actor.strands : []
                                comments: root.actor ? root.actor.comments : []
                                floatingValues: root.actor ? root.actor.floating_values : []
                                variables: Blocks.variableNames()
                                lists: root.lists
                                dicts: root.dicts
                                blockDefinitions: root.actor ? root.actor.block_defs : []
                                onStrandMoved: (strandId, x, y) => root.invoke("move_strand", { strandId: strandId, x: x, y: y })
                                onInstructionSplit: (strandId, path, x, y) => root.invoke("split_strand", { strandId: strandId, path: path, x: x, y: y })
                                onStrandsMerged: (draggedId, targetId, path, targetY) => root.invoke("merge_strand", { draggedId: draggedId, targetId: targetId, path: path, targetY: targetY === undefined ? null : targetY })
                                onTailMerged: (strandId, path, targetId, targetPath, targetY) => root.invoke("merge_tail", { strandId: strandId, path: path, targetId: targetId, targetPath: targetPath, targetY: targetY === undefined ? null : targetY })
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
                                onDetailsRequested: type => details.show(Blocks.labelFor(type), type, "")
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
                                // A branch chip on a block's header drops a `run branch` right under it.
                                onRunBranchRequested: (strandId, path, name) => {
                                    const next = JSON.parse(JSON.stringify(path));
                                    next[next.length - 1].index++;
                                    root.invoke("add_instruction", { strandId: strandId, path: next, instruction: { id: Blocks.uuid(), type: "RunBranch", name: name } });
                                }
                                onCanvasMenuAction: (action, x, y) => root.onCanvasMenu(action, x, y)
                            }
                        }
                        Item {
                            visible: !root.actor
                            anchors.fill: parent
                            Column {
                                anchors.centerIn: parent; spacing: 8
                                LucideIcon { anchors.horizontalCenter: parent.horizontalCenter; name: "blocks"; color: Theme.textDim; width: 48; height: 48 }
                                Text { anchors.horizontalCenter: parent.horizontalCenter; text: "No actor selected"; color: Theme.text; font.pixelSize: 16; font.weight: Font.DemiBold }
                                Text { text: "Pick an actor on the left, or add one, to see its blocks."; color: Theme.textDim; font.pixelSize: 13 }
                            }
                        }
                    }
                    Rectangle {
                        objectName: "split-divider"
                        visible: codeGame.both
                        x: codeGame.codeWidth; width: 4; height: parent.height
                        color: dividerMouse.containsMouse || dividerMouse.pressed ? Theme.accent : Theme.borderSoft
                        MouseArea {
                            id: dividerMouse
                            anchors.fill: parent; anchors.leftMargin: -3; anchors.rightMargin: -3
                            hoverEnabled: true; cursorShape: Qt.SizeHorCursor
                            onPositionChanged: mouse => { if (pressed) panels.splitRatio = mapToItem(codeGame, mouse.x, 0).x / codeGame.width; }
                        }
                    }
                    Loader {
                        objectName: "game-pane"
                        active: root.gameShown
                        visible: active
                        x: codeGame.both ? codeGame.codeWidth + 4 : 0
                        width: codeGame.both ? parent.width - x : parent.width; height: parent.height
                        sourceComponent: Component { PreviewPanel { app: root.app } }
                    }
                    }
                    UiDesigner { app: root.app }
                    DevicesPanel { app: root.app }
                }
            }
            PanelDock {
                page: root
                objectName: "dock-right"
                visible: panels.tab !== 2
                edge: "right"
                Layout.fillHeight: true
                Layout.preferredWidth: implicitWidth
                open: panels.rightOpen; size: panels.rightWidth; minSize: 220; maxSize: 480
                onOpenRequested: open => panels.rightOpen = open
                onResizeRequested: w => panels.rightWidth = w
            }
        }
        PanelDock {
            page: root
            id: bottomDock
            objectName: "dock-bottom"
            edge: "bottom"
            Layout.fillWidth: true
            Layout.preferredHeight: implicitHeight
            open: panels.bottomOpen; size: panels.bottomHeight; minSize: 90; maxSize: 560
            onOpenRequested: open => panels.bottomOpen = open
            onResizeRequested: h => panels.bottomHeight = h
        }
    }

    // Each panel exists once; the region it docks in borrows it (DockedPanels).
    Item {
        id: pool
        visible: false
        ActorList { id: actorsPanel; app: root.app; docked: true; onOpenRequested: open => root.setOpen(root.edgeOf("actors"), open) }
        InspectorPanel { id: inspector; app: root.app; docked: true; onOpenRequested: open => root.setOpen(root.edgeOf("inspector"), open) }
        AssetTray { id: assetsPanel; app: root.app; docked: true }
        RunLog { id: runLogPanel; app: root.app; docked: true; onOpenScriptRequested: (path, line) => inspector.openScript(path, line) }
        ProblemsPanel { id: problems; app: root.app; onOpenScriptRequested: (path, line) => inspector.openScript(path, line) }
        ProfilerPanel { id: profilerPanel; app: root.app }
    }
    Instantiator {
        id: pluginPanelItems
        model: root.pluginPanels
        delegate: PluginPanelView { required property var modelData; parent: pool; app: root.app; panel: modelData }
    }

    StitchDialog {
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
            Row { anchors.right: parent.right; anchors.rightMargin: 18; StitchButton { text: "Close"; onClicked: details.close() } }
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
