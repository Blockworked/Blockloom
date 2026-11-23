import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A form for one plugin record, drawn by the same RegistryForm as the
// built-in cards: `type` is an entry of the backend's `types` list (fields,
// defaults, inspector groups), `payload` what the record holds. Edits report
// the whole next payload, keeping keys the schema no longer names.
RegistryForm {
    id: root
    property var type: ({ fields: [], defaults: {} })
    property var payload: ({})
    plugin: type && type.plugin ? type.plugin : ""
    // A plugin schema keeps its defaults beside the fields; fold them in.
    fields: (type && type.fields ? type.fields : []).map(f => {
        const d = type.defaults ? type.defaults[f.name] : undefined;
        return f.default === undefined && d !== undefined ? Object.assign({}, f, { default: d }) : f;
    })
    groups: type && type.inspector && type.inspector.groups ? type.inspector.groups : []
    value: payload || ({})
}
