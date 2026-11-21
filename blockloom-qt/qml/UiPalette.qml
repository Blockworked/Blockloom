import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Widget kinds grouped by purpose, searchable. Click adds into the selected
// container; dragging onto the viewport drops at the pointer.
ColumnLayout {
    id: root
    required property var designer
    property string search: ""
    spacing: 6
    readonly property var groups: [
        {name: "Display", kinds: ["Label", "RichText", "Image", "Progress", "RadialProgress", "Tooltip"]},
        {name: "Controls", kinds: ["Button", "Toggle", "Slider", "Input", "Select", "Tabs", "Scrollbar", "List", "ListView"]},
        {name: "Containers", kinds: ["Panel", "Canvas", "VerticalBox", "HorizontalBox", "Grid", "WrapBox", "SizeBox", "Spacer"]}
    ]
    // Shorter names for the tiles; the kind itself shows in the tooltip.
    readonly property var names: ({VerticalBox: "Column", HorizontalBox: "Row", WrapBox: "Wrap", SizeBox: "Size box", RichText: "Rich text",
        RadialProgress: "Radial", ListView: "List view", Scrollbar: "Scrollbar", Progress: "Progress"})
    readonly property var rows: {
        const needle = search.trim().toLowerCase(), out = [];
        groups.forEach(g => {
            const kinds = g.kinds.filter(k => !needle || k.toLowerCase().indexOf(needle) >= 0);
            if (!kinds.length) return;
            out.push({header: g.name});
            kinds.forEach(k => out.push({kind: k}));
        });
        return out;
    }
    StitchTextField {
        objectName: "interfacePaletteSearch"
        Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
        placeholderText: "Find a widget"
        text: root.search
        onTextEdited: root.search = text
        Accessible.name: "Find a widget"
    }
    Flow {
        Layout.fillWidth: true; spacing: 4
        Repeater {
            model: root.rows
            delegate: Loader {
                required property var modelData
                sourceComponent: modelData.header !== undefined ? headerPiece : chip
                onLoaded: item.entry = modelData
            }
        }
    }
    Component {
        id: headerPiece
        Text {
            property var entry: ({})
            width: root.width
            topPadding: 4
            text: entry.header ? entry.header.toUpperCase() : ""
            color: Theme.textDim; font.pixelSize: 10; font.weight: Font.Bold; font.letterSpacing: 1
        }
    }
    Component {
        id: chip
        Rectangle {
            id: tile
            property var entry: ({})
            readonly property string kind: entry.kind || ""
            width: Math.max(72, (root.width - 4) / 2); height: 28
            radius: 5
            color: drag.active ? Theme.accent : hover.hovered ? "#3b3c40" : Theme.panelRaised
            border.color: Theme.borderSoft
            HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
            ToolTip.visible: hover.hovered; ToolTip.delay: 600; ToolTip.text: "Add a " + kind + ". Click, or drag onto the viewport."
            TapHandler { onTapped: root.designer.add(tile.kind, 20, 20, false) }
            DragHandler {
                id: drag
                target: null
                onActiveChanged: if (!active) root.designer.dropKind(tile.kind, tile, centroid.position.x, centroid.position.y)
            }
            Accessible.role: Accessible.Button
            Accessible.name: "Add " + kind
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 8; anchors.rightMargin: 6; spacing: 6
                LucideIcon { name: root.designer.kindIcon(tile.kind); color: Theme.text; width: 14; height: 14 }
                Text { Layout.fillWidth: true; text: root.names[tile.kind] || tile.kind; color: Theme.text; font.pixelSize: 12; elide: Text.ElideRight }
            }
        }
    }
}
