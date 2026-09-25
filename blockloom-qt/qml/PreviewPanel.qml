import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0
import com.blockworked.Blockloom 1.0

// The Game view, with run controls, single-step and input forwarding. The
// game draws at its own window size and is scaled to fit, so the view shows
// exactly what a player sees. An embedded world (Linux) draws straight into GameView
// on the GPU. A child-process world streams MJPEG instead, and can keep its
// OS window up beside the view or hide it (headless).
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    readonly property bool embedded: appState.runtime_embedded === true
    implicitHeight: 36 + (appState.preview_enabled ? frameArea.height + 8 : 0)
    color: Theme.panel
    border.color: Theme.borderSoft

    readonly property var gameSize: appState.game_size || [960, 720]
    // Play hands the keyboard to the game, the way its own window took it.
    readonly property bool running: appState.running === true
    onRunningChanged: {
        pointerSuspended = false;
        if (running && embedded) {
            frame.forceActiveFocus();
            input({ kind: "focus", focused: attentive });
        }
    }
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
        return codes[event.key] || "";
    }

    ColumnLayout {
        anchors.fill: parent; spacing: 4
        RowLayout {
            Layout.fillWidth: true; Layout.preferredHeight: 32; Layout.leftMargin: 6; Layout.rightMargin: 6; spacing: 4
            SwitchField { visible: !root.embedded; value: root.appState.preview_enabled; onToggled: on => root.report("set_preview_enabled", { enabled: on }) }
            Text { text: root.embedded ? "Game" : "Preview"; color: Theme.text; font.pixelSize: 12; Layout.rightMargin: 8 }
            IconButton {
                visible: root.appState.preview_enabled
                iconName: root.appState.running ? "square" : "play"; tip: root.appState.running ? "Stop" : "Play"
                onClicked: root.report(root.appState.running ? "stop_project" : "run_project")
            }
            IconButton {
                visible: root.appState.preview_enabled && root.appState.running
                iconName: root.appState.paused ? "play" : "pause"; tip: root.appState.paused ? "Resume" : "Pause"
                onClicked: root.report("pause_project", { paused: !root.appState.paused })
            }
            IconButton { visible: root.appState.preview_enabled && root.appState.running && root.appState.paused; iconName: "step-forward"; tip: "Advance one tick"; onClicked: root.report("step_project") }
            Item { Layout.fillWidth: true }
            SwitchField { visible: root.appState.preview_enabled && !root.embedded; value: root.appState.preview_headless; onToggled: on => root.report("set_preview_headless", { headless: on }) }
            Text { visible: root.appState.preview_enabled && !root.embedded; text: "Headless"; color: Theme.textDim; font.pixelSize: 12 }
        }
        Item {
            id: frameArea
            visible: root.appState.preview_enabled
            Layout.fillWidth: true
            Layout.preferredHeight: 360
            Rectangle {
                id: frame
                anchors.centerIn: parent
                height: parent.height; width: height * root.gameSize[0] / root.gameSize[1]
                color: "black"; border.color: frame.activeFocus ? Theme.accent : Theme.border
                focus: true
                readonly property bool showing: root.embedded ? gameView.hasFrame : !!root.app.previewFrame
                GameView {
                    id: gameView
                    visible: root.embedded
                    anchors.fill: parent; anchors.margins: 1
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
                }
                Keys.onPressed: event => {
                    // Escape always frees the pointer, and the game still hears it.
                    if (event.key === Qt.Key_Escape && gameView.pointerLocked) root.pointerSuspended = true;
                    const code = root.webCode(event);
                    if (code) root.input({ kind: "key", code: code, down: true });
                    // Printable characters also travel as text, for a focused in-game input.
                    if (event.text.length === 1 && !(event.modifiers & Qt.ControlModifier)) root.input({ kind: "text", text: event.text });
                    event.accepted = true;
                }
                Keys.onReleased: event => { const code = root.webCode(event); if (code && !event.isAutoRepeat) root.input({ kind: "key", code: code, down: false }); event.accepted = true; }
            }
        }
    }
}
