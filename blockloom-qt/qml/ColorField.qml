import QtQuick
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A swatch that opens a color picker; `picked` gives "#RRGGBB" in capitals.
Rectangle {
    id: root
    property string value: "#FFFFFF"
    signal picked(string color)
    Layout.preferredWidth: 46; Layout.preferredHeight: 26
    implicitWidth: 46; implicitHeight: 26
    radius: 5; color: value; border.color: hover.hovered ? Theme.accent : Theme.border
    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { onTapped: { dialog.selectedColor = root.value; dialog.open(); } }
    ColorDialog {
        id: dialog
        onAccepted: {
            const c = selectedColor;
            const hex = n => ("0" + Math.round(n * 255).toString(16)).slice(-2);
            root.picked(("#" + hex(c.r) + hex(c.g) + hex(c.b)).toUpperCase());
        }
    }
}
