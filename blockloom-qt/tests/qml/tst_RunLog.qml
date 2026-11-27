import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml" as Editor

TestCase {
    id: test
    name: "RunLog"
    visible: true
    width: 900; height: 500
    when: windowShown
    QtObject {
        id: backend
        property var appState: ({ running: true, paused: false })
        property var status: ({ time: 1, globals: [], render_metrics: [] })
        property var log: ({ lines: [], total: 0 })
        function invoke(command, args, done, failed) { if (done) done({}); }
    }
    Component { id: logFactory; Editor.RunLog { app: backend; docked: true; width: 900; height: 300 } }
    Component { id: profilerFactory; Editor.ProfilerPanel { app: backend; width: 600; height: 300 } }
    function lines() {
        return [
            { kind: "say", level: "info", actor: "Player", text: "hello" },
            { kind: "error", level: "error", actor: "Coin", text: "there's no actor named \"Nobody\"" },
            { kind: "info", level: "info", actor: "Tests", text: "Passed \"falls\"" },
            { kind: "say", level: "info", actor: "Guard", text: "halt" }
        ];
    }
    function init() { backend.log = { lines: [], total: 0 }; backend.status = { time: 1, globals: [], render_metrics: [] }; }
    function test_filtersByKindAndWords() {
        const log = createTemporaryObject(logFactory, test);
        backend.log = { lines: lines(), total: 4 };
        compare(log.all.length, 4);
        compare(log.errorCount, 1);
        log.show = "errors";
        compare(log.shownCount(), 1);
        log.show = "says";
        compare(log.shownCount(), 2);
        log.search = "guard";
        compare(log.shownCount(), 1, "an actor's name matches too");
        log.show = "all"; log.search = "";
        compare(log.shownCount(), 4);
        // New lines join only when they pass.
        log.show = "errors";
        backend.log = { lines: lines().concat([{ kind: "say", level: "info", actor: "P", text: "more" }]), total: 5 };
        compare(log.shownCount(), 1);
        compare(log.all.length, 5);
    }
    function test_profilerGroupsAndKeepsHistory() {
        const panel = createTemporaryObject(profilerFactory, test);
        backend.status = { time: 1, globals: [], render_metrics: [{ name: "loop/update", value: 4, unit: "ms" }, { name: "gpu/frame", value: 2, unit: "ms" }] };
        backend.status = { time: 2, globals: [], render_metrics: [{ name: "loop/update", value: 8, unit: "ms" }, { name: "gpu/frame", value: 2, unit: "ms" }] };
        compare(panel.rows.map(r => r.name).join(","), "gpu,gpu/frame,loop,loop/update");
        compare(panel.stats("loop/update").avg, 6);
        compare(panel.stats("loop/update").peak, 8);
        panel.filter = "loop";
        compare(panel.rows.length, 2);
        panel.paused = true;
        backend.status = { time: 3, globals: [], render_metrics: [{ name: "loop/update", value: 100, unit: "ms" }] };
        compare(panel.stats("loop/update").peak, 8, "held");
        compare(panel.format(1048576 * 3, "bytes"), "3.0 MiB");
    }
}
