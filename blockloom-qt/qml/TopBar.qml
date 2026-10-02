import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The open project's name, the way back to the Dashboard, the door to the
// project settings, and the run controls.
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    implicitHeight: 52
    color: Theme.panel
    border.color: Theme.borderSoft

    // A command that fails has nowhere else to say so - the run log is where
    // the user is already looking for what went wrong.
    function report(command, args) { app.invoke(command, args, null, e => app.invoke("push_log", { kind: "error", text: String(e) })); }

    RowLayout {
        anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 10; spacing: 4
        IconButton { iconName: "layout-grid"; tip: "All projects"; onClicked: root.report("close_project") }
        BwTextField {
            Layout.preferredWidth: 240
            text: root.appState.project ? root.appState.project.name : ""
            placeholderText: "Project name"
            ToolTip.visible: hovered && !!root.appState.project_path; ToolTip.text: root.appState.project_path || ""; ToolTip.delay: 700
            onEditingFinished: if (root.appState.project && text !== root.appState.project.name) root.report("set_project_name", { name: text })
        }
        IconButton { iconName: "save"; tip: "Save now"; onClicked: root.app.invoke("save_project") }
        IconButton { iconName: "upload"; tip: "Import a project"; onClicked: importFile.open() }
        IconButton { iconName: "download"; tip: "Export this project"; onClicked: root.app.invoke("export_file_name", {}, name => { const base = root.appState.default_export_location || root.appState.default_project_location; exportFile.currentFile = root.app.toFileUrl(base + "/" + name); exportFile.open(); }) }
        IconButton { iconName: "package"; tip: "Build a standalone game"; onClicked: buildDialog.open() }
        IconButton { iconName: "plug-zap"; tip: "Plugins"; onClicked: pluginManager.open() }
        IconButton {
            iconName: "panel-left"; tip: "Plugin panels"
            visible: !!(root.appState.plugins && root.appState.plugins.panels && root.appState.plugins.panels.length > 0)
            onClicked: pluginPanels.open()
        }
        IconButton { iconName: "settings"; tip: "Project settings"; onClicked: settingsDialog.open() }
        IconButton { iconName: "smartphone"; tip: "App settings (Android SDK)"; onClicked: appSettings.open() }
        Item { Layout.fillWidth: true }
        IconButton { iconName: "undo-2"; tip: "Undo"; enabled: root.appState.can_undo; onClicked: root.app.invoke("undo") }
        IconButton { iconName: "redo-2"; tip: "Redo"; enabled: root.appState.can_redo; onClicked: root.app.invoke("redo") }
        Rectangle {
            visible: root.appState.running
            implicitWidth: fpsText.implicitWidth + 16; implicitHeight: 24; radius: 12; color: Theme.panelRaised; border.color: Theme.border
            Text { id: fpsText; anchors.centerIn: parent; text: Math.round(root.app.status ? root.app.status.fps : 0) + " fps"; color: Theme.textDim; font.pixelSize: 11 }
            MouseArea { anchors.fill: parent; onClicked: profilerPopup.visible ? profilerPopup.close() : profilerPopup.open() }
            ToolTip.visible: fpsHover.hovered; ToolTip.text: "Show render timings"; ToolTip.delay: 500
            HoverHandler { id: fpsHover }
        }
        IconButton { visible: root.appState.runtime_open; iconName: "monitor-x"; tip: "Close the game window"; onClicked: root.app.invoke("close_runtime") }
        IconButton {
            visible: root.appState.running
            iconName: root.appState.paused ? "play" : "pause"; tip: root.appState.paused ? "Resume" : "Pause"
            onClicked: root.app.invoke("pause_project", { paused: !root.appState.paused })
        }
        IconButton { visible: root.appState.running && root.appState.paused; iconName: "step-forward"; tip: "Advance one tick"; onClicked: root.report("step_project") }
        BwButton {
            Layout.leftMargin: 6
            primary: !root.appState.running; danger: root.appState.running
            iconName: root.appState.running ? "square" : "play"
            text: root.appState.running ? "Stop" : "Play"
            onClicked: root.report(root.appState.running ? "stop_project" : "run_project")
        }
    }

    Popup {
        id: profilerPopup
        x: root.width - width - 110; y: root.height
        width: 350
        height: Math.min(480, metricsColumn.implicitHeight + padding * 2)
        padding: 12
        // Escape closes it, but clicks elsewhere must not: the point is to
        // watch these numbers while driving the game.
        closePolicy: Popup.CloseOnEscape
        background: Rectangle { color: Theme.panel; border.color: Theme.border; radius: 6 }
        function metric(name) {
            const all = root.app.status && root.app.status.render_metrics ? root.app.status.render_metrics : [];
            return all.find(m => m.name === name) || null;
        }
        function allMetrics() {
            return root.app.status && root.app.status.render_metrics ? root.app.status.render_metrics : [];
        }
        function formatBytes(v) { return (v / 1048576).toFixed(1) + " MiB"; }
        function formatMs(v) { return v.toFixed(2) + " ms"; }
        function memLabel(name) {
            const labels = {
                "memory/targets": "Render targets total",
                "memory/targets/hdr": "HDR targets (of color)",
                "memory/targets/color": "Color targets",
                "memory/targets/depth": "Depth targets",
                "memory/targets/prepass": "Prepass targets",
                "memory/targets/shadow": "Shadow maps",
                "memory/targets/post": "Post passes",
                "memory/targets/game_view": "Game view ring",
                "memory/targets/game_view_minimum": "Game view minimum",
                "memory/images": "Images",
                "memory/other": "Other",
                "memory/gpu_allocated": "GPU allocated",
                "memory/gpu_reserved": "GPU reserved",
                "memory/mesh_slabs": "Mesh slabs"
            };
            return labels[name] || name;
        }
        function prettyCount(name) {
            const stripped = name.replace(/^(batching|culling|streaming)\//, "");
            return stripped.replace(/_/g, " ");
        }
        // One row per render pass, pairing its GPU and CPU timings and
        // sorting the most expensive first.
        function passRows() {
            const pairs = {};
            for (const m of allMetrics()) {
                if (!m.name.startsWith("render/"))
                    continue;
                const gpu = m.name.endsWith("/elapsed_gpu");
                const cpu = m.name.endsWith("/elapsed_cpu");
                if (!gpu && !cpu)
                    continue;
                const base = m.name.replace(/^render\//, "").replace(/\/(elapsed_gpu|elapsed_cpu)$/, "");
                if (!pairs[base])
                    pairs[base] = { base: base, gpu: null, cpu: null };
                if (gpu)
                    pairs[base].gpu = m.value;
                else
                    pairs[base].cpu = m.value;
            }
            return Object.values(pairs).sort((a, b) => ((b.gpu || 0) + (b.cpu || 0)) - ((a.gpu || 0) + (a.cpu || 0)));
        }
        // Memory rows in a fixed display order, skipping what wasn't measured.
        function memRows() {
            const order = [
                "memory/targets", "memory/targets/hdr", "memory/targets/color",
                "memory/targets/depth", "memory/targets/prepass", "memory/targets/shadow",
                "memory/targets/post", "memory/targets/game_view", "memory/targets/game_view_minimum",
                "memory/images", "memory/other", "memory/gpu_allocated",
                "memory/gpu_reserved", "memory/mesh_slabs"
            ];
            const rows = [];
            for (const name of order) {
                const m = metric(name);
                if (m)
                    rows.push(m);
            }
            return rows;
        }
        // Batching, culling and streaming counts, alphabetical. Loop rows
        // have their own headline above.
        function countRows() {
            return allMetrics()
                .filter(m => m.unit === "count" && m.name.indexOf("loop/") !== 0)
                .slice()
                .sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
        }
        // Per-frame Update segments, most expensive first.
        function updateRows() {
            return allMetrics()
                .filter(m => m.name.indexOf("update/") === 0)
                .slice()
                .sort((a, b) => b.value - a.value);
        }
        contentItem: Flickable {
            clip: true
            contentHeight: metricsColumn.implicitHeight
            Column {
                id: metricsColumn
                width: parent.width
                spacing: 5
                RowLayout {
                    width: parent.width
                    Text { text: "Render profiler"; color: Theme.text; font.bold: true; font.pixelSize: 14; Layout.fillWidth: true }
                    IconButton { iconName: "x"; tip: "Close profiler"; onClicked: profilerPopup.close() }
                }
                Text { text: "Frame: " + (root.app.status && root.app.status.fps ? (1000 / root.app.status.fps).toFixed(1) : "-") + " ms"; color: Theme.textDim; font.pixelSize: 12 }
                Text {
                    readonly property var gpu: profilerPopup.metric("gpu/frame")
                    visible: gpu !== null
                    text: "GPU: " + (gpu ? gpu.value.toFixed(2) : "-") + " ms"
                    color: Theme.textDim; font.pixelSize: 12
                }
                Text {
                    readonly property var tonemap: profilerPopup.metric("hdr/tonemap")
                    visible: tonemap !== null
                    text: "Tonemap: " + (tonemap ? tonemap.value.toFixed(2) : "-") + " ms"
                    color: Theme.textDim; font.pixelSize: 12
                }
                Text {
                    readonly property var loopUpdate: profilerPopup.metric("loop/update")
                    visible: loopUpdate !== null
                    text: "Update (sim + render CPU): " + (loopUpdate ? loopUpdate.value.toFixed(1) : "-") + " ms"
                    color: Theme.textDim; font.pixelSize: 12
                }
                Text {
                    readonly property var loopWait: profilerPopup.metric("loop/present_wait")
                    visible: loopWait !== null
                    text: "Present wait (idle for the view): " + (loopWait ? loopWait.value.toFixed(1) : "-") + " ms"
                    color: Theme.textDim; font.pixelSize: 12
                }
                Text {
                    readonly property var loopMain: profilerPopup.metric("loop/main")
                    visible: loopMain !== null
                    text: "Main schedule: " + (loopMain ? loopMain.value.toFixed(1) : "-") + " ms"
                    color: Theme.textDim; font.pixelSize: 12
                }
                Text {
                    readonly property var loopRender: profilerPopup.metric("loop/render")
                    visible: loopRender !== null
                    text: "Render CPU (extract + encode): " + (loopRender ? loopRender.value.toFixed(1) : "-") + " ms"
                    color: Theme.textDim; font.pixelSize: 12
                }
                Text {
                    readonly property var fixed: profilerPopup.metric("loop/fixed")
                    readonly property var steps: profilerPopup.metric("loop/fixed_steps")
                    visible: fixed !== null
                    text: "Fixed sim: " + (fixed ? fixed.value.toFixed(1) : "-") + " ms" +
                          (steps ? " (" + Math.round(steps.value) + " steps)" : "")
                    color: Theme.textDim; font.pixelSize: 12
                }
                Text {
                    readonly property var flag: profilerPopup.metric("gpu/timestamps")
                    visible: flag !== null && flag.value === 0
                    width: parent.width; wrapMode: Text.Wrap
                    text: "This GPU has no timestamp queries, so only CPU timings show."
                    color: Theme.textDim; font.pixelSize: 11
                }
                Text {
                    readonly property var flag: profilerPopup.metric("memory/exact")
                    visible: flag !== null && flag.value === 0
                    width: parent.width; wrapMode: Text.Wrap
                    text: "This backend has no allocator report, so memory is render targets only, worked out from their sizes."
                    color: Theme.textDim; font.pixelSize: 11
                }
                Text {
                    visible: profilerPopup.passRows().length > 0
                    text: "GPU passes"; color: Theme.text; font.bold: true; font.pixelSize: 12
                }
                Repeater {
                    model: profilerPopup.passRows()
                    Text {
                        required property var modelData
                        width: parent.width; wrapMode: Text.Wrap
                        text: modelData.base + ": " +
                              (modelData.gpu !== null ? "gpu " + modelData.gpu.toFixed(2) + " ms" : "") +
                              (modelData.gpu !== null && modelData.cpu !== null ? " / " : "") +
                              (modelData.cpu !== null ? "cpu " + modelData.cpu.toFixed(2) + " ms" : "")
                        color: Theme.textDim; font.pixelSize: 11
                    }
                }
                Text {
                    visible: profilerPopup.memRows().length > 0
                    text: "Memory"; color: Theme.text; font.bold: true; font.pixelSize: 12
                }
                Repeater {
                    model: profilerPopup.memRows()
                    Text {
                        required property var modelData
                        text: profilerPopup.memLabel(modelData.name) + ": " + profilerPopup.formatBytes(modelData.value)
                        color: Theme.textDim; font.pixelSize: 11
                    }
                }
                Text {
                    visible: profilerPopup.countRows().length > 0
                    text: "Scene"; color: Theme.text; font.bold: true; font.pixelSize: 12
                }
                Repeater {
                    model: profilerPopup.countRows()
                    Text {
                        required property var modelData
                        text: profilerPopup.prettyCount(modelData.name) + ": " + Math.round(modelData.value)
                        color: Theme.textDim; font.pixelSize: 11
                    }
                }
                Text {
                    visible: profilerPopup.updateRows().length > 0
                    text: "Update detail"; color: Theme.text; font.bold: true; font.pixelSize: 12
                }
                Repeater {
                    model: profilerPopup.updateRows()
                    Text {
                        required property var modelData
                        text: modelData.name.replace(/^update\//, "") + ": " + modelData.value.toFixed(2) + " ms"
                        color: Theme.textDim; font.pixelSize: 11
                    }
                }
                Text {
                    visible: !root.app.status || !root.app.status.render_metrics || root.app.status.render_metrics.length === 0
                    text: "Waiting for render measurements"
                    color: Theme.textDim; font.pixelSize: 11
                }
            }
        }
    }

    FileDialog {
        id: importFile
        title: "Import project"
        nameFilters: ["Blockloom project (*.blockloom)"]
        onAccepted: root.report("import_project", { path: root.app.fromFileUrl(selectedFile) })
    }
    FileDialog {
        id: exportFile
        title: "Export project"
        fileMode: FileDialog.SaveFile
        nameFilters: ["Blockloom project (*.blockloom)"]
        onAccepted: root.report("export_project", { path: root.app.fromFileUrl(selectedFile) })
    }
    BuildDialog { id: buildDialog; app: root.app }
    PluginManagerDialog { id: pluginManager; app: root.app }
    PluginPanelDialog { id: pluginPanels; app: root.app }
    ProjectSettingsDialog { id: settingsDialog; app: root.app }
    AppSettingsDialog { id: appSettings; app: root.app }
}
