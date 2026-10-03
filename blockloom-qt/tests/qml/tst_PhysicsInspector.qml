import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "PhysicsInspector"
    visible: true
    width: 600; height: 900
    when: windowShown

    property var calls: []
    QtObject {
        id: stubApp
        property var assetDrag: ({ kind: "" })
        property var appState: ({ project: { physics: { layers: { names: ["Ground", "Player"] } } } })
        function invoke(command, args, done, failed) { test.calls.push({ command: command, args: args }); if (done) done({}); }
    }
    Component { id: bodyFactory; Editor.RigidbodyForm { app: stubApp; actorId: "a1" } }
    Component { id: colliderFactory; Editor.ColliderForm { app: stubApp } }
    Component { id: controllerFactory; Editor.CharacterControllerForm { app: stubApp; actorId: "a1" } }

    function init() { calls = []; }
    function last() { return calls[calls.length - 1]; }

    function test_aBlankRigidbodyShowsUnityDefaults() {
        const form = createTemporaryObject(bodyFactory, test, { component: { component: "Rigidbody", rigidbody: {} } });
        compare(form.r.body_type, "Dynamic");
        compare(form.r.mass.mass, 1);
        compare(form.r.interpolation, "None");
        compare(form.r.max_angular_velocity, 7);
    }
    function test_anEditSendsTheWholeNextSpec() {
        const form = createTemporaryObject(bodyFactory, test, { component: { component: "Rigidbody", rigidbody: { id: "rb", mass: { mode: "Explicit", mass: 3 } } } });
        form.write({ interpolation: "Interpolate" });
        compare(last().command, "set_rigidbody");
        compare(last().args.actorId, "a1");
        compare(last().args.rigidbody.interpolation, "Interpolate");
        compare(last().args.rigidbody.mass.mass, 3);
        compare(last().args.rigidbody.id, "rb");
    }
    function test_freezingAnAxisKeepsTheOthers() {
        const form = createTemporaryObject(bodyFactory, test, { component: { component: "Rigidbody", rigidbody: { constraints: { freeze_position: [false, true, false], freeze_rotation: [false, false, false] } } } });
        form.setAxis("freeze_rotation", 2, true);
        compare(last().args.rigidbody.constraints.freeze_position.join(","), "false,true,false");
        compare(last().args.rigidbody.constraints.freeze_rotation.join(","), "false,false,true");
    }
    function test_aTwoDimensionalBodyOffersStaticAndSimulated() {
        const form = createTemporaryObject(bodyFactory, test, { component: { rigidbody: {} }, is3d: false });
        compare(form.typeOptions.length, 3);
        compare(form.axes.length, 2);
    }
    function test_aCustomCentreBringsAnInertiaAlong() {
        const form = createTemporaryObject(bodyFactory, test, { component: { rigidbody: {} } });
        form.write({ center_of_mass: [0, 1, 0], inertia: form.r.inertia || [1, 1, 1] });
        compare(last().args.rigidbody.inertia.length, 3);
    }

    function test_aColliderEditSendsTheWholeSpecWithItsId() {
        const form = createTemporaryObject(colliderFactory, test, { component: { component: "Collider", collider: { id: "c1", geometry: { kind: "Shape", shape: { kind: "Box", size: [1, 2, 3] } }, layer: 2 } } });
        form.writeShape({ size: [1, 5, 3] });
        compare(last().command, "set_collider");
        compare(last().args.collider.id, "c1");
        compare(last().args.collider.geometry.shape.size.join(","), "1,5,3");
        compare(last().args.collider.layer, 2);
    }
    function test_choosingAShapeStartsFromItsDefaults() {
        const form = createTemporaryObject(colliderFactory, test, { component: { collider: { id: "c1", geometry: { kind: "FromLook" } } } });
        compare(form.shapeKind, "FromLook");
        form.chooseShape("Sphere");
        compare(last().args.collider.geometry.shape.kind, "Sphere");
        form.chooseShape("FromLook");
        compare(last().args.collider.geometry.kind, "FromLook");
    }
    function test_shapesOfTheOtherDimensionAreNotOffered() {
        const flat = createTemporaryObject(colliderFactory, test, { component: { collider: { id: "c" } }, is3d: false });
        verify(flat.shapeChoices.some(s => s.value === "Rect"));
        verify(!flat.shapeChoices.some(s => s.value === "Box"));
    }
    function test_layerNamesComeFromTheProject() {
        const form = createTemporaryObject(colliderFactory, test, { component: { collider: { id: "c" } } });
        compare(form.layerChoices.length, 32);
        compare(form.layerChoices[0].label, "1 Ground");
        compare(form.layerChoices[5].label, "Layer 6");
    }
    function test_aLayerBitFlipsOnlyThatLayer() {
        const form = createTemporaryObject(colliderFactory, test, { component: { collider: { id: "c", layer_overrides: { include: 1, exclude: 0, priority: 0 } } } });
        form.setLayerBit("include", 31);
        compare(last().args.collider.layer_overrides.include, 2147483649);
        form.setLayerBit("exclude", 0);
        compare(last().args.collider.layer_overrides.exclude, 1);
    }
    function test_theFrictionOverrideWritesBothCoefficientsOrClears() {
        const form = createTemporaryObject(colliderFactory, test, { component: { collider: { id: "c", material_overrides: { bounciness: 0.4 } } } });
        form.setOverride("bounciness", null);
        compare(last().args.collider.material_overrides.bounciness, undefined);
        form.setMaterial("Ice");
        compare(last().args.collider.material.kind, "BuiltIn");
        compare(last().args.collider.material.name, "Ice");
    }
    function test_pointsParseFromText() {
        const form = createTemporaryObject(colliderFactory, test, { component: { collider: { id: "c" } }, is3d: false });
        compare(JSON.stringify(form.parsePoints("0,0  10,0 5,8 junk 3")), "[[0,0],[10,0],[5,8]]");
    }

    function test_aBlankControllerShowsUnityDefaults() {
        const form = createTemporaryObject(controllerFactory, test, { component: { component: "CharacterController", controller: {} } });
        compare(form.c.radius, 0.5);
        compare(form.c.height, 2);
        compare(form.c.slope_limit, 45);
        compare(form.c.step_offset, 0.3);
    }
    function test_aTwoDimensionalControllerIsInPixels() {
        const form = createTemporaryObject(controllerFactory, test, { component: { controller: {} }, is3d: false });
        compare(form.c.radius, 16);
        compare(form.c.height, 64);
    }
    function test_aControllerEditSendsTheWholeNextSpec() {
        const form = createTemporaryObject(controllerFactory, test, { component: { controller: { id: "cc", slope_limit: 30 } } });
        form.write({ step_offset: 0.5 });
        compare(last().command, "set_character_controller");
        compare(last().args.actorId, "a1");
        compare(last().args.controller.id, "cc");
        compare(last().args.controller.slope_limit, 30);
        compare(last().args.controller.step_offset, 0.5);
    }
}
