import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: root
    property var layoutValue: null
    signal edited(var value)
    readonly property var defaults: ({width: "Auto", height: "Auto", min_size: [0,0], max_size: [0,0], margin: [0,0,0,0], padding: [0,0,0,0], gap: 8, columns: 2, grow: 0, align: "Stretch", absolute: false, row_height: 32})
    readonly property var effective: Object.assign({}, defaults, layoutValue || {})
    function setField(field, value) {
        const next = JSON.parse(JSON.stringify(effective));
        next[field] = value;
        edited(next);
    }
    function lengthMode(field) {
        const value = effective[field];
        return typeof value === "string" ? "Auto" : Object.keys(value)[0];
    }
    Label { text: "Layout"; font.bold: true }
    CheckBox {
        objectName: "interfaceExplicitLayout"
        text: "Explicit layout"
        checked: root.layoutValue !== null
        onToggled: root.edited(checked ? JSON.parse(JSON.stringify(root.defaults)) : null)
    }
    Label {
        Layout.fillWidth: true; wrapMode: Text.Wrap
        text: root.layoutValue === null ? "Legacy sizing. Enable explicit layout to edit flow dimensions and spacing." : "Auto retains legacy sizing. Max 0 means no limit. Percent uses the parent size."
    }
    ColumnLayout {
        Layout.fillWidth: true
        enabled: root.layoutValue !== null
        Repeater {
            model: ["width", "height"]
            delegate: RowLayout {
                required property string modelData
                Label { text: modelData; Layout.preferredWidth: 50 }
                ComboBox {
                    id: unit
                    model: ["Auto", "Px", "Percent"]
                    currentIndex: model.indexOf(root.lengthMode(modelData))
                    onActivated: {
                        if (currentText === "Auto") root.setField(modelData, "Auto");
                        else { const v = {}; v[currentText] = 100; root.setField(modelData, v); }
                    }
                }
                TextField {
                    Layout.fillWidth: true
                    enabled: unit.currentText !== "Auto"
                    validator: DoubleValidator { bottom: 0 }
                    text: root.lengthMode(modelData) === "Auto" ? "" : root.effective[modelData][root.lengthMode(modelData)]
                    onEditingFinished: { const v = {}; v[unit.currentText] = Number(text); root.setField(modelData, v); }
                }
            }
        }
        Repeater {
            model: ["gap", "grow", "columns", "row_height"]
            delegate: RowLayout {
                required property string modelData
                Label { text: modelData.replace(/_/g, " "); Layout.preferredWidth: 100 }
                TextField {
                    Layout.fillWidth: true
                    validator: DoubleValidator { bottom: modelData === "columns" || modelData === "row_height" ? 1 : 0 }
                    text: root.effective[modelData]
                    onEditingFinished: root.setField(modelData, Number(text))
                }
            }
        }
        Label { text: "Child alignment" }
        ComboBox {
            Layout.fillWidth: true; model: ["Stretch", "Start", "Center", "End"]
            currentIndex: model.indexOf(root.effective.align)
            onActivated: root.setField("align", currentText)
        }
        CheckBox { text: "Absolute placement"; checked: root.effective.absolute; onToggled: root.setField("absolute", checked) }
        Repeater {
            model: ["min_size", "max_size", "padding", "margin"]
            delegate: ColumnLayout {
                id: edges
                required property string modelData
                Layout.fillWidth: true
                Label { text: edges.modelData.replace(/_/g, " ") }
                Repeater {
                    model: edges.modelData.indexOf("size") >= 0 ? ["Width", "Height"] : ["Left", "Top", "Right", "Bottom"]
                    delegate: RowLayout {
                        required property string modelData
                        required property int index
                        Label { text: modelData; Layout.preferredWidth: 60 }
                        TextField {
                            Layout.fillWidth: true
                            validator: DoubleValidator { bottom: edges.modelData === "margin" ? -Infinity : 0 }
                            text: root.effective[edges.modelData][index]
                            onEditingFinished: {
                                const values = root.effective[edges.modelData].slice();
                                values[index] = Number(text);
                                root.setField(edges.modelData, values);
                            }
                        }
                    }
                }
            }
        }
    }
}
