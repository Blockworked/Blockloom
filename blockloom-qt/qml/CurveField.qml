import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A value over 0-1 (a particle's life, or along a ribbon) as draggable keys.
// Press empty space to add a key, drag one to move it, double-click one to
// remove it. `edited` hands back the whole key list, sorted, on release.
Rectangle {
    id: root
    property var keys: []
    property real low: 0
    property real high: 1
    signal edited(var keys)

    // The keys while a drag is under way, so the document isn't written per move.
    property var working: null
    readonly property var shown: sorted(working || keys || [])
    readonly property real pad: 6

    Layout.fillWidth: true
    implicitHeight: 72
    radius: 5; color: Theme.field; border.color: Theme.border

    function clamp(v, a, b) { return Math.min(b, Math.max(a, v)); }
    function sorted(list) { return list.map(k => ({ t: k.t, v: k.v })).sort((a, b) => a.t - b.t); }
    function span() { return Math.max(1e-6, high - low); }
    function xOf(t) { return pad + t * (width - 2 * pad); }
    function yOf(v) { return pad + (1 - clamp((v - low) / span(), 0, 1)) * (height - 2 * pad); }
    function keyAt(x, y) {
        const t = clamp((x - pad) / Math.max(1, width - 2 * pad), 0, 1);
        const v = low + (1 - clamp((y - pad) / Math.max(1, height - 2 * pad), 0, 1)) * span();
        return { t: Math.round(t * 1000) / 1000, v: Math.round(v * 1000) / 1000 };
    }
    function nearest(list, x, y) {
        let best = -1, bestDistance = 9;
        for (let i = 0; i < list.length; ++i) {
            const d = Math.hypot(xOf(list[i].t) - x, yOf(list[i].v) - y);
            if (d < bestDistance) { best = i; bestDistance = d; }
        }
        return best;
    }

    onShownChanged: canvas.requestPaint()
    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()
    onLowChanged: canvas.requestPaint()
    onHighChanged: canvas.requestPaint()

    Canvas {
        id: canvas
        anchors.fill: parent
        onPaint: {
            const ctx = getContext("2d");
            ctx.reset();
            ctx.strokeStyle = Theme.borderSoft;
            ctx.lineWidth = 1;
            ctx.beginPath();
            ctx.moveTo(root.pad, height / 2); ctx.lineTo(width - root.pad, height / 2);
            ctx.stroke();
            // No keys reads as 1, as the engine does.
            const list = root.shown.length ? root.shown : [{ t: 0, v: 1 }];
            ctx.strokeStyle = Theme.accent;
            ctx.lineWidth = 2;
            ctx.beginPath();
            ctx.moveTo(root.xOf(0), root.yOf(list[0].v));
            for (const k of list) ctx.lineTo(root.xOf(k.t), root.yOf(k.v));
            ctx.lineTo(root.xOf(1), root.yOf(list[list.length - 1].v));
            ctx.stroke();
            ctx.fillStyle = Theme.accent;
            for (const k of root.shown) {
                ctx.beginPath();
                ctx.arc(root.xOf(k.t), root.yOf(k.v), 4, 0, 2 * Math.PI);
                ctx.fill();
            }
        }
    }
    Text { x: 4; y: 2; text: Number(root.high.toPrecision(4)); color: Theme.textDim; font.pixelSize: 9 }
    Text { x: 4; anchors.bottom: parent.bottom; anchors.bottomMargin: 2; text: Number(root.low.toPrecision(4)); color: Theme.textDim; font.pixelSize: 9 }

    MouseArea {
        anchors.fill: parent
        property int dragging: -1
        onPressed: mouse => {
            const list = root.sorted(root.keys || []);
            let i = root.nearest(list, mouse.x, mouse.y);
            if (i < 0) { list.push(root.keyAt(mouse.x, mouse.y)); i = list.length - 1; }
            root.working = list;
            dragging = i;
        }
        onPositionChanged: mouse => {
            if (dragging < 0 || !root.working) return;
            const list = root.working.slice();
            list[dragging] = root.keyAt(mouse.x, mouse.y);
            root.working = list;
        }
        onReleased: {
            if (dragging < 0 || !root.working) return;
            const list = root.working.slice();
            dragging = -1;
            root.working = null;
            root.edited(root.sorted(list));
        }
        onDoubleClicked: mouse => {
            const list = root.sorted(root.keys || []);
            const i = root.nearest(list, mouse.x, mouse.y);
            if (i < 0 || list.length < 2) return;
            list.splice(i, 1);
            root.edited(list);
        }
    }
}
