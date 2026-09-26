import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// An HDR color: a base color plus an intensity multiplier. The swatch always
// shows the base color, whatever the intensity, and the badge says how many
// stops over paper white it is pushed. `picked` gives "#RRGGBB" and the
// multiplier, 0 for off.
RowLayout {
    id: root
    property string color: "#FFFFFF"
    property real intensity: 1
    property string hint: "The swatch keeps the color itself. Intensity multiplies it past paper white: at 1 and up it blooms, and every stop doubles it."
    signal picked(string color, real intensity)
    spacing: 4

    readonly property real minStops: -4
    readonly property real maxStops: 14
    function stopsOf(n) { return n > 0 ? Math.log(n) / Math.LN2 : minStops; }
    function badge(n) {
        if (!(n > 0)) return "off";
        const ev = stopsOf(n);
        return "×" + Number(n.toPrecision(3)) + "  " + (ev >= 0 ? "+" : "") + ev.toFixed(1) + " EV";
    }

    Rectangle {
        Layout.preferredWidth: 46; Layout.preferredHeight: 26
        radius: 5; color: root.color; border.color: hover.hovered ? Theme.accent : Theme.border
        // Struck through while off, so a dark swatch isn't mistaken for a dim glow.
        Rectangle {
            visible: !(root.intensity > 0)
            anchors.centerIn: parent; width: parent.width * 1.05; height: 2; rotation: -30; color: Theme.danger
        }
        HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
        TapHandler { onTapped: popup.open() }
    }
    Text {
        Layout.fillWidth: true
        text: root.badge(root.intensity); color: Theme.textDim; font.pixelSize: 11; elide: Text.ElideRight
        TapHandler { onTapped: popup.open() }
    }

    Popup {
        id: popup
        y: 30
        width: 260; padding: 10
        background: Rectangle { color: Theme.panel; border.color: Theme.border; radius: 6 }
        onOpened: stops.value = root.stopsOf(root.intensity)
        ColumnLayout {
            width: parent.width; spacing: 6
            RowLayout {
                Text { text: "Color"; color: Theme.textDim; font.pixelSize: 12; Layout.preferredWidth: 60 }
                ColorField { value: root.color; onPicked: c => root.picked(c, root.intensity) }
                Item { Layout.fillWidth: true }
            }
            RowLayout {
                Text { text: "Intensity"; color: Theme.textDim; font.pixelSize: 12; Layout.preferredWidth: 60 }
                NumberField {
                    value: root.intensity; fallback: 0
                    onCommitted: n => root.picked(root.color, Math.max(0, n))
                }
            }
            // Stops, since glow reads in doublings; the far left is off.
            SliderField {
                id: stops
                from: root.minStops; to: root.maxStops; stepSize: 0.1
                onPressedChanged: if (!pressed) root.picked(root.color, value <= root.minStops ? 0 : Math.pow(2, value))
            }
            RowLayout {
                Text { text: "off"; color: Theme.textDim; font.pixelSize: 10 }
                Item { Layout.fillWidth: true }
                Text { text: (stops.value <= root.minStops ? "off" : (stops.value >= 0 ? "+" : "") + stops.value.toFixed(1) + " EV"); color: Theme.text; font.pixelSize: 11 }
                Item { Layout.fillWidth: true }
                Text { text: "+" + root.maxStops + " EV"; color: Theme.textDim; font.pixelSize: 10 }
            }
            Text {
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: root.hint
            }
        }
    }
}
