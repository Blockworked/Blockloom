import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Naming a new project, choosing where it goes and which dimension it is.
// The folder name is not a field: the app names the folder after the project.
StitchDialog {
    id: root
    required property var app
    property string mode: "TwoD"
    // "" starts empty, otherwise a sample by name. Some only fit one dimension.
    property string sample: ""
    readonly property var samples: [
        { name: "physics-playground", label: "Physics playground", modes: ["TwoD", "ThreeD"],
          about: "A player, a stack of crates, a pendulum on a rope, a motor wheel, a door (3D only), a joint that snaps and a trigger zone, all made of components with no custom code." },
        { name: "platformer", label: "Platformer", modes: ["TwoD"],
          about: "Steps of platforms to jump up, coins to collect and a flag at the end." },
        { name: "top-down", label: "Top down", modes: ["TwoD"],
          about: "An arena seen from above with gems to collect and a guard on patrol who sends you back to the start." },
        { name: "third-person", label: "Third person", modes: ["ThreeD"],
          about: "A rigged figure that walks, runs and jumps on one blended animator, plants its feet and turns its head, and a boulder that knocks it limp for two seconds." },
        { name: "first-person", label: "First person", modes: ["ThreeD"],
          about: "A room with orbs to collect, stairs up to a ledge and a door that slides open once you have them all." },
        { name: "ui-puzzle", label: "Interface puzzle", modes: ["TwoD", "ThreeD"],
          about: "Nine lights made of interface buttons. Each press flips a light and its neighbours; turn them all on." }
    ]
    function sampleInfo(name) { return samples.find(s => s.name === name); }
    onModeChanged: { const s = sampleInfo(sample); if (s && s.modes.indexOf(mode) < 0) sample = ""; }
    property string error: ""
    property bool busy: false
    title: "New project"
    standardButtons: Dialog.NoButton

    onOpened: {
        nameField.text = "My Game";
        locationField.text = app.appState.default_project_location;
        mode = "TwoD"; sample = ""; error = ""; busy = false;
        nameField.forceActiveFocus(); nameField.selectAll();
    }
    // What the project's own folder will be called: anything a path can't
    // hold is shown as the `-` it becomes.
    readonly property string folder: {
        const cleaned = nameField.text.replace(/[\/\\:*?"<>|]/g, "-").trim().replace(/^\.+|\.+$/g, "").trim();
        return cleaned === "" ? "Project" : cleaned;
    }
    readonly property string fullPath: {
        const loc = locationField.text;
        const sep = loc.indexOf("\\") >= 0 ? "\\" : "/";
        return loc.replace(/[\/\\]+$/, "") + sep + folder;
    }
    function submit() {
        if (busy) return;
        if (!nameField.text.trim().length) { error = "Give the project a name"; return; }
        busy = true;
        app.invoke("create_project", { name: nameField.text.trim(), location: locationField.text.trim(), mode: mode, sample: sample },
            () => { busy = false; root.close(); }, e => { busy = false; error = String(e); });
    }

    ColumnLayout {
        width: 460; spacing: 8
        Text { visible: root.error.length > 0; text: root.error; color: Theme.danger; Layout.fillWidth: true; wrapMode: Text.WordWrap }
        Text { text: "Name"; color: Theme.textDim; font.pixelSize: 12 }
        StitchTextField { id: nameField; Layout.fillWidth: true; placeholderText: "My Game"; onAccepted: root.submit() }
        Text { text: "Where to keep it"; color: Theme.textDim; font.pixelSize: 12 }
        RowLayout {
            Layout.fillWidth: true
            StitchTextField { id: locationField; Layout.fillWidth: true; onAccepted: root.submit() }
            IconButton { iconName: "folder-open"; tip: "Browse for a folder"; flat: false; implicitWidth: 34; implicitHeight: 34; onClicked: browse.open() }
        }
        Text { Layout.fillWidth: true; wrapMode: Text.WrapAnywhere; color: Theme.textDim; font.pixelSize: 12; text: "The project gets a folder of its own: " + root.fullPath }
        Text { text: "Dimension"; color: Theme.textDim; font.pixelSize: 12 }
        RowLayout {
            spacing: 6
            StitchButton { text: "2D"; iconName: "square"; primary: root.mode === "TwoD"; onClicked: root.mode = "TwoD" }
            StitchButton { text: "3D"; iconName: "box"; primary: root.mode === "ThreeD"; onClicked: root.mode = "ThreeD" }
        }
        Text {
            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: (root.mode === "TwoD" ? "Sprites and flat physics, measured in pixels." : "Meshes and 3D physics, measured in metres.") + " A project can switch later."
        }
        Text { text: "Start from"; color: Theme.textDim; font.pixelSize: 12 }
        Flow {
            objectName: "samples"; Layout.fillWidth: true; spacing: 6
            StitchButton { objectName: "sample-empty"; text: "Empty"; primary: root.sample === ""; onClicked: root.sample = "" }
            Repeater {
                model: root.samples.filter(s => s.modes.indexOf(root.mode) >= 0)
                StitchButton {
                    required property var modelData
                    objectName: "sample-" + modelData.name
                    text: modelData.label; primary: root.sample === modelData.name
                    onClicked: root.sample = modelData.name
                }
            }
        }
        Text {
            visible: root.sample !== ""; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
            text: { const s = root.sampleInfo(root.sample); return s ? s.about : ""; }
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight; Layout.topMargin: 8; spacing: 8
            StitchButton { text: "Cancel"; onClicked: root.close() }
            StitchButton { text: "Create"; primary: true; enabled: !root.busy; onClicked: root.submit() }
        }
    }
    FolderDialog {
        id: browse
        title: "Where to keep the project"
        currentFolder: root.app.toFileUrl(locationField.text)
        onAccepted: locationField.text = root.app.fromFileUrl(selectedFolder)
    }
}
