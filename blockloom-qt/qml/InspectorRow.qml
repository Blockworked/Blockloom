import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A label on the left, and whatever edits it on the right.
RowLayout {
    id: root
    property string label: ""
    property real labelWidth: 78
    default property alias content: body.data
    spacing: 6
    Text {
        Layout.preferredWidth: root.labelWidth; Layout.alignment: Qt.AlignVCenter
        text: root.label; color: Theme.textDim; font.pixelSize: 12; elide: Text.ElideRight
    }
    RowLayout { id: body; Layout.fillWidth: true; spacing: 4 }
}
