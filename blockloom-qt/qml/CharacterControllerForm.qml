import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The CharacterController component: a capsule that blocks sweep through the
// world, drawn from the type registry. Every edit sends the whole next spec to
// `set_character_controller`, so the backend validates it and an undo step covers it.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    // The CharacterController component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    // The controller record's fields, from the registry of this dimension.
    property var fields: []
    spacing: 6

    readonly property var defaults: {
        const out = {};
        for (const f of fields) if (f.default !== undefined) out[f.name] = f.default;
        return out;
    }
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

    RegistryForm {
        objectName: "controller-form"
        app: root.app
        // The layer gets a picker with the project's layer names.
        fields: root.fields.filter(f => f.name !== "layer")
        value: root.c
        onChanged: next => root.write(next)
    }
    InspectorRow { label: "Layer"; Layout.fillWidth: true
        ChoiceField { objectName: "controller-layer"; options: root.layerChoices; value: String(root.c.layer); onChosen: v => root.write({ layer: Number(v) }) } }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Move it with the move controller block or a script. It slides along what it hits and does not need a Rigidbody." }
}
