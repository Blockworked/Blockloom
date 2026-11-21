import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The widget tree: kind icon and id per row, with editor-only hide and lock.
// Arrow keys move the selection and fold; Delete removes it.
ColumnLayout {
    id: root
    required property var designer
    spacing: 6
    StitchTextField {
        objectName: "interfaceSearch"
        Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
        placeholderText: "Search widgets"
        text: root.designer.search
        onTextEdited: root.designer.search = text
        Accessible.name: "Search widgets"
    }
    ListView {
        id: tree
        objectName: "interfaceTree"
        Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumHeight: 80
        clip: true
        model: root.designer.treeRows
        activeFocusOnTab: true
        keyNavigationEnabled: false
        property string renamingId: ""
        Accessible.role: Accessible.Tree
        Accessible.name: "Widget hierarchy"
        ScrollBar.vertical: ScrollBar {}
        Text {
            visible: tree.count === 0
            anchors.centerIn: parent; width: parent.width - 24
            horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap
            color: Theme.textDim; font.pixelSize: 12
            text: root.designer.search ? "No widget matches." : "No widgets yet. Add one from the palette."
        }
        function rowIndex() {
            const rows = root.designer.treeRows;
            for (let i = 0; i < rows.length; ++i) if (rows[i].id === root.designer.selectedId) return i;
            return -1;
        }
        Keys.onPressed: event => {
            const rows = root.designer.treeRows, at = rowIndex();
            const row = at >= 0 ? rows[at] : null;
            if (event.key === Qt.Key_Down) { root.designer.selectOnly(rows[Math.min(rows.length - 1, at + 1)].id); positionViewAtIndex(Math.min(rows.length - 1, at + 1), ListView.Contain); }
            else if (event.key === Qt.Key_Up && rows.length) { root.designer.selectOnly(rows[Math.max(0, at - 1)].id); positionViewAtIndex(Math.max(0, at - 1), ListView.Contain); }
            else if (event.key === Qt.Key_Right && row && row.kids && row.collapsed) root.designer.toggleCollapsed(row.id);
            else if (event.key === Qt.Key_Left && row && row.kids && !row.collapsed) root.designer.toggleCollapsed(row.id);
            else if (event.key === Qt.Key_Delete && row) root.designer.removeSelected();
            else if (event.key === Qt.Key_F2 && row && root.designer.editableId(row.id)) tree.renamingId = row.id;
            else { event.accepted = false; return; }
            event.accepted = true;
        }
        delegate: Rectangle {
            id: rowItem
            required property var modelData
            readonly property bool selected: root.designer.selectionIds.indexOf(modelData.id) >= 0
            readonly property bool hidden: !!root.designer.hiddenIds[modelData.id]
            readonly property bool locked: root.designer.isLocked(modelData.id)
            readonly property bool renaming: tree.renamingId === modelData.id
            width: ListView.view.width; height: 28
            color: selected ? "#1c3a57" : rowHover.hovered ? Theme.panelRaised : "transparent"
            radius: 4
            HoverHandler { id: rowHover }
            TapHandler {
                onDoubleTapped: if (root.designer.editableId(rowItem.modelData.id)) { root.designer.selectOnly(rowItem.modelData.id); tree.renamingId = rowItem.modelData.id; }
            }
            TapHandler {
                onTapped: {
                    if (Qt.application.keyboardModifiers & (Qt.ControlModifier | Qt.ShiftModifier)) root.designer.toggleSelected(rowItem.modelData.id);
                    else root.designer.selectOnly(rowItem.modelData.id);
                    tree.forceActiveFocus();
                }
            }
            Accessible.role: Accessible.TreeItem
            Accessible.name: modelData.id + " (" + modelData.kind + ")"
            Accessible.description: "Level " + (modelData.depth + 1) + (modelData.kids ? (modelData.collapsed ? ", collapsed" : ", expanded") : "")
            Accessible.selected: selected
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 4; anchors.rightMargin: 4; spacing: 4
                Item { Layout.preferredWidth: modelData.depth * 14 }
                Item {
                    objectName: "interfaceFold"
                    Layout.preferredWidth: 18; Layout.preferredHeight: 24
                    LucideIcon {
                        anchors.centerIn: parent; width: 12; height: 12
                        visible: modelData.kids > 0
                        name: modelData.collapsed ? "chevron-right" : "chevron-down"; color: Theme.textDim
                    }
                    TapHandler { enabled: modelData.kids > 0; onTapped: root.designer.toggleCollapsed(rowItem.modelData.id) }
                }
                LucideIcon { name: root.designer.kindIcon(modelData.kind); width: 14; height: 14; color: Theme.textDim; opacity: rowItem.hidden ? 0.4 : 1 }
                Text {
                    visible: !rowItem.renaming
                    Layout.fillWidth: true; elide: Text.ElideRight
                    text: modelData.id; font.pixelSize: 12
                    color: Theme.text; opacity: rowItem.hidden ? 0.4 : (rowItem.locked ? 0.7 : 1)
                }
                StitchTextField {
                    id: renameField
                    objectName: "interfaceRename"
                    visible: rowItem.renaming
                    Layout.fillWidth: true; implicitHeight: 24; font.pixelSize: 12
                    text: rowItem.modelData.id
                    onVisibleChanged: if (visible) { text = rowItem.modelData.id; forceActiveFocus(); selectAll(); }
                    onAccepted: { const id = rowItem.modelData.id, next = text; tree.renamingId = ""; root.designer.renameWidget(id, next); tree.forceActiveFocus(); }
                    Keys.onEscapePressed: { tree.renamingId = ""; tree.forceActiveFocus(); }
                    onActiveFocusChanged: if (!activeFocus && rowItem.renaming) tree.renamingId = ""
                    Accessible.name: "New id for " + rowItem.modelData.id
                }
                IconButton {
                    objectName: "interfaceHide"
                    implicitWidth: 24; implicitHeight: 24
                    iconName: "eye"; opacity: rowItem.hidden ? 0.35 : (rowHover.hovered ? 1 : 0.55)
                    tip: rowItem.hidden ? "Show in the preview (editor only)" : "Hide in the preview (editor only)"
                    Accessible.name: tip
                    onClicked: root.designer.toggleHidden(rowItem.modelData.id)
                }
                IconButton {
                    objectName: "interfaceLock"
                    implicitWidth: 24; implicitHeight: 24
                    iconName: rowItem.locked ? "lock" : "lock-open"
                    opacity: rowItem.locked ? 1 : (rowHover.hovered ? 0.6 : 0.2)
                    tip: rowItem.locked ? "Unlock (editor only)" : "Lock against viewport edits (editor only)"
                    Accessible.name: tip
                    onClicked: root.designer.toggleLock(rowItem.modelData.id)
                }
            }
        }
    }
}
