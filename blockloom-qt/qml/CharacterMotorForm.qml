import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The CharacterMotor component: walking, sprinting, jumping and crouching on
// top of a CharacterController, drawn from the type registry. Every edit sends
// the whole next spec to `set_character_motor`, so the backend validates it.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    // The CharacterMotor component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    // The motor record's fields, from the registry of this dimension.
    property var fields: []
    spacing: 6

    readonly property var defaults: {
        const out = {};
        for (const f of fields) if (f.default !== undefined) out[f.name] = f.default;
        return out;
    }
    readonly property var m: Object.assign({}, defaults, component.motor || {})

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function write(next) {
        app.invoke("set_character_motor", { actorId: actorId, motor: Object.assign(copy(m), next) });
    }

    RegistryForm {
        objectName: "motor-form"
        app: root.app
        fields: root.fields
        value: root.m
        onChanged: next => root.write(next)
    }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Needs a CharacterController. A player-driven motor reads the Move, Jump, Sprint and Crouch actions; a blocks or AI motor is steered with the motor block." }
}
