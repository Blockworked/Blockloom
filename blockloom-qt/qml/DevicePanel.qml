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
    property string transport: ""
    property bool watching: false
    property bool resumeWatching: false
    property bool pressed: false
    property string error: ""
    // Frames are as wide as the panel shows them, in device pixels.
    readonly property int wantedWidth: Math.max(240, Math.min(1080, Math.round(Math.min(viewport.width - 40, (viewport.height - 32) * 9 / 16) * Screen.devicePixelRatio)))
    spacing: 0

    function stopStream() { if (app) app.watchScreen("", 0); }
    function resetView() {
        frame = ""; error = ""; transport = "";
        stopStream();
        watching = false; resumeWatching = false; pressed = false;
    }
    onDeviceChanged: resetView()
    onConnectedChanged: { if (!connected) resetView(); }
    onActiveChanged: {
        if (!active) {
            resumeWatching = watching; stopStream(); watching = false; pressed = false;
        } else if (resumeWatching) {
            resumeWatching = false; startWatching();
        }
    }

    // The backend pushes each frame as the device finishes it; one for a
    // device no longer shown, or after pausing, is not shown.
    Connections {
        target: root.app
        function onScreenFrameChanged() { root.take(root.app.screenFrame); }
    }
    function take(update) {
        if (!update || !watching) return;
        if (update.error) { error = String(update.error); watching = false; return; }
        if (update.serial !== device) return;
        frame = update.image; frameWidth = update.width; frameHeight = update.height;
        transport = update.transport || ""; error = "";
    }
    function startWatching() {
        if (!connected || !device || !active) return;
        watching = true; error = "";
        app.watchScreen(device, wantedWidth);
    }
    function stopWatching() { stopStream(); watching = false; resumeWatching = false; pressed = false; }
    // Points are fractions of the frame; the backend maps them to device pixels.
    function touch(phase, x, y) {
        if (!watching || !connected || !active) return;
        app.screenInput({ type: "touch", phase: phase, x: x, y: y });
    }
    function key(code) {
        if (!watching || !connected || !active) return;
        app.screenInput({ type: "key", code: code });
    }

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
            Image { anchors.fill: parent; source: root.frame; fillMode: Image.Stretch; cache: false; smooth: true; asynchronous: true }
            MouseArea {
                anchors.fill: parent; enabled: root.watching
                function fraction(v, extent) { return Math.max(0, Math.min(1, v / extent)); }
                onPressed: m => { root.pressed = true; root.touch("down", fraction(m.x, width), fraction(m.y, height)); }
                onPositionChanged: m => { if (root.pressed) root.touch("move", fraction(m.x, width), fraction(m.y, height)); }
                onReleased: m => { if (root.pressed) root.touch("up", fraction(m.x, width), fraction(m.y, height)); root.pressed = false; }
                onCanceled: { if (root.pressed) root.touch("up", 0.5, 0.5); root.pressed = false; }
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
            text: root.watching ? (root.transport === "grpc" ? "Live (emulator stream)" : root.transport ? "Live (adb capture)" : "Live preview") : "Preview paused"
        }
        RowLayout {
            id: navigation
            objectName: "deviceNavigation"
            anchors.centerIn: parent
            IconButton { iconName: "arrow-left"; tip: "Android Back"; enabled: root.watching; onClicked: root.key("back") }
            IconButton { iconName: "house"; tip: "Android Home"; enabled: root.watching; onClicked: root.key("home") }
            IconButton { iconName: "panels-top-left"; tip: "Android recent apps"; enabled: root.watching; onClicked: root.key("recents") }
        }
    }
}
