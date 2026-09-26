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
        return Object.assign({ body: "None", gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5, density: 1, mass: null, trigger: false, one_way: false, character_controller: false, collision_layer: 1, collision_mask: 255 }, c.physics || {});
    }
    function cameraOf(c) { return Object.assign({ view: "Follow", offset: [0, 0.6, 0], distance: 6, pitch: 15, fov: 75 }, c.camera || {}); }
    function materialOf(c) { return Object.assign({ metallic: 0, roughness: 0.6, emissive: "#000000", emissive_energy: 0, albedo_texture: "", normal_texture: "", roughness_texture: "", tiling: [1, 1], offset: [0, 0], rotation: 0, sampler: "Clamp", anisotropy: 0, box_projection: false, texel_density: 1, double_sided: false, shader: null }, c.material || {}); }
    function emitterOf(c) { return Object.assign({ rate: 24, lifetime: 0.8, speed: 120, spread: 60, gravity_scale: 0.5, size_start: 6, size_end: 1, color_start: "#FFFFFF", color_end: "#FFAB19", max: 128, wind: 1 }, c.emitter || {}); }
    function lightOf(c) { return Object.assign({ kind: "Point", color: "#FFFFFF", intensity: 800, range: 20, radius: 0, inner_angle: 30, outer_angle: 45, shadows: false,
        unit: "Lumens", width: 1, height: 1, cookie: "", cookie_tiling: 1, ies: "", contact_shadows: false, soft_shadows: false, shadow_depth_bias: null, shadow_normal_bias: null, ray_traced: true, volumetric: true }, c.light || {}); }
    function beamOf(l) {
        const b = Object.assign({ density: 0, anisotropy: null, falloff: 1, near_fade: 0.5, far_fade: 2, mode: "Auto", shaft_intensity: 1, shaft_noise: 0.4, shaft_scroll: 0.3, motes: {} }, l.beam || {});
        b.motes = Object.assign({ enabled: false, count: 160, size: 0.012, alpha: 0.6, twinkle: 0.5, drift: 0.05 }, b.motes);
        return b;
    }
    function probeOf(c) { return Object.assign({ kind: "Reflection", size: [10, 5, 10], falloff: 0.2, resolution: 256, grid: [4, 3, 4], intensity: 1, box_projection: true, auto_bake: true }, c.probe || {}); }
    function terrainOf(c) { return Object.assign({ size: [256, 256], height: 40, resolution: 257, heights: "", splat: "", holes: "", layers: [newLayer(0)], pixel_error: 4, collision: true, texturing: {}, grass: [], scatter: [] }, c.terrain || {}); }
    function newLayer(n) {
        const looks = [["Grass", "#5E7D3A"], ["Rock", "#77716A"], ["Dirt", "#7A5C3E"], ["Snow", "#E8ECF0"]][n % 4];
        return { name: looks[0], color: looks[1], albedo_texture: "", normal_texture: "", roughness_texture: "", roughness: 0.85, texel_density: 0.25,
                 rules: { enabled: n > 0, slope: n === 1 ? [30, 90] : [0, 90], height: [-100000, 100000], curvature: 0, softness: 0.3 } };
    }
    function trailOf(c) { return Object.assign({ interval: 0.05, life: 0.4, color: "#FFFFFF" }, c.trail || {}); }
    function jointOf(c) { return Object.assign({ target: "", kind: "Fixed", anchor: [0, 0, 0], length: 2 }, c.joint || {}); }
    function animationOf(c) { return Object.assign({ clips: [], states: [] }, c.animation || {}); }
    function volumeOf(c) {
        const size = is3d ? 5 : 200;
        return Object.assign({ shape: "Box", half_extents: [size, size, size], radius: size, priority: 0, blend_distance: is3d ? 1 : 50, weight: 1, enabled: true, overrides: {} }, c.volume || {});
    }
    // What a volume property reads as before it is checked: the project's own.
    function projectValue(key) {
        const w = appState.project ? appState.project.world : {};
        const l = Object.assign({ light_direction: [8, 16, 8], light_color: "#FFFFFF", illuminance: 10000, ambient_color: "#FFFFFF", ambient_brightness: 80, ao_enabled: false }, w.lighting || {});
        const f = w.fog || { height: {}, volumetric: {}, aerial: {} };
        const p = Object.assign({ exposure_ev: 9.7, tonemapping: "TonyMcMapface", bloom_enabled: false, bloom_threshold: 1, bloom_intensity: 0.15, vignette_strength: 0 }, w.post || {});
        return ({ background: w.background || "#1B2431", sun_direction: l.light_direction, sun_color: l.light_color, illuminance: l.illuminance,
                  ambient_color: l.ambient_color, ambient_brightness: l.ambient_brightness, ao: l.ao_enabled, exposure: p.exposure_ev,
                  tonemapping: p.tonemapping, bloom: p.bloom_enabled, bloom_threshold: p.bloom_threshold, bloom_intensity: p.bloom_intensity,
                  vignette: p.vignette_strength, reflections: 1, indirect: 1,
                  sky_exposure: w.sky ? w.sky.exposure : 0, ambient_dimmer: w.sky ? w.sky.ambient_dimmer : 1,
                  cloud_coverage: (w.clouds || {}).coverage || 0, cloud_density: (w.clouds || {}).density || 0, cloud_type: (w.clouds || {}).cloud_type || 0,
                  fog_density: 3 / (f.height.distance || 400), fog_color: f.height.day_color || "#C2CAD2", fog_height: f.height.base_height || 0,
                  volumetric_density: f.volumetric.density !== undefined ? f.volumetric.density : 0.02, volumetric_albedo: f.volumetric.albedo || "#FFFFFF",
                  beams: 1, haze_distance: f.aerial.distance || 8000 })[key];
    }
    function overrideOf(v, key) { return Object.assign({ on: false, value: projectValue(key) }, (v.overrides || {})[key] || {}); }
    readonly property var volumeProperties: [
        { key: "background", label: "Background", kind: "color" },
        { key: "sun_direction", label: "Sun from", kind: "vec3", only3d: true },
        { key: "sun_color", label: "Sun color", kind: "color", only3d: true },
        { key: "illuminance", label: "Sun lux", kind: "number", only3d: true },
        { key: "ambient_color", label: "Ambient", kind: "color", only3d: true },
        { key: "ambient_brightness", label: "Ambient ×", kind: "number", only3d: true },
        { key: "ao", label: "AO", kind: "bool", only3d: true },
        { key: "exposure", label: "Exposure EV", kind: "number" },
        { key: "tonemapping", label: "Tonemap", kind: "tonemap" },
        { key: "bloom", label: "Bloom", kind: "bool" },
        { key: "bloom_threshold", label: "Bloom from", kind: "number" },
        { key: "bloom_intensity", label: "Bloom ×", kind: "number" },
        { key: "vignette", label: "Vignette", kind: "number" },
        { key: "reflections", label: "Reflections ×", kind: "number", only3d: true },
        { key: "indirect", label: "Indirect ×", kind: "number", only3d: true },
        { key: "sky_exposure", label: "Sky EV", kind: "number", only3d: true },
        { key: "ambient_dimmer", label: "Sky ambient ×", kind: "number", only3d: true },
        { key: "cloud_coverage", label: "Cloud coverage", kind: "number", only3d: true },
        { key: "cloud_density", label: "Cloud density", kind: "number", only3d: true },
        { key: "cloud_type", label: "Cloud type", kind: "number", only3d: true },
        { key: "fog_density", label: "Fog /m", kind: "number", only3d: true },
        { key: "fog_color", label: "Fog color", kind: "color", only3d: true },
        { key: "fog_height", label: "Fog base", kind: "number", only3d: true },
        { key: "volumetric_density", label: "Volumetric /m", kind: "number", only3d: true },
        { key: "volumetric_albedo", label: "Volumetric color", kind: "color", only3d: true },
        { key: "beams", label: "Beams ×", kind: "number", only3d: true },
        { key: "haze_distance", label: "Haze m", kind: "number", only3d: true }
    ]
    function brainOf(c) { return Object.assign({ target: "", speed: 4, sight: 12, fov: 120, separation: 1, tree: { node: "Selector", children: [{ node: "Sequence", children: [{ node: "CanSeeTarget" }, { node: "NavigateToTarget" }] }, { node: "Idle" }] } }, c.brain || {}); }
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
    function writeLight(c, next) { write("Light", { component: "Light", light: merged(lightOf(c), next) }); }
    function writeBeam(c, next) { writeLight(c, { beam: merged(beamOf(lightOf(c)), next) }); }
    function writeMotes(c, next) { writeBeam(c, { motes: merged(beamOf(lightOf(c)).motes, next) }); }
    function writeTrail(c, next) { write("Trail", { component: "Trail", trail: merged(trailOf(c), next) }); }
    function writeJoint(c, next) { write("Joint", { component: "Joint", joint: merged(jointOf(c), next) }); }
    function writeAnimation(c, next) { write("Animation", { component: "Animation", animation: merged(animationOf(c), next) }); }
    function writeProbe(c, next) { write("Probe", { component: "Probe", probe: merged(probeOf(c), next) }); }
    function writeTerrain(c, next) { write("Terrain", { component: "Terrain", terrain: merged(terrainOf(c), next) }); }
    function writeTerrainItem(c, list, index, next) {
        const l = copy(terrainOf(c)[list]);
        l[index] = Object.assign(l[index], next);
        const change = {}; change[list] = l;
        writeTerrain(c, change);
    }
    function writeVolume(c, next) { write("Volume", { component: "Volume", volume: merged(volumeOf(c), next) }); }
    function writeOverride(c, key, next) {
        const v = volumeOf(c);
        const overrides = copy(v.overrides || {});
        overrides[key] = Object.assign(overrideOf(v, key), next);
        writeVolume(c, { overrides: overrides });
    }
    function writeBrain(c, next) { write("Brain", { component: "Brain", brain: merged(brainOf(c), next) }); }
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
    readonly property var jointOptions: actor ? [{ value: "", label: "choose actor" }].concat(appState.project.actors.filter(a => a.id !== actor.id).map(a => ({ value: a.id, label: a.name }))) : []
    readonly property var addable: {
        if (!actor) return [];
        const held = actor.components.map(componentName);
        return ["Look","Render","Body","Joint","Brain","Camera","Script","Parent","Material","Emitter","Trail","Light","Animation","Volume","Probe","Terrain","Custom"]
            .filter(n => n === "Custom" || held.indexOf(n) < 0).map(n => ({ value: n, label: n === "Custom" ? "Custom…" : n }));
    }
    function blank(name) {
        switch (name) {
        case "Look": return { component: "Look", visual: is3d ? { shape: "Cuboid", color: "#4C97FF", size: [1, 1, 1] } : { shape: "Rect", color: "#4C97FF", size: [60, 60] } };
        case "Render": return { component: "Render", visible: true, layer: 0 };
        case "Body": return { component: "Body", physics: { body: "Dynamic", gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5, density: 1, mass: null, trigger: false, collision_layer: 1, collision_mask: 255 } };
        case "Joint": return { component: "Joint", joint: jointOf({}) };
        case "Brain": return { component: "Brain", brain: brainOf({}) };
        case "Camera": return { component: "Camera", camera: { view: "ThirdPerson", offset: [0, 0.6, 0], distance: 6, pitch: 15, fov: 75 } };
        case "Parent": return { component: "Parent", parent: "", offset: null };
        case "Material": return { component: "Material", material: materialOf({}) };
        case "Emitter": return { component: "Emitter", emitter: emitterOf({}) };
        case "Trail": return { component: "Trail", trail: trailOf({}) };
        case "Light": return { component: "Light", light: lightOf({}) };
        case "Animation": return { component: "Animation", animation: { clips: [], states: [] } };
        case "Volume": return { component: "Volume", volume: volumeOf({}) };
        case "Probe": return { component: "Probe", probe: probeOf({}) };
        case "Terrain": return { component: "Terrain", terrain: terrainOf({}) };
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
                            sourceComponent: ({ Place: placeCard, Look: lookCard, Parent: parentCard, Render: renderCard, Body: bodyCard, Joint: jointCard, Brain: brainCard, Camera: cameraCard,
                                                Script: scriptCard, Custom: customCard, Material: materialCard, Emitter: emitterCard, Trail: trailCard, Light: lightCard, Animation: animationCard, Volume: volumeCard, Probe: probeCard, Terrain: terrainCard })[card.c.component] || null
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
            InspectorRow { label: "Rotation deg"; Layout.fillWidth: true
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
                InspectorRow { label: "One-way"; visible: !root.is3d && body.p.body === "Static"; Layout.fillWidth: true; SwitchField { value: body.p.one_way; onToggled: on => root.writePhysics(body.c, { one_way: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Character control"; visible: body.p.body === "Kinematic"; Layout.fillWidth: true; SwitchField { value: body.p.character_controller; onToggled: on => root.writePhysics(body.c, { character_controller: on }) } Item { Layout.fillWidth: true } }
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
        id: jointCard
        ColumnLayout {
            id: joint
            readonly property var c: parent.c
            readonly property var j: root.jointOf(c)
            spacing: 6
            InspectorRow { label: "Connect to"; Layout.fillWidth: true
                ChoiceField { options: root.jointOptions; value: joint.j.target; onChosen: v => root.writeJoint(joint.c, { target: v }) } }
            InspectorRow { label: "Kind"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Fixed", label: "Fixed" }, { value: "Hinge", label: "Hinge" }, { value: "Rope", label: "Rope" }]; value: joint.j.kind; onChosen: v => root.writeJoint(joint.c, { kind: v }) } }
            InspectorRow { label: "Anchor X"; Layout.fillWidth: true
                NumberField { value: joint.j.anchor[0]; onCommitted: n => root.writeJoint(joint.c, { anchor: root.withIndex(joint.j.anchor, 0, n) }) } }
            InspectorRow { label: "Anchor Y"; Layout.fillWidth: true
                NumberField { value: joint.j.anchor[1]; onCommitted: n => root.writeJoint(joint.c, { anchor: root.withIndex(joint.j.anchor, 1, n) }) } }
            InspectorRow { label: "Anchor Z"; visible: root.is3d; Layout.fillWidth: true
                NumberField { value: joint.j.anchor[2]; onCommitted: n => root.writeJoint(joint.c, { anchor: root.withIndex(joint.j.anchor, 2, n) }) } }
            InspectorRow { label: "Rope length"; visible: joint.j.kind === "Rope"; Layout.fillWidth: true
                NumberField { value: joint.j.length; fallback: 2; onCommitted: n => root.writeJoint(joint.c, { length: n }) } }
        }
    }
    Component {
        id: brainCard
        ColumnLayout {
            id: brain
            readonly property var c: parent.c
            readonly property var b: root.brainOf(c)
            spacing: 6
            InspectorRow { label: "Target"; Layout.fillWidth: true
                ChoiceField { options: root.jointOptions; value: brain.b.target; onChosen: v => root.writeBrain(brain.c, { target: v }) } }
            InspectorRow { label: "Speed"; Layout.fillWidth: true
                NumberField { value: brain.b.speed; fallback: 4; onCommitted: n => root.writeBrain(brain.c, { speed: n }) } }
            InspectorRow { label: "Sight"; Layout.fillWidth: true
                NumberField { value: brain.b.sight; fallback: 12; onCommitted: n => root.writeBrain(brain.c, { sight: n }) } }
            InspectorRow { label: "View angle"; Layout.fillWidth: true
                NumberField { value: brain.b.fov; fallback: 120; onCommitted: n => root.writeBrain(brain.c, { fov: n }) } }
            InspectorRow { label: "Separation"; Layout.fillWidth: true
                NumberField { value: brain.b.separation; fallback: 1; onCommitted: n => root.writeBrain(brain.c, { separation: n }) } }
            InspectorRow { label: "Nav layer mask"; Layout.fillWidth: true
                NumberField { value: brain.b.layer === undefined ? 1 : brain.b.layer; fallback: 1; onCommitted: n => root.writeBrain(brain.c, { layer: Math.max(0, Math.round(n)) }) } }
            TextField {
                Layout.fillWidth: true; font.pixelSize: 11
                text: JSON.stringify(brain.b.tree)
                placeholderText: "Behavior tree JSON"
                onEditingFinished: {
                    try { const tree = JSON.parse(text); if (tree && tree.node) root.writeBrain(brain.c, { tree: tree }); }
                    catch (e) { text = JSON.stringify(brain.b.tree); }
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
            InspectorRow { label: "Normal map"; Layout.fillWidth: true; visible: root.is3d && mat.m.shader === null
                AssetField { app: root.app; accept: ["image"]; value: mat.m.normal_texture; placeholderText: "Optional normal"; onCommitted: p => root.writeMaterial(mat.c, { normal_texture: p }) } }
            InspectorRow { label: "Rough map"; Layout.fillWidth: true; visible: root.is3d && mat.m.shader === null
                AssetField { app: root.app; accept: ["image"]; value: mat.m.roughness_texture; placeholderText: "Roughness in green channel"; onCommitted: p => root.writeMaterial(mat.c, { roughness_texture: p }) } }
            InspectorRow { label: "Tile X / Y"; Layout.fillWidth: true
                NumberField { value: mat.m.tiling[0]; fallback: 1; onCommitted: n => root.writeMaterial(mat.c, { tiling: root.withIndex(mat.m.tiling, 0, n) }) }
                NumberField { value: mat.m.tiling[1]; fallback: 1; onCommitted: n => root.writeMaterial(mat.c, { tiling: root.withIndex(mat.m.tiling, 1, n) }) } }
            InspectorRow { label: "Offset X / Y"; Layout.fillWidth: true
                NumberField { value: mat.m.offset[0]; onCommitted: n => root.writeMaterial(mat.c, { offset: root.withIndex(mat.m.offset, 0, n) }) }
                NumberField { value: mat.m.offset[1]; onCommitted: n => root.writeMaterial(mat.c, { offset: root.withIndex(mat.m.offset, 1, n) }) } }
            InspectorRow { label: "Rotation"; Layout.fillWidth: true
                NumberField { value: mat.m.rotation; onCommitted: n => root.writeMaterial(mat.c, { rotation: n }) } }
            InspectorRow { label: "Sampler"; Layout.fillWidth: true
                ChoiceField { options: Blocks.opts(["Clamp", "Repeat", "Mirror"]); value: mat.m.sampler; onChosen: v => root.writeMaterial(mat.c, { sampler: v }) } }
            InspectorRow { label: "Anisotropy"; Layout.fillWidth: true
                ChoiceField { options: Blocks.opts(["0", "2", "4", "8", "16"]); value: String(mat.m.anisotropy); onChosen: v => root.writeMaterial(mat.c, { anisotropy: Number(v) }) } }
            InspectorRow { label: "Box projection"; Layout.fillWidth: true; visible: root.is3d
                SwitchField { value: mat.m.box_projection; onToggled: on => root.writeMaterial(mat.c, { box_projection: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Tiles / unit"; Layout.fillWidth: true
                NumberField { value: mat.m.texel_density; fallback: 1; onCommitted: n => root.writeMaterial(mat.c, { texel_density: n }) } }
            SurfaceDetailRows { Layout.fillWidth: true; visible: root.is3d && mat.m.shader === null; app: root.app; detail: mat.m.detail || {}
                onEdited: d => root.writeMaterial(mat.c, { detail: d }) }
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
                InspectorRow { label: "WESL"; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["shader"]; value: mat.m.shader && mat.m.shader.source ? mat.m.shader.source : ""; placeholderText: "Built-in motion"
                        onCommitted: p => root.writeShader(mat.c, { source: p.trim() }) } }
                RowLayout {
                    Layout.leftMargin: 84; spacing: 4
                    BwButton { text: "Export WESL"; iconName: "file-code"; implicitHeight: 28; font.pixelSize: 12
                        enabled: !(mat.m.shader && mat.m.shader.source)
                        onClicked: root.app.invoke("export_shader", { actorId: root.actor.id }) }
                    BwButton { text: "Check"; implicitHeight: 28; font.pixelSize: 12
                        enabled: !!(mat.m.shader && mat.m.shader.source)
                        onClicked: root.app.invoke("check_shader", { actorId: root.actor.id }) }
                }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                    text: mat.m.shader && mat.m.shader.source
                        ? "The file's graph_main(uv, time) draws this surface; Motion is ignored. Speed, Strength and Color still reach it as params and secondary."
                        : "Export writes the motion out as a .wesl file and draws with it, ready to edit by hand." }
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
            InspectorRow { label: "Wind ×"; Layout.fillWidth: true; NumberField { value: em.e.wind; fallback: 1; onCommitted: n => root.writeEmitter(em.c, { wind: Math.min(4, Math.max(0, n)) }) } }
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
        id: lightCard
        ColumnLayout {
            id: li
            readonly property var c: parent.c
            readonly property var l: root.lightOf(c)
            spacing: 6
            readonly property bool area: l.kind === "Rect" || l.kind === "Disk"
            InspectorRow { label: "Kind"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Point", label: "Point" }, { value: "Spot", label: "Spot" }, { value: "Rect", label: "Rect" }, { value: "Disk", label: "Disk" }]; value: li.l.kind; onChosen: k => root.writeLight(li.c, { kind: k }) } }
            InspectorRow { label: "Color"; Layout.fillWidth: true; ColorField { value: li.l.color; onPicked: col => root.writeLight(li.c, { color: col }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Intensity"; Layout.fillWidth: true
                NumberField { value: li.l.intensity; fallback: 800; onCommitted: n => root.writeLight(li.c, { intensity: Math.max(0, n) }) }
                ChoiceField { options: [{ value: "Lumens", label: "lm" }, { value: "Candela", label: "cd" }]; value: li.l.unit; onChosen: u => root.writeLight(li.c, { unit: u }) } }
            InspectorRow { label: "Range m"; Layout.fillWidth: true
                NumberField { value: li.l.range; fallback: 20; onCommitted: n => root.writeLight(li.c, { range: Math.max(0.01, n) }) } }
            InspectorRow { visible: !li.area; label: "Radius m"; Layout.fillWidth: true
                NumberField { value: li.l.radius; fallback: 0; onCommitted: n => root.writeLight(li.c, { radius: Math.max(0, n) }) } }
            InspectorRow { visible: li.area; label: li.l.kind === "Disk" ? "Diameter m" : "Size m"; Layout.fillWidth: true
                NumberField { value: li.l.width; fallback: 1; onCommitted: n => root.writeLight(li.c, { width: Math.max(0.01, n) }) }
                NumberField { visible: li.l.kind === "Rect"; value: li.l.height; fallback: 1; onCommitted: n => root.writeLight(li.c, { height: Math.max(0.01, n) }) } }
            InspectorRow { visible: li.l.kind === "Spot"; label: "Cone °"; Layout.fillWidth: true
                NumberField { value: li.l.inner_angle; fallback: 30; onCommitted: n => root.writeLight(li.c, { inner_angle: n }) }
                NumberField { value: li.l.outer_angle; fallback: 45; onCommitted: n => root.writeLight(li.c, { outer_angle: n }) } }
            InspectorRow { visible: li.l.kind === "Spot"; label: "Cookie"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["image"]; value: li.l.cookie; placeholderText: "Drag an image here"; onCommitted: p => root.writeLight(li.c, { cookie: p }) } }
            InspectorRow { visible: li.l.kind === "Spot" && li.l.cookie !== ""; label: "Tiling"; Layout.fillWidth: true
                NumberField { value: li.l.cookie_tiling; fallback: 1; onCommitted: n => root.writeLight(li.c, { cookie_tiling: Math.max(0.01, n) }) } }
            InspectorRow { visible: !li.area; label: "IES profile"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["light"]; value: li.l.ies; placeholderText: "Drag an .ies file here"; onCommitted: p => root.writeLight(li.c, { ies: p }) } }
            InspectorRow { visible: !li.area; label: "Shadows"; Layout.fillWidth: true
                SwitchField { value: li.l.shadows; onToggled: on => root.writeLight(li.c, { shadows: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: !li.area && li.l.shadows; label: "Soft (PCSS)"; Layout.fillWidth: true
                SwitchField { value: li.l.soft_shadows; onToggled: on => root.writeLight(li.c, { soft_shadows: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: !li.area && li.l.shadows; label: "Bias"; Layout.fillWidth: true
                NumberField { value: li.l.shadow_depth_bias !== null ? li.l.shadow_depth_bias : (li.l.kind === "Spot" ? 0.02 : 0.08); fallback: 0.02; onCommitted: n => root.writeLight(li.c, { shadow_depth_bias: Math.max(0, n) }) }
                NumberField { value: li.l.shadow_normal_bias !== null ? li.l.shadow_normal_bias : (li.l.kind === "Spot" ? 1.8 : 0.6); fallback: 0.6; onCommitted: n => root.writeLight(li.c, { shadow_normal_bias: Math.max(0, n) }) } }
            InspectorRow { visible: !li.area; label: "Contact"; Layout.fillWidth: true
                SwitchField { value: li.l.contact_shadows; onToggled: on => root.writeLight(li.c, { contact_shadows: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Traced"; Layout.fillWidth: true
                SwitchField { value: li.l.ray_traced; onToggled: on => root.writeLight(li.c, { ray_traced: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Lights fog"; Layout.fillWidth: true
                SwitchField { value: li.l.volumetric; onToggled: on => root.writeLight(li.c, { volumetric: on }) } Item { Layout.fillWidth: true } }
            readonly property var b: root.beamOf(l)
            readonly property bool beams: !area && l.volumetric
            readonly property bool beamOn: beams && b.density > 0
            InspectorRow { visible: li.beams; label: "Beam /m"; Layout.fillWidth: true
                NumberField { value: li.b.density; fallback: 0; onCommitted: n => root.writeBeam(li.c, { density: Math.min(Math.max(n, 0), 10) }) } }
            InspectorRow { visible: li.beamOn; label: "Drawn as"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Auto", label: "Auto" }, { value: "Volumetric", label: "Volumetric fog" }, { value: "Shaft", label: "Shaft cone" }]; value: li.b.mode; onChosen: m => root.writeBeam(li.c, { mode: m }) } }
            InspectorRow { visible: li.beamOn; label: "Own g"; Layout.fillWidth: true
                SwitchField { value: li.b.anisotropy !== null; onToggled: on => root.writeBeam(li.c, { anisotropy: on ? 0.6 : null }) }
                NumberField { visible: li.b.anisotropy !== null; value: li.b.anisotropy !== null ? li.b.anisotropy : 0.6; fallback: 0.6; onCommitted: n => root.writeBeam(li.c, { anisotropy: Math.min(Math.max(n, -0.9), 0.9) }) } }
            InspectorRow { visible: li.beamOn; label: "Falloff curve"; Layout.fillWidth: true
                NumberField { value: li.b.falloff; fallback: 1; onCommitted: n => root.writeBeam(li.c, { falloff: Math.min(Math.max(n, 0), 8) }) } }
            InspectorRow { visible: li.beamOn; label: "Fade near, far m"; Layout.fillWidth: true
                NumberField { value: li.b.near_fade; fallback: 0.5; onCommitted: n => root.writeBeam(li.c, { near_fade: Math.max(n, 0) }) }
                NumberField { value: li.b.far_fade; fallback: 2; onCommitted: n => root.writeBeam(li.c, { far_fade: Math.max(n, 0) }) } }
            InspectorRow { visible: li.beamOn && li.l.kind === "Spot" && li.b.mode !== "Volumetric"; label: "Shaft ×, noise"; Layout.fillWidth: true
                NumberField { value: li.b.shaft_intensity; fallback: 1; onCommitted: n => root.writeBeam(li.c, { shaft_intensity: Math.max(n, 0) }) }
                NumberField { value: li.b.shaft_noise; fallback: 0.4; onCommitted: n => root.writeBeam(li.c, { shaft_noise: Math.min(Math.max(n, 0), 1) }) } }
            InspectorRow { visible: li.beamOn && li.l.kind === "Spot" && li.b.mode !== "Volumetric"; label: "Scroll m/s"; Layout.fillWidth: true
                NumberField { value: li.b.shaft_scroll; fallback: 0.3; onCommitted: n => root.writeBeam(li.c, { shaft_scroll: n }) } }
            InspectorRow { visible: !li.area; label: "Dust motes"; Layout.fillWidth: true
                SwitchField { value: li.b.motes.enabled; onToggled: on => root.writeMotes(li.c, { enabled: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: !li.area && li.b.motes.enabled; label: "Count, size m"; Layout.fillWidth: true
                NumberField { value: li.b.motes.count; fallback: 160; onCommitted: n => root.writeMotes(li.c, { count: Math.min(Math.max(Math.round(n), 0), 4096) }) }
                NumberField { value: li.b.motes.size; fallback: 0.012; onCommitted: n => root.writeMotes(li.c, { size: Math.min(Math.max(n, 0.001), 1) }) } }
            InspectorRow { visible: !li.area && li.b.motes.enabled; label: "Alpha, twinkle"; Layout.fillWidth: true
                NumberField { value: li.b.motes.alpha; fallback: 0.6; onCommitted: n => root.writeMotes(li.c, { alpha: Math.min(Math.max(n, 0), 1) }) }
                NumberField { value: li.b.motes.twinkle; fallback: 0.5; onCommitted: n => root.writeMotes(li.c, { twinkle: Math.min(Math.max(n, 0), 1) }) } }
            InspectorRow { visible: !li.area && li.b.motes.enabled; label: "Drift m/s"; Layout.fillWidth: true
                NumberField { value: li.b.motes.drift; fallback: 0.05; onCommitted: n => root.writeMotes(li.c, { drift: Math.min(Math.max(n, 0), 10) }) } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: !root.is3d ? "Lights need a 3D world; in 2D this rests."
                    : li.area ? "An area light glows from a " + (li.l.kind === "Disk" ? "disc" : "rectangle") + " facing the actor's forward axis, with soft LTC highlights. It casts no shadow maps."
                    : li.l.unit === "Candela"
                    ? "Candela down the brightest direction (an IES profile's peak). About " + Math.round(li.l.intensity * 4 * Math.PI) + " lumens. Contact shadows also need them on in Project Settings."
                    : "About " + Math.round(li.l.intensity / (4 * Math.PI)) + " candela. A spot's cone doesn't gather the light, so narrowing it isn't brighter. Contact shadows also need them on in Project Settings." }
            Text { visible: li.beams && root.is3d; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "A beam is extra haze only this light scatters, thickening and thinning with `set fog density to`. Auto draws it in volumetric fog while that is on above Low quality, and as a cheap shaft cone otherwise; a point light's beam is a glow round it and needs volumetric fog. Motes drift in the light's reach." }
        }
    }
    Component {
        id: terrainCard
        ColumnLayout {
            id: ter
            readonly property var c: parent.c
            readonly property var t: root.terrainOf(c)
            property string erosionKind: "hydraulic"
            property real talus: 35
            property int iterations: 20
            property real erodeStrength: 0.3
            property bool previewing: false
            function erosion() {
                return erosionKind === "thermal" ? { kind: "thermal", iterations: iterations, talus: talus }
                                                 : { kind: "hydraulic", droplets: 0, seed: 7, erosion: erodeStrength, deposition: 0.3, inertia: 0.05 };
            }
            function preview() { previewing = true; root.app.invoke("preview_terrain_erosion", { actorId: root.actor.id, erosion: erosion() }); }
            spacing: 6
            InspectorRow { label: "Size X / Z"; Layout.fillWidth: true
                NumberField { value: ter.t.size[0]; fallback: 256; onCommitted: n => root.writeTerrain(ter.c, { size: root.withIndex(ter.t.size, 0, n) }) }
                NumberField { value: ter.t.size[1]; fallback: 256; onCommitted: n => root.writeTerrain(ter.c, { size: root.withIndex(ter.t.size, 1, n) }) } }
            InspectorRow { label: "Height"; Layout.fillWidth: true
                NumberField { value: ter.t.height; fallback: 40; onCommitted: n => root.writeTerrain(ter.c, { height: n }) } }
            InspectorRow { label: "Resolution"; Layout.fillWidth: true
                ChoiceField { options: Blocks.opts(["129", "257", "513", "1025", "2049", "4097"]); value: String(ter.t.resolution); onChosen: v => root.writeTerrain(ter.c, { resolution: Number(v) }) } }
            InspectorRow { label: "Pixel error"; Layout.fillWidth: true
                NumberField { value: ter.t.pixel_error; fallback: 4; onCommitted: n => root.writeTerrain(ter.c, { pixel_error: n }) } }
            InspectorRow { label: "Collision"; Layout.fillWidth: true
                SwitchField { value: ter.t.collision; onToggled: on => root.writeTerrain(ter.c, { collision: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Heightmap"; Layout.fillWidth: true
                AssetField { app: root.app; value: ""; placeholderText: "Import PNG, .r16 or .r32"
                    onCommitted: p => { if (p.trim() !== "") root.app.invoke("import_terrain_heightmap", { actorId: root.actor.id, path: p.trim() }); } } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Sculpt and paint with the Brush tool in the scene view. A heightmap is resampled to the resolution and replaces the heights." }

            Text { text: "Erosion"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { label: "Kind"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "hydraulic", label: "Hydraulic" }, { value: "thermal", label: "Thermal" }]; value: ter.erosionKind
                    onChosen: v => { ter.erosionKind = v; if (ter.previewing) ter.preview(); } } }
            InspectorRow { label: "Strength"; Layout.fillWidth: true; visible: ter.erosionKind === "hydraulic"
                NumberField { value: ter.erodeStrength; fallback: 0.3; onCommitted: n => { ter.erodeStrength = Math.min(1, Math.max(0, n)); if (ter.previewing) ter.preview(); } } }
            InspectorRow { label: "Passes / talus"; Layout.fillWidth: true; visible: ter.erosionKind === "thermal"
                NumberField { value: ter.iterations; fallback: 20; onCommitted: n => { ter.iterations = Math.max(1, Math.round(n)); if (ter.previewing) ter.preview(); } }
                NumberField { value: ter.talus; fallback: 35; onCommitted: n => { ter.talus = n; if (ter.previewing) ter.preview(); } } }
            RowLayout {
                Layout.leftMargin: 84; spacing: 4
                BwButton { text: ter.previewing ? "Clear preview" : "Preview"; implicitHeight: 28; font.pixelSize: 12
                    onClicked: {
                        if (ter.previewing) { ter.previewing = false; root.app.invoke("preview_terrain_erosion", { actorId: root.actor.id, erosion: null }); }
                        else ter.preview();
                    } }
                BwButton { text: "Apply"; implicitHeight: 28; font.pixelSize: 12
                    onClicked: { ter.previewing = false; root.app.invoke("erode_terrain", { actorId: root.actor.id, erosion: ter.erosion() }); } }
            }

            Text { text: "Paint layers"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            Repeater {
                model: ter.t.layers
                delegate: ColumnLayout {
                    required property int index
                    required property var modelData
                    readonly property var r: modelData.rules || {}
                    Layout.fillWidth: true; spacing: 4
                    RowLayout {
                        Layout.fillWidth: true; spacing: 4
                        BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; text: modelData.name; placeholderText: "Layer"
                            onEditingFinished: root.writeTerrainItem(ter.c, "layers", index, { name: text.trim() }) }
                        ColorField { value: modelData.color; onPicked: col => root.writeTerrainItem(ter.c, "layers", index, { color: col }) }
                        IconButton { visible: ter.t.layers.length > 1; iconName: "x"; tip: "Remove this layer"; implicitWidth: 24; implicitHeight: 24
                            onClicked: { const l = root.copy(ter.t.layers); l.splice(index, 1); root.writeTerrain(ter.c, { layers: l }); } }
                    }
                    InspectorRow { label: "Albedo"; Layout.fillWidth: true
                        AssetField { app: root.app; accept: ["image"]; value: modelData.albedo_texture; placeholderText: "Flat color"; onCommitted: p => root.writeTerrainItem(ter.c, "layers", index, { albedo_texture: p }) } }
                    InspectorRow { label: "Normal"; Layout.fillWidth: true
                        AssetField { app: root.app; accept: ["image"]; value: modelData.normal_texture; placeholderText: "Optional"; onCommitted: p => root.writeTerrainItem(ter.c, "layers", index, { normal_texture: p }) } }
                    InspectorRow { label: "Tiles / m, rough"; Layout.fillWidth: true
                        NumberField { value: modelData.texel_density; fallback: 0.25; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { texel_density: n }) }
                        NumberField { value: modelData.roughness; fallback: 0.85; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { roughness: n }) } }
                    InspectorRow { label: "Rules"; Layout.fillWidth: true
                        SwitchField { value: !!r.enabled; onToggled: on => root.writeTerrainItem(ter.c, "layers", index, { rules: Object.assign(root.copy(r), { enabled: on }) }) } Item { Layout.fillWidth: true } }
                    InspectorRow { label: "Slope from / to"; Layout.fillWidth: true; visible: !!r.enabled
                        NumberField { value: r.slope[0]; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { rules: Object.assign(root.copy(r), { slope: root.withIndex(r.slope, 0, n) }) }) }
                        NumberField { value: r.slope[1]; fallback: 90; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { rules: Object.assign(root.copy(r), { slope: root.withIndex(r.slope, 1, n) }) }) } }
                    InspectorRow { label: "Height from / to"; Layout.fillWidth: true; visible: !!r.enabled
                        NumberField { value: r.height[0]; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { rules: Object.assign(root.copy(r), { height: root.withIndex(r.height, 0, n) }) }) }
                        NumberField { value: r.height[1]; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { rules: Object.assign(root.copy(r), { height: root.withIndex(r.height, 1, n) }) }) } }
                    InspectorRow { label: "Curve / soft"; Layout.fillWidth: true; visible: !!r.enabled
                        NumberField { value: r.curvature; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { rules: Object.assign(root.copy(r), { curvature: n }) }) }
                        NumberField { value: r.softness; fallback: 0.3; onCommitted: n => root.writeTerrainItem(ter.c, "layers", index, { rules: Object.assign(root.copy(r), { softness: n }) }) } }
                }
            }
            BwButton { visible: ter.t.layers.length < 4; iconName: "plus"; text: "Add layer"; implicitHeight: 28; font.pixelSize: 12
                onClicked: { const l = root.copy(ter.t.layers); l.push(root.newLayer(l.length)); root.writeTerrain(ter.c, { layers: l }); } }

            Text { text: "Grass"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            Repeater {
                model: ter.t.grass
                delegate: ColumnLayout {
                    required property int index
                    required property var modelData
                    Layout.fillWidth: true; spacing: 4
                    RowLayout {
                        Layout.fillWidth: true; spacing: 4
                        BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; text: modelData.name; placeholderText: "Grass"
                            onEditingFinished: root.writeTerrainItem(ter.c, "grass", index, { name: text.trim() }) }
                        IconButton { iconName: "x"; tip: "Remove this grass"; implicitWidth: 24; implicitHeight: 24
                            onClicked: { const l = root.copy(ter.t.grass); l.splice(index, 1); root.writeTerrain(ter.c, { grass: l }); } }
                    }
                    InspectorRow { label: "On layer"; Layout.fillWidth: true
                        ChoiceField { options: [{ value: -1, label: "Everywhere" }].concat(ter.t.layers.map((l, i) => ({ value: i, label: l.name }))); value: modelData.layer
                            onChosen: v => root.writeTerrainItem(ter.c, "grass", index, { layer: v }) } }
                    InspectorRow { label: "Blades / m²"; Layout.fillWidth: true
                        NumberField { value: modelData.density; fallback: 16; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { density: n }) } }
                    InspectorRow { label: "Height min / max"; Layout.fillWidth: true
                        NumberField { value: modelData.height[0]; fallback: 0.25; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { height: root.withIndex(modelData.height, 0, n) }) }
                        NumberField { value: modelData.height[1]; fallback: 0.6; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { height: root.withIndex(modelData.height, 1, n) }) } }
                    InspectorRow { label: "Base / tip"; Layout.fillWidth: true
                        ColorField { value: modelData.base_color; onPicked: col => root.writeTerrainItem(ter.c, "grass", index, { base_color: col }) }
                        ColorField { value: modelData.tip_color; onPicked: col => root.writeTerrainItem(ter.c, "grass", index, { tip_color: col }) } }
                    InspectorRow { label: "Variation"; Layout.fillWidth: true
                        NumberField { value: modelData.variation; fallback: 0.2; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { variation: n }) } }
                    InspectorRow { label: "Stiff / wind"; Layout.fillWidth: true
                        NumberField { value: modelData.stiffness; fallback: 0.5; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { stiffness: n }) }
                        NumberField { value: modelData.wind; fallback: 1; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { wind: n }) } }
                    InspectorRow { label: "Fade / cull"; Layout.fillWidth: true
                        NumberField { value: modelData.fade_start; fallback: 30; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { fade_start: n }) }
                        NumberField { value: modelData.cull_distance; fallback: 60; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { cull_distance: n }) } }
                    InspectorRow { label: "Max slope"; Layout.fillWidth: true
                        NumberField { value: modelData.max_slope; fallback: 40; onCommitted: n => root.writeTerrainItem(ter.c, "grass", index, { max_slope: n }) } }
                }
            }
            BwButton { iconName: "plus"; text: "Add grass"; implicitHeight: 28; font.pixelSize: 12
                onClicked: { const l = root.copy(ter.t.grass); l.push({ name: "Grass", layer: 0, density: 16, height: [0.25, 0.6], width: 0.05, base_color: "#2E4A1B", tip_color: "#93AE4F",
                    variation: 0.2, stiffness: 0.5, wind: 1, max_slope: 40, fade_start: 30, cull_distance: 60, density_map: "", seed: l.length + 1 }); root.writeTerrain(ter.c, { grass: l }); } }

            Text { text: "Trees and rocks"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            Repeater {
                model: ter.t.scatter
                delegate: ColumnLayout {
                    required property int index
                    required property var modelData
                    Layout.fillWidth: true; spacing: 4
                    RowLayout {
                        Layout.fillWidth: true; spacing: 4
                        BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; text: modelData.name; placeholderText: "Trees"
                            onEditingFinished: root.writeTerrainItem(ter.c, "scatter", index, { name: text.trim() }) }
                        IconButton { iconName: "x"; tip: "Remove this scatter layer"; implicitWidth: 24; implicitHeight: 24
                            onClicked: { const l = root.copy(ter.t.scatter); l.splice(index, 1); root.writeTerrain(ter.c, { scatter: l }); } }
                    }
                    InspectorRow { label: "Shape"; Layout.fillWidth: true
                        ChoiceField { options: Blocks.opts(["Tree", "Pine", "Bush", "Rock"]); value: modelData.shape; onChosen: v => root.writeTerrainItem(ter.c, "scatter", index, { shape: v }) }
                        ColorField { value: modelData.color; onPicked: col => root.writeTerrainItem(ter.c, "scatter", index, { color: col }) } }
                    InspectorRow { label: "Model"; Layout.fillWidth: true
                        AssetField { app: root.app; accept: ["model"]; value: modelData.model; placeholderText: "Procedural shape"; onCommitted: p => root.writeTerrainItem(ter.c, "scatter", index, { model: p }) } }
                    InspectorRow { label: "Model LOD1"; Layout.fillWidth: true; visible: modelData.model !== ""
                        AssetField { app: root.app; accept: ["model"]; value: modelData.model_lod1; placeholderText: "Keeps the model"; onCommitted: p => root.writeTerrainItem(ter.c, "scatter", index, { model_lod1: p }) } }
                    InspectorRow { label: "Billboard"; Layout.fillWidth: true
                        AssetField { app: root.app; accept: ["image"]; value: modelData.billboard; placeholderText: "No billboard level"; onCommitted: p => root.writeTerrainItem(ter.c, "scatter", index, { billboard: p }) } }
                    InspectorRow { label: "On layer"; Layout.fillWidth: true
                        ChoiceField { options: [{ value: -1, label: "Everywhere" }].concat(ter.t.layers.map((l, i) => ({ value: i, label: l.name }))); value: modelData.layer
                            onChosen: v => root.writeTerrainItem(ter.c, "scatter", index, { layer: v }) } }
                    InspectorRow { label: "Per 100 m², gap"; Layout.fillWidth: true
                        NumberField { value: modelData.density; fallback: 0.5; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { density: n }) }
                        NumberField { value: modelData.spacing; fallback: 3; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { spacing: n }) } }
                    InspectorRow { label: "Clump / size"; Layout.fillWidth: true
                        NumberField { value: modelData.clumping; fallback: 0.4; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { clumping: n }) }
                        NumberField { value: modelData.clump_size; fallback: 24; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { clump_size: n }) } }
                    InspectorRow { label: "Slope from / to"; Layout.fillWidth: true
                        NumberField { value: modelData.slope[0]; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { slope: root.withIndex(modelData.slope, 0, n) }) }
                        NumberField { value: modelData.slope[1]; fallback: 30; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { slope: root.withIndex(modelData.slope, 1, n) }) } }
                    InspectorRow { label: "Altitude"; Layout.fillWidth: true
                        NumberField { value: modelData.altitude[0]; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { altitude: root.withIndex(modelData.altitude, 0, n) }) }
                        NumberField { value: modelData.altitude[1]; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { altitude: root.withIndex(modelData.altitude, 1, n) }) } }
                    InspectorRow { label: "Scale min / max"; Layout.fillWidth: true
                        NumberField { value: modelData.scale[0]; fallback: 0.8; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { scale: root.withIndex(modelData.scale, 0, n) }) }
                        NumberField { value: modelData.scale[1]; fallback: 1.3; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { scale: root.withIndex(modelData.scale, 1, n) }) } }
                    InspectorRow { label: "Tint / align"; Layout.fillWidth: true
                        NumberField { value: modelData.tint; fallback: 0.12; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { tint: n }) }
                        NumberField { value: modelData.align; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { align: n }) } }
                    InspectorRow { label: "LOD1 / cull"; Layout.fillWidth: true
                        NumberField { value: modelData.lod1_distance; fallback: 60; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { lod1_distance: n }) }
                        NumberField { value: modelData.cull_distance; fallback: 450; onCommitted: n => root.writeTerrainItem(ter.c, "scatter", index, { cull_distance: n }) } }
                    InspectorRow { label: "Collide / avoid"; Layout.fillWidth: true
                        SwitchField { value: modelData.collide; onToggled: on => root.writeTerrainItem(ter.c, "scatter", index, { collide: on }) }
                        SwitchField { value: modelData.avoid_actors; onToggled: on => root.writeTerrainItem(ter.c, "scatter", index, { avoid_actors: on }) } }
                }
            }
            BwButton { iconName: "plus"; text: "Add trees"; implicitHeight: 28; font.pixelSize: 12
                onClicked: { const l = root.copy(ter.t.scatter); l.push({ name: "Trees", shape: "Tree", seed: l.length + 1 }); root.writeTerrain(ter.c, { scatter: l }); } }

            Text { text: "Texturing"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            SurfaceDetailRows { Layout.fillWidth: true; app: root.app; terrain: true; detail: ter.t.texturing || {}
                onEdited: d => root.writeTerrain(ter.c, { texturing: d }) }
        }
    }
    Component {
        id: probeCard
        ColumnLayout {
            id: pr
            readonly property var c: parent.c
            readonly property var p: root.probeOf(c)
            property var status: null
            spacing: 6
            function refresh() {
                if (!root.actor) return;
                const id = root.actor.id;
                root.app.invoke("probe_status", {}, list => { pr.status = (list || []).find(x => x.actor === id) || null; });
            }
            Component.onCompleted: refresh()
            Timer { interval: 2000; running: pr.visible && root.is3d; repeat: true; onTriggered: pr.refresh() }
            InspectorRow { label: "Kind"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Reflection", label: "Reflection" }, { value: "Irradiance", label: "Irradiance" }]; value: pr.p.kind; onChosen: k => root.writeProbe(pr.c, { kind: k }) } }
            InspectorRow { label: "Size m"; Layout.fillWidth: true
                NumberField { value: pr.p.size[0]; fallback: 10; onCommitted: n => root.writeProbe(pr.c, { size: root.withIndex(pr.p.size, 0, Math.max(0.01, n)) }) }
                NumberField { value: pr.p.size[1]; fallback: 5; onCommitted: n => root.writeProbe(pr.c, { size: root.withIndex(pr.p.size, 1, Math.max(0.01, n)) }) }
                NumberField { value: pr.p.size[2]; fallback: 10; onCommitted: n => root.writeProbe(pr.c, { size: root.withIndex(pr.p.size, 2, Math.max(0.01, n)) }) } }
            InspectorRow { label: "Falloff"; Layout.fillWidth: true
                NumberField { value: pr.p.falloff; fallback: 0.2; onCommitted: n => root.writeProbe(pr.c, { falloff: Math.min(1, Math.max(0, n)) }) } }
            InspectorRow { label: "Intensity"; Layout.fillWidth: true
                NumberField { value: pr.p.intensity; fallback: 1; onCommitted: n => root.writeProbe(pr.c, { intensity: Math.max(0, n) }) } }
            InspectorRow { visible: pr.p.kind === "Reflection"; label: "Resolution"; Layout.fillWidth: true
                ChoiceField { options: [64, 128, 256, 512, 1024].map(n => ({ value: String(n), label: n + " px" })); value: String(pr.p.resolution); onChosen: v => root.writeProbe(pr.c, { resolution: Number(v) }) } }
            InspectorRow { visible: pr.p.kind === "Reflection"; label: "Box projection"; Layout.fillWidth: true
                SwitchField { value: pr.p.box_projection; onToggled: on => root.writeProbe(pr.c, { box_projection: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: pr.p.kind === "Irradiance"; label: "Bricks"; Layout.fillWidth: true
                NumberField { value: pr.p.grid[0]; fallback: 4; onCommitted: n => root.writeProbe(pr.c, { grid: root.withIndex(pr.p.grid, 0, Math.min(16, Math.max(1, Math.round(n)))) }) }
                NumberField { value: pr.p.grid[1]; fallback: 3; onCommitted: n => root.writeProbe(pr.c, { grid: root.withIndex(pr.p.grid, 1, Math.min(16, Math.max(1, Math.round(n)))) }) }
                NumberField { value: pr.p.grid[2]; fallback: 4; onCommitted: n => root.writeProbe(pr.c, { grid: root.withIndex(pr.p.grid, 2, Math.min(16, Math.max(1, Math.round(n)))) }) } }
            InspectorRow { label: "Auto-bake"; Layout.fillWidth: true
                SwitchField { value: pr.p.auto_bake; onToggled: on => root.writeProbe(pr.c, { auto_bake: on }) } Item { Layout.fillWidth: true } }
            RowLayout {
                Layout.fillWidth: true; spacing: 8
                BwButton { text: "Bake"; iconName: "aperture"; implicitHeight: 30; enabled: root.is3d
                    onClicked: root.app.invoke("bake_probes", { actors: [root.actor.id] }, () => bakeCheck.restart()) }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: 11
                    color: pr.status && pr.status.dirty ? Theme.warning : Theme.textDim
                    text: !pr.status ? "" : !pr.status.baked ? "Not baked yet" : pr.status.dirty ? "Stale: the probe or the scene moved since the bake" : "Baked and up to date" }
                Timer { id: bakeCheck; interval: 1500; onTriggered: pr.refresh() }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: !root.is3d ? "Light probes need a 3D world; in 2D this rests."
                    : pr.p.kind === "Reflection"
                    ? "Captures a cubemap from the actor's position that surfaces inside the box reflect. The box turns with the actor but isn't scaled by it. Give the probe actor no Look, or it sees itself."
                    : "Captures a grid of ambient cubes that light whatever moves through the box with bounced light." }
        }
    }
    Component {
        id: volumeCard
        ColumnLayout {
            id: vo
            readonly property var c: parent.c
            readonly property var v: root.volumeOf(c)
            readonly property var fog: Object.assign({ enabled: false, density: 0.2, albedo: "#FFFFFF", emissive: "#000000", emissive_strength: 0 }, v.fog || {})
            readonly property var wind: Object.assign({ enabled: false, mode: "Override", direction: 0, speed: 0, swirl: 20, inflow: 2, updraft: 5, turbulence: 0, turbulence_scale: 10 }, v.wind || {})
            function writeWind(next) { root.writeVolume(vo.c, { wind: Object.assign({}, vo.wind, next) }); }
            readonly property var live: root.actor && root.app.status && root.app.status.volumes
                ? (root.app.status.volumes.find(x => x.actor === root.actor.id) || null) : null
            spacing: 6
            InspectorRow { label: "Enabled"; Layout.fillWidth: true
                SwitchField { value: vo.v.enabled; onToggled: on => root.writeVolume(vo.c, { enabled: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Shape"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Box", label: root.is3d ? "Box" : "Rectangle" }, { value: "Sphere", label: root.is3d ? "Sphere" : "Circle" }, { value: "Global", label: "Global" }]
                    value: vo.v.shape; onChosen: s => root.writeVolume(vo.c, { shape: s }) } }
            InspectorRow { visible: vo.v.shape === "Box"; label: "Half size"; Layout.fillWidth: true
                NumberField { value: vo.v.half_extents[0]; onCommitted: n => root.writeVolume(vo.c, { half_extents: root.withIndex(vo.v.half_extents, 0, Math.max(0, n)) }) }
                NumberField { value: vo.v.half_extents[1]; onCommitted: n => root.writeVolume(vo.c, { half_extents: root.withIndex(vo.v.half_extents, 1, Math.max(0, n)) }) }
                NumberField { visible: root.is3d; value: vo.v.half_extents[2]; onCommitted: n => root.writeVolume(vo.c, { half_extents: root.withIndex(vo.v.half_extents, 2, Math.max(0, n)) }) } }
            InspectorRow { visible: vo.v.shape === "Sphere"; label: "Radius"; Layout.fillWidth: true
                NumberField { value: vo.v.radius; onCommitted: n => root.writeVolume(vo.c, { radius: Math.max(0, n) }) } }
            InspectorRow { visible: vo.v.shape !== "Global"; label: "Blend dist"; Layout.fillWidth: true
                NumberField { value: vo.v.blend_distance; onCommitted: n => root.writeVolume(vo.c, { blend_distance: Math.max(0, n) }) } }
            InspectorRow { label: "Priority"; Layout.fillWidth: true
                NumberField { value: vo.v.priority; onCommitted: n => root.writeVolume(vo.c, { priority: n }) } }
            InspectorRow { label: "Weight"; Layout.fillWidth: true
                NumberField { value: vo.v.weight; fallback: 1; onCommitted: n => root.writeVolume(vo.c, { weight: Math.min(1, Math.max(0, n)) }) } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: vo.live
                    ? "Blending at " + Math.round(vo.live.weight * 100) + "%, covering " + Math.round(vo.live.coverage * 100) + "% of the camera."
                    : "Not over the camera right now. Higher priority blends last and wins." }
            Text { text: "Overrides"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            Repeater {
                model: root.volumeProperties.filter(p => root.is3d || !p.only3d)
                delegate: RowLayout {
                    id: prop
                    required property var modelData
                    readonly property var o: root.overrideOf(vo.v, modelData.key)
                    Layout.fillWidth: true; spacing: 4
                    BwCheckBox { Layout.preferredWidth: 112; text: prop.modelData.label; checked: prop.o.on
                        onToggled: root.writeOverride(vo.c, prop.modelData.key, { on: checked }) }
                    Loader {
                        Layout.fillWidth: true
                        opacity: prop.o.on ? 1 : 0.45
                        readonly property var o: prop.o
                        readonly property string key: prop.modelData.key
                        sourceComponent: ({ number: overrideNumber, color: overrideColor, bool: overrideBool, vec3: overrideVec3, tonemap: overrideTonemap })[prop.modelData.kind]
                    }
                }
            }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Checked properties blend over the project's settings; the rest are left to what lies under this volume." }
            Text { visible: root.is3d && vo.v.shape !== "Global"; text: "Local fog"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { visible: root.is3d && vo.v.shape !== "Global"; label: "Fills it"; Layout.fillWidth: true
                SwitchField { value: vo.fog.enabled; onToggled: on => root.writeVolume(vo.c, { fog: Object.assign({}, vo.fog, { enabled: on }) }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: root.is3d && vo.v.shape !== "Global" && vo.fog.enabled; label: "Density /m"; Layout.fillWidth: true
                NumberField { value: vo.fog.density; fallback: 0.2; onCommitted: n => root.writeVolume(vo.c, { fog: Object.assign({}, vo.fog, { density: Math.min(10, Math.max(0, n)) }) }) } }
            InspectorRow { visible: root.is3d && vo.v.shape !== "Global" && vo.fog.enabled; label: "Albedo, glow"; Layout.fillWidth: true
                ColorField { value: vo.fog.albedo; onPicked: col => root.writeVolume(vo.c, { fog: Object.assign({}, vo.fog, { albedo: col }) }) }
                ColorField { value: vo.fog.emissive; onPicked: col => root.writeVolume(vo.c, { fog: Object.assign({}, vo.fog, { emissive: col }) }) }
                NumberField { value: vo.fog.emissive_strength; fallback: 0; onCommitted: n => root.writeVolume(vo.c, { fog: Object.assign({}, vo.fog, { emissive_strength: Math.max(0, n) }) }) } }
            Text { visible: root.is3d && vo.v.shape !== "Global" && vo.fog.enabled; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Adds volumetric fog inside the shape, fading out across the blend distance and scaled by the weight, wherever the camera is. Lit like the project's volumetric fog; glow nits are per unit of density." }
            Text { text: "Local wind"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { label: "Enabled"; Layout.fillWidth: true
                SwitchField { value: vo.wind.enabled; onToggled: on => vo.writeWind({ enabled: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: vo.wind.enabled; label: "Mode"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Override", label: "Blow instead" }, { value: "Add", label: "Blow on top" }, { value: "Swirl", label: "Swirl" }]; value: vo.wind.mode; onChosen: m => vo.writeWind({ mode: m }) } }
            InspectorRow { visible: vo.wind.enabled && vo.wind.mode !== "Swirl"; label: "Direction, speed"; Layout.fillWidth: true
                NumberField { value: vo.wind.direction; fallback: 0; onCommitted: n => vo.writeWind({ direction: ((n % 360) + 360) % 360 }) }
                NumberField { value: vo.wind.speed; fallback: 0; onCommitted: n => vo.writeWind({ speed: Math.min(10000, Math.max(0, n)) }) } }
            InspectorRow { visible: vo.wind.enabled && vo.wind.mode === "Swirl"; label: root.is3d ? "Round, in, up" : "Round, in"; Layout.fillWidth: true
                NumberField { value: vo.wind.swirl; fallback: 20; onCommitted: n => vo.writeWind({ swirl: n }) }
                NumberField { value: vo.wind.inflow; fallback: 2; onCommitted: n => vo.writeWind({ inflow: n }) }
                NumberField { visible: root.is3d; value: vo.wind.updraft; fallback: 5; onCommitted: n => vo.writeWind({ updraft: n }) } }
            InspectorRow { visible: vo.wind.enabled; label: "Turbulence, size"; Layout.fillWidth: true
                NumberField { value: vo.wind.turbulence; fallback: 0; onCommitted: n => vo.writeWind({ turbulence: Math.min(10000, Math.max(0, n)) }) }
                NumberField { value: vo.wind.turbulence_scale; fallback: 10; onCommitted: n => vo.writeWind({ turbulence_scale: Math.max(0.01, n) }) } }
            Text { visible: vo.wind.enabled; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Changes the project's wind inside the shape, fading out across the blend distance and scaled by the weight; higher priority zones act last. Blow instead with speed 0 is still air indoors. Swirl spins round the actor's up axis" + (root.is3d ? ", pulls in and lifts: a funnel." : " and pulls in.") + " Turbulence stirs eddies about the size given, harder in a storm." }
            Component { id: overrideNumber
                NumberField { value: parent.o.value; onCommitted: n => root.writeOverride(vo.c, parent.key, { value: n, on: true }) } }
            Component { id: overrideColor
                RowLayout {
                    id: tint
                    readonly property var o: parent.o
                    readonly property string key: parent.key
                    ColorField { value: tint.o.value; onPicked: col => root.writeOverride(vo.c, tint.key, { value: col, on: true }) }
                    Item { Layout.fillWidth: true }
                } }
            Component { id: overrideBool
                RowLayout {
                    id: flip
                    readonly property var o: parent.o
                    readonly property string key: parent.key
                    SwitchField { value: flip.o.value; onToggled: on => root.writeOverride(vo.c, flip.key, { value: on, on: true }) }
                    Item { Layout.fillWidth: true }
                } }
            Component { id: overrideVec3
                RowLayout {
                    id: vec
                    readonly property var o: parent.o
                    readonly property string key: parent.key
                    spacing: 4
                    Repeater {
                        model: 3
                        NumberField { required property int index; Layout.fillWidth: true; value: vec.o.value[index]
                            onCommitted: n => root.writeOverride(vo.c, vec.key, { value: root.withIndex(vec.o.value, index, n), on: true }) }
                    }
                } }
            Component { id: overrideTonemap
                ChoiceField { options: Blocks.opts(["TonyMcMapface","None","Reinhard","ReinhardLuminance","AcesFitted","Filmic"]); value: parent.o.value
                    onChosen: t => root.writeOverride(vo.c, parent.key, { value: t, on: true }) } }
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
    Component {
        id: animationCard
        ColumnLayout {
            id: an
            readonly property var c: parent.c
            readonly property var a: root.animationOf(c)
            spacing: 6
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Flipbooks over image files. play animation changes state; when animation ends fires the transition for a Once clip." }
            Text { text: "Clips"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            Repeater {
                model: an.a.clips
                delegate: ColumnLayout {
                    required property int index
                    required property var modelData
                    Layout.fillWidth: true; spacing: 4
                    RowLayout {
                        Layout.fillWidth: true; spacing: 4
                        BwTextField { Layout.preferredWidth: 90; implicitHeight: 30; font.pixelSize: 12; text: modelData.name; placeholderText: "Clip"
                            onEditingFinished: { const l = root.copy(an.a.clips); l[index].name = text.trim(); root.writeAnimation(an.c, { clips: l }); } }
                        NumberField { Layout.preferredWidth: 56; value: modelData.fps; fallback: 8; onCommitted: n => { const l = root.copy(an.a.clips); l[index].fps = Math.min(60, Math.max(0.1, n)); root.writeAnimation(an.c, { clips: l }); } }
                        ChoiceField { Layout.preferredWidth: 96; options: Blocks.loopModeOptions; value: modelData.loop_mode || "Once"
                            onChosen: v => { const l = root.copy(an.a.clips); l[index].loop_mode = v; root.writeAnimation(an.c, { clips: l }); } }
                        IconButton { iconName: "x"; tip: "Remove this clip"; implicitWidth: 24; implicitHeight: 24
                            onClicked: { const l = root.copy(an.a.clips); l.splice(index, 1); root.writeAnimation(an.c, { clips: l }); } }
                    }
                    BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Frames, one asset path per line or comma";
                        text: (modelData.frames || []).join(", ")
                        onEditingFinished: { const l = root.copy(an.a.clips); l[index].frames = text.split(/[\n,]+/).map(s => s.trim()).filter(s => s !== ""); root.writeAnimation(an.c, { clips: l }); } }
                }
            }
            BwButton {
                iconName: "plus"; text: "Add clip"; implicitHeight: 28; font.pixelSize: 12
                onClicked: {
                    const l = root.copy(an.a.clips);
                    const taken = l.map(x => x.name);
                    let name = "walk";
                    for (let n = 2; taken.indexOf(name) >= 0; ++n) name = "walk " + n;
                    l.push({ name: name, frames: [], fps: 8, loop_mode: "Loop" });
                    root.writeAnimation(an.c, { clips: l });
                }
            }
            Text { text: "States"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            Repeater {
                model: an.a.states
                delegate: RowLayout {
                    required property int index
                    required property var modelData
                    Layout.fillWidth: true; spacing: 4
                    BwTextField { Layout.preferredWidth: 80; implicitHeight: 30; font.pixelSize: 12; text: modelData.name; placeholderText: "State"
                        onEditingFinished: { const l = root.copy(an.a.states); l[index].name = text.trim(); root.writeAnimation(an.c, { states: l }); } }
                    ChoiceField { Layout.fillWidth: true; options: [{ value: "", label: "clip…" }].concat(an.a.clips.map(x => ({ value: x.name, label: x.name }))); value: modelData.clip || ""
                        onChosen: v => { const l = root.copy(an.a.states); l[index].clip = v; root.writeAnimation(an.c, { states: l }); } }
                    NumberField { Layout.preferredWidth: 52; value: modelData.speed; fallback: 1; onCommitted: n => { const l = root.copy(an.a.states); l[index].speed = Math.min(8, Math.max(0, n)); root.writeAnimation(an.c, { states: l }); } }
                    BwTextField { Layout.preferredWidth: 80; implicitHeight: 30; font.pixelSize: 12; text: modelData.next || ""; placeholderText: "Next state"
                        onEditingFinished: { const l = root.copy(an.a.states); l[index].next = text.trim(); root.writeAnimation(an.c, { states: l }); } }
                    IconButton { iconName: "x"; tip: "Remove this state"; implicitWidth: 24; implicitHeight: 24
                        onClicked: { const l = root.copy(an.a.states); l.splice(index, 1); root.writeAnimation(an.c, { states: l }); } }
                }
            }
            BwButton {
                iconName: "plus"; text: "Add state"; implicitHeight: 28; font.pixelSize: 12
                onClicked: {
                    const l = root.copy(an.a.states);
                    const taken = l.map(x => x.name);
                    let name = "idle";
                    for (let n = 2; taken.indexOf(name) >= 0; ++n) name = "idle " + n;
                    l.push({ name: name, clip: an.a.clips.length ? an.a.clips[0].name : "", speed: 1, next: "" });
                    root.writeAnimation(an.c, { states: l });
                }
            }
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
