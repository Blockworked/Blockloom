import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Offered on an actor that still has the legacy Body: read what converting it
// to a Rigidbody and Collider would store, then upgrade the actor or every
// actor in the project. One undo step each, and the project file is backed up.
ColumnLayout {
    id: root
    required property var app
    required property string actorId
    property var actorPreview: null
    property var projectPreview: null
    property string message: ""
    spacing: 6

    readonly property var actorNotes: {
        const scenes = actorPreview && actorPreview.scenes ? actorPreview.scenes : [];
        const out = [];
        for (const scene of scenes) for (const actor of scene.actors) for (const note of actor.notes || []) out.push(note);
        return out;
    }
    readonly property int projectTotal: projectPreview ? projectPreview.total : 0

    function refresh() {
        if (!actorId) return;
        app.invoke("physics_migration_preview", { actorId: actorId }, r => { actorPreview = r; }, e => { actorPreview = null; });
        app.invoke("physics_migration_preview", {}, r => { projectPreview = r; }, e => { projectPreview = null; });
    }
    function upgrade(args) {
        app.invoke("migrate_physics", args, r => { message = "Upgraded " + r.total + (r.total === 1 ? " actor." : " actors."); refresh(); },
            e => { message = String(e); });
    }
    onActorIdChanged: refresh()
    Component.onCompleted: refresh()

    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "This actor uses the older Body. It keeps working as it is. Upgrading replaces it with a Rigidbody and a Collider that behave the same, which you can then edit separately." }
    Text { objectName: "upgrade-notes"; visible: root.actorNotes.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: root.actorNotes.join("\n") }
    RowLayout { spacing: 6
        BwButton { objectName: "upgrade-actor"; text: "Upgrade this actor"; enabled: !!root.actorPreview && root.actorPreview.total > 0
            onClicked: root.upgrade({ actorId: root.actorId }) }
        BwButton { objectName: "upgrade-project"; text: "Upgrade all " + root.projectTotal; visible: root.projectTotal > 1
            onClicked: root.upgrade({}) }
    }
    Text { objectName: "upgrade-message"; visible: text.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11; text: root.message }
}
