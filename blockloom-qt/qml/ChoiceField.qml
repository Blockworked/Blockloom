import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A dropdown over [{value, label}] that reports the chosen value and keeps
// showing whatever `value` says, so an undo or a refused change shows too.
BwComboBox {
    id: root
    property var options: []
    property var value: ""
    property string placeholder: ""
    signal chosen(var value)
    function indexOfValue() { for (let i = 0; i < options.length; ++i) if (String(options[i].value) === String(value)) return i; return -1; }
    Layout.fillWidth: true
    implicitHeight: 30; font.pixelSize: 12
    model: options; textRole: "label"
    currentIndex: indexOfValue()
    displayText: currentIndex >= 0 && options[currentIndex] ? options[currentIndex].label : placeholder
    onActivated: index => {
        const picked = options[index].value;
        currentIndex = Qt.binding(() => indexOfValue());
        root.chosen(picked);
    }
}
