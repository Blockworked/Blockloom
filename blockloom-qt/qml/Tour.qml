import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A short walk round the editor: each step rings one part of the window,
// found by objectName under `host`, and says what it is for. It starts once
// on the first project opened and again from Help. It never takes clicks
// outside its own card, so the editor stays usable while it is up.
Item {
    id: root
    required property Item host
    property bool autoStart: true
    property int index: -1
    readonly property bool running: index >= 0
    readonly property var step: running ? steps[index] : null
    visible: running
    z: 1000

    Settings { id: seen; category: "tour"; property bool done: false }

    readonly property var steps: [
        { target: "dock-left", title: "Your actors",
          body: "Everything in the game is an actor: the player, a coin, the level. Pick one here to edit its blocks and components." },
        { target: "block-sidebar", title: "Blocks",
          body: "Drag a block from here onto the canvas. Start with a hat from Events, like \"when Play is pressed\", then snap steps under it." },
        { target: "code-pane", title: "The canvas",
          body: "The selected actor's blocks live here. Each strand under a hat runs on its own when its event happens." },
        { target: "play-button", title: "Play",
          body: "Runs the game right here in the Game view. Press it again to stop; nothing you change while playing is lost." },
        { target: "dock-right", title: "Inspector",
          body: "The selected actor's components: its look, physics body, colliders, lights and more. Add one with the + at the bottom." },
        { target: "dock-bottom", title: "Assets and the run log",
          body: "Images, sounds and models go in Assets. The run log shows what your blocks say and any errors, with the fix when there is one." },
        { target: "menu-Help", title: "Find anything",
          body: "Ctrl+K opens the command palette, which finds every menu item. This tour is under Help whenever you want it again." }
    ]

    function start() { root.index = 0; }
    function finish() { root.index = -1; seen.done = true; }
    function next() { if (root.index + 1 < root.steps.length) root.index++; else root.finish(); }
    function back() { if (root.index > 0) root.index--; }
    Component.onCompleted: if (root.autoStart && !seen.done) Qt.callLater(root.start)

    // Depth-first, through delegates too (their parent's children hold them).
    function find(item, name) {
        if (!item) return null;
        if (item.objectName === name && item.visible) return item;
        for (let i = 0; i < item.children.length; i++) {
            const hit = root.find(item.children[i], name);
            if (hit) return hit;
        }
        return null;
    }
    readonly property Item target: running ? find(host, step.target) : null
    readonly property rect ring: {
        if (!target || target.width <= 0) return Qt.rect(0, 0, 0, 0);
        // Re-read when the window moves things about.
        void(host.width + host.height);
        const p = target.mapToItem(root, 0, 0);
        return Qt.rect(p.x - 4, p.y - 4, target.width + 8, target.height + 8);
    }

    Rectangle {
        objectName: "tour-ring"
        visible: root.ring.width > 0
        x: root.ring.x; y: root.ring.y; width: root.ring.width; height: root.ring.height
        color: "transparent"; radius: 8
        border.color: Theme.accent; border.width: 3
    }

    Rectangle {
        id: card
        objectName: "tour-card"
        width: Math.min(340, root.width - 32)
        height: column.implicitHeight + 28
        radius: 10
        color: Theme.panel
        border.color: Theme.accent
        // Beside the ring where it fits, else in the middle.
        x: {
            if (root.ring.width <= 0) return (root.width - width) / 2;
            const right = root.ring.x + root.ring.width + 12;
            if (right + width <= root.width - 8) return right;
            const left = root.ring.x - width - 12;
            if (left >= 8) return left;
            return Math.max(8, Math.min(root.width - width - 8, root.ring.x));
        }
        y: {
            if (root.ring.width <= 0) return (root.height - height) / 2;
            const beside = x > root.ring.x + root.ring.width || x + width < root.ring.x;
            const top = beside ? root.ring.y : root.ring.y + root.ring.height + 12;
            const fits = top + height <= root.height - 8;
            return fits ? Math.max(8, top) : Math.max(8, root.ring.y - height - 12);
        }
        MouseArea { anchors.fill: parent }
        ColumnLayout {
            id: column
            anchors { left: parent.left; right: parent.right; top: parent.top; margins: 14 }
            spacing: 8
            Text {
                objectName: "tour-title"
                Layout.fillWidth: true
                text: root.step ? root.step.title : ""
                color: Theme.text; font.pixelSize: 15; font.bold: true
            }
            Text {
                Layout.fillWidth: true; wrapMode: Text.WordWrap
                text: root.step ? root.step.body : ""
                color: Theme.textDim; font.pixelSize: 13
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                Text {
                    text: (root.index + 1) + " of " + root.steps.length
                    color: Theme.textDim; font.pixelSize: 12
                }
                Item { Layout.fillWidth: true }
                StitchButton { objectName: "tour-skip"; text: "Skip"; implicitHeight: 28; onClicked: root.finish() }
                StitchButton { objectName: "tour-back"; text: "Back"; implicitHeight: 28; visible: root.index > 0; onClicked: root.back() }
                StitchButton {
                    objectName: "tour-next"
                    text: root.index + 1 < root.steps.length ? "Next" : "Done"
                    implicitHeight: 28; onClicked: root.next()
                }
            }
        }
    }
}
