import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The actors in the world, as a parent/child tree. Selecting one swaps the
// canvas to its own blocks. Dragging a row moves it: the top or bottom edge
// of another row drops next to it, the middle drops inside it, and the list
// background moves it to the end of the top level. A drop that would make a
// loop is refused, the same way the backend refuses it.
Rectangle {
    id: root
    required property var app
    property bool open: true
    signal openRequested(bool open)
    signal resizeRequested(real width)
    readonly property var appState: app.appState
    readonly property var actors: appState.project ? appState.project.actors : []
    readonly property string mode: appState.project ? appState.project.world.mode : "TwoD"
    color: Theme.panel
    border.color: Theme.borderSoft
    clip: true

    Settings { id: remembered; category: "actors"; property string collapsed: "[]" }
    property var collapsed: { try { return JSON.parse(remembered.collapsed); } catch (e) { return []; } }
    function toggleCollapsed(id) {
        const next = collapsed.slice();
        const at = next.indexOf(id);
        if (at >= 0) next.splice(at, 1); else next.push(id);
        remembered.collapsed = JSON.stringify(next);
    }

    // ─── The tree ──────────────────────────────────────────────────────────
    function parentOf(actor) {
        const p = actor ? actor.components.find(c => c.component === "Parent") : null;
        return p && p.parent ? p.parent : null;
    }
    function byId(id) { return actors.find(a => a.id === id) || null; }
    function rootActors() { return actors.filter(a => { const p = parentOf(a); return !p || !byId(p); }); }
    function childrenOf(id) { return actors.filter(a => parentOf(a) === id); }
    // Whether hanging `childId` off `parentId` would make a loop.
    function wouldCycle(childId, parentId) {
        if (parentId === childId) return true;
        const seen = {};
        let at = parentId;
        while (at && !seen[at]) { seen[at] = true; at = parentOf(byId(at)); if (at === childId) return true; }
        return false;
    }
    readonly property var rows: {
        const out = [];
        const walk = (actor, depth) => {
            out.push({ actor: actor, depth: depth, kids: childrenOf(actor.id).length });
            if (collapsed.indexOf(actor.id) >= 0) return;
            for (const child of childrenOf(actor.id)) walk(child, depth + 1);
        };
        for (const a of rootActors()) walk(a, 0);
        return out;
    }

    function visualOf(actor) { const look = actor.components.find(c => c.component === "Look"); return look ? look.visual : null; }
    function wrongDimension(actor) {
        const v = visualOf(actor);
        const list = mode === "ThreeD" ? ["Cuboid","Sphere","Capsule","Plane","Model","Tilemap"] : ["Rect","Circle","Image","Tilemap"];
        return !!v && list.indexOf(v.shape) < 0;
    }
    function bodyBadge(actor) {
        const body = actor.components.find(c => c.component === "Body");
        return body && body.physics.body !== "None" ? body.physics.body[0] : "";
    }

    // ─── Moving ────────────────────────────────────────────────────────────
    property string dragging: ""
    property string hintId: ""
    property string hintPos: ""   // before | after | in
    property bool rootHover: false
    function moveFor(targetId, pos) {
        if (pos === "in") return { parent: targetId, before: "" };
        const parent = parentOf(byId(targetId)) || "";
        if (pos === "before") return { parent: parent, before: targetId };
        const level = parent ? childrenOf(parent) : rootActors();
        const next = level[level.findIndex(a => a.id === targetId) + 1];
        return { parent: parent, before: next ? next.id : "" };
    }
    function dropInvalid(draggedId, targetId, pos) {
        if (draggedId === targetId) return true;
        const move = moveFor(targetId, pos);
        return !!move.parent && wouldCycle(draggedId, move.parent);
    }
    function updateHint(sceneX, sceneY) {
        hintId = ""; hintPos = ""; rootHover = false;
        const p = list.contentItem.mapFromItem(null, sceneX, sceneY);
        const item = list.itemAt(p.x, p.y);
        if (!item) { rootHover = list.contains(list.mapFromItem(null, sceneX, sceneY)); return; }
        const local = item.mapFromItem(null, sceneX, sceneY);
        const ratio = local.y / item.height;
        const pos = ratio < 0.25 ? "before" : ratio > 0.75 ? "after" : "in";
        if (dropInvalid(dragging, item.actorId, pos)) return;
        hintId = item.actorId; hintPos = pos;
    }
    function finishDrag() {
        const dragged = dragging;
        dragging = "";
        if (hintId) {
            const move = moveFor(hintId, hintPos);
            if (move.parent && collapsed.indexOf(move.parent) >= 0) toggleCollapsed(move.parent);
            app.invoke("move_actor", { actorId: dragged, parent: move.parent, before: move.before });
        } else if (rootHover) {
            app.invoke("move_actor", { actorId: dragged, parent: "", before: "" });
        }
        hintId = ""; hintPos = ""; rootHover = false;
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        visible: root.open
        RowLayout {
            Layout.fillWidth: true; Layout.margins: 8; spacing: 4
            BwComboBox {
                id: sceneBox
                Layout.fillWidth: true; implicitHeight: 28; font.pixelSize: 12
                readonly property var scenes: root.appState.project ? root.appState.project.scenes : []
                readonly property string active: root.appState.project ? root.appState.project.active_scene : ""
                model: scenes
                textRole: "name"
                currentIndex: {
                    const at = scenes.findIndex(s => s.id === active);
                    return at >= 0 ? at : -1;
                }
                displayText: {
                    const s = scenes.find(s => s.id === active);
                    return s ? s.name : "Scene";
                }
                onActivated: index => {
                    const s = scenes[index];
                    if (s && s.id !== active) root.app.invoke("set_active_scene", { sceneId: s.id });
                    currentIndex = Qt.binding(() => {
                        const at = sceneBox.scenes.findIndex(s => s.id === sceneBox.active);
                        return at >= 0 ? at : -1;
                    });
                }
                ToolTip.text: "Scene asset - one .blockscene file each"
            }
            IconButton { iconName: "plus"; tip: "Add scene"; implicitWidth: 26; implicitHeight: 26; onClicked: root.app.invoke("add_scene", { name: "" }) }
        }
        RowLayout {
            Layout.fillWidth: true; Layout.margins: 8; spacing: 4
            SectionLabel { label: "Actors"; topPadding: 0; Layout.fillWidth: true }
            BwComboBox {
                id: addBox
                implicitWidth: 86; implicitHeight: 28; font.pixelSize: 12
                readonly property var shapes: root.mode === "ThreeD"
                    ? [{value:"Cuboid",label:"Box"},{value:"Sphere",label:"Sphere"},{value:"Capsule",label:"Capsule"},{value:"Plane",label:"Ground plane"},{value:"Model",label:"Model"},{value:"Tilemap",label:"Tilemap"}]
                    : [{value:"Rect",label:"Square"},{value:"Circle",label:"Ball"},{value:"Image",label:"Image"},{value:"Tilemap",label:"Tilemap"}]
                model: shapes; textRole: "label"; currentIndex: -1; displayText: "Add"
                onActivated: index => { root.app.invoke("add_actor", { shape: shapes[index].value }); currentIndex = -1; }
            }
            IconButton { iconName: "chevron-left"; tip: "Hide the actor list"; implicitWidth: 26; implicitHeight: 26; onClicked: root.openRequested(false) }
        }
        ListView {
            id: list
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: root.rows
            ScrollBar.vertical: ScrollBar {}
            Rectangle { anchors.fill: parent; z: -1; color: root.rootHover ? "#1a1597ff" : "transparent" }
            delegate: Rectangle {
                id: row
                required property var modelData
                readonly property string actorId: modelData.actor.id
                readonly property bool selected: actorId === root.appState.selected_actor
                width: list.width; height: 32
                color: selected ? "#2b4a6b" : (rowHover.hovered ? "#303134" : "transparent")
                border.color: root.hintId === actorId && root.hintPos === "in" ? Theme.accent : "transparent"
                Rectangle { visible: root.hintId === row.actorId && root.hintPos === "before"; anchors.top: parent.top; width: parent.width; height: 2; color: Theme.accent }
                Rectangle { visible: root.hintId === row.actorId && root.hintPos === "after"; anchors.bottom: parent.bottom; width: parent.width; height: 2; color: Theme.accent }
                HoverHandler { id: rowHover }
                MouseArea {
                    anchors.fill: parent
                    property point press
                    property bool moved: false
                    onPressed: mouse => { press = mapToItem(null, mouse.x, mouse.y); moved = false; }
                    onPositionChanged: mouse => {
                        const p = mapToItem(null, mouse.x, mouse.y);
                        if (!moved && Math.hypot(p.x - press.x, p.y - press.y) > 6) { moved = true; root.dragging = row.actorId; }
                        if (moved) root.updateHint(p.x, p.y);
                    }
                    onReleased: { if (moved) root.finishDrag(); else root.app.invoke("select_actor", { actorId: row.actorId }); }
                    onCanceled: { root.dragging = ""; root.hintId = ""; root.rootHover = false; }
                }
                RowLayout {
                    anchors.fill: parent; anchors.leftMargin: 8 + row.modelData.depth * 16; anchors.rightMargin: 8; spacing: 6
                    Item {
                        implicitWidth: 16; implicitHeight: 16
                        LucideIcon {
                            visible: row.modelData.kids > 0; anchors.fill: parent; color: Theme.textDim
                            name: root.collapsed.indexOf(row.actorId) >= 0 ? "chevron-right" : "chevron-down"
                            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.toggleCollapsed(row.actorId) }
                        }
                    }
                    Rectangle {
                        readonly property var visual: root.visualOf(row.modelData.actor)
                        implicitWidth: 14; implicitHeight: 14
                        radius: visual && (visual.shape === "Circle" || visual.shape === "Sphere") ? 7 : 3
                        color: visual && visual.color ? visual.color : "#8e8e93"
                        border.color: Theme.border
                    }
                    Text { Layout.fillWidth: true; text: row.modelData.actor.name; color: Theme.text; font.pixelSize: 13; elide: Text.ElideRight }
                    Rectangle {
                        readonly property string badge: root.wrongDimension(row.modelData.actor) ? "!" : root.bodyBadge(row.modelData.actor)
                        visible: badge.length > 0
                        implicitWidth: 20; implicitHeight: 18; radius: 9; color: badge === "!" ? "#5a2a2a" : Theme.panelRaised; border.color: Theme.border
                        Text { anchors.centerIn: parent; text: parent.badge; color: parent.badge === "!" ? Theme.danger : Theme.textDim; font.pixelSize: 10; font.weight: Font.Bold }
                    }
                }
            }
        }
        RowLayout {
            visible: !!root.appState.selected_actor
            Layout.fillWidth: true; Layout.margins: 8; spacing: 4
            IconButton { iconName: "copy"; tip: "Duplicate this actor"; flat: false; onClicked: root.app.invoke("duplicate_actor", { actorId: root.appState.selected_actor }) }
            IconButton { iconName: "trash-2"; tip: "Delete this actor"; flat: false; danger: true; onClicked: removeDialog.open() }
        }
    }
    // The inner edge drags to resize.
    MouseArea {
        visible: root.open
        anchors.top: parent.top; anchors.bottom: parent.bottom; anchors.right: parent.right; width: 5
        cursorShape: Qt.SizeHorCursor
        property real startWidth: 0; property real startX: 0
        onPressed: mouse => { startWidth = root.width; startX = mapToItem(null, mouse.x, 0).x; }
        onPositionChanged: mouse => { if (pressed) root.resizeRequested(startWidth + mapToItem(null, mouse.x, 0).x - startX); }
    }
    IconButton {
        visible: !root.open
        anchors.horizontalCenter: parent.horizontalCenter; y: 10
        iconName: "chevron-right"; tip: "Show the actors"
        onClicked: root.openRequested(true)
    }
    BwDialog {
        id: removeDialog
        readonly property var actor: root.byId(root.appState.selected_actor)
        title: "Delete actor?"
        standardButtons: Dialog.Yes | Dialog.Cancel
        Text { text: removeDialog.actor ? "Delete actor “" + removeDialog.actor.name + "”? This cannot be undone." : ""; color: Theme.text }
        onAccepted: if (actor) root.app.invoke("remove_actor", { actorId: actor.id })
    }
}
