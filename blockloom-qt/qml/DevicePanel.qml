import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The embedded emulator screen: a headless AVD boot (`-no-window`) whose
// screen the editor polls as downscaled PNGs and whose touch the editor
// forwards through `adb shell input` - Android Studio's tool-window mode.
// A few frames a second, so menus and turn-based games read fine while
// action games keep the external window. `device` names a serial; empty
// means the only device, like every other android command.
ColumnLayout {
    id: root
    required property var app
    property string device: ""
    property string frame: ""
    property int frameWidth: 0
    property int frameHeight: 0
    property bool watching: false
    property bool busy: false
    property string error: ""
    property string note: ""
    // The serial this panel booted itself, so Stop names it.
    property string ownSerial: ""
    spacing: 6

    function serialArg() {
        const serial = root.ownSerial || root.device;
        return serial ? { device: serial } : {};
    }
    function startWatching() {
        root.watching = true;
        root.error = "";
        root.pollFrame();
    }
    function stopWatching() {
        root.watching = false;
    }
    function pollFrame() {
        if (!root.watching) return;
        const args = root.serialArg();
        root.app.invoke("android_mirror_frame", args, result => {
            if (!root.watching) return;
            root.frame = result.image;
            root.frameWidth = result.width;
            root.frameHeight = result.height;
            root.error = "";
        }, e => { if (root.watching) root.error = String(e); });
    }
    function tap(fx, fy) {
        const args = root.serialArg();
        args.x = fx; args.y = fy;
        root.app.invoke("android_mirror_tap", args, () => root.pollFrame(), e => root.error = String(e));
    }
    function swipe(fx1, fy1, fx2, fy2) {
        const args = root.serialArg();
        args.x1 = fx1; args.y1 = fy1; args.x2 = fx2; args.y2 = fy2;
        root.app.invoke("android_mirror_swipe", args, () => root.pollFrame(), e => root.error = String(e));
    }
    function key(code) {
        const args = root.serialArg();
        args.code = code;
        root.app.invoke("android_mirror_key", args, () => root.pollFrame(), e => root.error = String(e));
    }
    function startEmbedded() {
        if (root.busy) return;
        root.busy = true; root.error = ""; root.note = "";
        root.app.invoke("android_start_emulator", { headless: true }, result => {
            root.busy = false;
            root.ownSerial = result.serial || "";
            root.note = result.booted
                ? "Embedded " + result.avd + " (" + result.serial + "), booted."
                : "Embedded " + result.avd + " started, still booting - the screen appears on its own.";
            root.startWatching();
        }, e => { root.busy = false; root.error = String(e); });
    }
    function stopEmbedded() {
        if (root.busy) return;
        const serial = root.ownSerial || root.device;
        root.busy = true; root.error = ""; root.note = "";
        root.app.invoke("android_stop_emulator", serial ? { serial: serial } : {}, stopped => {
            root.busy = false;
            root.note = "Stopped " + stopped + ".";
            root.ownSerial = "";
            root.stopWatching();
            root.frame = "";
        }, e => { root.busy = false; root.error = String(e); });
    }

    Text { text: "Embedded emulator view"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
    Text {
        Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
        text: "The emulator runs headless on this PC and its screen shows here, like Android Studio's embedded emulator. A few frames a second: menus read fine, action games keep the external window."
    }
    Text { visible: root.error.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.error }
    Text { visible: root.note.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12; text: root.note }
    RowLayout {
        Layout.fillWidth: true; spacing: 8
        BwButton { text: root.watching ? "Pause view" : "Show screen"; onClicked: root.watching ? root.stopWatching() : root.startWatching() }
        BwButton { text: root.busy ? "Working..." : "Start embedded"; enabled: !root.busy; onClicked: root.startEmbedded() }
        BwButton { text: "Stop"; enabled: !root.busy && (!!root.ownSerial || !!root.device); onClicked: root.stopEmbedded() }
        Item { Layout.fillWidth: true }
        BwButton { text: "Back"; enabled: root.watching; onClicked: root.key("back") }
        BwButton { text: "Home"; enabled: root.watching; onClicked: root.key("home") }
        BwButton { text: "Recents"; enabled: root.watching; onClicked: root.key("recents") }
    }
    // A phone is tall: cap the height so the dialog never grows past it,
    // and keep the aspect from the frame so taps map one to one.
    Item {
        visible: root.frame.length > 0
        Layout.alignment: Qt.AlignHCenter
        width: Math.min(270, root.frameWidth > 0 ? root.frameWidth : 270)
        height: root.frameWidth > 0 && root.frameHeight > 0 ? width * root.frameHeight / root.frameWidth : width * 16 / 9
        Image {
            id: screen
            anchors.fill: parent
            source: root.frame
            fillMode: Image.Stretch
            cache: false
        }
        MouseArea {
            anchors.fill: parent
            property real px: 0
            property real py: 0
            onPressed: m => { px = m.x; py = m.y; }
            onReleased: m => {
                const fx1 = px / width, fy1 = py / height;
                const fx2 = m.x / width, fy2 = m.y / height;
                const moved = Math.hypot(m.x - px, m.y - py);
                if (moved < 8) root.tap(fx2, fy2);
                else root.swipe(fx1, fy1, fx2, fy2);
            }
        }
    }
    Text {
        visible: root.watching && root.frame.length === 0 && root.error.length === 0
        Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
        text: "Waiting for the first frame - boot the emulator or install the game, then it appears here."
    }
    Timer {
        interval: 900; running: root.watching; repeat: true
        onTriggered: root.pollFrame()
    }
}
