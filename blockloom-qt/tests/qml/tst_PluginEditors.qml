import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "PluginEditors"
    visible: true
    width: 800; height: 800
    when: windowShown
    property var calls: []
    property bool trusted: false
    QtObject {
        id: backend
        property var appState: ({
            plugins: { editorModules: [{
                plugin: "com.example.stamp", pluginName: "Stamp", hash: "h", trusted: test.trusted, changed: false,
                modules: [{ path: "editor/StampPanel.qml", title: "Stamp panel", file: test.trusted ? "/tmp/StampPanel.qml" : null }]
            }] },
            project: { name: "Demo", actors: [], plugin_resources: [{ plugin: "com.example.stamp", type_id: "Settings", schema_version: 1, payload: { size: 3 } }] }
        })
        function toFileUrl(path) { return "file://" + path; }
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (done) done({});
        }
    }
    Component { id: factory; Editor.PluginEditorsDialog { app: backend } }
    function init() { test.calls = []; test.trusted = false; }

    function test_anUntrustedPluginOffersTrustAndNoFile() {
        const d = createTemporaryObject(factory, test);
        compare(d.shipped.length, 1);
        compare(d.shipped[0].modules[0].file, null);
        d.trust("com.example.stamp");
        compare(test.calls[0].command, "plugin_trust");
        compare(test.calls[0].args.id, "com.example.stamp");
    }
    function test_trustCanBeTakenBack() {
        const d = createTemporaryObject(factory, test);
        d.revoke("com.example.stamp");
        compare(test.calls[0].command, "plugin_untrust");
    }
    function test_aModuleOnlyShowsWhileItsPluginIsTrusted() {
        test.trusted = true;
        const d = createTemporaryObject(factory, test);
        d.show(d.shipped[0], d.shipped[0].modules[0]);
        verify(d.live !== null);
        compare(d.live.title, "Stamp panel");
        test.trusted = false;
        compare(d.live, null);
    }
}
