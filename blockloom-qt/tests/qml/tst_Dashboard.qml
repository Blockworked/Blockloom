import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "Dashboard"
    visible: true
    when: windowShown
    width: 1000; height: 700
    property var calls: []
    property var failNext: null
    function library() {
        return [
            { path: "/games/Pond", name: "Pond", mode: "TwoD", opened_at: 0, available: true, error: null, installation: "release-0.2.0", installation_label: "Blockloom 0.2.0", wrong_editor: true, in_use: false },
            { path: "/games/Away", name: "Away", mode: "TwoD", opened_at: 0, available: false, error: "The folder is missing. Its drive may be disconnected.", installation: null, in_use: false },
            { path: "/games/Shared", name: "Shared", mode: "ThreeD", opened_at: 0, available: true, error: null, installation: null, in_use: true }
        ];
    }
    QtObject {
        id: backend
        property var appState: ({ library: test.library(), runtime_available: true, default_project_location: "/games", project: null })
        property string appVersion: "0.0.1"
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (test.failNext && failed) { const e = test.failNext; test.failNext = null; failed(e); }
            else if (done) done({});
        }
        function toFileUrl(path) { return "file://" + path; }
        function fromFileUrl(url) { return String(url); }
        function guardMessage(error) { return String(error).replace(/^(save-failed|wrong-editor):\s*/, ""); }
    }
    Component { id: factory; Editor.Dashboard { app: backend; width: 1000; height: 700 } }
    Component { id: guardFactory; Editor.NavigationGuard { app: backend } }
    function init() { calls = []; failNext = null; }

    function test_unavailableRowOffersRecoveryNotOpen() {
        const page = createTemporaryObject(factory, test);
        verify(page !== null);
        const list = findChild(page, "project-list");
        tryVerify(() => list.itemAtIndex(1) !== null);
        const away = list.itemAtIndex(1);
        verify(findChild(away, "locate-folder").visible);
        verify(findChild(away, "remove-from-list").visible);
        verify(!findChild(list.itemAtIndex(0), "remove-from-list").visible);
        findChild(away, "remove-from-list").clicked();
        compare(calls[0].command, "forget_project");
        compare(calls[0].args.path, "/games/Away");
    }
    function test_enterOpensTheFocusedRowOnly() {
        const page = createTemporaryObject(factory, test);
        const list = findChild(page, "project-list");
        verify(list !== null);
        tryVerify(() => list.itemAtIndex(1) !== null);
        const pond = list.itemAtIndex(0), away = list.itemAtIndex(1);
        pond.forceActiveFocus();
        keyClick(Qt.Key_Return);
        compare(calls.length, 1);
        compare(calls[0].command, "open_project");
        compare(calls[0].args.path, "/games/Pond");
        calls = [];
        away.forceActiveFocus();
        keyClick(Qt.Key_Return);
        compare(calls.length, 0);
    }
    function test_deleteNeedsTheTypedNameAndSaysTrash() {
        const page = createTemporaryObject(factory, test);
        page.showDelete(test.library()[0]);
        const confirm = findChild(page, "delete-confirm");
        const remove = findChild(page, "delete-project");
        tryVerify(() => remove.visible);
        compare(remove.enabled, false);
        compare(remove.text, "Move to trash");
        confirm.text = "Pond";
        compare(remove.enabled, true);
        findChild(page, "delete-permanently").checked = true;
        compare(remove.text, "Delete permanently");
        remove.clicked();
        compare(calls[0].command, "delete_project");
        compare(calls[0].args.permanent, true);
    }
    function test_deleteIsBlockedWhileAnotherEditorOwnsTheFolder() {
        const page = createTemporaryObject(factory, test);
        page.showDelete(test.library()[2]);
        findChild(page, "delete-confirm").text = "Shared";
        compare(findChild(page, "delete-project").enabled, false);
    }
    function test_searchShowsAnExplicitEmptyMessage() {
        const lib = [];
        for (let i = 0; i < 6; ++i) lib.push({ path: "/g/P" + i, name: "P" + i, mode: "TwoD", opened_at: 0, available: true, error: null, installation: null, in_use: false });
        backend.appState = { library: lib, runtime_available: true, default_project_location: "/games", project: null };
        const page = createTemporaryObject(factory, test);
        findChild(page, "project-search").text = "nothing-like-this";
        tryVerify(() => findChild(page, "no-search-match").visible);
        backend.appState = { library: test.library(), runtime_available: true, default_project_location: "/games", project: null };
    }
    function test_guardOffersRetryAndDiscardForAFailedSave() {
        const guard = createTemporaryObject(guardFactory, test);
        guard.ask("close_project", {}, "save-failed: Couldn't save \"Pond\" to /games/Pond: disk full");
        verify(guard.visible || guard.opened);
        compare(guard.kind, "save-failed");
        findChild(guard, "discard-changes").clicked();
        compare(calls[0].command, "close_project");
        compare(calls[0].args.discard, true);
    }
    function test_guardOpenAnywayRepeatsWithTheDecision() {
        const guard = createTemporaryObject(guardFactory, test);
        guard.ask("open_project", { path: "/games/Pond" }, "wrong-editor: This project is set to Blockloom 0.2.0, but this is Blockloom 0.1.0.");
        compare(guard.kind, "wrong-editor");
        findChild(guard, "open-anyway").clicked();
        compare(calls[0].args.other_editor, true);
        compare(calls[0].args.rebind, undefined);
        compare(calls[0].args.path, "/games/Pond");
    }
    function test_guardCanSwitchTheProjectToThisEditor() {
        const guard = createTemporaryObject(guardFactory, test);
        guard.ask("open_project", { path: "/games/Pond" }, "wrong-editor: This project is set to Blockloom 0.2.0");
        findChild(guard, "open-and-rebind").clicked();
        compare(calls[0].args.other_editor, true);
        compare(calls[0].args.rebind, true);
    }
    function test_aRowSaysWhenItsProjectIsSetToAnotherEditor() {
        const page = createTemporaryObject(factory, test);
        const list = findChild(page, "project-list");
        tryVerify(() => list.itemAtIndex(0) !== null);
        verify(page.editorLabel(test.library()[0]) === "Blockloom 0.2.0");
        verify(test.library()[0].wrong_editor);
    }
}
