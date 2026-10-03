import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The CharacterController component: a capsule that blocks sweep through the
// world. Every edit sends the whole next spec to `set_character_controller`,
// so the backend validates it and an undo step covers it.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    // The CharacterController component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    spacing: 6

    readonly property var defaults: is3d
        ? ({ enabled: true, center: [0, 0, 0], radius: 0.5, height: 2, slope_limit: 45, step_offset: 0.3,
             skin_width: 0.08, min_move_distance: 0.001, detect_collisions: true, overlap_recovery: true, layer: 1 })
        : ({ enabled: true, center: [0, 0, 0], radius: 16, height: 64, slope_limit: 45, step_offset: 12,
             skin_width: 2, min_move_distance: 0.05, detect_collisions: true, overlap_recovery: true, layer: 1 })
    readonly property var c: Object.assign({}, defaults, component.controller || {})

    readonly property var physics: app.appState.project ? app.appState.project.physics : null
    readonly property var layerNames: physics && physics.layers && physics.layers.names ? physics.layers.names : []
    readonly property var layerChoices: {
        const out = [];
        for (let i = 1; i <= 32; ++i) out.push({ value: String(i), label: layerNames[i - 1] ? i + " " + layerNames[i - 1] : "Layer " + i });
        return out;
    }

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function write(next) {
        app.invoke("set_character_controller", { actorId: actorId, controller: Object.assign(copy(c), next) });
    }
    function withIndex(array, index, value) { const next = array.slice(); next[index] = value; return next; }

    InspectorRow { label: "Enabled"; Layout.fillWidth: true
        SwitchField { objectName: "controller-enabled"; value: root.c.enabled; onToggled: on => root.write({ enabled: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Radius"; Layout.fillWidth: true
        NumberField { objectName: "controller-radius"; value: root.c.radius; fallback: root.defaults.radius
            onCommitted: n => root.write({ radius: Math.max(0.001, n) }) } }
    InspectorRow { label: "Height"; Layout.fillWidth: true
        NumberField { objectName: "controller-height"; value: root.c.height; fallback: root.defaults.height
            onCommitted: n => root.write({ height: Math.max(2 * root.c.radius, n) }) } }
    InspectorRow { label: "Centre"; Layout.fillWidth: true
        Repeater { model: root.is3d ? 3 : 2
            delegate: NumberField { required property int index; objectName: "controller-center-" + index; value: root.c.center[index]
                onCommitted: n => root.write({ center: root.withIndex(root.c.center, index, n) }) } } }
    InspectorRow { label: "Slope limit"; Layout.fillWidth: true
        NumberField { objectName: "controller-slope"; value: root.c.slope_limit; fallback: 45
            onCommitted: n => root.write({ slope_limit: Math.max(0, Math.min(90, n)) }) } }
    InspectorRow { label: "Step offset"; Layout.fillWidth: true
        NumberField { objectName: "controller-step"; value: root.c.step_offset; fallback: root.defaults.step_offset
            onCommitted: n => root.write({ step_offset: Math.max(0, n) }) } }
    InspectorRow { label: "Skin width"; Layout.fillWidth: true
        NumberField { objectName: "controller-skin"; value: root.c.skin_width; fallback: root.defaults.skin_width
            onCommitted: n => root.write({ skin_width: Math.max(0.0001, n) }) } }
    InspectorRow { label: "Minimum move"; Layout.fillWidth: true
        NumberField { objectName: "controller-min-move"; value: root.c.min_move_distance; fallback: root.defaults.min_move_distance
            onCommitted: n => root.write({ min_move_distance: Math.max(0, n) }) } }
    InspectorRow { label: "Detect collisions"; Layout.fillWidth: true
        SwitchField { objectName: "controller-detect"; value: root.c.detect_collisions; onToggled: on => root.write({ detect_collisions: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Overlap recovery"; Layout.fillWidth: true
        SwitchField { objectName: "controller-recovery"; value: root.c.overlap_recovery; onToggled: on => root.write({ overlap_recovery: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Layer"; Layout.fillWidth: true
        ChoiceField { objectName: "controller-layer"; options: root.layerChoices; value: String(root.c.layer); onChosen: v => root.write({ layer: Number(v) }) } }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Move it with the move controller block or a script. It slides along what it hits and does not need a Rigidbody." }
}
