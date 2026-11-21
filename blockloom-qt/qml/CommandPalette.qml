import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Type to find any action or shell command, Enter to run it.
Popup {
    id: root
    required property var actions
    property string query: ""
    property int current: 0
    modal: true
    focus: true
    width: Math.min(560, parent ? parent.width - 40 : 560)
    height: Math.min(440, column.implicitHeight + padding * 2)
    x: parent ? Math.round((parent.width - width) / 2) : 0
    y: 70
    padding: 8
    background: Rectangle { color: Theme.panelRaised; border.color: Theme.border; radius: 8 }

    // Every word must appear; a label match ranks above a summary match.
    function score(entry, words) {
        const label = (entry.label + " " + entry.id).toLowerCase();
        const detail = (entry.detail || "").toLowerCase();
        let total = 0;
        for (const w of words) {
            if (label.indexOf(w) >= 0) total += label.startsWith(w) ? 3 : 2;
            else if (detail.indexOf(w) >= 0) total += 1;
            else return -1;
        }
        return total + (entry.command ? 0 : 1) + (root.actions.enabled(entry.id) ? 10 : 0);
    }
    readonly property var matches: {
        const words = root.query.toLowerCase().split(/\s+/).filter(w => w.length);
        const all = root.actions.paletteEntries.filter(e => !e.command || words.length);
        if (!words.length) return all.filter(e => root.actions.enabled(e.id));
        return all.map(e => ({ entry: e, score: root.score(e, words) }))
            .filter(m => m.score >= 0)
            .sort((a, b) => b.score - a.score)
            .map(m => m.entry)
            .slice(0, 60);
    }
    onMatchesChanged: current = 0
    onOpened: { query = ""; field.text = ""; field.forceActiveFocus(); }
    function runCurrent() {
        const entry = root.matches[root.current];
        if (!entry) return;
        root.close();
        root.actions.runEntry(entry);
    }

    contentItem: ColumnLayout {
        id: column
        spacing: 6
        StitchTextField {
            id: field
            objectName: "palette-field"
            Layout.fillWidth: true
            placeholderText: "Search actions and commands"
            onTextChanged: root.query = text
            Keys.onDownPressed: root.current = Math.min(root.matches.length - 1, root.current + 1)
            Keys.onUpPressed: root.current = Math.max(0, root.current - 1)
            Keys.onReturnPressed: root.runCurrent()
            Keys.onEnterPressed: root.runCurrent()
        }
        ListView {
            id: list
            objectName: "palette-list"
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(contentHeight, 360)
            clip: true
            model: root.matches
            currentIndex: root.current
            onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)
            ScrollBar.vertical: ScrollBar {}
            delegate: Rectangle {
                id: row
                required property var modelData
                required property int index
                readonly property bool live: root.actions.enabled(modelData.id)
                width: list.width
                height: 40
                radius: 5
                color: index === root.current ? Theme.hover : "transparent"
                opacity: live ? 1 : 0.5
                RowLayout {
                    anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 10; spacing: 10
                    LucideIcon { name: row.modelData.icon; width: 15; height: 15; color: Theme.textDim }
                    ColumnLayout {
                        Layout.fillWidth: true; spacing: 1
                        Text { Layout.fillWidth: true; text: row.modelData.label; color: Theme.text; font.pixelSize: 13; elide: Text.ElideRight }
                        Text { Layout.fillWidth: true; text: row.modelData.detail || ""; color: Theme.textDim; font.pixelSize: 11; elide: Text.ElideRight; visible: text.length > 0 }
                    }
                    Text { text: row.modelData.keys || ""; color: Theme.textDim; font.pixelSize: 11 }
                }
                MouseArea {
                    anchors.fill: parent; hoverEnabled: true
                    onEntered: root.current = row.index
                    onClicked: { root.current = row.index; root.runCurrent(); }
                }
            }
        }
        Text {
            visible: root.matches.length === 0
            Layout.fillWidth: true; Layout.margins: 8
            text: "Nothing matches."; color: Theme.textDim; font.pixelSize: 12
        }
    }
}
