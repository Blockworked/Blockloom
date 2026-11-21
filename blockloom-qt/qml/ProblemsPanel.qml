import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// What stops Play or Build, and what will surprise later: plugin preflight
// and audit, script errors, stale imports and asset warnings, from `problems`.
Rectangle {
    id: root
    required property var app
    property var problems: []
    readonly property int errors: problems.filter(p => p.severity === "error").length
    readonly property int warnings: problems.length - errors
    signal openScriptRequested(string path, int line)
    color: Theme.panel

    function refresh() {
        if (!root.app.appState.project) { root.problems = []; return; }
        root.app.invoke("problems", {}, list => root.problems = list || [], () => {});
    }
    // The snapshot changes on every edit; ask again once it settles.
    Connections {
        target: root.app
        function onAppStateChanged() { settle.restart(); }
    }
    Timer { id: settle; interval: 700; onTriggered: root.refresh() }
    Component.onCompleted: refresh()

    readonly property var sourceLabels: ({ plugins: "Plugins", scripts: "Script", imports: "Import", assets: "Asset" })

    ListView {
        id: list
        objectName: "problems-list"
        anchors.fill: parent
        clip: true
        model: root.problems
        ScrollBar.vertical: ScrollBar {}
        delegate: Rectangle {
            id: row
            required property var modelData
            required property int index
            width: list.width
            height: Math.max(30, message.implicitHeight + 12)
            color: index % 2 ? "transparent" : Theme.field
            RowLayout {
                anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 8; spacing: 8
                LucideIcon {
                    Layout.alignment: Qt.AlignTop; Layout.topMargin: 7
                    name: row.modelData.severity === "error" ? "circle-x" : "triangle-alert"
                    width: 14; height: 14
                    color: row.modelData.severity === "error" ? Theme.danger : Theme.warning
                }
                Text {
                    Layout.alignment: Qt.AlignTop; Layout.topMargin: 6
                    Layout.preferredWidth: 52
                    text: root.sourceLabels[row.modelData.source] || row.modelData.source
                    color: Theme.textDim; font.pixelSize: 11
                }
                TextEdit {
                    id: message
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignVCenter
                    text: row.modelData.message
                    color: Theme.text; font.pixelSize: 12
                    wrapMode: TextEdit.Wrap; readOnly: true; selectByMouse: true
                    textFormat: TextEdit.PlainText
                }
                StitchButton {
                    visible: row.modelData.source === "scripts" && !!row.modelData.path
                    flat: true; implicitHeight: 24; font.pixelSize: 11
                    text: "Open script"
                    onClicked: root.openScriptRequested(row.modelData.path, row.modelData.line || 1)
                }
            }
        }
    }
    Column {
        visible: root.problems.length === 0
        anchors.centerIn: parent; spacing: 6
        LucideIcon { anchors.horizontalCenter: parent.horizontalCenter; name: "check"; width: 22; height: 22; color: Theme.textDim }
        Text { text: "No problems found."; color: Theme.textDim; font.pixelSize: 12 }
    }
}
