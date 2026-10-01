import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

ColumnLayout {
    id: root
    property string heading: ""
    property bool removable: false
    signal removeRequested()
    default property alias content: body.data
    Layout.fillWidth: true; spacing: 6
    Rectangle { Layout.fillWidth: true; Layout.topMargin: 6; height: 1; color: Theme.borderSoft }
    RowLayout {
        Layout.fillWidth: true
        Text { Layout.fillWidth: true; text: root.heading; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
        IconButton { visible: root.removable; iconName: "x"; tip: "Remove the " + root.heading + " component";
            implicitWidth: 24; implicitHeight: 24; onClicked: root.removeRequested() }
    }
    ColumnLayout { id: body; Layout.fillWidth: true; spacing: 6 }
}
