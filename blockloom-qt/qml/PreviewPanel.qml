import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0
import com.blockworked.Blockloom 1.0

// The Game tab: the game at its real pixel size, with a resolution and an
// aspect ratio to size it by, plus input forwarding. An embedded world
// (Linux) draws straight into GameView on the GPU at exactly the view's
// pixels. A child-process world streams MJPEG instead, and can keep its OS
// window up beside the view or hide it (headless).
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    readonly property bool embedded: appState.runtime_embedded === true
    color: Theme.panel

    // Editor preferences, not the project's: how big this machine shows it.
    Settings {
        id: view
        category: "gameView"
        property string aspect: "16:9"
        property string resolution: "free"
    }
    readonly property real dpr: Screen.devicePixelRatio
    readonly property var aspects: ["free", "16:9", "16:10", "4:3", "21:9", "1:1"]
    readonly property var resolutions: [
        [1280, 720], [1600, 900], [1920, 1080], [2560, 1440], [3840, 2160],
        [1280, 800], [1440, 900], [1920, 1200], [2560, 1600],
        [800, 600], [960, 720], [1024, 768], [1600, 1200],
        [2560, 1080], [3440, 1440],
        [720, 720], [1080, 1080]
    ]
    // [w, h] of an aspect or resolution value, or null for "free".
    function parsePair(value, sep) { const p = String(value).split(sep).map(Number); return p.length === 2 && p[0] > 0 && p[1] > 0 ? p : null; }
    readonly property var ratio: parsePair(view.aspect, ":")
    function matches(res) { return !ratio || res[0] * ratio[1] === res[1] * ratio[0]; }
    readonly property var resolutionOptions: [{ value: "free", label: "Free" }].concat(
        resolutions.filter(matches).map(r => ({ value: r[0] + "x" + r[1], label: r[0] + " × " + r[1] })))
    readonly property var fixed: parsePair(view.resolution, "x")
    // A resolution left over from another aspect ratio goes back to Free.
    onRatioChanged: if (fixed && !matches(fixed)) view.resolution = "free"

    // The game's on-screen size inside `area`, in logical pixels.
    function fitted(area) {
        if (fixed) {
            const w = fixed[0] / dpr, h = fixed[1] / dpr;
            const s = Math.min(1, area.width / w, area.height / h);
            return Qt.size(Math.floor(w * s), Math.floor(h * s));
        }
        if (ratio) {
            const s = Math.min(area.width / ratio[0], area.height / ratio[1]);
            return Qt.size(Math.floor(ratio[0] * s), Math.floor(ratio[1] * s));
        }
        return Qt.size(Math.floor(area.width), Math.floor(area.height));
    }
    readonly property size gameSize: fitted(Qt.size(Math.max(0, frameArea.width - 2), Math.max(0, frameArea.height - 2)))
    // What the world really draws at, and how much of it fits on screen.
    readonly property size drawnSize: fixed ? Qt.size(fixed[0], fixed[1]) : Qt.size(Math.round(gameSize.width * dpr), Math.round(gameSize.height * dpr))
    readonly property int shownPercent: fixed ? Math.round(100 * gameSize.width * dpr / fixed[0]) : 100

    // Play hands the keyboard to the game, the way its own window took it.
    readonly property bool running: appState.running === true
    onRunningChanged: {
        pointerSuspended = false;
        // Later, so the tab switch Play also causes has shown the view.
        if (running && embedded) Qt.callLater(() => { frame.forceActiveFocus(); input({ kind: "focus", focused: attentive }); });
    }
    onVisibleChanged: if (visible && running && embedded) frame.forceActiveFocus()
    // The view has the keyboard in the active window: the game's idea of focus.
    readonly property bool attentive: frame.activeFocus && frame.Window.active
    onAttentiveChanged: if (embedded) input({ kind: "focus", focused: attentive })
    // Escape or the system let go of the pointer; a click on the game takes it back.
    property bool pointerSuspended: false
    // Follow the stream whenever the sidecar has a port.
    readonly property int port: appState.preview_port || 0
    onPortChanged: app.watchPreview(port)
    Component.onCompleted: app.watchPreview(port)
    Component.onDestruction: app.watchPreview(0)

    function report(command, args) { app.invoke(command, args, null, e => app.invoke("push_log", { kind: "error", text: String(e) })); }
    function input(payload) { app.invoke("preview_input", { input: payload }, null, () => {}); }
    // Viewport pixels plus the viewport's size, so the runtime can scale onto its own window.
    function box(x, y) { return { x: x, y: y, w: frame.width, h: frame.height }; }
    // The key by where it sits, so WASD stays WASD on any layout. Qt's key
    // name is the fallback where the platform has no scan code (macOS).
    function keyCode(event) { return app.physicalKey(event.nativeScanCode) || webCode(event); }
    // Qt keys as the `KeyboardEvent.code` names the runtime expects.
    function webCode(event) {
        if (event.key >= Qt.Key_A && event.key <= Qt.Key_Z) return "Key" + String.fromCharCode(65 + event.key - Qt.Key_A);
        if (event.key >= Qt.Key_0 && event.key <= Qt.Key_9) return "Digit" + String.fromCharCode(48 + event.key - Qt.Key_0);
        if (event.key >= Qt.Key_F1 && event.key <= Qt.Key_F24) return "F" + String(event.key - Qt.Key_F1 + 1);
        const codes = {};
        codes[Qt.Key_Space] = "Space"; codes[Qt.Key_Return] = "Enter"; codes[Qt.Key_Enter] = "NumpadEnter"; codes[Qt.Key_Escape] = "Escape";
        codes[Qt.Key_Tab] = "Tab"; codes[Qt.Key_Backspace] = "Backspace"; codes[Qt.Key_Delete] = "Delete"; codes[Qt.Key_Left] = "ArrowLeft";
        codes[Qt.Key_Right] = "ArrowRight"; codes[Qt.Key_Up] = "ArrowUp"; codes[Qt.Key_Down] = "ArrowDown"; codes[Qt.Key_Shift] = "ShiftLeft";
        codes[Qt.Key_Control] = "ControlLeft"; codes[Qt.Key_Alt] = "AltLeft"; codes[Qt.Key_Minus] = "Minus"; codes[Qt.Key_Equal] = "Equal";
        codes[Qt.Key_Comma] = "Comma"; codes[Qt.Key_Period] = "Period"; codes[Qt.Key_Slash] = "Slash"; codes[Qt.Key_Semicolon] = "Semicolon";
        codes[Qt.Key_BracketLeft] = "BracketLeft"; codes[Qt.Key_BracketRight] = "BracketRight"; codes[Qt.Key_Apostrophe] = "Quote";
        codes[Qt.Key_QuoteLeft] = "Backquote"; codes[Qt.Key_Backslash] = "Backslash";
        return codes[event.key] || "";
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 4
        RowLayout {
            Layout.fillWidth: true; Layout.preferredHeight: 36; Layout.leftMargin: 8; Layout.rightMargin: 8; spacing: 6
            SwitchField { visible: !root.embedded; value: root.appState.preview_enabled; onToggled: on => root.report("set_preview_enabled", { enabled: on }) }
            Text { visible: !root.embedded; text: "Preview"; color: Theme.text; font.pixelSize: 12; Layout.rightMargin: 8 }
            SwitchField { visible: root.appState.preview_enabled && !root.embedded; value: root.appState.preview_headless; onToggled: on => root.report("set_preview_headless", { headless: on }) }
            Text { visible: root.appState.preview_enabled && !root.embedded; text: "Headless"; color: Theme.textDim; font.pixelSize: 12 }
            Item { Layout.fillWidth: true }
            Text {
                visible: root.embedded
                text: root.drawnSize.width + " × " + root.drawnSize.height + (root.shownPercent < 100 ? "  (shown at " + root.shownPercent + "%)" : "")
                color: Theme.textDim; font.pixelSize: 11
            }
            Text { text: "Aspect"; color: Theme.textDim; font.pixelSize: 12; Layout.leftMargin: 6 }
            ChoiceField {
                Layout.fillWidth: false; Layout.preferredWidth: 96
                options: root.aspects.map(a => ({ value: a, label: a === "free" ? "Free" : a }))
                value: view.aspect
                onChosen: a => view.aspect = a
            }
            Text { text: "Resolution"; color: Theme.textDim; font.pixelSize: 12; Layout.leftMargin: 6 }
            ChoiceField {
                Layout.fillWidth: false; Layout.preferredWidth: 130
                options: root.resolutionOptions
                value: view.resolution
                onChosen: r => view.resolution = r
            }
        }
        Item {
            id: frameArea
            Layout.fillWidth: true; Layout.fillHeight: true
            Layout.leftMargin: 8; Layout.rightMargin: 8; Layout.bottomMargin: 8
            Rectangle {
                id: frame
                anchors.centerIn: parent
                // The border sits outside the game, so a fixed resolution
                // maps one pixel to one pixel.
                width: root.gameSize.width + 2; height: root.gameSize.height + 2
                color: "black"; border.color: frame.activeFocus ? Theme.accent : Theme.border
                focus: true
                readonly property bool showing: root.embedded ? gameView.hasFrame : !!root.app.previewFrame
                GameView {
                    id: gameView
                    visible: root.embedded
                    anchors.fill: parent; anchors.margins: 1
                    resolution: root.fixed ? Qt.size(root.fixed[0], root.fixed[1]) : Qt.size(0, 0)
                    pointerLocked: root.embedded && root.appState.pointer_locked === true && root.attentive && !root.pointerSuspended
                    onPointerMoved: (dx, dy) => root.input({ kind: "mouse_delta", dx: dx, dy: dy })
                    onPointerReleased: root.pointerSuspended = true
                    onPointerHeldChanged: if (pointerHeld) escapeHint.shown = true
                }
                Image {
                    visible: !root.embedded
                    anchors.fill: parent; anchors.margins: 1
                    source: root.embedded ? "" : root.app.previewFrame; cache: false; fillMode: Image.PreserveAspectFit
                }
                Text {
                    visible: !frame.showing || gameView.error !== ""
                    anchors.centerIn: parent; width: parent.width - 24
                    horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap
                    color: gameView.error ? Theme.danger : Theme.textDim; font.pixelSize: 12
                    text: gameView.error ? "Can't show the game: " + gameView.error
                        : !root.appState.preview_enabled ? "Turn the preview on to see the game here."
                        : root.appState.runtime_open ? "Waiting for the first frame…" : "Press Play to see the game here."
                }
                Rectangle {
                    readonly property string text: !frame.activeFocus ? "Click to control the game"
                        : gameView.pointerLocked && !gameView.pointerHeld ? "Point at the game to lock the pointer"
                        : root.pointerSuspended && root.appState.pointer_locked === true ? "Click to lock the pointer" : ""
                    visible: root.embedded && root.running && frame.showing && text !== ""
                    anchors.bottom: parent.bottom; anchors.horizontalCenter: parent.horizontalCenter; anchors.bottomMargin: 8
                    width: hint.implicitWidth + 16; height: hint.implicitHeight + 8; radius: 4
                    color: "#b0000000"
                    Text { id: hint; anchors.centerIn: parent; text: parent.text; color: "white"; font.pixelSize: 11 }
                }
                Rectangle {
                    id: escapeHint
                    property bool shown: false
                    onShownChanged: if (shown) escapeTimer.restart()
                    visible: shown && gameView.pointerHeld
                    anchors.top: parent.top; anchors.horizontalCenter: parent.horizontalCenter; anchors.topMargin: 8
                    width: escapeText.implicitWidth + 16; height: escapeText.implicitHeight + 8; radius: 4
                    color: "#b0000000"
                    Text { id: escapeText; anchors.centerIn: parent; text: "Press Esc to release the pointer"; color: "white"; font.pixelSize: 11 }
                    Timer { id: escapeTimer; interval: 2500; onTriggered: escapeHint.shown = false }
                }
                MouseArea {
                    anchors.fill: parent; hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
                    // A held pointer is the game's to draw, if it draws one at all.
                    cursorShape: gameView.pointerHeld ? Qt.BlankCursor : Qt.ArrowCursor
                    readonly property var buttons: { const b = {}; b[Qt.LeftButton] = 0; b[Qt.RightButton] = 1; b[Qt.MiddleButton] = 2; return b; }
                    onPositionChanged: mouse => root.input(Object.assign({ kind: "mouse_move" }, root.box(mouse.x, mouse.y)))
                    onPressed: mouse => { frame.forceActiveFocus(); root.pointerSuspended = false; root.input(Object.assign({ kind: "mouse_button", button: buttons[mouse.button], down: true }, root.box(mouse.x, mouse.y))); }
                    onReleased: mouse => root.input(Object.assign({ kind: "mouse_button", button: buttons[mouse.button], down: false }, root.box(mouse.x, mouse.y)))
                    // Notches from a wheel, pixels from a touchpad that reports them.
                    onWheel: wheel => {
                        const px = wheel.pixelDelta, line = px.x === 0 && px.y === 0;
                        const dx = line ? wheel.angleDelta.x / 120 : px.x, dy = line ? wheel.angleDelta.y / 120 : px.y;
                        if (dx !== 0 || dy !== 0) root.input(Object.assign({ kind: "scroll", dx: dx, dy: dy, line: line }, root.box(wheel.x, wheel.y)));
                    }
                }
                // Fingers go to the game as touches, not as a mouse; the mouse passes through.
                MultiPointTouchArea {
                    anchors.fill: parent; mouseEnabled: false
                    function send(points, phase) { for (const p of points) root.input(Object.assign({ kind: "touch", id: p.pointId, phase: phase }, root.box(p.x, p.y))); }
                    onPressed: points => { frame.forceActiveFocus(); send(points, "start"); }
                    onUpdated: points => send(points, "move")
                    onReleased: points => send(points, "end")
                    onCanceled: points => send(points, "cancel")
                }
                Keys.onPressed: event => {
                    // Escape always frees the pointer, and the game still hears it.
                    if (event.key === Qt.Key_Escape && gameView.pointerLocked) root.pointerSuspended = true;
                    const code = root.keyCode(event);
                    if (code) root.input({ kind: "key", code: code, down: true });
                    // Printable characters also travel as text, for a focused in-game input.
                    if (event.text.length === 1 && !(event.modifiers & Qt.ControlModifier)) root.input({ kind: "text", text: event.text });
                    event.accepted = true;
                }
                Keys.onReleased: event => { const code = root.keyCode(event); if (code && !event.isAutoRepeat) root.input({ kind: "key", code: code, down: false }); event.accepted = true; }
            }
        }
    }
}
