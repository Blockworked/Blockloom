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
        property bool volumeBounds: true
        property bool volumeHeatmap: false
        property bool volumePanel: false
        property int pathSamples: 256
        property real pathSeconds: 60
        property string brushOp: "Raise"
        property string brushTarget: "Heights"
        property real brushRadius: 8
        property real brushStrength: 0.5
        property real brushFalloff: 0.6
        property real brushStep: 4
        property real brushScale: 12
        property string tileTool: "paint"
        property string tileTiles: "0"
        property string tileAutotile: ""
        property int tileSize: 1
        property real tileDensity: 0.3
        property real tileJitter: 0
        property int tileSeed: 1
        property bool tileCollision: false
        property bool tileRegions: true
        property bool tileRooms: true
        property bool tileParallax: true
    }
    // The reference path tracer is heavy, so it is never remembered on.
    property bool pathTracing: false
    readonly property var tracing: appState.ray_tracing || null
    // Holding the blend still is for this look only, never remembered.
    property bool volumeFreeze: false
    readonly property var volumeStatus: app.status && app.status.volumes ? app.status.volumes : []
    readonly property var volumeTrace: app.status && app.status.volume_trace ? app.status.volume_trace.filter(r => r.steps.length > 0) : []
    readonly property bool is3d: !!appState.project && appState.project.world.mode === "ThreeD"
    readonly property var sceneView: ({
        enabled: scene.enabled, tool: scene.tool, local: scene.local, snap: scene.snap,
        grid: is3d ? scene.grid3d : scene.grid2d, angle: scene.angle, scale: scene.scaleStep, show_grid: scene.showGrid,
        debug_view: scene.debugView,
        volumes: { bounds: scene.volumeBounds, heatmap: scene.volumeHeatmap, freeze: root.volumeFreeze },
        path_tracer: { enabled: root.pathTracing && is3d, samples: scene.pathSamples, seconds: scene.pathSeconds },
        brush: { op: scene.brushOp, target: brushTarget(scene.brushTarget), radius: scene.brushRadius, strength: scene.brushStrength,
                 falloff: scene.brushFalloff, level: null, step: scene.brushStep, scale: scene.brushScale, seed: 1 },
        tile_brush: { tool: scene.tileTool, tiles: tileList(scene.tileTiles), autotile: scene.tileAutotile, size: scene.tileSize,
                      density: scene.tileDensity, jitter: scene.tileJitter, seed: scene.tileSeed },
        tiles: { collision: scene.tileCollision, regions: scene.tileRegions, rooms: scene.tileRooms, parallax: scene.tileParallax }
    })
    // "3, 7 12" -> [3, 7, 12]: the tiles a brush paints with, variants after the first.
    function tileList(text) { const l = String(text).split(/[\s,]+/).filter(t => t !== "").map(Number).filter(n => Number.isInteger(n) && n >= 0); return l.length ? l : [0]; }
    // The selected actor's tilemap, which the Tiles tool paints on.
    readonly property var tileMap: {
        const p = appState.project;
        if (!p || !appState.selected_actor) return null;
        const a = p.actors.find(x => x.id === appState.selected_actor);
        const c = a ? a.components.find(x => x.component === "Look") : null;
        return c && c.visual && c.visual.shape === "Tilemap" ? c.visual.tilemap : null;
    }
    // A pick in the view hands its tile to the brush.
    readonly property int pickSerial: appState.picked_tile ? appState.picked_tile.serial : 0
    onPickSerialChanged: if (appState.picked_tile && appState.picked_tile.tile >= 0) { scene.tileTiles = String(appState.picked_tile.tile); scene.tileTool = "paint"; }
    // "Layer:1" stands for { kind: "Layer", layer: 1 }.
    function brushTarget(name) {
        const parts = name.split(":");
        return parts.length > 1 ? { kind: parts[0], layer: Number(parts[1]) } : { kind: parts[0] };
    }
    // The selected actor's terrain, which the Brush tool paints on.
    readonly property var brushTerrain: {
        const p = appState.project;
        if (!p || !appState.selected_actor) return null;
        const a = p.actors.find(x => x.id === appState.selected_actor);
        const c = a ? a.components.find(x => x.component === "Terrain") : null;
        return c ? c.terrain : null;
    }
    readonly property var brushTargets: {
        const t = brushTerrain;
        const list = [{ value: "Heights", label: "Heights" }, { value: "Holes", label: "Holes" }];
        if (!t) return list;
        (t.layers || []).forEach((l, i) => list.push({ value: "Layer:" + i, label: "Paint " + l.name }));
        (t.grass || []).forEach((g, i) => list.push({ value: "Grass:" + i, label: "Grass " + g.name }));
        (t.scatter || []).forEach((g, i) => list.push({ value: "Scatter:" + i, label: "Scatter " + g.name }));
        return list;
    }
    readonly property bool brushShapes: scene.brushTarget === "Heights"
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
                          .concat(root.is3d ? [{ value: "surface_blend", label: "Surface blend" }] : [])
                value: scene.debugView
                onChosen: v => scene.debugView = v
                ToolTip.visible: hovered; ToolTip.delay: 500
                ToolTip.text: "False color bands the exposed image by stops: green is middle grey, yellow nears white, red is past it.\n"
                    + "Clipping stripes whatever the display can't show. Histogram and waveform plot luminance in stops.\n"
                    + "Calibration shows patches at black, paper white and peak brightness.\n"
                    + "HDR preview shows the HDR output at paper white, clipping what only an HDR display could show.\n"
                    + "Surface blend paints terrain layers red, green, blue and yellow, rule masks magenta, snow white and wetness cyan."
            }
            IconButton {
                iconName: "layers"; tip: "Environment volumes: bounds, heat map, the blend and its lerp"
                onClicked: scene.volumePanel = !scene.volumePanel
            }
            IconButton {
                visible: root.is3d
                iconName: "sparkles"; highlighted: root.pathTracing
                tip: "Path tracer: a converging reference image of the scene's lighting, to check the raster and ray-traced looks against. Not for play."
                onClicked: root.pathTracing = !root.pathTracing
            }
            IconButton {
                visible: root.embedded || root.appState.preview_enabled
                iconName: "camera"; tip: "Save this frame as an EXR file (linear, before tonemapping). With the path tracer on, it waits for the sample budget."
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
                        ToolToggle { visible: scene.enabled && root.is3d; icon: "pencil"; tip: "Terrain brush (B): sculpt, paint, cut holes and place grass or trees on the selected terrain"; checked: scene.tool === "brush"; onClicked: root.setTool("brush") }
                        ToolToggle { visible: scene.enabled; icon: "palette"; tip: "Tiles (T): paint, erase, fill, draw lines and rects, scatter or pick on the selected tilemap"; checked: scene.tool === "tiles"; onClicked: root.setTool("tiles") }
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
                // The terrain brush's settings, under the toolbar.
                Rectangle {
                    visible: root.editing && scene.tool === "brush" && root.is3d
                    anchors.left: parent.left; anchors.top: parent.top; anchors.margins: 8; anchors.topMargin: 48
                    width: brushRows.implicitWidth + 16; height: brushRows.implicitHeight + 16; radius: 6
                    color: "#d0202124"; border.color: Theme.borderSoft
                    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onWheel: wheel => wheel.accepted = true }
                    ColumnLayout {
                        id: brushRows
                        anchors.centerIn: parent; spacing: 4
                        Text { visible: !root.brushTerrain; text: "Select a terrain to brush on."; color: Theme.textDim; font.pixelSize: 11 }
                        RowLayout {
                            spacing: 4
                            ChoiceField { Layout.preferredWidth: 130; options: root.brushTargets; value: scene.brushTarget
                                onChosen: v => { scene.brushTarget = v; if (v === "Heights") { if (scene.brushOp === "Paint" || scene.brushOp === "Erase") scene.brushOp = "Raise"; }
                                                 else if (scene.brushOp !== "Paint" && scene.brushOp !== "Erase") scene.brushOp = "Paint"; } }
                            ChoiceField { Layout.preferredWidth: 100
                                options: Blocks.opts(root.brushShapes ? ["Raise", "Lower", "Smooth", "Flatten", "Noise", "Terrace"] : ["Paint", "Erase"])
                                value: scene.brushOp; onChosen: v => scene.brushOp = v }
                        }
                        RowLayout {
                            spacing: 4
                            Text { text: "Radius"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 48; value: scene.brushRadius; fallback: 8; onCommitted: n => scene.brushRadius = Math.max(0.1, Number(n)) }
                            Text { text: "Strength"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 44; value: scene.brushStrength; fallback: 0.5; onCommitted: n => scene.brushStrength = Math.min(1, Math.max(0, Number(n))) }
                            Text { text: "Falloff"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 44; value: scene.brushFalloff; fallback: 0.6; onCommitted: n => scene.brushFalloff = Math.min(1, Math.max(0, Number(n))) }
                        }
                        RowLayout {
                            visible: scene.brushOp === "Terrace" || scene.brushOp === "Noise"
                            spacing: 4
                            Text { text: scene.brushOp === "Terrace" ? "Step (m)" : "Feature size (m)"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 48
                                value: scene.brushOp === "Terrace" ? scene.brushStep : scene.brushScale; fallback: scene.brushOp === "Terrace" ? 4 : 12
                                onCommitted: n => { const v = Math.max(0.05, Number(n)); if (scene.brushOp === "Terrace") scene.brushStep = v; else scene.brushScale = v; } }
                        }
                    }
                }
                // The Tiles tool's brush, overlays and the tileset to pick from.
                Rectangle {
                    visible: root.editing && scene.tool === "tiles"
                    anchors.left: parent.left; anchors.top: parent.top; anchors.margins: 8; anchors.topMargin: 48
                    width: tileRows.implicitWidth + 16; height: tileRows.implicitHeight + 16; radius: 6
                    color: "#d0202124"; border.color: Theme.borderSoft
                    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onWheel: wheel => wheel.accepted = true }
                    ColumnLayout {
                        id: tileRows
                        anchors.centerIn: parent; spacing: 4
                        Text { visible: !root.tileMap; text: "Select a tilemap to paint on."; color: Theme.textDim; font.pixelSize: 11 }
                        Row {
                            spacing: 2
                            Repeater {
                                model: [["paint", "pencil", "Paint"], ["erase", "eraser", "Erase"], ["fill", "palette", "Fill the connected area"], ["line", "trending-up", "Line"],
                                        ["rect", "square", "Rectangle"], ["scatter", "sparkles", "Scatter by density"], ["pick", "pipette", "Pick the tile under the pointer"]]
                                delegate: ToolToggle { required property var modelData; icon: modelData[1]; tip: modelData[2]; checked: scene.tileTool === modelData[0]; onClicked: scene.tileTool = modelData[0] }
                            }
                        }
                        RowLayout {
                            spacing: 4
                            Text { text: "Tiles"; color: Theme.textDim; font.pixelSize: 11 }
                            BwTextField { Layout.preferredWidth: 90; implicitHeight: 26; font.pixelSize: 11; text: scene.tileTiles
                                ToolTip.visible: hovered; ToolTip.delay: 500; ToolTip.text: "Sheet tiles to paint with; more than one are variants"
                                onEditingFinished: scene.tileTiles = root.tileList(text).join(", ") }
                            Text { text: "Size"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 40; value: scene.tileSize; fallback: 1; onCommitted: n => scene.tileSize = Math.min(16, Math.max(1, Math.round(n))) }
                            ChoiceField { Layout.preferredWidth: 110
                                options: [{ value: "", label: "No autotile" }].concat((root.tileMap && root.tileMap.autotiles ? root.tileMap.autotiles : []).map(s => ({ value: s.name, label: s.name })))
                                value: scene.tileAutotile; onChosen: v => scene.tileAutotile = v }
                        }
                        RowLayout {
                            spacing: 4
                            Text { text: "Density"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 44; value: scene.tileDensity; fallback: 0.3; onCommitted: n => scene.tileDensity = Math.min(1, Math.max(0, n)) }
                            Text { text: "Jitter"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 44; value: scene.tileJitter; fallback: 0; onCommitted: n => scene.tileJitter = Math.min(1, Math.max(0, n)) }
                            Text { text: "Seed"; color: Theme.textDim; font.pixelSize: 11 }
                            NumberField { Layout.preferredWidth: 44; value: scene.tileSeed; fallback: 1; onCommitted: n => scene.tileSeed = Math.max(0, Math.round(n)) }
                        }
                        RowLayout {
                            spacing: 2
                            BwCheckBox { text: "Collision"; checked: scene.tileCollision; onToggled: scene.tileCollision = checked }
                            BwCheckBox { text: "Regions"; checked: scene.tileRegions; onToggled: scene.tileRegions = checked }
                            BwCheckBox { text: "Rooms"; checked: scene.tileRooms; onToggled: scene.tileRooms = checked }
                            BwCheckBox { text: "Parallax"; checked: scene.tileParallax; onToggled: scene.tileParallax = checked
                                ToolTip.visible: hovered; ToolTip.delay: 500; ToolTip.text: "Scroll parallax layers against this camera, as the game's will" }
                        }
                        // The tileset sliced into its sheet: click a tile to paint with it, Shift-click to add a variant.
                        Item {
                            id: sheet
                            visible: !!root.tileMap && root.tileMap.tileset !== "" && sheetImage.status === Image.Ready
                            readonly property int cols: root.tileMap ? Math.max(1, root.tileMap.sheet_columns) : 1
                            readonly property int rows: root.tileMap ? Math.max(1, root.tileMap.sheet_rows) : 1
                            readonly property real scaleBy: sheetImage.implicitWidth > 0 ? Math.min(1, 280 / sheetImage.implicitWidth, 220 / sheetImage.implicitHeight) : 1
                            Layout.preferredWidth: sheetImage.implicitWidth * scaleBy
                            Layout.preferredHeight: sheetImage.implicitHeight * scaleBy
                            readonly property var chosen: root.tileList(scene.tileTiles)
                            Image {
                                id: sheetImage
                                anchors.fill: parent; smooth: false; cache: false; asynchronous: true
                                source: root.tileMap && root.tileMap.tileset !== "" ? root.app.assetUrl(root.tileMap.tileset) : ""
                            }
                            Repeater {
                                model: sheet.cols * sheet.rows
                                delegate: Rectangle {
                                    required property int index
                                    x: (index % sheet.cols) * sheet.width / sheet.cols
                                    y: Math.floor(index / sheet.cols) * sheet.height / sheet.rows
                                    width: sheet.width / sheet.cols; height: sheet.height / sheet.rows
                                    color: "transparent"
                                    border.width: sheet.chosen.indexOf(index) >= 0 ? 2 : 0.5
                                    border.color: sheet.chosen.indexOf(index) >= 0 ? Theme.accent : "#60ffffff"
                                    MouseArea {
                                        anchors.fill: parent; cursorShape: Qt.PointingHandCursor
                                        ToolTip.visible: containsMouse; ToolTip.delay: 400; ToolTip.text: "Tile " + index
                                        hoverEnabled: true
                                        onClicked: mouse => {
                                            if (mouse.modifiers & Qt.ShiftModifier) {
                                                const l = sheet.chosen.filter(t => t !== index);
                                                if (l.length === sheet.chosen.length) l.push(index);
                                                scene.tileTiles = (l.length ? l : [index]).join(", ");
                                            } else scene.tileTiles = String(index);
                                            if (scene.tileTool === "erase" || scene.tileTool === "pick") scene.tileTool = "paint";
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // The path tracer's progress and budget.
                Rectangle {
                    visible: root.pathTracing && root.is3d && root.appState.runtime_open === true
                    anchors.left: parent.left; anchors.bottom: parent.bottom; anchors.margins: 8
                    width: pathRow.implicitWidth + 16; height: pathRow.implicitHeight + 10; radius: 4
                    color: "#e0202124"; border.color: Theme.borderSoft
                    RowLayout {
                        id: pathRow
                        anchors.centerIn: parent; spacing: 6
                        Text {
                            color: root.tracing && !root.tracing.available ? Theme.warning : Theme.text; font.pixelSize: 11
                            text: !root.tracing ? "Path tracer starting…"
                                : !root.tracing.available ? "No path tracer here: " + root.tracing.reason
                                : !root.tracing.path_tracing ? "Path tracer starting…"
                                : "Path tracing  " + root.tracing.samples + " samples, " + Math.round(root.tracing.seconds) + " s"
                                    + (root.tracing.converged ? "  (converged)" : "")
                        }
                        Text { text: "Budget"; color: Theme.textDim; font.pixelSize: 11; Layout.leftMargin: 6 }
                        NumberField { Layout.preferredWidth: 64; value: scene.pathSamples; fallback: 256; onCommitted: n => scene.pathSamples = Math.max(1, Math.round(n)) }
                        Text { text: "samples or"; color: Theme.textDim; font.pixelSize: 11 }
                        NumberField { Layout.preferredWidth: 52; value: scene.pathSeconds; fallback: 60; onCommitted: n => scene.pathSeconds = Math.max(0, n) }
                        Text { text: "s"; color: Theme.textDim; font.pixelSize: 11 }
                    }
                }
                // Environment volumes: what is blending at the camera, and why.
                Rectangle {
                    visible: scene.volumePanel && root.appState.runtime_open === true
                    anchors.right: parent.right; anchors.top: parent.top; anchors.margins: 8
                    width: 300; height: Math.min(parent.height - 16, volumeColumn.implicitHeight + 16); radius: 6
                    color: "#e0202124"; border.color: Theme.borderSoft; clip: true
                    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onWheel: wheel => wheel.accepted = true }
                    Flickable {
                        anchors.fill: parent; anchors.margins: 8
                        contentHeight: volumeColumn.implicitHeight; boundsBehavior: Flickable.StopAtBounds
                        ColumnLayout {
                            id: volumeColumn
                            width: parent.width; spacing: 4
                            RowLayout {
                                spacing: 2
                                BwCheckBox { text: "Bounds"; checked: scene.volumeBounds; onToggled: scene.volumeBounds = checked }
                                BwCheckBox { text: "Heat map"; checked: scene.volumeHeatmap; onToggled: scene.volumeHeatmap = checked }
                                BwCheckBox { text: "Freeze"; checked: root.volumeFreeze; onToggled: root.volumeFreeze = checked
                                    ToolTip.visible: hovered; ToolTip.delay: 500
                                    ToolTip.text: "Holds the blend where it is: the camera can move but the look stays, and each property's lerp shows below." }
                            }
                            Text { text: "Blending at the camera"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold }
                            Text { visible: root.volumeStatus.length === 0; text: "Only the project's own settings."; color: Theme.textDim; font.pixelSize: 11 }
                            Repeater {
                                model: root.volumeStatus
                                delegate: ColumnLayout {
                                    required property var modelData
                                    Layout.fillWidth: true; spacing: 1
                                    RowLayout {
                                        Layout.fillWidth: true
                                        Text { Layout.fillWidth: true; text: modelData.name; color: Theme.text; font.pixelSize: 12; elide: Text.ElideRight }
                                        Text { text: "priority " + modelData.priority; color: Theme.textDim; font.pixelSize: 11 }
                                        Text { text: Math.round(modelData.weight * 100) + "%"; color: Theme.accent; font.pixelSize: 12; Layout.preferredWidth: 36; horizontalAlignment: Text.AlignRight }
                                    }
                                    Rectangle { Layout.fillWidth: true; height: 3; radius: 1; color: Theme.border
                                        Rectangle { width: parent.width * modelData.weight; height: parent.height; radius: 1; color: Theme.accent } }
                                    Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 10
                                        text: modelData.overrides.length ? modelData.overrides.join(", ").replace(/_/g, " ") : "overrides nothing" }
                                }
                            }
                            Text { visible: root.volumeFreeze; text: "Frozen lerp"; color: Theme.text; font.pixelSize: 12; font.weight: Font.DemiBold; Layout.topMargin: 4 }
                            Text { visible: root.volumeFreeze && root.volumeTrace.length === 0; text: "No volume is changing anything."; color: Theme.textDim; font.pixelSize: 11 }
                            Repeater {
                                model: root.volumeFreeze ? root.volumeTrace : []
                                delegate: ColumnLayout {
                                    required property var modelData
                                    Layout.fillWidth: true; spacing: 1
                                    Text { text: modelData.property.replace(/_/g, " "); color: Theme.text; font.pixelSize: 11; font.weight: Font.DemiBold }
                                    Text { text: "project  " + modelData.base; color: Theme.textDim; font.pixelSize: 10; font.family: "monospace" }
                                    Repeater {
                                        model: modelData.steps
                                        Text {
                                            required property var modelData
                                            Layout.fillWidth: true; elide: Text.ElideRight
                                            text: "+ " + modelData.volume + " " + Math.round(modelData.weight * 100) + "% of " + modelData.target + " = " + modelData.after
                                            color: Theme.textDim; font.pixelSize: 10; font.family: "monospace"
                                        }
                                    }
                                    Text { text: "result   " + modelData.result; color: Theme.accent; font.pixelSize: 10; font.family: "monospace" }
                                }
                            }
                        }
                    }
                }
                Keys.onPressed: event => {
                    // Escape always frees the pointer, and the game still hears it.
                    if (event.key === Qt.Key_Escape && gameView.pointerLocked) root.pointerSuspended = true;
                    const code = root.keyCode(event);
                    // The scene view's own keys, by where they sit; flying uses the same ones.
                    if (root.editing && !root.looking && !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier)) && !event.isAutoRepeat) {
                        const tools = { KeyW: "move", KeyE: "rotate", KeyR: "scale", KeyB: "brush", KeyT: "tiles" };
                        if (tools[code] && (code !== "KeyB" || root.is3d)) root.setTool(tools[code]);
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
