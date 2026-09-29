import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// App-level settings, kept beside projects.json: the Android SDK/NDK rows,
// the license stamp and the toolchain probes. Per-project Android rows
// (applicationId, version) live in Project settings instead.
BwDialog {
    id: root
    required property var app
    property var android: null
    property string androidError: ""
    property var devices: []
    property string deviceError: ""
    property var emulator: null
    property string emulatorError: ""
    property string emulatorNote: ""
    property bool emuBusy: false
    property bool emuPolling: false
    property string emuTarget: ""
    property string licenseText: ""
    property bool licensesAccepted: false
    property bool busy: false
    property bool sdkBusy: false
    property string sdkResult: ""
    title: "App settings"
    standardButtons: Dialog.NoButton
    width: 560

    onOpened: refresh()
    function refresh() {
        androidError = ""; deviceError = ""; sdkResult = "";
        emulatorError = ""; emulatorNote = "";
        app.invoke("android_status", {}, result => {
            android = result;
            licensesAccepted = !!result.licenses_accepted;
            sdkPathField.text = result.sdk_path || "";
            ndkPathField.text = result.ndk_path || "";
        }, e => androidError = String(e));
        app.invoke("android_device_status", {}, result => {
            devices = result || [];
        }, e => deviceError = String(e));
        app.invoke("android_emulator_status", {}, result => {
            emulator = result;
        }, e => emulatorError = String(e));
    }
    function refreshEmulator() {
        app.invoke("android_emulator_status", {}, result => {
            emulator = result;
        }, e => emulatorError = String(e));
    }
    function pollEmulator() {
        if (!emuPolling) return;
        app.invoke("android_emulator_status", {}, result => {
            emulator = result;
            const target = (result.avds || []).find(a => a.name === emuTarget);
            if (!target || target.booted) emuPolling = false;
        }, e => { emuPolling = false; emulatorError = String(e); });
    }
    function startAvd(name) {
        if (emuBusy) return;
        emuBusy = true; emulatorError = ""; emulatorNote = "";
        app.invoke("android_start_emulator", { avd: name }, result => {
            emuBusy = false;
            emulatorNote = result.booted
                ? "Started " + result.avd + " (" + result.serial + "), booted."
                : "Started " + result.avd + (result.serial ? " (" + result.serial + ")" : "") + ", still booting - this refreshes on its own.";
            emuTarget = result.avd;
            emuPolling = !result.booted;
            refreshEmulator();
        }, e => { emuBusy = false; emulatorError = String(e); });
    }
    function stopEmu(serial) {
        if (emuBusy) return;
        emuBusy = true; emulatorError = ""; emulatorNote = ""; emuPolling = false;
        app.invoke("android_stop_emulator", { serial: serial }, stopped => {
            emuBusy = false;
            emulatorNote = "Stopped " + stopped + ".";
            refreshEmulator();
        }, e => { emuBusy = false; emulatorError = String(e); });
    }
    function createAvd() {
        if (emuBusy) return;
        emuBusy = true; emulatorError = ""; emulatorNote = "";
        const name = newAvdField.text.trim();
        app.invoke("android_create_avd", name ? { name: name } : {}, created => {
            emuBusy = false; newAvdField.text = "";
            emulatorNote = "Created " + created + ".";
            refreshEmulator();
        }, e => { emuBusy = false; emulatorError = String(e); });
    }
    function row(ok, detail) { return (ok ? "OK  " : "Missing  ") + detail; }

    ScrollView {
        anchors.fill: parent; clip: true
        contentWidth: availableWidth
        ColumnLayout {
            width: parent.width - 12; spacing: 8
            Text { visible: root.androidError.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.androidError }
            Text { text: "Android"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "A project builds into an installable APK from Windows, Linux or macOS. No editor runs on the device: Android is a Build dialog row, never a Play path." }
            Text { text: "SDK path"; color: Theme.textDim; font.pixelSize: 12 }
            RowLayout {
                Layout.fillWidth: true
                BwTextField { id: sdkPathField; Layout.fillWidth: true; placeholderText: "Default SDK location"
                    onEditingFinished: root.app.invoke("android_set_sdk_path", { path: text.trim() }, resolved => { sdkPathField.text = resolved; root.refresh(); }, e => root.androidError = String(e)) }
                IconButton { iconName: "folder-open"; tip: "Browse for the SDK"; implicitWidth: 34; implicitHeight: 34; onClicked: sdkBrowse.open() }
            }
            Text { text: "NDK path"; color: Theme.textDim; font.pixelSize: 12 }
            RowLayout {
                Layout.fillWidth: true
                BwTextField { id: ndkPathField; Layout.fillWidth: true; placeholderText: "Pinned NDK inside the SDK"
                    onEditingFinished: root.app.invoke("android_set_ndk_path", { path: text.trim() }, resolved => { ndkPathField.text = resolved; root.refresh(); }, e => root.androidError = String(e)) }
                IconButton { iconName: "folder-open"; tip: "Browse for the NDK"; implicitWidth: 34; implicitHeight: 34; onClicked: ndkBrowse.open() }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "Paths are settings, not environment lookups: Blockloom never reads ANDROID_HOME, ANDROID_SDK_ROOT or ANDROID_NDK_HOME." }
            Repeater {
                model: root.android ? [
                    { label: "JDK", ok: root.android.jdk.ok, detail: root.android.jdk.detail },
                    { label: "SDK", ok: root.android.sdk.ok, detail: root.android.sdk.detail },
                    { label: "NDK", ok: root.android.ndk.ok, detail: root.android.ndk.detail },
                    { label: "Rust arm64", ok: root.android.rust_arm64.ok, detail: root.android.rust_arm64.detail },
                    { label: "Rust emulator", ok: root.android.rust_emulator.ok, detail: root.android.rust_emulator.detail }
                ] : []
                delegate: Text {
                    required property var modelData
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: 12
                    color: modelData.ok ? Theme.text : Theme.warning
                    text: modelData.label + ": " + root.row(modelData.ok, modelData.detail)
                }
            }
            Text { visible: !!root.android; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: root.android ? ("arm64 ready: " + (root.android.ready_arm64 ? "yes" : root.android.note_arm64) + "\nEmulator ready: " + (root.android.ready_emulator ? "yes" : root.android.note_emulator)) : "" }
            RowLayout {
                BwButton {
                    text: root.sdkBusy ? "Installing..." : "Install / update SDK"
                    enabled: !root.sdkBusy
                    onClicked: {
                        root.sdkBusy = true; root.sdkResult = ""; root.androidError = "";
                        root.app.invoke("android_install_sdk", {}, result => {
                            root.sdkBusy = false;
                            root.sdkResult = "Installed: " + (result.installed.length ? result.installed.join(", ") : "nothing new") + ". " + (result.still_missing.length ? "Still missing: " + result.still_missing.join(" ") : "All rows probe green.");
                            root.refresh();
                        }, e => { root.sdkBusy = false; root.androidError = String(e); });
                    }
                }
                BwButton {
                    text: "Show licenses"
                    onClicked: root.app.invoke("android_accept_licenses", { accept: false }, result => { root.licenseText = result.text; root.licensesAccepted = !!result.accepted; }, e => root.androidError = String(e))
                }
                BwButton {
                    text: "Accept licenses"; enabled: !root.licensesAccepted
                    onClicked: root.app.invoke("android_accept_licenses", { accept: true }, result => { root.licenseText = result.text; root.licensesAccepted = true; root.refresh(); }, e => root.androidError = String(e))
                }
            }
            Text { visible: root.sdkResult.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12; text: root.sdkResult }
            Text { visible: root.licensesAccepted; Layout.fillWidth: true; color: Theme.textDim; font.pixelSize: 12; text: "SDK licenses accepted." }
            TextEdit { visible: root.licenseText.length > 0; Layout.fillWidth: true; Layout.preferredHeight: 120; readOnly: true; selectByMouse: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11; text: root.licenseText }
            Text { text: "Devices"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold }
            Text { visible: root.deviceError.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.deviceError }
            Text { visible: root.deviceError.length === 0 && root.devices.length === 0; Layout.fillWidth: true; color: Theme.textDim; font.pixelSize: 12; text: "No devices: connect a phone or start an emulator, then Refresh." }
            Repeater {
                model: root.devices
                delegate: Text {
                    required property var modelData
                    Layout.fillWidth: true; color: Theme.text; font.pixelSize: 12
                    text: modelData.serial + " (" + modelData.state + (modelData.emulator ? ", emulator" : "") + ")"
                }
            }
            Text { text: "Emulator"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold }
            Text { visible: root.emulatorError.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.emulatorError }
            Text { visible: root.emulatorNote.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12; text: root.emulatorNote }
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
            RowLayout {
                Layout.fillWidth: true; spacing: 8
                BwTextField { id: newAvdField; Layout.fillWidth: true; placeholderText: "New AVD name (empty means blockloom)"; onAccepted: root.createAvd() }
                BwButton { text: root.emuBusy ? "Working..." : "Create AVD"; enabled: !root.emuBusy; onClicked: root.createAvd() }
            }
            RowLayout {
                Layout.alignment: Qt.AlignRight; Layout.topMargin: 8; spacing: 8
                BwButton { text: "Refresh"; onClicked: root.refresh() }
                BwButton { text: "Done"; primary: true; onClicked: root.close() }
            }
        }
    }
    onClosed: emuPolling = false
    Timer {
        interval: 10000; running: root.emuPolling; repeat: true
        onTriggered: root.pollEmulator()
    }
    FolderDialog {
        id: sdkBrowse
        title: "Android SDK location"
        currentFolder: root.app.toFileUrl(sdkPathField.text)
        onAccepted: root.app.invoke("android_set_sdk_path", { path: root.app.fromFileUrl(selectedFolder) }, resolved => { sdkPathField.text = resolved; root.refresh(); }, e => root.androidError = String(e))
    }
    FolderDialog {
        id: ndkBrowse
        title: "Android NDK location"
        currentFolder: root.app.toFileUrl(ndkPathField.text)
        onAccepted: root.app.invoke("android_set_ndk_path", { path: root.app.fromFileUrl(selectedFolder) }, resolved => { ndkPathField.text = resolved; root.refresh(); }, e => root.androidError = String(e))
    }
}
