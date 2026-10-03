import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The CharacterMotor component: walking, sprinting, jumping and crouching on
// top of a CharacterController. Every edit sends the whole next spec to
// `set_character_motor`, so the backend validates it and an undo step covers it.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    // The CharacterMotor component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    spacing: 6

    readonly property var defaults: {
        const k = is3d ? 1 : 48;
        return {
            enabled: true, owner: "Player", player: 0, space: is3d ? "Camera" : "World", top_down: false,
            walk_speed: 5 * k, sprint_speed: 8 * k, crouch_speed: 2.5 * k,
            ground_acceleration: 50 * k, ground_braking: 60 * k, air_acceleration: 15 * k, air_control: 1,
            turn_speed: is3d ? 720 : 0, normalize_input: true, gravity_scale: 1, terminal_fall_speed: 50 * k,
            ground_snap_distance: 0.3 * k, slide_on_steep: false, slide_speed: 6 * k,
            jump_height: 1.2 * k, max_jumps: 1, jump_cut: 0.5, coyote_time: 0.1, jump_buffer: 0.1,
            crouch_height: is3d ? 1 : 32, external_drag: 20 * k
        };
    }
    readonly property var m: Object.assign({}, defaults, component.motor || {})
    readonly property var ownerChoices: [{ value: "Player", label: "Player (keys and pad)" }, { value: "Script", label: "Blocks and scripts" }, { value: "Ai", label: "AI" }]
    readonly property var spaceChoices: [{ value: "Camera", label: "Camera" }, { value: "Actor", label: "Actor" }, { value: "World", label: "World" }]

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function write(next) {
        app.invoke("set_character_motor", { actorId: actorId, motor: Object.assign(copy(m), next) });
    }
    function atLeast(n, low) { return Math.max(low, n); }

    InspectorRow { label: "Enabled"; Layout.fillWidth: true
        SwitchField { objectName: "motor-enabled"; value: root.m.enabled; onToggled: on => root.write({ enabled: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Driven by"; Layout.fillWidth: true
        ChoiceField { objectName: "motor-owner"; options: root.ownerChoices; value: root.m.owner; onChosen: v => root.write({ owner: v }) } }
    InspectorRow { label: "Player"; visible: root.m.owner === "Player"; Layout.fillWidth: true
        NumberField { objectName: "motor-player"; value: root.m.player + 1; fallback: 1
            onCommitted: n => root.write({ player: Math.max(0, Math.min(7, Math.round(n) - 1)) }) } }
    InspectorRow { label: "Move space"; visible: root.is3d; Layout.fillWidth: true
        ChoiceField { objectName: "motor-space"; options: root.spaceChoices; value: root.m.space; onChosen: v => root.write({ space: v }) } }
    InspectorRow { label: "Top down"; visible: !root.is3d; Layout.fillWidth: true
        SwitchField { objectName: "motor-top-down"; value: root.m.top_down; onToggled: on => root.write({ top_down: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Walk speed"; Layout.fillWidth: true
        NumberField { objectName: "motor-walk"; value: root.m.walk_speed; fallback: root.defaults.walk_speed
            onCommitted: n => root.write({ walk_speed: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Sprint speed"; Layout.fillWidth: true
        NumberField { objectName: "motor-sprint"; value: root.m.sprint_speed; fallback: root.defaults.sprint_speed
            onCommitted: n => root.write({ sprint_speed: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Crouch speed"; Layout.fillWidth: true
        NumberField { objectName: "motor-crouch-speed"; value: root.m.crouch_speed; fallback: root.defaults.crouch_speed
            onCommitted: n => root.write({ crouch_speed: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Acceleration"; Layout.fillWidth: true
        NumberField { objectName: "motor-accel"; value: root.m.ground_acceleration; fallback: root.defaults.ground_acceleration
            onCommitted: n => root.write({ ground_acceleration: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Braking"; Layout.fillWidth: true
        NumberField { objectName: "motor-braking"; value: root.m.ground_braking; fallback: root.defaults.ground_braking
            onCommitted: n => root.write({ ground_braking: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Air acceleration"; Layout.fillWidth: true
        NumberField { objectName: "motor-air-accel"; value: root.m.air_acceleration; fallback: root.defaults.air_acceleration
            onCommitted: n => root.write({ air_acceleration: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Air control"; Layout.fillWidth: true
        NumberField { objectName: "motor-air-control"; value: root.m.air_control; fallback: 1
            onCommitted: n => root.write({ air_control: Math.max(0, Math.min(1, n)) }) } }
    InspectorRow { label: "Turn speed"; visible: root.is3d; Layout.fillWidth: true
        NumberField { objectName: "motor-turn"; value: root.m.turn_speed; fallback: root.defaults.turn_speed
            onCommitted: n => root.write({ turn_speed: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Normalize input"; Layout.fillWidth: true
        SwitchField { objectName: "motor-normalize"; value: root.m.normalize_input; onToggled: on => root.write({ normalize_input: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Jump height"; Layout.fillWidth: true
        NumberField { objectName: "motor-jump"; value: root.m.jump_height; fallback: root.defaults.jump_height
            onCommitted: n => root.write({ jump_height: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Max jumps"; Layout.fillWidth: true
        NumberField { objectName: "motor-max-jumps"; value: root.m.max_jumps; fallback: 1
            onCommitted: n => root.write({ max_jumps: Math.max(0, Math.round(n)) }) } }
    InspectorRow { label: "Jump cut"; Layout.fillWidth: true
        NumberField { objectName: "motor-jump-cut"; value: root.m.jump_cut; fallback: 0.5
            onCommitted: n => root.write({ jump_cut: Math.max(0, Math.min(1, n)) }) } }
    InspectorRow { label: "Coyote time"; Layout.fillWidth: true
        NumberField { objectName: "motor-coyote"; value: root.m.coyote_time; fallback: 0.1
            onCommitted: n => root.write({ coyote_time: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Jump buffer"; Layout.fillWidth: true
        NumberField { objectName: "motor-buffer"; value: root.m.jump_buffer; fallback: 0.1
            onCommitted: n => root.write({ jump_buffer: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Gravity scale"; Layout.fillWidth: true
        NumberField { objectName: "motor-gravity"; value: root.m.gravity_scale; fallback: 1
            onCommitted: n => root.write({ gravity_scale: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Terminal speed"; Layout.fillWidth: true
        NumberField { objectName: "motor-terminal"; value: root.m.terminal_fall_speed; fallback: root.defaults.terminal_fall_speed
            onCommitted: n => root.write({ terminal_fall_speed: root.atLeast(n, 0.001) }) } }
    InspectorRow { label: "Ground snap"; Layout.fillWidth: true
        NumberField { objectName: "motor-snap"; value: root.m.ground_snap_distance; fallback: root.defaults.ground_snap_distance
            onCommitted: n => root.write({ ground_snap_distance: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Slide on steep"; Layout.fillWidth: true
        SwitchField { objectName: "motor-slide"; value: root.m.slide_on_steep; onToggled: on => root.write({ slide_on_steep: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Slide speed"; visible: root.m.slide_on_steep; Layout.fillWidth: true
        NumberField { objectName: "motor-slide-speed"; value: root.m.slide_speed; fallback: root.defaults.slide_speed
            onCommitted: n => root.write({ slide_speed: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Crouch height"; Layout.fillWidth: true
        NumberField { objectName: "motor-crouch-height"; value: root.m.crouch_height; fallback: root.defaults.crouch_height
            onCommitted: n => root.write({ crouch_height: root.atLeast(n, 0) }) } }
    InspectorRow { label: "Knockback drag"; Layout.fillWidth: true
        NumberField { objectName: "motor-drag"; value: root.m.external_drag; fallback: root.defaults.external_drag
            onCommitted: n => root.write({ external_drag: root.atLeast(n, 0) }) } }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Needs a CharacterController. A player-driven motor reads the Move, Jump, Sprint and Crouch actions; a blocks or AI motor is steered with the motor block." }
}
