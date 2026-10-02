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

    // Fields in the order the inspector draws them: the ones no group names
    // first, then each group with its own heading.
    readonly property var sections: {
        const fields = root.type.fields || [];
        const groups = root.type.inspector && root.type.inspector.groups ? root.type.inspector.groups : [];
        const placed = {};
        const out = [];
        const named = [];
        for (const g of groups) {
            const members = [];
            for (const name of g.fields || []) {
                const i = fields.findIndex(f => f.name === name);
                if (i >= 0) { members.push(i); placed[name] = true; }
            }
            named.push({ label: g.label || "", collapsed: !!g.collapsed, members: members });
        }
        const loose = [];
        fields.forEach((f, i) => { if (!placed[f.name]) loose.push(i); });
        if (loose.length) out.push({ label: "", collapsed: false, members: loose });
        return out.concat(named);
    }
    // Which headings the user has flipped from how the schema starts them.
    property var flipped: ({})
    function isOpen(section) { return flipped[section.label] ? section.collapsed : !section.collapsed; }
    function flip(section) { const next = Object.assign({}, flipped); next[section.label] = !next[section.label]; flipped = next; }
    // Whether a field's `visible_when` holds against the record as it stands.
    function shown(field) {
        const when = field.ui ? field.ui.visible_when : null;
        if (!when) return true;
        const other = (root.type.fields || []).find(f => f.name === when.field);
        const now = other ? root.valueOf(other) : null;
        if (when.equals !== undefined && when.equals !== null) return JSON.stringify(now) === JSON.stringify(when.equals);
        if (when.not_equals !== undefined && when.not_equals !== null) return JSON.stringify(now) !== JSON.stringify(when.not_equals);
        if (Array.isArray(now)) return now.length > 0;
        return !!now;
    }

    Repeater {
        model: root.sections
        delegate: ColumnLayout {
            id: section
            required property var modelData
            readonly property bool open: root.isOpen(section.modelData)
            Layout.fillWidth: true; spacing: 6
            objectName: "plugin-section-" + section.modelData.label
            MouseArea {
                visible: section.modelData.label !== ""
                Layout.fillWidth: true; implicitHeight: visible ? 22 : 0
                cursorShape: Qt.PointingHandCursor
                onClicked: root.flip(section.modelData)
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: (section.open ? "\u25BE " : "\u25B8 ") + section.modelData.label
                    color: Theme.textDim; font.pixelSize: 11; font.bold: true
                }
            }
            Repeater {
                model: section.open ? section.modelData.members : []
                delegate: ColumnLayout {
                    id: row
                    required property int modelData
                    readonly property var field: root.type.fields[row.modelData]
                    objectName: "plugin-row-" + field.name
                    visible: root.shown(row.field)
                    Layout.fillWidth: true; spacing: 2
                    InspectorRow {
                        Layout.fillWidth: true
                        label: row.field.ui && row.field.ui.label ? row.field.ui.label : root.labelOf(row.field.name)
                        RowLayout {
                            Layout.fillWidth: true; spacing: 4
                            PluginValueEditor {
                                Layout.fillWidth: true
                                app: root.app; ty: row.field; actors: root.actors
                                value: root.valueOf(row.field)
                                onEdited: next => root.changed(root.with_(row.field, next))
                            }
                            Text {
                                visible: !!(row.field.ui && row.field.ui.unit)
                                text: row.field.ui ? row.field.ui.unit : ""; color: Theme.textDim; font.pixelSize: 11
                            }
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
    }
}
