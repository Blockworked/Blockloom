import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The panels installed plugins declare: text, a form for one of the plugin's
// resources, and buttons that run its commands. The editor draws all of it
// from the schema, so a plugin brings no UI code.
BwDialog {
    id: root
    required property var app
    property int current: 0
    property string message: ""
    property string error: ""
    // Arguments typed for each command button, by "<panel>/<index>".
    property var drafts: ({})
    readonly property var plugins: app.appState && app.appState.plugins ? app.appState.plugins : null
    readonly property var panels: plugins && plugins.panels ? plugins.panels : []
    readonly property var panel: panels.length > 0 ? panels[Math.min(current, panels.length - 1)] : null
    readonly property var actorOptions: app.appState && app.appState.project
        ? [{ value: "", label: "nothing" }].concat(app.appState.project.actors.map(a => ({ value: a.id, label: a.name })))
        : []
    title: panel ? panel.panel.title : "Plugin panels"
    standardButtons: Dialog.NoButton
    width: Math.min(parent ? parent.width - 80 : 560, 560)
    height: Math.min(parent ? parent.height - 80 : 640, 640)
    onOpened: { message = ""; error = ""; }

    function typeNamed(name) { return plugins && plugins.types ? plugins.types.find(t => t.name === name) || null : null; }
    function resourceType(item) { return typeNamed(panel.plugin + "/" + item.resource); }
    function resourcePayload(item) {
        const records = app.appState && app.appState.project && app.appState.project.plugin_resources ? app.appState.project.plugin_resources : [];
        const r = records.find(x => x.plugin === panel.plugin && x.type_id === item.resource);
        return r ? r.payload : ({});
    }
    function writeResource(item, next) {
        app.invoke("set_plugin_resource", { resource: panel.plugin + "/" + item.resource, payload: next }, null, e => root.error = String(e));
    }
    // A command's arguments as the form type the inspector's forms take.
    function commandType(item) {
        const info = panel.commands ? panel.commands[item.command] : null;
        const fields = info ? info.args : [];
        const defaults = {};
        for (const f of fields) if (f["default"] !== undefined) defaults[f.name] = f["default"];
        return { fields: fields, defaults: defaults };
    }
    function draftKey(index) { return panel.panel.name + "/" + index; }
    function setDraft(index, next) {
        const all = Object.assign({}, drafts);
        all[draftKey(index)] = next;
        drafts = all;
    }
    function runCommand(item, index) {
        message = ""; error = "";
        app.invoke("plugin_call", { command: panel.plugin + "/" + item.command, args: drafts[draftKey(index)] || ({}) },
            () => root.message = (item.label || item.command) + " done.", e => root.error = String(e));
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 8
        ChoiceField {
            visible: root.panels.length > 1
            options: root.panels.map((p, i) => ({ value: i, label: p.panel.title + "  (" + p.pluginName + ")" }))
            value: root.current
            onChosen: v => root.current = v
        }
        ScrollView {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width; spacing: 10
                Text {
                    visible: !!(root.panel && root.panel.panel.description)
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                    text: root.panel ? root.panel.panel.description || "" : ""
                }
                Repeater {
                    model: root.panel ? root.panel.panel.items : []
                    delegate: ColumnLayout {
                        id: item
                        required property var modelData
                        required property int index
                        objectName: "panel-item-" + index
                        Layout.fillWidth: true; spacing: 6
                        Text {
                            visible: item.modelData.kind === "text"
                            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 13
                            text: item.modelData.text || ""
                        }
                        PluginRecordForm {
                            visible: item.modelData.kind === "resource" && !!root.resourceType(item.modelData)
                            app: root.app
                            type: item.modelData.kind === "resource" && root.resourceType(item.modelData) ? root.resourceType(item.modelData) : ({ fields: [], defaults: {} })
                            payload: item.modelData.kind === "resource" ? root.resourcePayload(item.modelData) : ({})
                            actors: root.actorOptions
                            onChanged: next => root.writeResource(item.modelData, next)
                        }
                        PluginRecordForm {
                            visible: item.modelData.kind === "command" && type.fields.length > 0
                            app: root.app
                            type: item.modelData.kind === "command" && root.panel ? root.commandType(item.modelData) : ({ fields: [], defaults: {} })
                            payload: root.drafts[root.draftKey(item.index)] || ({})
                            actors: root.actorOptions
                            onChanged: next => root.setDraft(item.index, next)
                        }
                        BwButton {
                            visible: item.modelData.kind === "command"
                            objectName: "panel-run-" + item.index
                            text: item.modelData.label || item.modelData.command || ""
                            onClicked: root.runCommand(item.modelData, item.index)
                        }
                    }
                }
            }
        }
        Text { visible: root.message !== ""; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12; text: root.message }
        Text { visible: root.error !== ""; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: "#ff8a8a"; font.pixelSize: 12; text: root.error }
        RowLayout { Layout.fillWidth: true; Item { Layout.fillWidth: true } BwButton { text: "Close"; onClicked: root.close() } }
    }
}
