import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The PlayerCamera component: how the actor's Camera reacts to look input,
// walls and the target's motion. It configures the Camera; it never adds one.
// Every edit writes the whole next spec through `set_actor_component`.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    // The PlayerCamera component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    spacing: 6

    readonly property var defaults: ({
        enabled: true, player: 0, look: is3d, sensitivity: 0.12, stick_speed: 180, invert_y: false,
        min_pitch: -80, max_pitch: 80, turn_body: false, smoothing: is3d ? 0 : 0.15,
        collision: is3d, collision_radius: 0.25, min_distance: 0.6,
        zoom: false, zoom_min: 2, zoom_max: 12, zoom_speed: 1,
        dead_zone: is3d ? [0, 0] : [48, 32], look_ahead: is3d ? 0 : 64, look_ahead_smoothing: 0.25, eye_follows_stance: true
    })
    readonly property var p: Object.assign({}, defaults, component.player_camera || {})

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function write(next) {
        app.invoke("set_actor_component", { actorId: actorId, name: "PlayerCamera",
            component: { component: "PlayerCamera", player_camera: Object.assign(copy(p), next) } });
    }

    InspectorRow { label: "Enabled"; Layout.fillWidth: true
        SwitchField { objectName: "pcam-enabled"; value: root.p.enabled; onToggled: on => root.write({ enabled: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Look input"; visible: root.is3d; Layout.fillWidth: true
        SwitchField { objectName: "pcam-look"; value: root.p.look; onToggled: on => root.write({ look: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Sensitivity"; visible: root.is3d && root.p.look; Layout.fillWidth: true
        NumberField { objectName: "pcam-sensitivity"; value: root.p.sensitivity; fallback: 0.12
            onCommitted: n => root.write({ sensitivity: Math.max(0, n) }) } }
    InspectorRow { label: "Stick speed"; visible: root.is3d && root.p.look; Layout.fillWidth: true
        NumberField { objectName: "pcam-stick"; value: root.p.stick_speed; fallback: 180
            onCommitted: n => root.write({ stick_speed: Math.max(0, n) }) } }
    InspectorRow { label: "Invert Y"; visible: root.is3d && root.p.look; Layout.fillWidth: true
        SwitchField { objectName: "pcam-invert"; value: root.p.invert_y; onToggled: on => root.write({ invert_y: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Pitch limits"; visible: root.is3d && root.p.look; Layout.fillWidth: true
        NumberField { objectName: "pcam-min-pitch"; value: root.p.min_pitch; fallback: -80
            onCommitted: n => root.write({ min_pitch: Math.max(-89, Math.min(root.p.max_pitch, n)) }) }
        NumberField { objectName: "pcam-max-pitch"; value: root.p.max_pitch; fallback: 80
            onCommitted: n => root.write({ max_pitch: Math.min(89, Math.max(root.p.min_pitch, n)) }) } }
    InspectorRow { label: "Turn body"; visible: root.is3d && root.p.look; Layout.fillWidth: true
        SwitchField { objectName: "pcam-turn-body"; value: root.p.turn_body; onToggled: on => root.write({ turn_body: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Eye follows crouch"; visible: root.is3d; Layout.fillWidth: true
        SwitchField { objectName: "pcam-eye"; value: root.p.eye_follows_stance; onToggled: on => root.write({ eye_follows_stance: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Smoothing"; Layout.fillWidth: true
        NumberField { objectName: "pcam-smoothing"; value: root.p.smoothing; fallback: 0
            onCommitted: n => root.write({ smoothing: Math.max(0, n) }) } }
    InspectorRow { label: "Avoid walls"; visible: root.is3d; Layout.fillWidth: true
        SwitchField { objectName: "pcam-collision"; value: root.p.collision; onToggled: on => root.write({ collision: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Lens radius"; visible: root.is3d && root.p.collision; Layout.fillWidth: true
        NumberField { objectName: "pcam-radius"; value: root.p.collision_radius; fallback: 0.25
            onCommitted: n => root.write({ collision_radius: Math.max(0, n) }) } }
    InspectorRow { label: "Closest"; visible: root.is3d && root.p.collision; Layout.fillWidth: true
        NumberField { objectName: "pcam-min-distance"; value: root.p.min_distance; fallback: 0.6
            onCommitted: n => root.write({ min_distance: Math.max(0, n) }) } }
    InspectorRow { label: "Scroll zoom"; visible: root.is3d; Layout.fillWidth: true
        SwitchField { objectName: "pcam-zoom"; value: root.p.zoom; onToggled: on => root.write({ zoom: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Zoom range"; visible: root.is3d && root.p.zoom; Layout.fillWidth: true
        NumberField { objectName: "pcam-zoom-min"; value: root.p.zoom_min; fallback: 2
            onCommitted: n => root.write({ zoom_min: Math.max(0.1, Math.min(root.p.zoom_max, n)) }) }
        NumberField { objectName: "pcam-zoom-max"; value: root.p.zoom_max; fallback: 12
            onCommitted: n => root.write({ zoom_max: Math.max(root.p.zoom_min, n) }) } }
    InspectorRow { label: "Dead zone"; visible: !root.is3d; Layout.fillWidth: true
        NumberField { objectName: "pcam-dead-x"; value: root.p.dead_zone[0]; fallback: 0
            onCommitted: n => root.write({ dead_zone: [Math.max(0, n), root.p.dead_zone[1]] }) }
        NumberField { objectName: "pcam-dead-y"; value: root.p.dead_zone[1]; fallback: 0
            onCommitted: n => root.write({ dead_zone: [root.p.dead_zone[0], Math.max(0, n)] }) } }
    InspectorRow { label: "Look ahead"; visible: !root.is3d; Layout.fillWidth: true
        NumberField { objectName: "pcam-ahead"; value: root.p.look_ahead; fallback: 0
            onCommitted: n => root.write({ look_ahead: Math.max(0, n) }) } }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Works on the actor's Camera component. The mouse look needs the pointer locked; a first or third person preset takes it when Play starts." }
}
