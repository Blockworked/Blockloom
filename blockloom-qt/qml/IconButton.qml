import QtQuick
import QtQuick.Controls
import com.blockworked.Blockstitch 1.0

// A square icon-only button with a tooltip.
BwButton {
    id: control
    property string tip: ""
    text: ""
    flat: true
    implicitWidth: 32; implicitHeight: 32
    ToolTip.visible: tip.length > 0 && hovered
    ToolTip.delay: 500
    ToolTip.text: tip
}
