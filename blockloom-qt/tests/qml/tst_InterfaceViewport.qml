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
    property var draft: null
    property bool deferBegin: false
    property bool deferUpdate: false
    property var pendingBegin: null
    property var pendingUpdate: null
    QtObject {
        id: backend
        property var appState: ({running: false, runtime_embedded: true, project: {world: {interface: {
            widgets: [{element: {id: "back", kind: "Panel"}}, {element: {id: "front", kind: "Label"}}]
        }}}})
        property string previewFrame: ""
        property string previewLayout: ""
        function invoke(command, args, done, failed) {
            test.calls.push({command: command, args: args});
            if (command === "begin_interface_edit") {
                test.draft = JSON.parse(JSON.stringify(appState.project.world.interface));
                if (test.deferBegin) test.pendingBegin = done;
                else if (done) done("draft-token");
            } else if (command === "update_interface_edit") {
                const next = JSON.parse(JSON.stringify(test.draft));
                const w = next.widgets.find(w => w.element.id === args.edit.id);
                if (args.edit.kind === "Move" || args.edit.kind === "Resize") w.element.offset = args.edit.offset;
                if (args.edit.kind === "Resize") w.element.size = args.edit.size;
                if (args.edit.kind === "SetProperty") {
                    const property = args.edit.property;
                    if (property.path === "layout") w.layout = property.value;
                    else w.element[property.path.split(".")[1]] = property.value;
                }
                if (args.edit.kind === "Reparent") {
                    w.element.parent = args.edit.parent;
                    if (args.edit.placement.mode === "Free") {
                        w.element.offset = args.edit.placement.offset;
                        w.element.size = args.edit.placement.size;
                    } else w.element.offset = [0,0];
                }
                if (test.deferUpdate) test.pendingUpdate = () => done(next);
                else if (done) done(next);
            } else if (command === "commit_interface_edit") {
                appState = {running: false, runtime_embedded: true, sync: {revision: 2}, project: {world: {interface: panel.copy(panel.document)}}};
                if (done) done();
            } else if (done) done();
        }
    }
    Component { id: workspace; Editor.UiDesigner { app: backend } }
    function init() {
        calls = []; deferBegin = false; deferUpdate = false; pendingBegin = null; pendingUpdate = null;
        backend.appState = {running: false, runtime_embedded: true, sync: {revision: 1}, project: {world: {interface: {
            widgets: [{element: {id: "back", kind: "Panel", offset: [200,200], size: [200,200], anchor: "TopLeft"}},
                {element: {id: "front", kind: "Label", offset: [250,270], size: [100,60], anchor: "Center"}}]
        }}}};
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
    function test_typed_properties_and_layout_allow_flow_children() {
        const d = panel.copy(panel.document);
        d.widgets[1].element.parent = "back";
        panel.document = d;
        backend.appState.project.world.interface = d;
        panel.selectedId = "front";
        verify(!panel.editable(panel.widget));
        panel.propertyEdit("layout", {width: {Percent: 50}, gap: 12});
        compare(panel.document.widgets[1].layout.width.Percent, 50);
        compare(calls.filter(c => c.command === "commit_interface_edit").length, 1);
        panel.change("content", "Hello");
        compare(panel.document.widgets[1].element.content, "Hello");
        verify(!calls.some(c => c.command === "set_interface"));
    }
    function test_reparent_preserves_canvas_bounds_and_flow_is_explicit() {
        const d = panel.copy(panel.document);
        d.widgets[0].element.kind = "Canvas";
        panel.document = d; backend.appState.project.world.interface = d;
        panel.selectedId = "front";
        const g = geometry(panel.revision, panel.generation);
        g.widgets[0].transform = [1,0,0,1,300,300];
        panel.receiveLayout(JSON.stringify(g));
        panel.reparent("back");
        const edit = calls.find(c => c.command === "update_interface_edit").args.edit;
        compare(edit.kind, "Reparent"); compare(edit.placement.mode, "Free");
        compare(edit.placement.offset, [50,70]); compare(edit.placement.size, [100,60]);
        compare(panel.document.widgets[1].element.parent, "back");
        verify(panel.descendant("front", "back"));
        verify(!panel.descendant("back", "front"));
        panel.reparent("");
        verify(panel.error.indexOf("geometry") >= 0);
        const flow = panel.copy(panel.document); flow.widgets[0].element.kind = "VerticalBox";
        flow.widgets[1].element.parent = "";
        panel.document = flow; backend.appState.project.world.interface = flow;
        panel.reparent("back");
        compare(calls.filter(c => c.command === "update_interface_edit").slice(-1)[0].args.edit.placement.mode, "Flow");
    }
    function test_reparent_uses_canvas_scale_and_safe_area() {
        const d = panel.copy(panel.document);
        d.scale = "ScaleWithSize"; d.reference_size = [480,360]; d.safe_area = [10,20,10,20];
        d.widgets[0].element.kind = "Canvas";
        panel.document = d; backend.appState.project.world.interface = d;
        panel.selectedId = "front";
        const scale = panel.designScale;
        function frame() {
            const g = geometry(panel.revision, panel.generation);
            g.widgets[0].transform = [scale,0,0,scale,10+150*scale,20+140*scale];
            g.widgets[1].transform = [scale,0,0,scale,10+200*scale,20+150*scale];
            panel.receiveLayout(JSON.stringify(g));
        }
        frame(); panel.reparent("back");
        const free = calls.filter(c => c.command === "update_interface_edit").slice(-1)[0].args.edit.placement;
        fuzzyCompare(free.offset[0], 0, 0.001); fuzzyCompare(free.offset[1], 60, 0.001);
        compare(free.size, [100,60]);
        frame(); panel.reparent("");
        const root = calls.filter(c => c.command === "update_interface_edit").slice(-1)[0].args.edit.placement;
        fuzzyCompare(root.offset[0], 100, 0.001); fuzzyCompare(root.offset[1], 110, 0.001);
        compare(root.size, [100,60]);
    }
    function test_reparent_rejects_unrepresentable_transform() {
        panel.selectedId = "front";
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        panel.reparent(""); // Already a root, so no edit.
        const d = panel.copy(panel.document); d.widgets[0].element.kind = "Canvas";
        panel.document = d;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        panel.reparent("back");
        verify(panel.error.indexOf("transform") >= 0);
        verify(!calls.some(c => c.command === "begin_interface_edit"));
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
    function test_move_previews_and_commits_once_on_release() {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        const canvas = findChild(panel, "interfaceCanvas");
        mousePress(canvas,390,390);
        mouseMove(canvas,410,420);
        verify(panel.gesture !== null);
        fuzzyCompare(panel.document.widgets[0].element.offset[0],220,1.5);
        fuzzyCompare(panel.document.widgets[0].element.offset[1],230,1.5);
        verify(!calls.some(c => c.command === "commit_interface_edit" || c.command === "set_interface"));
        verify(findChild(panel,"interfacePicking").enabled);
        mouseMove(canvas,430,440);
        mouseRelease(canvas,430,440);
        compare(panel.gesture, null);
        compare(calls.filter(c => c.command === "begin_interface_edit").length, 1);
        compare(calls.filter(c => c.command === "commit_interface_edit").length, 1);
        fuzzyCompare(panel.document.widgets[0].element.offset[0],240,1.5);
        fuzzyCompare(panel.document.widgets[0].element.offset[1],250,1.5);
    }
    function test_resize_uses_runtime_transform_and_escape_discards() {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        panel.selectedId = "front";
        verify(panel.startEdit("Resize",300,300));
        panel.dragEdit(300,320);
        compare(panel.document.widgets[1].element.size.join(","), "120,60");
        compare(panel.document.widgets[1].element.offset.join(","), "250,280");
        keyClick(Qt.Key_Escape);
        compare(panel.gesture,null);
        compare(panel.document.widgets[1].element.size.join(","), "100,60");
        verify(calls.some(c => c.command === "cancel_interface_edit"));
        verify(!calls.some(c => c.command === "commit_interface_edit" || c.command === "set_interface"));
    }
    function test_resize_handle_retains_grab_during_preview() {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        panel.selectedId = "back";
        const handle = findChild(panel,"interfaceResizeHandle");
        verify(handle.visible);
        mousePress(handle,handle.width/2,handle.height/2);
        mouseMove(handle,handle.width/2+30,handle.height/2+20);
        verify(panel.gesture !== null);
        verify(handle.visible);
        fuzzyCompare(panel.document.widgets[0].element.size[0],230,1.5);
        fuzzyCompare(panel.document.widgets[0].element.size[1],220,1.5);
        mouseRelease(handle,handle.width/2+30,handle.height/2+20);
        compare(calls.filter(c => c.command === "commit_interface_edit").length,1);
    }
    function test_all_resize_handles_data() {
        return [
            {tag:"TopLeft", direction:{x:-1,y:-1}, size:[190,180], offset:[210,220]},
            {tag:"Top", direction:{x:0,y:-1}, size:[200,180], offset:[200,220]},
            {tag:"TopRight", direction:{x:1,y:-1}, size:[210,180], offset:[200,220]},
            {tag:"Right", direction:{x:1,y:0}, size:[210,200], offset:[200,200]},
            {tag:"BottomRight", direction:{x:1,y:1}, size:[210,220], offset:[200,200]},
            {tag:"Bottom", direction:{x:0,y:1}, size:[200,220], offset:[200,200]},
            {tag:"BottomLeft", direction:{x:-1,y:1}, size:[190,220], offset:[210,200]},
            {tag:"Left", direction:{x:-1,y:0}, size:[190,200], offset:[210,200]}
        ];
    }
    function test_all_resize_handles(data) {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        const handle = findChild(panel,data.tag === "BottomRight" ? "interfaceResizeHandle" : "interfaceResize"+data.tag);
        verify(handle.visible);
        mousePress(handle,handle.width/2,handle.height/2);
        mouseMove(handle,handle.width/2+10,handle.height/2+20);
        verify(panel.gesture !== null);
        verify(handle.visible);
        for (let i=0;i<2;++i) {
            fuzzyCompare(panel.document.widgets[0].element.size[i],data.size[i],1.5);
            fuzzyCompare(panel.document.widgets[0].element.offset[i],data.offset[i],1.5);
        }
        mouseRelease(handle,handle.width/2+10,handle.height/2+20);
        compare(calls.filter(c => c.command === "commit_interface_edit").length,1);
        verify(!calls.some(c => c.command === "set_interface"));
    }
    function test_left_resize_anchor_and_minimum_size() {
        const d = panel.copy(panel.document);
        d.widgets[0].element.anchor = "BottomRight";
        panel.document = d; backend.appState.project.world.interface = d;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Resize",200,300,{x:-1,y:0}));
        panel.dragEdit(210,330);
        compare(panel.document.widgets[0].element.size,[190,200]);
        compare(panel.document.widgets[0].element.offset,[200,200]);
        panel.dragEdit(700,300);
        compare(panel.document.widgets[0].element.size,[1,200]);
        compare(panel.document.widgets[0].element.offset,[200,200]);
        panel.cancelEdit();
    }
    function test_resize_in_transformed_parent_keeps_opposite_corner() {
        const d = panel.copy(panel.document);
        d.widgets[0].element.kind = "Canvas";
        d.widgets[1].element.parent = "back";
        d.widgets[1].layout = {absolute:true};
        panel.document = d; backend.appState.project.world.interface = d;
        const g = geometry(panel.revision,panel.generation);
        g.widgets[0].transform = [0,2,-2,0,300,300];
        g.widgets[1].transform = [0,2,-2,0,300,300];
        panel.receiveLayout(JSON.stringify(g)); panel.selectedId = "front";
        compare(panel.resizeCursor({x:0,y:-1}),Qt.SizeHorCursor);
        verify(panel.startEdit("Resize",300,300,{x:-1,y:-1}));
        panel.dragEdit(260,320);
        compare(panel.document.widgets[1].element.size,[90,40]);
        compare(panel.document.widgets[1].element.offset,[260,290]);
        panel.cancelEdit();
    }
    function test_grid_move_resize_and_shift_bypass() {
        panel.snapGrid = true; panel.snapStep = 8;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",300,300));
        panel.dragEdit(311,314);
        compare(panel.document.widgets[0].element.offset,[208,216]);
        panel.dragEdit(311,314,Qt.ShiftModifier);
        compare(panel.document.widgets[0].element.offset,[211,214]);
        panel.cancelEdit();
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        verify(panel.startEdit("Resize",200,200,{x:-1,y:-1}));
        panel.dragEdit(211,214);
        compare(panel.document.widgets[0].element.size,[192,184]);
        compare(panel.document.widgets[0].element.offset,[208,216]);
        panel.finishEdit();
        compare(calls.filter(c => c.command === "commit_interface_edit").length,1);
        compare(panel.snapLines.length,0);
    }
    function test_alignment_uses_frozen_siblings_and_guides() {
        panel.snapAlign = true;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",300,300));
        panel.dragEdit(369,300);
        compare(panel.document.widgets[0].element.offset,[270,200]);
        verify(panel.snapLines.some(line => line.a.x === 270 && line.b.x === 270));
        verify(findChild(panel,"interfaceSelection").visible);
        verify(!panel.layoutReady);
        panel.dragEdit(369,300,Qt.ShiftModifier);
        compare(panel.document.widgets[0].element.offset,[269,200]);
        compare(panel.snapLines.length,0);
        panel.cancelEdit();
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        verify(panel.startEdit("Resize",200,300,{x:-1,y:0}));
        panel.dragEdit(269,300);
        compare(panel.document.widgets[0].element.size,[130,200]);
        compare(panel.document.widgets[0].element.offset,[270,200]);
        verify(panel.snapLines.length > 0);
        panel.cancelEdit(); compare(panel.snapLines.length,0);
        compare(calls.filter(c => c.command === "commit_interface_edit").length,0);
    }
    function test_snapping_in_parent_coordinates_and_screen_tolerance() {
        const d = panel.copy(panel.document);
        d.widgets[0].element.kind = "Canvas";
        d.widgets[1].element.parent = "back";
        d.widgets[1].layout = {absolute:true};
        panel.document = d; backend.appState.project.world.interface = d;
        const g = geometry(panel.revision,panel.generation);
        g.widgets[0].transform = [0,2,-2,0,300,300];
        g.widgets[1].transform = [0,2,-2,0,300,300];
        panel.snapGrid = true; panel.snapStep = 8;
        panel.receiveLayout(JSON.stringify(g)); panel.selectedId = "front";
        verify(panel.startEdit("Move",300,300)); panel.dragEdit(280,322);
        compare(panel.document.widgets[1].element.offset,[264,280]); panel.cancelEdit();
        d.widgets[1].element.parent = "";
        panel.document = d; backend.appState.project.world.interface = d;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back"; panel.snapGrid = false; panel.snapAlign = true;
        panel.zoom = 2;
        verify(panel.startEdit("Move",300,300)); panel.dragEdit(366,300);
        compare(panel.document.widgets[0].element.offset[0],266); // Four logical pixels exceed six screen pixels.
        panel.dragEdit(368,300); compare(panel.document.widgets[0].element.offset[0],270);
        panel.cancelEdit();
    }
    function test_resize_grid_preserves_inactive_dimension_and_clamped_edge() {
        panel.snapGrid = true; panel.snapStep = 8;
        const d = panel.copy(panel.document); d.widgets[0].element.size = [203,201];
        panel.document = d; backend.appState.project.world.interface = d;
        const g = geometry(panel.revision,panel.generation); g.widgets[1].size = [203,201];
        panel.receiveLayout(JSON.stringify(g)); panel.selectedId = "back";
        verify(panel.startEdit("Resize",300,200,{x:0,y:-1})); panel.dragEdit(330,208);
        compare(panel.document.widgets[0].element.size,[203,192]);
        compare(panel.document.widgets[0].element.offset,[200,209]);
        panel.dragEdit(300,1000);
        compare(panel.document.widgets[0].element.size,[203,1]);
        compare(panel.document.widgets[0].element.offset,[200,400]);
        panel.cancelEdit();
    }
    function test_resize_limits_preserve_opposite_corner_with_grid() {
        const d = panel.copy(panel.document); d.widgets[0].layout = {min_size:[80,60],max_size:[240,230]};
        panel.document = d; backend.appState.project.world.interface = d;
        panel.snapGrid = true; panel.snapStep = 8;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation))); panel.selectedId = "back";
        verify(panel.startEdit("Resize",200,200,{x:-1,y:-1}));
        panel.dragEdit(700,700);
        compare(panel.document.widgets[0].element.size,[80,60]);
        compare(panel.document.widgets[0].element.offset,[320,340]);
        panel.dragEdit(-300,-300);
        compare(panel.document.widgets[0].element.size,[240,230]);
        compare(panel.document.widgets[0].element.offset,[160,170]);
        panel.cancelEdit();
    }
    function test_alignment_excludes_hidden_and_other_parent_widgets() {
        panel.snapAlign = true;
        const d = panel.copy(panel.document); d.widgets[1].element.parent = "missing";
        panel.document = d;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",300,300)); panel.dragEdit(369,300);
        compare(panel.document.widgets[0].element.offset,[269,200]); panel.cancelEdit();
        const g = geometry(panel.revision,panel.generation); g.widgets[0].visible = false;
        panel.receiveLayout(JSON.stringify(g));
        verify(panel.startEdit("Move",300,300)); panel.dragEdit(369,300);
        compare(panel.document.widgets[0].element.offset,[269,200]); panel.cancelEdit();
    }
    function test_inspector_uses_typed_edit_and_flow_widgets_are_disabled() {
        panel.selectedId = "back";
        panel.editDimension(0,123);
        compare(panel.document.widgets[0].element.offset[0],123);
        verify(calls.some(c => c.command === "update_interface_edit" && c.args.edit.kind === "Move"));
        verify(!calls.some(c => c.command === "set_interface"));
        const next = panel.copy(panel.document);
        next.widgets[1].element.parent = "back";
        panel.document = next;
        panel.selectedId = "front";
        verify(!panel.editable(panel.widget));
        verify(!panel.startEdit("Move",null,null));
    }
    function test_screen_and_viewport_changes_cancel_gestures() {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision, panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",390,390));
        panel.dragEdit(410,410);
        panel.previewWidth = 1280;
        compare(panel.gesture,null);
        compare(panel.document.widgets[0].element.offset.join(","),"200,200");
        verify(calls.some(c => c.command === "cancel_interface_edit"));
    }

    function test_release_before_begin_reply_flushes_final_edit() {
        deferBegin = true;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",300,300));
        panel.dragEdit(320,330);
        panel.dragEdit(340,350);
        panel.finishEdit();
        verify(!calls.some(c => c.command === "update_interface_edit"));
        pendingBegin("draft-token");
        compare(panel.gesture,null);
        compare(panel.document.widgets[0].element.offset.join(","),"240,250");
        compare(calls.filter(c => c.command === "update_interface_edit").length,1);
        compare(calls.filter(c => c.command === "commit_interface_edit").length,1);
    }
    function test_inflight_update_coalesces_and_cancel_ignores_reply() {
        deferUpdate = true;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",300,300));
        panel.dragEdit(310,310);
        panel.dragEdit(330,330);
        panel.dragEdit(350,350);
        compare(calls.filter(c => c.command === "update_interface_edit").length,1);
        panel.finishEdit();
        deferUpdate = false;
        pendingUpdate();
        compare(panel.document.widgets[0].element.offset.join(","),"250,250");
        compare(calls.filter(c => c.command === "update_interface_edit").length,2);
        compare(calls.filter(c => c.command === "commit_interface_edit").length,1);
        deferBegin = true;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        verify(panel.startEdit("Move",300,300));
        panel.dragEdit(310,310);
        panel.cancelEdit();
        pendingBegin("late-token");
        compare(panel.gesture,null);
        verify(calls.some(c => c.command === "cancel_interface_edit" && c.args.token === "late-token"));
        compare(calls.filter(c => c.command === "commit_interface_edit").length,1);
    }

    function test_moves_use_canvas_scale_and_parent_transform() {
        const next = panel.copy(panel.document);
        next.scale = "ScaleWithSize"; next.reference_size = [480,360];
        next.widgets[0].element.kind = "Canvas";
        panel.document = next;
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",300,300));
        panel.dragEdit(320,340);
        compare(panel.document.widgets[0].element.offset.join(","),"210,220");
        panel.cancelEdit();
        const nested = panel.copy(panel.document);
        nested.widgets[0].element.kind = "Canvas";
        nested.widgets[1].element.parent = "back";
        panel.document = nested;
        const frame = geometry(panel.revision,panel.generation);
        frame.widgets[1].transform = [0,2,-2,0,300,300];
        panel.receiveLayout(JSON.stringify(frame));
        panel.selectedId = "front";
        verify(panel.startEdit("Move",300,300));
        panel.dragEdit(320,340);
        compare(panel.document.widgets[1].element.offset.join(","),"270,260");
        panel.cancelEdit();
    }
    function test_continuous_drag_sends_previews_and_cancel_ignores_update_reply() {
        panel.receiveLayout(JSON.stringify(geometry(panel.revision,panel.generation)));
        panel.selectedId = "back";
        verify(panel.startEdit("Move",300,300));
        const before = calls.filter(c => c.command === "preview_interface").length;
        for (let i=0;i<6;++i) { panel.dragEdit(310+i,310+i); wait(30); }
        verify(calls.filter(c => c.command === "preview_interface").length > before);
        deferUpdate = true;
        panel.dragEdit(350,350);
        panel.cancelEdit();
        pendingUpdate();
        compare(panel.document.widgets[0].element.offset.join(","),"200,200");
        verify(!calls.some(c => c.command === "commit_interface_edit"));
    }

}
