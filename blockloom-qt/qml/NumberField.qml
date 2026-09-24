import QtQuick
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// A number box that commits on Enter or leaving it. `value` is what the
// document holds; `committed(n)` says what was typed, already parsed.
BwTextField {
    id: root
    property var value: 0
    property real fallback: 0
    // An empty box commits null instead of the fallback.
    property bool allowEmpty: false
    signal committed(var number)
    Layout.fillWidth: true
    Layout.minimumWidth: 44
    implicitHeight: 30; font.pixelSize: 12
    // Seven significant digits: enough for any dial, and no f32 noise.
    function shown(v) { return v === null || v === undefined ? "" : (Number.isInteger(v) ? String(v) : String(Number(Number(v).toPrecision(7)))); }
    text: shown(value)
    selectByMouse: true
    onEditingFinished: {
        const raw = text.trim();
        if (raw === "" && allowEmpty) { if (value !== null) committed(null); return; }
        const n = Number(raw);
        const next = raw !== "" && Number.isFinite(n) ? n : fallback;
        if (next !== value) committed(next);
        else text = shown(value);
    }
}
