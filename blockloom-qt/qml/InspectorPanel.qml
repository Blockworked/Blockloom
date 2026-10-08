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
    readonly property bool settingsVisible: app.inspectScene || !!app.inspectedLighting
    readonly property var actor: settingsVisible ? null : app.openActor
    readonly property string selectedActorId: appState.selected_actor || ""
    readonly property string projectPath: appState.project_path || ""
    onSelectedActorIdChanged: if (selectedActorId) { app.inspectScene = false; app.inspectedScene = ""; app.inspectedLighting = ""; }
    onProjectPathChanged: { app.inspectScene = false; app.inspectedScene = ""; app.inspectedLighting = ""; }
    readonly property string mode: appState.project ? appState.project.world.mode : "TwoD"
    readonly property bool is3d: mode === "ThreeD"
    // Where the actor is right now, while a run is going.
    readonly property var live: actor && app.status ? (app.status.actors.find(a => a.id === actor.id) || null) : null
    color: Theme.panel
    border.color: Theme.borderSoft
    clip: true

    function componentName(c) {
        if (c.component === "Plugin") return c.record.plugin + "/" + c.record.type_id;
        return c.component === "Custom" ? c.name : c.component;
    }
    // Plugin components are drawn from the schema their plugin declares.
    readonly property var pluginTypes: appState.plugins && appState.plugins.types ? appState.plugins.types : []
    function pluginType(name) { return pluginTypes.find(t => t.name === name) || null; }
    function componentTitle(c) {
        if (c.component === "Collider") return c.collider && c.collider.name ? "Collider: " + c.collider.name : "Collider";
        if (c.component === "Constraint") return c.constraint && c.constraint.name ? "Constraint: " + c.constraint.name : "Constraint: " + (c.constraint ? c.constraint.kind : "");
        if (c.component !== "Plugin") return componentName(c);
        const t = pluginType(componentName(c));
        return t ? t.displayName : c.record.type_id;
    }
    function writePlugin(c, payload) { if (actor) app.invoke("set_plugin_component", { actorId: actor.id, component: componentName(c), payload: payload }); }
    readonly property var actorOptions: appState.project ? [{ value: "", label: "nothing" }].concat(appState.project.actors.map(a => ({ value: a.id, label: a.name }))) : []
    function write(name, component) { if (actor) app.invoke("set_actor_component", { actorId: actor.id, name: name, component: component }); }
    function remove(name) { if (actor) app.invoke("remove_actor_component", { actorId: actor.id, name: name }); }
    // Physics components have their own commands: a collider is named by its id,
    // since an actor may carry several. Scripts are the same: one card edits
    // one path.
    function removeComponent(c) {
        if (!actor) return;
        if (c.component === "Collider") app.invoke("remove_collider", { colliderId: c.collider.id });
        else if (c.component === "Constraint") app.invoke("remove_constraint", { constraintId: c.constraint.id });
        else if (c.component === "Script") app.invoke("remove_script", { actorId: actor.id, path: c.path });
        else if (c.component === "Rigidbody") app.invoke("remove_rigidbody", { actorId: actor.id });
        else if (c.component === "CharacterController") app.invoke("remove_character_controller", { actorId: actor.id });
        else if (c.component === "CharacterMotor") app.invoke("remove_character_motor", { actorId: actor.id });
        else remove(componentName(c));
    }
    function motor2dDefaults() {
        const k = 48;
        return { space: "World", walk_speed: 5 * k, sprint_speed: 8 * k, crouch_speed: 2.5 * k, ground_acceleration: 50 * k, ground_braking: 60 * k,
                 air_acceleration: 15 * k, turn_speed: 0, terminal_fall_speed: 50 * k, ground_snap_distance: 0.3 * k, slide_speed: 6 * k,
                 jump_height: 1.2 * k, crouch_height: 32, external_drag: 20 * k };
    }
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
    function light2dOf(c) {
        const l = Object.assign({ kind: "Point", color: "#FFE2A8", intensity: 1, range: 240, falloff: 2, inner_angle: 25, outer_angle: 40, shadows: false, softness: 12, flicker: {} }, c.light2d || {});
        l.flicker = Object.assign({ amount: 0, speed: 8, seed: 1 }, l.flicker);
        return l;
    }
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
    // A 2D pool is in pixels; the backend fills in whatever a spec leaves out.
    function waterOf(c) { return c.water || {}; }
    function parallaxOf(c) { return Object.assign({ scroll: [0.5, 0.5], wrap: [false, false], dim: 0 }, c.parallax || {}); }
    function roomOf(c) { return Object.assign(is3d ? { size: [20, 10], depth: 20 } : { size: [1280, 720], depth: 720 }, { camera: true, blend: 0.4, stream: false }, c.room || {}); }
    function buoyancyOf(c) { return Object.assign({ density: 0.5, drag: 1, angular_drag: 1, points: 4, splash: true }, c.buoyancy || {}); }
    function trailOf(c) { return Object.assign({ interval: 0.05, life: 0.4, color: "#FFFFFF" }, c.trail || {}); }
    function jointOf(c) { return Object.assign({ target: "", kind: "Fixed", anchor: [0, 0, 0], length: 2 }, c.joint || {}); }
    function animationOf(c) { return Object.assign({ clips: [], states: [], initial: "", crossfade: 0, rig: "", skin: "", slot_tints: [] }, c.animation || {}); }
    function spriteOf(c) { return Object.assign({ flip_x: false, flip_y: false, order: 0, y_sort: false, slice: null, stack: null, palette: "", palette_index: 0, glow: 0, casts_shadow: false, outline_width: 0, outline_color: "#000000" }, c.sprite || {}); }
    // Transitions read and write as one line each: "run if speed > 2 blend 0.2",
    // "idle on end", "land on marker land", "jump on trigger jump".
    readonly property var compareSigns: ({ Less: "<", LessOrEqual: "<=", Equal: "=", NotEqual: "!=", Greater: ">", GreaterOrEqual: ">=" })
    function formatTransitions(list) {
        return (list || []).map(t => {
            const w = t.when || {};
            let line = t.to + " ";
            if (w.kind === "Marker") line += "on marker " + w.name;
            else if (w.kind === "Trigger") line += "on trigger " + w.name;
            else if (w.kind === "Variable") line += "if " + w.name + " " + compareSigns[w.compare || "Equal"] + " " + w.value;
            else line += "on end";
            if (t.blend !== undefined && t.blend >= 0) line += " blend " + t.blend;
            return line;
        }).join("; ");
    }
    function parseTransitions(text) {
        const ops = { "<": "Less", "<=": "LessOrEqual", "=": "Equal", "==": "Equal", "!=": "NotEqual", ">": "Greater", ">=": "GreaterOrEqual" };
        return text.split(/[;\n]+/).map(x => x.trim()).filter(x => x !== "").map(line => {
            let blend = -1;
            const b = line.match(/\s+blend\s+([0-9.]+)\s*$/i);
            if (b) { blend = parseFloat(b[1]); line = line.slice(0, b.index); }
            let m = line.match(/^(.+?)\s+on\s+end$/i);
            if (m) return { to: m[1].trim(), when: { kind: "Ended" }, blend: blend };
            m = line.match(/^(.+?)\s+on\s+(marker|trigger)\s+(.+)$/i);
            if (m) return { to: m[1].trim(), when: { kind: m[2].toLowerCase() === "marker" ? "Marker" : "Trigger", name: m[3].trim() }, blend: blend };
            m = line.match(/^(.+?)\s+if\s+(\S+?)\s*(<=|>=|!=|==|=|<|>)\s*(.+)$/i);
            if (m) return { to: m[1].trim(), when: { kind: "Variable", name: m[2], compare: ops[m[3]], value: m[4].trim() }, blend: blend };
            return null;
        }).filter(t => t !== null);
    }
    // "2:step, 5:land" <-> [{ frame: 2, name: "step" }, ...], frames from 0.
    function formatMarkers(list) { return (list || []).map(m => m.frame + ":" + m.name).join(", "); }
    function parseMarkers(text) {
        return text.split(/[,\n]+/).map(x => x.trim()).filter(x => x.indexOf(":") > 0)
            .map(x => ({ frame: Math.max(0, parseInt(x.slice(0, x.indexOf(":"))) || 0), name: x.slice(x.indexOf(":") + 1).trim() }))
            .filter(m => m.name !== "");
    }
    function parseNumbers(text) { return text.split(/[,\s]+/).filter(x => x !== "").map(x => Math.max(0, parseFloat(x) || 0)); }
    // "cape=#FF0000, hand=#00FF00" <-> [{ slot, color }].
    function formatTints(list) { return (list || []).map(t => t.slot + "=" + t.color).join(", "); }
    function parseTints(text) {
        return text.split(/[,\n]+/).map(x => x.trim()).filter(x => x.indexOf("=") > 0)
            .map(x => ({ slot: x.slice(0, x.indexOf("=")).trim(), color: x.slice(x.indexOf("=") + 1).trim() }));
    }
    function volumeOf(c) {
        const size = is3d ? 5 : 200;
        return Object.assign({ shape: "Box", half_extents: [size, size, size], radius: size, priority: 0, blend_distance: is3d ? 1 : 50, weight: 1, enabled: true, overrides: {} }, c.volume || {});
    }
    // What a volume property reads as before it is checked: the project's own.
    function projectValue(key) {
        const w = appState.project ? appState.project.world : {};
        const l = Object.assign({ light_direction: [8, 16, 8], light_color: "#FFFFFF", illuminance: 10000, ambient_color: "#FFFFFF", ambient_brightness: 80, ao_enabled: false }, w.lighting || {});
        const f = w.fog || { height: {}, volumetric: {}, aerial: {} };
        const p = Object.assign({ exposure_ev: 9.7, tonemapping: "TonyMcMapface", bloom_enabled: false, bloom_threshold: 1, bloom_intensity: 0.15, bloom_knee: 0.5, bloom_scatter: 0.7, bloom_dirt_intensity: 0,
                                  vignette_strength: 0, chromatic_aberration: 0, sharpen: 0 }, w.post || {});
        const ae = Object.assign({ enabled: false, min_ev: 2, max_ev: 16, compensation: 0 }, p.auto_exposure || {});
        const g = Object.assign({ temperature: 0, tint: 0, lift: [0, 0, 0], gamma: [1, 1, 1], gain: [1, 1, 1], saturation: 1, contrast: 1, lut_contribution: 1 }, p.grading || {});
        const dof = Object.assign({ enabled: false, focus_distance: 10, f_stops: 2.8 }, p.depth_of_field || {});
        const mb = Object.assign({ enabled: false, shutter_angle: 180 }, p.motion_blur || {});
        const ssr = Object.assign({ enabled: false, roughness_cutoff: 0.4 }, p.ssr || {});
        return ({ background: w.background || "#1B2431", sun_direction: l.light_direction, sun_color: l.light_color, illuminance: l.illuminance,
                  ambient_color: l.ambient_color, ambient_brightness: l.ambient_brightness, ao: l.ao_enabled, exposure: p.exposure_ev,
                  tonemapping: p.tonemapping, bloom: p.bloom_enabled, bloom_threshold: p.bloom_threshold, bloom_intensity: p.bloom_intensity,
                  vignette: p.vignette_strength, reflections: 1, indirect: 1,
                  auto_exposure: ae.enabled, auto_exposure_min: ae.min_ev, auto_exposure_max: ae.max_ev, exposure_compensation: ae.compensation,
                  bloom_knee: p.bloom_knee, bloom_scatter: p.bloom_scatter, bloom_dirt_intensity: p.bloom_dirt_intensity,
                  tone_toe: (p.tone || {}).toe || 0, tone_shoulder: (p.tone || {}).shoulder || 0,
                  temperature: g.temperature, tint: g.tint, lift: g.lift, gamma: g.gamma, gain: g.gain, saturation: g.saturation, contrast: g.contrast, lut_contribution: g.lut_contribution,
                  depth_of_field: dof.enabled, focus_distance: dof.focus_distance, f_stops: dof.f_stops, motion_blur: mb.enabled, shutter_angle: mb.shutter_angle,
                  ao_radius: (p.ao || {}).radius || 0.7285, ssr: ssr.enabled, ssr_roughness: ssr.roughness_cutoff,
                  chromatic_aberration: p.chromatic_aberration, grain: (p.grain || {}).intensity || 0, sharpen: p.sharpen,
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
        { key: "auto_exposure", label: "Auto exposure", kind: "bool", only3d: true },
        { key: "auto_exposure_min", label: "Auto EV min", kind: "number", only3d: true },
        { key: "auto_exposure_max", label: "Auto EV max", kind: "number", only3d: true },
        { key: "exposure_compensation", label: "Auto EV +", kind: "number", only3d: true },
        { key: "bloom_knee", label: "Bloom knee", kind: "number" },
        { key: "bloom_scatter", label: "Bloom spread", kind: "number" },
        { key: "bloom_dirt_intensity", label: "Lens dirt ×", kind: "number" },
        { key: "tone_toe", label: "Toe", kind: "number" },
        { key: "tone_shoulder", label: "Shoulder", kind: "number" },
        { key: "temperature", label: "Warmth", kind: "number" },
        { key: "tint", label: "Tint", kind: "number" },
        { key: "lift", label: "Lift RGB", kind: "vec3" },
        { key: "gamma", label: "Gamma RGB", kind: "vec3" },
        { key: "gain", label: "Gain RGB", kind: "vec3" },
        { key: "saturation", label: "Saturation", kind: "number" },
        { key: "contrast", label: "Contrast", kind: "number" },
        { key: "lut_contribution", label: "LUT amount", kind: "number" },
        { key: "depth_of_field", label: "Depth of field", kind: "bool", only3d: true },
        { key: "focus_distance", label: "Focus m", kind: "number", only3d: true },
        { key: "f_stops", label: "f-stop", kind: "number", only3d: true },
        { key: "motion_blur", label: "Motion blur", kind: "bool", only3d: true },
        { key: "shutter_angle", label: "Shutter °", kind: "number", only3d: true },
        { key: "ao_radius", label: "AO reach m", kind: "number", only3d: true },
        { key: "ssr", label: "SSR", kind: "bool", only3d: true },
        { key: "ssr_roughness", label: "SSR rough cutoff", kind: "number", only3d: true },
        { key: "chromatic_aberration", label: "Color fringes", kind: "number" },
        { key: "grain", label: "Film grain", kind: "number" },
        { key: "sharpen", label: "Sharpen", kind: "number" },
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
        return Object.assign({ tileset: "", tile_size: [32, 32], width: 8, height: 8, sheet_columns: 4, sheet_rows: 4, tiles: [], solid: false, passable: [], animations: [], autotiles: [], regions: [] }, v && v.tilemap ? v.tilemap : {});
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
    function writeLight2d(c, next) { write("Light2d", { component: "Light2d", light2d: merged(light2dOf(c), next) }); }
    function writeBeam(c, next) { writeLight(c, { beam: merged(beamOf(lightOf(c)), next) }); }
    function writeMotes(c, next) { writeBeam(c, { motes: merged(beamOf(lightOf(c)).motes, next) }); }
    function writeTrail(c, next) { write("Trail", { component: "Trail", trail: merged(trailOf(c), next) }); }
    function writeJoint(c, next) { write("Joint", { component: "Joint", joint: merged(jointOf(c), next) }); }
    function writeAnimation(c, next) { write("Animation", { component: "Animation", animation: merged(animationOf(c), next) }); }
    function writeSprite(c, next) { write("Sprite", { component: "Sprite", sprite: merged(spriteOf(c), next) }); }
    function writeClip(an, index, next) { const l = copy(an.a.clips); l[index] = merged(l[index], next); writeAnimation(an.c, { clips: l }); }
    function writeState(an, index, next) { const l = copy(an.a.states); l[index] = merged(l[index], next); writeAnimation(an.c, { states: l }); }
    function writeProbe(c, next) { write("Probe", { component: "Probe", probe: merged(probeOf(c), next) }); }
    function writeTerrain(c, next) { write("Terrain", { component: "Terrain", terrain: merged(terrainOf(c), next) }); }
    function writeTerrainItem(c, list, index, next) {
        const l = copy(terrainOf(c)[list]);
        l[index] = Object.assign(l[index], next);
        const change = {}; change[list] = l;
        writeTerrain(c, change);
    }
    function writeWater(c, next) { write("Water", { component: "Water", water: merged(waterOf(c), next) }); }
    function writeWaterPart(c, part, next) { const change = {}; change[part] = merged(waterOf(c)[part] || {}, next); writeWater(c, change); }
    function writeParallax(c, next) { write("Parallax", { component: "Parallax", parallax: merged(parallaxOf(c), next) }); }
    function writeRoom(c, next) { write("Room", { component: "Room", room: merged(roomOf(c), next) }); }
    function writeRegion(c, index, next) {
        const list = copy(tilemapOf(c.visual).regions);
        if (next === null) list.splice(index, 1); else list[index] = Object.assign(list[index], next);
        writeTilemap(c, { regions: list });
    }
    // The selected tilemap's make-up, fetched whenever its look changes.
    property var tileStats: null
    function refreshTileStats() {
        const look = actor ? actor.components.find(x => x.component === "Look") : null;
        if (!look || !look.visual || look.visual.shape !== "Tilemap") { tileStats = null; return; }
        app.invoke("tilemap_stats", { actorId: actor.id }, r => tileStats = r, () => tileStats = null);
    }
    onActorChanged: Qt.callLater(refreshTileStats)
    function writeBuoyancy(c, next) { write("Buoyancy", { component: "Buoyancy", buoyancy: merged(buoyancyOf(c), next) }); }
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
        return ["Look","Render","Body","Rigidbody","Collider","Constraint","CharacterController","CharacterMotor","PlayerCamera","Joint","Brain","Camera","Script","Parent","Material","Emitter","Trail","Light","Light2d","Animation","Sprite","Volume","Probe","Terrain","Fracture","Water","Buoyancy","Parallax","Room","Persist","Custom"]
            .filter(n => n !== "Sprite" || !is3d)
            .filter(n => n !== "Fracture" || is3d)
            .filter(n => n !== "Light2d" || !is3d)
            .filter(n => n === "Custom" || n === "Collider" || n === "Constraint" || n === "Script" || held.indexOf(n) < 0).map(n => ({ value: n, label: n === "Custom" ? "Custom…" : (n === "Light2d" ? "2D light" : n) }))
            .concat(pluginTypes.filter(t => t.kind === "component" && held.indexOf(t.name) < 0).map(t => ({ value: "plugin:" + t.name, label: t.displayName + " (" + t.pluginName + ")" })));
    }
    function blank(name) {
        switch (name) {
        case "Look": return { component: "Look", visual: is3d ? { shape: "Cuboid", color: "#4C97FF", size: [1, 1, 1] } : { shape: "Rect", color: "#4C97FF", size: [60, 60] } };
        case "Render": return { component: "Render", visible: true, layer: 0 };
        case "Body": return { component: "Body", physics: { body: "Dynamic", gravity_scale: 1, lock_rotation: false, restitution: 0, friction: 0.5, density: 1, mass: null, trigger: false, collision_layer: 1, collision_mask: 255 } };
        case "Joint": return { component: "Joint", joint: jointOf({}) };
        case "Brain": return { component: "Brain", brain: brainOf({}) };
        case "PlayerCamera": return { component: "PlayerCamera", player_camera: is3d ? {} : { look: false, collision: false, smoothing: 0.15, dead_zone: [48, 32], look_ahead: 64 } };
        case "Camera": return { component: "Camera", camera: { view: "ThirdPerson", offset: [0, 0.6, 0], distance: 6, pitch: 15, fov: 75 } };
        case "Parent": return { component: "Parent", parent: "", offset: null };
        case "Material": return { component: "Material", material: materialOf({}) };
        case "Emitter": return { component: "Emitter", emitter: emitterOf({}) };
        case "Trail": return { component: "Trail", trail: trailOf({}) };
        case "Light": return { component: "Light", light: lightOf({}) };
        case "Light2d": return { component: "Light2d", light2d: light2dOf({}) };
        case "Animation": return { component: "Animation", animation: { clips: [], states: [] } };
        case "Sprite": return { component: "Sprite", sprite: spriteOf({}) };
        case "Volume": return { component: "Volume", volume: volumeOf({}) };
        case "Probe": return { component: "Probe", probe: probeOf({}) };
        case "Terrain": return { component: "Terrain", terrain: terrainOf({}) };
        case "Fracture": return { component:"Fracture", fracture: { cells:16,seed:1,interior:{},impulse_threshold:8,lifetime:20,sleep_seconds:2,pool_cap:256,bounce_sound:"" } };
        case "Water": return { component: "Water", water: is3d ? {} : { size: [800, 300], depth: 300,
            waves: { amplitude: 6, wavelength: 220, direction: 90, spread: 20 }, detail: { scale: 60 }, look: { absorption: 220 },
            foam: { shore: 12, scale: 40 }, underwater: { distance: 600, caustics_scale: 60 }, splash: { min_speed: 60 }, ripples: { speed: 120, extent: 1600 } } };
        case "Buoyancy": return { component: "Buoyancy", buoyancy: buoyancyOf({}) };
        case "Parallax": return { component: "Parallax", parallax: parallaxOf({}) };
        case "Room": return { component: "Room", room: roomOf({}) };
        case "Persist": return { component: "Persist" };
        case "Custom": return { component: "Custom", name: "Component", fields: [{ name: "value", value: { kind: "Number", value: 0 } }] };
        default: return null;
        }
    }
    function add(name) {
        if (!actor) return;
        // A script needs a file on disk, so the backend makes both at once.
        // The first one uses the legacy entry point; further ones append.
        if (name === "Script") {
            const hasScript = actor.components.some(c => c.component === "Script");
            app.invoke(hasScript ? "add_script" : "create_script", { actorId: actor.id });
            return;
        }
        if (name === "CharacterMotor") { app.invoke("set_character_motor", { actorId: actor.id, motor: is3d ? {} : motor2dDefaults() }); return; }
        if (name === "CharacterController") { app.invoke("set_character_controller", { actorId: actor.id, controller: is3d ? {} : { radius: 16, height: 64, step_offset: 12, skin_width: 2, min_move_distance: 0.05 } }); return; }
        if (name === "Rigidbody") { app.invoke("set_rigidbody", { actorId: actor.id, rigidbody: { body_type: "Dynamic" } }); return; }
        if (name === "Collider") {
            app.invoke("add_collider", { actorId: actor.id, collider: { geometry: { kind: "Shape", shape: is3d ? { kind: "Box", size: [1, 1, 1] } : { kind: "Rect", size: [60, 60] } } } });
            return;
        }
        if (name === "Constraint") {
            app.invoke("add_constraint", { actorId: actor.id, constraint: is3d ? { kind: "Hinge", axis: [0, 0, 1], connected_axis: [0, 0, 1] } : { kind: "Hinge" } });
            return;
        }
        if (name.indexOf("plugin:") === 0) { app.invoke("add_plugin_component", { actorId: actor.id, component: name.slice(7) }); return; }
        const c = blank(name);
        if (c) app.invoke("add_actor_component", { actorId: actor.id, component: c });
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        visible: root.open
        RowLayout {
            visible: root.settingsVisible; Layout.fillWidth: true; Layout.margins: 8
            SectionLabel {
                Layout.fillWidth: true; topPadding: 0; elide: Text.ElideRight
                label: {
                    if (root.app.inspectedLighting) return root.app.inspectedLighting.split("/").pop().replace(/\.blocklighting$/i, "");
                    const project = root.appState.project;
                    const scene = project ? project.scenes.find(s => s.id === (root.app.inspectedScene || project.active_scene)) : null;
                    return scene ? scene.name : "Scene";
                }
            }
            IconButton { iconName: "chevron-right"; tip: "Hide the components"; implicitWidth: 26; implicitHeight: 26; onClicked: root.openRequested(false) }
        }
        SettingsFields {
            visible: root.settingsVisible; Layout.fillWidth: true; Layout.fillHeight: true
            app: root.app; page: root.app.inspectedLighting ? "lighting" : "scene"
            sceneId: root.app.inspectedScene
            lightingPath: root.app.inspectedLighting
        }
        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; spacing: 0
            visible: !root.settingsVisible && !!root.actor
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
                    PlayerSetupCard {
                        Layout.fillWidth: true
                        app: root.app; actorId: root.actor ? root.actor.id : ""; is3d: root.is3d
                        isPlayer: !!root.actor && root.actor.components.some(c => c.component === "CharacterController")
                    }
                    // A count rather than the array: a new snapshot with the same
                    // components updates the cards in place instead of rebuilding them.
                    Repeater {
                        model: root.actor ? root.actor.components.length : 0
                        delegate: InspectorComponentCard {
                            id: card
                            required property int index
                            readonly property var c: root.actor && root.actor.components[index] ? root.actor.components[index] : ({ component: "" })
                            Layout.fillWidth: true; spacing: 6
                            heading: root.componentTitle(card.c)
                            removable: card.c.component !== "Place"
                            onRemoveRequested: root.removeComponent(card.c)
                            Loader {
                                Layout.fillWidth: true
                                readonly property var c: card.c
                                sourceComponent: ({ Place: placeCard, Look: lookCard, Parent: parentCard, Render: renderCard, Body: bodyCard, Rigidbody: rigidbodyCard, Collider: colliderCard, Constraint: constraintCard, CharacterController: controllerCard, CharacterMotor: motorCard, PlayerCamera: playerCameraCard, Joint: jointCard, Brain: brainCard, Camera: cameraCard,
                                                    Script: scriptCard, Custom: customCard, Material: materialCard, Emitter: emitterCard, Trail: trailCard, Light: lightCard, Light2d: light2dCard, Animation: animationCard, Sprite: spriteCard, Volume: volumeCard, Probe: probeCard, Terrain: terrainCard, Fracture: fractureCard, Water: waterCard, Buoyancy: buoyancyCard, Parallax: parallaxCard, Room: roomCard, Persist: persistCard, Plugin: pluginCard })[card.c.component] || null
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
    }
    Text {
        visible: root.open && !root.actor && !root.settingsVisible
        anchors.centerIn: parent; width: parent.width - 32; wrapMode: Text.WordWrap; horizontalAlignment: Text.AlignHCenter
        text: "Select an actor to see its components."; color: Theme.textDim; font.pixelSize: 12
    }

    // ─── One card per component kind. `parent.c` is the component. ─────────
    Component {
        id: fractureCard
        ColumnLayout {
            id: fr
            readonly property var c: parent.c
            readonly property var f: Object.assign({ cells:16,seed:1,interior:{},impulse_threshold:8,lifetime:20,sleep_seconds:2,pool_cap:256,bounce_sound:"" }, c.fracture || {})
            function write(next) { root.write("Fracture", { component:"Fracture", fracture:root.merged(f,next) }); }
            function interior(next) { write({ interior:root.merged(f.interior,next) }); }
            spacing: 6
            InspectorRow { label:"Voronoi cells"; Layout.fillWidth:true
                NumberField { value:fr.f.cells; onCommitted:n => fr.write({cells:Math.round(Math.max(2,Math.min(64,n)))}) } }
            InspectorRow { label:"Seed"; Layout.fillWidth:true
                NumberField { value:fr.f.seed; onCommitted:n => fr.write({seed:Math.round(Math.max(0,n))}) } }
            InspectorRow { label:"Hit impulse"; Layout.fillWidth:true
                NumberField { value:fr.f.impulse_threshold; onCommitted:n => fr.write({impulse_threshold:Math.max(0,n)}) } }
            InspectorRow { label:"Lifetime s"; Layout.fillWidth:true
                NumberField { value:fr.f.lifetime; onCommitted:n => fr.write({lifetime:Math.max(0.1,n)}) } }
            InspectorRow { label:"Sleep retire s"; Layout.fillWidth:true
                NumberField { value:fr.f.sleep_seconds; onCommitted:n => fr.write({sleep_seconds:Math.max(0.1,n)}) } }
            InspectorRow { label:"Pool cap"; Layout.fillWidth:true
                NumberField { value:fr.f.pool_cap; onCommitted:n => fr.write({pool_cap:Math.round(Math.max(1,Math.min(256,n)))}) } }
            InspectorRow { label:"Cap roughness"; Layout.fillWidth:true
                NumberField { value:fr.f.interior.roughness === undefined ? 0.5 : fr.f.interior.roughness; onCommitted:n => fr.interior({roughness:Math.max(0,Math.min(1,n))}) } }
            InspectorRow { label:"Cap metallic"; Layout.fillWidth:true
                NumberField { value:fr.f.interior.metallic || 0; onCommitted:n => fr.interior({metallic:Math.max(0,Math.min(1,n))}) } }
            InspectorRow { label:"Cap albedo"; Layout.fillWidth:true
                AssetField { app:root.app; accept:["image"]; value:fr.f.interior.albedo_texture || ""; onCommitted:p => fr.interior({albedo_texture:p}) } }
            InspectorRow { label:"Cap normal"; Layout.fillWidth:true
                AssetField { app:root.app; accept:["image"]; value:fr.f.interior.normal_texture || ""; onCommitted:p => fr.interior({normal_texture:p}) } }
            InspectorRow { label:"Bounce sound"; Layout.fillWidth:true
                AssetField { app:root.app; accept:["audio"]; value:fr.f.bounce_sound; onCommitted:p => fr.write({bounce_sound:p}) } }
            Text { Layout.fillWidth:true; wrapMode:Text.WordWrap; color:Theme.textDim; font.pixelSize:11
                text:"Convex 3D primitives break on a hard hit or the fracture block. Debris inherits motion and retires on sleep or timeout. The authored actor returns on Play." }
        }
    }
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
                Text { visible: !!root.tileStats; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                    text: root.tileStats ? root.tileStats.tiles + " tiles, " + root.tileStats.draw_batches + " draw batch" + (root.tileStats.draw_batches === 1 ? "" : "es")
                        + ", " + root.tileStats.colliding_rects + " colliding rects" + (root.tileStats.animated ? ", " + root.tileStats.animated + " animated" : "")
                        + (root.tileStats.region_tiles ? ", " + root.tileStats.region_tiles + " in regions" : "") : "" }
                InspectorRow { label: "Import Tiled"; Layout.fillWidth: true
                    AssetField { app: root.app; assetKind: "text"; value: ""; placeholderText: "Drag a .tsj tileset here"
                        onCommitted: p => { if (p.trim() === "") return;
                            root.app.invoke("import_tileset", { actorId: root.actor.id, path: p.trim() },
                                skipped => { text = ""; if (skipped && skipped.length) root.app.invoke("push_log", { kind: "warning", text: "Tileset import skipped: " + skipped.join("; ") }); },
                                e => root.app.invoke("push_log", { kind: "error", text: String(e) })); } } }
                // Autotile sets: one row each, and a row to add a strip.
                Repeater {
                    model: look.t.autotiles.length
                    delegate: InspectorRow {
                        required property int index
                        readonly property var set: look.t.autotiles[index] || { name: "", mode: "Edge", rules: [] }
                        label: "Autotile"; Layout.fillWidth: true
                        Text { Layout.fillWidth: true; text: set.name + "  (" + set.mode + ", " + set.rules.length + " cases)"; color: Theme.text; font.pixelSize: 12; elide: Text.ElideRight }
                        IconButton { iconName: "x"; tip: "Remove this autotile set"; implicitWidth: 24; implicitHeight: 24
                            onClicked: { const list = root.copy(look.t.autotiles); list.splice(index, 1); root.writeTilemap(look.c, { autotiles: list }); } }
                    }
                }
                InspectorRow { id: autotileRow; label: "Add autotile"; Layout.fillWidth: true
                    property string mode: "Edge"
                    BwTextField { id: autotileName; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Name, e.g. ground" }
                    ChoiceField { Layout.maximumWidth: 70; options: [{ value: "Edge", label: "16" }, { value: "Blob", label: "47" }]; value: autotileRow.mode; onChosen: v => autotileRow.mode = v
                        ToolTip.visible: hovered; ToolTip.text: "16 edge cases, or 47 blob cases with corners" }
                    NumberField { id: autotileFirst; Layout.maximumWidth: 48; value: 0; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "The strip's first sheet tile" }
                    IconButton { iconName: "plus"; tip: "Add the set: consecutive sheet tiles from the first"; implicitWidth: 24; implicitHeight: 24
                        onClicked: root.app.invoke("add_autotile", { actorId: root.actor.id, name: autotileName.text, mode: autotileRow.mode, first: Math.max(0, Math.round(autotileFirst.value)) },
                            () => autotileName.text = "", e => root.app.invoke("push_log", { kind: "error", text: String(e) })) }
                }
                // Region tiles: spawn, checkpoint, kill, ladder, water.
                Repeater {
                    model: look.t.regions.length
                    delegate: InspectorRow {
                        required property int index
                        readonly property var region: look.t.regions[index] || { tile: 0, kind: "Spawn" }
                        label: "Region"; Layout.fillWidth: true
                        NumberField { Layout.maximumWidth: 48; value: region.tile; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "The sheet tile that marks it"
                            onCommitted: n => root.writeRegion(look.c, index, { tile: Math.max(0, Math.round(n)) }) }
                        ChoiceField { options: Blocks.opts(["Spawn", "Checkpoint", "Kill", "Ladder", "Water"]); value: region.kind; onChosen: v => root.writeRegion(look.c, index, { kind: v }) }
                        IconButton { iconName: "x"; tip: "Stop marking this tile"; implicitWidth: 24; implicitHeight: 24; onClicked: root.writeRegion(look.c, index, null) }
                    }
                }
                BwButton {
                    iconName: "plus"; text: "Mark a region tile"; implicitHeight: 28; font.pixelSize: 12
                    onClicked: {
                        const list = root.copy(look.t.regions);
                        list.push({ tile: Math.max(0, root.paintTile), kind: "Kill" });
                        root.writeTilemap(look.c, { regions: list });
                    }
                }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                    text: "Paint in the Game view with the Tiles tool (T). Moving bodies respawn at their last checkpoint (or the spawn) on a kill tile, lose gravity on a ladder, and float and drag in water." }
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
            PhysicsUpgradeCard { Layout.fillWidth: true; app: root.app; actorId: root.actor ? root.actor.id : "" }
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
        id: rigidbodyCard
        RigidbodyForm {
            readonly property var c: parent.c
            app: root.app; actorId: root.actor ? root.actor.id : ""; component: c; is3d: root.is3d
        }
    }
    Component {
        id: controllerCard
        CharacterControllerForm {
            readonly property var c: parent.c
            app: root.app; actorId: root.actor ? root.actor.id : ""; component: c; is3d: root.is3d
        }
    }
    Component {
        id: playerCameraCard
        PlayerCameraForm {
            readonly property var c: parent.c
            app: root.app; actorId: root.actor ? root.actor.id : ""; component: c; is3d: root.is3d
        }
    }
    Component {
        id: motorCard
        CharacterMotorForm {
            readonly property var c: parent.c
            app: root.app; actorId: root.actor ? root.actor.id : ""; component: c; is3d: root.is3d
        }
    }
    Component {
        id: constraintCard
        ConstraintForm {
            readonly property var c: parent.c
            app: root.app; actorId: root.actor ? root.actor.id : ""; component: c; is3d: root.is3d
        }
    }
    Component {
        id: colliderCard
        ColliderForm {
            readonly property var c: parent.c
            app: root.app; component: c; is3d: root.is3d
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
            readonly property var scriptStatus: (root.appState.script_statuses || {})[c.path] || null
            // Position among this actor's scripts: the order `start` runs.
            readonly property int scriptIndex: root.actor ? root.actor.components.filter(x => x.component === "Script").findIndex(x => x.path === c.path) : -1
            readonly property int scriptCount: root.actor ? root.actor.components.filter(x => x.component === "Script").length : 0
            spacing: 6
            Text {
                visible: true
                Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText
                color: scriptStatus && scriptStatus.error ? Theme.danger : Theme.textDim
                text: !scriptStatus ? "Not built yet. Check the script or press Play." : scriptStatus.stage === "build_failed" ? "Build failed: " + scriptStatus.error
                    : scriptStatus.stage === "load_failed" ? "Load failed: " + scriptStatus.error
                    : scriptStatus.stage === "loaded" ? "Script loaded" : "Script compiled"
            }
            AssetField { app: root.app; accept: ["script"]; value: c.path; Layout.fillWidth: true; onCommitted: p => root.write(c.path, { component: "Script", path: p }) }
            RowLayout {
                BwButton { text: "Edit"; iconName: "file-code"; implicitHeight: 30; onClicked: scriptDialog.openFor(root.actor, c.path) }
                BwButton { text: "Check"; implicitHeight: 30; onClicked: root.app.invoke("check_script", { actorId: root.actor.id, path: c.path }) }
                BwButton { visible: scriptIndex > 0; text: "Up"; implicitHeight: 30; onClicked: root.app.invoke("move_script", { actorId: root.actor.id, path: c.path, index: scriptIndex - 1 }) }
                BwButton { visible: scriptIndex >= 0 && scriptIndex < scriptCount - 1; text: "Down"; implicitHeight: 30; onClicked: root.app.invoke("move_script", { actorId: root.actor.id, path: c.path, index: scriptIndex + 1 }) }
            }
            Text { visible: scriptCount > 1; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Script " + (scriptIndex + 1) + " of " + scriptCount + ": `start` runs top to bottom." }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Real Rust, compiled when you press Play. It runs alongside this actor's blocks, not instead of them." }
        }
    }
    // Opens a script file from outside the inspector (a run-log line): finds
    // the actor running it and hands both to the script dialog, on its line.
    function openScript(path, line) {
        if (!appState.project) return;
        const owner = appState.project.actors.find(a => (a.components || []).some(c => c.component === "Script" && c.path === path));
        if (owner) scriptDialog.openFor(owner, path, line || 0);
    }
    // The QML section the plugin draws for one of its component types, from the
    // trusted editor modules: {file} once trusted, {file: null} until then, or
    // null when the plugin draws none.
    function pluginSection(plugin, typeId) {
        const shipped = appState.plugins && appState.plugins.editorModules ? appState.plugins.editorModules : [];
        const owner = shipped.find(p => p.plugin === plugin);
        const section = owner && owner.inspectors ? owner.inspectors.find(i => i.component === typeId) : null;
        return section ? { file: section.file || null, pluginName: owner.pluginName } : null;
    }

    // A plugin's component: a form from its schema, or a note that keeps the
    // data safe while the plugin is missing. A trusted plugin may draw its own
    // section instead, with the generated form one click away.
    Component {
        id: pluginCard
        ColumnLayout {
            id: plug
            readonly property var c: parent.c
            readonly property var t: root.pluginType(root.componentName(c))
            readonly property var section: root.pluginSection(c.record.plugin, c.record.type_id)
            readonly property bool drawn: !!section && !!section.file && !!t && !sectionFailed
            property bool sectionFailed: false
            property bool rawFields: false
            readonly property bool formShown: !!t && c.record.schema_version === t.version && (!drawn || rawFields)
            spacing: 6
            // What the plugin's section is handed as `host`.
            QtObject {
                id: sectionHost
                readonly property string plugin: plug.c.record.plugin
                readonly property string component: plug.c.record.type_id
                readonly property string actorId: root.actor ? root.actor.id : ""
                readonly property var payload: plug.c.record.payload
                readonly property var type: plug.t
                readonly property var app: root.app
                readonly property var project: root.appState.project
                function write(next) { root.writePlugin(plug.c, next); }
                function call(command, args, done, failed) {
                    root.app.invoke("plugin_call", { command: plugin + "/" + command, args: args || ({}) }, done, failed);
                }
            }
            Text {
                visible: !plug.t
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "The plugin " + plug.c.record.plugin + " is not installed here, or no longer has " + plug.c.record.type_id + ". The data is kept as it is; install the plugin from the Plugins dialog to edit it."
            }
            Text {
                visible: !!plug.t && plug.c.record.schema_version !== plug.t.version
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.warning; font.pixelSize: 12
                text: "Written at schema " + plug.c.record.schema_version + ", the plugin is at " + (plug.t ? plug.t.version : 0) + ". Migrate it from the Plugins dialog before running."
            }
            Text {
                visible: !!plug.section && !plug.section.file
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: plug.section ? plug.section.pluginName + " draws its own section for this. Trust it in Plugin editors to use it." : ""
            }
            Loader {
                id: sectionLoader
                objectName: "plugin-section"
                Layout.fillWidth: true
                active: plug.drawn && !!plug.t && plug.c.record.schema_version === plug.t.version
                visible: active && status === Loader.Ready
                onActiveChanged: if (active) setSource(root.app.toFileUrl(plug.section.file), { host: sectionHost })
                Component.onCompleted: if (active) setSource(root.app.toFileUrl(plug.section.file), { host: sectionHost })
                onStatusChanged: if (status === Loader.Error) plug.sectionFailed = true
            }
            BwButton {
                visible: plug.drawn && !!plug.t && plug.c.record.schema_version === plug.t.version
                implicitHeight: 24; font.pixelSize: 11
                text: plug.rawFields ? "Hide fields" : "Fields"
                onClicked: plug.rawFields = !plug.rawFields
            }
            PluginRecordForm {
                objectName: "plugin-fields"
                visible: plug.formShown
                app: root.app; type: plug.t || ({ fields: [], defaults: {} }); payload: plug.c.record.payload; actors: root.actorOptions
                onChanged: next => root.writePlugin(plug.c, next)
            }
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
                HdrColorField { color: mat.m.emissive; intensity: mat.m.emissive_energy; onPicked: (col, n) => root.writeMaterial(mat.c, { emissive: col, emissive_energy: n }) } }
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
            EmitterGraphRows { Layout.fillWidth: true; app: root.app; is3d: root.is3d; emitter: em.e
                onEdited: next => root.writeEmitter(em.c, next) }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Plays in the scene view while this actor is selected, looping every Duration. In a run it sprays while attached; detaching stops the spray and what is already flying fades out on its own." }
        }
    }
    Component {
        id: light2dCard
        ColumnLayout {
            id: l2
            readonly property var c: parent.c
            readonly property var l: root.light2dOf(c)
            spacing: 6
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Lights the 2D world once Lighting is on in Project Settings. Range and softness are in pixels." }
            InspectorRow { label: "Kind"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Point", label: "Point" }, { value: "Spot", label: "Spot" }]; value: l2.l.kind; onChosen: k => root.writeLight2d(l2.c, { kind: k }) } }
            InspectorRow { label: "Color"; Layout.fillWidth: true; ColorField { value: l2.l.color; onPicked: col => root.writeLight2d(l2.c, { color: col }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Intensity"; Layout.fillWidth: true
                NumberField { value: l2.l.intensity; fallback: 1; onCommitted: n => root.writeLight2d(l2.c, { intensity: Math.max(0, n) }) } }
            InspectorRow { label: "Range"; Layout.fillWidth: true
                NumberField { value: l2.l.range; fallback: 240; onCommitted: n => root.writeLight2d(l2.c, { range: Math.max(1, n) }) } }
            InspectorRow { label: "Falloff"; Layout.fillWidth: true
                NumberField { value: l2.l.falloff; fallback: 2; onCommitted: n => root.writeLight2d(l2.c, { falloff: Math.min(Math.max(n, 0.1), 8) }) } }
            InspectorRow { visible: l2.l.kind === "Spot"; label: "Cone °"; Layout.fillWidth: true
                NumberField { value: l2.l.inner_angle; fallback: 25; onCommitted: n => root.writeLight2d(l2.c, { inner_angle: n }) }
                NumberField { value: l2.l.outer_angle; fallback: 40; onCommitted: n => root.writeLight2d(l2.c, { outer_angle: n }) } }
            InspectorRow { label: "Shadows"; Layout.fillWidth: true
                SwitchField { value: l2.l.shadows; onToggled: on => root.writeLight2d(l2.c, { shadows: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: l2.l.shadows; label: "Softness"; Layout.fillWidth: true
                NumberField { value: l2.l.softness; fallback: 12; onCommitted: n => root.writeLight2d(l2.c, { softness: Math.max(0, n) }) } }
            InspectorRow { label: "Flicker"; Layout.fillWidth: true
                NumberField { value: l2.l.flicker.amount; fallback: 0; onCommitted: n => root.writeLight2d(l2.c, { flicker: Object.assign({}, l2.l.flicker, { amount: Math.min(Math.max(n, 0), 1) }) }) }
                NumberField { value: l2.l.flicker.speed; fallback: 8; onCommitted: n => root.writeLight2d(l2.c, { flicker: Object.assign({}, l2.l.flicker, { speed: Math.max(0, n) }) }) } }
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
                    ? "Candela down the brightest direction (an IES profile's peak). About " + Math.round(li.l.intensity * 4 * Math.PI) + " lumens. Enable contact shadows in the Lighting asset too."
                    : "About " + Math.round(li.l.intensity / (4 * Math.PI)) + " candela. A spot's cone doesn't gather the light, so narrowing it isn't brighter. Enable contact shadows in the Lighting asset too." }
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
                AssetField { app: root.app; assetKind: "height"; value: ""; placeholderText: "Import PNG, .r16 or .r32"
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
        id: waterCard
        ColumnLayout {
            id: wa
            readonly property var c: parent.c
            readonly property var w: root.waterOf(c)
            readonly property var waves: w.waves || {}
            readonly property var look: w.look || {}
            readonly property var foam: w.foam || {}
            readonly property var refl: w.reflections || {}
            readonly property var under: w.underwater || {}
            readonly property var splash: w.splash || {}
            readonly property var rip: w.ripples || {}
            readonly property bool ocean: w.kind === "Ocean"
            function part(name, next) { root.writeWaterPart(wa.c, name, next); }
            function clamp(n, lo, hi) { return Math.min(hi, Math.max(lo, n)); }
            spacing: 6
            InspectorRow { label: "Kind"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Lake", label: "Lake" }, { value: "River", label: "River" }, { value: "Ocean", label: "Ocean" }]; value: wa.w.kind || "Lake"; onChosen: k => root.writeWater(wa.c, { kind: k }) } }
            InspectorRow { visible: !wa.ocean; label: root.is3d ? "Size x, z" : "Width"; Layout.fillWidth: true
                NumberField { value: (wa.w.size || [40, 40])[0]; fallback: 40; onCommitted: n => root.writeWater(wa.c, { size: root.withIndex(wa.w.size || [40, 40], 0, Math.max(0.01, n)) }) }
                NumberField { visible: root.is3d; value: (wa.w.size || [40, 40])[1]; fallback: 40; onCommitted: n => root.writeWater(wa.c, { size: root.withIndex(wa.w.size || [40, 40], 1, Math.max(0.01, n)) }) } }
            InspectorRow { label: "Depth"; Layout.fillWidth: true
                NumberField { value: wa.w.depth; fallback: 8; onCommitted: n => root.writeWater(wa.c, { depth: Math.max(0, n) }) } }
            InspectorRow { visible: wa.w.kind === "River"; label: "Flow x, z"; Layout.fillWidth: true
                NumberField { value: (wa.w.flow || [0, 0])[0]; fallback: 0; onCommitted: n => root.writeWater(wa.c, { flow: root.withIndex(wa.w.flow || [0, 0], 0, n) }) }
                NumberField { visible: root.is3d; value: (wa.w.flow || [0, 0])[1]; fallback: 0; onCommitted: n => root.writeWater(wa.c, { flow: root.withIndex(wa.w.flow || [0, 0], 1, n) }) } }

            Text { text: "Waves"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { label: "Height, length"; Layout.fillWidth: true
                NumberField { value: wa.waves.amplitude; fallback: 0.25; onCommitted: n => wa.part("waves", { amplitude: Math.max(0, n) }) }
                NumberField { value: wa.waves.wavelength; fallback: 12; onCommitted: n => wa.part("waves", { wavelength: Math.max(0.01, n) }) } }
            InspectorRow { label: "Steepness"; Layout.fillWidth: true
                SliderField { from: 0; to: 1; value: wa.waves.steepness; onMoved: wa.part("waves", { steepness: value }) } }
            InspectorRow { label: "Chop"; Layout.fillWidth: true
                SliderField { from: 0; to: 1; value: wa.waves.chop; onMoved: wa.part("waves", { chop: value }) } }
            InspectorRow { label: "Waves, speed"; Layout.fillWidth: true
                NumberField { value: wa.waves.count; fallback: 10; onCommitted: n => wa.part("waves", { count: wa.clamp(Math.round(n), 1, 12) }) }
                NumberField { value: wa.waves.speed; fallback: 1; onCommitted: n => wa.part("waves", { speed: wa.clamp(n, 0, 10) }) } }
            InspectorRow { label: "Follow wind"; Layout.fillWidth: true
                SwitchField { value: wa.waves.follow_wind; onToggled: on => wa.part("waves", { follow_wind: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: !wa.waves.follow_wind; label: "Heading"; Layout.fillWidth: true
                NumberField { value: wa.waves.direction; fallback: 45; onCommitted: n => wa.part("waves", { direction: ((n % 360) + 360) % 360 }) } }
            InspectorRow { visible: wa.waves.follow_wind; label: "Turn seconds"; Layout.fillWidth: true
                NumberField { value: wa.waves.turn; fallback: 6; onCommitted: n => wa.part("waves", { turn: wa.clamp(n, 0, 120) }) } }
            InspectorRow { label: "Spread"; Layout.fillWidth: true
                NumberField { value: wa.waves.spread; fallback: 40; onCommitted: n => wa.part("waves", { spread: wa.clamp(n, 0, 180) }) } }
            InspectorRow { label: "Wind, fetch km"; Layout.fillWidth: true
                NumberField { value: wa.waves.wind; fallback: 0.5; onCommitted: n => wa.part("waves", { wind: wa.clamp(n, 0, 1) }) }
                NumberField { value: wa.waves.fetch; fallback: 2; onCommitted: n => wa.part("waves", { fetch: wa.clamp(n, 0.01, 5000) }) } }
            InspectorRow { label: "Detail, scale"; Layout.fillWidth: true
                NumberField { value: (wa.w.detail || {}).strength; fallback: 1; onCommitted: n => wa.part("detail", { strength: wa.clamp(n, 0, 2) }) }
                NumberField { value: (wa.w.detail || {}).scale; fallback: 3; onCommitted: n => wa.part("detail", { scale: Math.max(0.01, n) }) } }
            InspectorRow { label: "Seed"; Layout.fillWidth: true
                NumberField { value: wa.waves.seed; fallback: 1; onCommitted: n => wa.part("waves", { seed: Math.max(0, Math.round(n)) }) } }

            Text { text: "Color"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { label: "Shallow, deep"; Layout.fillWidth: true
                ColorField { value: wa.look.shallow || "#3AB3A6"; onPicked: col => wa.part("look", { shallow: col }) }
                ColorField { value: wa.look.deep || "#0B2E4A"; onPicked: col => wa.part("look", { deep: col }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Absorption"; Layout.fillWidth: true
                NumberField { value: wa.look.absorption; fallback: 6; onCommitted: n => wa.part("look", { absorption: Math.max(0.01, n) }) } }
            InspectorRow { label: "Clarity"; Layout.fillWidth: true
                SliderField { from: 0; to: 1; value: wa.look.clarity; onMoved: wa.part("look", { clarity: value }) } }
            InspectorRow { visible: root.is3d; label: "Refraction"; Layout.fillWidth: true
                SliderField { from: 0; to: 1; value: wa.look.refraction; onMoved: wa.part("look", { refraction: value }) } }
            InspectorRow { visible: root.is3d; label: "Roughness, glint"; Layout.fillWidth: true
                NumberField { value: wa.look.roughness; fallback: 0.06; onCommitted: n => wa.part("look", { roughness: wa.clamp(n, 0.02, 1) }) }
                NumberField { value: wa.look.glint; fallback: 1; onCommitted: n => wa.part("look", { glint: wa.clamp(n, 0, 4) }) } }

            Text { text: "Foam"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { label: "Amount"; Layout.fillWidth: true
                NumberField { value: wa.foam.amount; fallback: 1; onCommitted: n => wa.part("foam", { amount: wa.clamp(n, 0, 2) }) }
                ColorField { value: wa.foam.color || "#F2F6F8"; onPicked: col => wa.part("foam", { color: col }) } }
            InspectorRow { label: root.is3d ? "Shore, crest" : "Line, crest"; Layout.fillWidth: true
                NumberField { value: wa.foam.shore; fallback: 0.6; onCommitted: n => wa.part("foam", { shore: Math.max(0, n) }) }
                NumberField { value: wa.foam.crest; fallback: 0.45; onCommitted: n => wa.part("foam", { crest: wa.clamp(n, 0, 1) }) } }
            InspectorRow { label: "Scale, drift"; Layout.fillWidth: true
                NumberField { value: wa.foam.scale; fallback: 2.5; onCommitted: n => wa.part("foam", { scale: Math.max(0.01, n) }) }
                NumberField { value: wa.foam.drift; fallback: 0.2; onCommitted: n => wa.part("foam", { drift: n }) } }

            Text { visible: root.is3d; text: "Reflections"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { visible: root.is3d; label: "Source"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "Auto", label: "Screen, then probe" }, { value: "Probe", label: "Probe only" }, { value: "ScreenSpace", label: "Screen, then sky" }, { value: "Planar", label: "Mirror camera" }, { value: "Sky", label: "Sky color" }]
                    value: wa.refl.mode || "Auto"; onChosen: m => wa.part("reflections", { mode: m }) } }
            InspectorRow { visible: root.is3d && wa.refl.mode === "Planar"; label: "Mirror scale"; Layout.fillWidth: true
                SliderField { from: 0.25; to: 1; value: wa.refl.planar_scale === undefined ? 0.5 : wa.refl.planar_scale; onMoved: wa.part("reflections", { planar_scale: value }) } }
            InspectorRow { visible: root.is3d && (wa.refl.mode === "Auto" || wa.refl.mode === "Probe"); label: "Probe px, frames"; Layout.fillWidth: true
                ChoiceField { options: [64, 128, 256, 512].map(n => ({ value: String(n), label: n + " px" })); value: String(wa.refl.probe_resolution || 128); onChosen: v => wa.part("reflections", { probe_resolution: Number(v) }) }
                NumberField { value: wa.refl.probe_refresh; fallback: 30; onCommitted: n => wa.part("reflections", { probe_refresh: wa.clamp(Math.round(n), 1, 600) }) } }
            InspectorRow { visible: root.is3d; label: "Strength"; Layout.fillWidth: true
                SliderField { from: 0; to: 1; value: wa.refl.strength; onMoved: wa.part("reflections", { strength: value }) } }

            Text { text: "Underwater"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { visible: root.is3d; label: "Fog, distance"; Layout.fillWidth: true
                ColorField { value: wa.under.fog || "#0F4C5C"; onPicked: col => wa.part("underwater", { fog: col }) }
                NumberField { value: wa.under.distance; fallback: 18; onCommitted: n => wa.part("underwater", { distance: Math.max(0.01, n) }) } }
            InspectorRow { label: "Caustics, scale"; Layout.fillWidth: true
                NumberField { value: wa.under.caustics; fallback: 1; onCommitted: n => wa.part("underwater", { caustics: wa.clamp(n, 0, 4) }) }
                NumberField { value: wa.under.caustics_scale; fallback: 3; onCommitted: n => wa.part("underwater", { caustics_scale: Math.max(0.01, n) }) } }
            InspectorRow { visible: root.is3d; label: "Caustics image"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["image"]; value: wa.under.caustics_texture || ""; placeholderText: "Built-in pattern"; onCommitted: p => wa.part("underwater", { caustics_texture: p }) } }

            Text { text: "Splashes"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { label: "Min speed, drops"; Layout.fillWidth: true
                NumberField { value: wa.splash.min_speed; fallback: 1.5; onCommitted: n => wa.part("splash", { min_speed: Math.max(0, n) }) }
                NumberField { value: wa.splash.particles; fallback: 16; onCommitted: n => wa.part("splash", { particles: wa.clamp(Math.round(n), 0, 128) }) } }
            InspectorRow { label: "Splash ripples"; Layout.fillWidth: true
                SwitchField { value: wa.splash.ripples !== false; onToggled: on => wa.part("splash", { ripples: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Sound"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["audio"]; value: wa.splash.sound || ""; placeholderText: "Drag a sound here"; onCommitted: p => wa.part("splash", { sound: p }) } }

            Text { text: "Ripples"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
            InspectorRow { label: "Simulate"; Layout.fillWidth: true
                SwitchField { value: wa.rip.enabled !== false; onToggled: on => wa.part("ripples", { enabled: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: wa.rip.enabled !== false; label: "Speed, fade s"; Layout.fillWidth: true
                NumberField { value: wa.rip.speed; fallback: root.is3d ? 2 : 120; onCommitted: n => wa.part("ripples", { speed: Math.max(0.01, n) }) }
                NumberField { value: wa.rip.fade; fallback: 3; onCommitted: n => wa.part("ripples", { fade: wa.clamp(n, 0.1, 60) }) } }
            InspectorRow { visible: wa.rip.enabled !== false; label: "Wake, extent"; Layout.fillWidth: true
                NumberField { value: wa.rip.wake; fallback: 1; onCommitted: n => wa.part("ripples", { wake: wa.clamp(n, 0, 2) }) }
                NumberField { value: wa.rip.extent; fallback: root.is3d ? 48 : 1600; onCommitted: n => wa.part("ripples", { extent: Math.max(0.1, n) }) } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: wa.ocean ? "Reaches the horizon from wherever the camera is. The actor's height is the sea level."
                    : "The actor's position is the middle of the surface at rest; turning it turns the water and its current." }
        }
    }
    Component {
        id: buoyancyCard
        ColumnLayout {
            id: bu
            readonly property var c: parent.c
            readonly property var b: root.buoyancyOf(c)
            spacing: 6
            InspectorRow { label: "Density"; Layout.fillWidth: true
                NumberField { value: bu.b.density; fallback: 0.5; onCommitted: n => root.writeBuoyancy(bu.c, { density: Math.min(10, Math.max(0.01, n)) }) } }
            InspectorRow { label: "Drag, spin drag"; Layout.fillWidth: true
                NumberField { value: bu.b.drag; fallback: 1; onCommitted: n => root.writeBuoyancy(bu.c, { drag: Math.min(50, Math.max(0, n)) }) }
                NumberField { value: bu.b.angular_drag; fallback: 1; onCommitted: n => root.writeBuoyancy(bu.c, { angular_drag: Math.min(50, Math.max(0, n)) }) } }
            InspectorRow { label: "Sample points"; Layout.fillWidth: true
                ChoiceField { options: [{ value: "1", label: "Centre" }, { value: "4", label: "Four corners" }, { value: "8", label: "Eight corners" }]; value: String(bu.b.points); onChosen: v => root.writeBuoyancy(bu.c, { points: Number(v) }) } }
            InspectorRow { label: "Splashes"; Layout.fillWidth: true
                SwitchField { value: bu.b.splash; onToggled: on => root.writeBuoyancy(bu.c, { splash: on }) } Item { Layout.fillWidth: true } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Needs a dynamic Body. Density 0.5 floats half under, 1 hangs in the water, above 1 sinks." }
        }
    }
    Component {
        id: parallaxCard
        ColumnLayout {
            id: px
            readonly property var c: parent.c
            readonly property var p: root.parallaxOf(c)
            spacing: 6
            InspectorRow { label: "Scroll x, y"; Layout.fillWidth: true
                NumberField { value: px.p.scroll[0]; fallback: 0.5; onCommitted: n => root.writeParallax(px.c, { scroll: [Math.min(2, Math.max(0, n)), px.p.scroll[1]] }) }
                NumberField { value: px.p.scroll[1]; fallback: 0.5; onCommitted: n => root.writeParallax(px.c, { scroll: [px.p.scroll[0], Math.min(2, Math.max(0, n))] }) } }
            InspectorRow { label: "Wrap x, y"; Layout.fillWidth: true
                SwitchField { value: px.p.wrap[0]; onToggled: on => root.writeParallax(px.c, { wrap: [on, px.p.wrap[1]] }) }
                SwitchField { value: px.p.wrap[1]; onToggled: on => root.writeParallax(px.c, { wrap: [px.p.wrap[0], on] }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Distance dim"; Layout.fillWidth: true
                NumberField { value: px.p.dim; fallback: 0; onCommitted: n => root.writeParallax(px.c, { dim: Math.min(1, Math.max(0, n)) }) } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "0 rides the camera like a sky, 1 moves with the actors, up to 2 sweeps past as foreground. Where it stands is where it shows with the camera " + (root.is3d ? "where it starts; it scrolls against the camera's x and y." : "at the origin.") + (root.is3d ? " Its depth puts it behind or in front of actors" : " The Render layer puts it behind or in front of actors") + "; a wrapped layer repeats, so make it at least a screen wide." }
        }
    }
    Component {
        id: persistCard
        ColumnLayout {
            readonly property var c: parent.c
            spacing: 6
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "Keeps this actor across scene switches: a `switch scene to` carries it - live position, variables and attached components included - into the new scene instead of unloading it. Clones still die with the old scene." }
        }
    }
    Component {
        id: roomCard
        ColumnLayout {
            id: rm
            readonly property var c: parent.c
            readonly property var r: root.roomOf(c)
            spacing: 6
            InspectorRow { label: "Size"; Layout.fillWidth: true
                NumberField { value: rm.r.size[0]; fallback: 1280; onCommitted: n => root.writeRoom(rm.c, { size: [Math.max(1, n), rm.r.size[1]] }) }
                NumberField { value: rm.r.size[1]; fallback: 720; onCommitted: n => root.writeRoom(rm.c, { size: [rm.r.size[0], Math.max(1, n)] }) } }
            InspectorRow { visible: root.is3d; label: "Depth"; Layout.fillWidth: true
                NumberField { value: rm.r.depth; fallback: 20; onCommitted: n => root.writeRoom(rm.c, { depth: Math.max(0.01, n) }) } }
            InspectorRow { label: "Holds camera"; Layout.fillWidth: true
                SwitchField { value: rm.r.camera; onToggled: on => root.writeRoom(rm.c, { camera: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: rm.r.camera; label: "Handoff s"; Layout.fillWidth: true
                NumberField { value: rm.r.blend; fallback: 0.4; onCommitted: n => root.writeRoom(rm.c, { blend: Math.min(5, Math.max(0, n)) }) } }
            InspectorRow { label: "Stream maps"; Layout.fillWidth: true
                SwitchField { value: rm.r.stream; onToggled: on => root.writeRoom(rm.c, { stream: on }) } Item { Layout.fillWidth: true } }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: root.is3d ? "A box centred on this actor. The camera stays inside the room its target stands in, a little off the walls, sliding over when it moves on; `when I enter room` fires with this actor's name. Streamed rooms draw the tilemaps inside them only while the camera is near." : "Centred on this actor. The camera stays inside the room its target stands in, sliding over when it moves on; `when I enter room` fires with this actor's name. Streamed rooms build the tilemaps inside them only while the camera is near." }
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
                HdrColorField { color: vo.fog.emissive; intensity: vo.fog.emissive_strength
                    hint: "The swatch keeps the color itself. Intensity is the glow in nits per unit of density, and every stop doubles it."
                    onPicked: (col, n) => root.writeVolume(vo.c, { fog: Object.assign({}, vo.fog, { emissive: col, emissive_strength: n }) }) } }
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
                ChoiceField { options: Blocks.opts(["TonyMcMapface","None","Reinhard","ReinhardLuminance","AcesFitted","Filmic","AgX","Neutral"]); value: parent.o.value
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
                text: "Flipbooks over image files or a sheet, or a Spine/DragonBones rig. play animation changes state; transitions fire on the clip ending, a marker, a trigger or a variable." }
            InspectorRow { label: "Rig"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["text"]; value: an.a.rig || ""; placeholderText: "Spine or DragonBones .json (optional)"; onCommitted: p => root.writeAnimation(an.c, { rig: p }) } }
            InspectorRow { visible: (an.a.rig || "") !== ""; label: "Skin"; Layout.fillWidth: true
                BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "default"; text: an.a.skin || ""
                    onEditingFinished: if (text.trim() !== (an.a.skin || "")) root.writeAnimation(an.c, { skin: text.trim() }) } }
            InspectorRow { visible: (an.a.rig || "") !== ""; label: "Slot tints"; Layout.fillWidth: true
                BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "cape=#FF4C4C, hand=#FFFFFF"; text: root.formatTints(an.a.slot_tints)
                    onEditingFinished: root.writeAnimation(an.c, { slot_tints: root.parseTints(text) }) } }
            InspectorRow { label: "Starts in"; Layout.fillWidth: true
                ChoiceField { Layout.fillWidth: true; options: [{ value: "", label: "nothing" }].concat(an.a.states.map(x => ({ value: x.name, label: x.name }))); value: an.a.initial || ""
                    onChosen: v => root.writeAnimation(an.c, { initial: v }) } }
            InspectorRow { label: "Crossfade s"; Layout.fillWidth: true
                NumberField { value: an.a.crossfade || 0; fallback: 0; onCommitted: n => root.writeAnimation(an.c, { crossfade: Math.min(10, Math.max(0, n)) }) } }
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
                    BwTextField { visible: !modelData.sheet; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Frames, one asset path per line or comma";
                        text: (modelData.frames || []).join(", ")
                        onEditingFinished: { const l = root.copy(an.a.clips); l[index].frames = text.split(/[\n,]+/).map(s => s.trim()).filter(s => s !== ""); root.writeAnimation(an.c, { clips: l }); } }
                    AssetField { Layout.fillWidth: true; app: root.app; accept: ["image"]; value: modelData.sheet ? modelData.sheet.image : ""; placeholderText: "Or drag a sprite sheet here"
                        onCommitted: p => root.writeClip(an, index, { sheet: p === "" ? null : Object.assign({ columns: 4, rows: 1, first: 0, count: 4 }, modelData.sheet || {}, { image: p }) }) }
                    RowLayout {
                        visible: !!modelData.sheet
                        Layout.fillWidth: true; spacing: 4
                        Text { text: "cols"; color: Theme.textDim; font.pixelSize: 11 }
                        NumberField { Layout.preferredWidth: 44; value: modelData.sheet ? modelData.sheet.columns : 1; fallback: 1; onCommitted: n => root.writeClip(an, index, { sheet: Object.assign({}, modelData.sheet, { columns: Math.max(1, Math.round(n)) }) }) }
                        Text { text: "rows"; color: Theme.textDim; font.pixelSize: 11 }
                        NumberField { Layout.preferredWidth: 44; value: modelData.sheet ? modelData.sheet.rows : 1; fallback: 1; onCommitted: n => root.writeClip(an, index, { sheet: Object.assign({}, modelData.sheet, { rows: Math.max(1, Math.round(n)) }) }) }
                        Text { text: "from"; color: Theme.textDim; font.pixelSize: 11 }
                        NumberField { Layout.preferredWidth: 44; value: modelData.sheet ? modelData.sheet.first : 0; fallback: 0; onCommitted: n => root.writeClip(an, index, { sheet: Object.assign({}, modelData.sheet, { first: Math.max(0, Math.round(n)) }) }) }
                        Text { text: "count"; color: Theme.textDim; font.pixelSize: 11 }
                        NumberField { Layout.preferredWidth: 44; value: modelData.sheet ? modelData.sheet.count : 1; fallback: 1; onCommitted: n => root.writeClip(an, index, { sheet: Object.assign({}, modelData.sheet, { count: Math.max(1, Math.round(n)) }) }) }
                    }
                    BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Seconds per frame, e.g. 0.1, 0.3 (blank uses fps)"
                        text: (modelData.durations || []).join(", ")
                        onEditingFinished: root.writeClip(an, index, { durations: root.parseNumbers(text) }) }
                    BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Markers, frame:name, e.g. 2:step, 5:step"
                        text: root.formatMarkers(modelData.markers)
                        onEditingFinished: root.writeClip(an, index, { markers: root.parseMarkers(text) }) }
                    RowLayout {
                        Layout.fillWidth: true; spacing: 4
                        Text { text: "moves"; color: Theme.textDim; font.pixelSize: 11 }
                        NumberField { Layout.preferredWidth: 52; value: (modelData.motion || [0, 0])[0]; fallback: 0; onCommitted: n => root.writeClip(an, index, { motion: [n, (modelData.motion || [0, 0])[1]] }) }
                        NumberField { Layout.preferredWidth: 52; value: (modelData.motion || [0, 0])[1]; fallback: 0; onCommitted: n => root.writeClip(an, index, { motion: [(modelData.motion || [0, 0])[0], n] }) }
                        BwTextField { visible: (an.a.rig || "") !== ""; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Rig animation (clip name)"; text: modelData.rig_animation || ""
                            onEditingFinished: root.writeClip(an, index, { rig_animation: text.trim() }) }
                    }
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
                delegate: ColumnLayout {
                    required property int index
                    required property var modelData
                    Layout.fillWidth: true; spacing: 4
                  RowLayout {
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
                    BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Transitions: run if speed > 2 blend 0.2; jump on trigger jump"
                        text: root.formatTransitions(modelData.transitions)
                        onEditingFinished: root.writeState(an, index, { transitions: root.parseTransitions(text) }) }
                    RowLayout {
                        Layout.fillWidth: true; spacing: 6
                        SwitchField { value: !!modelData.root_motion; onToggled: on => root.writeState(an, index, { root_motion: on }) }
                        Text { text: "Root motion moves the actor"; color: Theme.textDim; font.pixelSize: 11 }
                        Item { Layout.fillWidth: true }
                    }
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

    Component {
        id: spriteCard
        ColumnLayout {
            id: sp
            readonly property var c: parent.c
            readonly property var s: root.spriteOf(c)
            spacing: 6
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
                text: "2D dials over the look. Order sorts inside the Render layer; Y-sort draws lower actors in front. The palette's rows are palettes: the sprite's red channel picks the column." }
            InspectorRow { label: "Flip X"; Layout.fillWidth: true
                SwitchField { value: sp.s.flip_x; onToggled: on => root.writeSprite(sp.c, { flip_x: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Flip Y"; Layout.fillWidth: true
                SwitchField { value: sp.s.flip_y; onToggled: on => root.writeSprite(sp.c, { flip_y: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Order"; Layout.fillWidth: true
                NumberField { value: sp.s.order; fallback: 0; onCommitted: n => root.writeSprite(sp.c, { order: Math.max(-40, Math.min(40, Math.round(n))) }) } }
            InspectorRow { label: "Y-sort"; Layout.fillWidth: true
                SwitchField { value: sp.s.y_sort; onToggled: on => root.writeSprite(sp.c, { y_sort: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "Glow"; Layout.fillWidth: true
                NumberField { value: sp.s.glow; fallback: 0; onCommitted: n => root.writeSprite(sp.c, { glow: Math.min(Math.max(n, 0), 64) }) } }
            InspectorRow { label: "Casts shadow"; Layout.fillWidth: true
                SwitchField { value: sp.s.casts_shadow; onToggled: on => root.writeSprite(sp.c, { casts_shadow: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { label: "9-slice"; Layout.fillWidth: true
                SwitchField { value: !!sp.s.slice; onToggled: on => root.writeSprite(sp.c, { slice: on ? { border: [8, 8, 8, 8], center: "Stretch", sides: "Stretch", max_corner_scale: 1 } : null }) } Item { Layout.fillWidth: true } }
            RowLayout {
                visible: !!sp.s.slice
                Layout.fillWidth: true; spacing: 4
                Repeater {
                    model: ["left", "right", "top", "bottom"]
                    delegate: NumberField {
                        required property int index
                        Layout.preferredWidth: 48; value: sp.s.slice ? sp.s.slice.border[index] : 0; fallback: 0
                        onCommitted: n => { const b = sp.s.slice.border.slice(); b[index] = Math.max(0, n); root.writeSprite(sp.c, { slice: Object.assign({}, sp.s.slice, { border: b }) }); }
                    }
                }
            }
            InspectorRow { visible: !!sp.s.slice; label: "Centre, sides"; Layout.fillWidth: true
                ChoiceField { Layout.fillWidth: true; options: Blocks.opts(["Stretch","Tile"]); value: sp.s.slice ? sp.s.slice.center : "Stretch"
                    onChosen: v => root.writeSprite(sp.c, { slice: Object.assign({}, sp.s.slice, { center: v }) }) }
                ChoiceField { Layout.fillWidth: true; options: Blocks.opts(["Stretch","Tile"]); value: sp.s.slice ? sp.s.slice.sides : "Stretch"
                    onChosen: v => root.writeSprite(sp.c, { slice: Object.assign({}, sp.s.slice, { sides: v }) }) } }
            InspectorRow { label: "Stack"; Layout.fillWidth: true
                SwitchField { value: !!sp.s.stack; onToggled: on => root.writeSprite(sp.c, { stack: on ? { image: "", layers: 8, offset: [0, 1] } : null }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: !!sp.s.stack; label: "Slices"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["image"]; value: sp.s.stack ? sp.s.stack.image : ""; placeholderText: "The look's image"
                    onCommitted: p => root.writeSprite(sp.c, { stack: Object.assign({}, sp.s.stack, { image: p }) }) } }
            InspectorRow { visible: !!sp.s.stack; label: "Layers, step"; Layout.fillWidth: true
                NumberField { Layout.preferredWidth: 48; value: sp.s.stack ? sp.s.stack.layers : 8; fallback: 8
                    onCommitted: n => root.writeSprite(sp.c, { stack: Object.assign({}, sp.s.stack, { layers: Math.max(1, Math.min(128, Math.round(n))) }) }) }
                NumberField { Layout.preferredWidth: 48; value: sp.s.stack ? sp.s.stack.offset[0] : 0; fallback: 0
                    onCommitted: n => root.writeSprite(sp.c, { stack: Object.assign({}, sp.s.stack, { offset: [n, sp.s.stack.offset[1]] }) }) }
                NumberField { Layout.preferredWidth: 48; value: sp.s.stack ? sp.s.stack.offset[1] : 1; fallback: 1
                    onCommitted: n => root.writeSprite(sp.c, { stack: Object.assign({}, sp.s.stack, { offset: [sp.s.stack.offset[0], n] }) }) } }
            InspectorRow { label: "Palette"; Layout.fillWidth: true
                AssetField { app: root.app; accept: ["image"]; value: sp.s.palette || ""; placeholderText: "None"; onCommitted: p => root.writeSprite(sp.c, { palette: p }) } }
            InspectorRow { visible: (sp.s.palette || "") !== ""; label: "Palette row"; Layout.fillWidth: true
                NumberField { value: sp.s.palette_index; fallback: 0; onCommitted: n => root.writeSprite(sp.c, { palette_index: Math.max(0, Math.round(n)) }) } }
            InspectorRow { label: "Outline px"; Layout.fillWidth: true
                NumberField { value: sp.s.outline_width; fallback: 0; onCommitted: n => root.writeSprite(sp.c, { outline_width: Math.max(0, Math.min(16, n)) }) } }
            InspectorRow { visible: sp.s.outline_width > 0; label: "Outline color"; Layout.fillWidth: true
                ColorField { value: sp.s.outline_color; onPicked: col => root.writeSprite(sp.c, { outline_color: col }) } Item { Layout.fillWidth: true } }
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
