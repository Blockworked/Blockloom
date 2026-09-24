import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// An on/off switch that only asks: it shows `value` and reports a flip,
// leaving the document to say what it is now.
Rectangle {
    id: root
    property bool value: false
    signal toggled(bool on)
    implicitWidth: 38; implicitHeight: 21; radius: 11
    Layout.alignment: Qt.AlignVCenter
    color: value ? Theme.accent : Theme.panelRaised
    border.color: value ? Theme.accentHover : Theme.border
    Rectangle {
        x: root.value ? 19 : 2; y: 2
        width: 17; height: 17; radius: 9; color: "#f4f4f5"
        Behavior on x { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
    }
    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.toggled(!root.value) }
}
