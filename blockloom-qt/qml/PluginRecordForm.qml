import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A form for one plugin record, built from its schema: `type` is an entry of
// the backend's `types` list (fields and defaults), `payload` what the record
// holds. Edits report the whole next payload, which keeps any key the schema
// no longer names, so a newer document is not trimmed by an older plugin.
ColumnLayout {
    id: root
    required property var app
    property var type: ({ fields: [], defaults: {} })
    property var payload: ({})
    property var actors: []
    signal changed(var next)
    Layout.fillWidth: true
    spacing: 6

    // "bar_color" reads as "Bar color".
    function labelOf(name) {
        const spaced = String(name).replace(/_/g, " ");
        return spaced.charAt(0).toUpperCase() + spaced.slice(1);
    }
    function valueOf(field) {
        const held = root.payload ? root.payload[field.name] : undefined;
        if (held !== undefined && held !== null) return held;
        const fallback = root.type.defaults ? root.type.defaults[field.name] : undefined;
        return fallback === undefined ? null : fallback;
    }
    function with_(field, next) {
        const out = Object.assign({}, root.payload || {});
        out[field.name] = next;
        return out;
    }

    Repeater {
        model: root.type.fields ? root.type.fields.length : 0
        delegate: ColumnLayout {
            id: row
            required property int index
            readonly property var field: root.type.fields[index]
            objectName: "plugin-row-" + field.name
            Layout.fillWidth: true; spacing: 2
            InspectorRow {
                Layout.fillWidth: true
                label: root.labelOf(row.field.name)
                PluginValueEditor {
                    Layout.fillWidth: true
                    app: root.app; ty: row.field; actors: root.actors
                    value: root.valueOf(row.field)
                    onEdited: next => root.changed(root.with_(row.field, next))
                }
            }
            Text {
                visible: !!row.field.description
                Layout.fillWidth: true; leftPadding: 84; wrapMode: Text.WordWrap
                text: row.field.description || ""; color: Theme.textDim; font.pixelSize: 11
            }
        }
    }
}
