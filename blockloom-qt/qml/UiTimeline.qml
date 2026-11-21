import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// One animation: a lane per widget property with keys you drag, a playhead you
// scrub, and an editor for the selected key. Edits go through the designer's
// whole-animation SetAnimation, so each is one undo step.
ColumnLayout {
    id: root
    required property var designer
    property string name: ""
    property int selTrack: -1
    property int selKey: -1
    readonly property var anim: (designer.document.animations || {})[name] || null
    readonly property real total: Math.max(0.05, anim ? anim.duration : 1)
    readonly property real playhead: designer.animPreviewing && designer.animPreviewName === name ? designer.animPreviewTime : 0
    readonly property var selected: anim && selTrack >= 0 && selTrack < anim.tracks.length && selKey >= 0 && selKey < anim.tracks[selTrack].keys.length
        ? anim.tracks[selTrack].keys[selKey] : null
    readonly property real labelWidth: 112
    spacing: 6
    property real wantTime: -1
    onNameChanged: { selTrack = -1; selKey = -1; wantTime = -1; }
    onAnimChanged: {
        if (wantTime < 0 || selTrack < 0 || !anim || selTrack >= anim.tracks.length) return;
        const found = anim.tracks[selTrack].keys.findIndex(k => Math.abs(k.time - wantTime) < 0.0005);
        if (found >= 0) selKey = found;
        wantTime = -1;
    }

    function timeAt(lane, x) { return Math.max(0, Math.min(total, x / Math.max(1, lane.width) * total)); }
    function round(v) { return Math.round(v * 100) / 100; }

    RowLayout {
        Layout.fillWidth: true; spacing: 6
        StitchButton {
            objectName: "interfacePreviewAnimation"
            text: root.designer.animPreviewing ? "Stop" : "Preview"
            iconName: root.designer.animPreviewing ? "pause" : "play"
            implicitHeight: 28; font.pixelSize: 12
            enabled: !root.designer.gesture && root.designer.designing
            onClicked: root.designer.animPreviewing ? root.designer.stopAnimationPreview() : root.designer.previewAnimation(root.name, 0)
        }
        Text { text: root.playhead.toFixed(2) + " s of "; color: Theme.textDim; font.pixelSize: 11 }
        NumberField {
            objectName: "interfaceAnimationLength"
            Layout.fillWidth: false; Layout.preferredWidth: 56
            value: root.anim ? root.anim.duration : 1; fallback: 1
            onCommitted: n => { if (n > 0) root.designer.withAnimation(root.name, a => { a.duration = n; a.tracks.forEach(t => { t.keys = t.keys.filter(k => k.time <= n); }); a.tracks = a.tracks.filter(t => t.keys.length); }); }
        }
        StitchCheckBox {
            objectName: "interfaceAnimationLoop"; text: "Loop"
            checked: !!(root.anim && root.anim.looping)
            onToggled: root.designer.withAnimation(root.name, a => { a.looping = checked; })
        }
    }

    // Ruler: press or drag to scrub the runtime preview.
    RowLayout {
        Layout.fillWidth: true; spacing: 0
        Item { Layout.preferredWidth: root.labelWidth }
        Rectangle {
            id: ruler
            objectName: "interfaceAnimationScrub"
            Layout.fillWidth: true; Layout.preferredHeight: 20
            color: Theme.field; radius: 3
            Repeater {
                model: 5
                delegate: Text {
                    required property int index
                    x: index / 4 * (ruler.width - 1) - (index === 4 ? width : 0) + (index === 0 ? 2 : 0)
                    anchors.verticalCenter: parent.verticalCenter
                    text: (root.total * index / 4).toFixed(1); color: Theme.textDim; font.pixelSize: 9
                }
            }
            Rectangle { x: root.playhead / root.total * (ruler.width - 2); width: 2; height: parent.height; color: Theme.accent }
            MouseArea {
                anchors.fill: parent
                function scrub(mouse) { root.designer.previewAnimation(root.name, root.timeAt(ruler, mouse.x)); }
                onPressed: mouse => scrub(mouse)
                onPositionChanged: mouse => { if (pressed) scrub(mouse); }
            }
        }
    }

    Text {
        visible: root.anim && root.anim.tracks.length === 0
        Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
        text: "No tracks yet. Select a widget, pick a property and add a key at the playhead."
    }

    Repeater {
        model: root.anim ? root.anim.tracks : []
        delegate: RowLayout {
            id: trackRow
            required property var modelData
            required property int index
            Layout.fillWidth: true; spacing: 0
            Item {
                Layout.preferredWidth: root.labelWidth; Layout.minimumWidth: root.labelWidth; Layout.maximumWidth: root.labelWidth
                Layout.preferredHeight: 24
                Text {
                    anchors.left: parent.left; anchors.right: removeTrack.left; anchors.verticalCenter: parent.verticalCenter
                    elide: Text.ElideRight; font.pixelSize: 11; color: Theme.text
                    text: trackRow.modelData.widget + " " + trackRow.modelData.property
                }
                IconButton {
                    id: removeTrack
                    objectName: "interfaceRemoveTrack" + trackRow.index
                    anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                    implicitWidth: 22; implicitHeight: 22; iconName: "x"; tip: "Remove track"
                    enabled: !root.designer.gesture
                    onClicked: { root.selTrack = -1; root.selKey = -1; root.designer.removeTrack(root.name, trackRow.index); }
                }
            }
            Rectangle {
                id: lane
                objectName: "interfaceTimeline" + trackRow.index
                Layout.fillWidth: true; Layout.preferredHeight: 24
                color: Theme.field; radius: 3
                border.color: root.selTrack === trackRow.index ? Theme.accent : Theme.borderSoft
                Rectangle { x: root.playhead / root.total * (lane.width - 1); width: 1; height: parent.height; color: Theme.accent; opacity: 0.8 }
                Repeater {
                    model: trackRow.modelData.keys
                    delegate: Rectangle {
                        id: key
                        required property var modelData
                        required property int index
                        readonly property bool current: root.selTrack === trackRow.index && root.selKey === index
                        property real dragTime: -1
                        readonly property real shown: dragTime >= 0 ? dragTime : modelData.time
                        x: shown / root.total * (lane.width - width)
                        y: (lane.height - height) / 2
                        width: 11; height: 11; rotation: 45
                        color: current ? Theme.warning : Theme.accent
                        MouseArea {
                            anchors.fill: parent; anchors.margins: -4
                            cursorShape: Qt.SizeHorCursor
                            drag.target: null
                            property real startX: 0
                            onPressed: mouse => { root.selTrack = trackRow.index; root.selKey = key.index; startX = mouse.x; }
                            onPositionChanged: mouse => {
                                if (!pressed) return;
                                const p = mapToItem(lane, mouse.x, mouse.y);
                                key.dragTime = root.round(root.timeAt(lane, p.x));
                            }
                            onReleased: {
                                if (key.dragTime >= 0 && Math.abs(key.dragTime - key.modelData.time) > 0.001)
                                { root.wantTime = key.dragTime; root.designer.setKey(root.name, trackRow.index, key.index, {time: key.dragTime}); }
                                key.dragTime = -1;
                            }
                        }
                    }
                }
            }
        }
    }

    // The selected key.
    ColumnLayout {
        visible: !!root.selected
        Layout.fillWidth: true; spacing: 4
        Text { text: "Selected key"; color: Theme.textDim; font.pixelSize: 11 }
        RowLayout {
            Layout.fillWidth: true; spacing: 6
            NumberField {
                objectName: "interfaceKeyTimeEdit"
                Layout.fillWidth: false; Layout.preferredWidth: 56
                value: root.selected ? root.selected.time : 0
                onCommitted: n => { root.wantTime = Math.max(0, Math.min(root.total, n)); root.designer.setKey(root.name, root.selTrack, root.selKey, {time: root.wantTime}); }
            }
            StitchTextField {
                objectName: "interfaceKeyValueEdit"
                Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                readonly property bool color: root.anim && root.selTrack >= 0 && root.designer.isColorProp(root.anim.tracks[root.selTrack].property)
                text: root.selected ? String(root.selected.value) : ""
                onEditingFinished: {
                    if (!root.selected || text === String(root.selected.value)) return;
                    const n = Number(text);
                    if (!color && !Number.isFinite(n)) { text = String(root.selected.value); return; }
                    root.designer.setKey(root.name, root.selTrack, root.selKey, {value: color ? text : n});
                }
            }
            StitchComboBox {
                objectName: "interfaceKeyEaseEdit"
                implicitWidth: 100; implicitHeight: 30; font.pixelSize: 12
                model: ["Linear", "EaseIn", "EaseOut", "EaseInOut"]
                currentIndex: root.selected ? Math.max(0, model.indexOf(root.selected.ease || "Linear")) : 0
                onActivated: root.designer.setKey(root.name, root.selTrack, root.selKey, {ease: currentText})
            }
            IconButton {
                objectName: "interfaceDeleteKey"
                iconName: "trash"; tip: "Delete key"; implicitWidth: 30; implicitHeight: 30
                onClicked: { const t = root.selTrack, k = root.selKey; root.selKey = -1; root.designer.removeKey(root.name, t, k); }
            }
        }
    }

    // Add a key for the selected widget at the playhead.
    ColumnLayout {
        Layout.fillWidth: true; spacing: 4
        Text { text: "New key for the selected widget"; color: Theme.textDim; font.pixelSize: 11 }
        RowLayout {
            Layout.fillWidth: true; spacing: 6
            StitchComboBox {
                id: keyProperty
                objectName: "interfaceKeyProperty"
                Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                model: root.designer.animatableProps
            }
            StitchTextField {
                id: keyValue
                objectName: "interfaceKeyValue"
                Layout.preferredWidth: 80; implicitHeight: 30; font.pixelSize: 12
                placeholderText: root.designer.isColorProp(keyProperty.currentText) ? "#rrggbbaa" : "value"
            }
        }
        RowLayout {
            Layout.fillWidth: true; spacing: 6
            StitchTextField {
                id: keyTime
                objectName: "interfaceKeyTime"
                Layout.preferredWidth: 70; implicitHeight: 30; font.pixelSize: 12
                placeholderText: "time"
                validator: DoubleValidator { bottom: 0 }
            }
            StitchComboBox { id: keyEase; objectName: "interfaceKeyEase"; implicitWidth: 100; implicitHeight: 30; font.pixelSize: 12; model: ["Linear", "EaseIn", "EaseOut", "EaseInOut"] }
            StitchButton {
                objectName: "interfaceAddKey"
                Layout.fillWidth: true
                text: "Add key"; implicitHeight: 30; font.pixelSize: 12
                enabled: !!root.designer.widget && !root.designer.gesture
                onClicked: {
                    const prop = keyProperty.currentText;
                    const time = keyTime.text !== "" ? Number(keyTime.text) : root.playhead;
                    const raw = keyValue.text !== "" ? keyValue.text : root.designer.currentValue(prop);
                    if (raw === undefined || raw === null || raw === "") { root.designer.error = "Enter a value for " + prop + "."; return; }
                    root.designer.addKey(root.name, root.designer.widget.element.id, prop, time, raw, keyEase.currentText);
                    keyValue.text = ""; keyTime.text = "";
                }
            }
        }
    }
}
