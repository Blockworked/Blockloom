import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// One 24h director track as draggable Bezier keys: the curve editor the
// TimeTrack model samples. Click empty space to add a key, drag one to move
// it, double-click one to remove it (empty means the dial stays untouched).
// The curve previews the Hermite sampling the engine does, tangents
// included; `edited` hands back the whole track on release or commit.
ColumnLayout {
    id: root
    property var track: ({ keys: [], loop_enabled: true })
    property real low: 0
    property real high: 1
    property int selected: -1
    signal edited(var track)

    property var working: null
    readonly property var shown: sortedKeys(working || root.track.keys || [])
    readonly property bool loop: root.workingLoop !== null && root.workingLoop !== undefined ? root.workingLoop : (root.track.loop_enabled !== false)
    property var workingLoop: null
    readonly property real pad: 8

    function clamp(v, a, b) { return Math.min(b, Math.max(a, v)); }
    function cleanKey(k) {
        return {
            time: clamp(Number(k.time) || 0, 0, 24),
            value: Number(k.value) || 0,
            in_tangent: Number(k.in_tangent) || 0,
            out_tangent: Number(k.out_tangent) || 0
        };
    }
    function sortedKeys(list) {
        return list.map(cleanKey).sort((a, b) => a.time - b.time);
    }
    function span() { return Math.max(1e-6, root.high - root.low); }
    function xOf(t) { return root.pad + clamp(t, 0, 24) / 24 * (canvas.width - 2 * pad); }
    function yOf(v) { return root.pad + (1 - clamp((v - root.low) / span(), 0, 1)) * (canvas.height - 2 * pad); }
    function keyAt(x, y) {
        const t = clamp((x - root.pad) / Math.max(1, canvas.width - 2 * pad), 0, 1) * 24;
        const v = root.low + (1 - clamp((y - root.pad) / Math.max(1, canvas.height - 2 * pad), 0, 1)) * span();
        return { time: Math.round(t * 100) / 100, value: Math.round(v * 1000) / 1000, in_tangent: 0, out_tangent: 0 };
    }
    function nearest(list, x, y) {
        let best = -1, bestDistance = 10;
        for (let i = 0; i < list.length; ++i) {
            const d = Math.hypot(xOf(list[i].time) - x, yOf(list[i].value) - y);
            if (d < bestDistance) { best = i; bestDistance = d; }
        }
        return best;
    }
    // The engine's Hermite sample, so the preview is what the game runs.
    function sampledAt(list, t) {
        if (!list.length) return null;
        if (list.length === 1) return list[0].value;
        if (root.loop) t = ((t % 24) + 24) % 24; else t = clamp(t, 0, 24);
        if (t <= list[0].time) return list[0].value;
        if (t >= list[list.length - 1].time) return list[list.length - 1].value;
        let i = 0;
        while (i + 1 < list.length && list[i + 1].time < t) ++i;
        const a = list[i], b = list[i + 1];
        const s = Math.max(1e-6, b.time - a.time);
        const u = clamp((t - a.time) / s, 0, 1);
        const m0 = a.out_tangent * s, m1 = b.in_tangent * s;
        const u2 = u * u, u3 = u2 * u;
        return (2 * u3 - 3 * u2 + 1) * a.value + (u3 - 2 * u2 + u) * m0
            + (-2 * u3 + 3 * u2) * b.value + (u3 - u2) * m1;
    }
    function commit(list) {
        const keys = sortedKeys(list);
        root.working = null;
        root.workingLoop = null;
        if (root.selected >= keys.length) root.selected = keys.length - 1;
        root.edited({ keys: keys, loop_enabled: root.loop });
    }

    onShownChanged: canvas.requestPaint()
    onLoopChanged: canvas.requestPaint()
    onLowChanged: canvas.requestPaint()
    onHighChanged: canvas.requestPaint()

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: 110
        radius: 5; color: Theme.field; border.color: Theme.border
        Canvas {
            id: canvas
            anchors.fill: parent
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
            onPaint: {
                const ctx = getContext("2d");
                ctx.reset();
                ctx.lineWidth = 1;
                // Hour gridlines at 0/6/12/18/24.
                ctx.strokeStyle = Theme.borderSoft;
                ctx.fillStyle = Theme.textDim;
                ctx.font = "9px sans-serif";
                for (let h = 0; h <= 24; h += 6) {
                    const x = root.xOf(h);
                    ctx.beginPath();
                    ctx.moveTo(x, 4); ctx.lineTo(x, height - 4);
                    ctx.stroke();
                    ctx.fillText(h + "h", x + 2, 11);
                }
                const list = root.shown;
                if (!list.length) {
                    ctx.fillStyle = Theme.textDim;
                    ctx.font = "11px sans-serif";
                    ctx.fillText("No keys - the director leaves this dial alone. Click to add one.", root.pad + 4, height / 2);
                    return;
                }
                ctx.strokeStyle = Theme.accent;
                ctx.lineWidth = 2;
                ctx.beginPath();
                for (let s = 0; s <= 96; ++s) {
                    const v = root.sampledAt(list, s / 4);
                    const x = root.xOf(s / 4), y = root.yOf(v);
                    if (s === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
                }
                ctx.stroke();
                ctx.fillStyle = Theme.accent;
                for (let i = 0; i < list.length; ++i) {
                    ctx.beginPath();
                    ctx.arc(root.xOf(list[i].time), root.yOf(list[i].value), i === root.selected ? 5 : 4, 0, 2 * Math.PI);
                    ctx.fill();
                }
                if (root.selected >= 0 && root.selected < list.length) {
                    ctx.strokeStyle = Theme.text;
                    ctx.beginPath();
                    ctx.arc(root.xOf(list[root.selected].time), root.yOf(list[root.selected].value), 8, 0, 2 * Math.PI);
                    ctx.stroke();
                }
            }
        }
        Text { x: 4; y: 13; text: Number(root.high.toPrecision(4)); color: Theme.textDim; font.pixelSize: 9 }
        Text { x: 4; anchors.bottom: parent.bottom; anchors.bottomMargin: 2; text: Number(root.low.toPrecision(4)); color: Theme.textDim; font.pixelSize: 9 }
        MouseArea {
            anchors.fill: parent
            property int dragging: -1
            onPressed: mouse => {
                const list = root.sortedKeys(root.track.keys || []);
                const i = root.nearest(list, mouse.x, mouse.y);
                if (i < 0) {
                    list.push(root.keyAt(mouse.x, mouse.y));
                    root.selected = list.length - 1;
                    root.working = root.sortedKeys(list);
                    dragging = root.selected;
                } else {
                    root.selected = i;
                    root.working = list;
                    dragging = i;
                }
            }
            onPositionChanged: mouse => {
                if (dragging < 0 || !root.working) return;
                const list = root.working.slice();
                const at = root.keyAt(mouse.x, mouse.y);
                list[dragging] = { time: at.time, value: at.value, in_tangent: list[dragging].in_tangent, out_tangent: list[dragging].out_tangent };
                root.selected = dragging;
                root.working = list;
            }
            onReleased: {
                if (dragging < 0 || !root.working) return;
                const list = root.working.slice();
                // A drag that ends on another key replaces it: no stacks.
                list.sort((a, b) => a.time - b.time);
                const merged = [];
                for (const k of list) {
                    if (merged.length && Math.abs(merged[merged.length - 1].time - k.time) < 1e-6) merged[merged.length - 1] = k;
                    else merged.push(k);
                }
                dragging = -1;
                root.selected = Math.min(root.selected, merged.length - 1);
                root.commit(merged);
            }
            onDoubleClicked: mouse => {
                const list = root.sortedKeys(root.track.keys || []);
                const i = root.nearest(list, mouse.x, mouse.y);
                if (i < 0) return;
                list.splice(i, 1);
                root.selected = -1;
                root.commit(list);
            }
        }
    }

    RowLayout {
        visible: root.selected >= 0 && root.selected < root.shown.length
        Layout.fillWidth: true
        spacing: 6
        Text { text: "Key"; color: Theme.textDim; font.pixelSize: 11 }
        NumberField {
            value: root.selected >= 0 ? root.shown[root.selected].time : 0; fallback: 12
            onCommitted: n => {
                const list = root.sortedKeys(root.track.keys || []);
                if (root.selected < 0 || root.selected >= list.length) return;
                list[root.selected] = Object.assign(list[root.selected], { time: Math.min(24, Math.max(0, n)) });
                root.commit(list);
            }
        }
        NumberField {
            value: root.selected >= 0 ? root.shown[root.selected].value : 0; fallback: 0
            onCommitted: n => {
                const list = root.sortedKeys(root.track.keys || []);
                if (root.selected < 0 || root.selected >= list.length) return;
                list[root.selected] = Object.assign(list[root.selected], { value: n });
                root.commit(list);
            }
        }
        Text { text: "In"; color: Theme.textDim; font.pixelSize: 11 }
        NumberField {
            value: root.selected >= 0 ? root.shown[root.selected].in_tangent : 0; fallback: 0
            onCommitted: n => {
                const list = root.sortedKeys(root.track.keys || []);
                if (root.selected < 0 || root.selected >= list.length) return;
                list[root.selected] = Object.assign(list[root.selected], { in_tangent: n });
                root.commit(list);
            }
        }
        Text { text: "Out"; color: Theme.textDim; font.pixelSize: 11 }
        NumberField {
            value: root.selected >= 0 ? root.shown[root.selected].out_tangent : 0; fallback: 0
            onCommitted: n => {
                const list = root.sortedKeys(root.track.keys || []);
                if (root.selected < 0 || root.selected >= list.length) return;
                list[root.selected] = Object.assign(list[root.selected], { out_tangent: n });
                root.commit(list);
            }
        }
        BwButton {
            text: "Remove"; implicitHeight: 28
            onClicked: {
                const list = root.sortedKeys(root.track.keys || []);
                if (root.selected < 0 || root.selected >= list.length) return;
                list.splice(root.selected, 1);
                root.selected = -1;
                root.commit(list);
            }
        }
    }
    RowLayout {
        Layout.fillWidth: true
        spacing: 6
        Text { text: "Loop past midnight"; color: Theme.textDim; font.pixelSize: 11 }
        SwitchField {
            value: root.loop
            onToggled: on => {
                root.workingLoop = on;
                root.commit(root.track.keys || []);
            }
        }
        Item { Layout.fillWidth: true }
        Text { text: (root.shown.length || 0) + " keys"; color: Theme.textDim; font.pixelSize: 11 }
    }
}
