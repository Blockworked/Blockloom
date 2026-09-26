import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The VFX graph half of an Emitter: spawn shape, bursts, the update stack,
// how particles draw, ribbons and where they simulate. `edited` hands back
// the fields it changed, for the inspector to lay over the emitter.
ColumnLayout {
    id: root
    required property var app
    property var emitter: ({})
    property bool is3d: true
    signal edited(var next)
    spacing: 6

    function copy(v) { return JSON.parse(JSON.stringify(v)); }
    function curve(from, to) { return { keys: [{ t: 0, v: from }, { t: 1, v: to }] }; }
    readonly property var e: emitter || {}
    readonly property var shape: Object.assign({ shape: "Point" }, e.shape || {})
    readonly property var bursts: e.bursts || []
    readonly property var modules: e.modules || []
    readonly property var r: {
        const r = e.render || {};
        return Object.assign({ blend: "Additive", facing: "Camera", stretch: 0.05, lit: false, soft: 0, intensity: 1, spin: 0, random_rotation: false, size_random: 0 }, r, {
            flipbook: Object.assign({ image: "", columns: 1, rows: 1, fps: 0, random_start: false, blend: true }, r.flipbook || {}),
            size: r.size || curve(1, 1),
            alpha: r.alpha || curve(1, 0),
            rotation: r.rotation || curve(0, 0),
            color: r.color || { keys: [] }
        });
    }
    readonly property var rb: {
        const b = e.ribbon || {};
        return Object.assign({ enabled: false, source: "Particles", points: 16, step: 0.25, tessellation: 2, width: 0.2, face_camera: true, intensity: 1, heads: true }, b, {
            width_curve: b.width_curve || curve(1, 0),
            color: b.color || { keys: [] }
        });
    }

    function setRender(next) { edited({ render: Object.assign(copy(r), next) }); }
    function setFlipbook(next) { setRender({ flipbook: Object.assign(copy(r.flipbook), next) }); }
    function setRibbon(next) { edited({ ribbon: Object.assign(copy(rb), next) }); }
    function setShape(next) { edited({ shape: Object.assign(copy(shape), next) }); }
    function setBurst(i, next) {
        const list = copy(bursts);
        if (next === null) list.splice(i, 1); else list[i] = Object.assign(list[i], next);
        edited({ bursts: list });
    }
    function setModule(i, next) {
        const list = copy(modules);
        if (next === null) list.splice(i, 1); else list[i] = Object.assign(list[i], next);
        edited({ modules: list });
    }
    function moveModule(i, by) {
        const j = i + by;
        if (j < 0 || j >= modules.length) return;
        const list = copy(modules);
        const moved = list.splice(i, 1)[0];
        list.splice(j, 0, moved);
        edited({ modules: list });
    }
    function withIndex(list, i, v) { const next = list.slice(); next[i] = v; return next; }

    readonly property var shapeDefaults: ({
        Point: { shape: "Point" },
        Sphere: { shape: "Sphere", radius: 1, surface: false },
        Box: { shape: "Box", size: [1, 1, 1] },
        Cone: { shape: "Cone", radius: 1 },
        MeshSurface: { shape: "MeshSurface" }
    })
    readonly property var moduleDefaults: ({
        Force: { module: "Force", force: [0, 0, 0] },
        Drag: { module: "Drag", amount: 1 },
        CurlNoise: { module: "CurlNoise", strength: 2, scale: 0.5, speed: 0.3 },
        Turbulence: { module: "Turbulence", strength: 2, scale: 0.5 },
        Attractor: { module: "Attractor", target: "", strength: 10, radius: 0, kill_radius: 0 },
        Collide: { module: "Collide", bounce: 0.4, friction: 0.2, lifetime_loss: 0, kill: false },
        KillPlane: { module: "KillPlane", point: [0, 0, 0], normal: [0, 1, 0] }
    })
    readonly property var moduleNames: ({ Force: "Force", Drag: "Drag", CurlNoise: "Curl noise", Turbulence: "Turbulence", Attractor: "Attractor", Collide: "Collide", KillPlane: "Kill plane" })
    function opts(list) { return list.map(v => ({ value: v[0], label: v[1] })); }

    // ─── Spawn ───────────────────────────────────────────────────────────────
    SectionLabel { label: "Spawn" }
    InspectorRow { label: "Shape"; Layout.fillWidth: true
        ChoiceField { options: root.opts([["Point", "Point"], ["Sphere", "Sphere"], ["Box", "Box"], ["Cone", "Cone"], ["MeshSurface", "My mesh's surface"]])
            value: root.shape.shape; onChosen: s => root.edited({ shape: root.copy(root.shapeDefaults[s]) }) } }
    InspectorRow { visible: root.shape.shape === "Sphere" || root.shape.shape === "Cone"; label: "Radius"; Layout.fillWidth: true
        NumberField { value: root.shape.radius; fallback: 1; onCommitted: n => root.setShape({ radius: Math.max(0, n) }) } }
    InspectorRow { visible: root.shape.shape === "Sphere"; label: "Surface only"; Layout.fillWidth: true
        SwitchField { value: !!root.shape.surface; onToggled: on => root.setShape({ surface: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { visible: root.shape.shape === "Box"; label: "Size"; Layout.fillWidth: true
        Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index
            value: root.shape.size ? root.shape.size[index] : 1; fallback: 1
            onCommitted: n => root.setShape({ size: root.withIndex(root.shape.size || [1, 1, 1], index, Math.max(0, n)) }) } } }
    Text { visible: root.shape.shape === "MeshSurface"; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Particles are born on the actor's own look, weighted by area. Without a mesh they come from its middle." }
    InspectorRow { label: "Direction"; Layout.fillWidth: true
        ChoiceField { options: root.opts([["Facing", "Where I face"], ["Outward", "Out of the shape"]]); value: root.e.direction || "Facing"
            onChosen: d => root.edited({ direction: d }) } }
    InspectorRow { label: "Duration s"; Layout.fillWidth: true
        NumberField { value: root.e.duration === undefined ? 2 : root.e.duration; fallback: 2; onCommitted: n => root.edited({ duration: n }) } }
    Repeater {
        model: root.bursts.length
        delegate: InspectorRow {
            required property int index
            readonly property var b: Object.assign({ time: 0, count: 16, cycles: 1, interval: 1 }, root.bursts[index] || {})
            label: "Burst " + (index + 1); Layout.fillWidth: true
            NumberField { value: b.time; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "Seconds into the effect"
                onCommitted: n => root.setBurst(index, { time: Math.max(0, n) }) }
            NumberField { value: b.count; fallback: 16; ToolTip.visible: hovered; ToolTip.text: "Particles"
                onCommitted: n => root.setBurst(index, { count: Math.max(0, Math.round(n)) }) }
            NumberField { value: b.cycles; fallback: 1; ToolTip.visible: hovered; ToolTip.text: "Times it fires, 0 for forever"
                onCommitted: n => root.setBurst(index, { cycles: Math.max(0, Math.round(n)) }) }
            NumberField { value: b.interval; fallback: 1; ToolTip.visible: hovered; ToolTip.text: "Seconds between cycles"
                onCommitted: n => root.setBurst(index, { interval: Math.max(0.01, n) }) }
            IconButton { iconName: "x"; tip: "Remove this burst"; implicitWidth: 24; implicitHeight: 24; onClicked: root.setBurst(index, null) }
        }
    }
    BwButton { text: "Add burst"; implicitHeight: 28; font.pixelSize: 12
        onClicked: root.edited({ bursts: root.copy(root.bursts).concat([{ time: 0, count: 16, cycles: 1, interval: 1 }]) }) }

    // ─── Update ──────────────────────────────────────────────────────────────
    SectionLabel { label: "Update" }
    Repeater {
        model: root.modules.length
        delegate: ColumnLayout {
            id: mod
            required property int index
            readonly property var m: Object.assign(root.copy(root.moduleDefaults[(root.modules[index] || {}).module] || {}), root.modules[index] || {})
            Layout.fillWidth: true; spacing: 4
            RowLayout {
                Layout.fillWidth: true
                Text { Layout.fillWidth: true; text: (mod.index + 1) + ". " + (root.moduleNames[mod.m.module] || mod.m.module); color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
                IconButton { iconName: "chevron-up"; tip: "Run earlier"; enabled: mod.index > 0; implicitWidth: 24; implicitHeight: 24; onClicked: root.moveModule(mod.index, -1) }
                IconButton { iconName: "chevron-down"; tip: "Run later"; enabled: mod.index < root.modules.length - 1; implicitWidth: 24; implicitHeight: 24; onClicked: root.moveModule(mod.index, 1) }
                IconButton { iconName: "x"; tip: "Remove this module"; implicitWidth: 24; implicitHeight: 24; onClicked: root.setModule(mod.index, null) }
            }
            InspectorRow { visible: mod.m.module === "Force"; label: "Force"; Layout.fillWidth: true
                Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index
                    value: mod.m.force ? mod.m.force[index] : 0; fallback: 0
                    onCommitted: n => root.setModule(mod.index, { force: root.withIndex(mod.m.force || [0, 0, 0], index, n) }) } } }
            InspectorRow { visible: mod.m.module === "Drag"; label: "Amount"; Layout.fillWidth: true
                NumberField { value: mod.m.amount; fallback: 1; onCommitted: n => root.setModule(mod.index, { amount: n }) } }
            InspectorRow { visible: ["CurlNoise", "Turbulence", "Attractor"].indexOf(mod.m.module) >= 0; label: "Strength"; Layout.fillWidth: true
                NumberField { value: mod.m.strength; fallback: 2; onCommitted: n => root.setModule(mod.index, { strength: n }) } }
            InspectorRow { visible: mod.m.module === "CurlNoise" || mod.m.module === "Turbulence"; label: "Scale"; Layout.fillWidth: true
                NumberField { value: mod.m.scale; fallback: 0.5; onCommitted: n => root.setModule(mod.index, { scale: n }) } }
            InspectorRow { visible: mod.m.module === "CurlNoise"; label: "Speed"; Layout.fillWidth: true
                NumberField { value: mod.m.speed; fallback: 0.3; onCommitted: n => root.setModule(mod.index, { speed: n }) } }
            InspectorRow { visible: mod.m.module === "Attractor"; label: "Target"; Layout.fillWidth: true
                BwTextField { Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Actor name (empty is me)"; text: mod.m.target || ""
                    onEditingFinished: if (text.trim() !== (mod.m.target || "")) root.setModule(mod.index, { target: text.trim() }) } }
            InspectorRow { visible: mod.m.module === "Attractor"; label: "Radius"; Layout.fillWidth: true
                NumberField { value: mod.m.radius; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "0 pulls from anywhere"
                    onCommitted: n => root.setModule(mod.index, { radius: Math.max(0, n) }) }
                NumberField { value: mod.m.kill_radius; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "Particles this close die"
                    onCommitted: n => root.setModule(mod.index, { kill_radius: Math.max(0, n) }) } }
            InspectorRow { visible: mod.m.module === "Collide"; label: "Bounce"; Layout.fillWidth: true
                NumberField { value: mod.m.bounce; fallback: 0.4; onCommitted: n => root.setModule(mod.index, { bounce: n }) }
                NumberField { value: mod.m.friction; fallback: 0.2; ToolTip.visible: hovered; ToolTip.text: "Friction"
                    onCommitted: n => root.setModule(mod.index, { friction: n }) } }
            InspectorRow { visible: mod.m.module === "Collide"; label: "Life lost"; Layout.fillWidth: true
                NumberField { value: mod.m.lifetime_loss; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "Share of its life a hit costs"
                    onCommitted: n => root.setModule(mod.index, { lifetime_loss: n }) } }
            InspectorRow { visible: mod.m.module === "Collide"; label: "Die on hit"; Layout.fillWidth: true
                SwitchField { value: !!mod.m.kill; onToggled: on => root.setModule(mod.index, { kill: on }) } Item { Layout.fillWidth: true } }
            InspectorRow { visible: mod.m.module === "KillPlane"; label: "Point"; Layout.fillWidth: true
                Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index
                    value: mod.m.point ? mod.m.point[index] : 0; fallback: 0
                    onCommitted: n => root.setModule(mod.index, { point: root.withIndex(mod.m.point || [0, 0, 0], index, n) }) } } }
            InspectorRow { visible: mod.m.module === "KillPlane"; label: "Normal"; Layout.fillWidth: true
                Repeater { model: root.is3d ? 3 : 2; delegate: NumberField { required property int index
                    value: mod.m.normal ? mod.m.normal[index] : 0; fallback: 0
                    onCommitted: n => root.setModule(mod.index, { normal: root.withIndex(mod.m.normal || [0, 1, 0], index, n) }) } } }
        }
    }
    InspectorRow { label: "Add"; Layout.fillWidth: true
        ChoiceField { placeholder: "Add a module"; value: ""
            options: root.opts([["Force", "Force"], ["Drag", "Drag"], ["CurlNoise", "Curl noise"], ["Turbulence", "Turbulence"], ["Attractor", "Attractor"], ["Collide", "Collide"], ["KillPlane", "Kill plane"]])
            onChosen: m => root.edited({ modules: root.copy(root.modules).concat([root.copy(root.moduleDefaults[m])]) }) } }
    Text { visible: root.modules.some(m => m.module === "Collide"); Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: root.is3d ? "On the GPU particles collide with what the camera sees (the depth buffer); on the CPU with actors' collision shapes."
                        : "Particles collide with actors' collision shapes." }

    // ─── Render ──────────────────────────────────────────────────────────────
    SectionLabel { label: "Render" }
    InspectorRow { label: "Blend"; Layout.fillWidth: true
        ChoiceField { options: root.opts([["Additive", "Additive (glow)"], ["Alpha", "Alpha (smoke)"]]); value: root.r.blend; onChosen: v => root.setRender({ blend: v }) } }
    InspectorRow { label: "Facing"; Layout.fillWidth: true
        ChoiceField { options: root.opts([["Camera", "The camera"], ["Velocity", "Along velocity"], ["Flat", "Flat on the ground"]]); value: root.r.facing; onChosen: v => root.setRender({ facing: v }) } }
    InspectorRow { visible: root.r.facing === "Velocity"; label: "Stretch"; Layout.fillWidth: true
        NumberField { value: root.r.stretch; fallback: 0.05; onCommitted: n => root.setRender({ stretch: Math.max(0, n) }) } }
    InspectorRow { label: "Intensity"; Layout.fillWidth: true
        NumberField { value: root.r.intensity; fallback: 1; onCommitted: n => root.setRender({ intensity: Math.max(0, n) }) } }
    InspectorRow { visible: root.is3d; label: "Lit"; Layout.fillWidth: true
        SwitchField { value: root.r.lit; onToggled: on => root.setRender({ lit: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { visible: root.is3d; label: "Soft"; Layout.fillWidth: true
        NumberField { value: root.r.soft; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "Distance over which particles fade into what they touch"
            onCommitted: n => root.setRender({ soft: Math.max(0, n) }) } }
    InspectorRow { label: "Spin"; Layout.fillWidth: true
        NumberField { value: root.r.spin; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "Degrees a second"; onCommitted: n => root.setRender({ spin: n }) }
        SwitchField { value: root.r.random_rotation; onToggled: on => root.setRender({ random_rotation: on }) }
        Text { text: "random start"; color: Theme.textDim; font.pixelSize: 11 } }
    InspectorRow { label: "Size jitter"; Layout.fillWidth: true
        NumberField { value: root.r.size_random; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "0 to 1: how much sizes vary"
            onCommitted: n => root.setRender({ size_random: Math.min(1, Math.max(0, n)) }) } }
    Text { text: "Size over life"; color: Theme.textDim; font.pixelSize: 12 }
    CurveField { keys: root.r.size.keys; low: 0; high: 2; onEdited: k => root.setRender({ size: { keys: k } }) }
    Text { text: "Opacity over life"; color: Theme.textDim; font.pixelSize: 12 }
    CurveField { keys: root.r.alpha.keys; low: 0; high: 1; onEdited: k => root.setRender({ alpha: { keys: k } }) }
    Text { text: "Rotation over life (degrees)"; color: Theme.textDim; font.pixelSize: 12 }
    CurveField { keys: root.r.rotation.keys; low: -360; high: 360; onEdited: k => root.setRender({ rotation: { keys: k } }) }
    Text { text: "Color over life, over the tint above"; color: Theme.textDim; font.pixelSize: 12 }
    Repeater {
        model: root.r.color.keys.length
        delegate: InspectorRow {
            required property int index
            readonly property var k: root.r.color.keys[index] || { t: 0, color: "#FFFFFF", alpha: 1 }
            function setKey(next) {
                const keys = root.copy(root.r.color.keys);
                if (next === null) keys.splice(index, 1); else keys[index] = Object.assign(keys[index], next);
                root.setRender({ color: { keys: keys } });
            }
            label: "At"; Layout.fillWidth: true
            NumberField { value: k.t; fallback: 0; onCommitted: n => setKey({ t: Math.min(1, Math.max(0, n)) }) }
            ColorField { value: k.color; onPicked: col => setKey({ color: col }) }
            NumberField { value: k.alpha === undefined ? 1 : k.alpha; fallback: 1; ToolTip.visible: hovered; ToolTip.text: "Opacity"
                onCommitted: n => setKey({ alpha: Math.min(1, Math.max(0, n)) }) }
            IconButton { iconName: "x"; tip: "Remove this color"; implicitWidth: 24; implicitHeight: 24; onClicked: setKey(null) }
        }
    }
    BwButton { text: "Add color"; implicitHeight: 28; font.pixelSize: 12
        onClicked: {
            const keys = root.copy(root.r.color.keys);
            keys.push({ t: keys.length ? 1 : 0, color: "#FFFFFF", alpha: 1 });
            root.setRender({ color: { keys: keys } });
        } }
    InspectorRow { label: "Flipbook"; Layout.fillWidth: true
        AssetField { app: root.app; accept: ["image"]; value: root.r.flipbook.image; placeholderText: "Drag a sprite sheet here"; onCommitted: p => root.setFlipbook({ image: p }) } }
    InspectorRow { visible: root.r.flipbook.image !== ""; label: "Grid"; Layout.fillWidth: true
        NumberField { value: root.r.flipbook.columns; fallback: 1; ToolTip.visible: hovered; ToolTip.text: "Columns"
            onCommitted: n => root.setFlipbook({ columns: Math.max(1, Math.round(n)) }) }
        NumberField { value: root.r.flipbook.rows; fallback: 1; ToolTip.visible: hovered; ToolTip.text: "Rows"
            onCommitted: n => root.setFlipbook({ rows: Math.max(1, Math.round(n)) }) } }
    InspectorRow { visible: root.r.flipbook.image !== ""; label: "Frames /s"; Layout.fillWidth: true
        NumberField { value: root.r.flipbook.fps; fallback: 0; ToolTip.visible: hovered; ToolTip.text: "0 plays the sheet once over each life"
            onCommitted: n => root.setFlipbook({ fps: Math.max(0, n) }) } }
    InspectorRow { visible: root.r.flipbook.image !== ""; label: "Random start"; Layout.fillWidth: true
        SwitchField { value: root.r.flipbook.random_start; onToggled: on => root.setFlipbook({ random_start: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { visible: root.r.flipbook.image !== ""; label: "Blend frames"; Layout.fillWidth: true
        SwitchField { value: root.r.flipbook.blend; onToggled: on => root.setFlipbook({ blend: on }) } Item { Layout.fillWidth: true } }

    // ─── Ribbons ─────────────────────────────────────────────────────────────
    SectionLabel { label: "Ribbons" }
    InspectorRow { label: "Ribbons"; Layout.fillWidth: true
        SwitchField { value: root.rb.enabled; onToggled: on => root.setRibbon({ enabled: on }) } Item { Layout.fillWidth: true } }
    ColumnLayout {
        visible: root.rb.enabled; Layout.fillWidth: true; spacing: 6
        InspectorRow { label: "Follow"; Layout.fillWidth: true
            ChoiceField { options: root.opts([["Particles", "Each particle"], ["Actor", "Me"]]); value: root.rb.source; onChosen: v => root.setRibbon({ source: v }) } }
        InspectorRow { label: "Points"; Layout.fillWidth: true
            NumberField { value: root.rb.points; fallback: 16; onCommitted: n => root.setRibbon({ points: Math.round(n) }) }
            NumberField { value: root.rb.step; fallback: 0.25; ToolTip.visible: hovered; ToolTip.text: "Distance between points"
                onCommitted: n => root.setRibbon({ step: n }) } }
        InspectorRow { label: "Smoothing"; Layout.fillWidth: true
            NumberField { value: root.rb.tessellation; fallback: 2; ToolTip.visible: hovered; ToolTip.text: "Extra segments between points"
                onCommitted: n => root.setRibbon({ tessellation: Math.round(n) }) } }
        InspectorRow { label: "Width"; Layout.fillWidth: true
            NumberField { value: root.rb.width; fallback: 0.2; onCommitted: n => root.setRibbon({ width: Math.max(0, n) }) } }
        Text { text: "Width along the ribbon"; color: Theme.textDim; font.pixelSize: 12 }
        CurveField { keys: root.rb.width_curve.keys; low: 0; high: 1; onEdited: k => root.setRibbon({ width_curve: { keys: k } }) }
        InspectorRow { label: "Intensity"; Layout.fillWidth: true
            NumberField { value: root.rb.intensity; fallback: 1; onCommitted: n => root.setRibbon({ intensity: Math.max(0, n) }) } }
        InspectorRow { visible: root.is3d; label: "Face camera"; Layout.fillWidth: true
            SwitchField { value: root.rb.face_camera; onToggled: on => root.setRibbon({ face_camera: on }) } Item { Layout.fillWidth: true } }
        InspectorRow { visible: root.rb.source === "Particles"; label: "Heads too"; Layout.fillWidth: true
            SwitchField { value: root.rb.heads; onToggled: on => root.setRibbon({ heads: on }) } Item { Layout.fillWidth: true } }
    }

    // ─── Simulation ──────────────────────────────────────────────────────────
    SectionLabel { label: "Simulation" }
    InspectorRow { visible: root.is3d; label: "Runs on"; Layout.fillWidth: true
        ChoiceField { options: root.opts([["Auto", "GPU when it can"], ["Cpu", "CPU"]]); value: root.e.sim || "Auto"; onChosen: v => root.edited({ sim: v }) } }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: root.is3d ? "The GPU runs up to 65536 particles an emitter; the CPU 4096, with collisions against actors and exact events. A ribbon following me always runs on the CPU."
                        : "2D emitters run on the CPU, up to 4096 particles each." }
}
