import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Everything that belongs to the whole project rather than one actor: its
// dimension and icon, the world, lighting, post-process, the sound mix and
// input actions. Each row writes straight through to the backend. World rows
// edit the open scene (see its name below); each scene owns its own World.
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
    function postOf() { return Object.assign({ exposure_ev: 9.7, tonemapping: "TonyMcMapface", bloom_enabled: false, bloom_threshold: 1, bloom_intensity: 0.15, bloom_knee: 0.5, bloom_scatter: 0.7, bloom_dirt: "", bloom_dirt_intensity: 0, vignette_strength: 0, chromatic_aberration: 0, sharpen: 0 }, world && world.post ? world.post : {}); }
    function soundOf() { return Object.assign({ master_volume: 1, music_volume: 1, sfx_volume: 1 }, world && world.sound ? world.sound : {}); }
    function navigationOf() { return Object.assign({ areas: [], links: [] }, world && world.navigation ? world.navigation : {}); }
    function writeNavigation(next) { invoke("set_navigation", { navigation: Object.assign(navigationOf(), next) }); }
    function writeCamera(next) { invoke("set_camera", { camera: Object.assign(JSON.parse(JSON.stringify(world.camera)), next) }); }
    function shadowsOf() {
        return Object.assign({ filter: "Gaussian", distance: 150, cascades: 4, first_cascade: 5, cascade_blend: 0.2, fade: 0.1, normal_bias: 1.8, sun_size: 0, contact: false, contact_length: 0.3, contact_thickness: 0.1 },
            world && world.lighting ? world.lighting.shadows || {} : {});
    }
    function writeShadows(next) { writeLighting({ shadows: Object.assign(shadowsOf(), next) }); }
    function tracingOf() {
        return Object.assign({ enabled: false, bounces: 3, samples: 8, denoiser: "Auto", gi_distance: 50, mode: "Hybrid", paths: 1 },
            world && world.lighting ? world.lighting.ray_tracing || {} : {});
    }
    function writeTracing(next) { writeLighting({ ray_tracing: Object.assign(tracingOf(), next) }); }
    // What the open world's GPU can do, once a 3D world has come up.
    readonly property var tracingStatus: app.appState.ray_tracing || null
    function writeSky(next) { invoke("set_sky", { sky: Object.assign(JSON.parse(JSON.stringify(world.sky)), next) }); }
    function writeSkyPart(part, next) { const o = {}; o[part] = Object.assign(JSON.parse(JSON.stringify(world.sky[part])), next); writeSky(o); }
    function writeFogPart(part, next) { const fog = JSON.parse(JSON.stringify(world.fog)); fog[part] = Object.assign(fog[part], next); invoke("set_fog", { fog: fog }); }
    function writeLightning(next) { invoke("set_lightning", { lightning: Object.assign(JSON.parse(JSON.stringify(world.lightning)), next) }); }
    function writeVolumetricClouds(next) { invoke("set_clouds", { clouds: Object.assign(JSON.parse(JSON.stringify(world.clouds)), next) }); }
    function cloudLayerDefaults() {
        return { enabled: true, name: "", coverage_texture: "", seed: 1, scale: 4, octaves: 5, coverage: 0.5, contrast: 2, tiling_km: 20, opacity: 1, altitude: 8000, parallax: 200,
            tint: "#FFFFFF", sun_tint: "#FFF4E6", edge_tint: "#FFE0C0", horizon_fade: [12, 1], day_tint: "#FFFFFF", sunset_tint: "#FFB08A", night_tint: "#8090B0",
            scroll: [0, 0], wind: 1, flow_map: "", flow_strength: 0, flow_period: 60, spin: 0, pivot: [0, 0], aerial: 1, shadow: 0.5, revision: 0 };
    }
    function cloudLayersOf() { return world && world.cloud_layers ? JSON.parse(JSON.stringify(world.cloud_layers)) : []; }
    function writeCloudLayers(layers) { invoke("set_cloud_layers", { layers: layers }); }
    function writeCloudLayer(index, next) { const layers = cloudLayersOf(); layers[index] = Object.assign(layers[index], next); writeCloudLayers(layers); }
    function vfxOf() { return Object.assign({ budget: 200000, cpu_only: false }, world && world.vfx ? world.vfx : {}); }
    function writeVfx(next) { invoke("set_vfx", { vfx: Object.assign(vfxOf(), next) }); }
    function writeWind(next) { invoke("set_wind", { wind: Object.assign(JSON.parse(JSON.stringify(world.wind)), next) }); }
    function writeClouds(next) { writeWind({ clouds: Object.assign(JSON.parse(JSON.stringify(world.wind.clouds)), next) }); }
    function writeLighting(next) { invoke("set_lighting", { lighting: Object.assign(JSON.parse(JSON.stringify(world.lighting)), next) }); }
    function writePost(next) { invoke("set_post_process", { post: Object.assign(postOf(), next) }); }
    function displayOf() { return Object.assign({ space: "Sdr", peak_nits: 1000, paper_white_nits: 200 }, world && world.display ? world.display : {}); }
    function writeDisplay(next) { invoke("set_display_output", { display: Object.assign(displayOf(), next) }); }
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
    component SubHeading: Text { color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold; Layout.topMargin: 4 }

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
                Note { text: (root.is3d ? "Meshes and 3D physics, measured in metres." : "Sprites and flat physics, measured in pixels.") + " Switching converts the open scene; a running game swaps live." }
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
                InspectorRow { label: "Default scene"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField {
                        Layout.fillWidth: true
                        options: (root.project && root.project.scenes ? root.project.scenes : []).map(s => ({ value: s.id, label: s.name }))
                        value: root.project ? (root.project.default_scene || root.project.active_scene) : ""
                        placeholder: "Scene"
                        onChosen: v => root.invoke("set_default_scene", { sceneId: v })
                    } }
                Note { text: "Which scene loads when the project boots, and where a built game starts." }
                InspectorRow { label: "Open scene"; labelWidth: 110; Layout.fillWidth: true
                    Text { Layout.fillWidth: true; color: Theme.text; font.pixelSize: 12; elide: Text.ElideRight
                        text: { const s = (root.project && root.project.scenes ? root.project.scenes : []).find(x => x.id === root.project.active_scene); return s ? s.name : ""; } } }
                Note { text: "World rows below edit this scene; each scene owns its own World. Double-click another scene file in the Assets tray to open it." }
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
                heading: "Navigation"; visible: !!root.world
                Note { text: root.is3d ? "Coordinates use X and Z. Areas add travel cost; links cross gaps or join separate surfaces. A zero layer mask applies to every agent." : "Coordinates use X and Y. Areas add travel cost; links cross gaps or join separate surfaces. A zero layer mask applies to every agent." }
                Note { text: "Cost areas: center, size, cost, layers. Higher cost discourages travel through an area." }
                TextField { Layout.fillWidth: true; text: JSON.stringify(root.navigationOf().areas); placeholderText: "Cost areas JSON"
                    onEditingFinished: {
                        try { const areas = JSON.parse(text); if (Array.isArray(areas)) root.writeNavigation({ areas: areas }); }
                        catch (e) { text = JSON.stringify(root.navigationOf().areas); }
                    } }
                Note { text: "Off-mesh links: from, to, cost, bidirectional, layers." }
                TextField { Layout.fillWidth: true; text: JSON.stringify(root.navigationOf().links); placeholderText: "Links JSON"
                    onEditingFinished: {
                        try { const links = JSON.parse(text); if (Array.isArray(links)) root.writeNavigation({ links: links }); }
                        catch (e) { text = JSON.stringify(root.navigationOf().links); }
                    } }
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
                Note { text: "Where the 3D sun shines from (aimed at the origin, unless the sky places it), and how the scene's ambient light looks. Occlusion darkens creases where objects meet but costs GPU time. Shadow detail snaps to a power of two; raise the bias if striped acne appears on lit faces. Applies on the next run of the game." }
            }
            Section {
                id: skySection
                heading: "Sky"; visible: !!root.world && root.is3d && !!root.world.sky
                readonly property var sky: root.world && root.world.sky ? root.world.sky : null
                readonly property string kind: sky ? sky.kind : "Flat"
                readonly property string sunMode: sky ? sky.sun.mode : "Light"
                readonly property var p: sky ? sky.physical : ({})
                readonly property var g: sky ? sky.gradient : ({})
                readonly property var h: sky ? sky.hdri : ({})
                readonly property var st: sky && sky.stars ? sky.stars : ({})
                readonly property var au: sky && sky.aurora ? sky.aurora : ({})
                readonly property bool stars: kind !== "Flat" && !!st.enabled
                readonly property bool aurora: kind !== "Flat" && !!au.enabled
                InspectorRow { label: "Kind"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Flat", label: "None (background color)" }, { value: "Physical", label: "Physical atmosphere" }, { value: "Gradient", label: "Gradient" }, { value: "Hdri", label: "HDR image" }]
                        value: skySection.kind; onChosen: v => root.writeSky({ kind: v }) } }
                InspectorRow { label: "Sun from"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Light", label: "Light direction" }, { value: "Manual", label: "Azimuth and elevation" }, { value: "Geographic", label: "Place and time" }]
                        value: skySection.sunMode; onChosen: v => root.writeSkyPart("sun", { mode: v }) } }
                InspectorRow { visible: skySection.sunMode === "Manual"; label: "Azimuth, elev °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.sky ? skySection.sky.sun.azimuth : 135; fallback: 135; onCommitted: n => root.writeSkyPart("sun", { azimuth: n }) }
                    NumberField { value: skySection.sky ? skySection.sky.sun.elevation : 54.7; fallback: 54.7; onCommitted: n => root.writeSkyPart("sun", { elevation: root.clamp(n, -90, 90) }) } }
                InspectorRow { visible: skySection.sunMode === "Geographic"; label: "Lat, long °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.sky ? skySection.sky.sun.latitude : 40; fallback: 40; onCommitted: n => root.writeSkyPart("sun", { latitude: root.clamp(n, -90, 90) }) }
                    NumberField { value: skySection.sky ? skySection.sky.sun.longitude : 0; fallback: 0; onCommitted: n => root.writeSkyPart("sun", { longitude: root.clamp(n, -180, 180) }) } }
                InspectorRow { visible: skySection.sunMode === "Geographic"; label: "Day, hour, UTC±"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.sky ? skySection.sky.sun.day_of_year : 172; fallback: 172; onCommitted: n => root.writeSkyPart("sun", { day_of_year: root.clamp(Math.round(n), 1, 365) }) }
                    NumberField { value: skySection.sky ? skySection.sky.sun.time_of_day : 12; fallback: 12; onCommitted: n => root.writeSkyPart("sun", { time_of_day: root.clamp(n, 0, 24) }) }
                    NumberField { value: skySection.sky ? skySection.sky.sun.utc_offset : 0; fallback: 0; onCommitted: n => root.writeSkyPart("sun", { utc_offset: root.clamp(n, -14, 14) }) } }

                // Physical
                InspectorRow { visible: skySection.kind === "Physical"; label: "Sun °, limb, ×"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.sun_size; fallback: 0.53; onCommitted: n => root.writeSkyPart("physical", { sun_size: root.clamp(n, 0.05, 20) }) }
                    NumberField { value: skySection.p.limb_darkening; fallback: 0.6; onCommitted: n => root.writeSkyPart("physical", { limb_darkening: root.clamp(n, 0, 1) }) }
                    NumberField { value: skySection.p.sun_intensity; fallback: 1; onCommitted: n => root.writeSkyPart("physical", { sun_intensity: root.clamp(n, 0, 100) }) } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Tint sunlight"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.p.tint_sun; onToggled: on => root.writeSkyPart("physical", { tint_sun: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Moon"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.p.moon; onToggled: on => root.writeSkyPart("physical", { moon: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.kind === "Physical" && !!skySection.p.moon; label: "Moon °, phase, nits"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.moon_size; fallback: 0.52; onCommitted: n => root.writeSkyPart("physical", { moon_size: root.clamp(n, 0.05, 20) }) }
                    NumberField { value: skySection.p.moon_phase; fallback: 0.5; onCommitted: n => root.writeSkyPart("physical", { moon_phase: root.clamp(n, 0, 1) }) }
                    NumberField { value: skySection.p.moon_brightness; fallback: 2500; onCommitted: n => root.writeSkyPart("physical", { moon_brightness: Math.max(n, 0) }) } }
                InspectorRow { visible: skySection.kind === "Physical" && !!skySection.p.moon; label: "Moon halo, power"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.moon_halo; fallback: 0.03; onCommitted: n => root.writeSkyPart("physical", { moon_halo: root.clamp(n, 0, 1) }) }
                    NumberField { value: skySection.p.moon_halo_power; fallback: 1000; onCommitted: n => root.writeSkyPart("physical", { moon_halo_power: root.clamp(n, 1, 100000) }) } }
                InspectorRow { visible: skySection.kind === "Physical" && !!skySection.p.moon && skySection.sunMode !== "Geographic"; label: "Moon az, elev °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.moon_azimuth; fallback: 300; onCommitted: n => root.writeSkyPart("physical", { moon_azimuth: n }) }
                    NumberField { value: skySection.p.moon_elevation; fallback: 30; onCommitted: n => root.writeSkyPart("physical", { moon_elevation: root.clamp(n, -90, 90) }) } }
                InspectorRow { visible: skySection.kind === "Physical" && !!skySection.p.moon; label: "Moonlight, lux"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.p.moon_light; onToggled: on => root.writeSkyPart("physical", { moon_light: on }) }
                    NumberField { visible: !!skySection.p.moon_light; value: skySection.p.moon_lux; fallback: 0.25; onCommitted: n => root.writeSkyPart("physical", { moon_lux: Math.max(n, 0) }) } }
                InspectorRow { visible: skySection.kind === "Physical" && !!skySection.p.moon && !!skySection.p.moon_light; label: "Moon color, shadows"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: skySection.p.moon_color || "#C9D6FF"; onPicked: c => root.writeSkyPart("physical", { moon_color: c }) }
                    SwitchField { value: !!skySection.p.moon_shadows; onToggled: on => root.writeSkyPart("physical", { moon_shadows: on }) } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Rayleigh /Mm"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 3; delegate: NumberField { required property int index; value: skySection.p.rayleigh ? skySection.p.rayleigh[index] : 0; onCommitted: n => root.writeSkyPart("physical", { rayleigh: root.withIndex(skySection.p.rayleigh, index, Math.max(n, 0)) }) } } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Mie /Mm, g"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.mie; fallback: 3.996; onCommitted: n => root.writeSkyPart("physical", { mie: Math.max(n, 0) }) }
                    NumberField { value: skySection.p.mie_g; fallback: 0.8; onCommitted: n => root.writeSkyPart("physical", { mie_g: root.clamp(n, -0.99, 0.99) }) } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Ozone /Mm"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 3; delegate: NumberField { required property int index; value: skySection.p.ozone ? skySection.p.ozone[index] : 0; onCommitted: n => root.writeSkyPart("physical", { ozone: root.withIndex(skySection.p.ozone, index, Math.max(n, 0)) }) } } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Heights km"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.rayleigh_height; fallback: 8; onCommitted: n => root.writeSkyPart("physical", { rayleigh_height: root.clamp(n, 0.1, 100) }) }
                    NumberField { value: skySection.p.mie_height; fallback: 1.2; onCommitted: n => root.writeSkyPart("physical", { mie_height: root.clamp(n, 0.1, 100) }) } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Planet, air km"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.planet_radius; fallback: 6360; onCommitted: n => root.writeSkyPart("physical", { planet_radius: root.clamp(n, 1, 100000) }) }
                    NumberField { value: skySection.p.atmosphere_height; fallback: 100; onCommitted: n => root.writeSkyPart("physical", { atmosphere_height: root.clamp(n, 1, 1000) }) } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Ground"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: skySection.p.ground_albedo || "#5A5A5A"; onPicked: c => root.writeSkyPart("physical", { ground_albedo: c }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Horizon curve"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.p.horizon_curve; fallback: 1; onCommitted: n => root.writeSkyPart("physical", { horizon_curve: root.clamp(n, 0.1, 10) }) } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Night, nits"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: skySection.p.night_color || "#1A2B4D"; onPicked: c => root.writeSkyPart("physical", { night_color: c }) }
                    NumberField { value: skySection.p.night_brightness; fallback: 1; onCommitted: n => root.writeSkyPart("physical", { night_brightness: Math.max(n, 0) }) } }
                InspectorRow { visible: skySection.kind === "Physical"; label: "Night from, to °"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 2; delegate: NumberField { required property int index; value: skySection.p.night_ramp ? skySection.p.night_ramp[index] : 0; onCommitted: n => root.writeSkyPart("physical", { night_ramp: root.withIndex(skySection.p.night_ramp, index, root.clamp(n, -90, 90)) }) } } }

                // Gradient
                InspectorRow { visible: skySection.kind === "Gradient"; label: "Top, middle"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: skySection.g.top || "#2F6BC4"; onPicked: c => root.writeSkyPart("gradient", { top: c }) }
                    ColorField { value: skySection.g.middle || "#A9CBE8"; onPicked: c => root.writeSkyPart("gradient", { middle: c }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.kind === "Gradient"; label: "Bottom"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: skySection.g.bottom || "#3A3F47"; onPicked: c => root.writeSkyPart("gradient", { bottom: c }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.kind === "Gradient"; label: "Horizon, soft"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.g.horizon_offset; fallback: 0; onCommitted: n => root.writeSkyPart("gradient", { horizon_offset: root.clamp(n, -1, 1) }) }
                    NumberField { value: skySection.g.softness; fallback: 0.4; onCommitted: n => root.writeSkyPart("gradient", { softness: root.clamp(n, 0.001, 1) }) } }
                InspectorRow { visible: skySection.kind === "Gradient"; label: "Warmth"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: skySection.g.warm_color || "#FF9A50"; onPicked: c => root.writeSkyPart("gradient", { warm_color: c }) }
                    NumberField { value: skySection.g.warmth; fallback: 0.5; onCommitted: n => root.writeSkyPart("gradient", { warmth: root.clamp(n, 0, 1) }) } }
                InspectorRow { visible: skySection.kind === "Gradient"; label: "Nits, dither"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.g.brightness; fallback: 1000; onCommitted: n => root.writeSkyPart("gradient", { brightness: Math.max(n, 0) }) }
                    SwitchField { value: !!skySection.g.dither; onToggled: on => root.writeSkyPart("gradient", { dither: on }) } }

                // HDRI
                InspectorRow { visible: skySection.kind === "Hdri"; label: "Image"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["hdr"]; value: skySection.h.path || ""; placeholderText: "Drag an HDR panorama here"; onCommitted: p => root.writeSkyPart("hdri", { path: p }) } }
                InspectorRow { visible: skySection.kind === "Hdri"; label: "Nits, tint"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.h.brightness; fallback: 1000; onCommitted: n => root.writeSkyPart("hdri", { brightness: Math.max(n, 0) }) }
                    ColorField { value: skySection.h.tint || "#FFFFFF"; onPicked: c => root.writeSkyPart("hdri", { tint: c }) } }
                InspectorRow { visible: skySection.kind === "Hdri"; label: "Turn, tilt °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.h.rotation; fallback: 0; onCommitted: n => root.writeSkyPart("hdri", { rotation: n }) }
                    NumberField { value: skySection.h.tilt; fallback: 0; onCommitted: n => root.writeSkyPart("hdri", { tilt: root.clamp(n, -90, 90) }) } }
                InspectorRow { visible: skySection.kind === "Hdri"; label: "Blur, seam °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.h.blur; fallback: 0; onCommitted: n => root.writeSkyPart("hdri", { blur: root.clamp(n, 0, 1) }) }
                    NumberField { value: skySection.h.seam_fix; fallback: 0; onCommitted: n => root.writeSkyPart("hdri", { seam_fix: root.clamp(n, 0, 45) }) } }

                // Shared
                InspectorRow { visible: skySection.kind !== "Flat"; label: "Background"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.sky && skySection.sky.background; onToggled: on => root.writeSky({ background: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.kind !== "Flat"; label: "Reflections"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.sky && skySection.sky.reflections; onToggled: on => root.writeSky({ reflections: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.kind !== "Flat"; label: "Ambient light"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.sky && skySection.sky.lighting; onToggled: on => root.writeSky({ lighting: on }) }
                    NumberField { visible: !!skySection.sky && skySection.sky.lighting; value: skySection.sky ? skySection.sky.ambient_dimmer : 1; fallback: 1; onCommitted: n => root.writeSky({ ambient_dimmer: root.clamp(n, 0, 10) }) } }
                InspectorRow { visible: skySection.kind !== "Flat"; label: "Sky exposure EV"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.sky ? skySection.sky.exposure : 0; fallback: 0; onCommitted: n => root.writeSky({ exposure: root.clamp(n, -16, 16) }) } }

                // Stars and aurora
                InspectorRow { visible: skySection.kind !== "Flat"; label: "Stars"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.st.enabled; onToggled: on => root.writeSkyPart("stars", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.stars; label: "Density, nits"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.st.density; fallback: 0.35; onCommitted: n => root.writeSkyPart("stars", { density: root.clamp(n, 0, 1) }) }
                    NumberField { value: skySection.st.brightness; fallback: 60; onCommitted: n => root.writeSkyPart("stars", { brightness: Math.max(n, 0) }) } }
                InspectorRow { visible: skySection.stars; label: "Faintness, color"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.st.magnitude_slope; fallback: 3; onCommitted: n => root.writeSkyPart("stars", { magnitude_slope: root.clamp(n, 0.5, 10) }) }
                    NumberField { value: skySection.st.color_variation; fallback: 0.5; onCommitted: n => root.writeSkyPart("stars", { color_variation: root.clamp(n, 0, 1) }) } }
                InspectorRow { visible: skySection.stars; label: "Twinkle, speed"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.st.twinkle; fallback: 0.3; onCommitted: n => root.writeSkyPart("stars", { twinkle: root.clamp(n, 0, 1) }) }
                    NumberField { value: skySection.st.twinkle_speed; fallback: 1.5; onCommitted: n => root.writeSkyPart("stars", { twinkle_speed: root.clamp(n, 0, 50) }) } }
                InspectorRow { visible: skySection.stars; label: "Horizon fade °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.st.horizon_fade; fallback: 8; onCommitted: n => root.writeSkyPart("stars", { horizon_fade: root.clamp(n, 0, 90) }) } }
                InspectorRow { visible: skySection.stars; label: "Sun fade °"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 2; delegate: NumberField { required property int index; value: skySection.st.sun_fade ? skySection.st.sun_fade[index] : 0; onCommitted: n => root.writeSkyPart("stars", { sun_fade: root.withIndex(skySection.st.sun_fade, index, root.clamp(n, -90, 90)) }) } } }
                InspectorRow { visible: skySection.stars; label: "Milky Way"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["image", "hdr"]; value: skySection.st.milky_way || ""; placeholderText: "Drag a panorama here"; onCommitted: p => root.writeSkyPart("stars", { milky_way: p }) } }
                InspectorRow { visible: skySection.stars && !!skySection.st.milky_way; label: "Band nits, turn, tilt"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.st.milky_way_brightness; fallback: 2; onCommitted: n => root.writeSkyPart("stars", { milky_way_brightness: Math.max(n, 0) }) }
                    NumberField { value: skySection.st.milky_way_rotation; fallback: 0; onCommitted: n => root.writeSkyPart("stars", { milky_way_rotation: n }) }
                    NumberField { value: skySection.st.milky_way_tilt; fallback: 60; onCommitted: n => root.writeSkyPart("stars", { milky_way_tilt: root.clamp(n, -90, 90) }) } }
                InspectorRow { visible: skySection.kind !== "Flat"; label: "Aurora, KP"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!skySection.au.enabled; onToggled: on => root.writeSkyPart("aurora", { enabled: on }) }
                    NumberField { visible: skySection.aurora; value: skySection.au.kp; fallback: 4; onCommitted: n => root.writeSkyPart("aurora", { kp: root.clamp(n, 0, 9) }) } }
                InspectorRow { visible: skySection.aurora; label: "Layers, pole °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.au.layers; fallback: 2; onCommitted: n => root.writeSkyPart("aurora", { layers: root.clamp(Math.round(n), 1, 3) }) }
                    NumberField { value: skySection.au.pole_azimuth; fallback: 0; onCommitted: n => root.writeSkyPart("aurora", { pole_azimuth: n }) } }
                InspectorRow { visible: skySection.aurora; label: "Foot, height km"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.au.altitude; fallback: 100; onCommitted: n => root.writeSkyPart("aurora", { altitude: root.clamp(n, 1, 1000) }) }
                    NumberField { value: skySection.au.height; fallback: 150; onCommitted: n => root.writeSkyPart("aurora", { height: root.clamp(n, 1, 1000) }) } }
                InspectorRow { visible: skySection.aurora; label: "Folds, rays km"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.au.width; fallback: 60; onCommitted: n => root.writeSkyPart("aurora", { width: root.clamp(n, 0.1, 10000) }) }
                    NumberField { value: skySection.au.ray_scale; fallback: 1.5; onCommitted: n => root.writeSkyPart("aurora", { ray_scale: root.clamp(n, 0.01, 1000) }) } }
                InspectorRow { visible: skySection.aurora; label: "Bottom, top"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: skySection.au.bottom_color || "#38FF8A"; onPicked: c => root.writeSkyPart("aurora", { bottom_color: c }) }
                    ColorField { value: skySection.au.top_color || "#A64DFF"; onPicked: c => root.writeSkyPart("aurora", { top_color: c }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: skySection.aurora; label: "Nits, flow, glow"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: skySection.au.brightness; fallback: 8; onCommitted: n => root.writeSkyPart("aurora", { brightness: Math.max(n, 0) }) }
                    NumberField { value: skySection.au.speed; fallback: 1; onCommitted: n => root.writeSkyPart("aurora", { speed: root.clamp(n, 0, 100) }) }
                    NumberField { value: skySection.au.horizon_glow; fallback: 0.3; onCommitted: n => root.writeSkyPart("aurora", { horizon_glow: root.clamp(n, 0, 1) }) } }
                Note { visible: skySection.kind !== "Flat"; text: "Stars fade in as the sun sinks between the two sun-fade elevations; faintness steepens how many dim stars there are for each bright one. The Milky Way is a 2:1 panorama laid over them. Aurora curtains hang between the foot and top altitudes; KP 0-9 sets how far from the pole they reach (9 is overhead), and `set aurora to KP` changes it while the game runs. The moon lights the world as a directional light of its own, dimmed by its phase and fading as it sets." }
                Note { text: "The sky draws the background and lights the world: its ambient light (scaled by the dimmer) and its reflections come from the same place. A physical sky scatters the sun (Rayleigh for blue, Mie for haze round the sun, ozone for twilight) and reddens the sunlight near the horizon; placing the sun by latitude, longitude, day and hour moves the light too, and the moon trails a geographic sun by its phase. A gradient is three stops that warm near a low sun. An HDR image is an .hdr or .exr panorama (2:1) or a strip of six faces, at its brightness in nits; blur softens the background only. Sky exposure is added to the camera's, never instead of it." }
            }
            Section {
                id: cloudSection
                heading: "Volumetric clouds"; visible: !!root.world && root.is3d && !!root.world.clouds
                readonly property var c: root.world && root.world.clouds ? root.world.clouds : ({})
                InspectorRow { label: "Enabled"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!cloudSection.c.enabled; onToggled: on => root.writeVolumetricClouds({ enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Quality"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: ["Low", "Medium", "High", "Ultra"].map(v => ({ value: v, label: v })); value: cloudSection.c.quality || "High"; onChosen: v => root.writeVolumetricClouds({ quality: v }) } }
                InspectorRow { label: "Coverage"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.coverage; fallback: 0.5; onCommitted: n => root.writeVolumetricClouds({ coverage: n }) } }
                InspectorRow { label: "Density"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.density; fallback: 0.8; onCommitted: n => root.writeVolumetricClouds({ density: n }) } }
                InspectorRow { label: "Stratus / cumulus"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.cloud_type; fallback: 0.7; onCommitted: n => root.writeVolumetricClouds({ cloud_type: n }) } }
                InspectorRow { label: "Bottom m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.bottom; fallback: 1500; onCommitted: n => root.writeVolumetricClouds({ bottom: n }) } }
                InspectorRow { label: "Top m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.top; fallback: 3500; onCommitted: n => root.writeVolumetricClouds({ top: n }) } }
                InspectorRow { label: "Tiling km"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.tiling_km; fallback: 12; onCommitted: n => root.writeVolumetricClouds({ tiling_km: n }) } }
                InspectorRow { label: "Coverage toe"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.toe; fallback: 0.2; onCommitted: n => root.writeVolumetricClouds({ toe: n }) } }
                InspectorRow { label: "Shoulder"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.shoulder; fallback: 0.8; onCommitted: n => root.writeVolumetricClouds({ shoulder: n }) } }
                InspectorRow { label: "Detail scale"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.detail_scale; fallback: 6; onCommitted: n => root.writeVolumetricClouds({ detail_scale: n }) } }
                InspectorRow { label: "Erosion"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.erosion; fallback: 0.35; onCommitted: n => root.writeVolumetricClouds({ erosion: n }) } }
                InspectorRow { label: "Detail speed"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.detail_speed; fallback: 1; onCommitted: n => root.writeVolumetricClouds({ detail_speed: n }) } }
                InspectorRow { label: "Anvil"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.anvil; fallback: 0.3; onCommitted: n => root.writeVolumetricClouds({ anvil: n }) } }
                InspectorRow { label: "Bottom billow"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.billow; fallback: 0.15; onCommitted: n => root.writeVolumetricClouds({ billow: n }) } }
                InspectorRow { label: "Top feather"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.feather; fallback: 0.2; onCommitted: n => root.writeVolumetricClouds({ feather: n }) } }
                InspectorRow { label: "Forward lobe"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.forward; fallback: 0.8; onCommitted: n => root.writeVolumetricClouds({ forward: n }) } }
                InspectorRow { label: "Back lobe"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.backward; fallback: -0.2; onCommitted: n => root.writeVolumetricClouds({ backward: n }) } }
                InspectorRow { label: "Back blend"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.back_blend; fallback: 0.2; onCommitted: n => root.writeVolumetricClouds({ back_blend: n }) } }
                InspectorRow { label: "Powder"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.powder; fallback: 1; onCommitted: n => root.writeVolumetricClouds({ powder: n }) } }
                InspectorRow { label: "Light bleed"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.lightbleed; fallback: 0.3; onCommitted: n => root.writeVolumetricClouds({ lightbleed: n }) } }
                InspectorRow { label: "Sky light"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.ambient; fallback: 1; onCommitted: n => root.writeVolumetricClouds({ ambient: n }) } }
                InspectorRow { label: "Belly occlusion"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.bottom_occlusion; fallback: 0.65; onCommitted: n => root.writeVolumetricClouds({ bottom_occlusion: n }) } }
                InspectorRow { label: "Shadow strength"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.shadow_strength; fallback: 0.7; onCommitted: n => root.writeVolumetricClouds({ shadow_strength: n }) } }
                InspectorRow { label: "Shadow range m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.shadow_range; fallback: 20000; onCommitted: n => root.writeVolumetricClouds({ shadow_range: n }) } }
                InspectorRow { label: "Early out"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: cloudSection.c.threshold; fallback: 0.01; onCommitted: n => root.writeVolumetricClouds({ threshold: n }) } }
                InspectorRow { label: "Shear X/Z m"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 2; delegate: NumberField { required property int index; value: (cloudSection.c.shear || [0, 0])[index]; onCommitted: n => root.writeVolumetricClouds({ shear: root.withIndex(cloudSection.c.shear || [0, 0], index, n) }) } } }
                InspectorRow { label: "Ground shadows"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!cloudSection.c.shadows; onToggled: on => root.writeVolumetricClouds({ shadows: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { label: "Sun / moon shadows"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!cloudSection.c.sun_shadows; onToggled: on => root.writeVolumetricClouds({ sun_shadows: on }) }
                    SwitchField { value: !!cloudSection.c.moon_shadows; onToggled: on => root.writeVolumetricClouds({ moon_shadows: on }) } }
                InspectorRow { label: "Shape noise"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["image", "volume"]; value: cloudSection.c.shape_volume || ""; placeholderText: "Baked from the seed"; onCommitted: p => root.writeVolumetricClouds({ shape_volume: p }) }
                    IconButton { iconName: "x"; tip: "Bake from the seed"; enabled: !!cloudSection.c.shape_volume; onClicked: root.writeVolumetricClouds({ shape_volume: "" }) } }
                InspectorRow { label: "Erosion noise"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["image", "volume"]; value: cloudSection.c.detail_volume || ""; placeholderText: "Baked from the seed"; onCommitted: p => root.writeVolumetricClouds({ detail_volume: p }) }
                    IconButton { iconName: "x"; tip: "Bake from the seed"; enabled: !!cloudSection.c.detail_volume; onClicked: root.writeVolumetricClouds({ detail_volume: "" }) } }
                InspectorRow { label: ""; labelWidth: 110; Layout.fillWidth: true
                    BwButton { text: "Bake noise to assets"; iconName: "download"; implicitHeight: 30; onClicked: root.invoke("bake_cloud_noise", {}) } Item { Layout.fillWidth: true } }
                Note { text: "Thickness is top minus bottom, in metres. Clouds ride the Wind section's cloud drift and seed. Quality controls view and light march steps: Low 16/3, Medium 32/5, High 48/6, Ultra 64/8. Noise volumes are image strips of square slices (e.g. 16384x128) or .cube files: shape reads red as its Perlin-Worley base, erosion reads red, green and blue as Worley octaves. Bake noise to assets writes the seed's own noise into assets/clouds to edit or swap." }
            }
            Section {
                id: layerSection
                heading: "Cloud layers"; visible: !!root.world && root.is3d
                readonly property var layers: root.world && root.world.cloud_layers ? root.world.cloud_layers : []
                // What the paint canvas does with a stroke.
                property string tool: "Cloud"
                property real radius: 0.05
                property real strength: 0.5
                Repeater {
                    model: layerSection.layers.length
                    delegate: ColumnLayout {
                        id: layerCard
                        required property int index
                        readonly property var l: layerSection.layers[index] || ({})
                        function write(next) { root.writeCloudLayer(index, next); }
                        Layout.fillWidth: true; spacing: 6
                        InspectorRow { label: "Layer " + (layerCard.index + 1); labelWidth: 110; Layout.fillWidth: true
                            SwitchField { value: !!layerCard.l.enabled; onToggled: on => layerCard.write({ enabled: on }) } Item { Layout.fillWidth: true }
                            IconButton { iconName: "trash-2"; tip: "Remove this layer"; onClicked: { const layers = root.cloudLayersOf(); layers.splice(layerCard.index, 1); root.writeCloudLayers(layers); } } }
                        InspectorRow { label: "Altitude m, opacity"; labelWidth: 110; Layout.fillWidth: true
                            NumberField { value: layerCard.l.altitude; fallback: 8000; onCommitted: n => layerCard.write({ altitude: root.clamp(n, -10000, 100000) }) }
                            NumberField { value: layerCard.l.opacity; fallback: 1; onCommitted: n => layerCard.write({ opacity: root.clamp(n, 0, 1) }) } }
                        InspectorRow { label: "Cover, contrast"; labelWidth: 110; Layout.fillWidth: true
                            NumberField { value: layerCard.l.coverage; fallback: 0.5; onCommitted: n => layerCard.write({ coverage: root.clamp(n, 0, 1) }) }
                            NumberField { value: layerCard.l.contrast; fallback: 2; onCommitted: n => layerCard.write({ contrast: root.clamp(n, 1, 16) }) } }
                        InspectorRow { label: "Coverage"; labelWidth: 110; Layout.fillWidth: true
                            AssetField { app: root.app; accept: ["image"]; value: layerCard.l.coverage_texture || ""; placeholderText: "Noise from the seed"; onCommitted: p => layerCard.write({ coverage_texture: p }) }
                            IconButton { iconName: "x"; tip: "Back to noise"; enabled: !!layerCard.l.coverage_texture; onClicked: layerCard.write({ coverage_texture: "" }) } }
                        InspectorRow { visible: !layerCard.l.coverage_texture; label: "Seed, scale, oct."; labelWidth: 110; Layout.fillWidth: true
                            NumberField { value: layerCard.l.seed; fallback: 1; onCommitted: n => layerCard.write({ seed: Math.max(Math.round(n), 0) }) }
                            NumberField { value: layerCard.l.scale; fallback: 4; onCommitted: n => layerCard.write({ scale: root.clamp(Math.round(n), 1, 64) }) }
                            NumberField { value: layerCard.l.octaves; fallback: 5; onCommitted: n => layerCard.write({ octaves: root.clamp(Math.round(n), 1, 8) }) } }
                        InspectorRow { label: "Tiling km, parallax"; labelWidth: 110; Layout.fillWidth: true
                            NumberField { value: layerCard.l.tiling_km; fallback: 20; onCommitted: n => layerCard.write({ tiling_km: root.clamp(n, 0.1, 1000) }) }
                            NumberField { value: layerCard.l.parallax; fallback: 200; onCommitted: n => layerCard.write({ parallax: root.clamp(n, 0, 10000) }) } }
                        InspectorRow { label: "Tint, sun, edge"; labelWidth: 110; Layout.fillWidth: true
                            ColorField { value: layerCard.l.tint || "#FFFFFF"; onPicked: c => layerCard.write({ tint: c }) }
                            ColorField { value: layerCard.l.sun_tint || "#FFF4E6"; onPicked: c => layerCard.write({ sun_tint: c }) }
                            ColorField { value: layerCard.l.edge_tint || "#FFE0C0"; onPicked: c => layerCard.write({ edge_tint: c }) } Item { Layout.fillWidth: true } }
                        InspectorRow { label: "Day, sunset, night"; labelWidth: 110; Layout.fillWidth: true
                            ColorField { value: layerCard.l.day_tint || "#FFFFFF"; onPicked: c => layerCard.write({ day_tint: c }) }
                            ColorField { value: layerCard.l.sunset_tint || "#FFB08A"; onPicked: c => layerCard.write({ sunset_tint: c }) }
                            ColorField { value: layerCard.l.night_tint || "#8090B0"; onPicked: c => layerCard.write({ night_tint: c }) } Item { Layout.fillWidth: true } }
                        InspectorRow { label: "Horizon fade °"; labelWidth: 110; Layout.fillWidth: true
                            Repeater { model: 2; delegate: NumberField { required property int index; value: (layerCard.l.horizon_fade || [12, 1])[index]; onCommitted: n => layerCard.write({ horizon_fade: root.withIndex(layerCard.l.horizon_fade || [12, 1], index, root.clamp(n, 0, 90)) }) } } }
                        InspectorRow { label: "Scroll m/s, wind"; labelWidth: 110; Layout.fillWidth: true
                            Repeater { model: 2; delegate: NumberField { required property int index; value: (layerCard.l.scroll || [0, 0])[index]; onCommitted: n => layerCard.write({ scroll: root.withIndex(layerCard.l.scroll || [0, 0], index, root.clamp(n, -10000, 10000)) }) } }
                            NumberField { value: layerCard.l.wind; fallback: 1; onCommitted: n => layerCard.write({ wind: root.clamp(n, 0, 100) }) } }
                        InspectorRow { label: "Flow map"; labelWidth: 110; Layout.fillWidth: true
                            AssetField { app: root.app; accept: ["image"]; value: layerCard.l.flow_map || ""; placeholderText: "None"; onCommitted: p => layerCard.write({ flow_map: p }) }
                            IconButton { iconName: "x"; tip: "No flow map"; enabled: !!layerCard.l.flow_map; onClicked: layerCard.write({ flow_map: "" }) } }
                        InspectorRow { visible: !!layerCard.l.flow_map; label: "Flow m, period s"; labelWidth: 110; Layout.fillWidth: true
                            NumberField { value: layerCard.l.flow_strength; fallback: 0; onCommitted: n => layerCard.write({ flow_strength: root.clamp(n, 0, 100000) }) }
                            NumberField { value: layerCard.l.flow_period; fallback: 60; onCommitted: n => layerCard.write({ flow_period: root.clamp(n, 0.1, 100000) }) } }
                        InspectorRow { label: "Spin °/s, pivot"; labelWidth: 110; Layout.fillWidth: true
                            NumberField { value: layerCard.l.spin; fallback: 0; onCommitted: n => layerCard.write({ spin: root.clamp(n, -360, 360) }) }
                            Repeater { model: 2; delegate: NumberField { required property int index; value: (layerCard.l.pivot || [0, 0])[index]; onCommitted: n => layerCard.write({ pivot: root.withIndex(layerCard.l.pivot || [0, 0], index, n) }) } } }
                        InspectorRow { label: "Haze, shadow"; labelWidth: 110; Layout.fillWidth: true
                            NumberField { value: layerCard.l.aerial; fallback: 1; onCommitted: n => layerCard.write({ aerial: root.clamp(n, 0, 1) }) }
                            NumberField { value: layerCard.l.shadow; fallback: 0.5; onCommitted: n => layerCard.write({ shadow: root.clamp(n, 0, 1) }) } }
                        // Paint canvas: one tile of coverage; a drag is one stroke.
                        Rectangle {
                            Layout.preferredWidth: 256; Layout.preferredHeight: 256; Layout.alignment: Qt.AlignHCenter
                            color: Theme.panelRaised; border.color: Theme.border; clip: true
                            Image {
                                anchors.fill: parent; cache: false; smooth: true
                                source: layerCard.l.coverage_texture ? root.app.assetUrl(layerCard.l.coverage_texture) + "?" + (layerCard.l.revision || 0) : ""
                            }
                            Text { anchors.centerIn: parent; visible: !layerCard.l.coverage_texture; color: Theme.textDim; font.pixelSize: 11; text: "Paint to start from the noise" }
                            Canvas {
                                id: strokeCanvas
                                anchors.fill: parent
                                property var points: []
                                onPaint: {
                                    const g = getContext("2d");
                                    g.clearRect(0, 0, width, height);
                                    if (points.length === 0) return;
                                    g.strokeStyle = layerSection.tool === "Eraser" ? "rgba(0,0,0,0.6)" : "rgba(255,255,255,0.6)";
                                    g.lineWidth = Math.max(layerSection.radius * 2 * width, 1);
                                    g.lineCap = "round"; g.lineJoin = "round";
                                    g.beginPath();
                                    g.moveTo(points[0][0] * width, points[0][1] * height);
                                    for (let i = 1; i < points.length; i++) g.lineTo(points[i][0] * width, points[i][1] * height);
                                    g.stroke();
                                }
                            }
                            MouseArea {
                                anchors.fill: parent
                                function at(mouse) { return [root.clamp(mouse.x / width, 0, 1), root.clamp(mouse.y / height, 0, 1)]; }
                                onPressed: mouse => { strokeCanvas.points = [at(mouse)]; strokeCanvas.requestPaint(); }
                                onPositionChanged: mouse => { const next = strokeCanvas.points.slice(); next.push(at(mouse)); strokeCanvas.points = next; strokeCanvas.requestPaint(); }
                                onReleased: {
                                    root.invoke("paint_cloud_layer", { layer: layerCard.index, points: strokeCanvas.points,
                                        brush: { tool: layerSection.tool, radius: layerSection.radius, strength: layerSection.strength, falloff: 0.7 } });
                                    strokeCanvas.points = []; strokeCanvas.requestPaint();
                                }
                            }
                        }
                    }
                }
                InspectorRow { visible: layerSection.layers.length > 0; label: "Brush"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: ["Cloud", "Eraser", "Blur", "Advect"].map(v => ({ value: v, label: v })); value: layerSection.tool; onChosen: v => layerSection.tool = v } }
                InspectorRow { visible: layerSection.layers.length > 0; label: "Radius, strength"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: layerSection.radius; fallback: 0.05; onCommitted: n => layerSection.radius = root.clamp(n, 0.001, 0.5) }
                    NumberField { value: layerSection.strength; fallback: 0.5; onCommitted: n => layerSection.strength = root.clamp(n, 0, 1) } }
                InspectorRow { label: ""; labelWidth: 110; Layout.fillWidth: true
                    BwButton { text: "Add layer"; iconName: "plus"; implicitHeight: 30; enabled: layerSection.layers.length < 4
                        onClicked: { const layers = root.cloudLayersOf(); const l = root.cloudLayerDefaults(); l.seed = layers.length + 1; l.altitude = 8000 - 2000 * layers.length; layers.push(l); root.writeCloudLayers(layers); } }
                    Item { Layout.fillWidth: true } }
                Note { text: "Up to four flat layers of cloud, drawn in front of or behind the volumetric clouds by altitude, and the whole sky's clouds when volumetrics are off. Coverage is an image (its brightness) or noise from the seed; cover 0 is clear, 1 overcast, and contrast sharpens the edges. Parallax is the layer's apparent thickness in metres, which also shades it from the sun. The tint is multiplied by the day, sunset and night tints as the sun sinks. Layers scroll by their own speed plus the Wind section's layer scroll times wind, and turn round the pivot for storm spin; a flow map's red and green push coverage around (0.5 is still). Shadow darkens the ground under full cover towards the sun. Paint on a layer's tile to draw coverage into assets/clouds/layer-N.png: Cloud adds, Eraser removes, Blur softens, Advect smears along the stroke, and undo takes strokes back. The `set cloud layer` block changes a layer's coverage, opacity, contrast, altitude or spin while the game runs." }
            }
            Section {
                id: fogSection
                heading: "Fog"; visible: !!root.world && root.is3d && !!root.world.fog
                readonly property var hf: root.world && root.world.fog ? root.world.fog.height : ({})
                readonly property var vf: root.world && root.world.fog ? root.world.fog.volumetric : ({})
                readonly property var af: root.world && root.world.fog ? root.world.fog.aerial : ({})
                InspectorRow { label: "Height fog"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!fogSection.hf.enabled; onToggled: on => root.writeFogPart("height", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: !!fogSection.hf.enabled; label: "See through m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.hf.distance; fallback: 400; onCommitted: n => root.writeFogPart("height", { distance: root.clamp(n, 0.1, 10000000) }) } }
                InspectorRow { visible: !!fogSection.hf.enabled; label: "Base, falloff"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.hf.base_height; fallback: 0; onCommitted: n => root.writeFogPart("height", { base_height: n }) }
                    NumberField { value: fogSection.hf.falloff; fallback: 0.05; onCommitted: n => root.writeFogPart("height", { falloff: root.clamp(n, 0, 10) }) } }
                InspectorRow { visible: !!fogSection.hf.enabled; label: "Starts at m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.hf.start; fallback: 0; onCommitted: n => root.writeFogPart("height", { start: Math.max(n, 0) }) } }
                InspectorRow { visible: !!fogSection.hf.enabled; label: "Day, dusk, night"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: fogSection.hf.day_color || "#C2CAD2"; onPicked: c => root.writeFogPart("height", { day_color: c }) }
                    ColorField { value: fogSection.hf.dusk_color || "#E8A778"; onPicked: c => root.writeFogPart("height", { dusk_color: c }) }
                    ColorField { value: fogSection.hf.night_color || "#2A3344"; onPicked: c => root.writeFogPart("height", { night_color: c }) } }
                InspectorRow { visible: !!fogSection.hf.enabled; label: "Sun glow, g"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.hf.sun_boost; fallback: 0.5; onCommitted: n => root.writeFogPart("height", { sun_boost: root.clamp(n, 0, 100) }) }
                    NumberField { value: fogSection.hf.sun_boost_g; fallback: 0.75; onCommitted: n => root.writeFogPart("height", { sun_boost_g: root.clamp(n, 0, 0.99) }) } }

                InspectorRow { label: "Volumetric"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!fogSection.vf.enabled; onToggled: on => root.writeFogPart("volumetric", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Density, g"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.vf.density; fallback: 0.02; onCommitted: n => root.writeFogPart("volumetric", { density: root.clamp(n, 0, 10) }) }
                    NumberField { value: fogSection.vf.anisotropy; fallback: 0.6; onCommitted: n => root.writeFogPart("volumetric", { anisotropy: root.clamp(n, -0.9, 0.9) }) } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Base, falloff"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.vf.base_height; fallback: 0; onCommitted: n => root.writeFogPart("volumetric", { base_height: n }) }
                    NumberField { value: fogSection.vf.falloff; fallback: 0.1; onCommitted: n => root.writeFogPart("volumetric", { falloff: root.clamp(n, 0, 10) }) } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Albedo, glow"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: fogSection.vf.albedo || "#FFFFFF"; onPicked: c => root.writeFogPart("volumetric", { albedo: c }) }
                    ColorField { value: fogSection.vf.emissive || "#000000"; onPicked: c => root.writeFogPart("volumetric", { emissive: c }) }
                    NumberField { value: fogSection.vf.emissive_strength; fallback: 0; onCommitted: n => root.writeFogPart("volumetric", { emissive_strength: Math.max(n, 0) }) } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Noise, scale m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.vf.noise; fallback: 0.5; onCommitted: n => root.writeFogPart("volumetric", { noise: root.clamp(n, 0, 1) }) }
                    NumberField { value: fogSection.vf.noise_scale; fallback: 12; onCommitted: n => root.writeFogPart("volumetric", { noise_scale: root.clamp(n, 0.1, 10000) }) } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Drift m/s"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: 3; delegate: NumberField { required property int index; value: fogSection.vf.noise_wind ? fogSection.vf.noise_wind[index] : 0; onCommitted: n => root.writeFogPart("volumetric", { noise_wind: root.withIndex(fogSection.vf.noise_wind, index, root.clamp(n, -1000, 1000)) }) } } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Range m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.vf.range; fallback: 96; onCommitted: n => root.writeFogPart("volumetric", { range: root.clamp(n, 4, 2000) }) } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Quality"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Low", label: "Low (80x45x48)" }, { value: "Medium", label: "Medium (128x72x64)" }, { value: "High", label: "High (160x90x96)" }]
                        value: fogSection.vf.quality || "Medium"; onChosen: v => root.writeFogPart("volumetric", { quality: v }) } }
                InspectorRow { visible: !!fogSection.vf.enabled; label: "Sun & moon, ambient"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!fogSection.vf.sun; onToggled: on => root.writeFogPart("volumetric", { sun: on }) }
                    NumberField { value: fogSection.vf.ambient; fallback: 1; onCommitted: n => root.writeFogPart("volumetric", { ambient: root.clamp(n, 0, 100) }) } }
                readonly property var dust: Object.assign({ enabled: false, count: 600, size: 0.012, alpha: 0.6, twinkle: 0.5, drift: 0.05 }, fogSection.vf.dust || {})
                function writeDust(next) { root.writeFogPart("volumetric", { dust: Object.assign(Object.assign({}, fogSection.dust), next) }); }
                InspectorRow { label: "Height dust"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!fogSection.dust.enabled; onToggled: on => fogSection.writeDust({ enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: !!fogSection.dust.enabled; label: "Up to m, count"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.vf.dust_height; fallback: 3; onCommitted: n => root.writeFogPart("volumetric", { dust_height: root.clamp(n, 0, 10000) }) }
                    NumberField { value: fogSection.dust.count; fallback: 600; onCommitted: n => fogSection.writeDust({ count: root.clamp(Math.round(n), 0, 4096) }) } }
                InspectorRow { visible: !!fogSection.dust.enabled; label: "Size m, alpha"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.dust.size; fallback: 0.012; onCommitted: n => fogSection.writeDust({ size: root.clamp(n, 0.001, 1) }) }
                    NumberField { value: fogSection.dust.alpha; fallback: 0.6; onCommitted: n => fogSection.writeDust({ alpha: root.clamp(n, 0, 1) }) } }
                InspectorRow { visible: !!fogSection.dust.enabled; label: "Twinkle, drift"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.dust.twinkle; fallback: 0.5; onCommitted: n => fogSection.writeDust({ twinkle: root.clamp(n, 0, 1) }) }
                    NumberField { value: fogSection.dust.drift; fallback: 0.05; onCommitted: n => fogSection.writeDust({ drift: root.clamp(n, 0, 10) }) } }

                InspectorRow { label: "Aerial haze"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!fogSection.af.enabled; onToggled: on => root.writeFogPart("aerial", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: !!fogSection.af.enabled; label: "Half gone m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.af.distance; fallback: 8000; onCommitted: n => root.writeFogPart("aerial", { distance: root.clamp(n, 1, 10000000) }) } }
                InspectorRow { visible: !!fogSection.af.enabled; label: "Tint, gray"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: fogSection.af.tint || "#FFFFFF"; onPicked: c => root.writeFogPart("aerial", { tint: c }) }
                    NumberField { value: fogSection.af.desaturation; fallback: 0.4; onCommitted: n => root.writeFogPart("aerial", { desaturation: root.clamp(n, 0, 1) }) } }
                InspectorRow { visible: !!fogSection.af.enabled; label: "Height m, blue"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: fogSection.af.height_scale; fallback: 1200; onCommitted: n => root.writeFogPart("aerial", { height_scale: root.clamp(n, 1, 100000) }) }
                    NumberField { value: fogSection.af.blue_shift; fallback: 0.7; onCommitted: n => root.writeFogPart("aerial", { blue_shift: root.clamp(n, 0, 1) }) } }
                Note { text: "Height fog thickens towards its base and thins going up; see-through is how far you can see at the base (95% gone). Its color follows the sun from day to dusk to night, and glows towards the sun. It covers the sky too, so the horizon melts into it. Volumetric fog fills a grid over the camera's first range metres: the sun and moon light it through their shadows (light shafts), lights opt in on their own card, it can glow by itself, and noise drifts through it with the wind. Volumes can add local fog inside their box. Aerial haze fades far things into the sky's color and grays them, less so higher up. Height dust hangs in the air near the volumetric base around the camera, lit by the sun and ambient, whether or not volumetric fog is on. `set fog density to` changes the height fog while the game runs, and scales volumetric fog and every light's beam by the same ratio, so 0 clears them all." }
            }
            Section {
                id: lightningSection
                heading: "Lightning"; visible: !!root.world && !!root.world.lightning
                readonly property var l: root.world && root.world.lightning ? root.world.lightning : ({})
                InspectorRow { label: "Storm, a minute"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!lightningSection.l.storm; onToggled: on => root.writeLightning({ storm: on }) }
                    NumberField { value: lightningSection.l.rate; fallback: 6; onCommitted: n => root.writeLightning({ rate: root.clamp(n, 0, 600) }) } }
                InspectorRow { label: "Region from"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index; value: root.world && root.world.lightning ? root.world.lightning.region_min[index] : 0; onCommitted: n => root.writeLightning({ region_min: root.withIndex(root.world.lightning.region_min, index, n) }) } } }
                InspectorRow { label: "Region to"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index; value: root.world && root.world.lightning ? root.world.lightning.region_max[index] : 0; onCommitted: n => root.writeLightning({ region_max: root.withIndex(root.world.lightning.region_max, index, n) }) } } }
                InspectorRow { visible: root.is3d; label: "Flash lumens"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: lightningSection.l.intensity; fallback: 20000000; onCommitted: n => root.writeLightning({ intensity: Math.max(n, 0) }) } }
                InspectorRow { label: "Color, decay s"; labelWidth: 110; Layout.fillWidth: true
                    ColorField { value: lightningSection.l.color || "#D8E4FF"; onPicked: c => root.writeLightning({ color: c }) }
                    NumberField { value: lightningSection.l.decay; fallback: 0.35; onCommitted: n => root.writeLightning({ decay: root.clamp(n, 0.01, 10) }) } }
                InspectorRow { visible: root.is3d; label: "Height, reach m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: lightningSection.l.flash_height; fallback: 60; onCommitted: n => root.writeLightning({ flash_height: root.clamp(n, 0, 10000) }) }
                    NumberField { value: lightningSection.l.range; fallback: 2000; onCommitted: n => root.writeLightning({ range: root.clamp(n, 1, 1000000) }) } }
                InspectorRow { label: "Sky pulse"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: lightningSection.l.sky_pulse; fallback: 6; onCommitted: n => root.writeLightning({ sky_pulse: root.clamp(n, 0, 1000) }) } }
                InspectorRow { label: "Thunder, volume"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!lightningSection.l.thunder; onToggled: on => root.writeLightning({ thunder: on }) }
                    NumberField { value: lightningSection.l.thunder_volume; fallback: 80; onCommitted: n => root.writeLightning({ thunder_volume: root.clamp(n, 0, 100) }) } }
                InspectorRow { label: "Thunder sound"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["audio"]; value: root.world && root.world.lightning ? root.world.lightning.thunder_sound : ""; placeholderText: "Built-in rumble"; onCommitted: p => root.writeLightning({ thunder_sound: p }) } }
                InspectorRow { label: "Seed"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: lightningSection.l.seed; fallback: 1; onCommitted: n => root.writeLightning({ seed: Math.max(Math.round(n), 0) }) } }
                Note { text: "A strike flashes a light above where it lands (3D), pulses the ambient light and the sky by the sky pulse, and its thunder arrives later the further away it is, at the speed of sound. A storm strikes at random inside the region at about its rate; one seed always throws the same storm. Blocks: `strike lightning at` and `set lightning storm to`." }
            }
            Section {
                id: windSection
                heading: "Wind"; visible: !!root.world && !!root.world.wind
                readonly property var w: root.world && root.world.wind ? root.world.wind : ({})
                readonly property var c: windSection.w.clouds || ({})
                InspectorRow { label: "Direction, speed"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.w.direction; fallback: 45; onCommitted: n => root.writeWind({ direction: ((n % 360) + 360) % 360 }) }
                    NumberField { value: windSection.w.speed; fallback: 0; onCommitted: n => root.writeWind({ speed: root.clamp(n, 0, 10000) }) } }
                InspectorRow { label: "Gust, a second"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.w.gust; fallback: 0; onCommitted: n => root.writeWind({ gust: root.clamp(n, 0, 10000) }) }
                    NumberField { value: windSection.w.gust_frequency; fallback: 0.2; onCommitted: n => root.writeWind({ gust_frequency: root.clamp(n, 0, 10) }) } }
                InspectorRow { label: "Veer, storm"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.w.veer; fallback: 15; onCommitted: n => root.writeWind({ veer: root.clamp(n, 0, 180) }) }
                    NumberField { value: windSection.w.storm; fallback: 0; onCommitted: n => root.writeWind({ storm: root.clamp(n, 0, 1) }) } }
                InspectorRow { visible: root.is3d; label: "Ground profile"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: windSection.w.profile !== false; onToggled: on => root.writeWind({ profile: on }) } }
                InspectorRow { visible: root.is3d && windSection.w.profile !== false; label: "At m, roughness"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.w.reference_height; fallback: 10; onCommitted: n => root.writeWind({ reference_height: root.clamp(n, 0.01, 10000) }) }
                    NumberField { value: windSection.w.roughness; fallback: 0.1; onCommitted: n => root.writeWind({ roughness: root.clamp(n, 0.0001, 10) }) } }
                InspectorRow { visible: root.is3d && windSection.w.profile !== false; label: "Ground y"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.w.ground; fallback: 0; onCommitted: n => root.writeWind({ ground: n }) } }
                InspectorRow { label: "Seed"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.w.seed; fallback: 1; onCommitted: n => root.writeWind({ seed: Math.max(Math.round(n), 0) }) } }
                InspectorRow { visible: root.is3d; label: "Clouds at m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.c.altitude; fallback: 1500; onCommitted: n => root.writeClouds({ altitude: root.clamp(n, 0, 100000) }) } }
                InspectorRow { label: "Follow, layers"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.c.follow; fallback: 1; onCommitted: n => root.writeClouds({ follow: root.clamp(n, 0, 100) }) }
                    NumberField { value: windSection.c.layer_scroll; fallback: 1; onCommitted: n => root.writeClouds({ layer_scroll: root.clamp(n, 0, 100) }) } }
                InspectorRow { label: "Cloud drift"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index; value: windSection.c.advection ? windSection.c.advection[index] : 0; onCommitted: n => root.writeClouds({ advection: root.withIndex(windSection.c.advection || [0, 0, 0], index, root.clamp(n, -10000, 10000)) }) } } }
                InspectorRow { label: "Erosion drift"; labelWidth: 110; Layout.fillWidth: true
                    Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index; value: windSection.c.erosion ? windSection.c.erosion[index] : 0; onCommitted: n => root.writeClouds({ erosion: root.withIndex(windSection.c.erosion || [0, 0.5, 0], index, root.clamp(n, -10000, 10000)) }) } } }
                InspectorRow { label: "Time-lapse"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.c.time_lapse; fallback: 1; onCommitted: n => root.writeClouds({ time_lapse: root.clamp(n, 1, 1000) }) } }
                InspectorRow { label: "Cloud seed"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: windSection.c.seed; fallback: 1; onCommitted: n => root.writeClouds({ seed: Math.max(Math.round(n), 0) }) }
                    BwButton { text: "Shuffle"; iconName: "refresh-cw"; implicitHeight: 30; onClicked: root.writeClouds({ seed: 1 + Math.floor(Math.random() * 2147483646) }) } }
                Note { text: "Direction is where the wind blows towards, in degrees clockwise from north (-Z in 3D, up the screen in 2D), and speed is measured at the reference height. Gusts come and go on smooth noise, swinging the direction by up to the veer; one seed always blows the same. In 3D the ground profile calms the wind towards the ground on a log law. Storm (0 to 1) triples the speed, quadruples the gusts and triples zone turbulence at full. Particles ride it (each emitter says how much), volumetric fog's noise drifts with it, and the clouds take the wind at their altitude times follow, plus their own drift; time-lapse runs them up to 1000x faster. Volumes can make local wind zones. Blocks: `set wind`, `set cloud drift to`, and the `wind speed`, `wind direction` and `storm` atmosphere readings." }
            }
            Section {
                heading: "Particles"; visible: !!root.world
                InspectorRow { label: "Budget"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.vfxOf().budget; fallback: 200000; onCommitted: n => root.writeVfx({ budget: Math.max(1, Math.round(n)) }) } }
                InspectorRow { visible: root.is3d; label: "CPU only"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: root.vfxOf().cpu_only; onToggled: on => root.writeVfx({ cpu_only: on }) } Item { Layout.fillWidth: true } }
                Note { text: "The budget caps live particles across every emitter: past it, emitters stop spawning until some die. 3D emitters simulate on the GPU where it has compute shaders; CPU only keeps them all on the CPU, with 4096 particles each at most. The profiler shows the count, the GPU emitters and the overdraw." }
            }
            Section {
                heading: "Shadows"; visible: !!root.world && root.is3d
                InspectorRow { label: "Filter"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Hardware", label: "Hardware 2x2" }, { value: "Gaussian", label: "Gaussian PCF" }, { value: "Temporal", label: "Temporal" }]
                        value: root.shadowsOf().filter; onChosen: v => root.writeShadows({ filter: v }) } }
                InspectorRow { label: "Distance m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().distance; fallback: 150; onCommitted: n => root.writeShadows({ distance: root.clamp(n, 1, 10000) }) } }
                InspectorRow { label: "Cascades"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().cascades; fallback: 4; onCommitted: n => root.writeShadows({ cascades: root.clamp(Math.round(n), 1, 4) }) } }
                InspectorRow { label: "First split m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().first_cascade; fallback: 5; onCommitted: n => root.writeShadows({ first_cascade: root.clamp(n, 0.1, 10000) }) } }
                InspectorRow { label: "Cascade blend"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().cascade_blend; fallback: 0.2; onCommitted: n => root.writeShadows({ cascade_blend: root.clamp(n, 0, 0.5) }) } }
                InspectorRow { label: "Fade out"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().fade; fallback: 0.1; onCommitted: n => root.writeShadows({ fade: root.clamp(n, 0, 0.5) }) } }
                InspectorRow { label: "Normal bias"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().normal_bias; fallback: 1.8; onCommitted: n => root.writeShadows({ normal_bias: root.clamp(n, 0, 10) }) } }
                InspectorRow { label: "Soft sun °"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().sun_size; fallback: 0; onCommitted: n => root.writeShadows({ sun_size: root.clamp(n, 0, 10) }) } }
                InspectorRow { label: "Contact"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: root.shadowsOf().contact; onToggled: on => root.writeShadows({ contact: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.shadowsOf().contact; label: "Contact length"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.shadowsOf().contact_length; fallback: 0.3; onCommitted: n => root.writeShadows({ contact_length: root.clamp(n, 0.01, 10) }) }
                    NumberField { value: root.shadowsOf().contact_thickness; fallback: 0.1; onCommitted: n => root.writeShadows({ contact_thickness: root.clamp(n, 0.001, 2) }) } }
                InspectorRow { label: "Sun cookie"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["image"]; value: root.world && root.world.lighting.sun_cookie ? root.world.lighting.sun_cookie : ""; placeholderText: "Drag an image here"; onCommitted: p => root.writeLighting({ sun_cookie: p }) } }
                InspectorRow { visible: !!root.world && !!root.world.lighting.sun_cookie; label: "Cookie tile m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.world && root.world.lighting.sun_cookie_size !== undefined ? root.world.lighting.sun_cookie_size : 20; fallback: 20; onCommitted: n => root.writeLighting({ sun_cookie_size: root.clamp(n, 0.01, 100000) }) } }
                Note { text: "Distance is how far from the camera the sun's shadows reach; cascades split it, the first ending at the first split, and blend fades one into the next. Soft sun gives PCSS penumbras as if the sun were that many degrees across (0 keeps them hard; Temporal smooths their noise). Contact shadows raymarch the depth buffer under feet and small clutter; lights opt in on their own card. A sun cookie tiles across the world like cloud shadows." }
            }
            Section {
                heading: "Ray tracing"; visible: !!root.world && root.is3d
                InspectorRow { label: "Ray traced"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: root.tracingOf().enabled; onToggled: on => root.writeTracing({ enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.tracingOf().enabled; label: "Mode"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Hybrid", label: "Hybrid (ReSTIR)" }, { value: "PathTraced", label: "Path traced" }]
                        value: root.tracingOf().mode; onChosen: v => root.writeTracing({ mode: v }) } }
                InspectorRow { visible: root.tracingOf().enabled; label: "Bounces"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.tracingOf().bounces; fallback: 3; onCommitted: n => root.writeTracing({ bounces: root.clamp(Math.round(n), 1, 8) }) } }
                InspectorRow { visible: root.tracingOf().enabled && root.tracingOf().mode !== "PathTraced"; label: "Samples"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.tracingOf().samples; fallback: 8; onCommitted: n => root.writeTracing({ samples: root.clamp(Math.round(n), 1, 32) }) } }
                InspectorRow { visible: root.tracingOf().enabled && root.tracingOf().mode === "PathTraced"; label: "Paths / pixel"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.tracingOf().paths; fallback: 1; onCommitted: n => root.writeTracing({ paths: root.clamp(Math.round(n), 1, 16) }) } }
                InspectorRow { visible: root.tracingOf().enabled; label: "Denoiser"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Auto", label: "Auto" }, { value: "Filter", label: "Filter only" }, { value: "Restir", label: "ReSTIR reuse only" }, { value: "None", label: "None (raw)" }]
                        value: root.tracingOf().denoiser; onChosen: v => root.writeTracing({ denoiser: v }) } }
                InspectorRow { visible: root.tracingOf().enabled && root.tracingOf().mode !== "PathTraced"; label: "GI reach m"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.tracingOf().gi_distance; fallback: 50; onCommitted: n => root.writeTracing({ gi_distance: root.clamp(n, 1, 10000) }) } }
                Note { visible: !!root.tracingStatus && !root.tracingStatus.available; color: Theme.warning
                    text: "Not on this machine: " + (root.tracingStatus ? root.tracingStatus.reason : "") + ". The raster lighting stands in." }
                Note { text: "Traces the sun, the sky, glowing surfaces and every light marked Traced for shadows, bounced light and reflections, in place of the raster lighting, on a GPU that can trace rays (Vulkan or DX12 ray queries). Hybrid reuses samples through ReSTIR and caches bounced light; Path traced sends fresh paths from every pixel each frame, heavier and closer to the truth. Auto filters the noise across space and time. Bounces and samples trade noise and reach for speed; blocks can change both mid-run. Box-projected and shader surfaces keep the raster lights. Elsewhere the raster rig carries on." }
            }
            Section {
                id: postSection
                heading: "Post-process"; visible: !!root.world
                readonly property var p: root.postOf()
                readonly property var auto: p.auto_exposure || {}
                readonly property var grading: p.grading || {}
                readonly property var tone: p.tone || {}
                readonly property var dof: p.depth_of_field || {}
                readonly property var blur: p.motion_blur || {}
                readonly property var ao: p.ao || {}
                readonly property var ssr: p.ssr || {}
                readonly property var grain: p.grain || {}
                function writePart(key, next) { const part = {}; part[key] = Object.assign({}, postSection.p[key] || {}, next); root.writePost(part); }
                function percent(v) { return Math.round(v * 100) + "%"; }
                SubHeading { text: "Exposure" }
                InspectorRow { label: "Exposure EV"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.p.exposure_ev; fallback: 9.7; onCommitted: n => root.writePost({ exposure_ev: root.clamp(n, 0, 20) }) } }
                InspectorRow { visible: root.is3d; label: "Auto"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!postSection.auto.enabled; onToggled: on => postSection.writePart("auto_exposure", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.is3d && !!postSection.auto.enabled; label: "Metering"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Spot", label: "Spot" }, { value: "CenterWeighted", label: "Center-weighted" }, { value: "Average", label: "Average" }]
                        value: postSection.auto.metering || "Spot"; onChosen: v => postSection.writePart("auto_exposure", { metering: v }) } }
                InspectorRow { visible: root.is3d && !!postSection.auto.enabled; label: "EV min, max"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.auto.min_ev; fallback: 2; onCommitted: n => postSection.writePart("auto_exposure", { min_ev: root.clamp(n, -10, 30) }) }
                    NumberField { value: postSection.auto.max_ev; fallback: 16; onCommitted: n => postSection.writePart("auto_exposure", { max_ev: root.clamp(n, -10, 30) }) } }
                InspectorRow { visible: root.is3d && !!postSection.auto.enabled; label: "EV/s up, down"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.auto.speed_up; fallback: 3; onCommitted: n => postSection.writePart("auto_exposure", { speed_up: root.clamp(n, 0.01, 100) }) }
                    NumberField { value: postSection.auto.speed_down; fallback: 1; onCommitted: n => postSection.writePart("auto_exposure", { speed_down: root.clamp(n, 0.01, 100) }) } }
                InspectorRow { visible: root.is3d && !!postSection.auto.enabled; label: "Compensation"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.auto.compensation; fallback: 0; onCommitted: n => postSection.writePart("auto_exposure", { compensation: root.clamp(n, -10, 10) }) } }
                SubHeading { text: "Bloom" }
                InspectorRow { label: "Glow"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: postSection.p.bloom_enabled; onToggled: on => root.writePost({ bloom_enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: postSection.p.bloom_enabled; label: "Glow limit, knee"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.p.bloom_threshold; fallback: 1; onCommitted: n => root.writePost({ bloom_threshold: Math.max(n, 0) }) }
                    NumberField { value: postSection.p.bloom_knee; fallback: 0.5; onCommitted: n => root.writePost({ bloom_knee: root.clamp(n, 0, 1) }) } }
                InspectorRow { visible: postSection.p.bloom_enabled; label: "Glow amount"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round(postSection.p.bloom_intensity * 100); onMoved: root.writePost({ bloom_intensity: value / 100 }) }
                    Text { text: postSection.percent(postSection.p.bloom_intensity); color: Theme.textDim; font.pixelSize: 12 } }
                InspectorRow { visible: postSection.p.bloom_enabled; label: "Spread"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round(postSection.p.bloom_scatter * 100); onMoved: root.writePost({ bloom_scatter: value / 100 }) }
                    Text { text: postSection.percent(postSection.p.bloom_scatter); color: Theme.textDim; font.pixelSize: 12 } }
                InspectorRow { visible: postSection.p.bloom_enabled; label: "Lens dirt"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["image"]; value: postSection.p.bloom_dirt || ""; placeholderText: "Drag a dirt image here"; onCommitted: p => root.writePost({ bloom_dirt: p }) }
                    NumberField { Layout.preferredWidth: 60; value: postSection.p.bloom_dirt_intensity; fallback: 0; onCommitted: n => root.writePost({ bloom_dirt_intensity: root.clamp(n, 0, 10) }) } }
                SubHeading { text: "Tone and color" }
                InspectorRow { label: "Tonemap"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: Blocks.opts(["TonyMcMapface","None","Reinhard","ReinhardLuminance","AcesFitted","Filmic","AgX","Neutral"]); value: postSection.p.tonemapping; onChosen: v => root.writePost({ tonemapping: v }) } }
                InspectorRow { label: "Toe, shoulder"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.tone.toe; fallback: 0; onCommitted: n => postSection.writePart("tone", { toe: root.clamp(n, -1, 1) }) }
                    NumberField { value: postSection.tone.shoulder; fallback: 0; onCommitted: n => postSection.writePart("tone", { shoulder: root.clamp(n, 0, 1) }) } }
                InspectorRow { label: "Warmth, tint"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.grading.temperature; fallback: 0; onCommitted: n => postSection.writePart("grading", { temperature: root.clamp(n, -1, 1) }) }
                    NumberField { value: postSection.grading.tint; fallback: 0; onCommitted: n => postSection.writePart("grading", { tint: root.clamp(n, -1, 1) }) } }
                Repeater {
                    model: [{ key: "lift", label: "Lift RGB", lo: -1, hi: 1, base: 0 }, { key: "gamma", label: "Gamma RGB", lo: 0.1, hi: 10, base: 1 }, { key: "gain", label: "Gain RGB", lo: 0, hi: 10, base: 1 }]
                    delegate: InspectorRow {
                        id: lgg
                        required property var modelData
                        label: modelData.label; labelWidth: 110; Layout.fillWidth: true
                        Repeater {
                            model: 3
                            NumberField { required property int index; Layout.fillWidth: true
                                value: (postSection.grading[lgg.modelData.key] || [lgg.modelData.base, lgg.modelData.base, lgg.modelData.base])[index]; fallback: lgg.modelData.base
                                onCommitted: n => { const part = {}; part[lgg.modelData.key] = root.withIndex(postSection.grading[lgg.modelData.key] || [lgg.modelData.base, lgg.modelData.base, lgg.modelData.base], index, root.clamp(n, lgg.modelData.lo, lgg.modelData.hi)); postSection.writePart("grading", part); } }
                        }
                    }
                }
                InspectorRow { label: "Saturation, contrast"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.grading.saturation; fallback: 1; onCommitted: n => postSection.writePart("grading", { saturation: root.clamp(n, 0, 2) }) }
                    NumberField { value: postSection.grading.contrast; fallback: 1; onCommitted: n => postSection.writePart("grading", { contrast: root.clamp(n, 0, 2) }) } }
                InspectorRow { label: "LUT, amount"; labelWidth: 110; Layout.fillWidth: true
                    AssetField { app: root.app; accept: ["image", "volume"]; value: postSection.grading.lut || ""; placeholderText: "Drag a .cube LUT here"; onCommitted: p => postSection.writePart("grading", { lut: p }) }
                    NumberField { Layout.preferredWidth: 60; value: postSection.grading.lut_contribution; fallback: 1; onCommitted: n => postSection.writePart("grading", { lut_contribution: root.clamp(n, 0, 1) }) } }
                SubHeading { text: "Lens" }
                InspectorRow { label: "Corners"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round(postSection.p.vignette_strength * 100); onMoved: root.writePost({ vignette_strength: value / 100 }) }
                    Text { text: postSection.percent(postSection.p.vignette_strength); color: Theme.textDim; font.pixelSize: 12 } }
                InspectorRow { label: "Color fringes"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round((postSection.p.chromatic_aberration || 0) * 100); onMoved: root.writePost({ chromatic_aberration: value / 100 }) }
                    Text { text: postSection.percent(postSection.p.chromatic_aberration || 0); color: Theme.textDim; font.pixelSize: 12 } }
                InspectorRow { label: "Film grain"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round((postSection.grain.intensity || 0) * 100); onMoved: postSection.writePart("grain", { intensity: value / 100 }) }
                    Text { text: postSection.percent(postSection.grain.intensity || 0); color: Theme.textDim; font.pixelSize: 12 } }
                InspectorRow { visible: (postSection.grain.intensity || 0) > 0; label: "Grain size, response"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.grain.size; fallback: 1.5; onCommitted: n => postSection.writePart("grain", { size: root.clamp(n, 1, 4) }) }
                    NumberField { value: postSection.grain.response; fallback: 0.8; onCommitted: n => postSection.writePart("grain", { response: root.clamp(n, 0, 1) }) } }
                InspectorRow { label: "Sharpen"; labelWidth: 110; Layout.fillWidth: true
                    SliderField { from: 0; to: 100; stepSize: 1; value: Math.round((postSection.p.sharpen || 0) * 100); onMoved: root.writePost({ sharpen: value / 100 }) }
                    Text { text: postSection.percent(postSection.p.sharpen || 0); color: Theme.textDim; font.pixelSize: 12 } }
                SubHeading { visible: root.is3d; text: "Depth of field" }
                InspectorRow { visible: root.is3d; label: "Enabled"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!postSection.dof.enabled; onToggled: on => postSection.writePart("depth_of_field", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.is3d && !!postSection.dof.enabled; label: "Focus on"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { Layout.fillWidth: true; options: [{ value: "", label: "Fixed distance" }].concat((root.project ? root.project.actors : []).map(a => ({ value: a.name, label: a.name })))
                        value: postSection.dof.target || ""; onChosen: v => postSection.writePart("depth_of_field", { target: v }) } }
                InspectorRow { visible: root.is3d && !!postSection.dof.enabled; label: "Distance m, f-stop"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.dof.focus_distance; fallback: 10; onCommitted: n => postSection.writePart("depth_of_field", { focus_distance: root.clamp(n, 0.05, 100000) }) }
                    NumberField { value: postSection.dof.f_stops; fallback: 2.8; onCommitted: n => postSection.writePart("depth_of_field", { f_stops: root.clamp(n, 0.5, 64) }) } }
                InspectorRow { visible: root.is3d && !!postSection.dof.enabled; label: "Bokeh"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Hexagonal", label: "Six blades" }, { value: "Circular", label: "Circular" }]; value: postSection.dof.bokeh || "Hexagonal"; onChosen: v => postSection.writePart("depth_of_field", { bokeh: v }) } }
                InspectorRow { visible: root.is3d && !!postSection.dof.enabled; label: "Blur near"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: postSection.dof.near !== false; onToggled: on => postSection.writePart("depth_of_field", { near: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.is3d && !!postSection.dof.enabled; label: "Far limit m, max px"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.dof.far_limit; fallback: 0; onCommitted: n => postSection.writePart("depth_of_field", { far_limit: Math.max(0, n) }) }
                    NumberField { value: postSection.dof.max_blur; fallback: 32; onCommitted: n => postSection.writePart("depth_of_field", { max_blur: root.clamp(n, 1, 128) }) } }
                SubHeading { visible: root.is3d; text: "Screen space" }
                InspectorRow { visible: root.is3d; label: "Motion blur"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!postSection.blur.enabled; onToggled: on => postSection.writePart("motion_blur", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.is3d && !!postSection.blur.enabled; label: "Shutter °, samples"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.blur.shutter_angle; fallback: 180; onCommitted: n => postSection.writePart("motion_blur", { shutter_angle: root.clamp(n, 0, 360) }) }
                    NumberField { value: postSection.blur.samples; fallback: 4; onCommitted: n => postSection.writePart("motion_blur", { samples: root.clamp(Math.round(n), 1, 32) }) } }
                InspectorRow { visible: root.is3d && !!root.world && root.world.lighting.ao_enabled; label: "AO reach m, strength"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.ao.radius; fallback: 0.7285; onCommitted: n => postSection.writePart("ao", { radius: root.clamp(n, 0.01, 10) }) }
                    NumberField { value: postSection.ao.intensity; fallback: 1; onCommitted: n => postSection.writePart("ao", { intensity: root.clamp(n, 0, 4) }) } }
                InspectorRow { visible: root.is3d; label: "Reflections"; labelWidth: 110; Layout.fillWidth: true
                    SwitchField { value: !!postSection.ssr.enabled; onToggled: on => postSection.writePart("ssr", { enabled: on }) } Item { Layout.fillWidth: true } }
                InspectorRow { visible: root.is3d && !!postSection.ssr.enabled; label: "Rough cutoff, thick"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: postSection.ssr.roughness_cutoff; fallback: 0.4; onCommitted: n => postSection.writePart("ssr", { roughness_cutoff: root.clamp(n, 0.05, 1) }) }
                    NumberField { value: postSection.ssr.thickness; fallback: 0.25; onCommitted: n => postSection.writePart("ssr", { thickness: root.clamp(n, 0.001, 10) }) } }
                Note { text: "The camera's finish, in a fixed order: blur and lens first, then glow, color and tone, then the tonemapper, sharpening, the LUT and grain. Lower exposure brightens; auto exposure meters the frame and adapts within its EV range at its speed, and a set exposure block still wins. Glow lets bright light bleed across five levels, spread by Spread and lit up by lens dirt. Toe deepens (or lifts) the shadows and shoulder rolls off the highlights ahead of the tonemapper. The LUT maps display colors. Volumes can override most of these. Screen-space reflections draw surfaces deferred. The Game view's debug menu shows bloom levels, blur size and AO alone." }
            }
            Section {
                heading: "Display"; visible: !!root.world
                InspectorRow { label: "Output"; labelWidth: 110; Layout.fillWidth: true
                    ChoiceField { options: [{ value: "Sdr", label: "SDR" }, { value: "Hdr10", label: "HDR10 (PQ)" }, { value: "Scrgb", label: "scRGB" }]
                        value: root.displayOf().space; onChosen: v => root.writeDisplay({ space: v }) } }
                InspectorRow { visible: root.displayOf().space !== "Sdr"; label: "Peak brightness"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.displayOf().peak_nits; fallback: 1000; onCommitted: n => root.writeDisplay({ peak_nits: root.clamp(n, 100, 10000) }) }
                    Text { text: "nits"; color: Theme.textDim; font.pixelSize: 12 } }
                InspectorRow { visible: root.displayOf().space !== "Sdr"; label: "Paper white"; labelWidth: 110; Layout.fillWidth: true
                    NumberField { value: root.displayOf().paper_white_nits; fallback: 200; onCommitted: n => root.writeDisplay({ paper_white_nits: root.clamp(n, 80, root.displayOf().peak_nits) }) }
                    Text { text: "nits"; color: Theme.textDim; font.pixelSize: 12 } }
                Note { text: "HDR output reaches a built game's window where the display offers it, and falls back to SDR where it doesn't; the Game view is always SDR, so use its HDR preview and calibration views. Paper white is how bright white UI and a lit white wall look; peak brightness is where highlights roll off. Applies on the next run of the game." }
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
