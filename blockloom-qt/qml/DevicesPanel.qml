import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

RowLayout {
    id: root
    required property var app
    property var devices: []
    property string device: ""
    property var emulator: null
    property string emulatorError: ""
    property bool emuBusy: false
    property bool embed: true
    property string pendingAvd: ""
    property double startDeadline: 0
    property string newAvd: ""
    property bool busy: false
    property bool refreshing: false
    property bool logBusy: false
    property string buildState: ""
    property string buildError: ""
    property var logLines: []
    property string logcat: ""
    property bool polling: false
    property string logDevice: ""
    property int logGeneration: 0
    readonly property var selectedDevice: devices.find(d => d.serial === device) || null
    readonly property bool booting: !!selectedDevice && selectedDevice.emulator && !!emulator
        && emulator.avds.some(a => a.serial === device && !a.booted)
    readonly property bool ready: !!selectedDevice && selectedDevice.state === "device" && !booting
    readonly property bool locked: busy || emuBusy
    spacing: 0

    function acceptDevices(result) {
        devices = result || [];
        if (!devices.find(d => d.serial === device)) {
            const online = devices.filter(d => d.state === "device");
            device = online.length === 1 ? online[0].serial : "";
        }
    }
    function refresh() {
        if (refreshing || locked) return;
        refreshing = true;
        app.invoke("android_device_status", {}, result => {
            acceptDevices(result);
            app.invoke("android_emulator_status", {}, status => {
                emulator = status; refreshing = false;
                if (pendingAvd) {
                    const avd = status.avds.find(a => a.name === pendingAvd && a.serial);
                    if (avd) { device = avd.serial; pendingAvd = ""; }
                    else if (Date.now() > startDeadline) {
                        emulatorError = pendingAvd + " did not connect. Check the emulator, then retry.";
                        pendingAvd = "";
                    }
                }
            }, e => { refreshing = false; emulatorError = String(e); });
        }, e => { refreshing = false; emulatorError = String(e); });
    }
    function startAvd(name) {
        if (locked || refreshing) return;
        emuBusy = true; emulatorError = "";
        app.invoke("android_start_emulator", { avd: name, headless: embed, wait_secs: 0 }, result => {
            emuBusy = false;
            if (result.serial) device = result.serial;
            else { pendingAvd = result.avd || name; startDeadline = Date.now() + 60000; }
            refresh();
        }, e => { emuBusy = false; emulatorError = String(e); });
    }
    function stopEmu(serial) {
        if (locked || refreshing || !serial || !serial.startsWith("emulator-")) return;
        emuBusy = true; emulatorError = "";
        app.invoke("android_stop_emulator", { serial: serial }, stopped => {
            emuBusy = false;
            if (stopped === device) device = "";
            refresh();
        }, e => { emuBusy = false; emulatorError = String(e); });
    }
    function createAvd() {
        const name = newAvd.trim();
        if (locked || refreshing || !name) return;
        emuBusy = true; emulatorError = "";
        app.invoke("android_create_avd", { name: name }, () => {
            emuBusy = false; newAvd = ""; avdDialog.close(); refresh();
        }, e => { emuBusy = false; emulatorError = String(e); });
    }
    function buildAndRun() {
        if (locked || !ready || !app.appState.project) return;
        // Capture selection once: callbacks must not install onto a later selection.
        const serial = device;
        const target = selectedDevice.emulator ? "x86_64-linux-android" : "aarch64-linux-android";
        busy = true; buildError = ""; buildState = "Building APK...";
        polling = false; logGeneration++; logLines = []; logcat = ""; logDevice = serial;
        const path = app.appState.default_build_location || app.appState.default_project_location;
        app.invoke("build_game", { path: path, target: target, fast: false, hdr: false }, built => {
            buildState = "Installing and launching...";
            app.invoke("android_install", { apk: built.binary, app: built.application_id, device: serial }, launched => {
                busy = false; buildState = "Running on " + serial;
                if (device === serial) { logDevice = serial; polling = true; pollLogcat(); }
            }, e => { busy = false; buildState = ""; buildError = String(e); });
        }, e => { busy = false; buildState = ""; buildError = String(e); });
    }
    function pollLogcat() {
        if (!polling || logBusy || !visible || locked) return;
        const serial = logDevice;
        const request = logGeneration;
        logBusy = true;
        app.invoke("android_logcat_tail", { device: serial }, dump => {
            logBusy = false;
            if (!polling || serial !== logDevice || request !== logGeneration) return;
            const lines = (dump.lines || []).concat(dump.panics || []);
            if (lines.length) { logLines = logLines.concat(lines).slice(-200); logcat = logLines.join("\n"); }
            if ((dump.panics || []).length) buildError = "Native crash reported in device log.";
        }, e => {
            logBusy = false;
            if (polling && serial === logDevice && request === logGeneration) { buildError = String(e); polling = false; }
        });
    }
    onDeviceChanged: { polling = false; logGeneration++; logDevice = ""; logLines = []; logcat = ""; buildState = ""; buildError = ""; }
    Component.onCompleted: refresh()
    onVisibleChanged: { if (visible) refresh(); }

    Rectangle {
        Layout.fillHeight: true
        Layout.preferredWidth: Math.max(240, Math.min(300, root.width * 0.38))
        color: Theme.panel
        ScrollView {
            id: sidebar
            anchors.fill: parent; clip: true; contentWidth: availableWidth
            ColumnLayout {
                width: sidebar.availableWidth; spacing: 12
                RowLayout {
                    Layout.fillWidth: true; Layout.margins: 16; Layout.bottomMargin: 0
                    Text { Layout.fillWidth: true; text: "Android devices"; color: Theme.text; font.pixelSize: 15; font.weight: Font.DemiBold }
                    IconButton { iconName: "refresh-cw"; tip: "Refresh devices"; enabled: !root.refreshing && !root.locked; onClicked: root.refresh() }
                }
                Text {
                    visible: !root.devices.length; Layout.fillWidth: true; Layout.leftMargin: 16; Layout.rightMargin: 16
                    text: "No devices connected"; color: Theme.textDim; font.pixelSize: 12; wrapMode: Text.WordWrap
                }
                Repeater {
                    model: root.devices
                    delegate: ItemDelegate {
                        required property var modelData
                        Layout.fillWidth: true; Layout.preferredHeight: 62
                        enabled: !root.locked
                        onClicked: root.device = modelData.serial
                        background: Rectangle {
                            color: root.device === modelData.serial ? Theme.panelRaised : parent.hovered ? Theme.panelRaised : "transparent"
                            Rectangle { width: 3; height: parent.height; color: Theme.accent; visible: root.device === modelData.serial }
                        }
                        contentItem: RowLayout {
                            spacing: 10
                            LucideIcon { name: modelData.emulator ? "monitor" : "plug-zap"; implicitWidth: 20; implicitHeight: 20; color: root.device === modelData.serial ? Theme.accent : Theme.textDim }
                            ColumnLayout {
                                Layout.fillWidth: true; spacing: 3
                                Text { Layout.fillWidth: true; text: modelData.serial; elide: Text.ElideMiddle; color: Theme.text; font.pixelSize: 12 }
                                Text {
                                    Layout.fillWidth: true; elide: Text.ElideRight; font.pixelSize: 11
                                    color: modelData.state === "device" ? Theme.textDim : Theme.warning
                                    text: modelData.state === "device" ? (modelData.emulator ? "Emulator connected" : "Phone connected")
                                        : modelData.state === "unauthorized" ? "Authorize USB debugging on phone" : modelData.state
                                }
                            }
                            LucideIcon { visible: root.device === modelData.serial; name: "check"; implicitWidth: 16; implicitHeight: 16; color: Theme.accent }
                        }
                    }
                }
                Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.borderSoft }
                ColumnLayout {
                    Layout.fillWidth: true; Layout.leftMargin: 16; Layout.rightMargin: 16; spacing: 10
                    Text { text: "Deploy project"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
                    Text {
                        Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                        text: root.booting ? "Emulator is booting..." : root.selectedDevice ? (root.selectedDevice.emulator ? "Android x86_64" : "Android ARM64") : "Select a connected device"
                    }
                    BwButton {
                        objectName: "deployButton"
                        Layout.fillWidth: true; iconName: "play"; primary: true
                        text: root.busy ? "Deploying..." : "Build and run"
                        enabled: root.ready && !root.locked && !!root.app.appState.project
                        onClicked: root.buildAndRun()
                    }
                    BusyIndicator { visible: root.busy; running: visible; Layout.alignment: Qt.AlignHCenter; implicitWidth: 24; implicitHeight: 24 }
                    Text { visible: !!root.buildState; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere; text: root.buildState; color: Theme.textDim; font.pixelSize: 11 }
                    Text { visible: !!root.buildError; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere; text: root.buildError; color: Theme.danger; font.pixelSize: 12 }
                }
                Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.borderSoft }
                ColumnLayout {
                    Layout.fillWidth: true; Layout.leftMargin: 16; Layout.rightMargin: 16; spacing: 10
                    RowLayout {
                        Layout.fillWidth: true
                        Text { Layout.fillWidth: true; text: "Emulators"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
                        IconButton { objectName: "newAvdButton"; iconName: "plus"; tip: "Create virtual device"; enabled: !!root.emulator && root.emulator.available && !root.locked && !root.refreshing; onClicked: { root.emulatorError = ""; avdDialog.open(); } }
                    }
                    Text { text: "Launch mode"; color: Theme.textDim; font.pixelSize: 11 }
                    RowLayout {
                        Layout.fillWidth: true; spacing: 0
                        BwButton { Layout.fillWidth: true; text: "Embedded"; primary: root.embed; enabled: !root.locked; onClicked: root.embed = true }
                        BwButton { Layout.fillWidth: true; text: "Window"; primary: !root.embed; enabled: !root.locked; onClicked: root.embed = false }
                    }
                    Text {
                        visible: !!root.emulator && (!root.emulator.available || !root.emulator.avds.length)
                        Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: 12; color: Theme.textDim
                        text: root.emulator && !root.emulator.available ? root.emulator.detail : "No virtual devices"
                    }
                    Repeater {
                        model: root.emulator ? root.emulator.avds : []
                        delegate: RowLayout {
                            required property var modelData
                            Layout.fillWidth: true
                            ColumnLayout {
                                Layout.fillWidth: true; spacing: 3
                                Text { Layout.fillWidth: true; elide: Text.ElideRight; text: modelData.name; color: Theme.text; font.pixelSize: 12 }
                                Text { text: modelData.serial ? (modelData.booted ? "Running" : "Booting...") : root.pendingAvd === modelData.name ? "Starting..." : "Stopped"; color: Theme.textDim; font.pixelSize: 11 }
                            }
                            IconButton {
                                iconName: modelData.serial ? "square" : "play"
                                tip: modelData.serial ? "Stop " + modelData.name : "Start " + modelData.name
                                enabled: !root.locked && !root.refreshing && root.pendingAvd !== modelData.name
                                onClicked: modelData.serial ? root.stopEmu(modelData.serial) : root.startAvd(modelData.name)
                            }
                        }
                    }
                    BusyIndicator { visible: root.emuBusy; running: visible; Layout.alignment: Qt.AlignHCenter; implicitWidth: 24; implicitHeight: 24 }
                    Text { visible: !!root.emulatorError; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere; text: root.emulatorError; color: Theme.danger; font.pixelSize: 12 }
                }
                Item { Layout.preferredHeight: 16 }
            }
        }
    }
    Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; color: Theme.borderSoft }
    Rectangle {
        Layout.fillWidth: true; Layout.fillHeight: true
        color: Theme.window
        ColumnLayout {
        anchors.fill: parent; spacing: 0
        DevicePanel {
            id: screen
            objectName: "deviceScreen"
            Layout.fillWidth: true; Layout.fillHeight: true
            app: root.app; device: root.device; connected: root.ready
            active: root.visible && !root.locked
        }
        Rectangle { visible: !!root.logDevice; Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.borderSoft }
        RowLayout {
            visible: !!root.logDevice; Layout.fillWidth: true; Layout.leftMargin: 16; Layout.rightMargin: 12
            Text { Layout.fillWidth: true; text: "Device log"; color: Theme.textDim; font.pixelSize: 12 }
            IconButton { iconName: root.polling ? "pause" : "play"; tip: root.polling ? "Pause device log" : "Resume device log"; enabled: !!root.logDevice; onClicked: root.polling = !root.polling }
            IconButton { iconName: "trash-2"; tip: "Clear visible device log"; onClicked: { root.logLines = []; root.logcat = ""; } }
        }
        ScrollView {
            id: logScroll
            visible: !!root.logDevice; Layout.fillWidth: true; Layout.preferredHeight: 150; Layout.maximumHeight: root.height * 0.3
            clip: true; contentWidth: availableWidth
            TextArea {
                width: logScroll.availableWidth; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere
                text: root.logcat; color: Theme.textDim; font.family: "monospace"; font.pixelSize: 11
                placeholderText: "No device output"
                background: null
            }
        }
    }
    }
    BwDialog {
        id: avdDialog
        objectName: "avdDialog"
        title: "Create virtual device"; implicitWidth: Math.min(380, root.width)
        showClose: !root.emuBusy
        standardButtons: Dialog.NoButton
        closePolicy: root.emuBusy ? Popup.NoAutoClose : Popup.CloseOnEscape | Popup.CloseOnPressOutside
        contentItem: ColumnLayout {
            spacing: 12
            BwTextField { Layout.fillWidth: true; text: root.newAvd; placeholderText: "Device name"; enabled: !root.emuBusy; onTextChanged: root.newAvd = text; onAccepted: root.createAvd() }
            Text { visible: !!root.emulatorError; Layout.fillWidth: true; text: root.emulatorError; color: Theme.danger; wrapMode: Text.WrapAnywhere; font.pixelSize: 12 }
            RowLayout {
                Layout.alignment: Qt.AlignRight
                BwButton { text: "Cancel"; enabled: !root.emuBusy; onClicked: avdDialog.close() }
                BwButton { text: root.emuBusy ? "Creating..." : "Create"; iconName: "plus"; primary: true; enabled: !root.locked && !!root.newAvd.trim(); onClicked: root.createAvd() }
            }
        }
    }
    Timer { interval: 4000; running: root.visible && !root.locked; repeat: true; onTriggered: root.refresh() }
    Timer { interval: 2000; running: root.visible && root.polling && !root.locked; repeat: true; onTriggered: root.pollLogcat() }
}
