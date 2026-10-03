import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// One Collider component: a shape, where it stands in its actor, its surface
// and its layer. An actor may carry several; each is addressed by its id, never
// its place in the list. Edits go through `set_collider`.
ColumnLayout {
    id: root
    required property var app
    // The Collider component as the snapshot holds it.
    property var component: ({})
    property bool is3d: true
    spacing: 6

    readonly property var defaults: ({
        name: "", enabled: true, center: [0, 0, 0], rotation: [0, 0, 0], material: { kind: "Default" },
        trigger: false, layer: 1, layer_overrides: { include: 0, exclude: 0, priority: 0 },
        contact_offset: null, queryable: true, one_way: false, material_overrides: {}
    })
    readonly property var k: Object.assign({}, defaults, component.collider || {})
    readonly property string geometryKind: k.geometry ? k.geometry.kind : "FromLook"
    readonly property var shape: geometryKind === "Shape" ? k.geometry.shape : null
    readonly property string shapeKind: shape ? shape.kind : "FromLook"
    readonly property var overrides: k.material_overrides || {}

    readonly property var physics: app.appState.project ? app.appState.project.physics : null
    readonly property var layerNames: physics && physics.layers && physics.layers.names ? physics.layers.names : []
    readonly property var layerChoices: {
        const out = [];
        for (let i = 1; i <= 32; ++i) out.push({ value: String(i), label: layerNames[i - 1] ? i + " " + layerNames[i - 1] : "Layer " + i });
        return out;
    }
    function layerName(n) { return layerNames[n - 1] || "Layer " + n; }

    readonly property var shapeChoices: {
        const out = [{ value: "FromLook", label: "Follows the look" }];
        const kinds = is3d ? ["Box", "Sphere", "Capsule", "ConvexHull", "TriangleMesh", "Terrain"]
                           : ["Rect", "Circle", "Capsule2d", "Polygon", "Edge", "Chain", "Tilemap"];
        const labels = { Box: "Box", Sphere: "Sphere", Capsule: "Capsule", ConvexHull: "Convex hull", TriangleMesh: "Triangle mesh", Terrain: "Terrain",
            Rect: "Rectangle", Circle: "Circle", Capsule2d: "Capsule", Polygon: "Polygon", Edge: "Edge", Chain: "Chain", Tilemap: "Tilemap" };
        for (const kind of kinds) out.push({ value: kind, label: labels[kind] });
        return out;
    }
    readonly property var materialChoices: {
        const out = [{ value: "Default", label: "Default" }, { value: "Ice", label: "Ice" }, { value: "Rubber", label: "Rubber" }, { value: "No Bounce", label: "No Bounce" }];
        if (k.material.kind === "Asset") out.push({ value: "asset:" + k.material.id, label: "Project material" });
        return out;
    }
    readonly property string materialValue: k.material.kind === "Default" ? "Default" : (k.material.kind === "BuiltIn" ? k.material.name : "asset:" + k.material.id)
    readonly property var axisChoices: [{ value: "X", label: "X" }, { value: "Y", label: "Y" }, { value: "Z", label: "Z" }]

    function copy(o) { return JSON.parse(JSON.stringify(o)); }
    function write(next) {
        app.invoke("set_collider", { collider: Object.assign(copy(k), next) });
    }
    function defaultShape(kind) {
        switch (kind) {
        case "Box": return { kind: "Box", size: [1, 1, 1] };
        case "Sphere": return { kind: "Sphere", radius: 0.5 };
        case "Capsule": return { kind: "Capsule", radius: 0.5, height: 2, axis: "Y" };
        case "ConvexHull": return { kind: "ConvexHull", mesh: "" };
        case "TriangleMesh": return { kind: "TriangleMesh", mesh: "" };
        case "Terrain": return { kind: "Terrain" };
        case "Rect": return { kind: "Rect", size: [60, 60] };
        case "Circle": return { kind: "Circle", radius: 30 };
        case "Capsule2d": return { kind: "Capsule2d", size: [40, 80], horizontal: false };
        case "Polygon": return { kind: "Polygon", points: [[-30, -30], [30, -30], [0, 30]] };
        case "Edge": return { kind: "Edge", a: [-30, 0], b: [30, 0] };
        case "Chain": return { kind: "Chain", points: [[-60, 0], [0, 20], [60, 0]], closed: false };
        default: return { kind: "Tilemap" };
        }
    }
    function chooseShape(kind) {
        if (kind === "FromLook") write({ geometry: { kind: "FromLook" } });
        else write({ geometry: { kind: "Shape", shape: defaultShape(kind) } });
    }
    function writeShape(next) { write({ geometry: { kind: "Shape", shape: Object.assign(copy(shape), next) } }); }
    function withIndex(array, index, value) { const next = array.slice(); next[index] = value; return next; }
    function pointsText(points) { return (points || []).map(p => p[0] + "," + p[1]).join("  "); }
    function parsePoints(text) {
        return text.split(/\s+/).filter(t => t !== "").map(t => t.split(",").map(Number)).filter(p => p.length === 2 && p.every(Number.isFinite));
    }
    function setMaterial(v) {
        if (v === "Default") write({ material: { kind: "Default" } });
        else if (v.indexOf("asset:") === 0) write({ material: { kind: "Asset", id: v.slice(6) } });
        else write({ material: { kind: "BuiltIn", name: v } });
    }
    function setOverride(key, n) {
        const next = copy(overrides);
        if (n === null) delete next[key]; else next[key] = Math.max(0, n);
        write({ material_overrides: next });
    }
    function setLayerBit(group, index) {
        const next = copy(k.layer_overrides);
        next[group] = (next[group] ^ (1 << index)) >>> 0;
        write({ layer_overrides: next });
    }

    InspectorRow { label: "Name"; Layout.fillWidth: true
        BwTextField { objectName: "collider-name"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; text: root.k.name
            placeholderText: root.shapeKind === "FromLook" ? "Collider" : root.shapeKind
            onEditingFinished: if (text !== root.k.name) root.write({ name: text }) } }
    InspectorRow { label: "Enabled"; Layout.fillWidth: true
        SwitchField { objectName: "collider-enabled"; value: root.k.enabled; onToggled: on => root.write({ enabled: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Trigger"; Layout.fillWidth: true
        SwitchField { objectName: "collider-trigger"; value: root.k.trigger; onToggled: on => root.write({ trigger: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "In queries"; Layout.fillWidth: true
        SwitchField { objectName: "collider-queryable"; value: root.k.queryable; onToggled: on => root.write({ queryable: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "One-way"; visible: !root.is3d; Layout.fillWidth: true
        SwitchField { objectName: "collider-one-way"; value: root.k.one_way; onToggled: on => root.write({ one_way: on }) } Item { Layout.fillWidth: true } }

    InspectorRow { label: "Shape"; Layout.fillWidth: true
        ChoiceField { objectName: "collider-shape"; options: root.shapeChoices; value: root.shapeKind; onChosen: v => root.chooseShape(v) } }

    // ─── The saved shape's own numbers ─────────────────────────────────────
    InspectorRow { label: "Size"; visible: root.shapeKind === "Box"; Layout.fillWidth: true
        Repeater { model: root.shapeKind === "Box" ? 3 : 0
            delegate: NumberField { required property int index; objectName: "box-size-" + index; value: root.shape.size[index]; fallback: 1
                onCommitted: n => root.writeShape({ size: root.withIndex(root.shape.size, index, Math.max(0.001, n)) }) } } }
    InspectorRow { label: "Size"; visible: root.shapeKind === "Rect" || root.shapeKind === "Capsule2d"; Layout.fillWidth: true
        Repeater { model: root.shapeKind === "Rect" || root.shapeKind === "Capsule2d" ? 2 : 0
            delegate: NumberField { required property int index; objectName: "size-" + index; value: root.shape.size[index]; fallback: 1
                onCommitted: n => root.writeShape({ size: root.withIndex(root.shape.size, index, Math.max(0.001, n)) }) } } }
    InspectorRow { label: "Radius"; visible: root.shapeKind === "Sphere" || root.shapeKind === "Capsule" || root.shapeKind === "Circle"; Layout.fillWidth: true
        NumberField { objectName: "shape-radius"; value: root.shape && root.shape.radius !== undefined ? root.shape.radius : 0.5; fallback: 0.5
            onCommitted: n => root.writeShape({ radius: Math.max(0.001, n) }) } }
    InspectorRow { label: "Height"; visible: root.shapeKind === "Capsule"; Layout.fillWidth: true
        NumberField { objectName: "capsule-height"; value: root.shape && root.shape.height !== undefined ? root.shape.height : 2; fallback: 2
            onCommitted: n => root.writeShape({ height: Math.max(0.001, n) }) } }
    InspectorRow { label: "Axis"; visible: root.shapeKind === "Capsule"; Layout.fillWidth: true
        ChoiceField { objectName: "capsule-axis"; options: root.axisChoices; value: root.shape && root.shape.axis ? root.shape.axis : "Y"; onChosen: v => root.writeShape({ axis: v }) } }
    InspectorRow { label: "Sideways"; visible: root.shapeKind === "Capsule2d"; Layout.fillWidth: true
        SwitchField { value: !!(root.shape && root.shape.horizontal); onToggled: on => root.writeShape({ horizontal: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "Mesh"; visible: root.shapeKind === "ConvexHull" || root.shapeKind === "TriangleMesh"; Layout.fillWidth: true
        AssetField { app: root.app; accept: ["model"]; value: root.shape && root.shape.mesh ? root.shape.mesh : ""; onCommitted: p => root.writeShape({ mesh: p }) } }
    InspectorRow { label: "Points"; visible: root.shapeKind === "Polygon" || root.shapeKind === "Chain"; Layout.fillWidth: true
        BwTextField { objectName: "shape-points"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
            text: root.shape && root.shape.points ? root.pointsText(root.shape.points) : ""
            onEditingFinished: { const p = root.parsePoints(text); if (p.length >= 2) root.writeShape({ points: p }); } } }
    InspectorRow { label: "Closed"; visible: root.shapeKind === "Chain"; Layout.fillWidth: true
        SwitchField { value: !!(root.shape && root.shape.closed); onToggled: on => root.writeShape({ closed: on }) } Item { Layout.fillWidth: true } }
    InspectorRow { label: "From"; visible: root.shapeKind === "Edge"; Layout.fillWidth: true
        Repeater { model: root.shapeKind === "Edge" ? 2 : 0
            delegate: NumberField { required property int index; value: root.shape.a[index]; onCommitted: n => root.writeShape({ a: root.withIndex(root.shape.a, index, n) }) } } }
    InspectorRow { label: "To"; visible: root.shapeKind === "Edge"; Layout.fillWidth: true
        Repeater { model: root.shapeKind === "Edge" ? 2 : 0
            delegate: NumberField { required property int index; value: root.shape.b[index]; onCommitted: n => root.writeShape({ b: root.withIndex(root.shape.b, index, n) }) } } }
    Text { visible: root.shapeKind === "Terrain" || root.shapeKind === "Tilemap"; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: root.shapeKind === "Terrain" ? "Uses this actor's Terrain heightfield." : "Uses the solid cells of this actor's tilemap." }
    BwButton {
        objectName: "fit-to-look"
        visible: root.geometryKind === "FromLook" || root.shapeKind === "Box" || root.shapeKind === "Rect" || root.shapeKind === "Sphere" || root.shapeKind === "Circle" || root.shapeKind === "Capsule" || root.shapeKind === "Capsule2d"
        text: root.geometryKind === "FromLook" ? "Make independent" : "Fit to the look"; iconName: "box"; implicitHeight: 28; font.pixelSize: 12
        onClicked: root.app.invoke("fit_collider_to_look", { colliderId: root.k.id })
    }

    // ─── Where it stands in the actor ──────────────────────────────────────
    InspectorRow { label: "Centre"; Layout.fillWidth: true
        Repeater { model: root.is3d ? 3 : 2
            delegate: NumberField { required property int index; objectName: "center-" + index; value: root.k.center[index]
                onCommitted: n => root.write({ center: root.withIndex(root.k.center, index, n) }) } } }
    InspectorRow { label: "Rotation deg"; Layout.fillWidth: true
        Repeater { model: root.is3d ? 3 : 1
            delegate: NumberField { required property int index; readonly property int axis: root.is3d ? index : 2
                objectName: "rotation-" + axis; value: root.k.rotation[axis]
                onCommitted: n => root.write({ rotation: root.withIndex(root.k.rotation, axis, n) }) } } }

    // ─── Surface ───────────────────────────────────────────────────────────
    InspectorRow { label: "Material"; Layout.fillWidth: true
        ChoiceField { objectName: "collider-material"; options: root.materialChoices; value: root.materialValue; onChosen: v => root.setMaterial(v) } }
    InspectorRow { label: "Friction"; Layout.fillWidth: true
        NumberField { objectName: "override-friction"; value: root.overrides.dynamic_friction !== undefined ? root.overrides.dynamic_friction : null; allowEmpty: true; placeholderText: "from material"
            onCommitted: n => { const next = root.copy(root.overrides); if (n === null) { delete next.dynamic_friction; delete next.static_friction; } else { next.dynamic_friction = Math.max(0, n); next.static_friction = Math.max(0, n); } root.write({ material_overrides: next }); } } }
    InspectorRow { label: "Bounce"; Layout.fillWidth: true
        NumberField { objectName: "override-bounce"; value: root.overrides.bounciness !== undefined ? root.overrides.bounciness : null; allowEmpty: true; placeholderText: "from material"
            onCommitted: n => root.setOverride("bounciness", n) } }

    // ─── Filtering ─────────────────────────────────────────────────────────
    InspectorRow { label: "Layer"; Layout.fillWidth: true
        ChoiceField { objectName: "collider-layer"; options: root.layerChoices; value: String(root.k.layer); onChosen: v => root.write({ layer: Number(v) }) } }
    Repeater {
        model: [{ key: "include", label: "Also hits" }, { key: "exclude", label: "Never hits" }]
        delegate: InspectorRow {
            required property var modelData
            label: modelData.label; Layout.fillWidth: true
            Flow {
                Layout.fillWidth: true; spacing: 2
                Repeater {
                    model: 32
                    delegate: Rectangle {
                        required property int index
                        readonly property bool on: ((root.k.layer_overrides[modelData.key] >>> index) & 1) === 1
                        width: 18; height: 18; radius: 3
                        color: on ? Theme.accent : Theme.field; border.color: Theme.border
                        Text { anchors.centerIn: parent; text: index + 1; color: parent.on ? "white" : Theme.textDim; font.pixelSize: 9 }
                        MouseArea {
                            anchors.fill: parent; cursorShape: Qt.PointingHandCursor; hoverEnabled: true
                            ToolTip.visible: containsMouse; ToolTip.text: root.layerName(index + 1)
                            onClicked: root.setLayerBit(modelData.key, index)
                        }
                    }
                }
            }
        }
    }
    InspectorRow { label: "Override rank"; visible: root.k.layer_overrides.include !== 0 || root.k.layer_overrides.exclude !== 0; Layout.fillWidth: true
        NumberField { objectName: "override-priority"; value: root.k.layer_overrides.priority
            onCommitted: n => root.write({ layer_overrides: Object.assign(root.copy(root.k.layer_overrides), { priority: Math.round(n) }) }) } }
    InspectorRow { label: "Contact offset"; Layout.fillWidth: true
        NumberField { objectName: "contact-offset"; value: root.k.contact_offset; allowEmpty: true; placeholderText: "default"
            onCommitted: n => root.write({ contact_offset: n === null ? null : Math.max(0, n) }) } }
}
