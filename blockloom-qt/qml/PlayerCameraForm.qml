import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The PlayerCamera component: how the actor's Camera reacts to look input,
// walls and the target's motion, drawn from the type registry. It configures
// the Camera; it never adds one. Edits go through `set_actor_component`.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    // The PlayerCamera component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    // The player camera record's fields, from the registry of this dimension.
    property var fields: []
    spacing: 6

    readonly property var defaults: {
        const out = {};
        for (const f of fields) if (f.default !== undefined) out[f.name] = f.default;
        return out;
    }
    readonly property var p: Object.assign({}, defaults, component.player_camera || {})

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function write(next) {
        app.invoke("set_actor_component", { actorId: actorId, name: "PlayerCamera",
            component: { component: "PlayerCamera", player_camera: Object.assign(copy(p), next) } });
    }

    RegistryForm {
        objectName: "pcam-form"
        app: root.app
        fields: root.fields
        value: root.p
        onChanged: next => root.write(next)
    }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Works on the actor's Camera component. The mouse look needs the pointer locked; a first or third person preset takes it when Play starts." }
}
