import QtQuick
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "InterfaceViewport"
    visible: true
    width: 1200; height: 800
    when: windowShown
    property var panel: null
    property var calls: []
    QtObject {
        id: backend
        property var appState: ({running: false, runtime_embedded: true, project: {world: {interface: {
            widgets: [{element: {id: "back", kind: "Panel"}}, {element: {id: "front", kind: "Label"}}]
        }}}})
        property string previewFrame: ""
        property string previewLayout: ""
        function invoke(command, args, done, failed) {
            test.calls.push({command: command, args: args});
            if (done) done();
        }
    }
    Component { id: workspace; Editor.UiDesigner { app: backend } }
    function init() {
        calls = [];
        panel = workspace.createObject(test, {width: test.width, height: test.height});
        verify(panel !== null);
        tryVerify(() => calls.some(c => c.command === "preview_interface" && !!c.args.design));
    }
    function cleanup() { panel.destroy(); panel = null; }
    function geometry(revision, generation) {
        return {revision: revision, generation: generation, viewport: [960,720], widgets: [
            {id: "front", size: [100,60], transform: [0,1,-1,0,300,300], visible: true, paint_order: 2, clips: []},
            {id: "back", size: [200,200], transform: [1,0,0,1,300,300], visible: true, paint_order: 1, clips: []}
        ]};
    }
    function test_pick_matches_runtime_order_and_never_saves() {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        verify(panel.layoutReady);
        const canvas = findChild(panel, "interfaceCanvas");
        mouseClick(canvas, 300, 300);
        compare(panel.selectedId, "front");
        verify(findChild(panel, "interfaceSelection").visible);
        mouseClick(canvas, 390, 390);
        compare(panel.selectedId, "back");
        mouseClick(canvas, 600, 500);
        compare(panel.selectedId, "");
        verify(!calls.some(c => c.command === "set_interface" || c.command === "preview_input"));
    }
    function test_stale_geometry_and_resolution_change_disable_picking() {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision-1, panel.generation)));
        verify(!panel.layoutReady);
        verify(!findChild(panel, "interfacePicking").enabled);
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        verify(panel.layoutReady);
        panel.previewWidth = 1280;
        verify(!panel.layoutReady);
        verify(!findChild(panel, "interfaceSelection").visible);
    }
    function test_tab_exit_cancels_preview() {
        panel.visible = false;
        verify(calls.some(c => c.command === "preview_interface" && !c.args.design));
    }
    function test_screen_isolation_invalidates_geometry_without_saving() {
        panel.selectedId = "front";
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        verify(panel.layoutReady);
        const oldRevision = panel.revision;
        panel.screenId = "back";
        panel.receiveLayout(JSON.stringify(geometry(oldRevision, panel.generation)));
        verify(!panel.layoutReady);
        compare(panel.selectedId, "");
        compare(panel.screenWidgets.length, 1);
        const revision = panel.revision;
        tryVerify(() => panel.revision > revision);
        compare(calls[calls.length - 1].args.design.screen, "back");
        compare(calls[calls.length - 1].args.design.document.widgets.length, 2);
        verify(!calls.some(c => c.command === "set_interface" || c.command === "preview_input"));
        panel.document = {widgets: [{element: {id: "front", kind: "Label"}}]};
        compare(panel.screenId, "");
    }
    function test_screen_membership_follows_nested_parents_and_ids() {
        panel.document = {widgets: [
            {element: {id: "child", parent: "parent"}},
            {element: {id: "front"}},
            {element: {id: "parent", parent: "back"}},
            {element: {id: "back"}}
        ]};
        panel.selectedId = "child";
        panel.screenId = "back";
        compare(panel.selectedId, "child");
        compare(panel.screenWidgets.map(w => w.element.id).join(","), "child,parent,back");
        panel.screenId = "front";
        compare(panel.selectedId, "");
        compare(panel.screenWidgets.length, 1);
        panel.screenId = "";
        compare(panel.screenWidgets.length, 4);
    }
}
