import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A foldable inspector section. Fold state is kept per key on the designer,
// so it survives selecting another widget.
ColumnLayout {
    id: root
    required property var designer
    property string title: ""
    property string key: title.toLowerCase()
    property string badge: ""
    default property alias content: body.data
    readonly property bool open: !designer.folded[key]
    Layout.fillWidth: true
    spacing: 0
    Rectangle {
        Layout.fillWidth: true; implicitHeight: 30
        color: hover.hovered ? Theme.panelRaised : "transparent"
        Rectangle { anchors.top: parent.top; width: parent.width; height: 1; color: Theme.borderSoft }
        RowLayout {
            anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 10; spacing: 6
            LucideIcon { name: root.open ? "chevron-down" : "chevron-right"; color: Theme.textDim; width: 14; height: 14 }
            Text { Layout.fillWidth: true; text: root.title; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold; elide: Text.ElideRight }
            Text { visible: root.badge !== ""; text: root.badge; color: Theme.textDim; font.pixelSize: 11 }
        }
        HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
        TapHandler { onTapped: root.designer.toggleFold(root.key) }
        Accessible.role: Accessible.Button
        Accessible.name: root.title
    }
    ColumnLayout {
        id: body
        visible: root.open
        Layout.fillWidth: true; Layout.leftMargin: 10; Layout.rightMargin: 10; Layout.topMargin: 4; Layout.bottomMargin: 10
        spacing: 6
    }
}
