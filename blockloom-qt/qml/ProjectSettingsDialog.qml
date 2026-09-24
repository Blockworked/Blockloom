import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Everything that belongs to the whole project rather than one actor: its
// dimension and icon, the world, lighting, post-process, the sound mix and
// input actions. Each row writes straight through to the backend.
BwDialog {
    id: root
    required property var app
    readonly property var project: app.appState.project
    readonly property var world: project ? project.world : null
    readonly property bool is3d: !!world && world.mode === "ThreeD"
    title: "Project settings"
    standardButtons: Dialog.NoButton
    width: 560; height: Math.min(parent ? parent.height - 80 : 800, 820)

    function invoke(command, args) { app.invoke(command, args); }
    function withIndex(array, index, value) { const next = array.slice(); next[index] = value; return next; }
    function clamp(n, lo, hi) { return Math.min(Math.max(n, lo), hi); }
    function postOf() { return Object.assign({ exposure_ev: 9.7, tonemapping: "TonyMcMapface", bloom_enabled: false, bloom_threshold: 1, bloom_intensity: 0.15, vignette_strength: 0 }, world && world.post ? world.post : {}); }
    function soundOf() { return Object.assign({ master_volume: 1, music_volume: 1, sfx_volume: 1 }, world && world.sound ? world.sound : {}); }
    function writeCamera(next) { invoke("set_camera", { camera: Object.assign(JSON.parse(JSON.stringify(world.camera)), next) }); }
    function writeLighting(next) { invoke("set_lighting", { lighting: Object.assign(JSON.parse(JSON.stringify(world.lighting)), next) }); }
    function writePost(next) { invoke("set_post_process", { post: Object.assign(postOf(), next) }); }
    function writeMixer(next) { invoke("set_sound_mixer", { mixer: Object.assign(soundOf(), next) }); }
    // Human spelling of a binding, the same text the `bind` block parses.
    function bindingText(b) {
        switch (b.binding) {
        case "Key": return b.key || "";
        case "Mouse": return "mouse:" + (b.button || "left").toLowerCase();
        case "GamepadButton": return "gamepad:" + (b.button || "").toLowerCase();
        case "GamepadAxis": return "gamepad:" + (b.axis || "").toLowerCase() + (b.direction === -1 ? "-" : b.direction === 1 ? "+" : "");
        default: return "";
        }
    }

    component Section: ColumnLayout {
        property string heading: ""
        default property alias content: body.data
        Layout.fillWidth: true; spacing: 6
        Text { text: parent.heading; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold; Layout.topMargin: 10 }
        ColumnLayout { id: body; Layout.fillWidth: true; spacing: 6 }
    }
    component Note: Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11 }

    ScrollView {
        id: scroll
        anchors.fill: parent; clip: true
        contentWidth: availableWidth
        ColumnLayout {
            width: scroll.availableWidth - 12; spacing: 6
            Section {
                heading: "Project"
                InspectorRow { label: "Type"; labelWidth: 110; Layout.fillWidth: true
                    BwButton { text: "2D"; iconName: "square"; primary: !root.is3d; implicitHeight: 30; onClicked: if (root.is3d) { modeDialog.target = "TwoD"; modeDialog.open(); } }
                    BwButton { text: "3D"; iconName: "box"; primary: root.is3d; implicitHeight: 30; onClicked: if (!root.is3d) { modeDialog.target = "ThreeD"; modeDialog.open(); } }
                    Item { Layout.fillWidth: true } }
                Note { text: (root.is3d ? "Meshes and 3D physics, measured in metres." : "Sprites and flat physics, measured in pixels.") + " Switching converts the scene and restarts a running game." }
                InspectorRow { label: "Game icon"; labelWidth: 110; Layout.fillWidth: true
                    Rectangle {
                        implicitWidth: 40; implicitHeight: 40; radius: 6; color: Theme.field; border.color: Theme.border
                        Image { anchors.fill: parent; anchors.margins: 3; fillMode: Image.PreserveAspectFit; cache: false; source: root.project && root.project.icon ? root.app.assetUrl(root.project.icon) : "" }
                        LucideIcon { visible: !(root.project && root.project.icon); anchors.centerIn: parent; name: "image"; width: 22; height: 22; color: Theme.textDim }
                    }
                    AssetField { app: root.app; accept: ["image"]; value: root.project ? root.project.icon : ""; placeholderText: "Blockloom default"; onCommitted: p => root.invoke("set_project_icon", { path: p }) }
                    IconButton { iconName: "folder-open"; tip: "Choose an image"; onClicked: iconFile.open() }
                    IconButton { iconName: "x"; tip: "Use the Blockloom default"; enabled: !!(root.project && root.project.icon); onClicked: root.invoke("set_project_icon", { path: "" }) } }
                Note { text: "Used for the packaged executable or platform launcher. Square PNG images work best." }
            }
            Section {
                heading: "World"; visible: !!root.world
                InspectorRow { label: "Background"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: root.world ? root.world.background : "#000000"; onPicked: c => root.invoke("set_background", { color: c }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Gravity"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index; value: root.world.gravity[index]; onCommitted: n => root.invoke("set_gravity", { gravity: root.withIndex(root.world.gravity, index, n) }) } } }
                InspectorRow { label: "Tick rate"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.world ? root.world.fixed_rate : 60; fallback: 60; onCommitted: n => root.invoke("set_fixed_rate", { fixedRate: root.clamp(n, 1, 1000) }) } }
                Note { text: "How many times a second the world's blocks and physics advance, whatever the display rate is. Higher is smoother but heavier. Applies on the next run of the game." }
                InspectorRow { visible: !root.is3d; label: "Zoom"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.world ? root.world.camera.zoom : 1; fallback: 1; onCommitted: n => root.writeCamera({ zoom: n }) } }
                InspectorRow { visible: root.is3d; label: "Camera at"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 3; delegate: NumberField { required property int index; value: root.world.camera.position[index]; onCommitted: n => root.writeCamera({ position: root.withIndex(root.world.camera.position, index, n) }) } } }
                Note { text: "Where the camera stands when no actor has a Camera component. A 2D unit is a pixel and a 3D unit is a metre." }
            }
            Section {
                heading: "Lighting"; visible: !!root.world && root.is3d
                InspectorRow { label: "Light direction"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 3; delegate: NumberField { required property int index; value: root.world.lighting.light_direction[index]; onCommitted: n => root.writeLighting({ light_direction: root.withIndex(root.world.lighting.light_direction, index, n) }) } } }
                InspectorRow { label: "Light color"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: root.world ? root.world.lighting.light_color : "#FFFFFF"; onPicked: c => root.writeLighting({ light_color: c }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Brightness"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.world ? root.world.lighting.illuminance : 10000; fallback: 10000; onCommitted: n => root.writeLighting({ illuminance: root.clamp(n, 0, 200000) }) } }
                InspectorRow { label: "Ambient color"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: root.world ? root.world.lighting.ambient_color : "#FFFFFF"; onPicked: c => root.writeLighting({ ambient_color: c }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Ambient"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.world ? root.world.lighting.ambient_brightness : 80; fallback: 80; onCommitted: n => root.writeLighting({ ambient_brightness: root.clamp(n, 0, 1000) }) } }
                InspectorRow { label: "Occlusion"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!root.world && root.world.lighting.ao_enabled; onToggled: on => root.writeLighting({ ao_enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Shadow detail"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.world && root.world.lighting.shadow_map_size !== undefined ? root.world.lighting.shadow_map_size : 2048; fallback: 2048; onCommitted: n => root.writeLighting({ shadow_map_size: root.clamp(Math.round(n), 512, 8192) }) } }
                InspectorRow { label: "Shadow bias"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.world && root.world.lighting.shadow_bias !== undefined ? root.world.lighting.shadow_bias : 0.02; fallback: 0.02; onCommitted: n => root.writeLighting({ shadow_bias: root.clamp(n, 0, 0.5) }) } }
                Note { text: "Where the 3D sun shines from (aimed at the origin), and how the scene's ambient light looks. Occlusion darkens creases where objects meet but costs GPU time. Shadow detail snaps to a power of two; raise the bias if striped acne appears on lit faces. Applies on the next run of the game." }
            }
            Section {
                heading: "Post-process"; visible: !!root.world
                InspectorRow { label: "Exposure"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.postOf().exposure_ev; fallback: 9.7; onCommitted: n => root.writePost({ exposure_ev: root.clamp(n, 0, 20) }) } }
                InspectorRow { label: "Tonemap"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: Blocks.opts(["TonyMcMapface","None","Reinhard","ReinhardLuminance","AcesFitted","Filmic"]); value: root.postOf().tonemapping; onChosen: v => root.writePost({ tonemapping: v }) } }
                InspectorRow { label: "Glow"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: root.postOf().bloom_enabled; onToggled: on => root.writePost({ bloom_enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.postOf().bloom_enabled; label: "Glow limit"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.postOf().bloom_threshold; fallback: 1; onCommitted: n => root.writePost({ bloom_threshold: Math.max(n, 0) }) } }
                InspectorRow { visible: root.postOf().bloom_enabled; label: "Glow amount"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round(root.postOf().bloom_intensity * 100); onMoved: root.writePost({ bloom_intensity: value / 100 }) }
                    Text { text: Math.round(root.postOf().bloom_intensity * 100) + "%"; color: Theme.textDim; font.pixelSize: 12 } }
                InspectorRow { label: "Corners"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round(root.postOf().vignette_strength * 100); onMoved: root.writePost({ vignette_strength: value / 100 }) }
                    Text { text: Math.round(root.postOf().vignette_strength * 100) + "%"; color: Theme.textDim; font.pixelSize: 12 } }
                Note { text: "The camera's finish, in both dimensions. Lower exposure brightens; glow makes emissive surfaces bloom; corners darkens the frame edges. Applies on the next run of the game." }
            }
            Section {
                heading: "Sound"; visible: !!root.world
                Repeater {
                    model: [{ key: "master_volume", label: "Master" }, { key: "music_volume", label: "Music" }, { key: "sfx_volume", label: "Effects" }]
                    delegate: InspectorRow {
                        required property var modelData
                        label: modelData.label; labelWidth: 110; Layout.fillWidth: true
                        SliderField { from: 0; to: 200; stepSize: 1; value: Math.round(root.soundOf()[modelData.key] * 100)
                            onMoved: { const next = {}; next[modelData.key] = value / 100; root.writeMixer(next); } }
                        Text { text: Math.round(root.soundOf()[modelData.key] * 100) + "%"; color: Theme.textDim; font.pixelSize: 12 }
                    }
                }
                Note { text: "The saved mix every voice plays through: master scales everything, music and effects scale their own bus on top of it. A `set bus volume` block moves the live mix without changing this. Applies on the next run of the game." }
            }
            Section {
                heading: "Input actions"; visible: !!root.world
                Repeater {
                    model: root.world && root.world.input ? root.world.input.actions : []
                    delegate: ColumnLayout {
                        id: action
                        required property var modelData
                        Layout.fillWidth: true; spacing: 4
                        RowLayout {
                            Layout.fillWidth: true
                            Text { Layout.fillWidth: true; text: action.modelData.name; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
                            IconButton { iconName: "x"; tip: "Delete this action"; onClicked: { deleteAction.name = action.modelData.name; deleteAction.open(); } }
                        }
                        Flow {
                            Layout.fillWidth: true; spacing: 4
                            Repeater {
                                model: action.modelData.bindings
                                delegate: Rectangle {
                                    required property var modelData
                                    implicitWidth: chip.implicitWidth + 30; implicitHeight: 24; radius: 12; color: Theme.panelRaised; border.color: Theme.border
                                    Text { id: chip; x: 10; anchors.verticalCenter: parent.verticalCenter; text: root.bindingText(modelData); color: Theme.text; font.pixelSize: 11 }
                                    LucideIcon { anchors.right: parent.right; anchors.rightMargin: 6; anchors.verticalCenter: parent.verticalCenter; name: "x"; width: 12; height: 12; color: Theme.textDim
                                        MouseArea { anchors.fill: parent; anchors.margins: -3; cursorShape: Qt.PointingHandCursor; onClicked: root.invoke("remove_input_binding", { name: action.modelData.name, binding: root.bindingText(modelData) }) } }
                                }
                            }
                            Text { visible: !action.modelData.bindings.length; text: "No bindings - blocks naming it read as unheld."; color: Theme.textDim; font.pixelSize: 11 }
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            BwTextField { id: bindingField; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "space, mouse:left, gamepad:south"
                                onAccepted: if (text.trim().length) { root.invoke("add_input_binding", { name: action.modelData.name, binding: text.trim() }); text = ""; } }
                            BwButton { text: "Add binding"; implicitHeight: 30; onClicked: bindingField.accepted() }
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    BwTextField { id: actionName; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "New action name"
                        onAccepted: if (text.trim().length) { root.invoke("create_input_action", { name: text.trim() }); text = ""; } }
                    BwButton { text: "Add action"; implicitHeight: 30; onClicked: actionName.accepted() }
                }
                Note { text: "One name for every way to say it: keys, mouse buttons, gamepad buttons and sticks. Blocks read an action held, pressed, released or as an analog value, and a `bind` block remaps one for the rest of the run. Applies on the next run of the game." }
            }
            RowLayout {
                Layout.fillWidth: true; Layout.topMargin: 12
                Item { Layout.fillWidth: true }
                BwButton { text: "Done"; primary: true; onClicked: root.close() }
            }
        }
    }

    BwDialog {
        id: modeDialog
        property string target: "TwoD"
        title: "Switch dimension?"
        standardButtons: Dialog.Yes | Dialog.Cancel
        Text { color: Theme.text; text: "Switch this project to " + (modeDialog.target === "ThreeD" ? "3D" : "2D") + "?\n\nContent is converted and a running game restarts." }
        onAccepted: root.invoke("set_mode", { mode: target })
    }
    BwDialog {
        id: deleteAction
        property string name: ""
        title: "Delete input action?"
        standardButtons: Dialog.Yes | Dialog.Cancel
        Text { color: Theme.text; text: "Delete the “" + deleteAction.name + "” input action?\n\nBlocks naming it will read as unheld." }
        onAccepted: root.invoke("delete_input_action", { name: name })
    }
    FileDialog {
        id: iconFile
        title: "Choose a game icon"
        nameFilters: ["Images (*.png *.jpg *.jpeg *.bmp *.gif *.webp *.ico)"]
        onAccepted: root.app.invoke("import_assets", { parent: "assets", paths: [root.app.fromFileUrl(selectedFile)] },
            imported => { if (imported && imported[0]) root.invoke("set_project_icon", { path: imported[0] }); })
    }
}
