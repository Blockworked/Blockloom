import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "Plugins"
    visible: true
    width: 1000; height: 900
    when: windowShown
    property var dialog: null
    property var calls: []
    property var installed: [{
        id: "com.example.health", name: "Health", version: "1.0.0", description: "Hit points.", license: "MIT", tier: "declarative",
        dev: false, support: { kind: "native" }, dependencies: [], components: ["com.example.health/Health"], blocks: 1, commands: 3
    }]
    property var types: [
        { name: "com.example.health/Health", plugin: "com.example.health", pluginName: "Health", type: "Health", displayName: "Health", kind: "component", version: 2, fields: [], defaults: {} },
        { name: "com.example.health/Difficulty", plugin: "com.example.health", pluginName: "Health", type: "Difficulty", displayName: "Difficulty", kind: "resource", version: 1,
          fields: [{ name: "damage_scale", type: "number", min: 0, max: 10 }], defaults: { damage_scale: 1 } }
    ]
    QtObject {
        id: backend
        property var appState: ({ project: { name: "Demo", actors: [], plugin_resources: [{ plugin: "com.example.health", type_id: "Difficulty", schema_version: 1, payload: { damage_scale: 2 } }] } })
        function toFileUrl(path) { return "file://" + path; }
        function fromFileUrl(path) { return String(path).replace("file://", ""); }
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (command === "plugin_list") done({ installed: test.installed, types: test.types, problems: [], history: 1, offline: false });
            else if (command === "plugin_check") done({ ok: true, canRun: true, issues: [], problems: [] });
            else if (command === "plugin_commands") done({ commands: [{ name: "com.example.health/set_hp", summary: "Set hit points." }] });
            else if (command === "plugin_update") done({ changes: [{ change: "changed", id: "com.example.health", from: "1.0.0", to: "1.1.0" }] });
            else if (command === "plugin_remove") failed("nothing to remove");
            else if (done) done({});
        }
    }
    Component { id: factory; Editor.PluginManagerDialog { app: backend } }
    function init() {
        calls = [];
        dialog = createTemporaryObject(factory, test);
        verify(dialog !== null);
        dialog.open();
        tryCompare(dialog, "opened", true);
    }
    function cleanup() { dialog.destroy(); dialog = null; }
    function count(command) { return calls.filter(c => c.command === command).length; }
    function test_openingListsEverything() {
        compare(count("plugin_list"), 1);
        compare(count("plugin_check"), 1);
        compare(count("plugin_commands"), 1);
        compare(dialog.listing.installed.length, 1);
        compare(dialog.commands.length, 1);
    }
    function test_aChangeIsSummarisedAndRefreshes() {
        dialog.run("plugin_update", { ids: ["com.example.health"] }, false);
        compare(dialog.message, "Changed com.example.health 1.0.0 to 1.1.0");
        compare(count("plugin_list"), 2);
        verify(!dialog.busy);
    }
    function test_aFailureIsShownAndLeavesTheDialogUsable() {
        dialog.run("plugin_remove", { id: "com.example.health" }, false);
        compare(dialog.error, "nothing to remove");
        verify(!dialog.busy);
    }
    function test_issuesSayWhatIsWrong() {
        const text = dialog.issueText({ status: "needs_migration", from: 1, to: 2, record: "Health", location: "actor Ball", blocks_run: true });
        verify(text.indexOf("schema 1") >= 0);
        verify(text.indexOf("stops Play") >= 0);
        verify(dialog.issueText({ status: "missing", reason: "not installed", record: "Health", location: "actor Ball", blocks_run: false }).indexOf("not installed") >= 0);
    }
    function test_aPreviewSaysNothingChanged() {
        verify(dialog.summary({ changes: [] }, true).indexOf("nothing changed") >= 0);
        verify(dialog.summary({ changes: [], keptRecords: ["Health (actor Ball)"] }, false).indexOf("Kept 1") >= 0);
    }
    function test_resourcesAreEditedFromTheirSchema() {
        compare(dialog.resourceTypes.length, 1);
        compare(dialog.resourcePayload(dialog.resourceTypes[0]).damage_scale, 2);
        verify(findChild(dialog.contentItem, "resource-com.example.health/Difficulty") !== null);
        dialog.writeResource(dialog.resourceTypes[0], { damage_scale: 3 });
        const set = calls.filter(c => c.command === "set_plugin_resource")[0];
        compare(set.args.resource, "com.example.health/Difficulty");
        compare(set.args.payload.damage_scale, 3);
        dialog.resetResource(dialog.resourceTypes[0]);
        compare(count("remove_plugin_resource"), 1);
    }
}
