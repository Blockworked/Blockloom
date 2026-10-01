import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml" as Editor
import "SettingsFixture.js" as Fixture

TestCase {
    id: test
    name: "Settings"
    visible: true
    when: windowShown
    width: 1000; height: 900
    property var calls: []
    property var panel: null
    readonly property var lightingEntry: ({ name: "Sunset.blocklighting", path: "assets/Sunset.blocklighting", kind: "lighting", size: 100, modified: 0, protected: false })
    function projectState() {
        const world = Fixture.world(); world.mode = "ThreeD";
        return { project_path: "/tmp/Demo", selected_actor: null, project: {
            world: world, actors: [], icon: "", active_scene: "one", default_scene: "one",
            scenes: [{ id: "one", name: "Scene 1", path: "assets/Scene 1.blockscene", world: world },
                     { id: "two", name: "Scene 2", path: "assets/Scene 2.blockscene", world: Fixture.world() }]
        } };
    }
    QtObject {
        id: backend
        property var appState: test.projectState()
        property var openActor: null
        property var status: null
        property var assetTargets: []
        property var assetDrag: null
        property string inspectedLighting: ""
        property bool inspectScene: false
        property string inspectedScene: ""
        function selectScene(id) { inspectedLighting = ""; inspectedScene = id; inspectScene = true; }
        function selectLighting(path) { inspectedScene = ""; inspectScene = false; inspectedLighting = path; }
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (command === "read_lighting_asset" && done) done(Fixture.world().lighting);
            else if (command === "create_asset" && done) done("assets/Scene Lighting.blocklighting");
            else if (command === "list_assets" && done) done([test.lightingEntry,
                { name: "Scene 2.blockscene", path: "assets/Scene 2.blockscene", kind: "scene", size: 100, modified: 0, protected: false }]);
            else if (command === "pipeline_status" && done) done([]);
            else if (done) done({});
        }
        function assetUrl(path) { return ""; }
        function fromFileUrl(path) { return String(path); }
    }
    Component { id: assetFieldFactory; Editor.AssetField { app: backend } }
    Component { id: factory; Editor.SettingsFields { app: backend } }
    Component { id: inspectorFactory; Editor.InspectorPanel { app: backend } }
    Component { id: dialogFactory; Editor.ProjectSettingsDialog { app: backend } }
    Component {
        id: editorFactory
        Item {
            width: 1000; height: 900
            Editor.ActorList { objectName: "hierarchy"; app: backend; width: 220; height: 690 }
            Editor.InspectorPanel { objectName: "inspector"; app: backend; x: 640; width: 360; height: 690 }
            Editor.AssetTray { objectName: "tray"; app: backend; y: 690; width: 1000; height: 200 }
        }
    }
    function init() {
        calls = []; backend.appState = projectState(); backend.openActor = null;
        backend.inspectedLighting = ""; backend.inspectedScene = ""; backend.inspectScene = false;
        panel = createTemporaryObject(factory, test, { width: 360, height: 720 });
        verify(panel !== null);
    }
    function test_assetDropTypeIcons() {
        const field = createTemporaryObject(assetFieldFactory, test, { width: 220, accept: ["lighting"] });
        const icon = findChild(field, "asset-type-icon");
        compare(icon.name, "sun");
        field.accept = ["image"]; compare(icon.name, "image");
        field.accept = ["audio"]; compare(icon.name, "music");
        field.accept = ["model"]; compare(icon.name, "box");
        field.accept = ["script"]; compare(icon.name, "file-code");
        field.accept = ["image", "volume"];
        backend.assetDrag = { kind: "volume" }; compare(icon.name, "layers");
        backend.assetDrag = null; compare(icon.name, "image");
        verify(field.leftPadding >= icon.width + icon.anchors.leftMargin);
    }
    function test_leftNavigationInOneWindow() {
        const dialog = createTemporaryObject(dialogFactory, test);
        verify(dialog !== null); dialog.open();
        tryCompare(dialog, "visible", true);
        mouseClick(findChild(dialog, "settings-app-info"));
        compare(dialog.page, "publishing");
        wait(50);
        grabImage(test.Window.window.contentItem).save("/tmp/blockloom-settings-navigation.png");
        verify(findChild(dialog, "settings-App Info").visible);
        verify(!findChild(dialog, "settings-Project").visible);
        mouseClick(findChild(dialog, "settings-android"));
        compare(dialog.page, "android");
        verify(findChild(dialog, "settings-Android").visible);
        mouseClick(findChild(dialog, "settings-general"));
        compare(dialog.page, "project");
        dialog.close();
    }
    function test_sceneUsesOpenComponentCards() {
        const physics = findChild(panel, "settings-Physics");
        verify(physics.visible);
        compare(physics.expanded, undefined);
        verify(findChild(panel, "scene-lighting-field").visible);
        verify(!panel.showSection("Sun and ambient"));
        panel.invoke("set_fixed_rate", { fixedRate: 120 });
        compare(calls[calls.length - 1].command, "set_fixed_rate");
        panel.sceneId = "two";
        panel.invoke("set_fixed_rate", { fixedRate: 90 });
        const call = calls[calls.length - 1];
        compare(call.command, "set_scene_component");
        compare(call.args.sceneId, "two");
        compare(call.args.component.component, "Physics");
        compare(call.args.component.fixed_rate, 90);
        compare(backend.appState.project.active_scene, "one");
    }
    function test_inspectorFollowsSelection() {
        const inspector = createTemporaryObject(inspectorFactory, test, { width: 360, height: 720 });
        verify(inspector !== null);
        backend.selectScene("two");
        verify(inspector.settingsVisible);
        backend.selectLighting("assets/Sun.blocklighting");
        verify(inspector.settingsVisible);
        backend.appState = Object.assign({}, backend.appState, { selected_actor: "actor" });
        compare(backend.inspectedLighting, "");
        compare(inspector.settingsVisible, false);
    }
    function test_dragLightingOntoSceneInspector() {
        panel.visible = false;
        const editor = createTemporaryObject(editorFactory, test);
        verify(editor !== null);
        const inspector = findChild(editor, "inspector");
        backend.selectScene("one");
        const tile = findChild(editor, "asset-assets/Sunset.blocklighting");
        const field = findChild(inspector, "scene-lighting-field");
        tryVerify(() => tile.visible && field.visible && tile.width > 0 && field.width > 0);
        wait(50);
        grabImage(editor).save("/tmp/blockloom-scene-inspector.png");
        const target = field.mapToItem(tile, field.width / 2, field.height / 2);
        mousePress(tile, tile.width / 2, tile.height / 2);
        compare(backend.inspectedScene, "one");
        mouseMove(tile, tile.width / 2 + 20, tile.height / 2, 20);
        compare(backend.inspectedLighting, "");
        verify(inspector.settingsVisible);
        verify(backend.assetDrag !== null);
        mouseMove(tile, target.x, target.y, 20);
        mouseRelease(tile, target.x, target.y);
        const assigned = calls.find(c => c.command === "set_scene_lighting_asset");
        verify(!!assigned, JSON.stringify({ calls: calls, target: target, field: field.mapToItem(null, 0, 0), tile: tile.mapToItem(null, 0, 0) }));
        compare(assigned.args.path, lightingEntry.path);
        compare(assigned.args.sceneId, "one");
        compare(backend.inspectedScene, "one");
        compare(backend.inspectedLighting, "");
    }
    function test_dropOnHierarchySceneAndSelectAssets() {
        panel.visible = false;
        const editor = createTemporaryObject(editorFactory, test);
        verify(editor !== null);
        const header = findChild(editor, "hierarchy-scene");
        const point = header.mapToItem(null, header.width / 2, header.height / 2);
        verify(header.takeDrop(lightingEntry, point.x, point.y));
        compare(backend.inspectedScene, "one");
        const tray = findChild(editor, "tray");
        tray.selectEntry(lightingEntry);
        compare(backend.inspectedLighting, lightingEntry.path);
        tray.selectEntry({ kind: "scene", path: "assets/Scene 2.blockscene" });
        compare(backend.inspectedScene, "two");
        compare(backend.appState.project.active_scene, "one");
    }
    function test_lightingAssetEdits() {
        panel.page = "lighting"; panel.lightingPath = lightingEntry.path;
        verify(panel.showSection("Sun and ambient"));
        verify(panel.showSection("Shadows"));
        verify(!panel.showSection("Physics"));
        panel.writeLighting({ illuminance: 456 });
        panel.writeLighting({ ambient_brightness: 12 });
        const writes = calls.filter(c => c.command === "write_lighting_asset");
        compare(writes[1].args.lighting.illuminance, 456);
        compare(writes[1].args.lighting.ambient_brightness, 12);
    }
    function test_convertInlineLighting() {
        panel.createLighting();
        compare(calls[0].command, "create_asset");
        compare(calls[1].command, "write_lighting_asset");
        compare(calls[2].command, "set_scene_lighting_asset");
        compare(backend.inspectedLighting, "assets/Scene Lighting.blocklighting");
    }
}
