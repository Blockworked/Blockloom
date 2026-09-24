import QtQuick
import com.blockworked.Blockstitch 1.0

// An uppercase group heading, as the sidebar and panels use.
Text {
    property string label: ""
    text: label.toUpperCase()
    color: Theme.textDim
    font.pixelSize: 10; font.weight: Font.Bold; font.letterSpacing: 1
    topPadding: 8
}
