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
    readonly property var log: app.log
    onLogChanged: sync()
    Component.onCompleted: sync()
    // Which lines show: "all", "errors", "says" or "other", and words to find
    // in a line or its actor's name.
    property string show: "all"
    property string search: ""
    onShowChanged: refilter()
    onSearchChanged: refilter()
    property int errorCount: 0
    // Paused holds the view still while the world keeps logging.
    property bool paused: false
    onPausedChanged: if (!paused) { unseen = 0; reconcile(); if (following) scrollToEnd(); }
    // Lines that arrived while paused or scrolled up, for the jump button.
    property int unseen: 0
    // Follows new lines only while scrolled to the bottom.
    property bool following: true
    onFollowingChanged: if (following) unseen = 0
    function isError(e) { return e.level === "error" || e.kind === "error"; }
    function passes(e) {
        if (root.show === "errors" && !root.isError(e)) return false;
        if (root.show === "says" && e.kind !== "say") return false;
        if (root.show === "other" && (e.kind === "say" || root.isError(e))) return false;
        const wanted = root.search.trim().toLowerCase();
        return !wanted || e.words.toLowerCase().indexOf(wanted) >= 0 || e.actor.toLowerCase().indexOf(wanted) >= 0;
    }
    // Every line the backend still holds; lineModel is the ones that pass.
    property var all: []
    function sync() {
        const incoming = log.lines || [], total = log.total || 0;
        // Lines carry an id that stays put while they are kept; an older
        // backend's don't, so count back from the total.
        all = incoming.map((line, i) => ({
            lid: line.id || total - incoming.length + i + 1,
            kind: line.kind, level: line.level || line.kind, actor: line.actor || "", words: line.text || "",
            path: line.path || "", line: line.line || 0, times: line.count || 1,
            time: line.time === undefined || line.time === null ? -1 : line.time
        }));
        let errors = 0;
        for (const e of all) if (root.isError(e)) errors++;
        root.errorCount = errors;
        if (root.paused) root.unseen = root.waiting();
        else root.reconcile();
    }
    // Lines past the last shown one that would show.
    function waiting() {
        const shown = root.shownId();
        let n = 0;
        for (let i = all.length - 1; i >= 0 && all[i].lid > shown; i--) if (root.passes(all[i])) n++;
        return n;
    }
    function lastId() { return all.length ? all[all.length - 1].lid : 0; }
    function shownId() { return lineModel.count ? lineModel.get(lineModel.count - 1).lid : 0; }
    // Brings lineModel up to `all` without rebuilding it: rows the backend
    // dropped leave from the top, a repeat ticks its count, new lines append.
    function reconcile() {
        if (!all.length) { lineModel.clear(); root.unseen = 0; return; }
        if (root.shownId() > root.lastId()) { refilter(); return; }
        const first = all[0].lid;
        let drop = 0;
        while (drop < lineModel.count && lineModel.get(drop).lid < first) drop++;
        // Rows leaving the top can nudge contentY; that isn't the person scrolling.
        positioning = true;
        if (drop) lineModel.remove(0, drop);
        const shown = root.shownId();
        let i = all.length - 1;
        while (i >= 0 && all[i].lid > shown) i--;
        if (i >= 0 && all[i].lid === shown) {
            const row = lineModel.count - 1, e = all[i];
            if (lineModel.get(row).times !== e.times) lineModel.set(row, { times: e.times, time: e.time });
        }
        let added = 0;
        for (let j = i + 1; j < all.length; j++)
            if (root.passes(all[j])) { lineModel.append(Object.assign({ expanded: false }, all[j])); added++; }
        positioning = false;
        if (!added) return;
        if (root.following) root.scrollToEnd();
        else root.unseen += added;
    }
    function refilter() {
        lineModel.clear();
        for (const e of all) if (root.passes(e)) lineModel.append(Object.assign({ expanded: false }, e));
        root.unseen = 0;
        root.following = true;
        root.scrollToEnd();
    }
    property bool positioning: false
    function scrollToEnd() {
        positioning = true;
        lines.positionViewAtEnd();
        positioning = false;
        following = true;
    }
    ListModel { id: lineModel }
    function shownCount() { return lineModel.count; }
    function shown(value) { return value.value === undefined ? String(value.kind) : String(value.value); }
    // Run time as 12.3 or 1:02.3, so the column stays narrow.
    function clock(t) {
        if (t < 0) return "";
        if (t < 60) return t.toFixed(1);
        const m = Math.floor(t / 60), s = t - m * 60;
        return m + ":" + (s < 10 ? "0" : "") + s.toFixed(1);
    }

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
            // Grows to its widest and stays, so the ticking clock moves nothing.
            Text {
                property real widest: 0
                onImplicitWidthChanged: widest = Math.max(widest, implicitWidth)
                Layout.preferredWidth: widest
                color: Theme.textDim; font.pixelSize: 12; font.features: { "tnum": 1 }
                text: root.appState.running ? "Running" + (root.appState.paused ? " (paused)" : "") + " · " + (root.app.status ? root.app.status.time : 0).toFixed(1) + "s" : "Not running"
            }
            Item { Layout.fillWidth: true }
            Repeater {
                model: [{ id: "all", label: "All" }, { id: "errors", label: "Errors" }, { id: "says", label: "Says" }, { id: "other", label: "Other" }]
                StitchButton {
                    required property var modelData
                    objectName: "runlog-show-" + modelData.id
                    flat: root.show !== modelData.id; primary: root.show === modelData.id
                    implicitHeight: 24; font.pixelSize: 11
                    property real widest: 0
                    onImplicitWidthChanged: widest = Math.max(widest, implicitWidth)
                    Layout.preferredWidth: widest
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
            IconButton {
                objectName: "runlog-pause"
                iconName: root.paused ? "play" : "pause"
                tip: root.paused ? "Show new lines again" : "Hold the log still; the game keeps running"
                implicitWidth: 24; implicitHeight: 24; onClicked: root.paused = !root.paused
            }
            IconButton { iconName: "trash-2"; tip: "Clear the log"; implicitWidth: 24; implicitHeight: 24; onClicked: root.app.invoke("clear_log") }
        }
        // Shared variables, on their own row so their changing values
        // never push the controls around.
        Flickable {
            visible: (root.docked || remembered.open) && root.globals.length > 0
            Layout.fillWidth: true; Layout.preferredHeight: 22; Layout.leftMargin: 8; Layout.rightMargin: 8
            clip: true; contentWidth: globalsRow.implicitWidth; flickableDirection: Flickable.HorizontalFlick
            Row {
                id: globalsRow
                spacing: 16; height: parent.height
                Repeater {
                    model: root.globals
                    delegate: Text {
                        required property var modelData
                        property real widest: 0
                        onImplicitWidthChanged: widest = Math.max(widest, implicitWidth)
                        width: Math.min(220, widest); elide: Text.ElideRight
                        anchors.verticalCenter: parent.verticalCenter
                        textFormat: Text.PlainText; font.family: "monospace"; font.pixelSize: 12; color: Theme.text
                        text: modelData.name + ": " + root.shown(modelData.value)
                    }
                }
            }
        }
        Item {
            visible: root.docked || remembered.open
            Layout.fillWidth: true; Layout.fillHeight: true
            ListView {
                id: lines
                objectName: "runlog-lines"
                anchors.fill: parent; clip: true
                model: lineModel
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar {}
                // Only the person scrolling moves contentY; growing content doesn't.
                onContentYChanged: if (!root.positioning) root.following = atYEnd || contentHeight <= height
                onHeightChanged: if (root.following) root.scrollToEnd()
                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string kind
                    required property string level
                    required property string actor
                    required property string words
                    required property string path
                    required property int line
                    required property int times
                    required property real time
                    required property bool expanded
                    readonly property bool failed: level === "error" || kind === "error"
                    readonly property bool multiline: words.indexOf("\n") >= 0 || summary.truncated
                    width: lines.width
                    implicitHeight: expanded ? Math.max(22, full.implicitHeight + 6) : 22
                    color: hover.hovered ? Theme.hover : failed ? Theme.dangerBackground : "transparent"
                    HoverHandler { id: hover }
                    TapHandler { enabled: row.multiline || row.expanded; onTapped: lineModel.setProperty(row.index, "expanded", !row.expanded) }
                    RowLayout {
                        anchors.fill: parent; anchors.leftMargin: 8; anchors.rightMargin: 12
                        spacing: 8
                        Text {
                            Layout.preferredWidth: 46; Layout.alignment: Qt.AlignTop; Layout.topMargin: 3
                            horizontalAlignment: Text.AlignRight
                            font.family: "monospace"; font.pixelSize: 11; color: Theme.textDim
                            text: root.clock(row.time)
                        }
                        Text {
                            Layout.preferredWidth: 104; Layout.alignment: Qt.AlignTop; Layout.topMargin: 3
                            elide: Text.ElideRight; textFormat: Text.PlainText
                            font.pixelSize: 12; color: row.failed ? Theme.danger : Theme.textDim
                            text: row.actor
                        }
                        LucideIcon {
                            visible: row.multiline || row.expanded
                            Layout.alignment: Qt.AlignTop; Layout.topMargin: 4
                            name: row.expanded ? "chevron-down" : "chevron-right"; implicitWidth: 12; implicitHeight: 12; color: Theme.textDim
                        }
                        Text {
                            id: summary
                            visible: !row.expanded
                            Layout.fillWidth: true; Layout.alignment: Qt.AlignTop; Layout.topMargin: 3
                            elide: Text.ElideRight; maximumLineCount: 1; wrapMode: Text.NoWrap; textFormat: Text.PlainText
                            font.family: "monospace"; font.pixelSize: 12; color: row.failed ? Theme.danger : Theme.text
                            text: row.words.split("\n")[0]
                        }
                        TextEdit {
                            id: full
                            visible: row.expanded
                            Layout.fillWidth: true; Layout.alignment: Qt.AlignTop; Layout.topMargin: 3
                            wrapMode: TextEdit.Wrap; font.family: "monospace"; font.pixelSize: 12
                            textFormat: TextEdit.PlainText
                            color: row.failed ? Theme.danger : Theme.text
                            text: row.expanded ? row.words : ""
                            readOnly: true; selectByMouse: true; selectByKeyboard: true; persistentSelection: true
                        }
                        // How many times in a row it was said.
                        Rectangle {
                            visible: row.times > 1
                            Layout.alignment: Qt.AlignTop; Layout.topMargin: 3
                            implicitWidth: repeats.implicitWidth + 10; implicitHeight: 16; radius: 8
                            color: Theme.panelRaised
                            Text {
                                id: repeats
                                anchors.centerIn: parent
                                font.pixelSize: 10; font.features: { "tnum": 1 }; color: Theme.textDim
                                text: "×" + row.times
                            }
                        }
                        StitchButton {
                            visible: row.path.length > 0
                            Layout.alignment: Qt.AlignTop
                            flat: true; implicitHeight: 22; font.pixelSize: 11
                            text: "Open script"
                            onClicked: root.openScriptRequested(row.path, row.line > 0 ? row.line : 1)
                        }
                    }
                }
            }
            StitchButton {
                objectName: "runlog-jump"
                visible: root.unseen > 0 && (!root.following || root.paused)
                anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.margins: 8
                implicitHeight: 24; font.pixelSize: 11; primary: true
                text: root.unseen + (root.unseen === 1 ? " new line" : " new lines")
                onClicked: { root.paused = false; root.scrollToEnd(); }
            }
        }
    }
}
