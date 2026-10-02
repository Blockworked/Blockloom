import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// A trusted inspector section for the sticky component: a note drawn on its
// own colour. `host` is handed in by the editor (`payload` is the record,
// `write(next)` saves the whole next payload).
ColumnLayout {
    id: root
    property var host
    readonly property var payload: host ? host.payload : ({})
    readonly property string tint: payload && payload.color ? payload.color : "#ffe27a"
    readonly property var swatches: ["#ffe27a", "#9fe3a1", "#8fc8ff", "#ffa8a8"]
    spacing: 6

    function save(key, value) {
        const next = Object.assign({}, root.payload || {});
        next[key] = value;
        root.host.write(next);
    }

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: 76
        radius: 4
        color: root.tint
        TextArea {
            id: area
            anchors.fill: parent
            wrapMode: TextArea.Wrap
            color: "#222222"
            placeholderText: "What is this for?"
            text: root.payload && root.payload.text ? root.payload.text : ""
            onEditingFinished: if (text !== (root.payload.text || "")) root.save("text", text)
        }
    }
    RowLayout {
        spacing: 6
        Repeater {
            model: root.swatches
            delegate: Rectangle {
                required property string modelData
                width: 22; height: 22; radius: 11
                color: modelData
                border.width: modelData === root.tint ? 2 : 0
                border.color: "#ffffff"
                MouseArea { anchors.fill: parent; onClicked: root.save("color", parent.modelData) }
            }
        }
    }
}
