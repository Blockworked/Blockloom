import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The right-hand panel: the selected actor as a list of components. Each
// component is a card with a way to remove it, and "Add component" gives the
// actor one it hasn't got - including a custom one, a named bag of values the
// blocks read and write. Every field writes straight through to the backend.
Rectangle {
    id: root
    required property var app
    property bool open: true
    signal openRequested(bool open)
    signal resizeRequested(real width)
    readonly property var appState: app.appState
    readonly property var actor: app.openActor
    readonly property string mode: appState.project ? appState.project.world.mode : "TwoD"
    readonly property bool is3d: mode === "ThreeD"
    // Where the actor is right now, while a run is going.
    readonly property var live: actor && app.status ? (app.status.actors.find(a => a.id === actor.id) || null) : null
    color: Theme.panel
    border.color: Theme.borderSoft
    clip: true

    function componentName(c) { return c.component === "Custom" ? c.name : c.component; }
    function write(name, component) { if (actor) app.invoke("set_actor_component", { actorId: actor.id, name: name, component: component }); }
    function remove(name) { if (actor) app.invoke("remove_actor_component", { actorId: actor.id, name: name }); }
    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function withIndex(array, index, value) { const next = array.slice(); next[index] = value; return next; }
    function merged(base, next) { return Object.assign(copy(base), next); }

    // ─── Components with defaults for what an old document left out ────────
    function physicsOf(c) {
        return Object.assign({ body: "None", gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5, density: 1, mass: null, trigger: false, collision_layer: 1, collision_mask: 255 }, c.physics || {});
    }
    function cameraOf(c) { return Object.assign({ view: "Follow", offset: [0, 0.6, 0], distance: 6, pitch: 15, fov: 75 }, c.camera || {}); }
    function materialOf(c) { return Object.assign({ metallic: 0, roughness: 0.6, emissive: "#000000", emissive_energy: 0, albedo_texture: "", double_sided: false, shader: null }, c.material || {}); }
    function emitterOf(c) { return Object.assign({ rate: 24, lifetime: 0.8, speed: 120, spread: 60, gravity_scale: 0.5, size_start: 6, size_end: 1, color_start: "#FFFFFF", color_end: "#FFAB19", max: 128 }, c.emitter || {}); }
    function trailOf(c) { return Object.assign({ interval: 0.05, life: 0.4, color: "#FFFFFF" }, c.trail || {}); }
    function tilemapOf(v) {
        return Object.assign({ tileset: "", tile_size: [32, 32], width: 8, height: 8, sheet_columns: 4, sheet_rows: 4, tiles: [], solid: false, passable: [], animations: [] }, v && v.tilemap ? v.tilemap : {});
    }
    // "3, 7 12" -> [3, 7, 12]: sheet indices typed as a list.
    function tileList(text) { return text.split(/[\s,]+/).filter(t => t !== "").map(Number).filter(n => Number.isInteger(n) && n >= 0); }
    function writeTileAnimation(c, index, next) {
        const list = copy(tilemapOf(c.visual).animations);
        if (next === null) list.splice(index, 1); else list[index] = Object.assign(list[index], next);
        writeTilemap(c, { animations: list });
    }

    function writePlacement(c, next) { write("Place", { component: "Place", placement: merged(c.placement, next) }); }
    function writeVector(c, key, index, value) { const next = {}; next[key] = withIndex(c.placement[key], index, value); writePlacement(c, next); }
    function writeVisual(c, next) { write("Look", { component: "Look", visual: merged(c.visual, next) }); }
    function writePhysics(c, next) { write("Body", { component: "Body", physics: merged(physicsOf(c), next) }); }
    function writeCamera(c, next) { write("Camera", { component: "Camera", camera: merged(cameraOf(c), next) }); }
    function writeMaterial(c, next) { write("Material", { component: "Material", material: merged(materialOf(c), next) }); }
    function writeShader(c, next) { writeMaterial(c, { shader: merged(materialOf(c).shader || { mode: "Solid", speed: 1, strength: 0.5, color: "#FFFFFF" }, next) }); }
    function writeEmitter(c, next) { write("Emitter", { component: "Emitter", emitter: merged(emitterOf(c), next) }); }
    function writeTrail(c, next) { write("Trail", { component: "Trail", trail: merged(trailOf(c), next) }); }
    function writeRender(c, next) { write("Render", { component: "Render", visible: next.visible !== undefined ? next.visible : c.visible, layer: next.layer !== undefined ? next.layer : (c.layer || 0) }); }
    function writeParent(c, next) {
        write("Parent", { component: "Parent", parent: next.parent !== undefined ? next.parent : c.parent, offset: next.offset !== undefined ? next.offset : (c.offset || null) });
    }
    function writeCustom(c, next) { write(c.name, { component: "Custom", name: next.name !== undefined ? next.name : c.name, fields: next.fields !== undefined ? next.fields : c.fields }); }
    // The grid is always exactly width x height; resizing keeps what overlaps.
    function writeTilemap(c, next) {
        const old = tilemapOf(c.visual);
        const t = Object.assign(copy(old), next);
        const width = Math.min(256, Math.max(1, Math.round(t.width)));
        const height = Math.min(256, Math.max(1, Math.round(t.height)));
        const tiles = [];
        for (let y = 0; y < height; ++y)
            for (let x = 0; x < width; ++x)
                tiles.push(y < old.height && x < old.width ? (old.tiles[y * old.width + x] !== undefined ? old.tiles[y * old.width + x] : -1) : -1);
        t.width = width; t.height = height; t.tiles = tiles;
        writeVisual(c, { tilemap: t });
    }
    property int paintTile: 0
    function paintAt(c, index) {
        const t = copy(tilemapOf(c.visual));
        const cells = Math.max(1, t.sheet_columns * t.sheet_rows);
        t.tiles[index] = paintTile >= 0 && paintTile < cells ? paintTile : -1;
        writeVisual(c, { tilemap: t });
    }
    // Swapping a shape keeps its color and takes sensible defaults for the rest.
    function writeShape(c, shape) {
        const color = c.visual.color || c.visual.tint || "#4C97FF";
        const visuals = {
            Rect: { shape: "Rect", color: color, size: [60, 60] },
            Circle: { shape: "Circle", color: color, radius: 30 },
            Image: { shape: "Image", path: "", size: [80, 80] },
            Cuboid: { shape: "Cuboid", color: color, size: [1, 1, 1] },
            Sphere: { shape: "Sphere", color: color, radius: 0.5 },
            Capsule: { shape: "Capsule", color: color, radius: 0.4, height: 1 },
            Plane: { shape: "Plane", color: color, size: [20, 20] },
            Model: { shape: "Model", path: "", tint: color, scale: [1, 1, 1], animation: "" },
            Tilemap: { shape: "Tilemap", tilemap: { tileset: "", tile_size: [32, 32], width: 8, height: 8, sheet_columns: 4, sheet_rows: 4, tiles: Array(64).fill(-1), solid: false, passable: [], animations: [] } }
        };
        if (visuals[shape]) write("Look", { component: "Look", visual: visuals[shape] });
    }
    // Keeps a field's type as typed: a number that parses stays a number.
    function parsedValue(raw) {
        const t = raw.trim();
        return t !== "" && Number.isFinite(Number(t)) ? { kind: "Number", value: Number(t) } : { kind: "Text", value: raw };
    }
    function hangsOffMe(id) {
        const seen = {};
        let at = id;
        while (at && !seen[at]) {
            seen[at] = true;
            const a = appState.project.actors.find(x => x.id === at);
            const p = a ? a.components.find(x => x.component === "Parent") : null;
            at = p && p.parent ? p.parent : null;
            if (at === actor.id) return true;
        }
        return false;
    }
    readonly property var parentOptions: actor ? [{ value: "", label: "nothing" }].concat(appState.project.actors.filter(a => a.id !== actor.id && !hangsOffMe(a.id)).map(a => ({ value: a.id, label: a.name }))) : []
    readonly property var addable: {
        if (!actor) return [];
        const held = actor.components.map(componentName);
        return ["Look","Render","Body","Camera","Script","Parent","Material","Emitter","Trail","Custom"]
            .filter(n => n === "Custom" || held.indexOf(n) < 0).map(n => ({ value: n, label: n === "Custom" ? "Custom…" : n }));
    }
    function blank(name) {
        switch (name) {
        case "Look": return { component: "Look", visual: is3d ? { shape: "Cuboid", color: "#4C97FF", size: [1, 1, 1] } : { shape: "Rect", color: "#4C97FF", size: [60, 60] } };
        case "Render": return { component: "Render", visible: true, layer: 0 };
        case "Body": return { component: "Body", physics: { body: "Dynamic", gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5, density: 1, mass: null, trigger: false, collision_layer: 1, collision_mask: 255 } };
        case "Camera": return { component: "Camera", camera: { view: "ThirdPerson", offset: [0, 0.6, 0], distance: 6, pitch: 15, fov: 75 } };
        case "Parent": return { component: "Parent", parent: "", offset: null };
        case "Material": return { component: "Material", material: materialOf({}) };
        case "Emitter": return { component: "Emitter", emitter: emitterOf({}) };
        case "Trail": return { component: "Trail", trail: trailOf({}) };
        case "Custom": return { component: "Custom", name: "Component", fields: [{ name: "value", value: { kind: "Number", value: 0 } }] };
        default: return null;
        }
    }
    function add(name) {
        if (!actor) return;
        // A script needs a file on disk, so the backend makes both at once.
        if (name === "Script") { app.invoke("create_script", { actorId: actor.id }); return; }
        const c = blank(name);
        if (c) app.invoke("add_actor_component", { actorId: actor.id, component: c });
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        visible: root.open && !!root.actor
        RowLayout {
            Layout.fillWidth: true; Layout.margins: 8
            SectionLabel { label: root.actor ? root.actor.name : ""; topPadding: 0; Layout.fillWidth: true; elide: Text.ElideRight }
            IconButton { iconName: "chevron-right"; tip: "Hide the components"; implicitWidth: 26; implicitHeight: 26; onClicked: root.openRequested(false) }
        }
        ScrollView {
            id: scroll
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: scroll.availableWidth - 16; x: 8; spacing: 6
                InspectorRow {
                    label: "Name"; Layout.fillWidth: true
                    BwTextField {
                        Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                        text: root.actor ? root.actor.name : ""
                        onEditingFinished: if (root.actor && text !== root.actor.name) root.app.invoke("rename_actor", { actorId: root.actor.id, name: text })
                    }
                }
                // A count rather than the array: a new snapshot with the same
                // components updates the cards in place instead of rebuilding them.
                Repeater {
                    model: root.actor ? root.actor.components.length : 0
                    delegate: ColumnLayout {
                        id: card
                        required property int index
                        readonly property var c: root.actor && root.actor.components[index] ? root.actor.components[index] : ({ component: "" })
                        Layout.fillWidth: true; spacing: 6
                        Rectangle { Layout.fillWidth: true; Layout.topMargin: 6; height: 1; color: Theme.borderSoft }
                        RowLayout {
                            Layout.fillWidth: true
                            Text { Layout.fillWidth: true; text: root.componentName(card.c); color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
                            IconButton { visible: card.c.component !== "Place"; iconName: "x"; tip: "Remove the " + root.componentName(card.c) + " component"; implicitWidth: 24; implicitHeight: 24; onClicked: root.remove(root.componentName(card.c)) }
                        }
                        Loader {
                            Layout.fillWidth: true
                            readonly property var c: card.c
                            sourceComponent: ({ Place: placeCard, Look: lookCard, Parent: parentCard, Render: renderCard, Body: bodyCard, Camera: cameraCard,
                                                Script: scriptCard, Custom: customCard, Material: materialCard, Emitter: emitterCard, Trail: trailCard })[card.c.component] || null
                        }
                    }
                }
                Item { Layout.preferredHeight: 6 }
                ChoiceField {
                    Layout.fillWidth: true
                    options: root.addable; value: ""; placeholder: "Add component"
                    onChosen: v => root.add(v)
                }
                Item { Layout.preferredHeight: 12 }
            }
        }
    }
    Text {
        visible: root.open && !root.actor
        anchors.centerIn: parent; width: parent.width - 32; wrapMode: Text.WordWrap; horizontalAlignment: Text.AlignHCenter
        text: "Select an actor to see its components."; color: Theme.textDim; font.pixelSize: 12
    }

    // ─── One card per component kind. `parent.c` is the component. ─────────
    Component {
        id: placeCard
        ColumnLayout {
            readonly property var c: parent.c
            spacing: 6
            InspectorRow { label: "Position"; Layout.fillWidth: true
                NumberField { value: c.placement.position[0]; onCommitted: n => root.writeVector(c, "position", 0, n) }
                NumberField { value: c.placement.position[1]; onCommitted: n => root.writeVector(c, "position", 1, n) }
                NumberField { visible: root.is3d; value: c.placement.position[2]; onCommitted: n => root.writeVector(c, "position", 2, n) } }
            InspectorRow { label: "Rotation"; Layout.fillWidth: true
                NumberField { visible: root.is3d; value: c.placement.rotation[0]; onCommitted: n => root.writeVector(c, "rotation", 0, n) }
                NumberField { visible: root.is3d; value: c.placement.rotation[1]; onCommitted: n => root.writeVector(c, "rotation", 1, n) }
                NumberField { value: c.placement.rotation[2]; onCommitted: n => root.writeVector(c, "rotation", 2, n) } }
            InspectorRow { label: "Size"; Layout.fillWidth: true
                NumberField { value: c.placement.scale; fallback: 1; onCommitted: n => root.writePlacement(c, { scale: n }) } }
            InspectorRow { id: stretchRow; label: "Stretch"; Layout.fillWidth: true
                readonly property var s: c.placement.stretch || [1, 1, 1]
                NumberField { value: stretchRow.s[0]; fallback: 1; onCommitted: n => root.writePlacement(c, { stretch: root.withIndex(stretchRow.s, 0, n) }) }
                NumberField { value: stretchRow.s[1]; fallback: 1; onCommitted: n => root.writePlacement(c, { stretch: root.withIndex(stretchRow.s, 1, n) }) }
                NumberField { visible: root.is3d; value: stretchRow.s[2]; fallback: 1; onCommitted: n => root.writePlacement(c, { stretch: root.withIndex(stretchRow.s, 2, n) }) } }
            Text { visible: !!root.live; text: root.live ? "Now at " + root.live.position.map(n => n.toFixed(1)).join(", ") : ""; color: Theme.textDim; font.pixelSize: 11 }
        }
    }
    Component {
        id: lookCard
        ColumnLayout {
            id: look
            readonly property var c: parent.c
            readonly property var v: c.visual
            readonly property var t: root.tilemapOf(v)
            property bool aspectLocked: true
            spacing: 6
            InspectorRow { label: "Shape"; Layout.fillWidth: true
                ChoiceField { options: (root.is3d ? ["Cuboid","Sphere","Capsule","Plane","Model","Tilemap"] : ["Rect","Circle","Image","Tilemap"]).map(s => ({ value: s, label: s })); value: look.v.shape; onChosen: s => root.writeShape(look.c, s) } }
            InspectorRow { visible: look.v.color !== undefined; label: "Color"; Layout.fillWidth: true
                ColorField { value: look.v.color || "#FFFFFF"; onPicked: col => root.writeVisual(look.c, { color: col }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: look.v.shape === "Image"; label: "Image"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["image"]; value: look.v.path || ""; placeholderText: "Drag an image here"; onCommitted: p => root.writeVisual(look.c, { path: p }) } }
            Image {
                id: preview
                visible: look.v.shape === "Image" && status === Image.Ready
                Layout.preferredHeight: 90; Layout.fillWidth: true; Layout.leftMargin: 84
                fillMode: Image.PreserveAspectFit; horizontalAlignment: Image.AlignLeft
                source: look.v.shape === "Image" ? root.app.assetUrl(look.v.path) : ""
                cache: false; asynchronous: true
            }
            InspectorRow { visible: look.v.shape === "Model"; label: "Model"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["model"]; value: look.v.path || ""; placeholderText: "Drag a model here"; onCommitted: p => root.writeVisual(look.c, { path: p }) } }
            InspectorRow { visible: look.v.shape === "Model"; label: "Tint"; Layout.fillWidth: true
                ColorField { value: look.v.tint || "#4C97FF"; onPicked: col => root.writeVisual(look.c, { tint: col }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: look.v.shape === "Model"; label: "Scale"; Layout.fillWidth: true
                Repeater { model: 3; delegate: NumberField { required property int index; value: look.v.scale ? look.v.scale[index] : 1; fallback: 1
                    onCommitted: n => root.writeVisual(look.c, { scale: root.withIndex(look.v.scale || [1, 1, 1], index, n) }) } } }
            InspectorRow { visible: look.v.shape === "Model"; label: "Animation"; Layout.fillWidth: true
                BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "First in the file"; text: look.v.animation || ""
                    onEditingFinished: if (text.trim() !== (look.v.animation || "")) root.writeVisual(look.c, { animation: text.trim() }) } }
            Text { visible: look.v.shape === "Model"; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: (look.v.path ? "" : "No file yet, so the actor shows as its box. ")
                    + "glTF and GLB draw as the file, scaled by Scale, and loop the named animation. OBJ and FBX show as the box, which is also what it collides as." }
            ColumnLayout {
                visible: look.v.shape === "Tilemap"; Layout.fillWidth: true; spacing: 6
                InspectorRow { label: "Tileset"; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["image"]; value: look.t.tileset; placeholderText: "Drag a tileset here"; onCommitted: p => root.writeTilemap(look.c, { tileset: p }) } }
                InspectorRow { label: "Tile px"; Layout.fillWidth: true
                    NumberField { value: look.t.tile_size[0]; fallback: 32; onCommitted: n => root.writeTilemap(look.c, { tile_size: [Math.max(1, n), look.t.tile_size[1]] }) }
                    NumberField { value: look.t.tile_size[1]; fallback: 32; onCommitted: n => root.writeTilemap(look.c, { tile_size: [look.t.tile_size[0], Math.max(1, n)] }) } }
                InspectorRow { label: "Map"; Layout.fillWidth: true
                    NumberField { value: look.t.width; fallback: 8; onCommitted: n => root.writeTilemap(look.c, { width: n }) }
                    NumberField { value: look.t.height; fallback: 8; onCommitted: n => root.writeTilemap(look.c, { height: n }) } }
                InspectorRow { label: "Sheet"; Layout.fillWidth: true
                    NumberField { value: look.t.sheet_columns; fallback: 4; onCommitted: n => root.writeTilemap(look.c, { sheet_columns: Math.max(1, Math.round(n)) }) }
                    NumberField { value: look.t.sheet_rows; fallback: 4; onCommitted: n => root.writeTilemap(look.c, { sheet_rows: Math.max(1, Math.round(n)) }) } }
                InspectorRow { label: "Solid"; Layout.fillWidth: true
                    SwitchField { value: look.t.solid; onToggled: on => root.writeTilemap(look.c, { solid: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: look.t.solid; label: "Passable"; Layout.fillWidth: true
                    BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Tiles bodies pass, e.g. 3, 7"
                        text: look.t.passable.join(", ")
                        onEditingFinished: { const next = root.tileList(text); if (next.join(",") !== look.t.passable.join(",")) root.writeTilemap(look.c, { passable: next }); else text = look.t.passable.join(", "); } } }
                Repeater {
                    model: look.t.animations.length
                    delegate: InspectorRow {
                        required property int index
                        readonly property var anim: look.t.animations[index] || { tile: 0, frames: [], fps: 8 }
                        label: "Animated"; Layout.fillWidth: true
                        NumberField { Layout.maximumWidth: 48; value: anim.tile; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "The painted tile that animates"
                            onCommitted: n => root.writeTileAnimation(look.c, index, { tile: Math.max(0, Math.round(n)) }) }
                        BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Frames, e.g. 4, 5, 6"; text: anim.frames.join(", ")
                            onEditingFinished: { const next = root.tileList(text); if (next.join(",") !== anim.frames.join(",")) root.writeTileAnimation(look.c, index, { frames: next }); else text = anim.frames.join(", "); } }
                        NumberField { Layout.maximumWidth: 48; value: anim.fps; fallback: 8; ToolTip.visible: hovered; ToolTip.text: "Frames a second"
                            onCommitted: n => root.writeTileAnimation(look.c, index, { fps: n }) }
                        IconButton { iconName: "x"; tip: "Stop animating this tile"; implicitWidth: 24; implicitHeight: 24; onClicked: root.writeTileAnimation(look.c, index, null) }
                    }
                }
                BwButton {
                    iconName: "plus"; text: "Animate a tile"; implicitHeight: 28; font.pixelSize: 12
                    onClicked: {
                        const list = root.copy(look.t.animations);
                        const tile = Math.max(0, root.paintTile);
                        list.push({ tile: tile, frames: [tile], fps: 8 });
                        root.writeTilemap(look.c, { animations: list });
                    }
                }
                InspectorRow { label: "Paint"; Layout.fillWidth: true
                    NumberField { value: root.paintTile; onCommitted: n => root.paintTile = Math.round(n) }
                    Text { text: "-1 erases"; color: Theme.textDim; font.pixelSize: 11 } }
                Flickable {
                    Layout.fillWidth: true; Layout.preferredHeight: Math.min(260, tileGrid.height)
                    contentWidth: tileGrid.width; contentHeight: tileGrid.height; clip: true
                    Grid {
                        id: tileGrid
                        columns: look.t.width; spacing: 1
                        Repeater {
                            model: look.t.tiles.length
                            delegate: Rectangle {
                                required property int index
                                readonly property var modelData: look.t.tiles[index]
                                width: 18; height: 18; radius: 2
                                color: modelData >= 0 ? "#2b4a6b" : Theme.field; border.color: Theme.borderSoft
                                Text { anchors.centerIn: parent; text: modelData >= 0 ? modelData : ""; color: Theme.text; font.pixelSize: 9 }
                                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.paintAt(look.c, index) }
                            }
                        }
                    }
                }
            }
            InspectorRow { visible: look.v.radius !== undefined; label: "Radius"; Layout.fillWidth: true
                NumberField { value: look.v.radius; fallback: 1; onCommitted: n => root.writeVisual(look.c, { radius: n }) } }
            InspectorRow { visible: look.v.shape === "Capsule"; label: "Height"; Layout.fillWidth: true
                NumberField { value: look.v.height; fallback: 1; onCommitted: n => root.writeVisual(look.c, { height: n }) } }
            InspectorRow {
                visible: !!look.v.size && look.v.shape !== "Tilemap"; label: "Size"; Layout.fillWidth: true
                Repeater {
                    model: look.v.size ? look.v.size.length : 0
                    delegate: NumberField {
                        required property int index
                        value: look.v.size[index]
                        onCommitted: n => {
                            // An Image keeps its file's aspect while the lock is on.
                            const natural = preview.status === Image.Ready ? preview.implicitWidth / Math.max(1, preview.implicitHeight) : 0;
                            if (look.v.shape === "Image" && look.aspectLocked && natural > 0 && n > 0)
                                root.writeVisual(look.c, { size: index === 0 ? [n, n / natural] : [n * natural, n] });
                            else root.writeVisual(look.c, { size: root.withIndex(look.v.size, index, n) });
                        }
                    }
                }
                IconButton {
                    visible: look.v.shape === "Image"
                    iconName: look.aspectLocked ? "lock" : "lock-open"
                    tip: preview.status === Image.Ready ? (look.aspectLocked ? "Locked to " : "Lock to ") + preview.implicitWidth + "x" + preview.implicitHeight : "Load an image to lock its aspect"
                    enabled: preview.status === Image.Ready
                    implicitWidth: 26; implicitHeight: 26
                    onClicked: {
                        look.aspectLocked = !look.aspectLocked;
                        const natural = preview.implicitWidth / Math.max(1, preview.implicitHeight);
                        if (look.aspectLocked && look.v.size[0] > 0) root.writeVisual(look.c, { size: [look.v.size[0], look.v.size[0] / natural] });
                    }
                }
            }
        }
    }
    Component {
        id: parentCard
        ColumnLayout {
            readonly property var c: parent.c
            spacing: 6
            InspectorRow { label: "Hangs off"; Layout.fillWidth: true
                ChoiceField { options: root.parentOptions; value: c.parent; placeholder: "nothing"; onChosen: id => root.writeParent(c, { parent: id }) } }
            InspectorRow { visible: !!c.parent; label: "Placed by it"; Layout.fillWidth: true
                SwitchField {
                    value: !!c.offset
                    onToggled: on => {
                        if (!on) { root.writeParent(c, { offset: null }); return; }
                        // Measured from where the two stand now, so the actor doesn't move.
                        const place = a => { const p = a ? a.components.find(x => x.component === "Place") : null; return p ? p.placement.position : [0, 0, 0]; };
                        const mine = place(root.actor), theirs = place(root.appState.project.actors.find(a => a.id === c.parent));
                        root.writeParent(c, { offset: [mine[0] - theirs[0], mine[1] - theirs[1], mine[2] - theirs[2]] });
                    }
                }
                Item { Layout.fillWidth: true } }
            InspectorRow { visible: !!c.offset; label: "Offset"; Layout.fillWidth: true
                NumberField { value: c.offset ? c.offset[0] : 0; onCommitted: n => root.writeParent(c, { offset: root.withIndex(c.offset, 0, n) }) }
                NumberField { value: c.offset ? c.offset[1] : 0; onCommitted: n => root.writeParent(c, { offset: root.withIndex(c.offset, 1, n) }) }
                NumberField { visible: root.is3d; value: c.offset ? c.offset[2] : 0; onCommitted: n => root.writeParent(c, { offset: root.withIndex(c.offset, 2, n) }) } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: c.offset ? "This actor starts that far from its parent, in the parent's own frame, and every move the parent makes is made to it too."
                               : "This actor keeps its own place, and every move its parent makes is made to it too." }
        }
    }
    Component {
        id: renderCard
        ColumnLayout {
            readonly property var c: parent.c
            spacing: 6
            InspectorRow { label: "Visible"; Layout.fillWidth: true
                SwitchField { value: c.visible; onToggled: on => root.writeRender(c, { visible: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: !root.is3d; label: "Layer"; Layout.fillWidth: true
                NumberField { value: c.layer || 0; onCommitted: n => root.writeRender(c, { layer: Math.round(n) }) } }
            Text { visible: !root.is3d; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Higher layers draw on top, without touching the actor's depth." }
        }
    }
    Component {
        id: bodyCard
        ColumnLayout {
            id: body
            readonly property var c: parent.c
            readonly property var p: root.physicsOf(c)
            spacing: 6
            InspectorRow { label: "Kind"; Layout.fillWidth: true
                ChoiceField { options: Blocks.bodyOptions; value: body.p.body; onChosen: v => root.writePhysics(body.c, { body: v }) } }
            ColumnLayout {
                visible: body.p.body !== "None"; Layout.fillWidth: true; spacing: 6
                InspectorRow { label: "Gravity ×"; Layout.fillWidth: true; NumberField { value: body.p.gravity_scale; fallback: 1; onCommitted: n => root.writePhysics(body.c, { gravity_scale: n }) } }
                InspectorRow { label: "Bounce"; Layout.fillWidth: true; NumberField { value: body.p.restitution; onCommitted: n => root.writePhysics(body.c, { restitution: n }) } }
                InspectorRow { label: "Friction"; Layout.fillWidth: true; NumberField { value: body.p.friction; fallback: 0.5; onCommitted: n => root.writePhysics(body.c, { friction: n }) } }
                InspectorRow { label: "Density"; Layout.fillWidth: true; NumberField { value: body.p.density; fallback: 1; onCommitted: n => root.writePhysics(body.c, { density: n }) } }
                InspectorRow { label: "Mass"; Layout.fillWidth: true; NumberField { value: body.p.mass; allowEmpty: true; fallback: 1; placeholderText: "density decides"; onCommitted: n => root.writePhysics(body.c, { mass: n }) } }
                InspectorRow { label: "Upright"; Layout.fillWidth: true; SwitchField { value: body.p.lock_rotation; onToggled: on => root.writePhysics(body.c, { lock_rotation: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Trigger"; Layout.fillWidth: true; SwitchField { value: body.p.trigger; onToggled: on => root.writePhysics(body.c, { trigger: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Layer"; Layout.fillWidth: true
                    ChoiceField { options: Blocks.layerOptions; value: String(body.p.collision_layer); onChosen: v => root.writePhysics(body.c, { collision_layer: Number(v) }) } }
                InspectorRow { label: "Hits"; Layout.fillWidth: true
                    Repeater {
                        model: 8
                        delegate: Rectangle {
                            required property int index
                            readonly property bool on: (body.p.collision_mask & (1 << index)) !== 0
                            implicitWidth: 22; implicitHeight: 22; radius: 4
                            color: on ? Theme.accent : Theme.field; border.color: Theme.border
                            Text { anchors.centerIn: parent; text: index + 1; color: parent.on ? "white" : Theme.textDim; font.pixelSize: 11 }
                            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.writePhysics(body.c, { collision_mask: body.p.collision_mask ^ (1 << index) }) }
                        }
                    }
                    Item { Layout.fillWidth: true }
                }
            }
        }
    }
    Component {
        id: cameraCard
        ColumnLayout {
            id: cam
            readonly property var c: parent.c
            readonly property var k: root.cameraOf(c)
            spacing: 6
            InspectorRow { label: "View"; Layout.fillWidth: true
                ChoiceField { options: Blocks.cameraViewOptions; value: cam.k.view; onChosen: v => root.writeCamera(cam.c, { view: v }) } }
            InspectorRow { label: "Eye at"; Layout.fillWidth: true
                Repeater { model: 3; delegate: NumberField { required property int index; value: cam.k.offset[index]; onCommitted: n => root.writeCamera(cam.c, { offset: root.withIndex(cam.k.offset, index, n) }) } } }
            InspectorRow { visible: root.is3d && cam.k.view === "ThirdPerson"; label: "Distance"; Layout.fillWidth: true
                NumberField { value: cam.k.distance; fallback: 6; onCommitted: n => root.writeCamera(cam.c, { distance: n }) } }
            InspectorRow { visible: root.is3d && cam.k.view === "ThirdPerson"; label: "Pitch"; Layout.fillWidth: true
                NumberField { value: cam.k.pitch; fallback: 15; onCommitted: n => root.writeCamera(cam.c, { pitch: n }) } }
            InspectorRow { visible: root.is3d; label: "FOV"; Layout.fillWidth: true
                NumberField { value: cam.k.fov; fallback: 75; onCommitted: n => root.writeCamera(cam.c, { fov: n }) } }
            Text { visible: !root.is3d; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "A 2D world has no depth to stand in, so every view here just keeps this actor centered." }
        }
    }
    Component {
        id: scriptCard
        ColumnLayout {
            readonly property var c: parent.c
            spacing: 6
            AssetField { app: root.app; accept: ["script"]; value: c.path; Layout.fillWidth: true; onCommitted: p => root.write("Script", { component: "Script", path: p }) }
            RowLayout {
                BwButton { text: "Edit"; iconName: "file-code"; implicitHeight: 30; onClicked: scriptDialog.openFor(root.actor, c.path) }
                BwButton { text: "Check"; implicitHeight: 30; onClicked: root.app.invoke("check_script", { actorId: root.actor.id }) }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Real Rust, compiled when you press Play. It runs alongside this actor's blocks, not instead of them." }
        }
    }
    Component {
        id: customCard
        ColumnLayout {
            id: custom
            readonly property var c: parent.c
            spacing: 6
            InspectorRow { label: "Name"; Layout.fillWidth: true
                BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; text: custom.c.name; onEditingFinished: if (text !== custom.c.name) root.writeCustom(custom.c, { name: text }) } }
            Repeater {
                model: custom.c.fields.length
                delegate: RowLayout {
                    required property int index
                    readonly property var modelData: custom.c.fields[index] || { name: "", value: { value: "" } }
                    Layout.fillWidth: true; spacing: 4
                    BwTextField { Layout.preferredWidth: 90; implicitHeight: 30; font.pixelSize: 12; text: modelData.name
                        onEditingFinished: if (text !== modelData.name) { const f = root.copy(custom.c.fields); f[index].name = text; root.writeCustom(custom.c, { fields: f }); } }
                    BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; text: String(modelData.value.value)
                        onEditingFinished: if (text !== String(modelData.value.value)) { const f = root.copy(custom.c.fields); f[index].value = root.parsedValue(text); root.writeCustom(custom.c, { fields: f }); } }
                    IconButton { iconName: "x"; tip: "Remove this field"; implicitWidth: 24; implicitHeight: 24
                        onClicked: { const f = root.copy(custom.c.fields); f.splice(index, 1); root.writeCustom(custom.c, { fields: f }); } }
                }
            }
            BwButton {
                iconName: "plus"; text: "Add field"; implicitHeight: 28; font.pixelSize: 12
                onClicked: {
                    const f = root.copy(custom.c.fields);
                    const taken = f.map(x => x.name);
                    let name = "value";
                    for (let n = 2; taken.indexOf(name) >= 0; ++n) name = "value " + n;
                    f.push({ name: name, value: { kind: "Number", value: 0 } });
                    root.writeCustom(custom.c, { fields: f });
                }
            }
        }
    }
    Component {
        id: materialCard
        ColumnLayout {
            id: mat
            readonly property var c: parent.c
            readonly property var m: root.materialOf(c)
            spacing: 6
            InspectorRow { label: "Metallic"; Layout.fillWidth: true; NumberField { value: mat.m.metallic; onCommitted: n => root.writeMaterial(mat.c, { metallic: n }) } }
            InspectorRow { label: "Rough"; Layout.fillWidth: true; NumberField { value: mat.m.roughness; fallback: 0.6; onCommitted: n => root.writeMaterial(mat.c, { roughness: n }) } }
            InspectorRow { label: "Glow"; Layout.fillWidth: true
                ColorField { value: mat.m.emissive; onPicked: col => root.writeMaterial(mat.c, { emissive: col }) }
                NumberField { value: mat.m.emissive_energy; onCommitted: n => root.writeMaterial(mat.c, { emissive_energy: n }) } }
            InspectorRow { label: "Texture"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["image"]; value: mat.m.albedo_texture; placeholderText: "Optional albedo"; onCommitted: p => root.writeMaterial(mat.c, { albedo_texture: p }) } }
            InspectorRow { label: "Two-sided"; Layout.fillWidth: true; SwitchField { value: mat.m.double_sided; onToggled: on => root.writeMaterial(mat.c, { double_sided: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Effect"; Layout.fillWidth: true
                SwitchField { value: mat.m.shader !== null; onToggled: on => root.writeMaterial(mat.c, { shader: on ? { mode: "Solid", speed: 1, strength: 0.5, color: "#FFFFFF" } : null }) } Item { Layout.fillWidth: true } }
            ColumnLayout {
                visible: mat.m.shader !== null; Layout.fillWidth: true; spacing: 6
                InspectorRow { label: "Motion"; Layout.fillWidth: true
                    ChoiceField { options: Blocks.opts(["Solid","Wave","Plasma","Pulse","Dissolve"]); value: mat.m.shader ? mat.m.shader.mode : "Solid"; onChosen: v => root.writeShader(mat.c, { mode: v }) } }
                InspectorRow { label: "Speed"; Layout.fillWidth: true; NumberField { value: mat.m.shader ? mat.m.shader.speed : 1; fallback: 1; onCommitted: n => root.writeShader(mat.c, { speed: n }) } }
                InspectorRow { label: "Strength"; Layout.fillWidth: true; NumberField { value: mat.m.shader ? mat.m.shader.strength : 0.5; fallback: 0.5; onCommitted: n => root.writeShader(mat.c, { strength: n }) } }
                InspectorRow { label: "Color"; Layout.fillWidth: true; ColorField { value: mat.m.shader ? mat.m.shader.color : "#FFFFFF"; onPicked: col => root.writeShader(mat.c, { color: col }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "WGSL"; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["shader"]; value: mat.m.shader && mat.m.shader.source ? mat.m.shader.source : ""; placeholderText: "Built-in motion"
                        onCommitted: p => root.writeShader(mat.c, { source: p.trim() }) } }
                RowLayout {
                    Layout.leftMargin: 84; spacing: 4
                    BwButton { text: "Export WGSL"; iconName: "file-code"; implicitHeight: 28; font.pixelSize: 12
                        enabled: !(mat.m.shader && mat.m.shader.source)
                        onClicked: root.app.invoke("export_shader", { actorId: root.actor.id }) }
                    BwButton { text: "Check"; implicitHeight: 28; font.pixelSize: 12
                        enabled: !!(mat.m.shader && mat.m.shader.source)
                        onClicked: root.app.invoke("check_shader", { actorId: root.actor.id }) }
                }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                    text: mat.m.shader && mat.m.shader.source
                        ? "The file's graph_main(uv, time) draws this surface; Motion is ignored. Speed, Strength and Color still reach it as params and secondary."
                        : "Export writes the motion out as a .wgsl file and draws with it, ready to edit by hand." }
            }
            Text { visible: !root.is3d && mat.m.shader === null; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Metallic, roughness and glow need 3D lighting; in 2D they rest until a custom effect is switched on." }
        }
    }
    Component {
        id: emitterCard
        ColumnLayout {
            id: em
            readonly property var c: parent.c
            readonly property var e: root.emitterOf(c)
            spacing: 6
            InspectorRow { label: "Rate /s"; Layout.fillWidth: true; NumberField { value: em.e.rate; fallback: 24; onCommitted: n => root.writeEmitter(em.c, { rate: n }) } }
            InspectorRow { label: "Life s"; Layout.fillWidth: true; NumberField { value: em.e.lifetime; fallback: 0.8; onCommitted: n => root.writeEmitter(em.c, { lifetime: n }) } }
            InspectorRow { label: "Speed"; Layout.fillWidth: true; NumberField { value: em.e.speed; fallback: 120; onCommitted: n => root.writeEmitter(em.c, { speed: n }) } }
            InspectorRow { label: "Spread"; Layout.fillWidth: true; NumberField { value: em.e.spread; fallback: 60; onCommitted: n => root.writeEmitter(em.c, { spread: n }) } }
            InspectorRow { label: "Gravity ×"; Layout.fillWidth: true; NumberField { value: em.e.gravity_scale; fallback: 0.5; onCommitted: n => root.writeEmitter(em.c, { gravity_scale: n }) } }
            InspectorRow { label: "Size"; Layout.fillWidth: true
                NumberField { value: em.e.size_start; fallback: 6; onCommitted: n => root.writeEmitter(em.c, { size_start: n }) }
                NumberField { value: em.e.size_end; fallback: 1; onCommitted: n => root.writeEmitter(em.c, { size_end: n }) } }
            InspectorRow { label: "Color"; Layout.fillWidth: true
                ColorField { value: em.e.color_start; onPicked: col => root.writeEmitter(em.c, { color_start: col }) }
                ColorField { value: em.e.color_end; onPicked: col => root.writeEmitter(em.c, { color_end: col }) }
                Item { Layout.fillWidth: true } }
            InspectorRow { label: "Max"; Layout.fillWidth: true; NumberField { value: em.e.max; fallback: 128; onCommitted: n => root.writeEmitter(em.c, { max: Math.round(n) }) } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Runs while attached - detaching the emitter stops the spray, and what is already flying fades out on its own." }
        }
    }
    Component {
        id: trailCard
        ColumnLayout {
            id: tr
            readonly property var c: parent.c
            readonly property var t: root.trailOf(c)
            spacing: 6
            InspectorRow { label: "Every s"; Layout.fillWidth: true; NumberField { value: tr.t.interval; fallback: 0.05; onCommitted: n => root.writeTrail(tr.c, { interval: n }) } }
            InspectorRow { label: "Lasts s"; Layout.fillWidth: true; NumberField { value: tr.t.life; fallback: 0.4; onCommitted: n => root.writeTrail(tr.c, { life: n }) } }
            InspectorRow { label: "Color"; Layout.fillWidth: true; ColorField { value: tr.t.color; onPicked: col => root.writeTrail(tr.c, { color: col }) } Item { Layout.fillWidth: true } }
        }
    }

    MouseArea {
        visible: root.open
        anchors.top: parent.top; anchors.bottom: parent.bottom; anchors.left: parent.left; width: 5
        cursorShape: Qt.SizeHorCursor
        property real startWidth: 0; property real startX: 0
        onPressed: mouse => { startWidth = root.width; startX = mapToItem(null, mouse.x, 0).x; }
        onPositionChanged: mouse => { if (pressed) root.resizeRequested(startWidth - (mapToItem(null, mouse.x, 0).x - startX)); }
    }
    IconButton {
        visible: !root.open
        anchors.horizontalCenter: parent.horizontalCenter; y: 10
        iconName: "chevron-left"; tip: "Show the components"
        onClicked: root.openRequested(true)
    }
    ScriptDialog { id: scriptDialog; app: root.app }
}
