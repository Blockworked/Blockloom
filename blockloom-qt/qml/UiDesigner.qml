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
    // Editor-only state: never saved and never sent to the runtime.
    property var lockedIds: ({})
    // Further selected widgets beside selectedId (Shift-click, Ctrl-click in the tree, marquee).
    property var extraIds: []
    readonly property var selectionIds: {
        if (!selectedId) return [];
        const have = {};
        document.widgets.forEach(w => { have[w.element.id] = true; });
        return [selectedId].concat(extraIds.filter(id => have[id] && id !== selectedId && inScreen(id)));
    }
    // A selection never lists a widget together with its ancestor: the ancestor's edit covers it.
    function topLevelSelection() {
        return selectionIds.filter(id => !selectionIds.some(other => other !== id && descendant(id, other)));
    }
    function toggleSelected(id) {
        if (!id) return;
        if (!selectedId) { selectedId = id; return; }
        if (id === selectedId) { selectedId = extraIds.length ? extraIds[0] : ""; extraIds = extraIds.slice(1); return; }
        extraIds = extraIds.indexOf(id) >= 0 ? extraIds.filter(x => x !== id) : extraIds.concat([id]);
    }
    function selectOnly(id) { extraIds = []; selectedId = id; }
    property var marquee: null
    function finishMarquee(m) {
        const x0 = Math.min(m.x0, m.x1), x1 = Math.max(m.x0, m.x1), y0 = Math.min(m.y0, m.y1), y1 = Math.max(m.y0, m.y1);
        const ids = pickable.filter(b => b.visible && editableId(b.id)).filter(b => {
            const c = geometry.point(b.transform, 0, 0);
            return c.x >= x0 && c.x <= x1 && c.y >= y0 && c.y <= y1;
        }).sort((a, b) => a.paint_order - b.paint_order).map(b => b.id);
        extraIds = ids.slice(1); selectedId = ids.length ? ids[0] : "";
    }
    function editableId(id) { return document.widgets.some(w => w.element.id === id && !w.world_actor); }
    property var hiddenIds: ({})
    property var collapsedIds: ({})
    function toggleHidden(id) {
        const next = Object.assign({}, hiddenIds); if (next[id]) delete next[id]; else next[id] = true;
        hiddenIds = next; cancelEdit(); ++revision; frameLayout = null; if (designing) previewDelay.restart();
    }
    // Only IDs that still exist: the runtime refuses a request naming a missing widget.
    readonly property var hiddenList: document.widgets.map(w => w.element.id).filter(id => !!hiddenIds[id])
    property string search: ""
    function isLocked(id) { return !!lockedIds[id]; }
    function toggleLock(id) { const next = Object.assign({}, lockedIds); if (next[id]) delete next[id]; else next[id] = true; lockedIds = next; if (gesture && gesture.id === id) cancelEdit(); }
    function toggleCollapsed(id) { const next = Object.assign({}, collapsedIds); if (next[id]) delete next[id]; else next[id] = true; collapsedIds = next; }
    readonly property var treeRows: {
        const widgets = screenWidgets, children = {}, byId = {};
        widgets.forEach(w => { byId[w.element.id] = w; });
        widgets.forEach(w => {
            const parent = byId[w.element.parent] ? w.element.parent : "";
            (children[parent] = children[parent] || []).push(w);
        });
        const needle = search.trim().toLowerCase();
        const matches = w => w.element.id.toLowerCase().indexOf(needle) >= 0 || (w.element.content || "").toLowerCase().indexOf(needle) >= 0;
        const keep = {};
        if (needle) {
            widgets.filter(matches).forEach(w => {
                let current = w;
                while (current && !keep[current.element.id]) { keep[current.element.id] = true; current = byId[current.element.parent]; }
            });
        }
        const rows = [];
        const visit = (parent, depth) => (children[parent] || []).forEach(w => {
            const id = w.element.id;
            if (needle && !keep[id]) return;
            const kids = (children[id] || []).length;
            rows.push({id: id, depth: depth, kids: kids, kind: w.element.kind, collapsed: !needle && !!collapsedIds[id]});
            if (needle || !collapsedIds[id]) visit(id, depth + 1);
        });
        visit("", 0);
        return rows;
    }
    readonly property var pickable: bounds.filter(b => !isLocked(b.id))
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
    property bool snapGrid: false
    property bool snapAlign: false
    property real snapStep: 8
    property var snapLines: []
    readonly property var resizeHandles: [
        {name: "TopLeft", x: -1, y: -1}, {name: "Top", x: 0, y: -1},
        {name: "TopRight", x: 1, y: -1}, {name: "Right", x: 1, y: 0},
        {name: "BottomRight", x: 1, y: 1}, {name: "Bottom", x: 0, y: 1},
        {name: "BottomLeft", x: -1, y: 1}, {name: "Left", x: -1, y: 0}]
    function handlePoint(handle) {
        const bound = gesture ? gesture.bound : selectedBounds;
        return bound ? geometry.point(bound.transform, handle.x*bound.size[0]/2, handle.y*bound.size[1]/2) : {x:0,y:0};
    }
    function resizeCursor(handle) {
        const bound = gesture ? gesture.bound : selectedBounds;
        if (!bound) return Qt.ArrowCursor;
        const matrix = bound.transform;
        const angle = (Math.atan2(matrix[1]*handle.x+matrix[3]*handle.y,matrix[0]*handle.x+matrix[2]*handle.y)*180/Math.PI+180)%180;
        return [Qt.SizeHorCursor,Qt.SizeFDiagCursor,Qt.SizeVerCursor,Qt.SizeBDiagCursor][Math.round(angle/45)%4];
    }
    function boxInFrame(bound, inverse) {
        const points = geometry.rectangle(bound.transform, [-bound.size[0]/2,-bound.size[1]/2,bound.size[0]/2,bound.size[1]/2]).map(p => geometry.point(inverse,p.x,p.y));
        return [Math.min(...points.map(p=>p.x)), Math.min(...points.map(p=>p.y)), Math.max(...points.map(p=>p.x)), Math.max(...points.map(p=>p.y))];
    }
    function alignAxis(values, targets, tolerance) {
        let result = {delta:0, target:null}, distance = tolerance;
        targets.forEach(target => values.forEach(value => {
            const d = Math.abs(target-value);
            if (d < distance) { distance = d; result = {delta:target-value, target:target}; }
        }));
        return result;
    }
    function showSnapLines(g, hits) {
        snapLines = hits.map((target,axis) => target === null ? null : {
            a: geometry.point(g.frame, axis === 0 ? target : g.extent[0], axis === 0 ? g.extent[1] : target),
            b: geometry.point(g.frame, axis === 0 ? target : g.extent[2], axis === 0 ? g.extent[3] : target)
        }).filter(line => line !== null);
    }
    function editable(w) {
        if (!w || w.world_actor || isLocked(w.element.id)) return false;
        const parent = document.widgets.find(p => p.element.id === w.element.parent);
        return !w.element.parent || (w.layout && w.layout.absolute) || (parent && parent.element.kind === "Canvas");
    }
    function startEdit(kind, x, y, handle) {
        if (gesture || !widget || ((kind === "Move" || kind === "Resize") && !editable(widget))) return false;
        const bound = selectedBounds;
        if (x !== null && (!layoutReady || !bound || !bound.visible)) return false;
        const parent = bounds.find(w => w.id === widget.element.parent);
        const frame = parent ? parent.transform.slice() : [designScale,0,0,designScale,safe[0],safe[1]];
        const frameInverse = geometry.inverse(frame);
        if (x !== null && (!geometry.inverse(bound.transform) || !frameInverse)) return false;
        const extent = parent ? [-parent.size[0]/2,-parent.size[1]/2,parent.size[0]/2,parent.size[1]/2] : [0,0,(previewWidth-safe[0]-safe[2])/designScale,(previewHeight-safe[1]-safe[3])/designScale];
        const targets = [[extent[0],(extent[0]+extent[2])/2,extent[2]], [extent[1],(extent[1]+extent[3])/2,extent[3]]];
        if (frameInverse) bounds.forEach(b => {
            const w = document.widgets.find(w => w.element.id === b.id);
            if (b.id === selectedId || !b.visible || !w || w.world_actor || (w.element.parent || "") !== (widget.element.parent || "")) return;
            const box = boxInFrame(b, frameInverse);
            for (let axis=0;axis<2;++axis) targets[axis].push(box[axis],(box[axis]+box[axis+2])/2,box[axis+2]);
        });
        const g = {kind: kind, id: selectedId, original: copy(document), token: null,
            offset: (widget.element.offset || [0,0]).slice(), start: {x: x, y: y},
            inverse: bound ? geometry.inverse(bound.transform) : null,
            parentInverse: parent ? geometry.inverse(parent.transform) : null,
            size: bound ? [bound.size[0], bound.size[1]] : (widget.element.size || [0,0]).slice(),
            bound: bound, direction: handle || {x:1,y:1}, frame: frame, frameInverse: frameInverse,
            box: bound && frameInverse ? boxInFrame(bound,frameInverse) : null, targets: targets, extent: extent,
            minimum: [0,1].map(axis => Math.max(1,widget.layout ? widget.layout.min_size?.[axis] || 0 : 0)),
            maximum: [0,1].map(axis => { const max = widget.layout ? widget.layout.max_size?.[axis] || 0 : 0; const min = widget.layout ? widget.layout.min_size?.[axis] || 0 : 0; return max > 0 ? Math.max(1,min,max) : Infinity; }),
            grid: snapGrid, align: snapAlign, step: Math.max(1,snapStep),
            tolerance: [6/(Math.max(0.05,zoom)*Math.hypot(frame[0],frame[1])),6/(Math.max(0.05,zoom)*Math.hypot(frame[2],frame[3]))],
            scale: designScale, committing: false, edit: null, sent: null, busy: false, released: false, canceled: false};
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
        g.canceled = true; gesture = null; snapLines = []; refresh(); error = String(e);
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
                if (g.select !== undefined) { root.extraIds = g.more || []; root.selectedId = g.select; }
                root.flushEdit(g);
            }, function(e) { root.failEdit(g, e); });
        } else if (g.released) {
            g.busy = true; g.committing = true;
            app.invoke("commit_interface_edit", {token: g.token}, function() {
                if (g.canceled || root.gesture !== g) return;
                root.gesture = null; root.snapLines = []; root.refresh(); root.error = "";
                if (root.designing) previewDelay.restart();
            }, function(e) { root.failEdit(g, e); });
        }
    }
    function dragEdit(x, y, modifiers) {
        const g = gesture;
        if (!g || g.released || !g.inverse) return;
        let dx = x-g.start.x, dy = y-g.start.y;
        const snapping = !(modifiers & Qt.ShiftModifier);
        let offset = g.offset.slice();
        snapLines = [];
        if (g.kind === "Move") {
            if (g.parentInverse) {
                const a = geometry.point(g.parentInverse, g.start.x, g.start.y), b = geometry.point(g.parentInverse, x, y);
                dx = b.x-a.x; dy = b.y-a.y;
            } else { dx /= g.scale; dy /= g.scale; }
            offset = [g.offset[0]+dx, g.offset[1]+dy];
            if (g.grid && snapping) offset = offset.map(v => Math.round(v/g.step)*g.step);
            if (g.align && snapping) {
                const delta = [offset[0]-g.offset[0],offset[1]-g.offset[1]];
                const hits = [0,1].map(axis => alignAxis([g.box[axis]+delta[axis],(g.box[axis]+g.box[axis+2])/2+delta[axis],g.box[axis+2]+delta[axis]],g.targets[axis],g.tolerance[axis]));
                offset = offset.map((v,axis)=>v+hits[axis].delta);
                showSnapLines(g,hits.map(hit=>hit.target));
            }
            g.edit = {kind: "Move", id: g.id, offset: offset};
        } else {
            const a = geometry.point(g.inverse, g.start.x, g.start.y), b = geometry.point(g.inverse, x, y);
            const direction = g.direction;
            const constrain = (value,axis) => Math.min(g.maximum[axis],Math.max(g.minimum[axis],value));
            let size = [direction.x ? g.size[0]+direction.x*(b.x-a.x) : g.size[0], direction.y ? g.size[1]+direction.y*(b.y-a.y) : g.size[1]];
            size = size.map((v,axis) => (axis === 0 ? direction.x : direction.y) ? constrain(g.grid && snapping ? Math.round(v/g.step)*g.step : v,axis) : v);
            // Edge alignment is representable only when widget axes match its parent.
            const origin = geometry.point(g.frameInverse,g.bound.transform[4],g.bound.transform[5]);
            const axisX = geometry.point(g.frameInverse,g.bound.transform[4]+g.bound.transform[0],g.bound.transform[5]+g.bound.transform[1]);
            const axisY = geometry.point(g.frameInverse,g.bound.transform[4]+g.bound.transform[2],g.bound.transform[5]+g.bound.transform[3]);
            const relative = [axisX.x-origin.x,axisX.y-origin.y,axisY.x-origin.x,axisY.y-origin.y];
            if (g.align && snapping && Math.abs(relative[1])<0.001 && Math.abs(relative[2])<0.001 && relative[0]>0 && relative[3]>0) {
                const hits = [null,null];
                [direction.x,direction.y].forEach((d,axis) => {
                    if (!d) return;
                    const scale = relative[axis === 0 ? 0 : 3];
                    const edge = g.box[axis+(d>0 ? 2 : 0)]+d*(size[axis]-g.size[axis])*scale;
                    const hit = alignAxis([edge],g.targets[axis],g.tolerance[axis]);
                    const snapped = size[axis]+d*hit.delta/scale;
                    if (snapped >= g.minimum[axis] && snapped <= g.maximum[axis]) { size[axis] = snapped; hits[axis] = hit.target; }
                });
                showSnapLines(g,hits);
            }
            const w = g.original.widgets.find(w => w.element.id === g.id);
            const fractions = {TopLeft:[0,0], Top:[0.5,0], TopRight:[1,0], Left:[0,0.5], Center:[0.5,0.5], Right:[1,0.5], BottomLeft:[0,1], Bottom:[0.5,1], BottomRight:[1,1]};
            const f = w.layout && w.layout.absolute ? [0,0] : fractions[w.element.anchor || "Center"];
            const delta = [size[0]-g.size[0],size[1]-g.size[1]];
            const center = [direction.x*delta[0]/2,direction.y*delta[1]/2];
            offset = [g.offset[0]+relative[0]*center[0]+relative[2]*center[1]+(f[0]-0.5)*delta[0], g.offset[1]+relative[1]*center[0]+relative[3]*center[1]+(f[1]-0.5)*delta[1]];
            g.edit = {kind: "Resize", id: g.id, size: size, offset: offset};
        }
        flushEdit(g);
    }
    function finishEdit() { if (gesture) { gesture.released = true; flushEdit(gesture); } }
    function cancelEdit() {
        const g = gesture;
        if (!g || g.committing) return;
        g.canceled = true; gesture = null; snapLines = [];
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
    readonly property var siblings: widget ? document.widgets.filter(w => (w.element.parent || "") === (widget.element.parent || "")) : []
    readonly property int siblingIndex: siblings.findIndex(w => w.element.id === selectedId)
    function reorderSibling(delta) {
        const index = siblingIndex+delta;
        if (!designing || gesture || index < 0 || index >= siblings.length) return;
        submitEdit({kind:"Reorder", id:selectedId, index:index});
    }
    function nudge(event) {
        if (!activeFocus || !designing || !editable(widget) || (event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier))) return false;
        const directions = {};
        directions[Qt.Key_Left] = [-1,0]; directions[Qt.Key_Right] = [1,0];
        directions[Qt.Key_Up] = [0,-1]; directions[Qt.Key_Down] = [0,1];
        const direction = directions[event.key];
        if (!direction) return false;
        if (!gesture) {
            if (!startEdit("Move",null,null)) return false;
            gesture.keyboard = true; gesture.held = {};
        }
        const g = gesture;
        if (!g.keyboard || g.committing) return false;
        g.released = false;
        g.held[event.key] = true;
        const offset = g.edit ? g.edit.offset.slice() : g.offset.slice();
        const step = event.modifiers & Qt.ShiftModifier ? 10 : 1;
        g.edit = {kind:"Move", id:g.id, offset:[offset[0]+direction[0]*step,offset[1]+direction[1]*step]};
        flushEdit(g);
        return true;
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
    Keys.priority: Keys.AfterItem
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Escape && gesture) { cancelEdit(); event.accepted = true; }
        else if ((event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace) && activeFocus && designing && !gesture && widget) { removeSelected(); event.accepted = true; }
        else event.accepted = nudge(event);
    }
    Keys.onReleased: event => {
        const g = gesture;
        if (!g || !g.keyboard || !g.held[event.key]) { event.accepted = false; return; }
        event.accepted = true;
        if (event.isAutoRepeat) return;
        delete g.held[event.key];
        if (!Object.keys(g.held).length) finishEdit();
    }
    onActiveFocusChanged: { if (!activeFocus && gesture && gesture.keyboard) cancelEdit(); }
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
            viewport: [previewWidth, previewHeight], screen: screenId || null, hidden: hiddenList, document: copy(document)}},
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
    onSelectedIdChanged: { if (gesture && gesture.keyboard) cancelEdit(); overlay.requestPaint(); }
    onHoveredIdChanged: overlay.requestPaint()
    onMarqueeChanged: overlay.requestPaint()
    onExtraIdsChanged: overlay.requestPaint()
    onSnapLinesChanged: overlay.requestPaint()
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
    function structuralEdit(edit, select, more) {
        if (!designing || gesture) return false;
        const g = {kind: edit.kind, id: "", original: copy(document), token: null, bound: null, committing: false,
            edit: edit, sent: null, busy: false, released: true, canceled: false, select: select, more: more || []};
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
    function uniqueId(base) {
        let n = 1;
        while (document.widgets.some(w => w.element.id === base + n)) ++n;
        return base + n;
    }
    readonly property var containerKinds: ["Canvas","VerticalBox","HorizontalBox","Grid","WrapBox"]
    function add(kind, x, y) {
        const id = uniqueId(kind.toLowerCase());
        const parent = widget && !widget.world_actor && containerKinds.indexOf(widget.element.kind) >= 0 && x === 20 && y === 20 ? widget : null;
        const free = !parent || parent.element.kind === "Canvas";
        // Values use the core's tagged representation; default values can be omitted.
        const created = {element: {id: id, kind: kind, content: ["Label","Button","RichText","Toggle"].indexOf(kind) >= 0 ? kind : "", anchor: "TopLeft", offset: free ? [Math.round(x),Math.round(y)] : [0,0], size: [180, kind === "Panel" || kind === "ListView" ? 180 : 40], parent: parent ? parent.element.id : "", modal: false, range: [0,100]}, style: {}, bindings: [], items: []};
        if (structuralEdit({kind: "Create", widget: created}, id)) screenId = "";
    }
    function duplicateSelected() {
        if (!widget || widget.world_actor) return;
        const taken = {};
        document.widgets.forEach(w => { taken[w.element.id] = true; });
        const edits = [];
        topLevelSelection().forEach(id => {
            const source = document.widgets.find(w => w.element.id === id);
            if (!source || source.world_actor) return;
            const base = id + "-copy";
            let name = base, n = 1;
            while (taken[name]) name = base + (++n);
            taken[name] = true;
            const edit = {kind: "Duplicate", id: id, new_id: name};
            if (editable(source)) { const o = source.element.offset || [0,0]; edit.offset = [o[0]+16, o[1]+16]; }
            edits.push(edit);
        });
        if (!edits.length) return;
        structuralEdit(edits.length === 1 ? edits[0] : {kind: "Batch", edits: edits}, edits[0].new_id, edits.slice(1).map(e => e.new_id));
    }
    function removeSelected() {
        if (!widget) return;
        const edits = topLevelSelection().map(id => ({kind: "Delete", id: id}));
        structuralEdit(edits.length === 1 ? edits[0] : {kind: "Batch", edits: edits}, "");
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
            TextField {
                objectName: "interfaceSearch"
                Layout.fillWidth: true; Layout.leftMargin: 6; Layout.rightMargin: 6
                placeholderText: "Search widgets"
                text: root.search
                onTextEdited: root.search = text
            }
            ListView {
                objectName: "interfaceTree"
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                model: root.treeRows
                delegate: ItemDelegate {
                    required property var modelData
                    width: ListView.view.width; height: 30
                    highlighted: root.selectionIds.indexOf(modelData.id) >= 0
                    onClicked: mouse => { if (mouse && (mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier))) root.toggleSelected(modelData.id); else root.selectOnly(modelData.id); root.forceActiveFocus(); }
                    contentItem: RowLayout {
                        spacing: 2
                        Item { Layout.preferredWidth: 10 + modelData.depth * 14 }
                        ToolButton {
                            objectName: "interfaceFold"
                            Layout.preferredWidth: 22; Layout.preferredHeight: 24
                            text: modelData.kids ? (modelData.collapsed ? "\u25B8" : "\u25BE") : ""
                            enabled: modelData.kids > 0
                            onClicked: root.toggleCollapsed(modelData.id)
                        }
                        Label { Layout.fillWidth: true; elide: Text.ElideRight; text: modelData.id; opacity: root.hiddenIds[modelData.id] ? 0.4 : (root.isLocked(modelData.id) ? 0.6 : 1) }
                        ToolButton {
                            objectName: "interfaceHide"
                            Layout.preferredWidth: 26; Layout.preferredHeight: 24
                            text: root.hiddenIds[modelData.id] ? "\u25CB" : "\u25CF"
                            ToolTip.visible: hovered; ToolTip.text: root.hiddenIds[modelData.id] ? "Show in preview (editor only)" : "Hide in preview (editor only)"
                            onClicked: root.toggleHidden(modelData.id)
                        }
                        ToolButton {
                            objectName: "interfaceLock"
                            Layout.preferredWidth: 26; Layout.preferredHeight: 24
                            text: root.isLocked(modelData.id) ? "\uD83D\uDD12" : "\u00B7"
                            ToolTip.visible: hovered; ToolTip.text: root.isLocked(modelData.id) ? "Unlock (editor only)" : "Lock in the viewport (editor only)"
                            onClicked: root.toggleLock(modelData.id)
                        }
                    }
                }
            }
            RowLayout {
                Button { objectName: "interfaceEarlier"; text: "Earlier"; enabled: root.designing && !root.gesture && root.siblingIndex > 0; onClicked: root.reorderSibling(-1) }
                Button { objectName: "interfaceLater"; text: "Later"; enabled: root.designing && !root.gesture && root.siblingIndex >= 0 && root.siblingIndex+1 < root.siblings.length; onClicked: root.reorderSibling(1) }
            }
            Label { Layout.fillWidth: true; wrapMode: Text.Wrap; text: "Sibling order controls flow layout and draw order." }
            RowLayout {
                Button { objectName: "interfaceDuplicate"; text: "Duplicate"; enabled: root.designing && !root.gesture && !!root.widget && !root.widget.world_actor; onClicked: root.duplicateSelected() }
                Button { objectName: "interfaceDelete"; text: "Delete"; enabled: root.designing && !root.gesture && !!root.widget; onClicked: root.removeSelected() }
            }
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
            RowLayout {
                CheckBox { text: "Grid"; checked: root.snapGrid; onToggled: root.snapGrid = checked }
                SpinBox { from: 1; to: 256; value: root.snapStep; editable: true; onValueModified: root.snapStep = value }
                Label { text: "px" }
                CheckBox { text: "Align edges/centers"; checked: root.snapAlign; onToggled: root.snapAlign = checked }
                Label { text: "Hold Shift to bypass snapping" }
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
                        visible: root.layoutReady || !!root.gesture
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
                            root.selectionIds.slice(1).forEach(id => outline(id, "#8fd0a0"));
                            outline(root.selectedId, "#70baff");
                            const m = root.marquee;
                            if (m) {
                                ctx.fillStyle = "rgba(112,186,255,0.15)"; ctx.strokeStyle = "#70baff"; ctx.lineWidth = 1/Math.max(0.05,root.zoom);
                                ctx.fillRect(Math.min(m.x0,m.x1), Math.min(m.y0,m.y1), Math.abs(m.x1-m.x0), Math.abs(m.y1-m.y0));
                                ctx.strokeRect(Math.min(m.x0,m.x1), Math.min(m.y0,m.y1), Math.abs(m.x1-m.x0), Math.abs(m.y1-m.y0));
                            }
                            ctx.strokeStyle = "#ffcc70";
                            ctx.lineWidth = 1/Math.max(0.05,root.zoom);
                            root.snapLines.forEach(line => { ctx.beginPath(); ctx.moveTo(line.a.x,line.a.y); ctx.lineTo(line.b.x,line.b.y); ctx.stroke(); });
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
                            if (pressed && marqueeStart) { root.marquee = {x0: marqueeStart.x, y0: marqueeStart.y, x1: mouse.x, y1: mouse.y}; return; }
                            if (pressed) {
                                if (!root.gesture && Math.hypot(mouse.x-pressX, mouse.y-pressY) > 3/Math.max(0.05,root.zoom)) root.startEdit("Move", pressX, pressY);
                                root.dragEdit(mouse.x, mouse.y, mouse.modifiers);
                            }
                            else root.hoveredId = geometry.pick(root.pickable, mouse.x, mouse.y);
                        }
                        onExited: root.hoveredId = ""
                        onPressed: mouse => {
                            root.forceActiveFocus();
                            const hit = geometry.pick(root.pickable, mouse.x, mouse.y);
                            pressX = mouse.x; pressY = mouse.y;
                            if (mouse.modifiers & Qt.ShiftModifier) { root.toggleSelected(hit); marqueeStart = null; }
                            else if (!hit) { root.selectOnly(""); marqueeStart = {x: mouse.x, y: mouse.y}; }
                            else { if (hit !== root.selectedId) root.selectOnly(hit); marqueeStart = null; }
                        }
                        property var marqueeStart: null
                        onReleased: {
                            if (marqueeStart && root.marquee) root.finishMarquee(root.marquee);
                            marqueeStart = null; root.marquee = null;
                            root.finishEdit();
                        }
                        onCanceled: { marqueeStart = null; root.marquee = null; root.cancelEdit(); }
                    }
                    Repeater {
                        model: root.resizeHandles
                        delegate: Rectangle {
                            required property var modelData
                            readonly property var point: root.handlePoint(modelData)
                            objectName: modelData.name === "BottomRight" ? "interfaceResizeHandle" : "interfaceResize"+modelData.name
                            visible: (root.gesture && root.gesture.kind === "Resize") || (root.layoutReady && !!root.selectedBounds && root.selectedBounds.visible && root.editable(root.widget))
                            x: point.x-width/2; y: point.y-height/2
                            width: 10/Math.max(0.05,root.zoom); height: width
                            color: "#70baff"
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: root.resizeCursor(modelData)
                                onPressed: mouse => { const p = canvas.mapFromItem(parent,mouse.x,mouse.y); root.startEdit("Resize",p.x,p.y,modelData); }
                                onPositionChanged: mouse => { if (pressed) { const p = canvas.mapFromItem(parent,mouse.x,mouse.y); root.dragEdit(p.x,p.y,mouse.modifiers); } }
                                onReleased: root.finishEdit()
                                onCanceled: root.cancelEdit()
                            }
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
            Label { text: root.error || "Drag to move/resize. Arrows nudge 1 px; Shift+arrows 10 px. Escape cancels. Parent layout controls flow widgets."; color: root.error ? "#ff8888" : Theme.textDim; wrapMode: Text.Wrap; Layout.fillWidth: true }
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
