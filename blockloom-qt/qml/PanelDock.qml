import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A dock region whose panels come from the editor page's layout: each panel
// exists once in the page and is borrowed by the region it is placed in, so
// a panel moves between regions without losing its state.
DockRegion {
    id: region
    required property var page
    readonly property var ids: page.idsAt(edge)
    tabs: ids.map(id => page.tabOf(id))
    currentIndex: Math.max(0, ids.indexOf(page.currentAt(edge)))
    onTabRequested: i => page.setCurrent(edge, ids[i])
    trailing: Component {
        IconButton {
            objectName: "dock-move-" + region.edge
            iconName: "move"; tip: "Move this panel"
            implicitWidth: 24; implicitHeight: 24
            onClicked: moveMenu.popup(this, 0, height)
            StitchMenu {
                id: moveMenu
                Repeater {
                    model: [{ edge: "left", label: "Dock on the left" }, { edge: "right", label: "Dock on the right" }, { edge: "bottom", label: "Dock at the bottom" }]
                    delegate: StitchMenuItem {
                        required property var modelData
                        objectName: "dock-move-to-" + modelData.edge
                        visible: modelData.edge !== region.edge
                        height: visible ? implicitHeight : 0
                        text: modelData.label
                        onTriggered: region.page.movePanel(region.ids[region.currentIndex], modelData.edge)
                    }
                }
            }
        }
    }

    Repeater {
        model: region.ids
        delegate: Item {
            id: holder
            required property string modelData
            readonly property var panel: region.page.panelItem(modelData)
            Binding { target: holder.panel; property: "parent"; value: holder; when: holder.panel !== null; restoreMode: Binding.RestoreNone }
            Binding { target: holder.panel; property: "width"; value: holder.width; when: holder.panel !== null; restoreMode: Binding.RestoreNone }
            Binding { target: holder.panel; property: "height"; value: holder.height; when: holder.panel !== null; restoreMode: Binding.RestoreNone }
        }
    }
}
