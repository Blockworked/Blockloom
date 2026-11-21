import QtQuick
import QtTest
import "../../qml" as Editor
import "SettingsFixture.js" as Fixture
import "fixtures/Registry3d.js" as Registry

// The Light, Emitter and Fog cards are drawn from the backend's type
// registry: the fixture is Rust's own listing, so these are Rust's defaults.
TestCase {
    id: test
    name: "RegistryCards"
    visible: true
    when: windowShown
    width: 420; height: 900
    property var calls: []
    function last() { return calls[calls.length - 1]; }
    function actorWith(components) {
        return { id: "a1", name: "Lamp", components: [{ component: "Place", placement: { position: [0, 0, 0], rotation: [0, 0, 0], scale: 1 } }].concat(components) };
    }
    function projectState() {
        const world = Fixture.world(); world.mode = "ThreeD";
        return { project_path: "/tmp/Demo", selected_actor: "a1", project: {
            world: world, actors: [], icon: "", active_scene: "one", default_scene: "one",
            scenes: [{ id: "one", name: "Scene 1", path: "assets/Scene 1.blockscene", world: world }] } };
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
        property var typeRegistry: Registry.registry
        function componentFields(name) {
            const kind = typeRegistry.components.find(k => k.name === name);
            return kind ? kind.fields : [];
        }
        function worldField(name) { return typeRegistry.world.find(f => f.name === name) || null; }
        function selectScene(id) {}
        function selectLighting(path) {}
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (done) done(["list_assets", "pipeline_status", "list_player_profiles"].indexOf(command) >= 0 ? [] : {});
        }
        function assetUrl(path) { return ""; }
        function fromFileUrl(path) { return String(path); }
    }
    Component { id: inspectorFactory; Editor.InspectorPanel { app: backend } }
    Component { id: settingsFactory; Editor.SettingsFields { app: backend } }

    function init() { calls = []; backend.appState = projectState(); }
    function inspect(components) {
        const actor = actorWith(components);
        backend.appState.project.actors = [actor];
        backend.openActor = actor;
        const panel = createTemporaryObject(inspectorFactory, test, { width: 400, height: 880 });
        verify(panel !== null);
        return panel;
    }
    function rowIn(item, name) { return findChild(item, "property-row-" + name); }

    function test_theLightCardIsTheRegistrysFields() {
        const panel = inspect([{ component: "Light", light: { kind: "Point" } }]);
        tryVerify(() => findChild(panel, "light-form") !== null);
        const form = findChild(panel, "light-form");
        tryVerify(() => rowIn(form, "intensity") !== null);
        // Rust's default shows for what the document left out.
        compare(findChild(rowIn(form, "intensity"), "property-number").text, "800");
        verify(rowIn(form, "radius").visible);
        verify(!rowIn(form, "inner_angle").visible, "the cone is a spot's");
        verify(!rowIn(form, "height").visible, "a size is an area light's");
    }

    function test_anEditWritesTheWholeLightClamped() {
        const panel = inspect([{ component: "Light", light: { kind: "Spot", intensity: 100 } }]);
        tryVerify(() => findChild(panel, "light-form") !== null);
        const form = findChild(panel, "light-form");
        tryVerify(() => rowIn(form, "inner_angle") !== null && rowIn(form, "inner_angle").visible);
        const box = findChild(rowIn(form, "intensity"), "property-number");
        box.text = "-20";
        box.editingFinished();
        const call = last();
        compare(call.command, "set_actor_component");
        compare(call.args.name, "Light");
        compare(call.args.component.component, "Light");
        compare(call.args.component.light.intensity, 0, "the schema's min clamps it");
        compare(call.args.component.light.kind, "Spot", "the rest of the light is kept");
    }

    function test_theBeamFollowsTheLight() {
        const panel = inspect([{ component: "Light", light: { kind: "Spot", volumetric: true, beam: { density: 0.5 } } }]);
        tryVerify(() => findChild(panel, "light-form") !== null);
        const beam = rowIn(findChild(panel, "light-form"), "beam");
        tryVerify(() => rowIn(beam, "shaft_noise") !== null);
        verify(rowIn(beam, "shaft_noise").visible, "a spot's beam can be a shaft");
        verify(rowIn(beam, "mode").visible);
    }

    function test_theEmitterCardIsTheRegistrysFields() {
        const panel = inspect([{ component: "Emitter", emitter: {} }]);
        tryVerify(() => findChild(panel, "emitter-form") !== null);
        const form = findChild(panel, "emitter-form");
        tryVerify(() => rowIn(form, "rate") !== null);
        compare(findChild(rowIn(form, "rate"), "property-number").text, "24");
        verify(rowIn(form, "shape") !== null, "the spawn shape is drawn too");
        verify(rowIn(form, "render") !== null);
    }

    function test_addingAComponentAsksTheRegistry() {
        const panel = inspect([]);
        panel.add("Light");
        compare(last().command, "add_component");
        compare(last().args.type, "Light");
        panel.add("Script");
        compare(last().command, "create_script", "a script still makes its file");
    }

    function test_theFogSectionIsTheRegistrysFields() {
        backend.inspectScene = true; backend.inspectedScene = "one";
        const settings = createTemporaryObject(settingsFactory, test, { width: 400, height: 880 });
        verify(settings !== null);
        tryVerify(() => findChild(settings, "fog-form") !== null);
        const form = findChild(settings, "fog-form");
        const height = rowIn(form, "height");
        tryVerify(() => rowIn(height, "enabled") !== null);
        findChild(rowIn(height, "enabled"), "property-switch").toggle();
        const call = last();
        compare(call.command, "set_fog");
        compare(call.args.fog.height.enabled, !Fixture.world().fog.height.enabled);
        verify(call.args.fog.volumetric !== undefined, "the whole fog goes back");
    }
}
