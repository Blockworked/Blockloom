import QtQuick

Item {
    property string interfaceLayout: ""
    property size resolution: Qt.size(0, 0)
    property bool hasFrame: false
    property string error: ""
    property bool pointerLocked: false
    property bool pointerHeld: false
    signal pointerMoved(real dx, real dy)
    signal pointerReleased()
}
