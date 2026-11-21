import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// One plugin's settings: each resource type it declares, drawn from its
// schema and written through set_plugin_resource. Empty `plugin` lists all.
ColumnLayout {
    id: root
    required property var app
    property string plugin: ""
    property string error: ""
    spacing: 10
    readonly property var types: app.appState && app.appState.plugins && app.appState.plugins.types ? app.appState.plugins.types : []
    readonly property var resourceTypes: types.filter(t => t.kind === "resource" && (!root.plugin || t.plugin === root.plugin))
    readonly property var resourceRecords: app.appState && app.appState.project && app.appState.project.plugin_resources ? app.appState.project.plugin_resources : []
    function resourceRecord(name) { return resourceRecords.find(r => r.plugin + "/" + r.type_id === name) || null; }
    function resourcePayload(type) { const r = resourceRecord(type.name); return r ? r.payload : ({}); }
    function writeResource(type, payload) {
        error = "";
        app.invoke("set_plugin_resource", { resource: type.name, payload: payload }, null, e => root.error = String(e));
    }
    function resetResource(type) {
        error = "";
        app.invoke("remove_plugin_resource", { resource: type.name }, null, e => root.error = String(e));
    }

    Text {
        visible: root.resourceTypes.length === 0
        text: "This plugin has no settings."; color: Theme.textDim; font.pixelSize: 12
    }
    Text { visible: root.error.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; text: root.error; color: Theme.danger; font.pixelSize: 12 }
    Repeater {
        model: root.resourceTypes
        delegate: Rectangle {
            id: resource
            required property var modelData
            objectName: "resource-" + modelData.name
            Layout.fillWidth: true
            implicitHeight: resourceColumn.implicitHeight + 16
            radius: 8; color: Theme.panelRaised; border.color: Theme.borderSoft
            ColumnLayout {
                id: resourceColumn
                anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
                anchors.margins: 8; spacing: 6
                RowLayout {
                    Layout.fillWidth: true; spacing: 8
                    Text {
                        Layout.fillWidth: true; elide: Text.ElideRight
                        color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold
                        text: resource.modelData.displayName + (root.plugin ? "" : "  (" + resource.modelData.pluginName + ")")
                    }
                    StitchButton {
                        text: "Reset"; flat: true
                        visible: !!root.resourceRecord(resource.modelData.name)
                        onClicked: root.resetResource(resource.modelData)
                    }
                }
                PluginRecordForm {
                    app: root.app; type: resource.modelData; payload: root.resourcePayload(resource.modelData)
                    actors: root.app.appState && root.app.appState.project
                        ? [{ value: "", label: "nothing" }].concat(root.app.appState.project.actors.map(a => ({ value: a.id, label: a.name })))
                        : []
                    onChanged: next => root.writeResource(resource.modelData, next)
                }
            }
        }
    }
}
