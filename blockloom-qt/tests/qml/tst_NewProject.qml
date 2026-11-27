import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "NewProject"
    visible: true
    width: 800; height: 700
    when: windowShown
    property var dialog: null
    property var calls: []
    QtObject {
        id: backend
        property var appState: ({ default_project_location: "/tmp/projects" })
        function toFileUrl(path) { return "file://" + path; }
        function fromFileUrl(path) { return String(path).replace("file://", ""); }
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (done) done({});
        }
    }
    Component { id: factory; Editor.NewProjectDialog { app: backend } }
    function init() {
        calls = [];
        dialog = createTemporaryObject(factory, test);
        dialog.open();
        tryCompare(dialog, "opened", true);
    }
    function cleanup() { dialog.destroy(); dialog = null; }
    // Repeater delegates aren't reached by findChild, so look through the row.
    function button(name) {
        const row = findChild(dialog, "samples");
        for (let i = 0; i < row.children.length; i++)
            if (row.children[i].objectName === "sample-" + name) return row.children[i];
        return null;
    }
    function test_templatesFollowTheDimension() {
        verify(button("platformer").visible);
        compare(button("first-person"), null);
        button("platformer").clicked();
        compare(dialog.sample, "platformer");
        dialog.mode = "ThreeD";
        compare(dialog.sample, "", "a 2D-only template is dropped in 3D");
        button("first-person").clicked();
        compare(dialog.sample, "first-person");
        dialog.mode = "TwoD";
        compare(dialog.sample, "");
        button("ui-puzzle").clicked();
        dialog.mode = "ThreeD";
        compare(dialog.sample, "ui-puzzle", "the puzzle fits both");
    }
    function test_createPassesTheTemplate() {
        button("top-down").clicked();
        dialog.submit();
        compare(calls.length, 1);
        compare(calls[0].command, "create_project");
        compare(calls[0].args.sample, "top-down");
        compare(calls[0].args.mode, "TwoD");
    }
}
