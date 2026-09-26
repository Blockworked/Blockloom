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
    readonly property var chosen: targets.find(t => t.triple === triple) || null
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
        app.invoke("build_game", { path: locationField.text.trim(), target: triple, fast: fast, hdr: hdr },
            result => { busy = false; built = result; }, e => { busy = false; error = String(e); });
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
        BwCheckBox { text: "HDR rendering and output"; enabled: !root.web; checked: root.hdr && !root.web; onToggled: root.hdr = checked }
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
                : "The game gets a folder of its own, named for the project and the platform, with the player and the project's assets inside it. Anyone on that platform can run it without Blockloom." }
        ColumnLayout {
            visible: !!root.built; Layout.fillWidth: true; spacing: 4
            TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; color: Theme.text; font.pixelSize: 12; text: !root.built ? "" : root.web ? "Web page: " + root.built.binary + " (" + root.sizeText(root.built.size) + ")" : "Runnable folder: " + root.built.dir }
            TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WrapAnywhere; color: Theme.text; font.pixelSize: 12; text: root.built ? "Shareable ZIP: " + root.built.archive : "" }
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
