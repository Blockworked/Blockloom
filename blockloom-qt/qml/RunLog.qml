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
    // In a dock region, which folds and sizes it instead.
    property bool docked: false
    // A script-linked line offers its file back: the editor opens it on it.
    signal openScriptRequested(string path, int line)
    // Appends the lines not yet shown and drops what the backend dropped.
    readonly property var log: app.log
    property real shownTotal: 0
    onLogChanged: sync()
    Component.onCompleted: sync()
    // Which lines show: "all", "errors", "says" or "other", and words to find
    // in a line or its actor's name.
    property string show: "all"
    property string search: ""
    onShowChanged: refilter()
    onSearchChanged: refilter()
    property int errorCount: 0
    function isError(e) { return e.level === "error" || e.kind === "error"; }
    function passes(e) {
        if (root.show === "errors" && !root.isError(e)) return false;
        if (root.show === "says" && e.kind !== "say") return false;
        if (root.show === "other" && (e.kind === "say" || root.isError(e))) return false;
        const wanted = root.search.trim().toLowerCase();
        return !wanted || e.words.toLowerCase().indexOf(wanted) >= 0 || e.actor.toLowerCase().indexOf(wanted) >= 0;
    }
    function sync() {
        const incoming = log.lines, added = log.total - shownTotal;
        const entry = line => ({ kind: line.kind, level: line.level || line.kind, actor: line.actor || "", words: line.text || "", path: line.path || "", line: line.line || 0 });
        if (added < 0 || added > incoming.length) {
            all = incoming.map(entry);
            refilter();
        } else {
            const fresh = [];
            for (let i = incoming.length - added; i < incoming.length; i++) fresh.push(entry(incoming[i]));
            const dropped = all.length + fresh.length - incoming.length;
            all = all.concat(fresh).slice(Math.max(0, dropped));
            if (dropped > 0) refilter();
            else for (const e of fresh) { if (root.isError(e)) root.errorCount++; if (root.passes(e)) lineModel.append(e); }
        }
        shownTotal = log.total;
    }
    // Every line the backend still holds; lineModel is the ones that pass.
    property var all: []
    function refilter() {
        lineModel.clear();
        let errors = 0;
        for (const e of all) { if (root.isError(e)) errors++; if (root.passes(e)) lineModel.append(e); }
        root.errorCount = errors;
    }
    ListModel { id: lineModel }
    function shownCount() { return lineModel.count; }
    function shown(value) { return value.value === undefined ? String(value.kind) : String(value.value); }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        // Drag the top edge to resize.
        Rectangle {
            visible: !root.docked && remembered.open
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
            IconButton { visible: !root.docked; iconName: remembered.open ? "chevron-down" : "chevron-up"; tip: remembered.open ? "Hide the log" : "Show the log"; implicitWidth: 24; implicitHeight: 24; onClicked: remembered.open = !remembered.open }
            Text {
                color: Theme.textDim; font.pixelSize: 12
                text: root.appState.running ? "Running" + (root.appState.paused ? " (paused)" : "") + " · " + (root.app.status ? root.app.status.time : 0).toFixed(1) + "s" : "Not running"
            }
            Repeater {
                model: root.globals
                delegate: Text { required property var modelData; text: modelData.name + ": " + root.shown(modelData.value); color: Theme.text; font.pixelSize: 12 }
            }
            Item { Layout.fillWidth: true }
            Repeater {
                model: [{ id: "all", label: "All" }, { id: "errors", label: "Errors" }, { id: "says", label: "Says" }, { id: "other", label: "Other" }]
                StitchButton {
                    required property var modelData
                    objectName: "runlog-show-" + modelData.id
                    flat: root.show !== modelData.id; primary: root.show === modelData.id
                    implicitHeight: 24; font.pixelSize: 11
                    text: modelData.id === "errors" && root.errorCount ? "Errors (" + root.errorCount + ")" : modelData.label
                    onClicked: root.show = modelData.id
                }
            }
            StitchTextField {
                objectName: "runlog-search"
                Layout.preferredWidth: 160; implicitHeight: 24; font.pixelSize: 11
                placeholderText: "Find in the log"
                onTextChanged: root.search = text
            }
            IconButton { iconName: "trash-2"; tip: "Clear the log"; implicitWidth: 24; implicitHeight: 24; onClicked: root.app.invoke("clear_log") }
        }
        ListView {
            id: lines
            visible: root.docked || remembered.open
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: lineModel
            ScrollBar.vertical: ScrollBar {}
            onCountChanged: positionViewAtEnd()
            delegate: RowLayout {
                required property string kind
                required property string level
                required property string actor
                required property string words
                required property string path
                required property int line
                width: lines.width - 16; x: 8
                spacing: 8
                TextEdit {
                    Layout.fillWidth: true
                    wrapMode: TextEdit.Wrap; font.family: "monospace"; font.pixelSize: 12
                    textFormat: TextEdit.PlainText
                    color: level === "error" || kind === "error" ? Theme.danger : Theme.text
                    text: (actor ? actor + "  " : "") + words
                    readOnly: true
                    selectByMouse: true
                    selectByKeyboard: true
                    persistentSelection: true
                }
                StitchButton {
                    visible: path.length > 0
                    flat: true; implicitHeight: 24; font.pixelSize: 11
                    text: "Open script"
                    onClicked: root.openScriptRequested(path, line > 0 ? line : 1)
                }
            }
        }
    }
}
