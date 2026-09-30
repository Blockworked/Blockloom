import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// App-level settings, kept beside projects.json: the Android SDK/NDK rows,
// the license stamp and the toolchain probes. Per-project Android rows
// (applicationId, version) live in Project settings instead; phones and
// emulators live in the Devices tab.
BwDialog {
    id: root
    required property var app
    property var android: null
    property string androidError: ""
    property string licenseText: ""
    property bool licensesAccepted: false
    property bool busy: false
    property bool sdkBusy: false
    property string sdkResult: ""
    title: "App settings"
    standardButtons: Dialog.NoButton
    width: 600

    onOpened: refresh()
    function refresh() {
        androidError = ""; sdkResult = "";
        app.invoke("android_status", {}, result => {
            android = result;
            licensesAccepted = !!result.licenses_accepted;
            sdkPathField.text = result.sdk_path || "";
            ndkPathField.text = result.ndk_path || "";
        }, e => androidError = String(e));
    }
    function row(ok, detail) { return (ok ? "OK  " : "Missing  ") + detail; }
    function installSdk() {
        if (root.sdkBusy) return;
        root.sdkBusy = true; root.sdkResult = ""; root.androidError = "";
        root.app.invoke("android_install_sdk", {}, result => {
            root.sdkBusy = false;
            root.sdkResult = "Installed: " + (result.installed.length ? result.installed.join(", ") : "nothing new") + ". " + (result.still_missing.length ? "Still missing: " + result.still_missing.join(" ") : "All rows probe green.");
            root.refresh();
        }, e => { root.sdkBusy = false; root.androidError = String(e); });
    }

    ScrollView {
        id: outerScroll
        anchors.fill: parent; clip: true
        contentWidth: availableWidth
        ColumnLayout {
            width: parent.width - 12; spacing: 8
            // Install failures arrive trimmed to the error tail already;
            // this caps the view so a long one never blows the dialog up,
            // and opens at the bottom where the actual error sits.
            ScrollView {
                id: errorScroll
                visible: root.androidError.length > 0
                Layout.fillWidth: true; Layout.preferredHeight: 140; clip: true
                TextEdit {
                    width: errorScroll.availableWidth
                    readOnly: true; selectByMouse: true; wrapMode: Text.Wrap
                    color: Theme.danger; font.family: "monospace"; font.pixelSize: 11
                    text: root.androidError
                    onTextChanged: errorScroll.contentItem.contentY = Math.max(0, errorScroll.contentItem.contentHeight - errorScroll.height)
                }
            }
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
                    onClicked: root.installSdk()
                }
                BwButton {
                    text: "Show licenses"; enabled: !root.sdkBusy
                    onClicked: root.app.invoke("android_accept_licenses", { accept: false }, result => { root.licenseText = result.text; root.licensesAccepted = !!result.accepted; }, e => root.androidError = String(e))
                }
                BwButton {
                    text: "Accept licenses"; enabled: !root.licensesAccepted && !root.sdkBusy
                    // Accepting is for installing: packages refused before
                    // this left an empty emulator dir behind, so install
                    // straight on rather than leaving them unfetched.
                    onClicked: root.app.invoke("android_accept_licenses", { accept: true }, result => {
                        root.licenseText = result.text; root.licensesAccepted = true; root.installSdk();
                    }, e => root.androidError = String(e))
                }
            }
            Text { visible: root.sdkBusy; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12; text: "Installing packages - the first run downloads a few hundred megabytes and takes a while." }
            Text { visible: root.sdkResult.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12; text: root.sdkResult }
            Text { visible: root.licensesAccepted; Layout.fillWidth: true; color: Theme.textDim; font.pixelSize: 12; text: "SDK licenses accepted." }
            // A fixed-height scroll rather than a fixed-height text box: the
            // box painted its overflow over the rows below it.
            ScrollView {
                id: licenseScroll
                visible: root.licenseText.length > 0
                Layout.fillWidth: true; Layout.preferredHeight: 140; clip: true
                TextEdit {
                    width: licenseScroll.availableWidth
                    readOnly: true; selectByMouse: true; wrapMode: Text.Wrap
                    color: Theme.textDim; font.family: "monospace"; font.pixelSize: 11
                    text: root.licenseText
                }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "Phones and emulators live in the editor's Devices tab: boot them there, see their screens and run the game on them." }
            RowLayout {
                Layout.alignment: Qt.AlignRight; Layout.topMargin: 8; spacing: 8
                BwButton { text: "Refresh"; onClicked: root.refresh() }
                BwButton { text: "Done"; primary: true; onClicked: root.close() }
            }
        }
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
