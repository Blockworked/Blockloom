import QtQuick
import QtTest
import QtQuick.Controls

// Drives the real editor from inside its own process: Qt's own test events
// (never desktop input) against the real backend and the embedded runtime.
// Needs a build with `--features qml-preview` and a project whose interface
// has widgets "hp_label", "hp_bar" and "score" and an animation "slide" with
// a keyed OffsetX on "score" (see README.md next to this file).
Item {
    id: root
    property var main: null
    property var designer: null
    property var log: []
    function say(msg) { console.log("HARNESS " + msg); }
    function walk(item, visit) {
        if (!item) return false;
        if (visit(item)) return true;
        const kids = item.children || [];
        for (let i = 0; i < kids.length; ++i) if (walk(kids[i], visit)) return true;
        return false;
    }
    function findByName(name) {
        let found = null;
        walk(main.contentItem, it => { if (it.objectName === name) { found = it; return true; } return false; });
        return found;
    }
    function findByText(text) {
        let found = null;
        walk(main.contentItem, it => { if (it.text === text && it.visible && it.width > 0) { found = it; return true; } return false; });
        return found;
    }
    TestCase {
        id: t
        name: "RealEditor"
        when: root.main !== null
        function bounds(id) { return root.designer.boundsById[id]; }
        function center(id) {
            const b = bounds(id), m = b.transform;
            return {x: m[4], y: m[5]};
        }
        function until(fn, ms, what) {
            const end = Date.now() + ms;
            while (Date.now() < end) {
                let ok = false;
                try { ok = !!fn(); } catch (e) { ok = false; }
                if (ok) return;
                wait(100);
            }
            let state = "";
            try { const d = root.designer; state = " designing=" + d.designing + " embedded=" + d.embedded + " error=" + d.error + " preview_enabled=" + root.main.appState.preview_enabled + " running=" + root.main.appState.running + " layout=" + (d.frameLayout ? "rev " + d.frameLayout.revision + "/" + d.revision + " gen " + d.frameLayout.generation + "/" + d.generation : "null"); } catch (e) { state = " (state unavailable)"; }
            throw new Error("timed out: " + what + state);
        }
        function waitLayout() { until(() => root.designer.layoutReady, 40000, "layout ready"); }
        function must(cond, what) { if (!cond) throw new Error("check failed: " + what); }
        function same(a, b, what) { if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error("mismatch " + what + ": " + JSON.stringify(a) + " vs " + JSON.stringify(b)); }
        function test_gestures_through_the_real_runtime() {
            try { body(); } catch (e) { console.log("HARNESS ERROR " + e); }
        }
        function body() {
            until(() => root.main.appState && root.main.appState.project, 60000, "project open");
            const tab = root.findByText("Interface");
            must(tab !== null, "Interface tab");
            wait(1500);
            mouseClick(tab);
            until(() => root.findByName("interfaceViewport") !== null, 20000, "viewport");
            root.designer = root.findByName("interfaceViewport").designer;
            waitLayout();
            const canvas = root.findByName("interfaceCanvas");
            const picking = root.findByName("interfacePicking");
            root.say("layout widgets " + root.designer.bounds.length + " viewport " + root.designer.frameLayout.viewport);

            // Click selects what the runtime drew under the pointer.
            const c = center("hp_label");
            mouseClick(picking, c.x, c.y);
            same(root.designer.selectedId, "hp_label", "selected");
            root.say("click selected " + root.designer.selectedId);

            // Drag moves it: one preview per move, one commit on release.
            const before = root.designer.widgetsById["hp_label"].element.offset.slice();
            const startRev = root.designer.savedRevision;
            mousePress(picking, c.x, c.y);
            for (let i = 1; i <= 8; ++i) { mouseMove(picking, c.x + 5 * i, c.y + 3 * i); wait(30); }
            must(root.designer.gesture !== null, "gesture started");
            mouseRelease(picking, c.x + 40, c.y + 24);
            until(() => root.designer.gesture === null, 20000, "gesture committed");
            const after = root.designer.widgetsById["hp_label"].element.offset;
            root.say("drag offset " + JSON.stringify(before) + " -> " + JSON.stringify(after));
            must(after[0] > before[0] + 30 && after[1] > before[1] + 15, "moved");
            until(() => root.designer.savedRevision > startRev, 20000, "saved");
            waitLayout();
            const moved = center("hp_label");
            verify(Math.abs(moved.x - (c.x + 40)) < 4, "runtime geometry follows the move " + moved.x + " vs " + (c.x + 40));
            root.say("geometry after move " + moved.x + "," + moved.y);

            // A resize handle: grow from the bottom right.
            root.designer.selectedId = "hp_label";
            wait(100);
            const handle = root.findByName("interfaceResizeHandle");
            must(handle !== null && handle.visible, "handle visible");
            const sizeBefore = root.designer.widgetsById["hp_label"].element.size.slice();
            mousePress(handle, handle.width / 2, handle.height / 2);
            for (let i = 1; i <= 6; ++i) { mouseMove(handle, handle.width / 2 + 5 * i, handle.height / 2 + 2 * i); wait(30); }
            mouseRelease(handle, handle.width / 2 + 30, handle.height / 2 + 12);
            until(() => root.designer.gesture === null, 20000, "resize committed");
            const sizeAfter = root.designer.widgetsById["hp_label"].element.size;
            root.say("resize " + JSON.stringify(sizeBefore) + " -> " + JSON.stringify(sizeAfter));
            must(sizeAfter[0] > sizeBefore[0] + 20, "wider");

            // Escape cancels a drag and leaves the document alone.
            waitLayout();
            const c2 = center("hp_label");
            const keep = root.designer.widgetsById["hp_label"].element.offset.slice();
            mousePress(picking, c2.x, c2.y);
            for (let i = 1; i <= 5; ++i) { mouseMove(picking, c2.x + 6 * i, c2.y); wait(30); }
            must(root.designer.gesture !== null, "gesture active");
            keyClick(Qt.Key_Escape);
            until(() => root.designer.gesture === null, 10000, "cancelled");
            mouseRelease(picking, c2.x + 30, c2.y);
            same(root.designer.widgetsById["hp_label"].element.offset, keep, "escape kept");
            root.say("escape kept " + JSON.stringify(keep));

            // Arrow keys nudge by one.
            waitLayout();
            root.designer.selectedId = "hp_label";
            root.designer.forceActiveFocus();
            keyPress(Qt.Key_Right); wait(50); keyRelease(Qt.Key_Right);
            until(() => root.designer.gesture === null && root.designer.widgetsById["hp_label"].element.offset[0] === keep[0] + 1, 20000, "nudged");
            root.say("nudged to " + root.designer.widgetsById["hp_label"].element.offset);

            // Scrub the keyed animation through the runtime.
            waitLayout();
            const x0 = center("score").x;
            root.designer.previewAnimation("slide", 1.0);
            until(() => root.designer.layoutReady && Math.abs(center("score").x - (x0 - 120)) < 2, 30000, "scrubbed");
            root.say("scrub score x " + x0 + " -> " + center("score").x);
            root.designer.stopAnimationPreview();

            // Hiding the world is a preview-only request the runtime accepts.
            root.designer.showWorld = false;
            until(() => root.designer.layoutReady && root.designer.error === "", 30000, "layout without the world");
            root.say("world hidden, layout ready");
            root.designer.showWorld = true;
            until(() => root.designer.layoutReady && root.designer.error === "", 30000, "layout with the world");

            // Rename, then undo restores everything.
            root.designer.renameWidget("hp_bar", "health_bar");
            until(() => root.designer.widgetsById["health_bar"] !== undefined, 20000, "renamed");
            root.say("renamed hp_bar -> health_bar");
            root.say("DONE");
        }
    }
    Component.onCompleted: {
        const c = Qt.createComponent("qrc:/qt/qml/com/blockworked/Blockloom/qml/Main.qml");
        if (c.status !== Component.Ready) { console.log("HARNESS load failed " + c.errorString()); return; }
        root.main = c.createObject(null);
    }
}
