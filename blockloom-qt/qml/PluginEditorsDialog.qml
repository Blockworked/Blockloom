import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Editor screens installed plugins ship as QML. That code runs with the
// editor's own access, so a plugin's modules load only after the user trusts
// that exact package here; an update asks again. Nothing else loads them.
BwDialog {
    id: root
    required property var app
    // The module showing: {plugin, pluginName, title, file}, or null for the list.
    property var showing: null
    property string error: ""
    readonly property var plugins: app.appState && app.appState.plugins ? app.appState.plugins : null
    readonly property var shipped: plugins && plugins.editorModules ? plugins.editorModules : []
    // A module stays up only while its plugin is still trusted.
    readonly property var live: showing && shipped.some(p => p.plugin === showing.plugin && p.trusted) ? showing : null
    title: live ? live.title + " (" + live.pluginName + ")" : "Plugin editors"
    standardButtons: Dialog.NoButton
    width: Math.min(parent ? parent.width - 80 : 760, 760)
    height: Math.min(parent ? parent.height - 80 : 640, 640)
    onOpened: { showing = null; error = ""; }

    function trust(id) { error = ""; app.invoke("plugin_trust", { id: id }, null, e => root.error = String(e)); }
    function revoke(id) { error = ""; app.invoke("plugin_untrust", { id: id }, null, e => root.error = String(e)); }
    function show(plugin, module) {
        error = "";
        showing = { plugin: plugin.plugin, pluginName: plugin.pluginName, title: module.title, file: module.file };
    }

    // What a module is handed as `host`. It can reach everything through `app`;
    // the rest is the short way to the plugin's own commands and resources.
    QtObject {
        id: hostApi
        readonly property string plugin: root.live ? root.live.plugin : ""
        readonly property var app: root.app
        readonly property var project: root.app.appState ? root.app.appState.project : null
        // The saved payload of one of the plugin's resources ({} until written).
        function resource(name) {
            const all = project && project.plugin_resources ? project.plugin_resources : [];
            const r = all.find(x => x.plugin === plugin && x.type_id === name);
            return r ? r.payload : ({});
        }
        function setResource(name, payload, done, failed) {
            root.app.invoke("set_plugin_resource", { resource: plugin + "/" + name, payload: payload }, done, failed);
        }
        // Runs one of the plugin's commands.
        function call(command, args, done, failed) {
            root.app.invoke("plugin_call", { command: plugin + "/" + command, args: args || ({}) }, done, failed);
        }
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 8
        // The list of plugins that ship editor code.
        ScrollView {
            visible: !root.live
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width; spacing: 10
                Text {
                    visible: root.shipped.length === 0
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                    text: "No installed plugin ships editor screens."
                }
                Repeater {
                    model: root.shipped
                    delegate: Rectangle {
                        id: entry
                        required property var modelData
                        required property int index
                        objectName: "editor-plugin-" + index
                        Layout.fillWidth: true
                        implicitHeight: body.implicitHeight + 20
                        radius: 6; color: Theme.panelRaised; border.color: Theme.borderSoft
                        ColumnLayout {
                            id: body
                            anchors.fill: parent; anchors.margins: 10; spacing: 6
                            RowLayout {
                                Layout.fillWidth: true
                                Text { Layout.fillWidth: true; text: entry.modelData.pluginName; color: Theme.text; font.pixelSize: 14; font.bold: true; elide: Text.ElideRight }
                                BwButton {
                                    objectName: "editor-trust-" + entry.index
                                    visible: !entry.modelData.trusted
                                    text: entry.modelData.changed ? "Trust the new version" : "Trust"
                                    onClicked: root.trust(entry.modelData.plugin)
                                }
                                BwButton {
                                    objectName: "editor-revoke-" + entry.index
                                    visible: entry.modelData.trusted
                                    text: "Stop trusting"
                                    onClicked: root.revoke(entry.modelData.plugin)
                                }
                            }
                            Text {
                                visible: !entry.modelData.trusted
                                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: "#e0b060"; font.pixelSize: 12
                                text: (entry.modelData.changed ? "This plugin changed since you trusted it. " : "")
                                    + "Its screens run code inside Blockloom with the same access you have. Trust it only if you trust where it came from."
                            }
                            Text {
                                visible: !!entry.modelData.inspectors && entry.modelData.inspectors.length > 0
                                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                                text: "Draws its own inspector section for: " + (entry.modelData.inspectors || []).map(i => i.component).join(", ")
                            }
                            Flow {
                                visible: entry.modelData.trusted
                                Layout.fillWidth: true; spacing: 6
                                Repeater {
                                    model: entry.modelData.modules
                                    delegate: BwButton {
                                        required property var modelData
                                        objectName: "editor-open-" + modelData.title
                                        text: modelData.title
                                        onClicked: root.show(entry.modelData, modelData)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // One module, loaded from the plugin's own file.
        Loader {
            id: screen
            objectName: "editor-screen"
            visible: !!root.live
            Layout.fillWidth: true; Layout.fillHeight: true
            active: !!root.live
            onActiveChanged: if (!active) source = ""
            Connections {
                target: root
                function onLiveChanged() {
                    if (root.live) screen.setSource(root.app.toFileUrl(root.live.file), { host: hostApi });
                }
            }
            onStatusChanged: if (status === Loader.Error) root.error = "The module could not be loaded. See the log for why."
        }
        Text { visible: root.error !== ""; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: "#ff8a8a"; font.pixelSize: 12; text: root.error }
        RowLayout {
            Layout.fillWidth: true
            BwButton { visible: !!root.live; text: "Back"; onClicked: root.showing = null }
            Item { Layout.fillWidth: true }
            BwButton { text: "Close"; onClicked: root.close() }
        }
    }
}
