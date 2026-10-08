import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A clip's frames in a row: click one to see it large, press play to scrub at the clip's rate.
ColumnLayout {
    id: root
    property var app
    property var clip: ({})
    property int current: 0
    property bool playing: false
    // The collider as fractions of a frame ({w, h, x, y, round}), or null.
    property var hitbox: null
    property bool showHitbox: true
    readonly property var sheet: clip && clip.sheet ? clip.sheet : null
    readonly property int frameCount: sheet ? Math.max(1, sheet.count) : (clip && clip.frames ? clip.frames.length : 0)
    readonly property real fps: clip && clip.fps > 0 ? clip.fps : 8
    Layout.fillWidth: true; spacing: 4
    visible: frameCount > 0

    function frameSeconds(i) {
        const d = clip && clip.durations ? clip.durations : [];
        return i < d.length && d[i] > 0 ? d[i] : 1 / fps;
    }
    function markersAt(i) {
        return (clip && clip.markers ? clip.markers : []).filter(m => m.frame === i).map(m => m.name);
    }
    onFrameCountChanged: if (current >= frameCount) current = 0

    Image { id: probe; visible: false; source: root.sheet ? root.app.assetUrl(root.sheet.image) : ""; asynchronous: true }
    Timer {
        running: root.playing && root.frameCount > 1; repeat: true
        interval: Math.max(16, root.frameSeconds(root.current) * 1000)
        onTriggered: root.current = (root.current + 1) % root.frameCount
    }

    // One frame drawn from either the sheet or its own file.
    component Cell: Item {
        id: cell
        required property int index
        property real side: 48
        implicitWidth: side; implicitHeight: side
        readonly property int sheetCell: root.sheet ? root.sheet.first + index : 0
        readonly property real cw: root.sheet && probe.implicitWidth > 0 ? probe.implicitWidth / Math.max(1, root.sheet.columns) : 0
        readonly property real ch: root.sheet && probe.implicitHeight > 0 ? probe.implicitHeight / Math.max(1, root.sheet.rows) : 0
        Rectangle {
            visible: root.showHitbox && !!root.hitbox
            z: 1; color: "transparent"; border.width: 1; border.color: "#ff5a5a"
            radius: root.hitbox && root.hitbox.round ? Math.min(width, height) / 2 : 0
            width: root.hitbox ? parent.side * root.hitbox.w : 0
            height: root.hitbox ? parent.side * root.hitbox.h : 0
            x: root.hitbox ? (parent.side - width) / 2 + parent.side * root.hitbox.x : 0
            y: root.hitbox ? (parent.side - height) / 2 + parent.side * root.hitbox.y : 0
        }
        Image {
            anchors.fill: parent; fillMode: Image.PreserveAspectFit; smooth: false; asynchronous: true
            source: root.sheet ? probe.source : (root.clip.frames && root.clip.frames[cell.index] ? root.app.assetUrl(root.clip.frames[cell.index]) : "")
            sourceClipRect: root.sheet && cell.cw > 0
                ? Qt.rect((cell.sheetCell % root.sheet.columns) * cell.cw, Math.floor(cell.sheetCell / root.sheet.columns) * cell.ch, cell.cw, cell.ch)
                : Qt.rect(0, 0, 0, 0)
        }
    }

    RowLayout {
        Layout.fillWidth: true; spacing: 8
        Rectangle {
            implicitWidth: 96; implicitHeight: 96; color: Theme.borderSoft; radius: 4
            Cell { anchors.fill: parent; anchors.margins: 4; index: root.current; side: 88 }
        }
        ColumnLayout {
            Layout.fillWidth: true; spacing: 4
            RowLayout {
                spacing: 4
                IconButton { iconName: root.playing ? "pause" : "play"; tip: root.playing ? "Stop scrubbing" : "Play the clip here"
                    implicitWidth: 26; implicitHeight: 26; onClicked: root.playing = !root.playing }
                IconButton { visible: !!root.hitbox; iconName: "eye"; opacity: root.showHitbox ? 1 : 0.4; tip: "Show the collider on each frame"
                    implicitWidth: 26; implicitHeight: 26; onClicked: root.showHitbox = !root.showHitbox }
                Text { color: Theme.text; font.pixelSize: 12; text: "Frame " + (root.current + 1) + " of " + root.frameCount }
            }
            Text { color: Theme.textDim; font.pixelSize: 11; Layout.fillWidth: true; wrapMode: Text.WordWrap
                text: root.frameSeconds(root.current).toFixed(3) + " s" + (root.markersAt(root.current).length ? "  marker: " + root.markersAt(root.current).join(", ") : "") }
        }
    }
    Flickable {
        Layout.fillWidth: true; implicitHeight: 54; clip: true
        contentWidth: strip.width; contentHeight: height; boundsBehavior: Flickable.StopAtBounds
        Row {
            id: strip; spacing: 2
            Repeater {
                model: root.frameCount
                delegate: Rectangle {
                    required property int index
                    width: 50; height: 50; radius: 3; color: "transparent"
                    border.width: index === root.current ? 2 : 1
                    border.color: index === root.current ? Theme.accent : Theme.borderSoft
                    Cell { anchors.centerIn: parent; index: parent.index; side: 44 }
                    Rectangle { visible: root.markersAt(parent.index).length > 0; width: 6; height: 6; radius: 3; color: Theme.accent; anchors.top: parent.top; anchors.right: parent.right; anchors.margins: 2 }
                    MouseArea { anchors.fill: parent; onClicked: { root.playing = false; root.current = parent.index } }
                }
            }
        }
    }
}
