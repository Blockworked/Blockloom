import QtQuick
import QtTest
import "../../qml" as Editor
import "SettingsFixture.js" as Fixture

// The editor shell: menus and the palette come from one action table, keys
// from the backend's keymap, and the top bar keeps only identity and Play.
Item {
    id: scene
    width: 1200; height: 800

    QtObject {
        id: backend
        property var appState: ({ project: { name: "Demo", actors: [], plugin_resources: [], world: Fixture.world(), scenes: [], active_scene: "", default_scene: "" }, project_path: "/tmp/Demo",
                                  running: false, paused: false, can_undo: true, can_redo: false, runtime_open: false,
                                  plugins: { panels: [], editorModules: [], surfaces: { menus: [{ title: "Regenerate", group: "Voxel", command: "voxel/generate", args: {} }], shortcuts: [] } } })
        property var status: null
        property var assetTargets: []
        property var assetDrag: null
        property var calls: []
        property var problems: []
        function invoke(command, args, done, failed) {
            calls = calls.concat([{ command: command, args: args }]);
            if (command === "editor_commands" && done)
                done({ commands: [
                        { name: "save-project", cmd: "save_project", summary: "Save the project.", args: [] },
                        { name: "plugin-sync", cmd: "plugin_sync", summary: "Install what plugins.lock names.", args: [] },
                        { name: "add-actor", cmd: "add_actor", summary: "Add an actor.", args: [{ name: "name", ty: "text", required: true }] }
                     ], keymap: [{ action: "save_project", keys: ["Ctrl+S"] }, { action: "editor.palette", keys: ["Ctrl+Shift+P"] }] });
            else if (command === "problems" && done) done(problems);
            else if (done) done({});
        }
        function toFileUrl(p) { return "file://" + p; }
        function fromFileUrl(p) { return String(p); }
    }
    QtObject {
        id: shellPage
        property var ran: []
        function editorAction(id) { ran = ran.concat([id]); }
        function actionChecked(id) { return id === "editor.split"; }
    }

    TestCase {
        id: test
        name: "EditorShell"
        when: windowShown

        Component { id: actionsComponent; Editor.EditorActions { app: backend; page: shellPage } }
        Component { id: paletteComponent; Editor.CommandPalette { } }
        Component { id: problemsComponent; Editor.ProblemsPanel { app: backend; width: 600; height: 200 } }

        function init() { backend.calls = []; shellPage.ran = []; }
        function makeActions() {
            const actions = createTemporaryObject(actionsComponent, scene);
            verify(actions);
            tryVerify(() => actions.specs.length > 0);
            return actions;
        }

        function test_keysComeFromTheBackendKeymap() {
            const actions = makeActions();
            compare(actions.keyText("save_project"), "Ctrl+S");
            compare(actions.keyText("undo"), "", "unbound in this keymap");
        }

        function test_actionsFollowState() {
            const actions = makeActions();
            verify(actions.enabled("undo"));
            verify(!actions.enabled("redo"));
            verify(actions.enabled("run_project"));
            verify(!actions.enabled("stop_project"));
            verify(!actions.visible("editor.pluginPanels"), "no plugin panels installed");
            actions.run("redo");
            compare(backend.calls.filter(c => c.command === "redo").length, 0, "a disabled action does nothing");
            actions.run("undo");
            compare(backend.calls[backend.calls.length - 1].command, "undo");
            actions.run("editor.build");
            compare(shellPage.ran, ["editor.build"], "window actions go back to the page");
        }

        function test_thePaletteListsMenusAndArgumentlessCommands() {
            const actions = makeActions();
            const ids = actions.paletteEntries.map(e => e.id);
            verify(ids.indexOf("editor.projectSettings") >= 0);
            verify(ids.indexOf("plugin_sync") >= 0, "a shell command with no arguments");
            verify(ids.indexOf("add_actor") < 0, "one that needs arguments isn't offered");
            compare(ids.filter(i => i === "save_project").length, 1, "a menu action isn't listed twice");
        }

        function test_typingFiltersAndEnterRuns() {
            const actions = makeActions();
            const palette = createTemporaryObject(paletteComponent, scene, { actions: actions });
            palette.open();
            tryVerify(() => palette.opened);
            const field = findChild(palette.contentItem, "palette-field");
            field.text = "plugin sync";
            compare(palette.matches[0].id, "plugin_sync");
            palette.runCurrent();
            verify(!palette.visible);
            tryVerify(() => backend.calls.some(c => c.command === "plugin_sync"));
            palette.open();
            tryVerify(() => palette.opened);
            findChild(palette.contentItem, "palette-field").text = "project settings";
            compare(palette.matches[0].id, "editor.projectSettings");
            palette.runCurrent();
            compare(shellPage.ran, ["editor.projectSettings"]);
        }

        Component { id: topBarComponent; Editor.TopBar { app: backend; width: 1200 } }
        function test_theTopBarIsIdentityMenusAndPlay() {
            const actions = makeActions();
            const bar = createTemporaryObject(topBarComponent, scene, { actions: actions });
            verify(bar);
            for (const title of ["File", "Edit", "View", "Run", "Plugins", "Help"])
                verify(findChild(bar, "menu-" + title) !== null, title);
            verify(findChild(bar, "play-button").visible);
            const item = (title, id) => {
                const menu = bar.menus[title];
                for (let i = 0; i < menu.count; i++) if (menu.itemAt(i).objectName === "menu-item-" + id) return menu.itemAt(i);
                return null;
            };
            verify(item("Plugins", "editor.plugins").enabled);
            verify(!item("Plugins", "editor.pluginPanels").visible, "no panels, no item");
            const plugins = bar.menus["Plugins"];
            compare(plugins.itemAt(plugins.count - 1).text, "Voxel: Regenerate", "a plugin's own menu item joins the Plugins menu");
            compare(item("File", "save_project").shortcutText, "Ctrl+S");
            verify(item("View", "editor.split").iconName === "check", "a checked action shows a tick");
        }

        function test_problemsAreListed() {
            backend.problems = [
                { source: "plugins", severity: "error", message: "actor Lamp: the block voxel/set isn't provided", path: null, line: null },
                { source: "scripts", severity: "error", message: "assets/scripts/ball.rs didn't build", path: "assets/scripts/ball.rs", line: 12 }
            ];
            const panel = createTemporaryObject(problemsComponent, scene);
            tryCompare(panel, "errors", 2);
            const list = findChild(panel, "problems-list");
            compare(list.count, 2);
        }
    }
}
