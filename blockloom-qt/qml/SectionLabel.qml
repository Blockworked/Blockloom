import QtQuick
import com.blockworked.Blockstitch 1.0

// A group heading in a panel, as the palette writes its section names.
Text {
    property string label: ""
    text: label
    color: Theme.textDim
    font.pixelSize: 13; font.weight: Font.DemiBold
    topPadding: 8
}
