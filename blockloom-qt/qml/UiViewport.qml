import QtQuick
import QtQuick.Controls
import com.blockworked.Blockstitch 1.0
import com.blockworked.Blockloom 1.0

// The design surface: the runtime's frame, rulers and guides, and the overlay
// that draws selection, handles, snap lines and the marquee. It computes no
// layout of its own; geometry comes from the runtime, matched by revision.
Item {
    id: viewport
    required property var designer
    readonly property real stageWidth: stage.width
    readonly property real stageHeight: stage.height
    readonly property Item canvasItem: canvas
    function repaint() { overlay.requestPaint(); }
    // Editor-only drawing colours (see docs/ui-tokens.md, "Interface designer").
    readonly property color stageColor: Theme.canvas
    readonly property color rulerColor: Theme.panel
    readonly property color rulerInk: Theme.textDim
    readonly property color selectionColor: "#70baff"
    readonly property color hoverColor: "#b9dfff"
    readonly property color alsoSelectedColor: "#8fd0a0"
    readonly property color safeAreaColor: "#ffb454"
    readonly property color referenceColor: "#c792ea"
    readonly property color guideColor: "#4dd0e1"
    readonly property color snapColor: "#ffcc70"
    readonly property color lightCanvasColor: "#d9dfe8"
    readonly property color darkCanvasColor: "#252d3a"
    readonly property string marqueeFill: "rgba(112,186,255,0.15)"

    Rectangle {
        id: stage; anchors.fill: parent; color: stageColor; clip: true
        // Ctrl+wheel zooms at the pointer; the wheel pans (Shift for sideways); middle-drag pans.
        WheelHandler {
            objectName: "interfaceWheel"
            acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
            onWheel: event => {
                const dy = event.angleDelta.y, dx = event.angleDelta.x;
                if (event.modifiers & Qt.ControlModifier) designer.zoomAt(point.position.x, point.position.y, Math.pow(1.0015, dy));
                else if (event.modifiers & Qt.ShiftModifier) designer.panX += (dy || dx) / 2;
                else { designer.panX += dx / 2; designer.panY += dy / 2; }
            }
        }
        DragHandler {
            objectName: "interfacePan"
            acceptedButtons: Qt.MiddleButton
            target: null
            property real lastX: 0
            property real lastY: 0
            onActiveChanged: { lastX = 0; lastY = 0; }
            onTranslationChanged: { designer.panX += translation.x - lastX; designer.panY += translation.y - lastY; lastX = translation.x; lastY = translation.y; }
        }
        // Rulers: drag out of one for a guide, and drag a guide back onto it to remove it.
        Repeater {
            model: designer.showRulers ? [0, 1] : []
            delegate: Item {
                id: ruler
                required property int modelData
                // The top ruler measures x and pulls out horizontal guides; the left one the reverse.
                readonly property int measures: modelData
                readonly property int makes: 1 - modelData
                objectName: measures === 0 ? "interfaceRulerTop" : "interfaceRulerLeft"
                z: 5
                width: measures === 0 ? stage.width : 18
                height: measures === 0 ? 18 : stage.height
                property int dragging: -1
                Rectangle { anchors.fill: parent; color: rulerColor; opacity: 0.92 }
                Repeater {
                    // A tick every 50 preview pixels, labelled every 100.
                    model: Math.ceil((ruler.measures === 0 ? designer.previewWidth : designer.previewHeight) / 50) + 1
                    delegate: Item {
                        required property int index
                        readonly property real at: designer.canvasToStage(ruler.measures, index * 50)
                        x: ruler.measures === 0 ? at : 0
                        y: ruler.measures === 0 ? 0 : at
                        Rectangle {
                            width: ruler.measures === 0 ? 1 : (index % 2 ? 6 : 12)
                            height: ruler.measures === 0 ? (index % 2 ? 6 : 12) : 1
                            color: rulerInk
                        }
                        Label {
                            visible: index % 2 === 0
                            x: ruler.measures === 0 ? 3 : 1; y: ruler.measures === 0 ? 0 : 3
                            text: index * 50; font.pixelSize: 8; color: rulerInk
                        }
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    function canvasPoint(mouse) {
                        const p = mapToItem(stage, mouse.x, mouse.y);
                        return Math.round(designer.stageToCanvas(ruler.makes, ruler.makes === 0 ? p.x : p.y));
                    }
                    onPressed: mouse => { ruler.dragging = designer.addGuide(ruler.makes, canvasPoint(mouse)); }
                    onPositionChanged: mouse => { if (ruler.dragging >= 0) designer.moveGuide(ruler.dragging, canvasPoint(mouse)); }
                    onReleased: mouse => {
                        if (ruler.dragging < 0) return;
                        const p = mapToItem(stage, mouse.x, mouse.y);
                        // Released on a ruler (or off the stage) takes the guide away again.
                        if ((ruler.makes === 0 ? p.x : p.y) <= 18) designer.removeGuide(ruler.dragging);
                        ruler.dragging = -1;
                    }
                    onCanceled: { if (ruler.dragging >= 0) designer.removeGuide(ruler.dragging); ruler.dragging = -1; }
                }
            }
        }
        Rectangle {
            id: canvas; objectName: "interfaceCanvas"; anchors.centerIn: parent
            anchors.horizontalCenterOffset: designer.panX; anchors.verticalCenterOffset: designer.panY
            width: designer.previewWidth; height: designer.previewHeight
            scale: Math.max(0.05,designer.zoom); color: designer.document.theme === "Light" ? lightCanvasColor : darkCanvasColor
            Loader {
                id: nativeView
                anchors.fill: parent
                active: designer.designing && designer.embedded
                sourceComponent: Component {
                    GameView {
                        resolution: Qt.size(designer.previewWidth, designer.previewHeight)
                        onInterfaceLayoutChanged: designer.receiveLayout(interfaceLayout)
                    }
                }
            }
            Image {
                id: streamedView
                anchors.fill: parent
                visible: !designer.embedded && designer.designing
                source: visible ? designer.app.previewFrame : ""
                cache: false; asynchronous: false; fillMode: Image.PreserveAspectFit
                onSourceChanged: designer.receiveLayout(designer.app.previewLayout)
            }
            Canvas {
                id: overlay
                objectName: "interfaceSelection"
                anchors.fill: parent
                visible: designer.layoutReady || !!designer.gesture
                onPaint: {
                    const ctx = getContext("2d");
                    ctx.clearRect(0, 0, width, height);
                    function outline(id, color) {
                        const widget = designer.boundsById[id];
                        const polygon = designer.geo.polygon(widget);
                        if (!polygon.length) return;
                        ctx.beginPath(); ctx.moveTo(polygon[0].x, polygon[0].y);
                        for (let i = 1; i < polygon.length; ++i) ctx.lineTo(polygon[i].x, polygon[i].y);
                        ctx.closePath(); ctx.strokeStyle = color;
                        ctx.lineWidth = 2/Math.max(0.05, designer.zoom); ctx.stroke();
                    }
                    if (designer.hoveredId !== designer.selectedId) outline(designer.hoveredId, hoverColor);
                    const inset = {x: designer.safe[0], y: designer.safe[1], w: designer.previewWidth-designer.safe[0]-designer.safe[2], h: designer.previewHeight-designer.safe[1]-designer.safe[3]};
                    if (designer.showSafeArea) {
                        ctx.save(); ctx.setLineDash([6/Math.max(0.05,designer.zoom), 4/Math.max(0.05,designer.zoom)]);
                        ctx.strokeStyle = safeAreaColor; ctx.lineWidth = 1/Math.max(0.05,designer.zoom);
                        ctx.strokeRect(inset.x, inset.y, inset.w, inset.h); ctx.restore();
                    }
                    if (designer.showReference) {
                        const ref = designer.document.reference_size || [960,720];
                        ctx.save(); ctx.setLineDash([2/Math.max(0.05,designer.zoom), 3/Math.max(0.05,designer.zoom)]);
                        ctx.strokeStyle = referenceColor; ctx.lineWidth = 1/Math.max(0.05,designer.zoom);
                        ctx.strokeRect(inset.x, inset.y, ref[0]*designer.designScale, ref[1]*designer.designScale); ctx.restore();
                    }
                    ctx.save(); ctx.strokeStyle = guideColor; ctx.lineWidth = 1/Math.max(0.05,designer.zoom);
                    designer.guides.forEach(g => {
                        ctx.beginPath();
                        if (g.axis === 0) { ctx.moveTo(g.pos, 0); ctx.lineTo(g.pos, height); } else { ctx.moveTo(0, g.pos); ctx.lineTo(width, g.pos); }
                        ctx.stroke();
                    });
                    ctx.restore();
                    designer.selectionIds.slice(1).forEach(id => outline(id, alsoSelectedColor));
                    outline(designer.selectedId, selectionColor);
                    const m = designer.marquee;
                    if (m) {
                        ctx.fillStyle = marqueeFill; ctx.strokeStyle = selectionColor; ctx.lineWidth = 1/Math.max(0.05,designer.zoom);
                        ctx.fillRect(Math.min(m.x0,m.x1), Math.min(m.y0,m.y1), Math.abs(m.x1-m.x0), Math.abs(m.y1-m.y0));
                        ctx.strokeRect(Math.min(m.x0,m.x1), Math.min(m.y0,m.y1), Math.abs(m.x1-m.x0), Math.abs(m.y1-m.y0));
                    }
                    ctx.strokeStyle = snapColor;
                    ctx.lineWidth = 1/Math.max(0.05,designer.zoom);
                    designer.snapLines.forEach(line => { ctx.beginPath(); ctx.moveTo(line.a.x,line.a.y); ctx.lineTo(line.b.x,line.b.y); ctx.stroke(); });
                }
            }
            MouseArea {
                objectName: "interfacePicking"
                anchors.fill: parent
                enabled: designer.layoutReady || !!designer.gesture
                property real pressX: 0
                property real pressY: 0
                hoverEnabled: true
                onPositionChanged: mouse => {
                    if (pressed && marqueeStart) { designer.marquee = {x0: marqueeStart.x, y0: marqueeStart.y, x1: mouse.x, y1: mouse.y}; return; }
                    if (pressed) {
                        if (!designer.gesture && Math.hypot(mouse.x-pressX, mouse.y-pressY) > 3/Math.max(0.05,designer.zoom)) designer.startEdit("Move", pressX, pressY);
                        designer.dragEdit(mouse.x, mouse.y, mouse.modifiers);
                    }
                    else designer.hoveredId = designer.geo.pick(designer.pickable, mouse.x, mouse.y);
                }
                onExited: designer.hoveredId = ""
                onPressed: mouse => {
                    designer.forceActiveFocus();
                    const hit = designer.geo.pick(designer.pickable, mouse.x, mouse.y);
                    pressX = mouse.x; pressY = mouse.y;
                    if (mouse.modifiers & Qt.ShiftModifier) { designer.toggleSelected(hit); marqueeStart = null; }
                    else if (!hit) { designer.selectOnly(""); marqueeStart = {x: mouse.x, y: mouse.y}; }
                    else { if (hit !== designer.selectedId) designer.selectOnly(hit); marqueeStart = null; }
                }
                property var marqueeStart: null
                onReleased: {
                    if (marqueeStart && designer.marquee) designer.finishMarquee(designer.marquee);
                    marqueeStart = null; designer.marquee = null;
                    designer.finishEdit();
                }
                onCanceled: { marqueeStart = null; designer.marquee = null; designer.cancelEdit(); }
            }
            Repeater {
                model: designer.resizeHandles
                delegate: Rectangle {
                    required property var modelData
                    readonly property var point: designer.handlePoint(modelData)
                    objectName: modelData.name === "BottomRight" ? "interfaceResizeHandle" : "interfaceResize"+modelData.name
                    visible: (designer.gesture && designer.gesture.kind === "Resize") || (designer.layoutReady && !!designer.selectedBounds && designer.selectedBounds.visible && designer.editable(designer.widget))
                    x: point.x-width/2; y: point.y-height/2
                    width: 10/Math.max(0.05,designer.zoom); height: width
                    color: selectionColor
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: designer.resizeCursor(modelData)
                        onPressed: mouse => { const p = canvas.mapFromItem(parent,mouse.x,mouse.y); designer.startEdit("Resize",p.x,p.y,modelData); }
                        onPositionChanged: mouse => { if (pressed) { const p = canvas.mapFromItem(parent,mouse.x,mouse.y); designer.dragEdit(p.x,p.y,mouse.modifiers); } }
                        onReleased: designer.finishEdit()
                        onCanceled: designer.cancelEdit()
                    }
                }
            }
            Rectangle {
                anchors.fill: parent
                visible: !designer.layoutReady && !designer.gesture
                color: Theme.panelRaised
                Text { anchors.centerIn: parent; color: Theme.text; text: designer.app.appState.running ? "Stop the game to edit the interface" : designer.error ? "Preview unavailable" : "Rendering interface..." }
            }
        }
    }
}
