import QtQuick
import QtQuick.Controls
import com.blockworked.Blockstitch 1.0

// "Make a Block": blockstitch's shared definition editor, limited to what
// the VM runs, branches on any shape.
StitchDialog {
    id: root
    required property var app
    property var editTarget: null
    title: editTarget ? "Edit block" : "Make a block"
    width: 720
    height: visible ? Math.min(760, parent ? parent.height - 32 : 760) : 760
    standardButtons: Dialog.NoButton

    function openForCreate() { editTarget = null; open(); }
    function openForEdit(def) { editTarget = def; open(); }
    function send(command, args) {
        app.invoke(command, args, () => root.close(), e => editor.fail(e));
    }
    onOpened: {
        if (editTarget) editor.load(editTarget);
        else editor.reset();
        editor.focusFirst();
    }
    onClosed: editor.closePopups()

    contentItem: BlockDefinitionEditor {
        id: editor
        submitText: root.editTarget ? "Save" : "Create"
        detailsNote: "A public identifier and a description, kept with the block in the project."
        onCreateRequested: draft => root.send("create_block", draft)
        onEditRequested: (blockId, draft) => root.send("edit_block", Object.assign({ blockId: blockId }, draft))
        onCancelled: root.close()
    }
}
