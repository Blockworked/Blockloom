import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The Rigidbody component: motion, mass and integration, independent of any
// shape. Every edit sends the whole next spec to `set_rigidbody`, so the
// backend validates it and an undo step covers it.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    // The Rigidbody component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    spacing: 6

    readonly property var defaults: ({
        body_type: "Dynamic", simulated: true, mass: { mode: "Explicit", mass: 1 },
        linear_damping: 0, angular_damping: 0.05, use_gravity: true, gravity_scale: 1,
        interpolation: "None", collision_detection: "Discrete",
        constraints: { freeze_position: [false, false, false], freeze_rotation: [false, false, false] },
        max_linear_velocity: 10000, max_angular_velocity: 7, sleep_threshold: 0.005,
        solver_iterations: null, max_depenetration_velocity: 10
    })
    readonly property var r: Object.assign({}, defaults, component.rigidbody || {})
    readonly property bool density: r.mass.mode === "Density"
    readonly property var axes: is3d ? ["X", "Y", "Z"] : ["X", "Y"]

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function write(next) {
        app.invoke("set_rigidbody", { actorId: actorId, rigidbody: Object.assign(copy(r), next) });
    }
    function setAxis(group, index, on) {
        const constraints = copy(r.constraints);
        constraints[group][index] = on;
        write({ constraints: constraints });
    }
    function setCenter(index, n) {
        const c = (r.center_of_mass || [0, 0, 0]).slice();
        c[index] = n;
        write({ center_of_mass: c });
    }
    function setInertia(index, n) {
        const c = (r.inertia || [1, 1, 1]).slice();
        c[index] = Math.max(0.0001, n);
        write({ inertia: c });
    }

    readonly property var typeOptions: is3d
        ? [{ value: "Dynamic", label: "Dynamic" }, { value: "Kinematic", label: "Kinematic" }]
        : [{ value: "Dynamic", label: "Dynamic" }, { value: "Kinematic", label: "Kinematic" }, { value: "Static", label: "Static" }]
    readonly property var detectionOptions: [
        { value: "Discrete", label: "Discrete" }, { value: "Continuous", label: "Continuous" },
        { value: "ContinuousDynamic", label: "Continuous Dynamic" }, { value: "ContinuousSpeculative", label: "Continuous Speculative" }]
    readonly property var interpolationOptions: [
        { value: "None", label: "None" }, { value: "Interpolate", label: "Interpolate" }, { value: "Extrapolate", label: "Extrapolate" }]
    readonly property var massOptions: [{ value: "Explicit", label: "Mass" }, { value: "Density", label: "Density" }]

    InspectorRow { label: "Body type"; Layout.fillWidth: true
        ChoiceField { objectName: "body-type"; options: root.typeOptions; value: root.r.body_type; onChosen: v => root.write({ body_type: v }) } }
    InspectorRow { label: "Simulated"; visible: !root.is3d; Layout.fillWidth: true
        SwitchField { value: root.r.simulated; onToggled: on => root.write({ simulated: on }) } Item { Layout.fillWidth: true } }

    InspectorRow { label: "Mass from"; Layout.fillWidth: true
        ChoiceField { objectName: "mass-mode"; options: root.massOptions; value: root.r.mass.mode
            onChosen: v => root.write({ mass: v === "Density" ? { mode: "Density", density: 1 } : { mode: "Explicit", mass: 1 } }) } }
    InspectorRow { label: root.density ? "Density" : "Mass kg"; Layout.fillWidth: true
        NumberField { objectName: "mass-value"; value: root.density ? root.r.mass.density : root.r.mass.mass; fallback: 1
            onCommitted: n => root.write({ mass: root.density ? { mode: "Density", density: Math.max(0.0001, n) } : { mode: "Explicit", mass: Math.max(0.0001, n) } }) } }
    InspectorRow { label: "Centre of mass"; Layout.fillWidth: true
        SwitchField { objectName: "custom-centre"; value: !!root.r.center_of_mass
            onToggled: on => root.write({ center_of_mass: on ? [0, 0, 0] : null, inertia: on ? (root.r.inertia || [1, 1, 1]) : root.r.inertia }) }
        Repeater {
            model: root.r.center_of_mass ? root.axes.length : 0
            delegate: NumberField { required property int index; value: root.r.center_of_mass[index]; onCommitted: n => root.setCenter(index, n) }
        }
        Item { visible: !root.r.center_of_mass; Layout.fillWidth: true }
    }
    Text { visible: !!root.r.center_of_mass; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "A custom centre of mass keeps an inertia too, set below." }
    InspectorRow { label: "Inertia"; Layout.fillWidth: true
        SwitchField { objectName: "custom-inertia"; value: !!root.r.inertia; enabled: !root.r.center_of_mass; onToggled: on => root.write({ inertia: on ? [1, 1, 1] : null }) }
        Repeater {
            model: root.r.inertia ? (root.is3d ? 3 : 1) : 0
            delegate: NumberField { required property int index; value: root.r.inertia[index]; fallback: 1; onCommitted: n => root.setInertia(index, n) }
        }
        Item { visible: !root.r.inertia; Layout.fillWidth: true }
    }

    InspectorRow { label: "Drag"; Layout.fillWidth: true
        NumberField { objectName: "linear-damping"; value: root.r.linear_damping; onCommitted: n => root.write({ linear_damping: Math.max(0, n) }) } }
    InspectorRow { label: "Angular drag"; Layout.fillWidth: true
        NumberField { objectName: "angular-damping"; value: root.r.angular_damping; fallback: 0.05; onCommitted: n => root.write({ angular_damping: Math.max(0, n) }) } }
    InspectorRow { label: "Use gravity"; Layout.fillWidth: true
        SwitchField { value: root.r.use_gravity; onToggled: on => root.write({ use_gravity: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Gravity ×"; visible: root.r.use_gravity; Layout.fillWidth: true
        NumberField { objectName: "gravity-scale"; value: root.r.gravity_scale; fallback: 1; onCommitted: n => root.write({ gravity_scale: n }) } }
    InspectorRow { label: "Interpolate"; Layout.fillWidth: true
        ChoiceField { objectName: "interpolation"; options: root.interpolationOptions; value: root.r.interpolation; onChosen: v => root.write({ interpolation: v }) } }
    InspectorRow { label: "Detection"; visible: root.is3d; Layout.fillWidth: true
        ChoiceField { objectName: "collision-detection"; options: root.detectionOptions; value: root.r.collision_detection; onChosen: v => root.write({ collision_detection: v }) } }

    InspectorRow { label: "Freeze position"; Layout.fillWidth: true
        Repeater {
            model: root.axes.length
            delegate: RowLayout {
                required property int index
                spacing: 3
                Text { text: root.axes[index]; color: Theme.textDim; font.pixelSize: 11 }
                SwitchField { objectName: "freeze-position-" + index; value: root.r.constraints.freeze_position[index]; onToggled: on => root.setAxis("freeze_position", index, on) }
            }
        }
        Item { Layout.fillWidth: true }
    }
    InspectorRow { label: "Freeze rotation"; Layout.fillWidth: true
        Repeater {
            model: root.is3d ? 3 : 1
            delegate: RowLayout {
                required property int index
                readonly property int axis: root.is3d ? index : 2
                spacing: 3
                Text { text: ["X", "Y", "Z"][axis]; color: Theme.textDim; font.pixelSize: 11 }
                SwitchField { objectName: "freeze-rotation-" + axis; value: root.r.constraints.freeze_rotation[axis]; onToggled: on => root.setAxis("freeze_rotation", axis, on) }
            }
        }
        Item { Layout.fillWidth: true }
    }

    InspectorRow { label: "Max speed"; Layout.fillWidth: true
        NumberField { objectName: "max-linear"; value: root.r.max_linear_velocity; fallback: 10000; onCommitted: n => root.write({ max_linear_velocity: Math.max(0.01, n) }) } }
    InspectorRow { label: "Max spin"; visible: root.is3d; Layout.fillWidth: true
        NumberField { objectName: "max-angular"; value: root.r.max_angular_velocity; fallback: 7; onCommitted: n => root.write({ max_angular_velocity: Math.max(0.01, n) }) } }
    InspectorRow { label: "Sleep below"; Layout.fillWidth: true
        NumberField { objectName: "sleep-threshold"; value: root.r.sleep_threshold; fallback: 0.005; onCommitted: n => root.write({ sleep_threshold: Math.max(0, n) }) } }
    InspectorRow { label: "Solver passes"; Layout.fillWidth: true
        NumberField { objectName: "solver-iterations"; value: root.r.solver_iterations; allowEmpty: true; fallback: 1; placeholderText: "world default"
            onCommitted: n => root.write({ solver_iterations: n === null ? null : Math.max(1, Math.round(n)) }) } }
    InspectorRow { label: "Push-out m/s"; Layout.fillWidth: true
        NumberField { objectName: "max-depenetration"; value: root.r.max_depenetration_velocity; fallback: 10; onCommitted: n => root.write({ max_depenetration_velocity: Math.max(0, n) }) } }
    Text { visible: root.r.legacy_character_control === true; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Migrated from a Body with character control on; it keeps the old kinematic controller." }
}
