import QtQml

// The arithmetic behind move, resize, snap, align and reparent, with no view
// state: every function takes the numbers it needs and returns a result.
// Frames are the 2x3 matrices the runtime reports.
QtObject {
    id: root
    readonly property var geometry: InterfaceGeometry {}

    function boxInFrame(bound, inverse) {
        const points = geometry.rectangle(bound.transform, [-bound.size[0]/2, -bound.size[1]/2, bound.size[0]/2, bound.size[1]/2])
            .map(p => geometry.point(inverse, p.x, p.y));
        return [Math.min(...points.map(p => p.x)), Math.min(...points.map(p => p.y)),
                Math.max(...points.map(p => p.x)), Math.max(...points.map(p => p.y))];
    }

    // The target nearest to any of `values` within `tolerance`: {delta, target|null}.
    function alignAxis(values, targets, tolerance) {
        let result = {delta: 0, target: null}, distance = tolerance;
        targets.forEach(target => values.forEach(value => {
            const d = Math.abs(target - value);
            if (d < distance) { distance = d; result = {delta: target - value, target: target}; }
        }));
        return result;
    }

    // Snap lines in preview pixels from per-axis hits (a target or null each).
    function snapLines(frame, extent, hits) {
        return hits.map((target, axis) => target === null ? null : {
            a: geometry.point(frame, axis === 0 ? target : extent[0], axis === 0 ? extent[1] : target),
            b: geometry.point(frame, axis === 0 ? target : extent[2], axis === 0 ? extent[3] : target)
        }).filter(line => line !== null);
    }

    // Snap targets per axis in the frame's own units: the frame's edges and centre,
    // its visible free siblings' edges and centres, and any guides.
    //   siblings: [{box: [x0, y0, x1, y1]}] already in the frame.
    function snapTargets(extent, siblings, guides) {
        const targets = [[extent[0], (extent[0] + extent[2]) / 2, extent[2]], [extent[1], (extent[1] + extent[3]) / 2, extent[3]]];
        siblings.forEach(s => {
            for (let axis = 0; axis < 2; ++axis) targets[axis].push(s.box[axis], (s.box[axis] + s.box[axis + 2]) / 2, s.box[axis + 2]);
        });
        for (let axis = 0; axis < 2; ++axis) (guides[axis] || []).forEach(t => targets[axis].push(t));
        return targets;
    }

    // Where a Move lands. `g` carries offset, grid, align, step, box, targets,
    // tolerance, scale and parentInverse; (dx, dy) is the pointer travel already
    // in the parent's frame.
    function moveResult(g, dx, dy, snapping) {
        let offset = [g.offset[0] + dx, g.offset[1] + dy];
        let hits = null;
        if (g.grid && snapping) offset = offset.map(v => Math.round(v / g.step) * g.step);
        if (g.align && snapping) {
            const delta = [offset[0] - g.offset[0], offset[1] - g.offset[1]];
            hits = [0, 1].map(axis => alignAxis([g.box[axis] + delta[axis], (g.box[axis] + g.box[axis + 2]) / 2 + delta[axis], g.box[axis + 2] + delta[axis]], g.targets[axis], g.tolerance[axis]));
            offset = offset.map((v, axis) => v + hits[axis].delta);
            hits = hits.map(hit => hit.target);
        }
        return {offset: offset, hits: hits};
    }

    // Where a Resize lands. `a` and `b` are the press and current points in the
    // widget's own frame; `direction` is the handle (-1, 0 or 1 per axis);
    // `fraction` is the anchor fraction the offset is measured from.
    function resizeResult(g, a, b, snapping, fraction) {
        const direction = g.direction;
        const constrain = (value, axis) => Math.min(g.maximum[axis], Math.max(g.minimum[axis], value));
        let size = [direction.x ? g.size[0] + direction.x * (b.x - a.x) : g.size[0], direction.y ? g.size[1] + direction.y * (b.y - a.y) : g.size[1]];
        size = size.map((v, axis) => (axis === 0 ? direction.x : direction.y) ? constrain(g.grid && snapping ? Math.round(v / g.step) * g.step : v, axis) : v);
        // Edge alignment is representable only when widget axes match its parent.
        const origin = geometry.point(g.frameInverse, g.bound.transform[4], g.bound.transform[5]);
        const axisX = geometry.point(g.frameInverse, g.bound.transform[4] + g.bound.transform[0], g.bound.transform[5] + g.bound.transform[1]);
        const axisY = geometry.point(g.frameInverse, g.bound.transform[4] + g.bound.transform[2], g.bound.transform[5] + g.bound.transform[3]);
        const relative = [axisX.x - origin.x, axisX.y - origin.y, axisY.x - origin.x, axisY.y - origin.y];
        let hits = null;
        if (g.align && snapping && Math.abs(relative[1]) < 0.001 && Math.abs(relative[2]) < 0.001 && relative[0] > 0 && relative[3] > 0) {
            hits = [null, null];
            [direction.x, direction.y].forEach((d, axis) => {
                if (!d) return;
                const scale = relative[axis === 0 ? 0 : 3];
                const edge = g.box[axis + (d > 0 ? 2 : 0)] + d * (size[axis] - g.size[axis]) * scale;
                const hit = alignAxis([edge], g.targets[axis], g.tolerance[axis]);
                const snapped = size[axis] + d * hit.delta / scale;
                if (snapped >= g.minimum[axis] && snapped <= g.maximum[axis]) { size[axis] = snapped; hits[axis] = hit.target; }
            });
        }
        const delta = [size[0] - g.size[0], size[1] - g.size[1]];
        const center = [direction.x * delta[0] / 2, direction.y * delta[1] / 2];
        const offset = [g.offset[0] + relative[0] * center[0] + relative[2] * center[1] + (fraction[0] - 0.5) * delta[0],
                        g.offset[1] + relative[1] * center[0] + relative[3] * center[1] + (fraction[1] - 0.5) * delta[1]];
        return {size: size, offset: offset, hits: hits};
    }

    readonly property var anchorFractions: ({TopLeft: [0, 0], Top: [0.5, 0], TopRight: [1, 0], Left: [0, 0.5], Center: [0.5, 0.5],
        Right: [1, 0.5], BottomLeft: [0, 1], Bottom: [0.5, 1], BottomRight: [1, 1]})

    // Alignment and distribution. `boxes` maps id to [x0, y0, x1, y1] in the shared
    // parent's frame; returns {axis, delta: {id: amount}}.
    function arrangeDeltas(op, ids, boxes) {
        const axis = op === "left" || op === "hcenter" || op === "right" || op === "hdist" ? 0 : 1;
        const lo = id => boxes[id][axis], hi = id => boxes[id][axis + 2], mid = id => (lo(id) + hi(id)) / 2;
        const delta = Object.create(null);
        if (op === "hdist" || op === "vdist") {
            const sorted = ids.slice().sort((a, b) => mid(a) - mid(b));
            const span = hi(sorted[sorted.length - 1]) - lo(sorted[0]);
            const total = sorted.reduce((sum, id) => sum + hi(id) - lo(id), 0);
            const gap = (span - total) / (sorted.length - 1);
            let at = lo(sorted[0]);
            sorted.forEach(id => { delta[id] = at - lo(id); at += hi(id) - lo(id) + gap; });
        } else {
            const first = op === "left" || op === "top", last = op === "right" || op === "bottom";
            const target = first ? Math.min(...ids.map(lo)) : last ? Math.max(...ids.map(hi)) : (Math.min(...ids.map(lo)) + Math.max(...ids.map(hi))) / 2;
            ids.forEach(id => { delta[id] = target - (first ? lo(id) : last ? hi(id) : mid(id)); });
        }
        return {axis: axis, delta: delta};
    }

    // A widget's bound as an axis-aligned box in `inverse`'s frame, or null when it
    // is rotated or flipped relative to it.
    function alignedBox(bound, inverse) {
        const t = bound.transform;
        const origin = geometry.point(inverse, t[4], t[5]);
        const ax = geometry.point(inverse, t[4] + t[0], t[5] + t[1]);
        const ay = geometry.point(inverse, t[4] + t[2], t[5] + t[3]);
        if (Math.abs(ax.y - origin.y) > 0.001 || Math.abs(ay.x - origin.x) > 0.001 || ax.x <= origin.x || ay.y <= origin.y) return null;
        return boxInFrame(bound, inverse);
    }

    // The Free placement that keeps `bound` where it is when it moves under
    // `parent` (null for the screen root). Returns {placement} or {error}.
    function reparentPlacement(bound, parent, designScale, safe) {
        const transform = parent ? geometry.inverse(parent.transform)
            : [1 / designScale, 0, 0, 1 / designScale, -safe[0] / designScale, -safe[1] / designScale];
        if (!transform) return {error: "The parent transform cannot be inverted."};
        const corner = geometry.point(bound.transform, -bound.size[0] / 2, -bound.size[1] / 2);
        const at = geometry.point(transform, corner.x, corner.y);
        const x = geometry.point(bound.transform, bound.size[0] / 2, -bound.size[1] / 2);
        const y = geometry.point(bound.transform, -bound.size[0] / 2, bound.size[1] / 2);
        const right = geometry.point(transform, x.x, x.y), bottom = geometry.point(transform, y.x, y.y);
        if (Math.abs(right.y - at.y) > 0.01 || Math.abs(bottom.x - at.x) > 0.01 || right.x <= at.x || bottom.y <= at.y)
            return {error: "This transform cannot preserve placement in the new parent."};
        return {placement: {mode: "Free", offset: [at.x + (parent ? parent.size[0] / 2 : 0), at.y + (parent ? parent.size[1] / 2 : 0)],
                            size: [right.x - at.x, bottom.y - at.y]}};
    }

}
