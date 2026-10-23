import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "MakeBlock"
    visible: true
    width: 1000; height: 900
    when: windowShown
    property var calls: []
    property string refusal: ""
    QtObject {
        id: backend
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: JSON.parse(JSON.stringify(args)) }]);
            if (test.refusal.length) failed(test.refusal);
            else done(null);
        }
    }
    Component { id: factory; Editor.MakeBlockDialog { app: backend } }
    function init() { calls = []; refusal = ""; }
    function dialog() {
        const d = createTemporaryObject(factory, test);
        verify(!!d);
        return d;
    }
    function named(object, name) { return findChild(object, name); }

    function test_create_sends_icon_identifier_and_description() {
        const d = dialog();
        d.openForCreate();
        tryCompare(d, "opened", true);
        const e = d.contentItem;
        e.blockIcon = "rocket";
        e.blockIdentifier = " JUMP ";
        e.blockDescription = "Jumps.";
        e.addPiece("Input", "Bool");
        e.commitPiece();
        e.setInputDefault(1, "Bool", true);
        e.submit();
        compare(calls.length, 1);
        compare(calls[0].command, "create_block");
        const a = calls[0].args;
        compare(a.icon, "rocket");
        compare(a.identifier, "JUMP");
        compare(a.description, "Jumps.");
        compare(a.shape, "Normal");
        compare(a.pieces.length, 2);
        compare(a.pieces[1].value_type, "Bool");
        compare(a.pieces[1].default_value.value, true);
        tryCompare(d, "visible", false);
    }

    function test_edit_keeps_labels_ids_and_what_it_does_not_show() {
        const d = dialog();
        d.openForEdit({ id: "b", shape: "ReturnsBool", color: "#9966FF", icon: "star", identifier: "kept", description: "", pieces: [
            { kind: "Label", id: "l1", text: "is" },
            { kind: "Input", id: "i1", name: "x", value_type: "Any", default_value: { kind: "Text", value: "hi" } },
            { kind: "Label", id: "l2", text: "near" },
            { kind: "Branch", id: "br", name: "body" }
        ] });
        tryCompare(d, "opened", true);
        d.contentItem.submit();
        compare(calls[0].command, "edit_block");
        const a = calls[0].args;
        compare(a.blockId, "b");
        compare(a.pieces.map(p => p.kind + ":" + p.id).join(" "), "Label:l1 Input:i1 Label:l2 Branch:br");
        compare(a.pieces[2].text, "near");
        compare(a.pieces[1].default_value.value, "hi");
        compare(a.shape, "ReturnsBool");
        compare(a.color, "#9966FF");
        compare(a.icon, "star");
        compare(a.identifier, "kept");
    }

    function test_no_branch_input_offered() {
        const d = dialog();
        d.openForCreate();
        tryCompare(d, "opened", true);
        verify(!d.contentItem.allowBranches);
        const buttons = findAll(d.contentItem, o => o.modelData && o.modelData.kind === "Branch" && o.hasOwnProperty("checkable"));
        verify(buttons.length > 0);
        verify(buttons.every(b => !b.visible));
    }

    function test_refusal_shows_and_stays_open() {
        refusal = "Input names must be unique";
        const d = dialog();
        d.openForCreate();
        tryCompare(d, "opened", true);
        d.contentItem.submit();
        compare(d.contentItem.validationError, "Input names must be unique");
        verify(d.visible);
        mouseClick(named(d, "cancelButton"));
        tryCompare(d, "visible", false);
        compare(calls.length, 1);
    }

    function test_cancel_sends_nothing() {
        const d = dialog();
        d.openForEdit({ id: "c", shape: "Normal", color: "#4C97FF", pieces: [{ kind: "Label", id: "l", text: "c" }] });
        tryCompare(d, "opened", true);
        d.contentItem.addPiece("Label");
        mouseClick(named(d, "cancelButton"));
        tryCompare(d, "visible", false);
        compare(calls.length, 0);
    }

    function findAll(object, pred) {
        let out = pred(object) ? [object] : [];
        for (const child of object.children || []) out = out.concat(findAll(child, pred));
        return out;
    }
}
