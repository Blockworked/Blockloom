import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A slider drawn in the app's theme.
Slider {
    id: control
    Layout.fillWidth: true
    implicitHeight: 26
    HoverHandler { cursorShape: Qt.PointingHandCursor }
    background: Rectangle {
        x: control.leftPadding; y: control.topPadding + control.availableHeight / 2 - height / 2
        width: control.availableWidth; height: 5; radius: 3; color: "#45474d"
        Rectangle { width: control.visualPosition * parent.width; height: parent.height; radius: 3; color: Theme.accent }
    }
    handle: Rectangle {
        x: control.leftPadding + control.visualPosition * (control.availableWidth - width)
        y: control.topPadding + control.availableHeight / 2 - height / 2
        width: 16; height: 16; radius: 8; color: "white"; border.width: 2; border.color: Theme.accent
    }
}
