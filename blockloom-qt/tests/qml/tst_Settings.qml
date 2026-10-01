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
    width: 640; height: 800
    property var calls: []
    property var panel: null
    QtObject {
        id: backend
        property var appState: ({ project: { world: Fixture.world(), scenes: [{ id: "one", name: "Scene 1" }], active_scene: "one", icon: "", actors: [] } })
        property var openActor: null
        property var status: null
        property var assetTargets: []
        property var assetDrag: null
        property string inspectedLighting: ""
        property bool inspectScene: false
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (command === "read_lighting_asset" && done) done(Fixture.world().lighting);
            else if (command === "create_asset" && done) done("assets/Scene Lighting.blocklighting");
            else if (done) done({});
        }
        function assetUrl(path) { return ""; }
        function fromFileUrl(path) { return String(path); }
    }
    Component { id: factory; Editor.SettingsFields { app: backend } }
    Component { id: inspectorFactory; Editor.InspectorPanel { app: backend } }
    function init() {
        calls = [];
        backend.inspectedLighting = "";
        panel = createTemporaryObject(factory, test, { width: 290, height: 720 });
        verify(panel !== null);
    }
    function test_pagesAndSceneComponents() {
        verify(panel.showSection("Physics"));
        verify(!panel.showSection("Lighting"));
        verify(!panel.showSection("App Info"));
        panel.page = "publishing";
        verify(panel.showSection("App Info"));
        verify(!panel.showSection("Physics"));
        const appInfo = findChild(panel, "settings-App Info");
        verify(appInfo.visible);
        verify(appInfo.expanded);
        panel.page = "scene";
        const physics = findChild(panel, "settings-Physics");
        verify(physics.visible);
        compare(physics.expanded, false);
        physics.expanded = true;
        panel.invoke("set_fixed_rate", { fixedRate: 120 });
        compare(calls[calls.length - 1].command, "set_fixed_rate");
        compare(panel.componentFor("Performance and scaling"), "Quality");
    }
    function test_sidebarKeepsSceneSelectionWhenActorClears() {
        backend.appState = { project_path: "/tmp/Demo", selected_actor: "actor", project: {
            world: Fixture.world(), scenes: [{ id: "one", name: "Scene 1" }], active_scene: "one", icon: "", actors: []
        } };
        const inspector = createTemporaryObject(inspectorFactory, test, { width: 360, height: 720 });
        verify(inspector !== null);
        backend.inspectScene = true;
        verify(inspector.settingsVisible);
        backend.appState = Object.assign({}, backend.appState, { selected_actor: null });
        verify(inspector.settingsVisible);
        backend.inspectedLighting = "assets/Sun.blocklighting";
        backend.appState = Object.assign({}, backend.appState, { selected_actor: "different" });
        compare(backend.inspectedLighting, "");
        compare(inspector.settingsVisible, false);
    }
    function test_lightingAssetEdits() {
        panel.page = "lighting";
        panel.lightingPath = "assets/Sunset.blocklighting";
        compare(calls[0].command, "read_lighting_asset");
        verify(panel.showSection("Lighting"));
        verify(panel.showSection("Shadows"));
        verify(panel.showSection("Ray tracing"));
        verify(!panel.showSection("Physics"));
        panel.writeLighting({ illuminance: 456 });
        const write = calls.find(c => c.command === "write_lighting_asset");
        verify(!!write);
        compare(write.args.path, "assets/Sunset.blocklighting");
        compare(write.args.lighting.illuminance, 456);
        panel.writeLighting({ ambient_brightness: 12 });
        const writes = calls.filter(c => c.command === "write_lighting_asset");
        compare(writes[1].args.lighting.illuminance, 456);
        compare(writes[1].args.lighting.ambient_brightness, 12);
    }
    function test_convertInlineLighting() {
        panel.createLighting();
        compare(calls[0].command, "create_asset");
        compare(calls[1].command, "set_scene_lighting_asset");
        compare(backend.inspectedLighting, "assets/Scene Lighting.blocklighting");
    }
}
