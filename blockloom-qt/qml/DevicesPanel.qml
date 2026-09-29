import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The Devices tab: phones and emulators live here, not in the Build
// dialog. AVDs boot here (embedded or in their own window), the running
// screen shows here, and Build and run puts the game on the selected
// device and streams its log back. The Build dialog only builds; App
// settings only keeps the SDK toolchain.
RowLayout {
    id: root
    required property var app
    property var devices: []
    property string device: ""
    property var emulator: null
    property string emulatorError: ""
    property bool emuBusy: false
    property bool embed: true
    property string newAvd: ""
    property bool busy: false
    property string buildState: ""
    property string buildError: ""
    property var logLines: []
    property string logcat: ""
    property bool polling: false
    // Whatever the screen is actually showing, whoever booted it.
    readonly property string activeSerial: screen.ownSerial || screen.device || root.device

    function refresh() {
        root.emulatorError = ""; root.buildError = "";
        app.invoke("android_device_status", {}, result => {
            devices = result || [];
            if (devices.length === 1) device = devices[0].serial;
            else if (!devices.find(d => d.serial === device)) device = "";
        }, e => root.emulatorError = String(e));
        refreshEmulator();
    }
    function refreshEmulator() {
        app.invoke("android_emulator_status", {}, result => {
            emulator = result;
        }, e => root.emulatorError = String(e));
    }
    function selectedIsEmulator() {
        const found = devices.find(d => d.serial === root.activeSerial);
        return found ? !!found.emulator : true;
    }
    function startAvd(name) {
        if (root.emuBusy) return;
        root.emuBusy = true; root.emulatorError = "";
        // Embedded boots hide the host window: the screen beside this
        // column shows it instead, like Android Studio's tool window.
        app.invoke("android_start_emulator", { avd: name, headless: root.embed }, result => {
            root.emuBusy = false;
            // The screen follows even though its own button didn't boot it.
            if (result.serial) root.device = result.serial;
            refreshEmulator();
            refreshDevicesQuiet();
        }, e => { root.emuBusy = false; root.emulatorError = String(e); });
    }
    function stopEmu(serial) {
        if (root.emuBusy) return;
        root.emuBusy = true; root.emulatorError = "";
        app.invoke("android_stop_emulator", serial ? { serial: serial } : {}, stopped => {
            root.emuBusy = false;
            if (stopped === root.device) root.device = "";
            refreshEmulator();
            refreshDevicesQuiet();
        }, e => { root.emuBusy = false; root.emulatorError = String(e); });
    }
    function createAvd() {
        if (root.emuBusy) return;
        const name = root.newAvd.trim();
        root.emuBusy = true; root.emulatorError = "";
        app.invoke("android_create_avd", name ? { name: name } : {}, created => {
            root.emuBusy = false; root.newAvd = "";
            refreshEmulator();
        }, e => { root.emuBusy = false; root.emulatorError = String(e); });
    }
    function refreshDevicesQuiet() {
        app.invoke("android_device_status", {}, result => {
            devices = result || [];
            if (devices.length === 1) device = devices[0].serial;
        }, e => {});
    }
    function buildAndRun() {
        if (root.busy) return;
        const serial = root.activeSerial;
        if (!serial && devices.length !== 1) {
            root.buildError = "No device: start an emulator above, or connect a phone.";
            return;
        }
        root.busy = true; root.buildError = ""; root.buildState = "";
        root.logLines = []; root.logcat = ""; root.polling = false;
        // Emulators take the x86_64 build, phones the arm64 one. Debug
        // signing is automatic; a release key with remembered passwords
        // works too, and anything else fails with where to type them.
        const target = root.selectedIsEmulator() ? "x86_64-linux-android" : "aarch64-linux-android";
        const path = app.appState.default_build_location || app.appState.default_project_location;
        root.buildState = "Building for " + target + "...";
        app.invoke("build_game", { path: path, target: target, fast: false, hdr: false }, built => {
            root.buildState = "Installing on " + (serial || devices[0].serial) + "...";
            app.invoke("android_install", { apk: built.binary, app: built.application_id, device: (serial || devices[0].serial) || undefined }, launched => {
                root.busy = false;
                root.buildState = "Launched " + launched.component + (launched.device ? " on " + launched.device : "") + ".";
                root.polling = true;
                root.pollLogcat();
            }, e => { root.busy = false; root.buildError = String(e); root.buildState = ""; });
        }, e => { root.busy = false; root.buildError = String(e); root.buildState = ""; });
    }
    function pollLogcat() {
        if (!root.polling) return;
        const serial = root.activeSerial;
        app.invoke("android_logcat_tail", serial ? { device: serial } : {}, dump => {
            if (!root.polling) return;
            const lines = (dump.lines || []).concat(dump.panics || []);
            if (lines.length) { logLines = logLines.concat(lines).slice(-200); logcat = logLines.join("\n"); }
            if ((dump.panics || []).length) root.buildError = "The device log reports a native crash - see below.";
        }, e => { if (root.polling) root.buildError = String(e); });
    }

    Component.onCompleted: root.refresh()

    ColumnLayout {
        Layout.fillHeight: true; Layout.preferredWidth: 360; Layout.topMargin: 8; Layout.leftMargin: 8; spacing: 8
        Text { visible: root.emulatorError.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.emulatorError }
        Text { text: "Devices"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold }
        Text { visible: root.devices.length === 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12; text: "Nothing attached: start an emulator below, or plug a phone in." }
        Repeater {
            model: root.devices
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true; spacing: 8
                Rectangle {
                    width: 8; height: 8; radius: 4; anchors.verticalCenter: parent.verticalCenter
                    color: modelData.state === "device" ? Theme.accent : Theme.warning
                }
                Text {
                    Layout.fillWidth: true; elide: Text.ElideRight; font.pixelSize: 12
                    color: modelData.serial === root.device ? Theme.text : Theme.textDim
                    text: modelData.serial + (modelData.emulator ? " (emulator)" : "") + " - " + modelData.state
                }
                BwButton {
                    visible: modelData.serial !== root.device; text: "Use"
                    onClicked: root.device = modelData.serial
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true; spacing: 8
            BwButton { text: "Refresh"; onClicked: root.refresh() }
            Item { Layout.fillWidth: true }
        }
        Text { text: "Emulator"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold }
        Text { visible: !!root.emulator; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: root.emulator ? (root.emulator.available ? root.emulator.detail + (root.emulator.avds.length ? "" : " No AVDs yet - create one below.") : root.emulator.detail) : "" }
        Repeater {
            model: root.emulator ? root.emulator.avds : []
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true; spacing: 8
                Text {
                    Layout.fillWidth: true; elide: Text.ElideRight; color: Theme.text; font.pixelSize: 12
                    text: modelData.name + (modelData.serial ? " - " + modelData.serial + (modelData.booted ? " (booted)" : " (booting...)") : " - off")
                }
                BwButton { visible: !modelData.serial; text: "Start"; enabled: !root.emuBusy; onClicked: root.startAvd(modelData.name) }
                BwButton { visible: !!modelData.serial; text: "Stop"; enabled: !root.emuBusy; onClicked: root.stopEmu(modelData.serial) }
            }
        }
        BwCheckBox { text: "Start embedded (no host window - the screen shows here)"; checked: root.embed; onToggled: root.embed = checked }
        RowLayout {
            Layout.fillWidth: true; spacing: 8
            BwTextField { Layout.fillWidth: true; text: root.newAvd; placeholderText: "New AVD name (empty means blockloom)"; onTextChanged: root.newAvd = text; onAccepted: root.createAvd() }
            BwButton { text: root.emuBusy ? "Working..." : "Create AVD"; enabled: !root.emuBusy; onClicked: root.createAvd() }
        }
        Text { text: "Run on device"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold }
        Text { visible: root.buildError.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.buildError }
        Text { visible: root.buildState.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12; text: root.buildState }
        BwButton {
            text: root.busy ? "Working..." : "Build and run on " + (root.activeSerial || "device")
            primary: true; enabled: !root.busy
            onClicked: root.buildAndRun()
        }
        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: "Builds the emulator (x86_64) or phone (arm64) APK, installs it and launches it. The device log streams below and into the RunLog." }
        ScrollView {
            id: logScroll
            visible: root.logcat.length > 0
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            TextEdit {
                width: logScroll.availableWidth
                readOnly: true; selectByMouse: true; wrapMode: Text.Wrap
                color: Theme.textDim; font.family: "monospace"; font.pixelSize: 11
                text: root.logcat
                onTextChanged: logScroll.contentItem.contentY = Math.max(0, logScroll.contentItem.contentHeight - logScroll.height)
            }
        }
    }
    ScrollView {
        Layout.fillWidth: true; Layout.fillHeight: true; clip: true
        contentWidth: availableWidth
        ColumnLayout {
            width: parent.width
            DevicePanel {
                id: screen
                Layout.fillWidth: true; Layout.topMargin: 8
                app: root.app
                device: root.device
            }
        }
    }
    Timer {
        interval: 2000; running: root.polling; repeat: true
        onTriggered: root.pollLogcat()
    }
}
