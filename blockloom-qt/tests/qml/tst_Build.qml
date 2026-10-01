import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "Build"
    visible: true
    width: 1000; height: 900
    when: windowShown
    property var dialog: null
    property bool deferTargets: false
    property var targetsDone: null
    property var jobStatus: ({ id: 1, state: "running", step: "Creating shareable archive", detail: "" })
    property var calls: []
    property bool deferStart: false
    property var startDone: null
    property bool deferCancel: false
    property var rows: [
        { triple: "x86_64-unknown-linux-gnu", label: "Linux x64", host: true, ready: true, fast_ready: true, hdr: true, note: "This machine.", fast_note: "Native blocks available.", hdr_note: "HDR available." },
        { triple: "wasm32-unknown-unknown", label: "Web", host: false, ready: false, fast_ready: false, hdr: false, note: "No web player.", fast_note: "VM only.", hdr_note: "SDR only." }
    ]
    QtObject {
        id: backend
        property var appState: ({ project: { name: "Demo" }, default_build_location: "/tmp/build" })
        function toFileUrl(path) { return "file://" + path; }
        function fromFileUrl(path) { return String(path).replace("file://", ""); }
        function invoke(command, args, done, failed) {
            test.calls = test.calls.concat([{ command: command, args: args }]);
            if (command === "list_build_targets" && test.deferTargets) test.targetsDone = done;
            else if (command === "list_build_targets") done(test.rows);
            else if (command === "start_build_game" && test.deferStart) test.startDone = done;
            else if (command === "start_build_game") done(test.jobStatus);
            else if (command === "build_job_status") done(test.jobStatus);
            else if (command === "cancel_build_job") { if (!test.deferCancel) test.jobStatus = { id: args.id, state: "cancelled", step: "Cancelled" }; done({}); }
            else if (done) done({});
        }
    }
    Component { id: factory; Editor.BuildDialog { app: backend } }
    function init() {
        test.Window.window.width = 1000; test.Window.window.height = 900;
        calls = []; deferTargets = false; deferStart = false; deferCancel = false; targetsDone = null; startDone = null;
        jobStatus = { id: 1, state: "running", step: "Creating shareable archive", detail: "" };
        dialog = createTemporaryObject(factory, test);
        verify(dialog !== null);
        dialog.open();
        tryCompare(dialog, "opened", true);
    }
    function cleanup() { dialog.destroy(); dialog = null; }
    function test_platformList() {
        compare(dialog.targets.length, 2);
        compare(dialog.triple, rows[0].triple);
        verify(dialog.chosen !== null);
        const combo = findChild(dialog, "buildPlatform");
        compare(combo.count, 2);
        compare(combo.displayText, "Linux x64 (this machine)");
        combo.popup.open();
        tryCompare(combo.popup, "visible", true);
        verify(combo.popup.height > 32);
        combo.activated(1);
        compare(dialog.triple, rows[1].triple);
        verify(!findChild(dialog, "buildGameButton").enabled);
        combo.popup.close();
    }
    function test_delayedPlatforms() {
        dialog.close(); tryCompare(dialog, "opened", false);
        deferTargets = true;
        dialog.targets = []; dialog.triple = "";
        dialog.open(); tryCompare(dialog, "opened", true);
        const combo = findChild(dialog, "buildPlatform");
        compare(combo.displayText, "Loading platforms...");
        verify(!combo.enabled);
        targetsDone(rows);
        compare(combo.count, 2);
        compare(combo.displayText, "Linux x64 (this machine)");
        verify(combo.enabled);
    }
    function test_buildProgressCancelAndCompletion() {
        dialog.submit();
        const progress = findChild(dialog, "gameBuildProgress");
        verify(dialog.busy);
        verify(progress.visible);
        compare(progress.step, "Creating shareable archive");
        verify(!findChild(dialog, "buildPlatform").enabled);
        grabImage(test.Window.window.contentItem).save("/tmp/blockloom-build-progress.png");
        verify(findChild(progress, "buildProgressBar").indeterminate);
        mouseClick(findChild(progress, "cancelBuildButton"));
        verify(!dialog.busy);
        compare(dialog.error, "Build cancelled.");
        jobStatus = { id: 2, state: "running", step: "Copying game assets", detail: "" };
        dialog.submit();
        jobStatus = { id: 2, state: "complete", step: "Build complete", built: { binary: "/tmp/Demo", dir: "/tmp/Demo", archive: "/tmp/Demo.zip" } };
        progress.poll();
        verify(!dialog.busy);
        compare(dialog.built.archive, "/tmp/Demo.zip");
    }
    function test_cancelBeforeStartReply() {
        deferStart = true; deferCancel = true;
        dialog.submit();
        const progress = findChild(dialog, "gameBuildProgress");
        progress.cancel();
        verify(progress.cancelling);
        compare(calls.filter(c => c.command === "cancel_build_job").length, 0);
        startDone(jobStatus);
        compare(calls.filter(c => c.command === "cancel_build_job").length, 1);
        verify(dialog.busy);
        progress.cancel();
        compare(calls.filter(c => c.command === "cancel_build_job").length, 1);
        jobStatus = { id: 1, state: "cancelled", step: "Cancelled" };
        progress.poll();
        verify(!dialog.busy);
    }

}
