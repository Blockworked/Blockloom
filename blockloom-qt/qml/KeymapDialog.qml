import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Every action with its keys, rebindable for this user. Keys are typed the
// way Qt spells them (Ctrl+Shift+K); a comma gives an action a second key.
StitchDialog {
    id: root
    required property var actions
    property string filter: ""
    property string error: ""
    title: "Keyboard shortcuts"
    standardButtons: Dialog.NoButton
    width: Math.min(parent ? parent.width - 80 : 620, 620)
    height: Math.min(parent ? parent.height - 80 : 640, 640)
    onOpened: { filter = ""; error = ""; }

    readonly property var rows: {
        const words = root.filter.toLowerCase().split(/\s+/).filter(w => w.length);
        return root.actions.paletteEntries.filter(e => {
            const text = (e.label + " " + e.id + " " + root.keysText(e.id)).toLowerCase();
            return words.every(w => text.indexOf(w) >= 0);
        });
    }
    function keysText(id) { return root.actions.keys(id).join(", "); }
    function parse(text) { return text.split(",").map(k => k.trim()).filter(k => k.length); }
    function changed(id) {
        const now = root.actions.keys(id), was = root.actions.defaultKeys[id] || [];
        return JSON.stringify(now) !== JSON.stringify(was);
    }
    function apply(id, text) {
        if (text === root.keysText(id)) return;
        root.error = "";
        root.actions.rebind(id, root.parse(text), null, e => root.error = e);
    }
    function reset(id) { root.error = ""; root.actions.rebind(id, null, null, e => root.error = e); }

    ColumnLayout {
        anchors.fill: parent; spacing: 8
        StitchTextField {
            objectName: "keymap-filter"
            Layout.fillWidth: true
            placeholderText: "Find an action or a key"
            text: root.filter
            onTextChanged: root.filter = text
        }
        Text { visible: root.error !== ""; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.error }
        ListView {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: root.rows
            spacing: 2
            delegate: RowLayout {
                id: row
                required property var modelData
                width: ListView.view.width; spacing: 6
                objectName: "keymap-row-" + modelData.id
                LucideIcon { name: row.modelData.icon || "command"; width: 14; height: 14; color: Theme.textDim }
                Text { Layout.fillWidth: true; text: row.modelData.label; color: Theme.text; font.pixelSize: 13; elide: Text.ElideRight }
                StitchTextField {
                    objectName: "keymap-keys-" + row.modelData.id
                    Layout.preferredWidth: 200
                    placeholderText: "No key"
                    text: root.keysText(row.modelData.id)
                    onEditingFinished: root.apply(row.modelData.id, text)
                }
                IconButton {
                    iconName: "rotate-ccw"; tip: "Put the default back"
                    implicitWidth: 26; implicitHeight: 26
                    enabled: root.changed(row.modelData.id)
                    opacity: enabled ? 1 : 0.35
                    onClicked: root.reset(row.modelData.id)
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            StitchButton { text: "Reset all"; onClicked: root.actions.app.invoke("reset_keymap", {}, r => root.actions.load(r), e => root.error = String(e)) }
            Item { Layout.fillWidth: true }
            StitchButton { text: "Close"; onClicked: root.close() }
        }
    }
}
