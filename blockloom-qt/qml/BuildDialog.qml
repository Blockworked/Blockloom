import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Building the project into a game somebody else can run: which platform, and
// where to put it. Every platform that isn't available says why.
BwDialog {
    id: root
    required property var app
    property var targets: []
    property string triple: ""
    property string error: ""
    property var built: null
    property bool busy: false
    property bool fast: false
    property bool hdr: true
    // The browser build: one .html file, SDR and the VM only.
    readonly property bool web: !!chosen && chosen.triple.indexOf("wasm32") === 0
    // An APK cross-built through the NDK: SDR-only like web, plus installable.
    readonly property bool android: !!chosen && chosen.triple.indexOf("linux-android") >= 0
    readonly property var chosen: targets.find(t => t.triple === triple) || null
    // Install on a connected device: behind adb, listed when the APK lands.
    property var devices: []
    property string device: ""
    property string installState: ""
    property string installError: ""
    property bool installing: false
    property string logcat: ""
    function sizeText(bytes) {
        if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + " MB";
        return Math.max(1, Math.round(bytes / 1024)) + " KB";
    }

    onChosenChanged: { fast = !!chosen && chosen.fast_ready; hdr = !chosen || chosen.hdr !== false; }
    title: "Build a game"
    standardButtons: Dialog.NoButton
    // Fixed width so long notes and target labels wrap instead of stretching
    // the dialog: the content column is 480 wide plus this dialog's padding.
    width: 524

    onOpened: {
        error = ""; built = null; busy = false;
        devices = []; device = ""; installState = ""; installError = ""; logcat = "";
        locationField.text = app.appState.default_build_location || app.appState.default_project_location;
        app.invoke("list_build_targets", {}, list => {
            targets = list;
            // This machine comes first and can always build, so it is the default.
            const ready = list.find(t => t.ready) || list[0];
            triple = ready ? ready.triple : "";
        }, e => error = String(e));
    }
    function submit() {
        if (busy || !chosen || !chosen.ready) return;
        busy = true; error = ""; built = null;
        installState = ""; installError = ""; logcat = "";
        app.invoke("build_game", { path: locationField.text.trim(), target: triple, fast: fast, hdr: root.android || root.web ? false : hdr },
            result => { busy = false; built = result; if (root.android) root.listDevices(); }, e => { busy = false; error = String(e); });
    }
    function listDevices() {
        app.invoke("android_device_status", {}, result => {
            devices = result || [];
            if (devices.length === 1) device = devices[0].serial;
            else if (!devices.find(d => d.serial === device)) device = "";
        }, e => installError = String(e));
    }
    function install() {
        if (installing || !built || !built.binary) return;
        installing = true; installState = ""; installError = ""; logcat = "";
        app.invoke("android_device_status", {}, result => {
            devices = result || [];
            const serial = device || (devices.length === 1 ? devices[0].serial : "");
            // The build report carries the id the APK was signed with, so
            // the launch names the right component whatever the rows say.
            app.invoke("android_install", { apk: built.binary, app: built.application_id, device: serial || undefined },
                launched => {
                    installState = "Installed and launched " + launched.component + (launched.device ? " on " + launched.device : "") + ".";
                    app.invoke("android_logcat", serial ? { device: serial } : {}, dump => {
                        installing = false;
                        const lines = (dump.lines || []).concat(dump.panics || []);
                        logcat = lines.join("\n");
                        if ((dump.panics || []).length) installError = "The device log reports a native crash - see below.";
                    }, e => { installing = false; installError = String(e); });
                }, e => { installing = false; installError = String(e); });
        }, e => { installing = false; installError = String(e); });
    }

    ColumnLayout {
        width: parent.width; spacing: 8
        Text { visible: root.error.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; text: root.error; color: Theme.danger }
        Text { text: "Platform"; color: Theme.textDim; font.pixelSize: 12 }
        ChoiceField {
            options: root.targets.map(t => ({ value: t.triple, label: t.label + (t.host ? " (this machine)" : "") + (t.ready ? "" : " - unavailable") }))
            value: root.triple; onChosen: v => root.triple = v
        }
        Text { visible: !!root.chosen; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12; text: root.chosen ? root.chosen.note : "" }
        BwCheckBox { text: "Compile blocks for maximum speed"; enabled: !!root.chosen && root.chosen.fast_ready; checked: root.fast; onToggled: root.fast = checked }
        Text { visible: !!root.chosen; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12; text: root.chosen ? root.chosen.fast_note : "" }
        BwCheckBox { text: "HDR rendering and output"; enabled: !root.web && !root.android; checked: root.hdr && !root.web && !root.android; onToggled: root.hdr = checked }
        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: (root.chosen ? root.chosen.hdr_note + " " : "") + "Off makes an SDR-only build: 8-bit frames and no HDR window, for weak GPUs and old displays." }
        Text { text: "Where to put it"; color: Theme.textDim; font.pixelSize: 12 }
        RowLayout {
            Layout.fillWidth: true
            BwTextField { id: locationField; Layout.fillWidth: true; onAccepted: root.submit() }
            IconButton { iconName: "folder-open"; tip: "Browse for a folder"; flat: false; implicitWidth: 34; implicitHeight: 34; onClicked: browse.open() }
        }
        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: root.web
                ? "The game becomes one .html file with the player, the blocks and every asset inside it, in a folder named for the project. It opens straight from disk or from any static host. The whole file loads before the game starts, and packing costs about a third over the gzip'd parts; a host that serves it gzip'd wins most of that back."
                : root.android
                ? "The game becomes one signed APK: the NDK cross-builds the runtime, scripts and native blocks into lib/<abi>/, and the game folder rides under assets/. Install it on a phone, a tablet or an emulator below."
                : "The game gets a folder of its own, named for the project and the platform, with the player and the project's assets inside it. Anyone on that platform can run it without Blockloom." }
        ColumnLayout {
            visible: !!root.built; Layout.fillWidth: true; spacing: 4
            TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; color: Theme.text; font.pixelSize: 12; text: !root.built ? "" : root.web ? "Web page: " + root.built.binary + " (" + root.sizeText(root.built.size) + ")" : root.android ? "APK: " + root.built.binary + " (" + root.sizeText(root.built.size) + ", debug-signed)" : "Runnable folder: " + root.built.dir }
            TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; color: Theme.text; font.pixelSize: 12; text: root.built ? "Shareable ZIP: " + root.built.archive : "" }
        }
        ColumnLayout {
            visible: root.android && !!root.built; Layout.fillWidth: true; spacing: 6
            Text { text: "Install on a connected device"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
            Text { visible: root.installError.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: root.installError }
            Text { visible: root.installState.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12; text: root.installState }
            RowLayout {
                Layout.fillWidth: true; spacing: 8
                ChoiceField {
                    Layout.fillWidth: true
                    options: root.devices.map(d => ({ value: d.serial, label: d.serial + (d.emulator ? " (emulator)" : "") + " - " + d.state }))
                    value: root.device; onChosen: v => root.device = v
                }
                BwButton { text: "Refresh"; onClicked: root.listDevices() }
                BwButton { text: root.installing ? "Installing..." : "Install and launch"; primary: true; enabled: !root.installing && !!root.built; onClicked: root.install() }
            }
            Text { visible: root.devices.length === 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12; text: "No devices seen: connect a phone or start an emulator. An emulator counts as a device on the x86_64 row." }
            TextEdit { visible: root.logcat.length > 0; Layout.fillWidth: true; Layout.preferredHeight: 120; readOnly: true; selectByMouse: true; wrapMode: Text.Wrap; color: Theme.textDim; font.family: "monospace"; font.pixelSize: 11; text: root.logcat }
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight; Layout.topMargin: 8; spacing: 8
            BwButton { text: root.built ? "Done" : "Cancel"; onClicked: root.close() }
            BwButton { text: root.busy ? "Building..." : "Build"; primary: true; enabled: !root.busy && !!root.chosen && root.chosen.ready; onClicked: root.submit() }
        }
    }
    FolderDialog {
        id: browse
        title: "Where to put the built game"
        currentFolder: root.app.toFileUrl(locationField.text)
        onAccepted: locationField.text = root.app.fromFileUrl(selectedFolder)
    }
}
