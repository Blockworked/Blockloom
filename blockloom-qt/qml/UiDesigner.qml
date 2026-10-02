import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockloom 1.0
import com.blockworked.Blockstitch 1.0

Item {
    id: root
    InterfaceGeometry { id: geometry }
    required property var app
    property var document: ({widgets: [], styles: {}, prefabs: {}, reference_size: [960,720], safe_area: [0,0,0,0], theme: "Dark", scale: "ConstantPixel"})
    property string selectedId: ""
    property string screenId: ""
    readonly property var screens: document.widgets.filter(w => !w.element.parent).map(w => w.element.id)
    readonly property var screenWidgets: document.widgets.filter(w => inScreen(w.element.id))
    function inScreen(id) {
        if (!screenId) return true;
        let current = document.widgets.find(w => w.element.id === id);
        const visited = [];
        while (current && visited.indexOf(current.element.id) < 0) {
            if (current.element.id === screenId) return true;
            visited.push(current.element.id);
            current = document.widgets.find(w => w.element.id === current.element.parent);
        }
        return false;
    }
    onScreenIdChanged: {
        cancelEdit();
        ++revision;
        frameLayout = null;
        hoveredId = "";
        if (!inScreen(selectedId)) selectedId = "";
        if (designing) previewDelay.restart();
    }
    readonly property int selected: document.widgets.findIndex(w => w.element.id === selectedId)
    property var gesture: null
    readonly property var selectedBounds: bounds.find(w => w.id === selectedId) || null
    readonly property var resizePoint: selectedBounds ? geometry.point(selectedBounds.transform, selectedBounds.size[0]/2, selectedBounds.size[1]/2) : ({x: 0, y: 0})
    function editable(w) {
        if (!w || w.world_actor) return false;
        const parent = document.widgets.find(p => p.element.id === w.element.parent);
        return !w.element.parent || (w.layout && w.layout.absolute) || (parent && parent.element.kind === "Canvas");
    }
    function startEdit(kind, x, y) {
        if (gesture || !widget || ((kind === "Move" || kind === "Resize") && !editable(widget))) return false;
        const bound = selectedBounds;
        if (x !== null && (!layoutReady || !bound)) return false;
        const parent = bounds.find(w => w.id === widget.element.parent);
        const g = {kind: kind, id: selectedId, original: copy(document), token: null,
            offset: (widget.element.offset || [0,0]).slice(), start: {x: x, y: y},
            inverse: bound ? geometry.inverse(bound.transform) : null,
            parentInverse: parent ? geometry.inverse(parent.transform) : null,
            size: bound ? [bound.size[0], bound.size[1]] : (widget.element.size || [0,0]).slice(),
            handle: {x: resizePoint.x, y: resizePoint.y}, scale: designScale, committing: false, edit: null, sent: null, busy: false, released: false, canceled: false};
        gesture = g;
        forceActiveFocus();
        const backend = app;
        app.invoke("begin_interface_edit", {revision: savedRevision}, function(token) {
            g.token = token;
            if (g.canceled) backend.invoke("cancel_interface_edit", {token: token});
            else root.flushEdit(g);
        }, function(e) { root.failEdit(g, e); });
        return true;
    }
    function failEdit(g, e) {
        if (g.canceled) return;
        if (g.token) app.invoke("cancel_interface_edit", {token: g.token});
        if (gesture !== g) return;
        g.canceled = true; gesture = null; refresh(); error = String(e);
        if (designing) previewDelay.restart();
    }
    function flushEdit(g) {
        if (g.canceled || !g.token || g.busy) return;
        if (g.edit && g.edit !== g.sent) {
            const edit = g.edit; g.sent = edit; g.busy = true;
            app.invoke("update_interface_edit", {token: g.token, edit: edit}, function(next) {
                g.busy = false;
                if (g.canceled || root.gesture !== g) return;
                root.document = next;
                root.flushEdit(g);
            }, function(e) { root.failEdit(g, e); });
        } else if (g.released) {
            g.busy = true; g.committing = true;
            app.invoke("commit_interface_edit", {token: g.token}, function() {
                if (g.canceled || root.gesture !== g) return;
                root.gesture = null; root.refresh(); root.error = "";
                if (root.designing) previewDelay.restart();
            }, function(e) { root.failEdit(g, e); });
        }
    }
    function dragEdit(x, y) {
        const g = gesture;
        if (!g || g.released || !g.inverse) return;
        let dx = x-g.start.x, dy = y-g.start.y;
        let offset = g.offset.slice();
        if (g.kind === "Move") {
            if (g.parentInverse) {
                const a = geometry.point(g.parentInverse, g.start.x, g.start.y), b = geometry.point(g.parentInverse, x, y);
                dx = b.x-a.x; dy = b.y-a.y;
            } else { dx /= g.scale; dy /= g.scale; }
            offset = [g.offset[0]+dx, g.offset[1]+dy];
            g.edit = {kind: "Move", id: g.id, offset: offset};
        } else {
            const a = geometry.point(g.inverse, g.start.x, g.start.y), b = geometry.point(g.inverse, x, y);
            const size = [Math.max(1,g.size[0]+b.x-a.x), Math.max(1,g.size[1]+b.y-a.y)];
            const w = g.original.widgets.find(w => w.element.id === g.id);
            const fractions = {TopLeft:[0,0], Top:[0.5,0], TopRight:[1,0], Left:[0,0.5], Center:[0.5,0.5], Right:[1,0.5], BottomLeft:[0,1], Bottom:[0.5,1], BottomRight:[1,1]};
            const f = w.layout && w.layout.absolute ? [0,0] : fractions[w.element.anchor || "Center"];
            offset = [g.offset[0]+f[0]*(size[0]-g.size[0]), g.offset[1]+f[1]*(size[1]-g.size[1])];
            g.edit = {kind: "Resize", id: g.id, size: size, offset: offset};
        }
        flushEdit(g);
    }
    function finishEdit() { if (gesture) { gesture.released = true; flushEdit(gesture); } }
    function cancelEdit() {
        const g = gesture;
        if (!g || g.committing) return;
        g.canceled = true; gesture = null;
        if (g.token) app.invoke("cancel_interface_edit", {token: g.token});
        refresh(); if (designing) previewDelay.restart();
    }
    function editDimension(index, value) {
        if (!widget || !Number.isFinite(value) || !startEdit(index < 2 ? "Move" : "Resize", null, null)) return;
        const g = gesture, offset = g.offset.slice(), size = g.size.slice();
        if (index < 2) offset[index] = value; else size[index-2] = value;
        g.edit = index < 2 ? {kind:"Move", id:g.id, offset:offset} : {kind:"Resize", id:g.id, size:size, offset:offset};
        finishEdit();
    }
    function submitEdit(edit) {
        if (!startEdit(edit.kind, null, null)) return;
        gesture.edit = edit;
        finishEdit();
    }
    function propertyEdit(path, value) {
        if (widget) submitEdit({kind: "SetProperty", id: selectedId, property: {path: path, value: value}});
    }
    function descendant(id, ancestor) {
        const visited = [];
        let w = document.widgets.find(w => w.element.id === id);
        while (w && visited.indexOf(w.element.id) < 0) {
            if (w.element.id === ancestor) return true;
            visited.push(w.element.id);
            w = document.widgets.find(p => p.element.id === w.element.parent);
        }
        return false;
    }
    function reparent(parentId) {
        if (!widget || (widget.element.parent || "") === parentId) return;
        const target = document.widgets.find(w => w.element.id === parentId);
        let placement = {mode: "Flow"};
        if (!target || target.element.kind === "Canvas") {
            const bound = selectedBounds, parent = bounds.find(w => w.id === parentId);
            if (!layoutReady || !bound || (target && !parent)) { error = "Wait for matching geometry before reparenting."; return; }
            if (!bound.visible || (parent && !parent.visible)) { error = "Show all screens before reparenting into a hidden tree."; return; }
            const transform = parent ? geometry.inverse(parent.transform) : [1/designScale,0,0,1/designScale,-safe[0]/designScale,-safe[1]/designScale];
            if (!transform) { error = "The parent transform cannot be inverted."; return; }
            const corner = geometry.point(bound.transform,-bound.size[0]/2,-bound.size[1]/2);
            const at = geometry.point(transform,corner.x,corner.y);
            const x = geometry.point(bound.transform,bound.size[0]/2,-bound.size[1]/2);
            const y = geometry.point(bound.transform,-bound.size[0]/2,bound.size[1]/2);
            const right = geometry.point(transform,x.x,x.y), bottom = geometry.point(transform,y.x,y.y);
            if (Math.abs(right.y-at.y)>0.01 || Math.abs(bottom.x-at.x)>0.01 || right.x<=at.x || bottom.y<=at.y) {
                error = "This transform cannot preserve placement in the new parent."; return;
            }
            placement = {mode: "Free", offset: [at.x+(parent ? parent.size[0]/2 : 0),at.y+(parent ? parent.size[1]/2 : 0)], size: [right.x-at.x,bottom.y-at.y]};
        }
        submitEdit({kind: "Reparent", id: selectedId, parent: parentId, placement: placement});
    }
    Keys.onEscapePressed: cancelEdit()
    property string error: ""
    readonly property bool designing: visible && app.appState.running !== true
    readonly property bool embedded: app.appState.runtime_embedded === true
    property double revision: Date.now()
    property double generation: Date.now()
    property var frameLayout: null
    property string hoveredId: ""
    property bool enabledPreview: false
    readonly property bool layoutReady: designing && frameLayout !== null
        && frameLayout.revision === revision && frameLayout.generation === generation
        && frameLayout.viewport[0] === previewWidth && frameLayout.viewport[1] === previewHeight
    readonly property var bounds: layoutReady ? frameLayout.widgets : []
    function receiveLayout(text) {
        try { frameLayout = text ? JSON.parse(text) : null; } catch(e) { frameLayout = null; }
    }
    function requestPreview() {
        if (!designing || !app.appState.project) return;
        ++revision;
        frameLayout = null;
        app.invoke("preview_interface", {design: {revision: revision, generation: generation,
            viewport: [previewWidth, previewHeight], screen: screenId || null, document: copy(document)}},
            function() { root.error = ""; }, function(e) { root.error = String(e); });
    }
    function updateSession() {
        if (!designing) cancelEdit();
        frameLayout = null;
        if (designing) {
            if (!embedded && !app.appState.preview_enabled) {
                enabledPreview = true;
                app.invoke("set_preview_enabled", {enabled: true});
            }
            previewDelay.restart();
        } else {
            previewDelay.stop();
            if (app.appState.project && app.appState.running !== true) app.invoke("preview_interface", {});
            if (enabledPreview) { enabledPreview = false; app.invoke("set_preview_enabled", {enabled: false}); }
        }
    }
    onDesigningChanged: updateSession()
    onDocumentChanged: {
        ++revision;
        frameLayout = null;
        if (screenId && !document.widgets.some(w => w.element.id === screenId && !w.element.parent)) screenId = "";
        if (!inScreen(selectedId)) selectedId = "";
        if (designing) {
            if (gesture) { if (!previewDelay.running) previewDelay.start(); }
            else previewDelay.restart();
        }
    }
    onPreviewWidthChanged: { cancelEdit(); ++generation; frameLayout = null; if (designing) previewDelay.restart(); }
    onPreviewHeightChanged: { cancelEdit(); ++generation; frameLayout = null; if (designing) previewDelay.restart(); }
    onSelectedIdChanged: overlay.requestPaint()
    onHoveredIdChanged: overlay.requestPaint()
    onFrameLayoutChanged: overlay.requestPaint()
    onLayoutReadyChanged: overlay.requestPaint()
    Timer { id: previewDelay; interval: 80; onTriggered: root.requestPreview() }
    readonly property double savedRevision: app.appState.sync ? app.appState.sync.revision : 0
    onSavedRevisionChanged: { if (gesture && !gesture.committing) cancelEdit(); frameLayout = null; if (designing) previewDelay.restart(); }
    readonly property var widget: selected >= 0 && selected < document.widgets.length ? document.widgets[selected] : null
    readonly property var kinds: ["Panel","Label","Button","Image","Input","Slider","Toggle","List","VerticalBox","HorizontalBox","Grid","Canvas","WrapBox","SizeBox","Spacer","Progress","RadialProgress","ListView","Tabs","Select","Scrollbar","RichText","Tooltip"]
    property int previewWidth: 960
    property int previewHeight: 720
    readonly property var safe: document.safe_area || [0,0,0,0]
    readonly property real designScale: document.scale === "ScaleWithSize" ? Math.max(0.01, Math.min((previewWidth-safe[0]-safe[2])/(document.reference_size || [960,720])[0], (previewHeight-safe[1]-safe[3])/(document.reference_size || [960,720])[1])) : 1
    property real zoom: Math.min((stage.width - 32) / previewWidth, (stage.height - 32) / previewHeight)
    function copy(v) { return JSON.parse(JSON.stringify(v)); }
    function refresh() {
        if (gesture) return;
        const saved = app.appState.project ? app.appState.project.world.interface : null;
        if (saved && JSON.stringify(saved) !== JSON.stringify(document)) document = copy(saved);
        if (selected < 0) selectedId = "";
    }
    readonly property string projectId: app.appState.project ? app.appState.project.id || "" : ""
    onProjectIdChanged: cancelEdit()
    Component.onDestruction: {
        const g = gesture;
        if (g) {
            g.canceled = true;
            if (g.token && !g.committing) app.invoke("cancel_interface_edit", {token: g.token});
        }
    }
    Component.onCompleted: { refresh(); if (designing) updateSession(); }
    Connections {
        target: root.app
        function onAppStateChanged() { root.refresh(); }
        function onPreviewLayoutChanged() { if (root.designing && !root.embedded) Qt.callLater(() => root.receiveLayout(root.app.previewLayout)); }
    }
    function save(next) {
        if (gesture) return;
        document = next;
        app.invoke("set_interface", {document: next}, function() { root.error = ""; }, function(e) { root.error = String(e); root.refresh(); });
    }
    function change(field, value) {
        if (!widget) return;
        propertyEdit("element." + field, value);
    }
    function extra(field, value) {
        if (!widget) return;
        const next = copy(document); next.widgets[selected][field] = value; save(next);
    }
    function paint(field, value) {
        if (!widget) return;
        const style = copy(widget.style || {});
        if (!style[styleState.currentText]) style[styleState.currentText] = {};
        style[styleState.currentText][field] = value;
        extra("style", style);
    }
    function add(kind, x, y) {
        const next = copy(document);
        let id = kind.toLowerCase(), n = 1;
        while (next.widgets.some(w => w.element.id === id + n)) ++n;
        next.widgets.push({element: {id: id+n, kind: kind, content: ["Label","Button","RichText","Toggle"].indexOf(kind) >= 0 ? kind : "", anchor: "TopLeft", offset: [Math.round(x),Math.round(y)], size: [180, kind === "Panel" || kind === "ListView" ? 180 : 40], parent: "", modal: false, range: [0,100], value: {Number: 0}}, style: {}, bindings: [], items: []});
        // Values use the core's tagged representation; default values can be omitted.
        delete next.widgets[next.widgets.length-1].element.value;
        screenId = "";
        selectedId = id+n; save(next);
    }
    function removeSelected() {
        if (!widget) return;
        const next = copy(document); let gone = [widget.element.id];
        for (let i=0;i<gone.length;++i) next.widgets.forEach(w => { if(w.element.parent === gone[i] && gone.indexOf(w.element.id)<0) gone.push(w.element.id); });
        next.widgets = next.widgets.filter(w => gone.indexOf(w.element.id)<0); selectedId = ""; save(next);
    }
    RowLayout {
        anchors.fill: parent; spacing: 0
        ColumnLayout {
            Layout.preferredWidth: 170; Layout.fillHeight: true; spacing: 6
            Label { text: "Widgets"; font.bold: true; padding: 8 }
            ScrollView {
                Layout.fillWidth: true; Layout.preferredHeight: 210
                Column {
                    width: parent.width
                    Repeater {
                        model: root.kinds
                        delegate: Button {
                            required property string modelData
                            width: 158; height: 29; text: modelData
                            onClicked: root.add(modelData,20,20)
                            DragHandler {
                                target: null
                                onActiveChanged: if (!active) {
                                    const p=canvas.mapFromItem(parent,centroid.position.x,centroid.position.y);
                                    if (p.x>=0 && p.y>=0 && p.x<canvas.width && p.y<canvas.height) root.add(parent.modelData,(p.x-root.safe[0])/root.designScale,(p.y-root.safe[1])/root.designScale);
                                }
                            }
                        }
                    }
                }
            }
            Label { text: "Hierarchy"; font.bold: true; padding: 8 }
            ListView {
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                model: root.screenWidgets
                delegate: ItemDelegate {
                    required property var modelData
                    required property int index
                    width: ListView.view.width; height: 30
                    text: (modelData.element.parent ? "    " : "")+modelData.element.id
                    highlighted: root.selectedId === modelData.element.id
                    onClicked: root.selectedId=modelData.element.id
                }
            }
            Button { text: "Delete widget"; enabled: !!root.widget; onClicked: root.removeSelected() }
        }
        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true
            RowLayout {
                Label { text: "Screen" }
                ComboBox {
                    objectName: "interfaceScreen"
                    model: ["All screens"].concat(root.screens)
                    currentIndex: root.screenId ? root.screens.indexOf(root.screenId) + 1 : 0
                    onActivated: root.screenId = currentIndex > 0 ? root.screens[currentIndex - 1] : ""
                }
                ComboBox { model: ["960 × 720","1280 × 720","1920 × 1080","720 × 1280"]; onActivated: { const sizes=[[960,720],[1280,720],[1920,1080],[720,1280]]; root.previewWidth=sizes[currentIndex][0]; root.previewHeight=sizes[currentIndex][1]; } }
                ComboBox { model: ["Dark","Light","HighContrast"]; currentIndex: model.indexOf(root.document.theme || "Dark"); onActivated: { const d=root.copy(root.document); d.theme=currentText; root.save(d); } }
                ComboBox { model: ["ConstantPixel","ScaleWithSize"]; currentIndex: model.indexOf(root.document.scale || "ConstantPixel"); onActivated: { const d=root.copy(root.document); d.scale=currentText; root.save(d); } }
                Item { Layout.fillWidth: true }
            }
            Rectangle {
                id: stage; Layout.fillWidth: true; Layout.fillHeight: true; color: "#171a21"; clip: true
                Rectangle {
                    id: canvas; objectName: "interfaceCanvas"; anchors.centerIn: parent
                    width: root.previewWidth; height: root.previewHeight
                    scale: Math.max(0.05,root.zoom); color: root.document.theme === "Light" ? "#d9dfe8" : "#252d3a"
                    Loader {
                        id: nativeView
                        anchors.fill: parent
                        active: root.designing && root.embedded
                        sourceComponent: Component {
                            GameView {
                                resolution: Qt.size(root.previewWidth, root.previewHeight)
                                onInterfaceLayoutChanged: root.receiveLayout(interfaceLayout)
                            }
                        }
                    }
                    Image {
                        id: streamedView
                        anchors.fill: parent
                        visible: !root.embedded && root.designing
                        source: visible ? root.app.previewFrame : ""
                        cache: false; asynchronous: false; fillMode: Image.PreserveAspectFit
                        onSourceChanged: root.receiveLayout(root.app.previewLayout)
                    }
                    Canvas {
                        id: overlay
                        objectName: "interfaceSelection"
                        anchors.fill: parent
                        visible: root.layoutReady
                        onPaint: {
                            const ctx = getContext("2d");
                            ctx.clearRect(0, 0, width, height);
                            function outline(id, color) {
                                const widget = root.bounds.find(w => w.id === id);
                                const polygon = geometry.polygon(widget);
                                if (!polygon.length) return;
                                ctx.beginPath(); ctx.moveTo(polygon[0].x, polygon[0].y);
                                for (let i = 1; i < polygon.length; ++i) ctx.lineTo(polygon[i].x, polygon[i].y);
                                ctx.closePath(); ctx.strokeStyle = color;
                                ctx.lineWidth = 2/Math.max(0.05, root.zoom); ctx.stroke();
                            }
                            if (root.hoveredId !== root.selectedId) outline(root.hoveredId, "#b9dfff");
                            outline(root.selectedId, "#70baff");
                        }
                    }
                    MouseArea {
                        objectName: "interfacePicking"
                        anchors.fill: parent
                        enabled: root.layoutReady || !!root.gesture
                        property real pressX: 0
                        property real pressY: 0
                        hoverEnabled: true
                        onPositionChanged: mouse => {
                            if (pressed) {
                                if (!root.gesture && Math.hypot(mouse.x-pressX, mouse.y-pressY) > 3/Math.max(0.05,root.zoom)) root.startEdit("Move", pressX, pressY);
                                root.dragEdit(mouse.x, mouse.y);
                            }
                            else root.hoveredId = geometry.pick(root.bounds, mouse.x, mouse.y);
                        }
                        onExited: root.hoveredId = ""
                        onPressed: mouse => {
                            root.selectedId = geometry.pick(root.bounds, mouse.x, mouse.y);
                            pressX = mouse.x; pressY = mouse.y;
                        }
                        onReleased: root.finishEdit()
                        onCanceled: root.cancelEdit()
                    }
                    Rectangle {
                        objectName: "interfaceResizeHandle"
                        visible: (root.gesture && root.gesture.kind === "Resize") || (root.layoutReady && !!root.selectedBounds && root.editable(root.widget))
                        x: (root.gesture ? root.gesture.handle.x : root.resizePoint.x)-width/2; y: (root.gesture ? root.gesture.handle.y : root.resizePoint.y)-height/2
                        width: 10/Math.max(0.05,root.zoom); height: width
                        color: "#70baff"
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.SizeFDiagCursor
                            onPressed: mouse => { const p = canvas.mapFromItem(parent,mouse.x,mouse.y); root.startEdit("Resize",p.x,p.y); }
                            onPositionChanged: mouse => { if (pressed) { const p = canvas.mapFromItem(parent,mouse.x,mouse.y); root.dragEdit(p.x,p.y); } }
                            onReleased: root.finishEdit()
                            onCanceled: root.cancelEdit()
                        }
                    }
                    Rectangle {
                        anchors.fill: parent
                        visible: !root.layoutReady && !root.gesture
                        color: "#252d3a"
                        Text { anchors.centerIn: parent; color: "white"; text: root.app.appState.running ? "Stop the game to edit the interface" : root.error ? "Preview unavailable" : "Rendering interface…" }
                    }
                }
            }
            Label { text: root.error || "Drag to move; drag the corner to resize. Escape cancels. Parent layout controls flow widgets."; color: root.error ? "#ff8888" : Theme.textDim; wrapMode: Text.Wrap; Layout.fillWidth: true }
        }
        ScrollView {
            Layout.preferredWidth: 245; Layout.fillHeight: true
            ColumnLayout {
                width: 225; spacing: 8
                Label { text: "Widget inspector"; font.bold: true }
                Label { text: root.widget ? root.widget.element.id : "Select a widget" }
                ComboBox { Layout.fillWidth: true; model: root.kinds; currentIndex: root.widget ? root.kinds.indexOf(root.widget.element.kind) : -1; enabled: !!root.widget; onActivated: root.change("kind",currentText) }
                TextField { Layout.fillWidth: true; placeholderText: "Text or image asset"; text: root.widget ? root.widget.element.content || "" : ""; enabled: !!root.widget; onEditingFinished: root.change("content",text) }
                Label { text: "Parent (flow containers control placement)"; wrapMode: Text.Wrap; Layout.fillWidth: true }
                ComboBox { Layout.fillWidth: true; model: [""].concat(root.document.widgets.filter(w=>!w.world_actor && !root.descendant(w.element.id,root.selectedId)).map(w=>w.element.id)); currentIndex: root.widget ? model.indexOf(root.widget.element.parent || "") : 0; enabled: !!root.widget && !root.widget.world_actor && !root.gesture; onActivated: root.reparent(currentText) }
                Label { text: "Anchor (unused with absolute placement)"; wrapMode: Text.Wrap; Layout.fillWidth: true }
                ComboBox { Layout.fillWidth: true; model: ["TopLeft","Top","TopRight","Left","Center","Right","BottomLeft","Bottom","BottomRight"]; currentIndex: root.widget ? model.indexOf(root.widget.element.anchor || "Center") : 0; onActivated: root.change("anchor",currentText) }
                Repeater {
                    model: ["X","Y","Width","Height"]
                    delegate: RowLayout {
                        required property string modelData
                        required property int index
                        Label { text: modelData; Layout.preferredWidth: 55 }
                        TextField { Layout.fillWidth: true; text: root.widget ? (index<2 ? root.widget.element.offset || [0,0] : root.widget.element.size || [0,0])[index%2] : "0"; validator: DoubleValidator {}
                            enabled: root.editable(root.widget) && !root.gesture
                            onEditingFinished: root.editDimension(index, Number(text)) }
                    }
                }
                UiLayoutInspector {
                    Layout.fillWidth: true
                    enabled: !!root.widget && !root.gesture
                    layoutValue: root.widget ? root.widget.layout || null : null
                    onEdited: value => root.propertyEdit("layout", value)
                }
                CheckBox { text: "Modal"; checked: root.widget ? root.widget.element.modal === true : false; onToggled: root.change("modal",checked) }
                TextField { Layout.fillWidth: true; placeholderText: "Tooltip"; text: root.widget ? root.widget.tooltip || "" : ""; onEditingFinished: root.extra("tooltip",text) }
                TextField { Layout.fillWidth: true; placeholderText: "World actor id"; text: root.widget ? root.widget.world_actor || "" : ""; onEditingFinished: root.extra("world_actor",text) }
                TextField { Layout.fillWidth: true; placeholderText: "Scrollbar target widget id"; text: root.widget ? root.widget.scroll_target || "" : ""; onEditingFinished: root.extra("scroll_target",text) }
                TextField { Layout.fillWidth: true; placeholderText: "Tab page number (1-based)"; validator: IntValidator { bottom: 1 } text: root.widget ? root.widget.tab_index ?? "" : ""; onEditingFinished: root.extra("tab_index",text ? Number(text) : null) }
                Label { text: "Items (one per line)" }
                TextArea { Layout.fillWidth: true; Layout.preferredHeight: 70; text: root.widget ? (root.widget.items || []).join("\n") : ""; onActiveFocusChanged: if(!activeFocus && root.widget) root.extra("items",text ? text.split("\n") : []) }
                Label { text: "Style"; font.bold: true }
                ComboBox { id: styleState; Layout.fillWidth: true; model: ["normal","hover","pressed","disabled","focused"] }
                Repeater {
                    model: ["background","text_color","border_color","shadow"]
                    delegate: TextField {
                        required property string modelData
                        Layout.fillWidth: true
                        placeholderText: modelData.replace(/_/g," ") + " (#RRGGBB)"
                        text: root.widget ? ((root.widget.style || {})[styleState.currentText] || {})[modelData] || "" : ""
                        onEditingFinished: root.paint(modelData,text || null)
                    }
                }
                Repeater {
                    model: ["text_size","border_width","radius"]
                    delegate: RowLayout {
                        required property string modelData
                        Label { text: modelData.replace(/_/g," "); Layout.preferredWidth: 100 }
                        TextField {
                            Layout.fillWidth: true; validator: DoubleValidator { bottom: 0 }
                            text: root.widget ? ((root.widget.style || {})[styleState.currentText] || {})[modelData] ?? "" : ""
                            onEditingFinished: root.paint(modelData,text ? Number(text) : null)
                        }
                    }
                }
                RowLayout {
                    Label { text: "Transition (seconds)" }
                    TextField { Layout.fillWidth: true; validator: DoubleValidator { bottom: 0 }
                        text: root.widget ? root.widget.transition || 0 : 0
                        onEditingFinished: root.extra("transition",Number(text)) }
                }
                Label { text: "Variable binding"; font.bold: true }
                ComboBox { id: bindingProperty; Layout.fillWidth: true; model: ["Text","Value","Visible","SelectedIndex"] }
                TextField { id: bindingActor; Layout.fillWidth: true; placeholderText: "Actor id (blank for global)" }
                TextField { id: bindingName; Layout.fillWidth: true; placeholderText: "Variable name" }
                CheckBox { id: bindingWrite; text: "Write input back to variable" }
                Button { text: "Bind variable"; enabled: !!root.widget && !!bindingName.text; onClicked: {
                    const bindings=root.copy(root.widget.bindings || []).filter(b=>b.property!==bindingProperty.currentText);
                    bindings.push({property:bindingProperty.currentText,source:{Variable:{actor:bindingActor.text,name:bindingName.text}},two_way:bindingWrite.checked});
                    root.extra("bindings",bindings);
                } }
                Label { text: "Advanced properties"; font.bold: true }
                Repeater {
                    model: ["layout","style","bindings"]
                    delegate: ColumnLayout {
                        required property string modelData
                        Layout.fillWidth: true
                        Label { text: modelData + " (JSON)" }
                        TextArea { Layout.fillWidth: true; Layout.preferredHeight: 100; wrapMode: TextEdit.Wrap; text: root.widget ? JSON.stringify(root.widget[modelData] || (modelData === "bindings" ? [] : {}),null,2) : "";
                            onActiveFocusChanged: if(!activeFocus && root.widget) { try { root.extra(modelData,JSON.parse(text)); } catch(e) { root.error=String(e); } } }
                    }
                }
                Label { text: "Interface assets"; font.bold: true }
                TextField { id: assetName; Layout.fillWidth: true; placeholderText: "menu.json" }
                Button { text: "Save as asset"; enabled: !!assetName.text; onClicked: root.app.invoke("save_interface_asset",{name:assetName.text},function(){root.error="";},function(e){root.error=String(e);}) }
                TextField { id: assetPath; Layout.fillWidth: true; placeholderText: "assets/ui/menu.json" }
                Button { text: "Load interface asset"; enabled: !!assetPath.text; onClicked: root.app.invoke("load_interface_asset",{path:assetPath.text},function(){root.error="";},function(e){root.error=String(e);}) }
                Label { text: "Reusable menus"; font.bold: true }
                TextField { id: prefabName; Layout.fillWidth: true; placeholderText: "Prefab name" }
                Button { text: "Save interface as prefab"; enabled: !!prefabName.text; onClicked: { const d=root.copy(root.document); if(!d.prefabs)d.prefabs={}; d.prefabs[prefabName.text]=root.copy(d.widgets); root.save(d); } }
                ComboBox { id: prefab; Layout.fillWidth: true; model: Object.keys(root.document.prefabs || {}) }
                Button { text: "Insert prefab"; enabled: prefab.currentIndex>=0; onClicked: {
                    const d=root.copy(root.document), prefix="copy"+Date.now()+"_";
                    const widgets=root.copy(d.prefabs[prefab.currentText]); const ids=widgets.map(w=>w.element.id); widgets.forEach(w=>{if(ids.indexOf(w.scroll_target)>=0)w.scroll_target=prefix+w.scroll_target;w.element.id=prefix+w.element.id;if(w.element.parent)w.element.parent=prefix+w.element.parent;});
                    d.widgets=d.widgets.concat(widgets);root.save(d);
                } }
            }
        }
    }
}
