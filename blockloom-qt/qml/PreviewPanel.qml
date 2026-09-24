import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The embedded preview: the runtime's MJPEG sidecar shown in place, with run
// controls, a resolution switch, headless mode, single-step and input
// forwarding. Windowed mode keeps the OS game window up beside the viewport;
// headless hides it while the hidden window keeps rendering the stream.
Rectangle {
    id: root
    required property var app
    readonly property var appState: app.appState
    implicitHeight: 36 + (appState.preview_enabled ? frameArea.height + 8 : 0)
    color: Theme.panel
    border.color: Theme.borderSoft

    readonly property var resolutions: [{ label: "270p", width: 480, height: 270 }, { label: "360p", width: 640, height: 360 }, { label: "540p", width: 960, height: 540 }]
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
            SwitchField { value: root.appState.preview_enabled; onToggled: on => root.report("set_preview_enabled", { enabled: on }) }
            Text { text: "Preview"; color: Theme.text; font.pixelSize: 12; Layout.rightMargin: 8 }
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
            ChoiceField {
                visible: root.appState.preview_enabled
                Layout.fillWidth: false; implicitWidth: 90
                options: root.resolutions.map(r => ({ value: r.width + "x" + r.height, label: r.label }))
                value: root.appState.preview_width + "x" + root.appState.preview_height
                onChosen: v => { const r = root.resolutions.find(r => r.width + "x" + r.height === v); if (r) root.report("set_preview_size", { width: r.width, height: r.height }); }
            }
            SwitchField { visible: root.appState.preview_enabled; value: root.appState.preview_headless; onToggled: on => root.report("set_preview_headless", { headless: on }) }
            Text { visible: root.appState.preview_enabled; text: "Headless"; color: Theme.textDim; font.pixelSize: 12 }
        }
        Item {
            id: frameArea
            visible: root.appState.preview_enabled
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(root.appState.preview_height, 360)
            Rectangle {
                id: frame
                anchors.centerIn: parent
                height: parent.height; width: height * root.appState.preview_width / Math.max(1, root.appState.preview_height)
                color: "black"; border.color: frame.activeFocus ? Theme.accent : Theme.border
                focus: true
                Image {
                    anchors.fill: parent; anchors.margins: 1
                    source: root.app.previewFrame; cache: false; fillMode: Image.PreserveAspectFit
                }
                Text {
                    visible: !root.app.previewFrame
                    anchors.centerIn: parent; color: Theme.textDim; font.pixelSize: 12
                    text: root.appState.running ? "Waiting for the first frame…" : "Press Play to see the game here."
                }
                MouseArea {
                    anchors.fill: parent; hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
                    readonly property var buttons: { const b = {}; b[Qt.LeftButton] = 0; b[Qt.MiddleButton] = 1; b[Qt.RightButton] = 2; return b; }
                    onPositionChanged: mouse => root.input(Object.assign({ kind: "mouse_move" }, root.box(mouse.x, mouse.y)))
                    onPressed: mouse => { frame.forceActiveFocus(); root.input(Object.assign({ kind: "mouse_button", button: buttons[mouse.button], down: true }, root.box(mouse.x, mouse.y))); }
                    onReleased: mouse => root.input(Object.assign({ kind: "mouse_button", button: buttons[mouse.button], down: false }, root.box(mouse.x, mouse.y)))
                }
                Keys.onPressed: event => {
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
