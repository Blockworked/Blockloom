import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// "Make a Block": a name, some inputs, a shape and a color. The pieces it
// builds are exactly what the backend stores as the block's prototype -
// the name as one label, then every input.
BwDialog {
    id: root
    required property var app
    property var editTarget: null
    property string shape: "Normal"
    property string color: "#4C97FF"
    property var inputs: []   // [{ id, name, bool }]
    property string error: ""
    readonly property var colors: ["#4C97FF", "#FFAB19", "#40BF4A", "#FF6680", "#9966FF", "#FF8C1A"]
    title: editTarget ? "Edit block" : "Make a block"
    standardButtons: Dialog.NoButton

    function openForCreate() { editTarget = null; nameField.text = ""; inputs = []; shape = "Normal"; color = colors[0]; error = ""; open(); nameField.forceActiveFocus(); }
    function openForEdit(def) {
        editTarget = def;
        nameField.text = def.pieces.filter(p => p.kind === "Label").map(p => p.text).join(" ");
        inputs = def.pieces.filter(p => p.kind === "Input").map(p => ({ id: p.id, name: p.name, bool: p.value_type === "Bool" }));
        shape = def.shape; color = def.color; error = ""; open();
    }
    function setInput(index, change) { const next = inputs.slice(); next[index] = Object.assign({}, next[index], change); inputs = next; }
    function submit() {
        const name = nameField.text.trim();
        if (!name.length) { error = "Give the block a name"; return; }
        const label = editTarget ? editTarget.pieces.find(p => p.kind === "Label") : null;
        const pieces = [{ kind: "Label", id: label ? label.id : Blocks.uuid(), text: name }]
            .concat(inputs.map(i => ({ kind: "Input", id: i.id, name: i.name.trim(), value_type: i.bool ? "Bool" : "Any" })));
        const done = () => root.close(), failed = e => error = String(e);
        if (editTarget) app.invoke("edit_block", { blockId: editTarget.id, pieces: pieces, shape: shape, color: color }, done, failed);
        else app.invoke("create_block", { pieces: pieces, shape: shape, color: color }, done, failed);
    }

    ColumnLayout {
        width: 400; spacing: 8
        Text { visible: root.error.length > 0; text: root.error; color: Theme.danger; Layout.fillWidth: true; wrapMode: Text.WordWrap }
        BwTextField { id: nameField; Layout.fillWidth: true; placeholderText: "jump"; onAccepted: root.submit() }
        ChoiceField {
            options: [{ value: "Normal", label: "a command" }, { value: "Ending", label: "a command that ends the stack" },
                      { value: "ReturnsValue", label: "a reporter (gives a value)" }, { value: "ReturnsBool", label: "a reporter (gives yes or no)" }]
            value: root.shape; onChosen: v => root.shape = v
        }
        Row {
            spacing: 6
            Repeater {
                model: root.colors
                delegate: Rectangle {
                    required property string modelData
                    width: 26; height: 26; radius: 6; color: modelData
                    border.width: modelData === root.color ? 2 : 1; border.color: modelData === root.color ? "white" : Theme.border
                    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.color = parent.modelData }
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            SectionLabel { label: "Inputs"; Layout.fillWidth: true }
            IconButton { iconName: "plus"; tip: "Add an input"; onClicked: root.inputs = root.inputs.concat([{ id: Blocks.uuid(), name: "input" + (root.inputs.length + 1), bool: false }]) }
        }
        Repeater {
            model: root.inputs.length
            delegate: RowLayout {
                required property int index
                Layout.fillWidth: true
                BwTextField { Layout.fillWidth: true; implicitHeight: 30; text: root.inputs[index].name; placeholderText: "height"; onTextEdited: root.setInput(index, { name: text }) }
                BwButton { implicitHeight: 30; font.pixelSize: 12; primary: root.inputs[index].bool; text: root.inputs[index].bool ? "yes/no" : "value"; onClicked: root.setInput(index, { bool: !root.inputs[index].bool }) }
                IconButton { iconName: "trash-2"; tip: "Remove this input"; onClicked: { const next = root.inputs.slice(); next.splice(index, 1); root.inputs = next; } }
            }
        }
        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: "The block's body goes on the canvas, under its own hat. Drag an input's oval out of that hat to use it inside the body." }
        RowLayout {
            Layout.alignment: Qt.AlignRight; spacing: 8
            BwButton { text: "Cancel"; onClicked: root.close() }
            BwButton { text: root.editTarget ? "Save" : "Create"; primary: true; onClicked: root.submit() }
        }
    }
}
