import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The palette: every block, grouped, plus the value blocks, the variables,
// lists and dicts (this actor's own and the shared ones), and "My Blocks".
// Dragging anything out of here drops a copy of it onto the canvas; dropping
// a canvas block back on it deletes that block.
Rectangle {
    id: root
    required property var app
    required property var editor
    property bool open: true
    property bool trashArmed: false
    signal openRequested(bool open)
    signal resizeRequested(real width)
    signal dragStarted(var spec, real sceneX, real sceneY, real offsetX, real offsetY)
    signal dragMoved(real sceneX, real sceneY)
    signal dragEnded(real sceneX, real sceneY)
    signal dragCanceled()
    signal entryActivated(var spec)
    signal detailsRequested(string name, string identifier, string explainer)
    readonly property var appState: app.appState
    readonly property var project: appState.project
    readonly property var actor: app.openActor
    readonly property var blockDefs: actor ? actor.block_defs : []
    color: Theme.panel
    border.color: Theme.borderSoft
    clip: true

    function sortedNames(items) { return (items || []).map(x => x.name).sort((a, b) => a.localeCompare(b)); }
    readonly property var actorVariables: sortedNames(actor ? actor.variables : [])
    readonly property var globalVariables: sortedNames(project ? project.globals : []).filter(n => actorVariables.indexOf(n) < 0)
    readonly property var actorLists: actor ? actor.lists : []
    readonly property var sharedLists: (project ? project.global_lists : []).filter(l => !actorLists.some(o => o.name === l.name))
    readonly property var actorDicts: actor ? actor.dicts : []
    readonly property var sharedDicts: (project ? project.global_dicts : []).filter(d => !actorDicts.some(o => o.name === d.name))
    readonly property var variableNames: Blocks.variableNames()
    readonly property var listNames: Blocks.listNames()
    readonly property var dictNames: Blocks.dictNames()

    // The block a custom definition's palette entry drops.
    function callInstruction(def) {
        const args = (def.pieces || []).filter(p => p.kind === "Input").map(p => p.value_type === "Bool" ? { kind: "Bool" } : { kind: "Number", value: 0 });
        return { id: "palette-call-" + def.id, type: "CallBlock", block_id: def.id, args: args };
    }
    function callValue(def) {
        const args = (def.pieces || []).filter(p => p.kind === "Input").map(p => p.value_type === "Bool" ? { kind: "Bool" } : { kind: "Number", value: 0 });
        return { kind: "Call", block_id: def.id, args: args, branches: [], saved: { kind: "Number", value: 0 } };
    }
    function isReporter(def) { return def.shape === "ReturnsValue" || def.shape === "ReturnsBool"; }
    function defLabel(def) { return (def.pieces || []).map(p => p.kind === "Label" ? p.text : "(" + p.name + ")").join(" "); }
    // A prefab naming a variable/list/dict follows the first real name until
    // someone picks one, so it never drops onto the canvas pointing at nothing.
    function namesFor(type) {
        if (Blocks.variableCommandTypes.indexOf(type) >= 0) return variableNames;
        if (Blocks.listCommandTypes.indexOf(type) >= 0) return listNames;
        if (Blocks.dictCommandTypes.indexOf(type) >= 0) return dictNames;
        return null;
    }

    component PaletteEntry: PaletteBlock {
        id: entry
        required property string modelData
        readonly property var names: root.namesFor(modelData)
        // Made once: as a binding it was remade (and redrawn) on every edit to the actor.
        Component.onCompleted: instruction = Blocks.fresh(modelData)
        spec: ({ kind: "instruction", type: modelData, instruction: instruction })
        blockDefinitions: root.blockDefs
        onNamesChanged: if (instruction && names && names.length && names.indexOf(instruction.name) < 0) { const next = JSON.parse(JSON.stringify(instruction)); next.name = names[0]; instruction = next; }
        onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
        onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
        onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
        onDragCanceled: root.dragCanceled()
        onActivated: root.entryActivated(spec)
        onDetailsRequested: type => root.detailsRequested(Blocks.labels[type] || type, type, "")
    }
    component PluginEntry: PaletteBlock {
        id: entry
        required property var modelData
        Component.onCompleted: instruction = Blocks.pluginFresh(modelData)
        spec: ({ kind: "instruction", type: instruction ? instruction.type : "PluginBlock", instruction: instruction })
        blockDefinitions: root.blockDefs
        onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
        onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
        onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
        onDragCanceled: root.dragCanceled()
        onActivated: root.entryActivated(spec)
        onDetailsRequested: root.detailsRequested(modelData.block.type_id, instruction ? instruction.type : "PluginBlock", modelData.block.help || ("A block from the " + modelData.plugin + " plugin."))
    }
    component PluginReporterEntry: PaletteValue {
        id: entry
        required property var modelData
        valueData: Blocks.pluginValue(modelData)
        forceBoolean: Blocks.pluginIsBool(modelData)
        spec: ({ kind: "value", value: valueData, forceBoolean: forceBoolean })
        blockDefinitions: root.blockDefs
        onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
        onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
        onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
        onDragCanceled: root.dragCanceled()
        onActivated: root.entryActivated(spec)
        onDetailsRequested: root.detailsRequested(modelData.block.type_id, "PluginRead", modelData.block.help || ("A reporter from the " + modelData.plugin + " plugin: drop it into any slot that takes a value."))
    }
    component OperatorEntry: PaletteValue {
        id: entry
        required property string modelData
        readonly property var names: Blocks.listOperatorKinds.indexOf(modelData) >= 0 ? root.listNames : Blocks.dictOperatorKinds.indexOf(modelData) >= 0 ? root.dictNames : null
        valueData: Blocks.operatorValue(modelData)
        forceBoolean: Blocks.isBoolKind(modelData)
        spec: ({ kind: "value", value: valueData, forceBoolean: forceBoolean })
        blockDefinitions: root.blockDefs
        onNamesChanged: {
            const spec = BlockRegistry.operator(valueData.op);
            if (!names || !names.length || !spec || !spec.enumArg) return;
            const arg = valueData.args[spec.enumArg.index];
            if (arg && arg.kind === "Text" && names.indexOf(arg.value) < 0) {
                const next = JSON.parse(JSON.stringify(valueData));
                next.args[spec.enumArg.index] = { kind: "Text", value: names[0] };
                valueData = next;
            }
        }
        onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
        onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
        onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
        onDragCanceled: root.dragCanceled()
        onActivated: root.entryActivated(spec)
        onDetailsRequested: kind => root.detailsRequested(kind, kind, "A value block: drop it into any slot that takes one.")
    }
    component Heading: RowLayout {
        property string label: ""
        property string action: ""
        signal triggered()
        width: parent ? parent.width - 16 : 200
        spacing: 8
        SectionLabel { label: parent.label; Layout.fillWidth: true }
        BwButton { visible: parent.action.length > 0; text: parent.action; implicitHeight: 26; font.pixelSize: 11; Layout.topMargin: 8; onClicked: parent.triggered() }
    }
    component Note: Text {
        width: parent ? parent.width - 16 : 200
        wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
    }
    component VariableEntry: RowLayout {
        id: variableRow
        required property string modelData
        spacing: 4
        PaletteValue {
            valueData: ({ kind: "Var", name: variableRow.modelData }); editable: false
            spec: ({ kind: "value", value: valueData })
            blockDefinitions: root.blockDefs
            onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
            onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
            onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
            onDragCanceled: root.dragCanceled()
            onActivated: root.entryActivated(spec)
        }
        IconButton { iconName: "pencil"; tip: "Rename “" + variableRow.modelData + "”"; implicitWidth: 24; implicitHeight: 24; onClicked: variableDialog.openForRename(variableRow.modelData) }
        IconButton { iconName: "trash-2"; tip: "Delete “" + variableRow.modelData + "”"; danger: true; implicitWidth: 24; implicitHeight: 24; onClicked: root.app.invoke("delete_variable", { name: variableRow.modelData }) }
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        visible: root.open
        Rectangle {
            Layout.fillWidth: true; Layout.preferredHeight: 60
            color: root.trashArmed ? "#33ff4848" : "transparent"
            Column {
                anchors.centerIn: parent; spacing: 4
                LucideIcon { anchors.horizontalCenter: parent.horizontalCenter; name: "trash-2"; color: root.trashArmed ? Theme.danger : Theme.textDim; width: 18; height: 18; opacity: root.trashArmed ? 1 : .65 }
                Text { text: root.trashArmed ? "Release to delete" : "Drag a block here to delete it"; color: root.trashArmed ? Theme.danger : Theme.textDim; font.pixelSize: 11 }
            }
            IconButton { anchors.right: parent.right; anchors.top: parent.top; anchors.margins: 4; iconName: "chevron-left"; tip: "Hide the blocks"; implicitWidth: 24; implicitHeight: 24; onClicked: root.openRequested(false) }
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.borderSoft }
        ScrollView {
            id: scroll
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            contentWidth: availableWidth
            Column {
                width: scroll.availableWidth; spacing: 6; leftPadding: 8; topPadding: 4; bottomPadding: 16
                Repeater {
                    model: Blocks.blockGroups
                    delegate: Column {
                        required property var modelData
                        spacing: 6
                        SectionLabel { label: modelData.label }
                        Repeater { model: modelData.types; delegate: PaletteEntry {} }
                    }
                }
                Repeater {
                    model: Blocks.pluginGroups
                    delegate: Column {
                        required property var modelData
                        spacing: 6
                        SectionLabel { label: modelData.label }
                        Repeater { model: modelData.entries; delegate: PluginEntry {} }
                        Repeater { model: modelData.reporters; delegate: PluginReporterEntry {} }
                    }
                }
                Repeater {
                    model: Blocks.operatorGroups
                    delegate: Column {
                        required property var modelData
                        spacing: 6
                        SectionLabel { label: modelData.label }
                        Repeater { model: modelData.kinds; delegate: OperatorEntry {} }
                    }
                }
                SectionLabel { label: "Values" }
                Repeater {
                    model: [{ kind: "Number", value: 0 }, { kind: "Text", value: "" }]
                    delegate: PaletteValue {
                        required property var modelData
                        valueData: modelData; spec: ({ kind: "value", value: valueData })
                        onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
                        onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
                        onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
                        onDragCanceled: root.dragCanceled()
                        onActivated: root.entryActivated(spec)
                    }
                }

                Heading { label: "This actor's variables"; action: "New"; onTriggered: variableDialog.openFor("actor") }
                Repeater { model: root.actorVariables; delegate: VariableEntry {} }
                Heading { label: "Shared variables"; action: "New"; onTriggered: variableDialog.openFor("global") }
                Repeater { model: root.globalVariables; delegate: VariableEntry {} }
                Repeater { model: root.variableNames.length ? Blocks.variableCommandTypes : []; delegate: PaletteEntry {} }
                Note { visible: !root.variableNames.length; text: "A variable remembers a number or some text - a score, a level, a name." }

                Heading { label: "This actor's lists"; action: "New"; onTriggered: listDialog.openFor("actor") }
                CollectionPanel {
                    collections: root.actorLists; noun: "list"; rowWidth: scroll.availableWidth - 20
                    onEditorStateRequested: (name, visible, x, y) => root.app.invoke("set_list_editor_state", { name: name, visible: visible, x: x, y: y })
                    onRenameRequested: name => listDialog.openForRename(name)
                    onDeleteRequested: name => root.app.invoke("delete_list", { name: name })
                    onDetailsRequested: name => root.detailsRequested(name, "List", "An ordered collection of numbers and text only this actor sees.")
                }
                Heading { label: "Shared lists"; action: "New"; onTriggered: listDialog.openFor("global") }
                CollectionPanel {
                    collections: root.sharedLists; noun: "list"; rowWidth: scroll.availableWidth - 20
                    onEditorStateRequested: (name, visible, x, y) => root.app.invoke("set_list_editor_state", { name: name, visible: visible, x: x, y: y })
                    onRenameRequested: name => listDialog.openForRename(name)
                    onDeleteRequested: name => root.app.invoke("delete_list", { name: name })
                    onDetailsRequested: name => root.detailsRequested(name, "List", "An ordered collection of numbers and text every actor shares.")
                }
                Repeater { model: root.listNames.length ? Blocks.listOperatorKinds : []; delegate: OperatorEntry {} }
                Repeater { model: root.listNames.length ? Blocks.listCommandTypes : []; delegate: PaletteEntry {} }
                Note { visible: !root.listNames.length; text: "A list holds numbers or text in order - a queue, a hand of cards, a high-score table." }

                Heading { label: "This actor's dicts"; action: "New"; onTriggered: dictDialog.openFor("actor") }
                CollectionPanel {
                    collections: root.actorDicts; noun: "dict"; rowWidth: scroll.availableWidth - 20
                    onEditorStateRequested: (name, visible, x, y) => root.app.invoke("set_dict_editor_state", { name: name, visible: visible, x: x, y: y })
                    onRenameRequested: name => dictDialog.openForRename(name)
                    onDeleteRequested: name => root.app.invoke("delete_dict", { name: name })
                    onDetailsRequested: name => root.detailsRequested(name, "Dict", "Numbers and text by key, only this actor sees.")
                }
                Heading { label: "Shared dicts"; action: "New"; onTriggered: dictDialog.openFor("global") }
                CollectionPanel {
                    collections: root.sharedDicts; noun: "dict"; rowWidth: scroll.availableWidth - 20
                    onEditorStateRequested: (name, visible, x, y) => root.app.invoke("set_dict_editor_state", { name: name, visible: visible, x: x, y: y })
                    onRenameRequested: name => dictDialog.openForRename(name)
                    onDeleteRequested: name => root.app.invoke("delete_dict", { name: name })
                    onDetailsRequested: name => root.detailsRequested(name, "Dict", "Numbers and text by key, every actor shares.")
                }
                Repeater { model: root.dictNames.length ? Blocks.dictOperatorKinds : []; delegate: OperatorEntry {} }
                Repeater { model: root.dictNames.length ? Blocks.dictCommandTypes : []; delegate: PaletteEntry {} }
                Note { visible: !root.dictNames.length; text: "A dict holds numbers or text by key - a save slot, an inventory, a settings table." }

                Heading { label: "My Blocks"; action: "Make a Block"; onTriggered: blockDialog.openForCreate() }
                Repeater {
                    model: root.blockDefs
                    delegate: RowLayout {
                        id: myBlock
                        required property var modelData
                        spacing: 4
                        Loader {
                            sourceComponent: root.isReporter(myBlock.modelData) ? reporterEntry : commandEntry
                            Component {
                                id: commandEntry
                                PaletteBlock {
                                    instruction: root.callInstruction(myBlock.modelData)
                                    spec: ({ kind: "instruction", type: "CallBlock", instruction: instruction })
                                    blockDefinitions: root.blockDefs
                                    onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
                                    onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
                                    onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
                                    onDragCanceled: root.dragCanceled()
                                    onActivated: root.entryActivated(spec)
                                }
                            }
                            Component {
                                id: reporterEntry
                                PaletteValue {
                                    valueData: root.callValue(myBlock.modelData); editable: false
                                    forceBoolean: myBlock.modelData.shape === "ReturnsBool"
                                    spec: ({ kind: "value", value: valueData, forceBoolean: forceBoolean })
                                    blockDefinitions: root.blockDefs
                                    onDragStarted: (sp, sx, sy, ox, oy) => root.dragStarted(sp, sx, sy, ox, oy)
                                    onDragMoved: (sx, sy) => root.dragMoved(sx, sy)
                                    onDragEnded: (sx, sy) => root.dragEnded(sx, sy)
                                    onDragCanceled: root.dragCanceled()
                                    onActivated: root.entryActivated(spec)
                                }
                            }
                        }
                        IconButton { iconName: "pencil"; tip: "Edit this block"; implicitWidth: 24; implicitHeight: 24; onClicked: blockDialog.openForEdit(myBlock.modelData) }
                        IconButton { iconName: "scissors"; tip: "Delete this block"; danger: true; implicitWidth: 24; implicitHeight: 24; onClicked: root.app.invoke("delete_block", { blockId: myBlock.modelData.id }) }
                    }
                }
                PaletteEntry { modelData: "Return" }
            }
        }
    }
    // The outer edge drags to resize.
    MouseArea {
        visible: root.open
        anchors.top: parent.top; anchors.bottom: parent.bottom; anchors.right: parent.right; width: 5
        cursorShape: Qt.SizeHorCursor
        property real startWidth: 0; property real startX: 0
        onPressed: mouse => { startWidth = root.width; startX = mapToItem(null, mouse.x, 0).x; }
        onPositionChanged: mouse => { if (pressed) root.resizeRequested(startWidth + mapToItem(null, mouse.x, 0).x - startX); }
    }
    IconButton {
        visible: !root.open
        anchors.horizontalCenter: parent.horizontalCenter; y: 10
        iconName: "chevron-right"; tip: "Show the blocks"
        onClicked: root.openRequested(true)
    }

    // Naming a variable, list or dict: new (in this actor or shared) or renamed.
    component ScopedNameDialog: NameDialog {
        id: dialog
        property string scope: "actor"
        property string createCommand: ""
        property string renameCommand: ""
        function openFor(s) { scope = s; openForCreate(); }
        title: renameTarget.length ? "Rename “" + renameTarget + "”" : (scope === "global" ? "New shared " + noun : "New " + noun + " for this actor")
        onSubmitted: (name, target) => {
            if (target.length) root.app.invoke(renameCommand, { oldName: target, newName: name }, () => {}, e => dialog.fail(String(e)));
            else root.app.invoke(createCommand, { name: name, scope: scope }, () => {}, e => dialog.fail(String(e)));
        }
        Text {
            visible: !dialog.renameTarget.length; width: 340; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: dialog.scope === "global" ? "Every actor can read and write a shared " + dialog.noun + "." : "Only this actor can see its own " + dialog.noun + "s."
        }
    }
    ScopedNameDialog { id: variableDialog; noun: "variable"; placeholder: "score"; createCommand: "create_variable"; renameCommand: "rename_variable" }
    ScopedNameDialog { id: listDialog; noun: "list"; placeholder: "items"; createCommand: "create_list"; renameCommand: "rename_list" }
    ScopedNameDialog { id: dictDialog; noun: "dict"; placeholder: "stats"; createCommand: "create_dict"; renameCommand: "rename_dict" }
    MakeBlockDialog { id: blockDialog; app: root.app }
}
