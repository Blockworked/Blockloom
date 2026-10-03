import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Turns the selected actor into a playable character in one undoable step:
// pick a preset, read what it will change, apply. Saved profiles carry a
// finished setup between actors and projects.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    property bool is3d: true
    // Whether the actor already has a CharacterController.
    property bool isPlayer: false
    spacing: 6

    readonly property var presets: is3d
        ? [{ value: "first-person-3d", label: "First person" }, { value: "third-person-3d", label: "Third person" }, { value: "top-down-3d", label: "Top down" }]
        : [{ value: "platformer-2d", label: "Platformer" }, { value: "top-down-2d", label: "Top down" }]
    property string preset: presets[0].value
    property var preview: null
    property string message: ""
    property bool convert: false
    readonly property var conflicts: preview && preview.conflicts ? preview.conflicts : []
    property var profiles: []
    property string profile: ""
    property string profileName: ""

    function refresh() {
        if (!actorId) return;
        message = "";
        app.invoke("preview_player_preset", { actorId: actorId, preset: preset }, result => { preview = result; }, e => { preview = null; message = String(e); });
        app.invoke("list_player_profiles", {}, list => {
            profiles = list.filter(p => p.valid && p.mode === (is3d ? "ThreeD" : "TwoD")).map(p => ({ value: p.name, label: p.name }));
            if (profiles.length > 0 && !profiles.some(p => p.value === profile)) profile = profiles[0].value;
        }, e => {});
    }
    function describe(step) {
        const verb = { Add: "adds", Replace: "replaces", Keep: "keeps", Remove: "takes away", Convert: "converts" }[step.kind] || step.kind;
        return verb + " " + step.component + (step.detail ? ": " + step.detail : "");
    }
    function summary() {
        if (!preview) return "";
        if (preview.blocked) return preview.blocked;
        return (preview.steps || []).map(describe).join("\n");
    }
    function apply() {
        app.invoke("apply_player_preset", { actorId: actorId, preset: preset, convert: convert },
            result => { message = "Ready to play."; refresh(); }, e => { message = String(e); });
    }
    onPresetChanged: refresh()
    onActorIdChanged: refresh()
    Component.onCompleted: refresh()

    Rectangle { Layout.fillWidth: true; Layout.topMargin: 6; height: 1; color: Theme.borderSoft }
    Text { text: root.isPlayer ? "Player setup" : "Make this a player"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
    InspectorRow { label: "Preset"; Layout.fillWidth: true
        ChoiceField { objectName: "player-preset"; options: root.presets; value: root.preset; onChosen: v => root.preset = v } }
    Text { objectName: "player-preview"; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        visible: text.length > 0; text: root.summary() }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        visible: root.conflicts.length > 0
        text: root.preview ? root.conflicts.map(c => c.component + ": " + c.reason + " - converting: " + c.conversion).join("\n") : "" }
    InspectorRow { label: "Convert"; visible: root.conflicts.length > 0; Layout.fillWidth: true
        SwitchField { objectName: "player-convert"; value: root.convert; onToggled: on => root.convert = on } Item { Layout.fillWidth: true } }
    BwButton { objectName: "player-apply"; text: root.isPlayer ? "Apply preset again" : "Make player"
        enabled: !!root.preview && !root.preview.blocked && (root.convert || root.conflicts.length === 0)
        onClicked: root.apply() }
    Text { objectName: "player-message"; visible: text.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11; text: root.message }

    InspectorRow { label: "Profile"; visible: root.profiles.length > 0; Layout.fillWidth: true
        ChoiceField { objectName: "player-profile"; options: root.profiles; value: root.profile; onChosen: v => root.profile = v } }
    BwButton { objectName: "player-apply-profile"; visible: root.profiles.length > 0; text: "Apply profile"
        onClicked: root.app.invoke("apply_player_profile", { actorId: root.actorId, name: root.profile, convert: root.convert },
            r => { root.message = "Profile applied."; root.refresh(); }, e => { root.message = String(e); }) }
    InspectorRow { label: "Save as"; visible: root.isPlayer; Layout.fillWidth: true
        BwTextField { objectName: "player-profile-name"; Layout.fillWidth: true; implicitHeight: 28; font.pixelSize: 12
            placeholderText: "profile name"; text: root.profileName; onTextEdited: root.profileName = text } }
    BwButton { objectName: "player-save-profile"; visible: root.isPlayer; text: "Save profile"; enabled: root.profileName.trim().length > 0
        onClicked: root.app.invoke("save_player_profile", { actorId: root.actorId, name: root.profileName.trim() },
            path => { root.message = "Saved to " + path; root.refresh(); }, e => { root.message = String(e); }) }
}
