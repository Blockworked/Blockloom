import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// What the running world has said, plus the shared variables' live values.
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    readonly property var globals: app.status ? app.status.globals : []
    implicitHeight: 30 + (remembered.open ? remembered.height : 0)
    color: Theme.panel
    border.color: Theme.borderSoft

    Settings { id: remembered; category: "runlog"; property bool open: true; property real height: 96 }
    // Appends the lines not yet shown and drops what the backend dropped.
    readonly property var log: app.log
    property real shownTotal: 0
    onLogChanged: sync()
    Component.onCompleted: sync()
    function sync() {
        const incoming = log.lines, added = log.total - shownTotal;
        const entry = line => ({ kind: line.kind, actor: line.actor, words: line.text });
        if (added < 0 || added > incoming.length) {
            lineModel.clear();
            for (const line of incoming) lineModel.append(entry(line));
        } else {
            for (let i = incoming.length - added; i < incoming.length; i++) lineModel.append(entry(incoming[i]));
            if (lineModel.count > incoming.length) lineModel.remove(0, lineModel.count - incoming.length);
        }
        shownTotal = log.total;
    }
    ListModel { id: lineModel }
    function shown(value) { return value.value === undefined ? String(value.kind) : String(value.value); }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        // Drag the top edge to resize.
        Rectangle {
            visible: remembered.open
            Layout.fillWidth: true; Layout.preferredHeight: 4; color: resizer.containsMouse || resizer.pressed ? Theme.accent : "transparent"
            MouseArea {
                id: resizer
                anchors.fill: parent; anchors.topMargin: -3; anchors.bottomMargin: -3; hoverEnabled: true; cursorShape: Qt.SizeVerCursor
                property real startHeight: 0; property real startY: 0
                onPressed: mouse => { startHeight = remembered.height; startY = mapToItem(null, 0, mouse.y).y; }
                onPositionChanged: mouse => { if (pressed) remembered.height = Math.max(60, Math.min(520, startHeight + startY - mapToItem(null, 0, mouse.y).y)); }
            }
        }
        RowLayout {
            Layout.fillWidth: true; Layout.preferredHeight: 30; Layout.leftMargin: 6; Layout.rightMargin: 6; spacing: 12
            IconButton { iconName: remembered.open ? "chevron-down" : "chevron-up"; tip: remembered.open ? "Hide the log" : "Show the log"; implicitWidth: 24; implicitHeight: 24; onClicked: remembered.open = !remembered.open }
            Text {
                color: Theme.textDim; font.pixelSize: 12
                text: root.appState.running ? "Running" + (root.appState.paused ? " (paused)" : "") + " · " + (root.app.status ? root.app.status.time : 0).toFixed(1) + "s" : "Not running"
            }
            Repeater {
                model: root.globals
                delegate: Text { required property var modelData; text: modelData.name + ": " + root.shown(modelData.value); color: Theme.text; font.pixelSize: 12 }
            }
            Item { Layout.fillWidth: true }
            IconButton { iconName: "trash-2"; tip: "Clear the log"; implicitWidth: 24; implicitHeight: 24; onClicked: root.app.invoke("clear_log") }
        }
        ListView {
            id: lines
            visible: remembered.open
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: lineModel
            ScrollBar.vertical: ScrollBar {}
            onCountChanged: positionViewAtEnd()
            delegate: TextEdit {
                required property string kind
                required property string actor
                required property string words
                width: lines.width - 16; x: 8
                wrapMode: TextEdit.Wrap; font.family: "monospace"; font.pixelSize: 12
                textFormat: TextEdit.PlainText
                color: kind === "error" ? Theme.danger : Theme.text
                text: (actor ? actor + "  " : "") + words
                readOnly: true
                selectByMouse: true
                selectByKeyboard: true
                persistentSelection: true
            }
        }
    }
}
