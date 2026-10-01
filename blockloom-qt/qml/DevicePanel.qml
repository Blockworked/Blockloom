import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Mirrors only the selected device. Emulator lifecycle belongs to DevicesPanel.
ColumnLayout {
    id: root
    required property var app
    property string device: ""
    property bool connected: false
    property bool active: true
    property string frame: ""
    property int frameWidth: 0
    property int frameHeight: 0
    property bool watching: false
    property bool resumeWatching: false
    property bool frameBusy: false
    property bool inputBusy: false
    property int generation: 0
    property string error: ""
    spacing: 0

    function resetView() {
        generation++;
        frame = ""; error = "";
        watching = false; resumeWatching = false;
    }
    onDeviceChanged: resetView()
    onConnectedChanged: { if (!connected) resetView(); }
    onActiveChanged: {
        if (!active) {
            resumeWatching = watching; generation++; watching = false;
        } else if (resumeWatching) {
            resumeWatching = false; startWatching();
        }
    }

    function startWatching() {
        if (!connected || !device || !active) return;
        watching = true; error = "";
        pollFrame();
    }
    function stopWatching() { generation++; watching = false; resumeWatching = false; }
    function pollFrame() {
        if (!watching || frameBusy || !active || !connected) return;
        const request = generation;
        const serial = device;
        frameBusy = true;
        app.invoke("android_mirror_frame", { device: serial }, result => {
            frameBusy = false;
            if (request !== generation || serial !== device || !watching) return;
            frame = result.image; frameWidth = result.width; frameHeight = result.height;
            error = "";
        }, e => {
            frameBusy = false;
            if (request !== generation || !watching) return;
            error = String(e); watching = false;
        });
    }
    function input(command, args) {
        if (!watching || !connected || inputBusy || !active) return;
        const request = generation;
        args.device = device;
        inputBusy = true;
        app.invoke(command, args, () => {
            inputBusy = false;
            if (request === generation) pollFrame();
        }, e => {
            inputBusy = false;
            if (request === generation) error = String(e);
        });
    }
    function tap(x, y) { input("android_mirror_tap", { x: x, y: y }); }
    function swipe(x1, y1, x2, y2) { input("android_mirror_swipe", { x1: x1, y1: y1, x2: x2, y2: y2 }); }
    function key(code) { input("android_mirror_key", { code: code }); }

    RowLayout {
        Layout.fillWidth: true; Layout.margins: 16; spacing: 8
        LucideIcon { name: "monitor"; implicitWidth: 18; implicitHeight: 18; color: Theme.textDim }
        Text { text: "Device screen"; color: Theme.text; font.pixelSize: 14; font.weight: Font.DemiBold }
        Item { Layout.fillWidth: true }
        IconButton {
            objectName: "mirrorToggle"
            visible: root.frame.length > 0 || root.watching
            iconName: root.watching ? "pause" : "monitor"
            tip: root.watching ? "Pause screen preview" : "Show selected device screen"
            enabled: root.connected && root.active
            onClicked: root.watching ? root.stopWatching() : root.startWatching()
        }
    }
    Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.borderSoft }
    Item {
        id: viewport
        Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumHeight: 180
        clip: true
        ColumnLayout {
            anchors.centerIn: parent; width: Math.max(0, Math.min(320, parent.width - 40)); spacing: 12
            visible: !root.frame.length
            LucideIcon { Layout.alignment: Qt.AlignHCenter; name: root.connected ? "monitor" : "unplug"; implicitWidth: 36; implicitHeight: 36; color: Theme.textDim }
            Text {
                Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.WordWrap
                text: !root.connected ? (root.device ? "Device unavailable" : "No device selected") : root.watching ? "Connecting to screen..." : "Preview paused"
                color: Theme.text; font.pixelSize: 14
            }
            BwButton {
                objectName: "showScreenButton"
                Layout.alignment: Qt.AlignHCenter; visible: root.connected && !root.watching
                text: "Show screen"; iconName: "monitor"; onClicked: root.startWatching()
            }
        }
        Item {
            id: phone
            objectName: "mirrorImage"
            visible: root.frame.length > 0
            anchors.centerIn: parent
            readonly property real ratio: root.frameWidth > 0 && root.frameHeight > 0 ? root.frameWidth / root.frameHeight : 9 / 16
            width: Math.max(0, Math.min(viewport.width - 40, (viewport.height - 32) * ratio))
            height: width / ratio
            Image { anchors.fill: parent; source: root.frame; fillMode: Image.Stretch; cache: false; smooth: true }
            MouseArea {
                anchors.fill: parent; enabled: root.watching && !root.inputBusy
                property real px: 0
                property real py: 0
                onPressed: m => { px = m.x; py = m.y; }
                onReleased: m => {
                    const x = Math.max(0, Math.min(1, m.x / width));
                    const y = Math.max(0, Math.min(1, m.y / height));
                    if (Math.hypot(m.x - px, m.y - py) < 8) root.tap(x, y);
                    else root.swipe(px / width, py / height, x, y);
                }
            }
        }
    }
    Text {
        visible: root.error.length > 0; Layout.fillWidth: true; Layout.margins: 16
        text: root.error; color: Theme.danger; font.pixelSize: 12; wrapMode: Text.WrapAnywhere
    }
    Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.borderSoft }
    Item {
        Layout.fillWidth: true; Layout.margins: 12; implicitHeight: navigation.implicitHeight
        Text {
            anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
            width: Math.max(0, (parent.width - navigation.width) / 2 - 12)
            elide: Text.ElideRight; font.pixelSize: 11; color: Theme.textDim
            text: root.watching ? "Live preview" : "Preview paused"
        }
        RowLayout {
            id: navigation
            objectName: "deviceNavigation"
            anchors.centerIn: parent
            IconButton { iconName: "arrow-left"; tip: "Android Back"; enabled: root.watching && !root.inputBusy; onClicked: root.key("back") }
            IconButton { iconName: "house"; tip: "Android Home"; enabled: root.watching && !root.inputBusy; onClicked: root.key("home") }
            IconButton { iconName: "panels-top-left"; tip: "Android recent apps"; enabled: root.watching && !root.inputBusy; onClicked: root.key("recents") }
        }
    }
    Timer { interval: 1000; running: root.watching && root.active; repeat: true; onTriggered: root.pollFrame() }
}
