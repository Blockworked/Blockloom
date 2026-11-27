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
    function says(from, to) {
        const out = [];
        for (let id = from; id <= to; id++) out.push({ id: id, count: 1, time: id / 10, kind: "say", level: "info", actor: "P", text: "line " + id });
        return out;
    }
    function test_repeatsTickInPlace() {
        const log = createTemporaryObject(logFactory, test);
        const view = findChild(log, "runlog-lines");
        backend.log = { lines: says(1, 3), total: 3 };
        const first = view.itemAtIndex(0);
        const folded = says(1, 3);
        folded[2].count = 40;
        backend.log = { lines: folded, total: 42 };
        compare(log.shownCount(), 3, "a repeat adds no row");
        compare(view.itemAtIndex(2).times, 40);
        verify(view.itemAtIndex(0) === first, "rows are kept, not rebuilt");
    }
    function test_scrolledUpStaysPut() {
        const log = createTemporaryObject(logFactory, test);
        const view = findChild(log, "runlog-lines");
        backend.log = { lines: says(1, 60), total: 60 };
        verify(view.atYEnd, "follows new lines from the start");
        view.contentY = view.originY + 40;
        verify(!log.following);
        const at = view.contentY;
        // The backend drops the oldest lines once full: rows leave the top.
        backend.log = { lines: says(11, 70), total: 70 };
        compare(log.shownCount(), 60);
        compare(log.unseen, 10);
        verify(!view.atYEnd, "doesn't jump to the bottom");
        verify(findChild(log, "runlog-jump").visible);
        findChild(log, "runlog-jump").clicked();
        verify(log.following);
        compare(log.unseen, 0);
        verify(view.atYEnd);
    }
    function test_pauseHoldsTheView() {
        const log = createTemporaryObject(logFactory, test);
        backend.log = { lines: says(1, 3), total: 3 };
        log.paused = true;
        backend.log = { lines: says(1, 8), total: 8 };
        compare(log.shownCount(), 3);
        compare(log.unseen, 5);
        log.paused = false;
        compare(log.shownCount(), 8);
        compare(log.unseen, 0);
        backend.log = { lines: [], total: 8 };
        compare(log.shownCount(), 0, "clearing empties it");
    }
    function test_clockStaysNarrow() {
        const log = createTemporaryObject(logFactory, test);
        compare(log.clock(-1), "");
        compare(log.clock(4.26), "4.3");
        compare(log.clock(62.04), "1:02.0");
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
