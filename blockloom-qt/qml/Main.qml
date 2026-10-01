import QtQuick
import QtQuick.Controls
import com.blockworked.Blockstitch 1.0
import com.blockworked.Blockloom 1.0

// Two pages: the Dashboard the app starts on, and the editor a project opens
// into. `appState` is the one copy of backend state, replaced wholesale
// whenever the backend publishes a new snapshot; nothing here edits it.
ApplicationWindow {
    id: root
    width: 1480
    height: 920
    minimumWidth: 1000
    minimumHeight: 640
    visible: true
    title: appState.project ? appState.project.name + " - Blockloom" : "Blockloom"
    color: Theme.window

    // Keep every control on the app theme instead of the system palette.
    palette {
        window: Theme.panel; windowText: Theme.text; base: Theme.field; text: Theme.text
        button: Theme.panelRaised; buttonText: Theme.text; highlight: Theme.accent; highlightedText: "white"
        placeholderText: Theme.textDim; toolTipBase: Theme.panelRaised; toolTipText: Theme.text
        mid: Theme.border; dark: Theme.borderSoft; light: Theme.panelRaised; shadow: "#000000"
    }

    property var appState: ({
        library: [], default_project_location: "", default_export_location: "", default_build_location: "", project_path: null, project: null, selected_actor: null,
        can_undo: false, can_redo: false, invalid_field_buffers: [], running: false, paused: false, status: null,
        log: [], log_total: 0, runtime_available: true, runtime_open: false, preview_enabled: false, preview_headless: false,
        preview_port: null, game_size: [960, 720], runtime_embedded: false, pointer_locked: false
    })
    // The run's live status, kept apart from `appState` so it can change
    // several times a second without re-evaluating the whole editor.
    property var status: null
    // The run log, likewise: `total` counts every line ever added, so the log
    // view appends what's new instead of rebuilding.
    property var log: ({ total: 0, lines: [] })
    readonly property var openActor: Blocks.actor
    // The tray entry being dragged onto an asset box, and the boxes that take one.
    property string inspectedLighting: ""
    property bool inspectScene: false
    property string inspectedScene: ""
    function selectScene(id) {
        if (appState.selected_actor) invoke("select_actor", { actorId: "" });
        inspectedLighting = "";
        inspectedScene = id || (appState.project ? appState.project.active_scene : "");
        inspectScene = true;
    }
    function selectLighting(path) {
        if (appState.selected_actor) invoke("select_actor", { actorId: "" });
        inspectedScene = ""; inspectScene = false; inspectedLighting = path;
    }
    property var assetDrag: null
    property var assetTargets: []

    readonly property string previewFrame: bridge.previewFrame
    readonly property string appVersion: bridge.appVersion
    function watchPreview(port) { bridge.watchPreview(port); }
    function physicalKey(scanCode) { return bridge.physicalKey(scanCode); }
    // The watched Android device's newest frame (`{serial, image, width, height,
    // transport, ...}`), or `{error}` once its stream ends. Pushed, not polled.
    property var screenFrame: null
    function watchScreen(serial, width) { bridge.watchScreen(serial || "", width || 0); }
    function screenInput(input) { bridge.screenInput(JSON.stringify(input)); }

    // ─── Talking to the backend ────────────────────────────────────────────
    property var pending: ({})
    property int nextToken: 1
    // Runs a backend command. `done(result)` on success; a failure goes to
    // `failed(error)`, or to the error bar when no handler was given.
    function invoke(command, args, done, failed) {
        if (command === "select_actor") { inspectScene = false; inspectedScene = ""; inspectedLighting = ""; }
        const token = nextToken++;
        pending[token] = { command: command, done: done, failed: failed };
        bridge.invokeCommand(token, command, JSON.stringify(args || {}));
    }
    function showError(text) { errorBar.text = String(text); errorBar.visible = true; errorTimer.restart(); }
    function toFileUrl(path) {
        if (!path) return "";
        const p = String(path).replace(/\\/g, "/");
        return p.startsWith("/") ? "file://" + p : "file:///" + p;
    }
    function fromFileUrl(url) {
        let s = decodeURIComponent(String(url));
        if (s.startsWith("file:///") && /^file:\/\/\/[A-Za-z]:/.test(s)) return s.slice(8);
        return s.startsWith("file://") ? s.slice(7) : s;
    }
    // An asset path relative to the open project folder, as a file URL.
    function assetUrl(path) { return path && appState.project_path ? toFileUrl(appState.project_path + "/" + path) : ""; }

    // `next` with every part that equals the same part of `old` swapped for
    // the old object, so an unchanged actor, project or list stays the same
    // value and nothing bound to it re-evaluates when a snapshot lands.
    function reuse(old, next) {
        if (old === next || old === null || next === null || typeof old !== "object" || typeof next !== "object") return next;
        if (Array.isArray(next) !== Array.isArray(old)) return next;
        let same = true;
        if (Array.isArray(next)) {
            // Match by id where there is one, so an insert doesn't shift every match.
            const byId = new Map();
            for (const o of old) if (o && o.id !== undefined) byId.set(o.id, o);
            for (let i = 0; i < next.length; ++i) {
                const n = next[i];
                next[i] = reuse(n && n.id !== undefined && byId.has(n.id) ? byId.get(n.id) : old[i], n);
                if (next[i] !== old[i]) same = false;
            }
            return same && old.length === next.length ? old : next;
        }
        const keys = Object.keys(next);
        if (keys.length !== Object.keys(old).length) same = false;
        for (const key of keys) {
            next[key] = reuse(old[key], next[key]);
            if (next[key] !== old[key]) same = false;
        }
        return same ? old : next;
    }

    AppBridge {
        id: bridge
        onStateJsonChanged: {
            try { root.appState = root.reuse(root.appState, JSON.parse(stateJson)); }
            catch (error) { console.warn("Invalid Blockloom state", error); return; }
            Blocks.appState = root.appState;
            root.status = root.appState.status || null;
            if (root.appState.log_total !== root.log.total || root.appState.log.length !== root.log.lines.length)
                root.log = { total: root.appState.log_total || 0, lines: root.appState.log || [] };
        }
        // A running world's status, several times a second: only what reads
        // `status` re-evaluates, not everything bound to the whole snapshot.
        onStatusJsonChanged: {
            try { root.status = JSON.parse(statusJson); }
            catch (error) { console.warn("Invalid run status", error); }
        }
        onLogJsonChanged: {
            try { root.log = JSON.parse(logJson); }
            catch (error) { console.warn("Invalid run log", error); }
        }
        onScreenJsonChanged: {
            try { root.screenFrame = screenJson ? JSON.parse(screenJson) : null; }
            catch (error) { console.warn("Invalid device frame", error); }
        }
        onReplied: (token, response) => {
            const call = root.pending[token];
            delete root.pending[token];
            let reply;
            try { reply = JSON.parse(response); } catch (error) { reply = { ok: false, error: String(error) }; }
            if (reply.ok) { if (call && call.done) call.done(reply.result); }
            else if (call && call.failed) call.failed(reply.error);
            else { console.warn((call ? call.command : "command") + " failed:", reply.error); root.showError(reply.error); }
        }
    }
    Component.onCompleted: bridge.start()
    onClosing: bridge.shutdown()

    // Undo and redo, unless a text field wants the keys for itself.
    Shortcut { sequences: [StandardKey.Undo]; enabled: !!root.appState.project; onActivated: root.invoke("undo") }
    Shortcut { sequences: [StandardKey.Redo, "Ctrl+Y"]; enabled: !!root.appState.project; onActivated: root.invoke("redo") }
    Shortcut { sequences: [StandardKey.Save]; enabled: !!root.appState.project; onActivated: root.invoke("save_project") }

    Loader {
        id: pageLoader
        anchors.fill: parent
        sourceComponent: root.appState.project ? editorComponent : dashboardComponent
    }
    Component { id: dashboardComponent; Dashboard { app: root } }
    Component { id: editorComponent; EditorPage { app: root } }

    Rectangle {
        id: errorBar
        property alias text: errorText.text
        visible: false
        anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
        height: Math.max(38, errorText.implicitHeight + 16); color: "#4b2225"; z: 200
        Text { id: errorText; anchors.left: parent.left; anchors.right: closeError.left; anchors.margins: 14; anchors.verticalCenter: parent.verticalCenter; color: "#ffb5b5"; font.pixelSize: 13; wrapMode: Text.WordWrap }
        BwButton { id: closeError; anchors.right: parent.right; anchors.rightMargin: 8; anchors.verticalCenter: parent.verticalCenter; iconName: "x"; text: ""; flat: true; implicitWidth: 28; implicitHeight: 28; onClicked: errorBar.visible = false }
        Timer { id: errorTimer; interval: 8000; onTriggered: errorBar.visible = false }
    }
}
