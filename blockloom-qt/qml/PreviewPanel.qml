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
//
// While nothing runs it is the scene view: the same world, loaded but not
// started, seen through an editor camera, with the input driving that camera
// and a gizmo instead of the game.
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
    // How the scene view edits. Also the editor's, not the project's.
    Settings {
        id: scene
        category: "sceneView"
        property bool enabled: true
        property string tool: "move"
        property bool local: false
        property bool snap: false
        property real grid2d: 10
        property real grid3d: 0.5
        property real angle: 15
        property real scaleStep: 0.1
        property bool showGrid: true
        property string debugView: "lit"
    }
    readonly property bool is3d: !!appState.project && appState.project.world.mode === "ThreeD"
    readonly property var sceneView: ({
        enabled: scene.enabled, tool: scene.tool, local: scene.local, snap: scene.snap,
        grid: is3d ? scene.grid3d : scene.grid2d, angle: scene.angle, scale: scene.scaleStep, show_grid: scene.showGrid,
        debug_view: scene.debugView
    })
    onSceneViewChanged: app.invoke("set_scene_view", { view: sceneView }, null, () => {})
    // The scene view is what's showing: a world is up and nothing runs.
    readonly property bool editing: !running && scene.enabled && appState.runtime_open === true
    // The right button is held to fly, with the pointer held still.
    property bool looking: false
    onEditingChanged: looking = false
    // A world to edit comes up whenever the tab is looked at without one, as
    // long as it would draw here rather than in a window of its own.
    function openWorld() {
        if (visible && !!appState.project && appState.runtime_available && appState.runtime_open !== true
                && (embedded || appState.preview_enabled === true))
            report("open_world");
    }
    readonly property string projectPath: appState.project_path || ""
    onProjectPathChanged: Qt.callLater(openWorld)
    function setTool(tool) { scene.tool = tool; }
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
    onVisibleChanged: { if (visible && running && embedded) frame.forceActiveFocus(); openWorld(); }
    // The view has the keyboard in the active window: the game's idea of focus.
    readonly property bool attentive: frame.activeFocus && frame.Window.active
    onAttentiveChanged: if (embedded) input({ kind: "focus", focused: attentive })
    // Escape or the system let go of the pointer; a click on the game takes it back.
    property bool pointerSuspended: false
    // Follow the stream whenever the sidecar has a port.
    readonly property int port: appState.preview_port || 0
    onPortChanged: app.watchPreview(port)
    Component.onCompleted: { app.watchPreview(port); app.invoke("set_scene_view", { view: sceneView }, null, () => {}); Qt.callLater(openWorld); }
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
            Text { text: "View"; color: Theme.textDim; font.pixelSize: 12; Layout.leftMargin: 6 }
            ChoiceField {
                Layout.fillWidth: false; Layout.preferredWidth: 120
                options: [{ value: "lit", label: "Lit" }, { value: "false_color", label: "False color" }, { value: "clipping", label: "Clipping" },
                          { value: "histogram", label: "Histogram" }, { value: "waveform", label: "Waveform" },
                          { value: "calibration", label: "Calibration" }, { value: "hdr_preview", label: "HDR preview" }]
                value: scene.debugView
                onChosen: v => scene.debugView = v
                ToolTip.visible: hovered; ToolTip.delay: 500
                ToolTip.text: "False color bands the exposed image by stops: green is middle grey, yellow nears white, red is past it.\n"
                    + "Clipping stripes whatever the display can't show. Histogram and waveform plot luminance in stops.\n"
                    + "Calibration shows patches at black, paper white and peak brightness.\n"
                    + "HDR preview shows the HDR output at paper white, clipping what only an HDR display could show."
            }
            IconButton {
                visible: root.embedded || root.appState.preview_enabled
                iconName: "camera"; tip: "Save this frame as an EXR file (linear, before tonemapping)"
                onClicked: root.report("capture_exr", {})
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
                    pointerLocked: root.embedded && root.attentive && (root.editing ? root.looking && root.is3d
                        : root.appState.pointer_locked === true && !root.pointerSuspended)
                    onPointerMoved: (dx, dy) => root.input({ kind: "mouse_delta", dx: dx, dy: dy })
                    onPointerReleased: root.pointerSuspended = true
                    onPointerHeldChanged: if (pointerHeld && root.running) escapeHint.shown = true
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
                        : root.appState.runtime_open ? "Waiting for the first frame…" : "Press Play, or open the scene to edit it here."
                }
                BwButton {
                    visible: !frame.showing && !root.appState.runtime_open && root.appState.runtime_available && gameView.error === ""
                    anchors.horizontalCenter: parent.horizontalCenter; anchors.top: parent.verticalCenter; anchors.topMargin: 20
                    text: "Open the scene"; iconName: "box"
                    onClicked: root.report("open_world")
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
                    onPressed: mouse => {
                        frame.forceActiveFocus(); root.pointerSuspended = false;
                        if (root.editing && mouse.button === Qt.RightButton) root.looking = true;
                        root.input(Object.assign({ kind: "mouse_button", button: buttons[mouse.button], down: true }, root.box(mouse.x, mouse.y)));
                    }
                    onReleased: mouse => {
                        if (mouse.button === Qt.RightButton) root.looking = false;
                        root.input(Object.assign({ kind: "mouse_button", button: buttons[mouse.button], down: false }, root.box(mouse.x, mouse.y)));
                    }
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
                // The scene view's tools, over the top-left of the view.
                Rectangle {
                    visible: !root.running && root.appState.runtime_open === true
                    anchors.left: parent.left; anchors.top: parent.top; anchors.margins: 8
                    width: tools.implicitWidth + 8; height: tools.implicitHeight + 8; radius: 6
                    color: "#d0202124"; border.color: Theme.borderSoft
                    // Swallows clicks between the buttons, so they never pick in the world.
                    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onWheel: wheel => wheel.accepted = true }
                    Row {
                        id: tools
                        anchors.centerIn: parent; spacing: 2
                        ToolToggle { icon: "box"; tip: scene.enabled ? "Scene view: editing through the editor camera. Click to look through the game's camera." : "Game camera: click to edit the scene"; checked: scene.enabled; onClicked: scene.enabled = !scene.enabled }
                        Rectangle { width: 1; height: 20; color: Theme.border; anchors.verticalCenter: parent.verticalCenter; visible: scene.enabled }
                        ToolToggle { visible: scene.enabled; icon: "move"; tip: "Move (W)"; checked: scene.tool === "move"; onClicked: root.setTool("move") }
                        ToolToggle { visible: scene.enabled; icon: "rotate-cw"; tip: "Rotate (E)"; checked: scene.tool === "rotate"; onClicked: root.setTool("rotate") }
                        ToolToggle { visible: scene.enabled; icon: "scale"; tip: "Scale (R)"; checked: scene.tool === "scale"; onClicked: root.setTool("scale") }
                        ToolToggle { visible: scene.enabled && root.is3d; icon: "move-3d"; tip: scene.local ? "Local axes: the actor's own" : "World axes"; checked: scene.local; onClicked: scene.local = !scene.local }
                        Rectangle { width: 1; height: 20; color: Theme.border; anchors.verticalCenter: parent.verticalCenter; visible: scene.enabled }
                        ToolToggle { visible: scene.enabled; icon: "layout-grid"; tip: "Snap to the grid (hold Ctrl to flip)"; checked: scene.snap; onClicked: scene.snap = !scene.snap }
                        NumberField {
                            visible: scene.enabled && scene.snap
                            width: 52; implicitHeight: 26; anchors.verticalCenter: parent.verticalCenter
                            value: root.is3d ? scene.grid3d : scene.grid2d; fallback: root.is3d ? 0.5 : 10
                            ToolTip.visible: hovered; ToolTip.delay: 500; ToolTip.text: root.is3d ? "Grid step, in metres" : "Grid step, in pixels"
                            onCommitted: n => { const v = Math.max(0.001, Number(n)); if (root.is3d) scene.grid3d = v; else scene.grid2d = v; }
                        }
                        NumberField {
                            visible: scene.enabled && scene.snap
                            width: 44; implicitHeight: 26; anchors.verticalCenter: parent.verticalCenter
                            value: scene.angle; fallback: 15
                            ToolTip.visible: hovered; ToolTip.delay: 500; ToolTip.text: "Angle step, in degrees"
                            onCommitted: n => scene.angle = Math.max(0.1, Number(n))
                        }
                        ToolToggle { visible: scene.enabled; icon: "hash"; tip: "Show the grid (G)"; checked: scene.showGrid; onClicked: scene.showGrid = !scene.showGrid }
                        ToolToggle { visible: scene.enabled; icon: "crosshair"; tip: "Frame the selected actor (F)"; onClicked: root.report("frame_selected") }
                        ToolToggle {
                            visible: scene.enabled; icon: "info"
                            tip: root.is3d
                                ? "Hold the right button to look around, and fly with W A S D, Q and E (Shift to hurry, wheel for speed).\nMiddle-drag pans, Alt-drag orbits, the wheel dollies.\nClick an actor to select it; drag it or its handles to place it. Esc cancels a drag."
                                : "Right- or middle-drag pans, the wheel zooms.\nClick an actor to select it; drag it or its handles to place it. Esc cancels a drag."
                        }
                    }
                }
                Keys.onPressed: event => {
                    // Escape always frees the pointer, and the game still hears it.
                    if (event.key === Qt.Key_Escape && gameView.pointerLocked) root.pointerSuspended = true;
                    const code = root.keyCode(event);
                    // The scene view's own keys, by where they sit; flying uses the same ones.
                    if (root.editing && !root.looking && !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier)) && !event.isAutoRepeat) {
                        const tools = { KeyW: "move", KeyE: "rotate", KeyR: "scale" };
                        if (tools[code]) root.setTool(tools[code]);
                        else if (code === "KeyF") root.report("frame_selected");
                        else if (code === "KeyG") scene.showGrid = !scene.showGrid;
                    }
                    if (code) root.input({ kind: "key", code: code, down: true });
                    // Printable characters also travel as text, for a focused in-game input.
                    if (event.text.length === 1 && !(event.modifiers & Qt.ControlModifier)) root.input({ kind: "text", text: event.text });
                    event.accepted = true;
                }
                Keys.onReleased: event => { const code = root.keyCode(event); if (code && !event.isAutoRepeat) root.input({ kind: "key", code: code, down: false }); event.accepted = true; }
            }
        }
    }

    // One button of the scene view's toolbar, lit while its setting is on.
    component ToolToggle: Rectangle {
        id: toggle
        property string icon: ""
        property string tip: ""
        property bool checked: false
        signal clicked()
        width: 26; height: 26; radius: 4
        anchors.verticalCenter: parent ? parent.verticalCenter : undefined
        color: checked ? Theme.accent : toggleMouse.containsMouse ? "#3b3c40" : "transparent"
        LucideIcon { anchors.centerIn: parent; width: 15; height: 15; name: toggle.icon; color: toggle.checked ? "white" : Theme.text }
        MouseArea { id: toggleMouse; anchors.fill: parent; hoverEnabled: true; onClicked: toggle.clicked() }
        ToolTip.visible: toggleMouse.containsMouse && tip.length > 0
        ToolTip.delay: 500
        ToolTip.text: tip
    }
}
