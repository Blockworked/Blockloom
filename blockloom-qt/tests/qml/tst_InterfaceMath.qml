import QtQuick
import QtTest
import "../../qml" as Editor

// The arithmetic behind move, resize, snap, align and reparent, with no view.
TestCase {
    name: "InterfaceMath"
    Editor.UiEditMath { id: m }
    function bound(x, y, w, h) { return {id: "b", size: [w, h], transform: [1, 0, 0, 1, x + w / 2, y + h / 2], visible: true, paint_order: 1, clips: []}; }
    function gesture(extra) {
        return Object.assign({offset: [100, 100], grid: false, align: false, step: 8, box: [100, 100, 200, 160],
            targets: [[0, 480, 960], [0, 360, 720]], tolerance: [6, 6], direction: {x: 1, y: 1}, size: [100, 60],
            minimum: [1, 1], maximum: [Infinity, Infinity], bound: bound(100, 100, 100, 60), frameInverse: [1, 0, 0, 1, 0, 0]}, extra || {});
    }
    function test_move_snaps_to_the_grid_and_to_targets_unless_shift() {
        let r = m.moveResult(gesture({grid: true}), 13, 5, true);
        compare(r.offset, [112, 104]);
        r = m.moveResult(gesture({grid: true}), 13, 5, false);
        compare(r.offset, [113, 105]);
        // The widget's left edge lands within tolerance of the frame's left edge.
        r = m.moveResult(gesture({align: true}), -97, 0, true);
        compare(r.offset[0], 0);
        compare(r.hits[0], 0);
        compare(r.hits[1], null);
    }
    function test_resize_from_each_corner_keeps_the_opposite_edge() {
        const g = gesture();
        const br = m.resizeResult(g, {x: 50, y: 30}, {x: 60, y: 50}, true, [0, 0]);
        compare(br.size, [110, 80]);
        compare(br.offset, [100, 100]);
        const tl = m.resizeResult(gesture({direction: {x: -1, y: -1}}), {x: -50, y: -30}, {x: -40, y: -10}, true, [0, 0]);
        compare(tl.size, [90, 40]);
        compare(tl.offset, [110, 120]);
    }
    function test_resize_respects_limits_and_grid() {
        const g = gesture({minimum: [20, 20], maximum: [120, 120], grid: true});
        compare(m.resizeResult(g, {x: 0, y: 0}, {x: -500, y: -500}, true, [0, 0]).size, [20, 20]);
        compare(m.resizeResult(g, {x: 0, y: 0}, {x: 500, y: 500}, true, [0, 0]).size, [120, 120]);
        // The grid snaps every axis the handle moves, so 60 rounds to 64.
        compare(m.resizeResult(g, {x: 0, y: 0}, {x: 3, y: 0}, true, [0, 0]).size, [104, 64]);
    }
    function test_arrange_aligns_and_spaces_evenly() {
        const boxes = {a: [0, 0, 10, 10], b: [30, 5, 50, 15], c: [100, 0, 110, 10]};
        let r = m.arrangeDeltas("left", ["a", "b", "c"], boxes);
        compare(r.axis, 0);
        compare(r.delta.a, 0); compare(r.delta.b, -30); compare(r.delta.c, -100);
        r = m.arrangeDeltas("hdist", ["a", "b", "c"], boxes);
        // 110 total span, 40 of widgets, so two gaps of 35.
        compare(r.delta.a, 0); compare(r.delta.b, 15); compare(r.delta.c, 0);
        r = m.arrangeDeltas("vcenter", ["a", "b"], boxes);
        compare(r.axis, 1);
    }
    function test_rotated_widgets_have_no_aligned_box() {
        const rotated = {size: [10, 10], transform: [0, 1, -1, 0, 5, 5], visible: true, clips: []};
        verify(m.alignedBox(rotated, [1, 0, 0, 1, 0, 0]) === null);
        compare(m.alignedBox(bound(10, 20, 30, 40), [1, 0, 0, 1, 0, 0]), [10, 20, 40, 60]);
    }
    function test_reparent_placement_inside_a_parent_and_the_screen() {
        const parent = {size: [200, 200], transform: [1, 0, 0, 1, 300, 300], visible: true, clips: []};
        const child = {size: [100, 60], transform: [1, 0, 0, 1, 350, 330], visible: true, clips: []};
        let r = m.reparentPlacement(child, parent, 1, [0, 0, 0, 0]);
        compare(r.placement.offset, [100, 100]);
        compare(r.placement.size, [100, 60]);
        r = m.reparentPlacement(child, null, 2, [10, 20, 0, 0]);
        fuzzyCompare(r.placement.offset[0], (300 - 10) / 2, 0.001);
        fuzzyCompare(r.placement.offset[1], (300 - 20) / 2, 0.001);
        const turned = {size: [100, 60], transform: [0, 1, -1, 0, 350, 330], visible: true, clips: []};
        verify(m.reparentPlacement(turned, parent, 1, [0, 0, 0, 0]).error !== undefined);
    }
    function test_snap_targets_collect_frame_siblings_and_guides() {
        const t = m.snapTargets([0, 0, 100, 50], [{box: [10, 10, 30, 20]}], [[77], []]);
        compare(t[0], [0, 50, 100, 10, 20, 30, 77]);
        compare(t[1], [0, 25, 50, 10, 15, 20]);
    }
}
