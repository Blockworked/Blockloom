import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Every metric the running world reports, grouped by the part before the
// first "/", with the latest value and the average and peak over the last
// few seconds of status. A filter narrows the rows by name.
Rectangle {
    id: root
    required property var app
    color: Theme.panel
    readonly property var metrics: app.status && app.status.render_metrics ? app.status.render_metrics : []
    // Recent values by metric name, newest last.
    property var history: ({})
    readonly property int keep: 30
    property string filter: ""
    property bool paused: false
    onMetricsChanged: record()
    function record() {
        if (root.paused) return;
        const next = {};
        for (const m of root.metrics) {
            const was = root.history[m.name] || [];
            next[m.name] = was.concat([m.value]).slice(-root.keep);
        }
        root.history = next;
    }
    function stats(name) {
        const h = root.history[name] || [];
        if (!h.length) return { avg: 0, peak: 0 };
        return { avg: h.reduce((a, b) => a + b, 0) / h.length, peak: Math.max(...h) };
    }
    function format(value, unit) {
        if (unit === "bytes") return (value / 1048576).toFixed(1) + " MiB";
        if (unit === "ms") return value.toFixed(2) + " ms";
        if (Number.isInteger(value)) return String(value) + (unit && unit !== "count" ? " " + unit : "");
        return value.toFixed(2) + (unit && unit !== "count" ? " " + unit : "");
    }
    // Rows in name order, a heading before each group.
    readonly property var rows: {
        const wanted = root.filter.trim().toLowerCase();
        const list = root.metrics.filter(m => !wanted || m.name.toLowerCase().indexOf(wanted) >= 0)
            .slice().sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
        const out = [];
        let group = null;
        for (const m of list) {
            const g = m.name.split("/")[0];
            if (g !== group) { out.push({ heading: true, name: g }); group = g; }
            out.push({ heading: false, name: m.name, value: m.value, unit: m.unit });
        }
        return out;
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 0
        RowLayout {
            Layout.fillWidth: true; Layout.preferredHeight: 34; Layout.leftMargin: 8; Layout.rightMargin: 8; spacing: 8
            StitchTextField {
                objectName: "profiler-filter"
                Layout.fillWidth: true; placeholderText: "Filter metrics (gpu, loop, physics...)"
                onTextChanged: root.filter = text
            }
            IconButton {
                objectName: "profiler-pause"
                iconName: root.paused ? "play" : "pause"; tip: root.paused ? "Keep recording" : "Hold these numbers"
                implicitWidth: 26; implicitHeight: 26
                onClicked: root.paused = !root.paused
            }
        }
        Text {
            visible: root.metrics.length === 0
            Layout.fillWidth: true; Layout.margins: 12; wrapMode: Text.WordWrap
            text: "Press Play to see what each part of the frame costs."
            color: Theme.textDim; font.pixelSize: 12
        }
        ListView {
            id: list
            objectName: "profiler-rows"
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: root.rows
            ScrollBar.vertical: ScrollBar {}
            delegate: RowLayout {
                required property var modelData
                width: list.width - 16; x: 8; spacing: 8
                height: modelData.heading ? 24 : 18
                Text {
                    Layout.fillWidth: true; elide: Text.ElideRight
                    text: modelData.heading ? modelData.name : modelData.name.slice(modelData.name.indexOf("/") + 1)
                    color: modelData.heading ? Theme.text : Theme.textDim
                    font.bold: modelData.heading; font.pixelSize: modelData.heading ? 12 : 11
                    leftPadding: modelData.heading ? 0 : 10
                }
                Text {
                    visible: !modelData.heading
                    Layout.preferredWidth: 90; horizontalAlignment: Text.AlignRight
                    text: modelData.heading ? "" : root.format(modelData.value, modelData.unit)
                    color: Theme.text; font.pixelSize: 11; font.family: "monospace"
                }
                Text {
                    visible: !modelData.heading
                    Layout.preferredWidth: 160; horizontalAlignment: Text.AlignRight
                    readonly property var s: root.stats(modelData.name)
                    text: modelData.heading ? "" : "avg " + root.format(s.avg, modelData.unit) + "  peak " + root.format(s.peak, modelData.unit)
                    color: Theme.textDim; font.pixelSize: 11; font.family: "monospace"
                }
            }
        }
    }
}
