import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// One Constraint component: a joint to another body or to the world. An actor
// may carry several; each is addressed by its id. Edits go through
// `set_constraint`, which refuses a setting that would leave the actor invalid.
ColumnLayout {
    id: root
    required property var app
    // The Constraint component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    property string actorId: ""
    spacing: 6

    readonly property var defaults: ({
        name: "", enabled: true, kind: "Fixed", target: "", anchor: [0, 0, 0], connected_anchor: [0, 0, 0], auto_configure: true,
        axis: [1, 0, 0], connected_axis: [1, 0, 0], limit: { enabled: false, min: 0, max: 0 },
        motor: { mode: "Off", target: 0, max_force: 0, stiffness: 100, damping: 10 },
        spring: { stiffness: 100, damping: 10, rest_length: 1 }, min_distance: 0, max_distance: 1,
        axes: ["Locked", "Locked", "Locked", "Locked", "Locked", "Locked"], axis_limits: [[0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0]],
        break_force: null, break_torque: null, enable_collision: false, break_message: ""
    })
    readonly property var k: Object.assign({}, defaults, component.constraint || {})
    readonly property string kind: k.kind
    readonly property bool hasAxis: is3d && (kind === "Hinge" || kind === "Slider" || kind === "Wheel")
    readonly property bool hasLimit: kind === "Hinge" || kind === "Slider" || kind === "Wheel" || kind === "Ball" || kind === "Spring"
    readonly property bool hasMotor: kind === "Hinge" || kind === "Slider" || kind === "Wheel"
    readonly property bool turns: kind === "Hinge" || kind === "Ball"

    readonly property var kindChoices: {
        const all = [["Fixed", "Fixed"], ["Hinge", "Hinge"], ["Ball", "Ball and socket"], ["Slider", "Slider"], ["Spring", "Spring"],
                     ["Distance", "Distance (rope)"], ["Wheel", "Wheel"], ["Configurable", "Configurable"]];
        return all.filter(p => is3d || p[0] !== "Ball").map(p => ({ value: p[0], label: p[1] }));
    }
    readonly property var motorChoices: [{ value: "Off", label: "Off" }, { value: "Velocity", label: "Speed" }, { value: "Position", label: "Target" }]
    readonly property var axisModeChoices: [{ value: "Locked", label: "Locked" }, { value: "Limited", label: "Limited" }, { value: "Free", label: "Free" }]
    readonly property var axisNames: is3d ? ["Move X", "Move Y", "Move Z", "Turn X", "Turn Y", "Turn Z"] : ["Move X", "Move Y", "", "", "", "Turn"]
    readonly property var targetChoices: {
        const out = [{ value: "", label: "The world" }];
        const project = app.appState.project;
        const actors = (project && project.actors) || [];
        for (const a of actors) if (a.id !== actorId) out.push({ value: a.id, label: a.name });
        return out;
    }

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function withIndex(array, index, value) { const next = array.slice(); next[index] = value; return next; }
    function write(next) { app.invoke("set_constraint", { constraint: Object.assign(copy(k), next) }); }
    function writeIn(key, next) { write({ [key]: Object.assign(copy(k[key]), next) }); }
    // A blank box means "never breaks"; a number is the threshold.
    function threshold(n) { return n > 0 ? n : null; }

    InspectorRow { label: "Name"; Layout.fillWidth: true
        BwTextField { objectName: "constraint-name"; Layout.fillWidth: true; implicitHeight: 28; font.pixelSize: 12
            placeholderText: "used by joint blocks"; text: root.k.name; onEditingFinished: if (text !== root.k.name) root.write({ name: text }) } }
    InspectorRow { label: "Enabled"; Layout.fillWidth: true
        SwitchField { objectName: "constraint-enabled"; value: root.k.enabled; onToggled: on => root.write({ enabled: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Kind"; Layout.fillWidth: true
        ChoiceField { objectName: "constraint-kind"; options: root.kindChoices; value: root.kind; onChosen: v => root.write({ kind: v }) } }
    InspectorRow { label: "Connected to"; Layout.fillWidth: true
        ChoiceField { objectName: "constraint-target"; options: root.targetChoices; value: root.k.target; onChosen: v => root.write({ target: v }) } }
    InspectorRow { label: "Anchor"; Layout.fillWidth: true
        Repeater { model: root.is3d ? 3 : 2
            delegate: NumberField { required property int index; objectName: "constraint-anchor-" + index; value: root.k.anchor[index]
                onCommitted: n => root.write({ anchor: root.withIndex(root.k.anchor, index, n) }) } } }
    InspectorRow { label: "Auto configure"; Layout.fillWidth: true
        SwitchField { objectName: "constraint-auto"; value: root.k.auto_configure; onToggled: on => root.write({ auto_configure: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: root.k.target === "" ? "World anchor" : "Other anchor"; visible: !root.k.auto_configure; Layout.fillWidth: true
        Repeater { model: root.is3d ? 3 : 2
            delegate: NumberField { required property int index; objectName: "constraint-connected-" + index; value: root.k.connected_anchor[index]
                onCommitted: n => root.write({ connected_anchor: root.withIndex(root.k.connected_anchor, index, n) }) } } }
    InspectorRow { label: "Axis"; visible: root.hasAxis; Layout.fillWidth: true
        Repeater { model: 3
            delegate: NumberField { required property int index; objectName: "constraint-axis-" + index; value: root.k.axis[index]
                onCommitted: n => root.write({ axis: root.withIndex(root.k.axis, index, n), connected_axis: root.withIndex(root.k.axis, index, n) }) } } }

    // ─── Limits, motor, spring, distance ───────────────────────────────────
    InspectorRow { label: root.turns ? "Limit degrees" : "Limit travel"; visible: root.hasLimit && root.kind !== "Spring"; Layout.fillWidth: true
        SwitchField { objectName: "constraint-limit"; value: root.k.limit.enabled; onToggled: on => root.writeIn("limit", { enabled: on }) }
        NumberField { objectName: "constraint-limit-min"; visible: root.k.limit.enabled; value: root.k.limit.min
            onCommitted: n => root.writeIn("limit", { min: Math.min(n, root.k.limit.max) }) }
        NumberField { objectName: "constraint-limit-max"; visible: root.k.limit.enabled; value: root.k.limit.max
            onCommitted: n => root.writeIn("limit", { max: Math.max(n, root.k.limit.min) }) } }
    InspectorRow { label: "Motor"; visible: root.hasMotor; Layout.fillWidth: true
        ChoiceField { objectName: "constraint-motor"; options: root.motorChoices; value: root.k.motor.mode; onChosen: v => root.writeIn("motor", { mode: v }) } }
    InspectorRow { label: root.k.motor.mode === "Velocity" ? "Speed" : "Target"; visible: root.hasMotor && root.k.motor.mode !== "Off"; Layout.fillWidth: true
        NumberField { objectName: "constraint-motor-target"; value: root.k.motor.target; onCommitted: n => root.writeIn("motor", { target: n }) } }
    InspectorRow { label: "Max force"; visible: root.hasMotor && root.k.motor.mode !== "Off"; Layout.fillWidth: true
        NumberField { objectName: "constraint-motor-force"; value: root.k.motor.max_force; fallback: 0
            onCommitted: n => root.writeIn("motor", { max_force: Math.max(0, n) }) } }
    InspectorRow { label: "Stiffness"; visible: root.kind === "Spring" || root.kind === "Wheel" || (root.hasMotor && root.k.motor.mode === "Position"); Layout.fillWidth: true
        NumberField { objectName: "constraint-stiffness"; value: root.kind === "Hinge" || root.kind === "Slider" ? root.k.motor.stiffness : root.k.spring.stiffness
            onCommitted: n => root.kind === "Hinge" || root.kind === "Slider" ? root.writeIn("motor", { stiffness: Math.max(0, n) }) : root.writeIn("spring", { stiffness: Math.max(0, n) }) } }
    InspectorRow { label: "Damping"; visible: root.kind === "Spring" || root.kind === "Wheel" || root.hasMotor && root.k.motor.mode !== "Off"; Layout.fillWidth: true
        NumberField { objectName: "constraint-damping"; value: root.kind === "Hinge" || root.kind === "Slider" ? root.k.motor.damping : root.k.spring.damping
            onCommitted: n => root.kind === "Hinge" || root.kind === "Slider" ? root.writeIn("motor", { damping: Math.max(0, n) }) : root.writeIn("spring", { damping: Math.max(0, n) }) } }
    InspectorRow { label: "Rest length"; visible: root.kind === "Spring"; Layout.fillWidth: true
        NumberField { objectName: "constraint-rest"; value: root.k.spring.rest_length; onCommitted: n => root.writeIn("spring", { rest_length: Math.max(0, n) }) } }
    InspectorRow { label: "Distance range"; visible: root.kind === "Distance"; Layout.fillWidth: true
        NumberField { objectName: "constraint-min-distance"; value: root.k.min_distance
            onCommitted: n => root.write({ min_distance: Math.max(0, Math.min(n, root.k.max_distance)) }) }
        NumberField { objectName: "constraint-max-distance"; value: root.k.max_distance
            onCommitted: n => root.write({ max_distance: Math.max(0.01, n, root.k.min_distance) }) } }
    Repeater {
        model: root.kind === "Configurable" ? (root.is3d ? 6 : 3) : 0
        delegate: InspectorRow {
            required property int index
            readonly property int slot: root.is3d ? index : (index === 2 ? 5 : index)
            label: root.axisNames[slot]; Layout.fillWidth: true
            ChoiceField { objectName: "constraint-axis-mode-" + slot; options: root.axisModeChoices; value: root.k.axes[slot]
                onChosen: v => root.write({ axes: root.withIndex(root.k.axes, slot, v) }) }
            NumberField { objectName: "constraint-axis-min-" + slot; visible: root.k.axes[slot] === "Limited"; value: root.k.axis_limits[slot][0]
                onCommitted: n => root.write({ axis_limits: root.withIndex(root.k.axis_limits, slot, [n, root.k.axis_limits[slot][1]]) }) }
            NumberField { objectName: "constraint-axis-max-" + slot; visible: root.k.axes[slot] === "Limited"; value: root.k.axis_limits[slot][1]
                onCommitted: n => root.write({ axis_limits: root.withIndex(root.k.axis_limits, slot, [root.k.axis_limits[slot][0], n]) }) }
        }
    }

    // ─── Breaking and collision ────────────────────────────────────────────
    InspectorRow { label: "Break force"; Layout.fillWidth: true
        NumberField { objectName: "constraint-break-force"; value: root.k.break_force === null ? 0 : root.k.break_force; fallback: 0
            onCommitted: n => root.write({ break_force: root.threshold(n) }) } }
    InspectorRow { label: "Break torque"; Layout.fillWidth: true
        NumberField { objectName: "constraint-break-torque"; value: root.k.break_torque === null ? 0 : root.k.break_torque; fallback: 0
            onCommitted: n => root.write({ break_torque: root.threshold(n) }) } }
    InspectorRow { label: "When it breaks"; visible: root.k.break_force !== null || root.k.break_torque !== null; Layout.fillWidth: true
        BwTextField { objectName: "constraint-break-message"; Layout.fillWidth: true; implicitHeight: 28; font.pixelSize: 12
            placeholderText: "message to broadcast"; text: root.k.break_message; onEditingFinished: if (text !== root.k.break_message) root.write({ break_message: text }) } }
    InspectorRow { label: "Collide with it"; Layout.fillWidth: true
        SwitchField { objectName: "constraint-collide"; value: root.k.enable_collision; onToggled: on => root.write({ enable_collision: on }) } Item { Layout.fillWidth: true } }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Break force and torque of 0 mean it never breaks. Both actors need Rigidbodies. Joint blocks reach this by name, or by its place (1 is the first)." }
}
