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
    property var results: ({})
    QtObject {
        id: stubApp
        property var assetDrag: ({ kind: "" })
        property var appState: ({ project: { physics: { layers: { names: ["Ground", "Player"] } } } })
        function invoke(command, args, done, failed) { test.calls.push({ command: command, args: args }); if (done) done(test.results[command] !== undefined ? test.results[command] : {}); }
    }
    Component { id: bodyFactory; Editor.RigidbodyForm { app: stubApp; actorId: "a1" } }
    Component { id: colliderFactory; Editor.ColliderForm { app: stubApp } }
    Component { id: motorFactory; Editor.CharacterMotorForm { app: stubApp; actorId: "a1" } }
    Component { id: cameraFactory; Editor.PlayerCameraForm { app: stubApp; actorId: "a1" } }
    Component { id: setupFactory; Editor.PlayerSetupCard { app: stubApp; actorId: "a1" } }
    Component { id: upgradeFactory; Editor.PhysicsUpgradeCard { app: stubApp; actorId: "a1" } }
    Component { id: constraintFactory; Editor.ConstraintForm { app: stubApp; actorId: "a1" } }
    Component { id: controllerFactory; Editor.CharacterControllerForm { app: stubApp; actorId: "a1" } }

    function init() { calls = []; results = ({}); }
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

    function test_aBlankMotorShowsTheDefaultsOfItsDimension() {
        const three = createTemporaryObject(motorFactory, test, { component: { component: "CharacterMotor", motor: {} } });
        compare(three.m.walk_speed, 5);
        compare(three.m.space, "Camera");
        const two = createTemporaryObject(motorFactory, test, { component: { motor: {} }, is3d: false });
        compare(two.m.walk_speed, 240);
        compare(two.m.space, "World");
        compare(two.m.turn_speed, 0);
    }
    function test_aMotorEditSendsTheWholeNextSpec() {
        const form = createTemporaryObject(motorFactory, test, { component: { motor: { id: "mm", jump_height: 2 } } });
        form.write({ max_jumps: 2 });
        compare(last().command, "set_character_motor");
        compare(last().args.actorId, "a1");
        compare(last().args.motor.id, "mm");
        compare(last().args.motor.jump_height, 2);
        compare(last().args.motor.max_jumps, 2);
    }

    function test_aBlankPlayerCameraShowsTheDefaultsOfItsDimension() {
        const three = createTemporaryObject(cameraFactory, test, { component: { component: "PlayerCamera", player_camera: {} } });
        compare(three.p.look, true);
        compare(three.p.collision, true);
        const two = createTemporaryObject(cameraFactory, test, { component: { player_camera: {} }, is3d: false });
        compare(two.p.look, false);
        compare(two.p.dead_zone.length, 2);
    }
    function test_aPlayerCameraEditSendsTheWholeNextSpec() {
        const form = createTemporaryObject(cameraFactory, test, { component: { player_camera: { sensitivity: 0.2 } } });
        form.write({ invert_y: true });
        compare(last().command, "set_actor_component");
        compare(last().args.name, "PlayerCamera");
        compare(last().args.component.component, "PlayerCamera");
        compare(last().args.component.player_camera.sensitivity, 0.2);
        compare(last().args.component.player_camera.invert_y, true);
    }
    function test_theSetupCardPreviewsAPresetForTheDimension() {
        results = { preview_player_preset: { steps: [{ component: "CharacterMotor", kind: "Add", detail: "" }], conflicts: [], blocked: null }, list_player_profiles: [] };
        const card = createTemporaryObject(setupFactory, test, {});
        compare(card.presets.length, 3);
        compare(card.preset, "first-person-3d");
        compare(card.summary(), "adds CharacterMotor");
        const flat = createTemporaryObject(setupFactory, test, { is3d: false });
        compare(flat.preset, "platformer-2d");
    }
    function test_aConflictNeedsTheConversionBeforeApplying() {
        results = { preview_player_preset: { steps: [], conflicts: [{ component: "Rigidbody", reason: "dynamic", conversion: "kinematic" }], blocked: null }, list_player_profiles: [] };
        const card = createTemporaryObject(setupFactory, test, {});
        compare(card.conflicts.length, 1);
        card.apply();
        compare(last().command, "apply_player_preset");
        compare(last().args.convert, false);
        card.convert = true;
        card.apply();
        compare(last().args.convert, true);
    }

    function test_aBlankConstraintShowsItsDefaults() {
        const form = createTemporaryObject(constraintFactory, test, { component: { component: "Constraint", constraint: { id: "k1", kind: "Hinge" } } });
        compare(form.k.kind, "Hinge");
        compare(form.k.enabled, true);
        compare(form.k.auto_configure, true);
        compare(form.hasMotor, true);
        compare(form.hasAxis, true);
    }
    function test_aConstraintEditSendsTheWholeSpecWithItsId() {
        const form = createTemporaryObject(constraintFactory, test, { component: { component: "Constraint", constraint: { id: "k1", name: "door", kind: "Hinge", break_force: 50 } } });
        form.writeIn("limit", { enabled: true, min: -30, max: 90 });
        compare(last().command, "set_constraint");
        compare(last().args.constraint.id, "k1");
        compare(last().args.constraint.name, "door");
        compare(last().args.constraint.break_force, 50);
        compare(last().args.constraint.limit.max, 90);
        compare(last().args.constraint.motor.mode, "Off");
    }
    function test_aBreakThresholdOfZeroMeansItNeverBreaks() {
        const form = createTemporaryObject(constraintFactory, test, { component: { constraint: { id: "k1", kind: "Fixed", break_force: 10 } } });
        form.write({ break_force: form.threshold(0) });
        compare(last().args.constraint.break_force, null);
        form.write({ break_torque: form.threshold(5) });
        compare(last().args.constraint.break_torque, 5);
    }
    function test_theTargetListHasTheWorldAndEveryOtherActor() {
        stubApp.appState = { project: { physics: { layers: { names: [] } }, actors: [{ id: "a1", name: "Door" }, { id: "a2", name: "Frame" }] } };
        const form = createTemporaryObject(constraintFactory, test, { component: { constraint: { kind: "Fixed" } } });
        compare(form.targetChoices.length, 2);
        compare(form.targetChoices[0].label, "The world");
        compare(form.targetChoices[1].value, "a2");
        stubApp.appState = { project: { physics: { layers: { names: ["Ground", "Player"] } } } };
    }
    function test_twoDimensionalConstraintsHaveNoBallOrAxis() {
        const form = createTemporaryObject(constraintFactory, test, { component: { constraint: { kind: "Hinge" } }, is3d: false });
        compare(form.hasAxis, false);
        verify(form.kindChoices.every(c => c.value !== "Ball"));
        compare(form.kindChoices.length, 7);
    }
    function test_onlyDrivenKindsOfferAMotor() {
        const spring = createTemporaryObject(constraintFactory, test, { component: { constraint: { kind: "Spring" } } });
        compare(spring.hasMotor, false);
        compare(spring.hasLimit, true);
        const rope = createTemporaryObject(constraintFactory, test, { component: { constraint: { kind: "Distance" } } });
        compare(rope.hasMotor, false);
        compare(rope.hasLimit, false);
    }

    function test_theUpgradeCardShowsTheNotesAndUpgradesOneActorOrAll() {
        results = {
            physics_migration_preview: { applied: false, total: 3, scenes: [{ scene: "S", actors: [{ actor: "a1", notes: ["Keeps following its Look."] }] }] },
            migrate_physics: { applied: true, total: 1, scenes: [] }
        };
        const card = createTemporaryObject(upgradeFactory, test);
        compare(card.projectTotal, 3);
        compare(card.actorNotes.length, 1);
        card.upgrade({ actorId: "a1" });
        const call = calls.filter(c => c.command === "migrate_physics")[0];
        compare(call.args.actorId, "a1");
        compare(card.message, "Upgraded 1 actor.");
    }
}
