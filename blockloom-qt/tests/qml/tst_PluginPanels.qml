import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "PluginPanels"
    visible: true
    width: 800; height: 800
    when: windowShown
    property var calls: []
    property var panel: ({
        plugin: "com.example.lab", pluginName: "Lab",
        panel: { name: "main", title: "Lab", description: "Tools.", items: [
            { kind: "text", text: "Hello" },
            { kind: "resource", resource: "Settings" },
            { kind: "command", command: "spawn", label: "Spawn" }
        ] },
        commands: { spawn: { summary: "Spawn.", args: [{ name: "count", type: "int", min: 1, max: 9, default: 3 }] } }
    })
    QtObject {
        id: backend
        property var appState: ({
            plugins: {
                panels: [test.panel],
                types: [{ name: "com.example.lab/Settings", plugin: "com.example.lab", pluginName: "Lab", type: "Settings", displayName: "Settings", kind: "resource", version: 1,
                          fields: [{ name: "level", type: "int", min: 0, max: 5 }], defaults: { level: 1 } }]
            },
            project: { name: "Demo", actors: [], plugin_resources: [{ plugin: "com.example.lab", type_id: "Settings", schema_version: 1, payload: { level: 4 } }] }
        })
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (done) done({});
        }
    }
    Component { id: factory; Editor.PluginPanelDialog { app: backend } }
    function init() { test.calls = []; }

    function test_aPanelReadsItsResourceFromTheProject() {
        const d = createTemporaryObject(factory, test);
        compare(d.panels.length, 1);
        compare(d.resourceType({ resource: "Settings" }).displayName, "Settings");
        compare(d.resourcePayload({ resource: "Settings" }).level, 4);
    }
    function test_editingTheResourceWritesItThroughTheBackend() {
        const d = createTemporaryObject(factory, test);
        d.writeResource({ resource: "Settings" }, { level: 2 });
        compare(test.calls[0].command, "set_plugin_resource");
        compare(test.calls[0].args.resource, "com.example.lab/Settings");
        compare(test.calls[0].args.payload.level, 2);
    }
    function test_aCommandFormStartsFromTheArgumentDefaults() {
        const d = createTemporaryObject(factory, test);
        const type = d.commandType({ command: "spawn" });
        compare(type.fields.length, 1);
        compare(type.defaults.count, 3);
    }
    function test_aCommandButtonRunsTheQualifiedCommandWithItsDraft() {
        const d = createTemporaryObject(factory, test);
        d.setDraft(2, { count: 5 });
        d.runCommand({ command: "spawn", label: "Spawn" }, 2);
        compare(test.calls[0].command, "plugin_call");
        compare(test.calls[0].args.command, "com.example.lab/spawn");
        compare(test.calls[0].args.args.count, 5);
        compare(d.message, "Spawn done.");
    }
}
