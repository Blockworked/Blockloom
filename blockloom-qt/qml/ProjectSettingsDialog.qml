import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Project settings as a page tree: the project's own pages, a page per world
// setting the type registry describes, and a page per plugin with settings.
StitchDialog {
    id: root
    required property var app
    property string page: "project"
    property string filter: ""
    title: "Project settings"
    standardButtons: Dialog.Close
    width: Math.min(parent ? parent.width - 80 : 860, 860)
    height: Math.min(parent ? parent.height - 80 : 720, 720)
    function showPage(next) { page = next; open(); }

    function labelOf(field) {
        if (field.ui && field.ui.label) return field.ui.label;
        const spaced = String(field.name).replace(/_/g, " ");
        return spaced.charAt(0).toUpperCase() + spaced.slice(1);
    }
    // Every label under a field, so the filter finds a page by what it holds.
    function wordsOf(fields, depth) {
        let out = "";
        for (const f of fields || []) {
            out += " " + root.labelOf(f);
            if (depth < 3) out += root.wordsOf(f.fields, depth + 1);
        }
        return out;
    }
    readonly property var registry: app.typeRegistry || ({ world: [] })
    // Plain world values share one page; `mode` has its own command.
    readonly property var basics: (registry.world || []).filter(f => f.type !== "struct" && f.type !== "list" && f.name !== "mode")
    readonly property var worldPages: (registry.world || []).filter(f => f.type === "struct" || f.type === "list")
    readonly property var pluginsWithSettings: {
        const types = app.appState && app.appState.plugins && app.appState.plugins.types ? app.appState.plugins.types : [];
        const out = [];
        for (const t of types)
            if (t.kind === "resource" && !out.some(p => p.id === t.plugin)) out.push({ id: t.plugin, name: t.pluginName || t.plugin });
        return out;
    }
    // The tree, flattened: headings (no id) and pages at a depth.
    readonly property var pages: {
        const out = [
            { id: "project", label: "General", depth: 0, name: "settings-general" },
            { id: "multiplayer", label: "Multiplayer", depth: 0, name: "settings-multiplayer" },
            { label: "Publishing", depth: 0 },
            { id: "publishing", label: "App Info", depth: 1, name: "settings-app-info" },
            { id: "android", label: "Android", depth: 1, name: "settings-android" },
            { label: "World", depth: 0 }
        ];
        if (root.basics.length) out.push({ id: "world:", label: "Basics", depth: 1, words: root.wordsOf(root.basics, 0) });
        for (const f of root.worldPages)
            out.push({ id: "world:" + f.name, label: root.labelOf(f), depth: 1, words: root.wordsOf(f.fields, 0) });
        if (root.pluginsWithSettings.length) {
            out.push({ label: "Plugins", depth: 0 });
            for (const p of root.pluginsWithSettings) out.push({ id: "plugin:" + p.id, label: p.name, depth: 1 });
        }
        return out.map(p => Object.assign({ name: p.id ? "settings-page-" + p.id : "" }, p));
    }
    readonly property var shownPages: {
        const words = root.filter.toLowerCase().split(/\s+/).filter(w => w.length);
        if (!words.length) return root.pages;
        const hit = p => p.id && words.every(w => (p.label + " " + (p.words || "")).toLowerCase().indexOf(w) >= 0);
        return root.pages.filter(hit);
    }
    readonly property string worldField: page.startsWith("world:") ? page.slice(6) : ""
    readonly property var worldSchema: worldField ? (registry.world || []).find(f => f.name === worldField) || null : null
    readonly property var world: app.appState && app.appState.project ? app.appState.project.world : ({})
    function writeWorld(path, value) {
        app.invoke("set_property", { target: "world", path: path, value: value }, null, e => app.invoke("push_log", { kind: "error", text: String(e) }));
    }

    RowLayout {
        anchors.fill: parent; spacing: 0
        Rectangle {
            Layout.preferredWidth: 200; Layout.fillHeight: true
            color: Theme.panel
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 8; spacing: 6
                StitchTextField {
                    objectName: "settings-filter"
                    Layout.fillWidth: true
                    placeholderText: "Find a setting"
                    onTextChanged: root.filter = text
                }
                StitchScrollView {
                    id: tree
                    Layout.fillWidth: true; Layout.fillHeight: true
                    clip: true
                    contentWidth: availableWidth
                    Column {
                        width: tree.availableWidth
                        spacing: 2
                        Repeater {
                            model: root.shownPages
                            delegate: Item {
                                id: entry
                                required property var modelData
                                width: parent.width
                                height: modelData.id ? 30 : 28
                                Text {
                                    visible: !entry.modelData.id
                                    anchors.verticalCenter: parent.verticalCenter; x: 8
                                    text: entry.modelData.label; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold
                                }
                                StitchButton {
                                    objectName: entry.modelData.name
                                    visible: !!entry.modelData.id
                                    x: entry.modelData.depth * 12; width: parent.width - x; height: 30
                                    text: entry.modelData.label
                                    primary: root.page === entry.modelData.id
                                    onClicked: root.page = entry.modelData.id
                                }
                            }
                        }
                    }
                }
            }
        }
        Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; color: Theme.borderSoft }
        SettingsFields {
            objectName: "project-settings-fields"
            visible: !root.worldField && !root.page.startsWith("plugin:") && root.page !== "world:"
            Layout.fillWidth: true; Layout.fillHeight: true; app: root.app; page: visible ? root.page : "project"
        }
        ScrollView {
            id: other
            visible: !!root.worldField || root.page === "world:" || root.page.startsWith("plugin:")
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: other.availableWidth - 24; x: 12; y: 12
                spacing: 10
                Text {
                    text: (root.pages.find(p => p.id === root.page) || { label: "" }).label
                    color: Theme.text; font.pixelSize: 16; font.weight: Font.Bold
                }
                Text {
                    visible: root.page.startsWith("world:")
                    Layout.fillWidth: true; wrapMode: Text.WordWrap
                    text: "These apply to the open scene."; color: Theme.textDim; font.pixelSize: 12
                }
                RegistryForm {
                    objectName: "world-basics-form"
                    visible: root.page === "world:"
                    app: root.app
                    labelWidth: 130
                    fields: root.basics
                    value: root.world
                    onChanged: next => { for (const f of root.basics) if (JSON.stringify(next[f.name]) !== JSON.stringify(root.world[f.name])) root.writeWorld(f.name, next[f.name]); }
                }
                RegistryForm {
                    objectName: "world-page-form"
                    visible: !!root.worldSchema
                    app: root.app
                    labelWidth: 130
                    fields: root.worldSchema ? (root.worldSchema.type === "struct" ? root.worldSchema.fields : [root.worldSchema]) : []
                    value: root.worldSchema ? (root.worldSchema.type === "struct" ? (root.world[root.worldField] || ({})) : root.world) : ({})
                    onChanged: next => root.writeWorld(root.worldField, root.worldSchema.type === "struct" ? next : next[root.worldField])
                }
                PluginSettingsPage {
                    objectName: "plugin-settings-page"
                    visible: root.page.startsWith("plugin:")
                    Layout.fillWidth: true
                    app: root.app
                    plugin: visible ? root.page.slice(7) : ""
                }
            }
        }
    }
}
