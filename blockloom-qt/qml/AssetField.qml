import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A path box an asset dragged out of the tray can land on. While an asset it
// would take is in flight it shows its outline, so you can see where a file
// is allowed to go before you get there.
BwTextField {
    id: root
    required property var app
    property string value: ""
    // Asset kinds this box takes; empty takes any file.
    property var accept: []
    readonly property var icons: ({ image: "image", audio: "music", font: "file-type", model: "box", script: "file-code",
        shader: "sparkles", text: "file-text", hdr: "sun", volume: "layers", light: "zap", height: "trending-up", scene: "map", lighting: "sun" })
    property string assetKind: accept.length ? accept[0] : "other"
    readonly property string iconName: icons[ready ? app.assetDrag.kind : assetKind] || "file"
    signal committed(string path)
    Layout.fillWidth: true
    implicitHeight: 30; font.pixelSize: 12; leftPadding: 34
    text: value
    onEditingFinished: if (text !== value) committed(text)

    function wants(entry) { return !!entry && entry.kind !== "folder" && (!accept.length || accept.indexOf(entry.kind) >= 0); }
    readonly property bool ready: wants(app.assetDrag)
    LucideIcon {
        objectName: "asset-type-icon"
        name: root.iconName; color: root.ready ? Theme.accent : Theme.textDim
        width: 16; height: 16; anchors.left: parent.left; anchors.leftMargin: 10; anchors.verticalCenter: parent.verticalCenter
    }
    background: Rectangle {
        radius: 5; color: root.enabled ? Theme.field : "#27282a"
        border.color: root.ready ? Theme.accent : (root.activeFocus ? Theme.accent : Theme.border)
        border.width: root.ready ? 2 : (root.activeFocus ? 1.5 : 1)
    }
    // The tray asks every registered box whether a drop landed on it.
    function takeDrop(entry, sceneX, sceneY) {
        if (!visible || !wants(entry)) return false;
        const p = mapFromItem(null, sceneX, sceneY);
        if (p.x < 0 || p.y < 0 || p.x > width || p.y > height) return false;
        committed(entry.path);
        return true;
    }
    Component.onCompleted: app.assetTargets.push(root)
    Component.onDestruction: { const i = app.assetTargets.indexOf(root); if (i >= 0) app.assetTargets.splice(i, 1); }
}
