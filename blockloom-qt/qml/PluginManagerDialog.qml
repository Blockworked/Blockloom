import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The open project's plugins: what is installed, what needs attention, and
// the package changes (install, update, remove, sync, roll back). Every row
// runs the same backend command the shell's plugin-* lines do, so a change
// here and one there are the same transaction.
BwDialog {
    id: root
    required property var app
    property var listing: null
    property var check: null
    property var commands: []
    property var inspected: null
    property string message: ""
    property string error: ""
    property bool busy: false
    title: "Plugins"
    standardButtons: Dialog.NoButton
    width: Math.min(parent ? parent.width - 80 : 700, 700)
    height: Math.min(parent ? parent.height - 80 : 720, 720)

    onOpened: {
        message = ""; error = ""; inspected = null;
        sourceField.text = ""; idField.text = "";
        refresh();
    }

    function refresh() {
        app.invoke("plugin_list", {}, result => root.listing = result, e => root.error = String(e));
        app.invoke("plugin_check", {}, result => root.check = result, e => root.error = String(e));
        app.invoke("plugin_commands", {}, result => root.commands = result.commands || [], e => root.error = String(e));
    }

    function changeText(change) {
        if (change.change === "added") return "Added " + change.id + " " + change.version;
        if (change.change === "removed") return "Removed " + change.id + " " + change.version;
        return "Changed " + change.id + " " + change.from + " to " + change.to;
    }
    function summary(result, dryRun) {
        const lines = [];
        const changes = result.changes || [];
        for (let i = 0; i < changes.length; ++i) lines.push(changeText(changes[i]));
        if (changes.length === 0 && result.removed === undefined) lines.push("Nothing to change.");
        const kept = result.keptRecords || [];
        if (kept.length > 0) lines.push("Kept " + kept.length + " record(s) the plugin owned: " + kept.join(", "));
        if (result.removed !== undefined) {
            const n = Array.isArray(result.removed) ? result.removed.length : result.removed;
            lines.push("Removed " + n + " cached package(s) nothing needs.");
        }
        return (dryRun ? "Preview, nothing changed:\n" : "") + lines.join("\n");
    }
    // Runs one backend command and says what it did. A failure leaves the
    // project's plugin files as they were: every change is a transaction.
    function run(command, args, dryRun) {
        if (busy) return;
        busy = true; message = ""; error = "";
        app.invoke(command, args, result => {
            root.busy = false;
            root.message = root.summary(result, dryRun);
            root.refresh();
        }, e => { root.busy = false; root.error = String(e); });
    }
    function inspect() {
        const source = sourceField.text.trim();
        if (source.indexOf("path:") !== 0) return;
        error = ""; message = "";
        app.invoke("plugin_inspect", { path: source.slice(5) }, result => {
            root.inspected = result;
            idField.text = result.id;
        }, e => { root.inspected = null; root.error = String(e); });
    }
    function issueText(issue) {
        let why = issue.status;
        if (issue.status === "needs_migration") why = "written at schema " + issue.from + ", the plugin has " + issue.to + " (plugin-migrate upgrades it)";
        else if (issue.status === "missing") why = "its plugin is not available: " + issue.reason;
        else if (issue.status === "unknown_type") why = "the plugin has no such type";
        else if (issue.status === "schema_too_new") why = "written at schema " + issue.found + ", the installed plugin reads up to " + issue.supported;
        else if (issue.status === "invalid") why = "its data fails the plugin's schema";
        return issue.record + " on " + issue.location + ": " + why + (issue.blocks_run ? " (stops Play)" : "");
    }
    // The project-wide records plugins own (their settings), edited in place.
    readonly property var resourceTypes: listing && listing.types ? listing.types.filter(t => t.kind === "resource") : []
    readonly property var resourceRecords: app.appState && app.appState.project && app.appState.project.plugin_resources ? app.appState.project.plugin_resources : []
    function resourceRecord(name) { return resourceRecords.find(r => r.plugin + "/" + r.type_id === name) || null; }
    function resourcePayload(type) { const r = resourceRecord(type.name); return r ? r.payload : ({}); }
    function writeResource(type, payload) {
        app.invoke("set_plugin_resource", { resource: type.name, payload: payload }, null, e => root.error = String(e));
    }
    function resetResource(type) {
        app.invoke("remove_plugin_resource", { resource: type.name }, null, e => root.error = String(e));
    }
    function supportText(support) {
        if (!support) return "";
        if (support.kind === "unsupported") return "Not available on this machine: " + support.reason;
        return support.kind === "portable" ? "Runs through the portable module" : "";
    }

    ScrollView {
        id: outerScroll
        anchors.fill: parent; clip: true
        contentWidth: availableWidth
        ColumnLayout {
            width: parent.width - 12; spacing: 8

            Text {
                visible: root.error.length > 0
                Layout.fillWidth: true; wrapMode: Text.WordWrap
                color: Theme.danger; font.family: "monospace"; font.pixelSize: 11
                text: root.error
            }
            Text {
                visible: root.message.length > 0
                Layout.fillWidth: true; wrapMode: Text.WordWrap
                color: Theme.text; font.pixelSize: 12
                text: root.message
            }
            Text {
                visible: !!root.listing && root.listing.offline
                Layout.fillWidth: true; wrapMode: Text.WordWrap
                color: Theme.warning; font.pixelSize: 12
                text: "Offline: only packages already in the cache or on disk can be installed."
            }

            Text { text: "Installed"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold }
            Text {
                visible: !root.listing || root.listing.installed.length === 0
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "No plugins are installed in this project."
            }
            Repeater {
                model: root.listing ? root.listing.installed : []
                delegate: Rectangle {
                    id: card
                    required property var modelData
                    Layout.fillWidth: true
                    implicitHeight: cardColumn.implicitHeight + 16
                    radius: 8; color: Theme.panelRaised; border.color: Theme.borderSoft
                    ColumnLayout {
                        id: cardColumn
                        anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
                        anchors.margins: 8; spacing: 4
                        RowLayout {
                            Layout.fillWidth: true; spacing: 8
                            Text {
                                Layout.fillWidth: true; elide: Text.ElideRight
                                color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold
                                text: card.modelData.name + "  " + card.modelData.version + (card.modelData.dev ? "  (local development)" : "")
                            }
                            BwButton {
                                text: "Update"; flat: true; enabled: !root.busy
                                onClicked: root.run("plugin_update", { ids: [card.modelData.id] }, false)
                            }
                            BwButton {
                                text: "Remove"; flat: true; danger: true; enabled: !root.busy
                                onClicked: root.run("plugin_remove", { id: card.modelData.id }, false)
                            }
                        }
                        Text {
                            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                            text: card.modelData.id + " - " + card.modelData.tier + " - " + card.modelData.license
                        }
                        Text {
                            visible: card.modelData.description.length > 0
                            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12
                            text: card.modelData.description
                        }
                        Text {
                            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                            text: card.modelData.components.length + " component(s), " + card.modelData.blocks + " block(s), " + card.modelData.commands + " command(s)"
                                + (card.modelData.dependencies.length > 0 ? ". Needs " + card.modelData.dependencies.join(", ") : "")
                        }
                        Text {
                            visible: root.supportText(card.modelData.support).length > 0
                            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.warning; font.pixelSize: 12
                            text: root.supportText(card.modelData.support)
                        }
                    }
                }
            }

            Text {
                visible: !!root.listing && root.listing.problems.length > 0
                text: "Failed to load"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold; Layout.topMargin: 6
            }
            Repeater {
                model: root.listing ? root.listing.problems : []
                delegate: Text {
                    required property var modelData
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.warning; font.pixelSize: 12
                    text: modelData.id + ": " + modelData.message
                }
            }

            Text {
                visible: !!root.check && root.check.issues.length > 0
                text: "Needs attention"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold; Layout.topMargin: 6
            }
            Repeater {
                model: root.check ? root.check.issues : []
                delegate: Text {
                    required property var modelData
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: 12
                    color: modelData.blocks_run ? Theme.warning : Theme.textDim
                    text: root.issueText(modelData)
                }
            }
            Text {
                visible: !!root.check && !root.check.canRun
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.warning; font.pixelSize: 12
                text: "Play and Build are stopped until these are fixed. The data is kept: reinstalling the plugin brings it back."
            }

            Text {
                visible: root.resourceTypes.length > 0
                text: "Settings"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold; Layout.topMargin: 6
            }
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
                                text: resource.modelData.displayName + "  (" + resource.modelData.pluginName + ")"
                            }
                            BwButton {
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

            Text { text: "Install"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold; Layout.topMargin: 6 }
            Text {
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "A source is path:<folder>, archive:<zip>, git:<url>#<commit> or registry:<name>. Leave it empty to look in the project's registries."
            }
            RowLayout {
                Layout.fillWidth: true
                BwTextField {
                    id: sourceField; Layout.fillWidth: true; placeholderText: "Source"
                    onEditingFinished: { root.inspected = null; root.inspect(); }
                }
                IconButton { iconName: "folder-open"; tip: "Pick a package folder"; implicitWidth: 34; implicitHeight: 34; onClicked: packageBrowse.open() }
            }
            RowLayout {
                Layout.fillWidth: true
                BwTextField { id: idField; Layout.fillWidth: true; placeholderText: "Plugin id, such as com.example.health" }
            }
            Text {
                visible: !!root.inspected
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12
                text: root.inspected
                    ? root.inspected.name + " " + root.inspected.version + " (" + root.inspected.tier + ", " + root.inspected.license + "): "
                        + root.inspected.components + " component(s), " + root.inspected.blocks + " block(s), " + root.inspected.commands + " command(s)."
                        + (root.inspected.capabilities.length > 0 ? " Asks for " + root.inspected.capabilities.join(", ") + "." : "")
                    : ""
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 8
                BwButton {
                    text: "Preview"; enabled: !root.busy && idField.text.trim() !== ""
                    onClicked: root.run("plugin_install", { id: idField.text.trim(), source: sourceField.text.trim(), dryRun: true }, true)
                }
                BwButton {
                    text: root.busy ? "Working..." : "Install"; primary: true
                    enabled: !root.busy && idField.text.trim() !== ""
                    onClicked: root.run("plugin_install", { id: idField.text.trim(), source: sourceField.text.trim() }, false)
                }
            }

            Text { text: "Project"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold; Layout.topMargin: 6 }
            Flow {
                Layout.fillWidth: true; spacing: 8
                BwButton {
                    text: "Update all"; enabled: !root.busy
                    onClicked: root.run("plugin_update", { ids: [] }, false)
                }
                BwButton {
                    text: "Install what the lock says"; enabled: !root.busy
                    onClicked: root.run("plugin_sync", {}, false)
                }
                BwButton {
                    text: "Undo last change"; enabled: !root.busy && !!root.listing && root.listing.history > 0
                    onClicked: root.run("plugin_rollback", {}, false)
                }
                BwButton {
                    text: "Clean the cache"; enabled: !root.busy
                    onClicked: root.run("plugin_gc", {}, false)
                }
            }

            Text {
                visible: root.commands.length > 0
                text: "Commands"; color: Theme.text; font.pixelSize: 14; font.weight: Font.Bold; Layout.topMargin: 6
            }
            Text {
                visible: root.commands.length > 0
                Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 12
                text: "Run one from the shell as plugin-id/name, or from a block."
            }
            Repeater {
                model: root.commands
                delegate: Text {
                    required property var modelData
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: 12
                    text: modelData.name + " - " + modelData.summary
                }
            }
        }
    }

    FolderDialog {
        id: packageBrowse
        title: "Pick a plugin package folder"
        onAccepted: {
            sourceField.text = "path:" + root.app.fromFileUrl(selectedFolder);
            root.inspected = null;
            root.inspect();
        }
    }
}
