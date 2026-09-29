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
    // After a launch the dialog polls the device log into the RunLog (see
    // `android_logcat_tail`): each poll clears the buffer, so every answer
    // holds only what arrived since the last.
    property var devices: []
    property string device: ""
    property string installState: ""
    property string installError: ""
    property bool installing: false
    property string logcat: ""
    property bool polling: false
    // Release signing: the project's key rows, and this build's passwords.
    // Passwords live in these fields only, never in the project or config -
    // unless remembered into the OS keyring below, which the next build
    // reads back instead of asking.
    readonly property string releaseKeystore: app.appState.project && app.appState.project.android ? (app.appState.project.android.keystore || "") : ""
    readonly property string releaseAlias: app.appState.project && app.appState.project.android ? (app.appState.project.android.key_alias || "") : ""
    property string storePass: ""
    property string keyPass: ""
    property bool remember: false
    property var keyring: ({available: false, store_saved: false, key_saved: false})
    function sizeText(bytes) {
        if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + " MB";
        return Math.max(1, Math.round(bytes / 1024)) + " KB";
    }

    onChosenChanged: { fast = !!chosen && chosen.fast_ready; hdr = !chosen || chosen.hdr !== false; root.refreshKeyring(); }
    onClosed: polling = false
    // The stream behind `pollLogcat`: every two seconds the device log is
    // dumped, cleared and appended to the RunLog, until the dialog closes
    // or a new build or install restarts it.
    Timer {
        interval: 2000; running: root.polling; repeat: true
        onTriggered: root.pollLogcat()
    }
    title: "Build a game"
    standardButtons: Dialog.NoButton
    // Fixed width so long notes and target labels wrap instead of stretching
    // the dialog: the content column is 480 wide plus this dialog's padding.
    width: 524

    onOpened: {
        error = ""; built = null; busy = false;
        devices = []; device = ""; installState = ""; installError = ""; logcat = ""; polling = false;
        storePass = ""; keyPass = ""; remember = false;
        keyring = ({available: false, store_saved: false, key_saved: false});
        locationField.text = app.appState.default_build_location || app.appState.default_project_location;
        app.invoke("list_build_targets", {}, list => {
            targets = list;
            // This machine comes first and can always build, so it is the default.
            const ready = list.find(t => t.ready) || list[0];
            triple = ready ? ready.triple : "";
            root.refreshKeyring();
        }, e => error = String(e));
    }
    function refreshKeyring() {
        if (!root.android) return;
        app.invoke("android_keyring_status", {}, s => { keyring = s; }, e => {});
    }
    function forgetKeyring() {
        app.invoke("android_forget_passwords", {}, forgot => { root.refreshKeyring(); }, e => installError = String(e));
    }
    function submit() {
        if (busy || !chosen || !chosen.ready) return;
        busy = true; error = ""; built = null;
        installState = ""; installError = ""; logcat = ""; polling = false;
        const args = { path: locationField.text.trim(), target: triple, fast: fast, hdr: root.android || root.web ? false : hdr };
        // Passwords ride this call only: a release row without them stops
        // the build with where to type them, and headless reads the env.
        // Remembered ones are already in the keyring, so empty fields do.
        if (root.android && root.releaseKeystore !== "") {
            args.storePass = storePass;
            args.keyPass = keyPass;
            args.rememberPasswords = remember;
        }
        app.invoke("build_game", args,
            result => { busy = false; built = result; storePass = ""; keyPass = ""; remember = false; if (root.android) { root.refreshKeyring(); root.listDevices(); } }, e => { busy = false; error = String(e); });
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
        installing = true; installState = ""; installError = ""; logcat = ""; polling = false;
        app.invoke("android_device_status", {}, result => {
            devices = result || [];
            const serial = device || (devices.length === 1 ? devices[0].serial : "");
            // The build report carries the id the APK was signed with, so
            // the launch names the right component whatever the rows say.
            app.invoke("android_install", { apk: built.binary, app: built.application_id, device: serial || undefined },
                launched => {
                    installing = false;
                    installState = "Installed and launched " + launched.component + (launched.device ? " on " + launched.device : "") + ". Streaming the device log into the RunLog below.";
                    // The install cleared the buffer, so the first poll reads
                    // only the fresh run; every poll after it only the new.
                    polling = true;
                    root.pollLogcat();
                }, e => { installing = false; installError = String(e); });
        }, e => { installing = false; installError = String(e); });
    }
    function pollLogcat() {
        if (!polling) return;
        const serial = device || (devices.length === 1 ? devices[0].serial : "");
        app.invoke("android_logcat_tail", serial ? { device: serial } : {}, dump => {
            if (!polling) return;
            const lines = (dump.lines || []).concat(dump.panics || []);
            if (lines.length) logcat = (logcat ? logcat + "\n" : "") + lines.join("\n");
            if ((dump.panics || []).length) installError = "The device log reports a native crash - see below and in the RunLog.";
        }, e => { if (polling) installError = String(e); });
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
            TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; color: Theme.text; font.pixelSize: 12; text: !root.built ? "" : root.web ? "Web page: " + root.built.binary + " (" + root.sizeText(root.built.size) + ")" : root.android ? "APK: " + root.built.binary + " (" + root.sizeText(root.built.size) + ", " + (root.built.signed || "debug") + "-signed)" : "Runnable folder: " + root.built.dir }
            TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; color: Theme.text; font.pixelSize: 12; text: root.built ? "Shareable ZIP: " + root.built.archive : "" }
        }
        ColumnLayout {
            visible: root.android; Layout.fillWidth: true; spacing: 6
            Text { text: "Signing"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
            Text { visible: root.releaseKeystore === ""; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "Debug-signed: fine for devices, refused by the Play store. Pick a release key file plus alias in Project settings for store uploads - or make one below." }
            ColumnLayout { visible: root.releaseKeystore !== ""; Layout.fillWidth: true; spacing: 6
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12; text: "Release key: " + root.releaseKeystore + " (" + (root.releaseAlias || "no alias set") + "). Passwords are asked on every build and never stored unless remembered below; headless builds read BLOCKLOOM_ANDROID_STORE_PASS / BLOCKLOOM_ANDROID_KEY_PASS instead." }
                Text { visible: root.keyring.store_saved || root.keyring.key_saved; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                    text: "The system keyring holds the passwords - leave the fields empty to reuse them, or type new ones." }
                RowLayout {
                    Layout.fillWidth: true; spacing: 8
                    Text { text: "Keystore password"; color: Theme.textDim; font.pixelSize: 12; Layout.preferredWidth: 130 }
                    BwTextField { Layout.fillWidth: true; echoMode: TextInput.Password; text: root.storePass; placeholderText: "Asked every build"; onTextChanged: root.storePass = text }
                }
                RowLayout {
                    Layout.fillWidth: true; spacing: 8
                    Text { text: "Key password"; color: Theme.textDim; font.pixelSize: 12; Layout.preferredWidth: 130 }
                    BwTextField { Layout.fillWidth: true; echoMode: TextInput.Password; text: root.keyPass; placeholderText: "Empty means the keystore password"; onTextChanged: root.keyPass = text }
                }
                RowLayout {
                    visible: root.keyring.available; Layout.fillWidth: true; spacing: 8
                    BwCheckBox { text: "Remember passwords in the system keyring"; checked: root.remember; onToggled: root.remember = checked }
                    Item { Layout.fillWidth: true }
                    BwButton { visible: root.keyring.store_saved || root.keyring.key_saved; text: "Forget saved"; onClicked: root.forgetKeyring() }
                }
                Text { visible: !root.keyring.available; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                    text: "No scriptable system keyring on this machine, so every build asks." }
            }
            BwButton { text: "Create a new release key..."; onClicked: { newKey.error = ""; newKey.open(); } }
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
    BwDialog {
        id: newKey
        property string error: ""
        property string keyPath: ""
        property string keyAlias: ""
        property string keyStorePass: ""
        property string keyKeyPass: ""
        property bool remember: false
        property bool busy: false
        title: "Create a release key"
        standardButtons: Dialog.NoButton
        width: 480
        onOpened: { error = ""; busy = false; remember = false; }
        function create() {
            if (busy || keyPath.trim() === "" || keyAlias.trim() === "") return;
            busy = true; error = "";
            app.invoke("android_create_keystore", { path: keyPath.trim(), alias: keyAlias.trim(), storePass: keyStorePass, keyPass: keyKeyPass, rememberPasswords: newKey.remember },
                aliases => {
                    busy = false;
                    // The new key signs this project from now on.
                    app.invoke("set_android_settings", { keystore: keyPath.trim(), keyAlias: keyAlias.trim() },
                        () => { newKey.close(); root.refreshKeyring(); }, e => error = String(e));
                }, e => { busy = false; error = String(e); });
        }
        ColumnLayout {
            width: parent.width; spacing: 8
            Text { visible: newKey.error.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.danger; font.pixelSize: 12; text: newKey.error }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "A new RSA keypair for store uploads. Guard the file and its passwords: lose them and updates to the game cannot be signed." }
            Text { text: "Key file"; color: Theme.textDim; font.pixelSize: 12 }
            RowLayout {
                Layout.fillWidth: true
                BwTextField { Layout.fillWidth: true; text: newKey.keyPath; placeholderText: "/path/to/release.keystore"; onTextChanged: newKey.keyPath = text }
                IconButton { iconName: "folder-open"; tip: "Choose where the key file goes"; implicitWidth: 34; implicitHeight: 34; onClicked: newKeyBrowse.open() }
            }
            Text { text: "Alias"; color: Theme.textDim; font.pixelSize: 12 }
            BwTextField { Layout.fillWidth: true; text: newKey.keyAlias; placeholderText: "upload"; onTextChanged: newKey.keyAlias = text }
            Text { text: "Keystore password"; color: Theme.textDim; font.pixelSize: 12 }
            BwTextField { Layout.fillWidth: true; echoMode: TextInput.Password; text: newKey.keyStorePass; placeholderText: "At least 6 characters"; onTextChanged: newKey.keyStorePass = text }
            Text { text: "Key password"; color: Theme.textDim; font.pixelSize: 12 }
            BwTextField { Layout.fillWidth: true; echoMode: TextInput.Password; text: newKey.keyKeyPass; placeholderText: "Empty means the keystore password"; onTextChanged: newKey.keyKeyPass = text }
            BwCheckBox { visible: root.keyring.available; text: "Remember passwords in the system keyring"; checked: newKey.remember; onToggled: newKey.remember = checked }
            RowLayout {
                Layout.alignment: Qt.AlignRight; Layout.topMargin: 8; spacing: 8
                BwButton { text: "Cancel"; onClicked: newKey.close() }
                BwButton { text: newKey.busy ? "Creating..." : "Create key"; primary: true; enabled: !newKey.busy && newKey.keyPath.trim() !== "" && newKey.keyAlias.trim() !== ""; onClicked: newKey.create() }
            }
        }
    }
    FileDialog {
        id: newKeyBrowse
        title: "Where the release key file goes"
        fileMode: FileDialog.SaveFile
        nameFilters: ["Key files (*.jks *.keystore)", "All files (*)"]
        onAccepted: newKey.keyPath = root.app.fromFileUrl(selectedFile)
    }
    FolderDialog {
        id: browse
        title: "Where to put the built game"
        currentFolder: root.app.toFileUrl(locationField.text)
        onAccepted: locationField.text = root.app.fromFileUrl(selectedFolder)
    }
}
