import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Layout of the selected widget. Only the fields its kind uses are shown:
// size rules always, spacing for containers, columns for grids.
ColumnLayout {
    id: root
    property var layoutValue: null
    property string kind: ""
    signal edited(var value)
    readonly property var defaults: ({width: "Auto", height: "Auto", min_size: [0,0], max_size: [0,0], margin: [0,0,0,0], padding: [0,0,0,0], gap: 8, columns: 2, grow: 0, align: "Stretch", absolute: false, row_height: 32})
    readonly property var effective: Object.assign({}, defaults, layoutValue || {})
    readonly property bool container: ["Panel", "VerticalBox", "HorizontalBox", "Grid", "WrapBox", "Canvas", "List", "ListView", "SizeBox"].indexOf(kind) >= 0
    readonly property bool flowContainer: ["Panel", "VerticalBox", "HorizontalBox", "Grid", "WrapBox"].indexOf(kind) >= 0
    spacing: 6
    function setField(field, value) {
        const next = JSON.parse(JSON.stringify(effective));
        next[field] = value;
        edited(next);
    }
    function lengthMode(field) {
        const value = effective[field];
        return typeof value === "string" ? "Auto" : Object.keys(value)[0];
    }
    RowLayout {
        Layout.fillWidth: true; spacing: 8
        StitchCheckBox {
            objectName: "interfaceExplicitLayout"
            text: "Explicit layout"
            checked: root.layoutValue !== null
            onToggled: root.edited(checked ? JSON.parse(JSON.stringify(root.defaults)) : null)
        }
    }
    Text {
        Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
        text: root.layoutValue === null ? "Sizing follows the widget's legacy size. Turn on explicit layout for percent sizes, limits and spacing."
                                        : "Auto keeps the legacy size. A maximum of 0 means no limit. Percent is of the parent."
    }
    ColumnLayout {
        Layout.fillWidth: true
        visible: root.layoutValue !== null
        spacing: 6
        Repeater {
            model: ["width", "height"]
            delegate: InspectorRow {
                id: dimension
                required property string modelData
                label: modelData === "width" ? "Width" : "Height"; Layout.fillWidth: true
                StitchComboBox {
                    id: unit
                    implicitWidth: 92; implicitHeight: 30; font.pixelSize: 12
                    model: ["Auto", "Px", "Percent"]
                    currentIndex: model.indexOf(root.lengthMode(dimension.modelData))
                    onActivated: {
                        if (currentText === "Auto") root.setField(dimension.modelData, "Auto");
                        else { const v = {}; v[currentText] = 100; root.setField(dimension.modelData, v); }
                    }
                }
                NumberField {
                    enabled: unit.currentText !== "Auto"
                    value: root.lengthMode(dimension.modelData) === "Auto" ? "" : root.effective[dimension.modelData][root.lengthMode(dimension.modelData)]
                    onCommitted: n => { const v = {}; v[unit.currentText] = Math.max(0, n); root.setField(dimension.modelData, v); }
                }
            }
        }
        InspectorRow {
            visible: root.flowContainer; label: "Gap"; Layout.fillWidth: true
            NumberField { value: root.effective.gap; onCommitted: n => root.setField("gap", Math.max(0, n)) }
        }
        InspectorRow {
            visible: root.kind === "Grid"; label: "Columns"; Layout.fillWidth: true
            NumberField { value: root.effective.columns; onCommitted: n => root.setField("columns", Math.max(1, Math.round(n))) }
        }
        InspectorRow {
            visible: root.kind === "ListView"; label: "Row height"; Layout.fillWidth: true
            NumberField { value: root.effective.row_height; onCommitted: n => root.setField("row_height", Math.max(1, n)) }
        }
        InspectorRow {
            label: "Grow"; Layout.fillWidth: true
            NumberField { value: root.effective.grow; onCommitted: n => root.setField("grow", Math.max(0, n)) }
        }
        InspectorRow {
            visible: root.container; label: "Children"; Layout.fillWidth: true
            ChoiceField {
                options: ["Stretch", "Start", "Center", "End"].map(v => ({value: v, label: v}))
                value: root.effective.align
                onChosen: v => root.setField("align", v)
            }
        }
        StitchCheckBox { text: "Absolute placement"; checked: root.effective.absolute; onToggled: root.setField("absolute", checked) }
        Repeater {
            model: [
                {key: "min_size", title: "Minimum size", labels: ["W", "H"], low: 0},
                {key: "max_size", title: "Maximum size", labels: ["W", "H"], low: 0},
                {key: "padding", title: "Padding", labels: ["L", "T", "R", "B"], low: 0},
                {key: "margin", title: "Margin", labels: ["L", "T", "R", "B"], low: -100000}
            ]
            delegate: ColumnLayout {
                id: edges
                required property var modelData
                visible: modelData.key !== "padding" || root.container
                Layout.fillWidth: true; spacing: 2
                Text { text: edges.modelData.title; color: Theme.textDim; font.pixelSize: 11 }
                RowLayout {
                    Layout.fillWidth: true; spacing: 4
                    Repeater {
                        model: edges.modelData.labels
                        delegate: RowLayout {
                            required property string modelData
                            required property int index
                            Layout.fillWidth: true; spacing: 3
                            Text { text: modelData; color: Theme.textDim; font.pixelSize: 10 }
                            NumberField {
                                value: root.effective[edges.modelData.key][index]
                                onCommitted: n => {
                                    const values = root.effective[edges.modelData.key].slice();
                                    values[index] = Math.max(edges.modelData.low, n);
                                    root.setField(edges.modelData.key, values);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
