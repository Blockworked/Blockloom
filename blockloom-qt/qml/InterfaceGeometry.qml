import QtQml

QtObject {

function point(matrix, x, y) {
    return {x: matrix[0]*x + matrix[2]*y + matrix[4], y: matrix[1]*x + matrix[3]*y + matrix[5]};
}

function inverse(matrix) {
    const d = matrix[0]*matrix[3] - matrix[1]*matrix[2];
    if (Math.abs(d) < 1e-10) return null;
    return [matrix[3]/d, -matrix[1]/d, -matrix[2]/d, matrix[0]/d,
            (matrix[2]*matrix[5]-matrix[3]*matrix[4])/d, (matrix[1]*matrix[4]-matrix[0]*matrix[5])/d];
}

function contains(widget, x, y) {
    if (!widget.visible) return false;
    const matrix = inverse(widget.transform);
    if (!matrix) return false;
    const p = point(matrix, x, y);
    if (Math.abs(p.x) > widget.size[0]/2 || Math.abs(p.y) > widget.size[1]/2) return false;
    return widget.clips.every(c => {
        const q = point(c.viewport_to_local, x, y), r = c.rect;
        return q.x >= r[0] && q.y >= r[1] && q.x <= r[2] && q.y <= r[3];
    });
}

function pick(widgets, x, y) {
    let best = null;
    widgets.forEach(w => { if (contains(w, x, y) && (!best || w.paint_order > best.paint_order)) best = w; });
    return best ? best.id : "";
}

function rectangle(matrix, rect) {
    return [point(matrix, rect[0], rect[1]), point(matrix, rect[2], rect[1]),
            point(matrix, rect[2], rect[3]), point(matrix, rect[0], rect[3])];
}

function polygon(widget) {
    if (!widget || !widget.visible) return [];
    const w = widget.size[0]/2, h = widget.size[1]/2;
    let result = rectangle(widget.transform, [-w, -h, w, h]);
    widget.clips.forEach(c => {
        const matrix = inverse(c.viewport_to_local);
        if (!matrix) { result = []; return; }
        const clip = rectangle(matrix, c.rect);
        const sign = matrix[0]*matrix[3]-matrix[1]*matrix[2] >= 0 ? 1 : -1;
        for (let i = 0; i < 4 && result.length; ++i) {
            const a = clip[i], b = clip[(i+1)%4];
            const side = p => sign*((b.x-a.x)*(p.y-a.y)-(b.y-a.y)*(p.x-a.x));
            const input = result; result = [];
            let prev = input[input.length-1], ps = side(prev);
            input.forEach(p => {
                const s = side(p);
                if ((s >= 0) !== (ps >= 0)) {
                    const t = ps/(ps-s);
                    result.push({x: prev.x+t*(p.x-prev.x), y: prev.y+t*(p.y-prev.y)});
                }
                if (s >= 0) result.push(p);
                prev = p; ps = s;
            });
        }
    });
    return result;
}

}
