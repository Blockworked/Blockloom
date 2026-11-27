import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "Search"
    visible: true
    width: 900; height: 600
    when: windowShown
    property var calls: []
    QtObject {
        id: backend
        property var assetTargets: []
        property var assetDrag: null
        property bool inspectScene: false
        property string inspectedScene: ""
        property string inspectedLighting: ""
        property var appState: ({
            project_path: "/tmp/p",
            project: {
                name: "P", active_scene: "s1", scenes: [{ id: "s1", name: "Main" }], world: { mode: "TwoD" },
                actors: [
                    { id: "a", name: "Player", components: [{ component: "CharacterMotor" }] },
                    { id: "b", name: "Coin", components: [{ component: "Parent", parent: "c" }] },
                    { id: "c", name: "Level", components: [] }
                ]
            }
        })
        function selectScene(id) {}
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (command === "find_assets") done([{ name: "Guard.png", path: "assets/enemies/Guard.png", kind: "Image", size: 1 }]);
            else if (command === "list_assets") done([]);
            else if (command === "pipeline_status") done([]);
            else if (command === "plugin_imports") done({ imports: [] });
            else if (done) done({});
        }
    }
    Component { id: actorsFactory; Editor.ActorList { app: backend; docked: true; width: 300; height: 400 } }
    Component { id: assetsFactory; Editor.AssetTray { app: backend; docked: true; width: 800; height: 300 } }
    function test_actorSearchIsFlatAndMatchesComponents() {
        const list = createTemporaryObject(actorsFactory, test);
        compare(list.rows.length, 3);
        list.search = "coin";
        compare(list.rows.length, 1);
        compare(list.rows[0].depth, 0, "a child shows on its own");
        list.search = "motor";
        compare(list.rows[0].actor.name, "Player");
    }
    function test_assetSearchLooksEverywhere() {
        const tray = createTemporaryObject(assetsFactory, test);
        calls = [];
        tray.query = "guard";
        tryVerify(() => calls.some(c => c.command === "find_assets" && c.args.query === "guard"));
        compare(tray.entries.length, 1);
        tray.goTo("assets");
        compare(tray.query, "");
    }
}
