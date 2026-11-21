import QtQuick
import QtQuick.Controls
import com.blockworked.Blockstitch 1.0

// The palette: blockstitch's PalettePanel over Blockloom's blocks
// (Blocks.qml's categories and sections), and the dialogs its buttons open.
// Dragging an entry out drops a copy on the canvas; dropping a canvas block
// back on it deletes that block.
PalettePanel {
    id: root
    required property var app
    readonly property var appState: app.appState
    readonly property var project: appState.project
    readonly property var actor: app.openActor
    // A click on an entry: the editor adds it where the canvas is looking.
    signal entryActivated(var spec)
    signal detailsShown(string name, string identifier, string explainer)

    customBlockDetails: "A custom block this actor defines."
    customReporterDetails: "A custom reporter this actor defines."
    sectionsListBlocks: true
    sections: Blocks.paletteSections
    appCategories: Blocks.appCategories
    extraOperators: Blocks.extraOperators
    listInstructionTypes: Blocks.listCommandTypes
    dictsEnabled: true
    dictInstructionTypes: Blocks.dictCommandTypes
    accordionCategory: "motion"
    // The names as a string, so an edit that doesn't touch them leaves the
    // palette's variable list alone.
    readonly property string variableKey: JSON.stringify(Blocks.variableNames())
    variables: JSON.parse(variableKey)
    lists: actor ? Blocks.visibleCollections(actor.lists, project.global_lists) : []
    dicts: actor ? Blocks.visibleCollections(actor.dicts, project.global_dicts) : []
    blockDefinitions: actor ? actor.block_defs : []

    onBlockActivated: (type, instruction) => entryActivated({ kind: "instruction", type: type, instruction: instruction })
    onCustomBlockActivated: definition => entryActivated({ kind: "instruction", type: "CallBlock", instruction: customInstruction(definition) })
    onValueActivated: value => entryActivated({ kind: "value", value: value })
    onDetailsRequested: (name, identifier, explainer) => detailsShown(Blocks.labelFor(name), identifier, explainer)

    onMakeVariableRequested: variableDialog.openFor("actor")
    onRenameVariableRequested: name => variableDialog.openForRename(name)
    onDeleteVariableRequested: name => app.invoke("delete_variable", { name: name })
    onMakeListRequested: listDialog.openFor("actor")
    onRenameListRequested: name => listDialog.openForRename(name)
    onDeleteListRequested: name => app.invoke("delete_list", { name: name })
    onListEditorStateRequested: (name, visible, x, y) => app.invoke("set_list_editor_state", { name: name, visible: visible, x: x, y: y })
    onMakeDictRequested: dictDialog.openFor("actor")
    onRenameDictRequested: name => dictDialog.openForRename(name)
    onDeleteDictRequested: name => app.invoke("delete_dict", { name: name })
    onDictEditorStateRequested: (name, visible, x, y) => app.invoke("set_dict_editor_state", { name: name, visible: visible, x: x, y: y })
    onMakeBlockRequested: blockDialog.openForCreate()
    onEditBlockRequested: definition => blockDialog.openForEdit(definition)
    onDeleteBlockRequested: blockId => app.invoke("delete_block", { blockId: blockId })

    // Naming a variable, list or dict: new (this actor's own or shared) or renamed.
    component ScopedNameDialog: NameDialog {
        id: dialog
        property string scope: "actor"
        property string createCommand: ""
        property string renameCommand: ""
        function openFor(s) { scope = s; openForCreate(); }
        onSubmitted: (name, target) => {
            if (target.length) root.app.invoke(renameCommand, { oldName: target, newName: name }, () => dialog.close(), e => dialog.fail(String(e)));
            else root.app.invoke(createCommand, { name: name, scope: scope }, () => dialog.close(), e => dialog.fail(String(e)));
        }
        StitchCheckBox {
            visible: !dialog.renameTarget.length
            text: "Share with every actor"
            checked: dialog.scope === "global"
            onToggled: dialog.scope = checked ? "global" : "actor"
        }
        Text {
            visible: !dialog.renameTarget.length; width: parent ? parent.width : 320; wrapMode: Text.WordWrap
            color: Theme.textDim; font.pixelSize: 12
            text: dialog.scope === "global" ? "Every actor can read and change a shared " + dialog.noun + "." : "Only this actor can see its own " + dialog.noun + "s."
        }
    }
    ScopedNameDialog { id: variableDialog; noun: "variable"; placeholder: "score"; createCommand: "create_variable"; renameCommand: "rename_variable" }
    ScopedNameDialog { id: listDialog; noun: "list"; placeholder: "items"; createCommand: "create_list"; renameCommand: "rename_list" }
    ScopedNameDialog { id: dictDialog; noun: "dict"; placeholder: "stats"; createCommand: "create_dict"; renameCommand: "rename_dict" }
    MakeBlockDialog { id: blockDialog; app: root.app }
}
