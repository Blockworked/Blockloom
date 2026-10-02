import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Edits one value of a plugin schema type: `ty` is a field type as the
// backend lists it ({type: "int", min, max} and so on). It only asks: it shows
// `value` and reports `edited(next)`, leaving the document to say what is held.
ColumnLayout {
    id: root
    required property var app
    property var ty: ({ type: "text" })
    property var value: null
    // Actors an `actor` value can name, as [{value, label}].
    property var actors: []
    signal edited(var next)
    Layout.fillWidth: true
    spacing: 4

    // What a new list item of this type starts as.
    function zeroOf(t) {
        switch (t.type) {
        case "bool": return false;
        case "int": case "number": return t.min !== undefined && t.min !== null && t.min > 0 ? t.min : 0;
        case "color": return "#FFFFFF";
        case "vec3": return [0, 0, 0];
        case "choice": return t.options && t.options.length ? t.options[0] : "";
        case "list": return [];
        default: return "";
        }
    }
    function clamp(n, lo, hi) {
        let v = n;
        if (lo !== undefined && lo !== null) v = Math.max(lo, v);
        if (hi !== undefined && hi !== null) v = Math.min(hi, v);
        return v;
    }
    // The asset kinds the tray names differ from the schema's for sounds.
    function acceptOf(kind) { return kind === "any" || !kind ? [] : [kind === "sound" ? "audio" : kind]; }
    function choices(t) { return (t.options || []).map(o => ({ value: o, label: o })); }
    function vec(v) { return Array.isArray(v) && v.length === 3 ? v : [0, 0, 0]; }
    function withComponent(v, i, n) { const next = vec(v).slice(); next[i] = n; return next; }
    // A color keeps the alpha pair it had when only the swatch is picked.
    function recolor(old, picked) { return String(old).length === 9 ? picked + String(old).slice(7) : picked; }
    readonly property string kind: ty && ty.type ? ty.type : "text"
    readonly property var items: kind === "list" && Array.isArray(value) ? value : []

    Loader {
        Layout.fillWidth: true
        visible: root.kind !== "list"
        sourceComponent: ({ bool: boolEditor, int: intEditor, number: numberEditor, text: textEditor, color: colorEditor,
                            vec3: vec3Editor, choice: choiceEditor, asset: assetEditor, actor: actorEditor })[root.kind] || null
    }
    Component { id: boolEditor
        RowLayout { SwitchField { value: root.value === true; onToggled: on => root.edited(on) } Item { Layout.fillWidth: true } } }
    Component { id: intEditor
        NumberField { value: root.value; onCommitted: n => root.edited(root.clamp(Math.round(n), root.ty.min, root.ty.max)) } }
    Component { id: numberEditor
        NumberField { value: root.value; onCommitted: n => root.edited(root.clamp(n, root.ty.min, root.ty.max)) } }
    Component { id: textEditor
        BwTextField {
            Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
            maximumLength: root.ty.max_len !== undefined && root.ty.max_len !== null ? root.ty.max_len : 32767
            text: root.value === null || root.value === undefined ? "" : String(root.value)
            onEditingFinished: if (text !== String(root.value)) root.edited(text)
        } }
    Component { id: colorEditor
        RowLayout {
            ColorField { value: String(root.value || "#FFFFFF").slice(0, 7); onPicked: c => root.edited(root.recolor(root.value || "", c)) }
            Item { Layout.fillWidth: true }
        } }
    Component { id: vec3Editor
        RowLayout {
            spacing: 4
            Repeater {
                model: 3
                delegate: NumberField { required property int index; value: root.vec(root.value)[index]; onCommitted: n => root.edited(root.withComponent(root.value, index, n)) }
            }
        } }
    Component { id: choiceEditor
        ChoiceField { options: root.choices(root.ty); value: root.value; onChosen: v => root.edited(v) } }
    Component { id: assetEditor
        AssetField { app: root.app; accept: root.acceptOf(root.ty.kind); value: String(root.value || ""); onCommitted: p => root.edited(p) } }
    Component { id: actorEditor
        ChoiceField { options: root.actors; value: root.value || ""; placeholder: "nothing"; onChosen: v => root.edited(v) } }

    // A list is its items, each edited as its own value, and a way to add one.
    Repeater {
        model: root.kind === "list" ? root.items.length : 0
        delegate: RowLayout {
            required property int index
            Layout.fillWidth: true; spacing: 4
            PluginValueEditor {
                Layout.fillWidth: true
                app: root.app; ty: root.ty.item || ({ type: "text" }); actors: root.actors
                value: root.items[index]
                onEdited: next => { const list = root.items.slice(); list[index] = next; root.edited(list); }
            }
            IconButton { iconName: "x"; tip: "Remove this item"; implicitWidth: 24; implicitHeight: 24
                onClicked: { const list = root.items.slice(); list.splice(index, 1); root.edited(list); } }
        }
    }
    BwButton {
        visible: root.kind === "list" && (root.ty.max_len === undefined || root.ty.max_len === null || root.items.length < root.ty.max_len)
        iconName: "plus"; text: "Add item"; implicitHeight: 28; font.pixelSize: 12
        onClicked: root.edited(root.items.concat([root.zeroOf(root.ty.item || ({ type: "text" }))]))
    }
}
