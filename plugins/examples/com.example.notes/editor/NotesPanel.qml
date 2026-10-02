import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// A trusted editor screen: the project's notes, saved through the plugin's
// own command. `host` is handed in by the editor.
Item {
    id: root
    property var host
    readonly property string saved: host ? String(host.resource("notes").text || "") : ""

    ColumnLayout {
        anchors.fill: parent
        spacing: 8
        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            TextArea {
                id: area
                wrapMode: TextArea.Wrap
                placeholderText: "Notes about this project"
                text: root.saved
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                text: "Save"
                enabled: area.text !== root.saved
                onClicked: root.host.call("set_notes", { value: area.text })
            }
            Label {
                text: area.text === root.saved ? "Saved" : "Unsaved changes"
                opacity: 0.6
            }
        }
    }
}
