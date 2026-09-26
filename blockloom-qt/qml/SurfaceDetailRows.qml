import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// World-space texturing rows for a Material or a Terrain: texture bombing,
// macro variation, a detail normal map and the slope/height/cavity/snow/wetness
// mask stack. `edited` hands back the whole detail with one field replaced.
ColumnLayout {
    id: root
    required property var app
    property var detail: ({})
    property bool terrain: false
    signal edited(var detail)
    spacing: 6

    readonly property var rule: ({ enabled: false, start: 35, blend: 10, invert: false, color: "#6E6A64", strength: 1, roughness: 0.8 })
    readonly property var d: {
        const m = Object.assign({}, detail.masks || {});
        return Object.assign({ stochastic: false, stochastic_contrast: 0.5, macro_texture: "", macro_size: 32, macro_strength: 0,
                               detail_texture: "", detail_size: 0.5, detail_strength: 0, detail_distance: 20 }, detail, {
            masks: {
                slope: Object.assign({}, rule, m.slope || {}),
                height: Object.assign({}, rule, m.height || {}),
                cavity: Object.assign({}, rule, { start: 0.5, blend: 0.2 }, m.cavity || {}),
                snow: Object.assign({ enabled: false, amount: 1, max_slope: 50, min_height: -100000, blend: 8, color: "#F2F5F8", roughness: 0.75 }, m.snow || {}),
                wetness: Object.assign({ enabled: false, amount: 1, darken: 0.4, roughness: 0.08, puddles: 0.5 }, m.wetness || {})
            }
        });
    }
    function set(next) { edited(Object.assign(JSON.parse(JSON.stringify(d)), next)); }
    function setMask(key, next) {
        const masks = JSON.parse(JSON.stringify(d.masks));
        masks[key] = Object.assign(masks[key], next);
        set({ masks: masks });
    }
    readonly property var rules: terrain ? [["slope", "Slope"], ["height", "Height"], ["cavity", "Cavity"]] : [["slope", "Slope"], ["height", "Height"]]

    InspectorRow { label: "Bombing"; Layout.fillWidth: true
        SwitchField { value: root.d.stochastic; onToggled: on => root.set({ stochastic: on }) }
        NumberField { visible: root.d.stochastic; value: root.d.stochastic_contrast; fallback: 0.5; onCommitted: n => root.set({ stochastic_contrast: n }) } }
    InspectorRow { label: "Macro"; Layout.fillWidth: true
        AssetField { app: root.app; accept: ["image"]; value: root.d.macro_texture; placeholderText: "Procedural noise"; onCommitted: p => root.set({ macro_texture: p }) } }
    InspectorRow { label: "Macro size / str"; Layout.fillWidth: true
        NumberField { value: root.d.macro_size; fallback: 32; onCommitted: n => root.set({ macro_size: n }) }
        NumberField { value: root.d.macro_strength; onCommitted: n => root.set({ macro_strength: n }) } }
    InspectorRow { label: "Detail normal"; Layout.fillWidth: true
        AssetField { app: root.app; accept: ["image"]; value: root.d.detail_texture; placeholderText: "Optional"; onCommitted: p => root.set({ detail_texture: p }) } }
    InspectorRow { label: "Detail size / str"; Layout.fillWidth: true; visible: root.d.detail_texture !== ""
        NumberField { value: root.d.detail_size; fallback: 0.5; onCommitted: n => root.set({ detail_size: n }) }
        NumberField { value: root.d.detail_strength; onCommitted: n => root.set({ detail_strength: n }) } }
    InspectorRow { label: "Detail fade"; Layout.fillWidth: true; visible: root.d.detail_texture !== ""
        NumberField { value: root.d.detail_distance; fallback: 20; onCommitted: n => root.set({ detail_distance: n }) } }

    Repeater {
        model: root.rules
        delegate: ColumnLayout {
            required property var modelData
            readonly property string key: modelData[0]
            readonly property var m: root.d.masks[key]
            Layout.fillWidth: true; spacing: 6
            InspectorRow { label: modelData[1] + " mask"; Layout.fillWidth: true
                SwitchField { value: m.enabled; onToggled: on => root.setMask(key, { enabled: on }) }
                ColorField { visible: m.enabled; value: m.color; onPicked: col => root.setMask(key, { color: col }) } }
            InspectorRow { label: "Start / blend"; Layout.fillWidth: true; visible: m.enabled
                NumberField { value: m.start; onCommitted: n => root.setMask(key, { start: n }) }
                NumberField { value: m.blend; onCommitted: n => root.setMask(key, { blend: n }) } }
            InspectorRow { label: "Str / rough"; Layout.fillWidth: true; visible: m.enabled
                NumberField { value: m.strength; fallback: 1; onCommitted: n => root.setMask(key, { strength: n }) }
                NumberField { value: m.roughness; fallback: 0.8; onCommitted: n => root.setMask(key, { roughness: n }) } }
            InspectorRow { label: "Invert"; Layout.fillWidth: true; visible: m.enabled
                SwitchField { value: m.invert; onToggled: on => root.setMask(key, { invert: on }) } Item { Layout.fillWidth: true } }
        }
    }
    InspectorRow { label: "Snow"; Layout.fillWidth: true
        SwitchField { value: root.d.masks.snow.enabled; onToggled: on => root.setMask("snow", { enabled: on }) }
        NumberField { visible: root.d.masks.snow.enabled; value: root.d.masks.snow.amount; fallback: 1; onCommitted: n => root.setMask("snow", { amount: n }) } }
    InspectorRow { label: "Max slope / min y"; Layout.fillWidth: true; visible: root.d.masks.snow.enabled
        NumberField { value: root.d.masks.snow.max_slope; fallback: 50; onCommitted: n => root.setMask("snow", { max_slope: n }) }
        NumberField { value: root.d.masks.snow.min_height; fallback: -100000; onCommitted: n => root.setMask("snow", { min_height: n }) } }
    InspectorRow { label: "Snow color"; Layout.fillWidth: true; visible: root.d.masks.snow.enabled
        ColorField { value: root.d.masks.snow.color; onPicked: col => root.setMask("snow", { color: col }) }
        NumberField { value: root.d.masks.snow.blend; fallback: 8; onCommitted: n => root.setMask("snow", { blend: n }) } }
    InspectorRow { label: "Wetness"; Layout.fillWidth: true
        SwitchField { value: root.d.masks.wetness.enabled; onToggled: on => root.setMask("wetness", { enabled: on }) }
        NumberField { visible: root.d.masks.wetness.enabled; value: root.d.masks.wetness.amount; fallback: 1; onCommitted: n => root.setMask("wetness", { amount: n }) } }
    InspectorRow { label: "Darken / puddles"; Layout.fillWidth: true; visible: root.d.masks.wetness.enabled
        NumberField { value: root.d.masks.wetness.darken; fallback: 0.4; onCommitted: n => root.setMask("wetness", { darken: n }) }
        NumberField { value: root.d.masks.wetness.puddles; fallback: 0.5; onCommitted: n => root.setMask("wetness", { puddles: n }) } }
    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
        text: "Masks tint by slope and height in world space. Snow and wetness also follow the project's weather and volumes; the Surface blend debug view shows what each one covers." }
}
