import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore
import com.blockworked.Blockloom 1.0
import com.blockworked.Blockstitch 1.0

Item {
    id: root
    InterfaceGeometry { id: geometry }
    UiEditMath { id: editMath }
    readonly property var geo: geometry
    // Editor-only preferences: preview size and which inspector sections are folded.
    Loader {
        id: prefsLoader
        // Settings need an application identity; a bare test host has none.
        active: Qt.application.organization !== ""
        sourceComponent: Settings {
            category: "interfaceDesigner"
            property int previewWidth: 960
            property int previewHeight: 720
            property string folds: ""
        }
    }
    readonly property var prefs: prefsLoader.item
    required property var app
    property var document: ({widgets: [], styles: {}, prefabs: {}, reference_size: [960,720], safe_area: [0,0,0,0], theme: "Dark", scale: "ConstantPixel"})
    property string selectedId: ""
    property string screenId: ""
    // The .blockui file being edited, or "" for the scene's own interface.
    property string target: ""
    property var targetDocument: null
    // Every screen the open scene or the game draws, each once.
    readonly property var targets: {
        const p = app.appState.project;
        if (!p) return [];
        const all = ((p.world && p.world.screens) || []).concat((p.settings && p.settings.screens) || []).map(l => l.screen);
        return all.filter((s, i) => all.indexOf(s) === i);
    }
    onTargetsChanged: if (target && targets.indexOf(target) < 0) target = ""
    onTargetChanged: { cancelEdit(); selectedId = ""; extraIds = []; screenId = ""; targetDocument = null; lastSaved = null; loadTarget(); }
    function loadTarget() {
        if (!target) { refresh(); return; }
        const path = target;
        app.invoke("screen_document", {path: path}, function(doc) {
            if (root.target !== path) return;
            root.targetDocument = doc; root.refresh();
            if (root.designing) previewDelay.restart();
        }, function(e) { if (root.target === path) root.error = String(e); });
    }
    readonly property var widgetsById: {
        const m = Object.create(null);
        document.widgets.forEach(w => { m[w.element.id] = w; });
        return m;
    }
    function widgetNamed(id) { return widgetsById[id] || null; }
    readonly property var screens: document.widgets.filter(w => !w.element.parent).map(w => w.element.id)
    readonly property var screenWidgets: document.widgets.filter(w => inScreen(w.element.id))
    function inScreen(id) {
        if (!screenId) return true;
        let current = widgetsById[id];
        const visited = Object.create(null);
        while (current && !visited[current.element.id]) {
            if (current.element.id === screenId) return true;
            visited[current.element.id] = true;
            current = widgetsById[current.element.parent];
        }
        return false;
    }
    // Editor-only state: never saved and never sent to the runtime.
    property var lockedIds: Object.create(null)
    // Further selected widgets beside selectedId (Shift-click, Ctrl-click in the tree, marquee).
    // Draw the project's actors behind the interface in the preview. Editor only.
    property bool showWorld: true
    onShowWorldChanged: { cancelEdit(); ++revision; frameLayout = null; if (designing) previewDelay.restart(); }
    property bool showSafeArea: false
    property bool showReference: false
    // Editor-only guides in preview pixels: {axis: 0 vertical line at x, 1 horizontal line at y, pos}.
    property bool showRulers: false
    property var guides: []
    onGuidesChanged: viewport.repaint()
    function addGuide(axis, pos) { guides = guides.concat([{axis: axis, pos: pos}]); return guides.length - 1; }
    function moveGuide(index, pos) { guides = guides.map((g, i) => i === index ? {axis: g.axis, pos: pos} : g); }
    function removeGuide(index) { guides = guides.filter((g, i) => i !== index); }
    // Axis 0 is x, 1 is y; stage pixels are the viewport, canvas pixels the preview.
    function stageToCanvas(axis, s) { return (axis === 0 ? previewWidth : previewHeight) / 2 + (s - (axis === 0 ? viewport.stageWidth : viewport.stageHeight) / 2 - (axis === 0 ? panX : panY)) / Math.max(0.05, zoom); }
    function canvasToStage(axis, c) { return (axis === 0 ? viewport.stageWidth : viewport.stageHeight) / 2 + (axis === 0 ? panX : panY) + (c - (axis === 0 ? previewWidth : previewHeight) / 2) * Math.max(0.05, zoom); }
    function guideTargets(frame, axis) {
        // Guides only snap in axis-aligned frames, in the frame's own units.
        if (Math.abs(frame[1]) > 1e-6 || Math.abs(frame[2]) > 1e-6) return [];
        return guides.filter(g => g.axis === axis).map(g => (g.pos - frame[axis === 0 ? 4 : 5]) / frame[axis === 0 ? 0 : 3]);
    }
    onShowSafeAreaChanged: viewport.repaint()
    onShowReferenceChanged: viewport.repaint()
    onSafeChanged: viewport.repaint()
    property var extraIds: []
    readonly property var selectionIds: {
        if (!selectedId) return [];
        return [selectedId].concat(extraIds.filter(id => widgetsById[id] && id !== selectedId && inScreen(id)));
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
            const polygon = geometry.polygon(b);
            if (!polygon.length) return false;
            const xs = polygon.map(p => p.x), ys = polygon.map(p => p.y);
            return Math.max(...xs) >= x0 && Math.min(...xs) <= x1 && Math.max(...ys) >= y0 && Math.min(...ys) <= y1;
        }).sort((a, b) => a.paint_order - b.paint_order).map(b => b.id);
        extraIds = ids.slice(1); selectedId = ids.length ? ids[0] : "";
    }
    function editableId(id) { const w = widgetsById[id]; return !!w && !w.world_actor; }
    property var hiddenIds: Object.create(null)
    property var collapsedIds: Object.create(null)
    function toggleHidden(id) {
        const next = Object.assign(Object.create(null), hiddenIds); if (next[id]) delete next[id]; else next[id] = true;
        hiddenIds = next; cancelEdit(); ++revision; frameLayout = null; if (designing) previewDelay.restart();
    }
    // Only IDs that still exist: the runtime refuses a request naming a missing widget.
    readonly property var hiddenList: document.widgets.map(w => w.element.id).filter(id => !!hiddenIds[id])
    property string search: ""
    function isLocked(id) { return !!lockedIds[id]; }
    function toggleLock(id) { const next = Object.assign(Object.create(null), lockedIds); if (next[id]) delete next[id]; else next[id] = true; lockedIds = next; if (gesture && gesture.id === id) cancelEdit(); }
    function toggleCollapsed(id) { const next = Object.assign(Object.create(null), collapsedIds); if (next[id]) delete next[id]; else next[id] = true; collapsedIds = next; }
    readonly property var treeRows: {
        const widgets = screenWidgets, children = Object.create(null), byId = Object.create(null);
        widgets.forEach(w => { byId[w.element.id] = w; });
        widgets.forEach(w => {
            const parent = byId[w.element.parent] ? w.element.parent : "";
            (children[parent] = children[parent] || []).push(w);
        });
        const needle = search.trim().toLowerCase();
        const matches = w => w.element.id.toLowerCase().indexOf(needle) >= 0 || (w.element.content || "").toLowerCase().indexOf(needle) >= 0;
        const keep = Object.create(null);
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
    readonly property var selectedBounds: boundsById[selectedId] || null
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
    function boxInFrame(bound, inverse) { return editMath.boxInFrame(bound, inverse); }
    function alignAxis(values, targets, tolerance) { return editMath.alignAxis(values, targets, tolerance); }
    function showSnapLines(g, hits) { snapLines = editMath.snapLines(g.frame, g.extent, hits); }
    function editable(w) {
        if (!w || w.world_actor || isLocked(w.element.id) || animPreviewing) return false;
        const parent = widgetsById[w.element.parent];
        return !w.element.parent || (w.layout && w.layout.absolute) || (parent && parent.element.kind === "Canvas");
    }
    function startEdit(kind, x, y, handle) {
        if (gesture || !widget || ((kind === "Move" || kind === "Resize") && !editable(widget))) return false;
        const bound = selectedBounds;
        if (x !== null && (!layoutReady || !bound || !bound.visible)) return false;
        const parent = boundsById[widget.element.parent] || null;
        const frame = parent ? parent.transform.slice() : [designScale,0,0,designScale,safe[0],safe[1]];
        const frameInverse = geometry.inverse(frame);
        if (x !== null && (!geometry.inverse(bound.transform) || !frameInverse)) return false;
        const extent = parent ? [-parent.size[0]/2,-parent.size[1]/2,parent.size[0]/2,parent.size[1]/2] : [0,0,(previewWidth-safe[0]-safe[2])/designScale,(previewHeight-safe[1]-safe[3])/designScale];
        const siblingBoxes = [];
        if (frameInverse) bounds.forEach(b => {
            const w = widgetsById[b.id];
            if (b.id === selectedId || !b.visible || !w || w.world_actor || (w.element.parent || "") !== (widget.element.parent || "")) return;
            siblingBoxes.push({box: editMath.boxInFrame(b, frameInverse)});
        });
        const targets = editMath.snapTargets(extent, siblingBoxes, [guideTargets(frame, 0), guideTargets(frame, 1)]);
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
        const others = selectionIds.slice(1).map(id => widgetsById[id] || null);
        g.group = kind === "Move" && others.length && others.every(w => w && editable(w) && (w.element.parent || "") === (widget.element.parent || ""))
            ? others.map(w => ({id: w.element.id, offset: (w.element.offset || [0,0]).slice()})) : null;
        gesture = g;
        forceActiveFocus();
        const backend = app;
        app.invoke("begin_interface_edit", {revision: savedRevision, screen: target || null}, function(token) {
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
            app.invoke("update_interface_edit", {token: g.token, edit: edit, patch: true}, function(reply) {
                g.busy = false;
                if (g.canceled || root.gesture !== g) return;
                const next = root.applyUpdate(reply);
                if (!next) { root.failEdit(g, "The editor's reply did not match the draft. Try again."); return; }
                root.document = next;
                if (g.select !== undefined) { root.extraIds = g.more || []; root.selectedId = g.select; }
                root.flushEdit(g);
            }, function(e) { root.failEdit(g, e); });
        } else if (g.released) {
            g.busy = true; g.committing = true;
            app.invoke("commit_interface_edit", {token: g.token}, function() {
                if (g.canceled || root.gesture !== g) return;
                // A file isn't in the snapshot: keep the draft until it is read back.
                if (root.target) { root.targetDocument = root.copy(root.document); root.loadTarget(); }
                root.gesture = null; root.snapLines = []; root.refresh(); root.error = "";
                if (root.designing) previewDelay.restart();
            }, function(e) { root.failEdit(g, e); });
        }
    }
    // An update answers with the whole document or, as asked here, a patch:
    // every field but the widgets, all ids in order, and the widgets that
    // changed. Unchanged widgets keep their objects, so their bindings stay put.
    function applyUpdate(reply) {
        if (!reply || !Array.isArray(reply.order)) return reply;
        const changed = Object.create(null);
        (reply.widgets || []).forEach(w => { changed[w.element.id] = w; });
        const widgets = reply.order.map(id => changed[id] || widgetsById[id]);
        if (widgets.some(w => !w)) return null;
        return Object.assign({}, reply.document, {widgets: widgets});
    }
    function dragEdit(x, y, modifiers) {
        const g = gesture;
        if (!g || g.released || !g.inverse) return;
        const snapping = !(modifiers & Qt.ShiftModifier);
        snapLines = [];
        if (g.kind === "Move") {
            let dx = x - g.start.x, dy = y - g.start.y;
            if (g.parentInverse) {
                const a = geometry.point(g.parentInverse, g.start.x, g.start.y), b = geometry.point(g.parentInverse, x, y);
                dx = b.x - a.x; dy = b.y - a.y;
            } else { dx /= g.scale; dy /= g.scale; }
            const result = editMath.moveResult(g, dx, dy, snapping);
            if (result.hits) showSnapLines(g, result.hits);
            g.edit = {kind: "Move", id: g.id, offset: result.offset};
            if (g.group) {
                const shift = [result.offset[0] - g.offset[0], result.offset[1] - g.offset[1]];
                g.edit = {kind: "Batch", edits: [g.edit].concat(g.group.map(o => ({kind: "Move", id: o.id, offset: [o.offset[0] + shift[0], o.offset[1] + shift[1]]})))};
            }
        } else {
            const a = geometry.point(g.inverse, g.start.x, g.start.y), b = geometry.point(g.inverse, x, y);
            const w = g.original.widgets.find(w => w.element.id === g.id);
            const fraction = w.layout && w.layout.absolute ? [0, 0] : editMath.anchorFractions[w.element.anchor || "Center"];
            const result = editMath.resizeResult(g, a, b, snapping, fraction);
            if (result.hits) showSnapLines(g, result.hits);
            g.edit = {kind: "Resize", id: g.id, size: result.size, offset: result.offset};
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
        const visited = Object.create(null);
        let w = widgetsById[id];
        while (w && !visited[w.element.id]) {
            if (w.element.id === ancestor) return true;
            visited[w.element.id] = true;
            w = widgetsById[w.element.parent];
        }
        return false;
    }
    function reparent(parentId) {
        if (!widget || (widget.element.parent || "") === parentId) return;
        const target = document.widgets.find(w => w.element.id === parentId);
        let placement = {mode: "Flow"};
        if (!target || target.element.kind === "Canvas") {
            const bound = selectedBounds, parent = boundsById[parentId] || null;
            if (!layoutReady || !bound || (target && !parent)) { error = "Wait for matching geometry before reparenting."; return; }
            if (!bound.visible || (parent && !parent.visible)) { error = "Show all screens before reparenting into a hidden tree."; return; }
            const result = editMath.reparentPlacement(bound, parent || null, designScale, safe);
            if (result.error) { error = result.error; return; }
            placement = result.placement;
        }
        submitEdit({kind: "Reparent", id: selectedId, parent: parentId, placement: placement});
    }
    Keys.priority: Keys.AfterItem
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Escape && gesture) { cancelEdit(); event.accepted = true; }
        else if (event.key === Qt.Key_Delete && activeFocus && designing && !gesture && widget) { removeSelected(); event.accepted = true; }
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
    readonly property var boundsById: {
        const m = Object.create(null);
        bounds.forEach(b => { m[b.id] = b; });
        return m;
    }
    function receiveLayout(text) {
        try { frameLayout = text ? JSON.parse(text) : null; } catch(e) { frameLayout = null; }
    }
    function requestPreview() {
        if (!designing || !app.appState.project) return;
        ++revision;
        frameLayout = null;
        app.invoke("preview_interface", {design: {revision: revision, generation: generation,
            viewport: [previewWidth, previewHeight], screen: screenId || null, hidden: hiddenList, world: showWorld,
            animation: animPreviewing ? {name: animPreviewName, time: animPreviewTime} : null, document: copy(document)}},
            function() { root.error = ""; }, function(e) { root.error = String(e); });
    }
    function updateSession() {
        if (!designing) { cancelEdit(); animPreviewName = ""; }
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
    onSelectedIdChanged: { if (gesture && gesture.keyboard) cancelEdit(); viewport.repaint(); Qt.callLater(loadOverrides); }
    onHoveredIdChanged: viewport.repaint()
    onMarqueeChanged: viewport.repaint()
    onExtraIdsChanged: viewport.repaint()
    onSnapLinesChanged: viewport.repaint()
    onFrameLayoutChanged: viewport.repaint()
    onLayoutReadyChanged: viewport.repaint()
    Timer { id: previewDelay; interval: 80; onTriggered: root.requestPreview() }
    readonly property double savedRevision: app.appState.sync ? app.appState.sync.revision : 0
    onSavedRevisionChanged: { if (gesture && !gesture.committing) cancelEdit(); frameLayout = null; if (designing) previewDelay.restart(); }
    readonly property var widget: selected >= 0 && selected < document.widgets.length ? document.widgets[selected] : null
    // Kinds each property applies to, from the backend's metadata; empty until loaded.
    property var propertyKinds: ({})
    // The backend's property table, in its order: every editable path with label, group and type.
    property var propertyInfo: []
    function propertyValue(path) {
        let value = widget;
        for (const step of path.split(".")) value = value === undefined || value === null ? undefined : value[step];
        return value;
    }
    // Text shown for a property in the generated form, and its inverse.
    function formatProperty(ty, value) {
        if (value === undefined || value === null) return ty === "layout" ? "null" : "";
        if (ty === "range") return value[0] + ", " + value[1];
        if (ty === "lines") return value.join("\n");
        if (ty === "layout" || ty === "style" || ty === "bindings" || ty === "actions" || ty === "variants" || ty === "fx") return JSON.stringify(value, null, 2);
        return String(value);
    }
    function parseProperty(ty, text) {
        if (ty === "range") { const p = text.split(",").map(t => Number(t.trim())); if (p.length !== 2 || p.some(isNaN)) throw "Range is two numbers, like 0, 100"; return p; }
        if (ty === "lines") return text === "" ? [] : text.split("\n");
        if (ty === "int") return text.trim() === "" ? null : Math.round(Number(text));
        if (ty === "number") { const n = Number(text); if (isNaN(n)) throw "Not a number"; return n; }
        if (ty === "layout" || ty === "style" || ty === "bindings" || ty === "actions" || ty === "variants" || ty === "fx") return JSON.parse(text);
        return text;
    }
    function applies(path) {
        const ks = propertyKinds[path];
        return !ks || ks.length === 0 || !widget || ks.indexOf(widget.element.kind) >= 0;
    }
    function loadPropertyKinds() {
        app.invoke("interface_properties", {}, function(list) {
            const m = Object.create(null);
            for (const p of (list || [])) m[p.path] = p.kinds || [];
            root.propertyKinds = m;
            root.propertyInfo = list || [];
        });
    }
    readonly property var kinds: ["Panel","Label","Button","Image","Input","Slider","Toggle","List","VerticalBox","HorizontalBox","Grid","Canvas","WrapBox","SizeBox","Spacer","Progress","RadialProgress","ListView","Tabs","Select","Scrollbar","RichText","Tooltip"]
    property int previewWidth: 960
    property int previewHeight: 720
    readonly property var safe: document.safe_area || [0,0,0,0]
    readonly property real designScale: document.scale === "ScaleWithSize" ? Math.max(0.01, Math.min((previewWidth-safe[0]-safe[2])/(document.reference_size || [960,720])[0], (previewHeight-safe[1]-safe[3])/(document.reference_size || [960,720])[1])) : 1
    readonly property real fitZoom: Math.min((viewport.stageWidth - 32) / previewWidth, (viewport.stageHeight - 32) / previewHeight)
    // Zoom follows the fit until the user sets one; panning is the canvas offset from centre.
    property real zoom: fitZoom
    property real panX: 0
    property real panY: 0
    function fitView() { zoom = Qt.binding(() => fitZoom); panX = 0; panY = 0; }
    function actualSize() { zoom = 1; panX = 0; panY = 0; }
    function zoomBy(factor) { zoomAt(viewport.stageWidth / 2, viewport.stageHeight / 2, factor); }
    function setPreviewSize(w, h) {
        if (w === previewWidth && h === previewHeight) return;
        previewWidth = w; previewHeight = h;
        if (prefs) { prefs.previewWidth = w; prefs.previewHeight = h; }
    }
    // Zooms by `factor` keeping the preview point under the stage position (sx, sy) where it is.
    function zoomAt(sx, sy, factor) {
        const old = Math.max(0.05, zoom);
        const next = Math.max(0.1, Math.min(8, old * factor));
        const cx = previewWidth / 2 + (sx - viewport.stageWidth / 2 - panX) / old;
        const cy = previewHeight / 2 + (sy - viewport.stageHeight / 2 - panY) / old;
        zoom = next;
        panX = sx - viewport.stageWidth / 2 - (cx - previewWidth / 2) * next;
        panY = sy - viewport.stageHeight / 2 - (cy - previewHeight / 2) * next;
    }
    function copy(v) { return JSON.parse(JSON.stringify(v)); }
    property var lastSaved: null
    property var lastDocument: null
    function refresh() {
        if (gesture) return;
        const saved = !app.appState.project ? null : target ? targetDocument : app.appState.project.world.interface;
        // Skip the comparison while the snapshot and our copy of it are both unchanged.
        if (saved && (saved !== lastSaved || document !== lastDocument)) {
            lastSaved = saved;
            if (JSON.stringify(saved) !== JSON.stringify(document)) document = copy(saved);
            lastDocument = document;
        }
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
    Component.onCompleted: {
        loadPropertyKinds(); refresh(); if (designing) updateSession();
        if (prefs) {
            try { if (prefs.folds) folded = Object.assign({}, folded, JSON.parse(prefs.folds)); } catch (e) {}
            previewWidth = prefs.previewWidth; previewHeight = prefs.previewHeight;
        }
    }
    Connections {
        target: root.app
        function onAppStateChanged() { root.refresh(); }
        function onPreviewLayoutChanged() { if (root.designing && !root.embedded) Qt.callLater(() => root.receiveLayout(root.app.previewLayout)); }
    }
    function change(field, value) {
        if (!widget) return;
        propertyEdit("element." + field, value);
    }
    // Fields written through a plain SetProperty of the same name.
    function extra(field, value) {
        if (!widget) return;
        propertyEdit(field, value);
    }
    function paint(field, value) {
        if (!widget) return;
        const style = copy(widget.style || {});
        if (!style[styleStateName]) style[styleStateName] = {};
        style[styleStateName][field] = value;
        extra("style", style);
    }
    // Image fit as the inspector sees it: auto, stretch, sliced or tiled.
    function imageFit() {
        const fit = widget ? ((widget.style || {})[styleStateName] || {}).image_fit : null;
        if (!fit) return {mode: "auto", border: [0,0,0,0]};
        if (fit === "Stretch") return {mode: "stretch", border: [0,0,0,0]};
        if (fit === "Tiled") return {mode: "tiled", border: [0,0,0,0]};
        return {mode: "sliced", border: (fit.Sliced && fit.Sliced.border) || [0,0,0,0]};
    }
    function setImageFit(mode, border) {
        if (mode === "auto") paint("image_fit", null);
        else if (mode === "stretch") paint("image_fit", "Stretch");
        else if (mode === "tiled") paint("image_fit", "Tiled");
        else paint("image_fit", {Sliced: {border: border || [0,0,0,0]}});
    }
    // The class's value for this state when the widget sets none of its own.
    function inheritedPaint(field) {
        if (!widget || !widget.class || !document.styles) return null;
        const sheet = document.styles[widget.class];
        const paint = sheet ? sheet[styleStateName] : null;
        return paint && paint[field] !== undefined && paint[field] !== null ? paint[field] : null;
    }
    function saveClass(name) {
        if (!widget) return;
        const edits = [{kind: "SetClass", name: name, styles: copy(widget.style || {})},
            {kind: "SetProperty", id: widget.element.id, property: {path: "class", value: name}}];
        structuralEdit({kind: "Batch", edits: edits}, selectedId, extraIds.slice());
    }
    function deleteClass(name) {
        const edits = [{kind: "SetClass", name: name, styles: null}].concat(
            document.widgets.filter(w => w.class === name).map(w => ({kind: "SetProperty", id: w.element.id, property: {path: "class", value: ""}})));
        structuralEdit({kind: "Batch", edits: edits}, selectedId, extraIds.slice());
    }
    function structuralEdit(edit, select, more) {
        if (!designing || gesture) return false;
        const g = {kind: edit.kind, id: "", original: copy(document), token: null, bound: null, committing: false,
            edit: edit, sent: null, busy: false, released: true, canceled: false, select: select, more: more || []};
        gesture = g;
        forceActiveFocus();
        const backend = app;
        app.invoke("begin_interface_edit", {revision: savedRevision, screen: target || null}, function(token) {
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
    function add(kind, x, y, dropped) {
        const id = uniqueId(kind.toLowerCase());
        const parent = !dropped && widget && !widget.world_actor && containerKinds.indexOf(widget.element.kind) >= 0 ? widget : null;
        const free = !parent || parent.element.kind === "Canvas";
        // Values use the core's tagged representation; default values can be omitted.
        const created = {element: {id: id, kind: kind, content: ["Label","Button","RichText","Toggle"].indexOf(kind) >= 0 ? kind : "", anchor: "TopLeft", offset: free ? [Math.round(x),Math.round(y)] : [0,0], size: [180, kind === "Panel" || kind === "ListView" ? 180 : 40], parent: parent ? parent.element.id : "", modal: false, range: [0,100]}, style: {}, bindings: [], items: []};
        if (structuralEdit({kind: "Create", widget: created}, id)) screenId = "";
    }
    function duplicateSelected() {
        if (!widget || widget.world_actor) return;
        const taken = Object.create(null);
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
    // Reusable components: prefabs saved from a subtree, instanced under a unique prefix.
    function saveComponent(name, update) {
        if (!widget || widget.world_actor || !name) return;
        structuralEdit({kind: "SavePrefab", name: name, root: widget.element.id, update_instances: update}, widget.element.id, []);
    }
    function insertComponent(name) {
        const prefabs = document.prefabs || {};
        const root = (prefabs[name] || []).find(w => !w.element.parent);
        if (!root) return;
        const taken = Object.create(null);
        document.widgets.forEach(w => { taken[w.element.id] = true; });
        let n = 1;
        while (taken["i" + n + "." + root.element.id]) n++;
        const prefix = "i" + n + ".";
        const parent = widget && !widget.world_actor ? widget.element.id : "";
        structuralEdit({kind: "InstantiatePrefab", name: name, prefix: prefix, parent: parent}, prefix + root.element.id, []);
    }
    readonly property var kindIcons: ({
        Panel: "square", Canvas: "square-dashed", VerticalBox: "layout-panel-top", HorizontalBox: "panel-left", Grid: "layout-grid",
        WrapBox: "boxes", SizeBox: "maximize", Spacer: "move", Label: "type", RichText: "file-text", Image: "image",
        Progress: "gauge", RadialProgress: "loader-circle", Tooltip: "message-square", Button: "mouse-pointer-click",
        Toggle: "toggle-left", Slider: "sliders-horizontal", Input: "text-cursor-input", Select: "chevron-down",
        Tabs: "panels-top-left", Scrollbar: "grip-vertical", List: "list", ListView: "list-ordered"})
    function kindIcon(kind) { return kindIcons[kind] || "square"; }
    // Inspector sections start folded when they are rarely needed; the choice is remembered.
    property var folded: ({advanced: true, responsive: true, component: true, animation: true})
    function toggleFold(key) {
        const next = Object.assign({}, folded);
        next[key] = !next[key];
        folded = next;
        try { if (prefs) prefs.folds = JSON.stringify(next); } catch (e) {}
    }
    // A palette tile released over the viewport adds its kind at the pointer.
    function dropKind(kind, item, x, y) {
        const c = viewport.canvasItem;
        const p = c.mapFromItem(item, x, y);
        if (p.x >= 0 && p.y >= 0 && p.x < c.width && p.y < c.height)
            add(kind, Math.round((p.x - safe[0]) / designScale), Math.round((p.y - safe[1]) / designScale), true);
    }
    property string styleStateName: "normal"
    property string animName: ""
    property var overrides: []
    function loadOverrides() {
        const w = widget;
        if (!designing || !w || !w.instance_of) { overrides = []; return; }
        app.invoke("interface_overrides", {id: w.element.id, screen: target || null}, function(list) { root.overrides = list || []; }, function() { root.overrides = []; });
    }
    function describeAction(a) {
        const run = a.run || {};
        if (run.Broadcast) return a.on + ": broadcast " + run.Broadcast.message;
        if (run.PlayAnimation) return a.on + ": play " + run.PlayAnimation.name;
        if (run.StopAnimation) return a.on + ": stop " + run.StopAnimation.name;
        const v = run.SetVariable || {};
        return a.on + ": set " + (v.actor ? v.actor + "." : "") + v.name + " to " + v.value;
    }
    function describeBinding(b) {
        const v = b.source && b.source.Variable;
        return b.property + (b.two_way ? " <-> " : " <- ") + (v ? (v.actor ? v.actor + "." : "") + v.name : "component field");
    }
    function addAction(action) {
        if (!widget) return;
        propertyEdit("actions", (widget.actions || []).concat([action]));
    }
    function removeAction(index) {
        if (!widget) return;
        propertyEdit("actions", (widget.actions || []).filter((a, i) => i !== index));
    }
    // Animations: edited whole through SetAnimation so every change is one undo step.
    readonly property var animatableProps: ["Width","Height","TextSize","CornerRadius","Padding","Value","Opacity","OffsetX","OffsetY","Rotation","Background","TextColor"]
    // Scrubbing shows an animation frozen at a time in the runtime preview; nothing is saved.
    property string animPreviewName: ""
    property real animPreviewTime: 0
    readonly property bool animPreviewing: animPreviewName !== "" && !!(document.animations || {})[animPreviewName]
    function previewAnimation(name, time) {
        cancelEdit();
        animPreviewName = name; animPreviewTime = Math.max(0, time);
        ++revision; frameLayout = null;
        if (designing) previewDelay.restart();
    }
    function stopAnimationPreview() { if (animPreviewName !== "") previewAnimation("", 0); }
    function isColorProp(p) { return p === "Background" || p === "TextColor"; }
    function animationNames() { return Object.keys(document.animations || {}); }
    function saveAnimation(name, animation) {
        structuralEdit({kind: "SetAnimation", name: name, animation: animation}, selectedId, extraIds.slice());
    }
    function newAnimation(name) {
        if (!name || (document.animations || {})[name]) return;
        saveAnimation(name, {duration: 1, looping: false, tracks: []});
    }
    function deleteAnimation(name) {
        if ((document.animations || {})[name]) saveAnimation(name, null);
    }
    function withAnimation(name, change) {
        const current = (document.animations || {})[name];
        if (!current) return;
        const next = copy(current);
        change(next);
        saveAnimation(name, next);
    }
    function addKey(name, widgetId, property, time, rawValue, ease) {
        const value = isColorProp(property) ? String(rawValue) : Number(rawValue);
        withAnimation(name, a => {
            let track = a.tracks.find(t => t.widget === widgetId && t.property === property);
            if (!track) { track = {widget: widgetId, property: property, keys: []}; a.tracks.push(track); }
            track.keys = track.keys.filter(k => k.time !== Number(time));
            track.keys.push({time: Number(time), value: value, ease: ease});
            track.keys.sort((x, y) => x.time - y.time);
            if (Number(time) > a.duration) a.duration = Number(time);
        });
    }
    // Changes one key in place: any of time, value and ease.
    function setKey(name, trackIndex, keyIndex, patch) {
        withAnimation(name, a => {
            const track = a.tracks[trackIndex];
            if (!track || !track.keys[keyIndex]) return;
            const key = track.keys[keyIndex];
            Object.assign(key, patch);
            if (patch.time !== undefined) {
                track.keys = track.keys.filter((other, i) => i === keyIndex || other.time !== key.time);
                track.keys.sort((x, y) => x.time - y.time);
                if (key.time > a.duration) a.duration = key.time;
            }
        });
    }
    // What the selected widget has now for an animatable property, if it says.
    function currentValue(prop) {
        if (!widget) return undefined;
        const st = (widget.style || {})[styleStateName] || {};
        switch (prop) {
        case "Width": return (widget.element.size || [0, 0])[0];
        case "Height": return (widget.element.size || [0, 0])[1];
        case "TextSize": return st.text_size;
        case "CornerRadius": return st.radius;
        case "Padding": return widget.layout && widget.layout.padding ? widget.layout.padding[0] : undefined;
        case "Opacity": return fxValue("opacity");
        case "OffsetX": return fxValue("offset_x");
        case "OffsetY": return fxValue("offset_y");
        case "Rotation": return fxValue("rotation");
        case "Background": return st.background;
        case "TextColor": return st.text_color;
        }
        return undefined;
    }
    function fxValue(key) {
        const fx = widget && widget.fx ? widget.fx : {};
        const offset = fx.offset || [0, 0];
        if (key === "opacity") return fx.opacity === undefined ? 1 : fx.opacity;
        if (key === "offset_x") return offset[0];
        if (key === "offset_y") return offset[1];
        return fx.rotation || 0;
    }
    function setFx(key, value) {
        if (!widget || !Number.isFinite(value)) return;
        const next = {opacity: fxValue("opacity"), offset: [fxValue("offset_x"), fxValue("offset_y")], rotation: fxValue("rotation")};
        if (key === "opacity") next.opacity = Math.max(0, Math.min(1, value));
        else if (key === "offset_x") next.offset[0] = value;
        else if (key === "offset_y") next.offset[1] = value;
        else next.rotation = value;
        propertyEdit("fx", next);
    }
    function removeTrack(name, index) {
        withAnimation(name, a => { a.tracks.splice(index, 1); });
    }
    function removeKey(name, trackIndex, keyIndex) {
        withAnimation(name, a => {
            const track = a.tracks[trackIndex];
            track.keys.splice(keyIndex, 1);
            if (!track.keys.length) a.tracks.splice(trackIndex, 1);
        });
    }
    function describeTrack(t) { return t.widget + " " + t.property + " (" + t.keys.length + " keys)"; }
    function describeVariant(v) {
        const w = v.when || {};
        const parts = [];
        if (w.min_width != null) parts.push("w >= " + w.min_width);
        if (w.max_width != null) parts.push("w <= " + w.max_width);
        if (w.min_height != null) parts.push("h >= " + w.min_height);
        if (w.max_height != null) parts.push("h <= " + w.max_height);
        const does = [];
        if (v.layout) does.push("own layout");
        if (v.visible === true) does.push("shown");
        if (v.visible === false) does.push("hidden");
        return (parts.join(", ") || "any size") + ": " + (does.join(", ") || "no change");
    }
    function addVariant(when, useLayout, visible) {
        if (!widget) return;
        const v = {when: when};
        if (useLayout && widget.layout) v.layout = copy(widget.layout);
        if (visible === true || visible === false) v.visible = visible;
        propertyEdit("variants", (widget.variants || []).concat([v]));
    }
    function removeVariant(index) {
        if (!widget) return;
        propertyEdit("variants", (widget.variants || []).filter((v, i) => i !== index));
    }
    function addBinding(property, actor, name, twoWay) {
        if (!widget || !name) return;
        const next = copy(widget.bindings || []).filter(b => b.property !== property);
        next.push({property: property, source: {Variable: {actor: actor, name: name}}, two_way: !!twoWay});
        propertyEdit("bindings", next);
    }
    function variantActive(v) {
        const w = v.when || {};
        return (w.min_width == null || previewWidth >= w.min_width) && (w.max_width == null || previewWidth <= w.max_width)
            && (w.min_height == null || previewHeight >= w.min_height) && (w.max_height == null || previewHeight <= w.max_height);
    }
    function removeBinding(index) {
        if (!widget) return;
        propertyEdit("bindings", (widget.bindings || []).filter((b, i) => i !== index));
    }
    // Exposed properties of the selected instance: [{name, value}] read from its widgets.
    function exposedRows() {
        if (!widget || !widget.instance_of) return [];
        const prefab = (document.prefabs || {})[widget.instance_of] || [];
        const rootId = (prefab.find(w => !w.element.parent) || {element: {id: ""}}).element.id;
        const id = widget.element.id;
        if (!rootId || id.slice(id.length - rootId.length) !== rootId) return [];
        const prefix = id.slice(0, id.length - rootId.length);
        return ((document.exports || {})[widget.instance_of] || []).map(e => {
            const target = document.widgets.find(w => w.element.id === prefix + e.widget);
            let value = target;
            (e.path ? e.path.split(".") : []).forEach(step => { value = value === undefined || value === null ? undefined : value[step]; });
            return {name: e.name, slot: !!e.slot, value: e.slot ? "" : (value === undefined || value === null ? "" : value)};
        });
    }
    function setExposed(name, text) {
        if (!widget) return;
        let value = text;
        const current = exposedRows().find(r => r.name === name);
        if (current && typeof current.value !== "string") { try { value = JSON.parse(text); } catch (e) { error = String(e); return; } }
        structuralEdit({kind: "SetExposed", instance: widget.element.id, name: name, value: value}, widget.element.id, []);
    }
    function fillSlot(slot, widgetId) {
        if (!widget || !widgetId) return;
        structuralEdit({kind: "FillSlot", instance: widget.element.id, slot: slot, widget: widgetId}, widgetId, []);
    }
    function exposeProperty(prefab, name, path) {
        if (!widget) return;
        structuralEdit({kind: "ExposeProperty", prefab: prefab, name: name, widget: widget.element.id, path: path}, widget.element.id, []);
    }
    function exposeSlot(prefab, name) {
        if (!widget) return;
        structuralEdit({kind: "ExposeSlot", prefab: prefab, name: name, widget: widget.element.id}, widget.element.id, []);
    }
    function resetInstance() {
        if (widget && widget.instance_of) structuralEdit({kind: "ResetInstance", id: widget.element.id}, widget.element.id, []);
    }
    function detachInstance() {
        if (widget && widget.instance_of) structuralEdit({kind: "DetachInstance", id: widget.element.id}, widget.element.id, []);
    }
    // Align or distribute the selection inside its shared parent, as one Batch of Moves.
    function arrange(op) {
        const ids = topLevelSelection();
        const widgets = ids.map(id => widgetsById[id]);
        const need = op === "hdist" || op === "vdist" ? 3 : 2;
        if (!layoutReady || widgets.length < need) return false;
        const parentId = widgets[0].element.parent || "";
        if (!widgets.every(w => editable(w) && (w.element.parent || "") === parentId)) { error = "Select free-placed widgets that share a parent."; return false; }
        const parent = boundsById[parentId] || null;
        const frame = parent ? parent.transform.slice() : [designScale,0,0,designScale,safe[0],safe[1]];
        const inverse = geometry.inverse(frame);
        const boxes = Object.create(null);
        for (const id of ids) {
            const b = boundsById[id];
            if (!b || !b.visible || !inverse) { error = "Wait for matching geometry before arranging."; return false; }
            const box = editMath.alignedBox(b, inverse);
            if (!box) { error = "Rotated widgets cannot be aligned."; return false; }
            boxes[id] = box;
        }
        const result = editMath.arrangeDeltas(op, ids, boxes);
        const edits = widgets.filter(w => Math.abs(result.delta[w.element.id]) > 0.005).map(w => {
            const offset = (w.element.offset || [0,0]).slice();
            offset[result.axis] = Math.round((offset[result.axis] + result.delta[w.element.id]) * 100) / 100;
            return {kind: "Move", id: w.element.id, offset: offset};
        });
        if (!edits.length) return true;
        return structuralEdit(edits.length === 1 ? edits[0] : {kind: "Batch", edits: edits}, selectedId, extraIds.slice());
    }
    function renameWidget(id, newId) {
        const next = (newId || "").trim();
        if (!id || !next || next === id) return false;
        if (widgetsById[next]) { error = "A widget named " + next + " already exists."; return false; }
        const was = extraIds.slice();
        return structuralEdit({kind: "Rename", id: id, new_id: next}, selectedId === id ? next : selectedId, was.map(x => x === id ? next : x));
    }
    // `entry` is one line of instance_overrides: "widget id: path".
    function resetOverride(entry) {
        if (!widget || !widget.instance_of) return false;
        const at = entry.lastIndexOf(": ");
        if (at < 0) return false;
        const target = entry.slice(0, at), path = entry.slice(at + 2);
        if (path === "added" || path === "removed") { error = "Use Reset to component or Detach for added and removed widgets."; return false; }
        return structuralEdit({kind: "ResetField", instance: widget.element.id, widget: target, path: path}, selectedId, extraIds.slice());
    }
    function removeSelected() {
        if (!widget) return;
        const edits = topLevelSelection().map(id => ({kind: "Delete", id: id}));
        structuralEdit(edits.length === 1 ? edits[0] : {kind: "Batch", edits: edits}, "");
    }    // ── Layout: palette and tree, viewport, inspector ──────────────────────
    property real leftWidth: 230
    property real rightWidth: 310
    readonly property bool hasSelection: selectionIds.length > 0
    readonly property bool canEdit: designing && !gesture
    RowLayout {
        anchors.fill: parent; spacing: 0

        Rectangle {
            Layout.preferredWidth: root.leftWidth; Layout.minimumWidth: 180; Layout.maximumWidth: 380
            Layout.fillHeight: true
            color: Theme.panel
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 10; spacing: 6
                SectionLabel { label: "Widgets"; Layout.fillWidth: true; topPadding: 0 }
                Flickable {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.min(palette.implicitHeight, 300)
                    contentHeight: palette.implicitHeight; contentWidth: width
                    clip: true; boundsBehavior: Flickable.StopAtBounds
                    UiPalette { id: palette; designer: root; width: parent.width }
                }
                SectionLabel { label: "Hierarchy"; Layout.fillWidth: true }
                UiHierarchy { objectName: "interfaceHierarchy"; designer: root; Layout.fillWidth: true; Layout.fillHeight: true }
            }
        }
        Rectangle {
            Layout.preferredWidth: 5; Layout.fillHeight: true; color: leftSplit.containsMouse || leftSplit.pressed ? Theme.accent : Theme.borderSoft
            MouseArea {
                id: leftSplit
                anchors.fill: parent; anchors.leftMargin: -3; anchors.rightMargin: -3
                hoverEnabled: true; cursorShape: Qt.SplitHCursor
                property real startX: 0
                property real startWidth: 0
                onPressed: mouse => { startX = mapToItem(root, mouse.x, 0).x; startWidth = root.leftWidth; }
                onPositionChanged: mouse => { if (pressed) root.leftWidth = Math.max(180, Math.min(380, startWidth + mapToItem(root, mouse.x, 0).x - startX)); }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumWidth: 280
            spacing: 0
            Rectangle {
                Layout.fillWidth: true; Layout.preferredHeight: toolbar.height + 16
                color: Theme.panel
                UiToolbar { id: toolbar; objectName: "interfaceToolbar"; designer: root; x: 10; y: 8; width: parent.width - 20 }
            }
            Rectangle {
                visible: root.hasSelection
                Layout.fillWidth: true; Layout.preferredHeight: visible ? selectionBar.height + 10 : 0
                color: Theme.window
                Flow {
                    id: selectionBar
                    x: 10; y: 5; width: parent.width - 20; spacing: 6
                    Text { text: root.selectionIds.length > 1 ? root.selectionIds.length + " selected" : root.selectedId; color: Theme.textDim; font.pixelSize: 12; height: 28; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight; width: Math.min(implicitWidth, 160) }
                    IconButton { objectName: "interfaceDuplicate"; iconName: "copy"; tip: "Duplicate"; implicitWidth: 28; implicitHeight: 28; enabled: root.canEdit && !!root.widget && !root.widget.world_actor; onClicked: root.duplicateSelected() }
                    IconButton { objectName: "interfaceDelete"; iconName: "trash"; tip: "Delete (Delete key)"; implicitWidth: 28; implicitHeight: 28; enabled: root.canEdit && !!root.widget; onClicked: root.removeSelected() }
                    IconButton { objectName: "interfaceEarlier"; iconName: "chevron-left"; tip: "Move earlier in sibling order (drawn below, flows first)"; implicitWidth: 28; implicitHeight: 28; enabled: root.canEdit && root.siblingIndex > 0; onClicked: root.reorderSibling(-1) }
                    IconButton { objectName: "interfaceLater"; iconName: "chevron-right"; tip: "Move later in sibling order (drawn above, flows last)"; implicitWidth: 28; implicitHeight: 28; enabled: root.canEdit && root.siblingIndex >= 0 && root.siblingIndex + 1 < root.siblings.length; onClicked: root.reorderSibling(1) }
                    Repeater {
                        model: [["Left","left"],["Centre","hcenter"],["Right","right"],["Top","top"],["Middle","vcenter"],["Bottom","bottom"],["Space across","hdist"],["Space down","vdist"]]
                        delegate: StitchButton {
                            required property var modelData
                            objectName: "interfaceArrange_" + modelData[1]
                            text: modelData[0]; implicitHeight: 28; font.pixelSize: 11
                            visible: root.selectionIds.length > 1
                            enabled: root.canEdit && root.selectionIds.length > 1
                            ToolTip.visible: hovered; ToolTip.text: modelData[1].indexOf("dist") >= 0 ? "Even gaps (3 or more)" : "Align " + modelData[0].toLowerCase()
                            onClicked: root.arrange(modelData[1])
                        }
                    }
                }
            }
            UiViewport { id: viewport; objectName: "interfaceViewport"; designer: root; Layout.fillWidth: true; Layout.fillHeight: true }
            Rectangle {
                visible: root.error !== ""
                Layout.fillWidth: true; Layout.preferredHeight: visible ? errorText.implicitHeight + 14 : 0
                color: "#3a1d1d"; border.color: Theme.danger
                RowLayout {
                    anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 6
                    Text { id: errorText; objectName: "interfaceError"; Layout.fillWidth: true; text: root.error; color: "#ff9b9b"; font.pixelSize: 12; wrapMode: Text.Wrap }
                    IconButton { iconName: "x"; tip: "Dismiss"; implicitWidth: 24; implicitHeight: 24; onClicked: root.error = "" }
                }
            }
            Text {
                visible: root.error === ""
                Layout.fillWidth: true; Layout.margins: 6; wrapMode: Text.Wrap
                color: Theme.textDim; font.pixelSize: 11
                text: "Drag to move or resize. Arrows nudge 1 px, Shift+arrows 10 px. Hold Shift to bypass snapping. Escape cancels."
            }
        }

        Rectangle {
            Layout.preferredWidth: 5; Layout.fillHeight: true; color: rightSplit.containsMouse || rightSplit.pressed ? Theme.accent : Theme.borderSoft
            MouseArea {
                id: rightSplit
                anchors.fill: parent; anchors.leftMargin: -3; anchors.rightMargin: -3
                hoverEnabled: true; cursorShape: Qt.SplitHCursor
                property real startX: 0
                property real startWidth: 0
                onPressed: mouse => { startX = mapToItem(root, mouse.x, 0).x; startWidth = root.rightWidth; }
                onPositionChanged: mouse => { if (pressed) root.rightWidth = Math.max(240, Math.min(480, startWidth - (mapToItem(root, mouse.x, 0).x - startX))); }
            }
        }
        Rectangle {
            Layout.preferredWidth: root.rightWidth; Layout.minimumWidth: 240; Layout.maximumWidth: 480
            Layout.fillHeight: true
            color: Theme.panel
            UiInspector { objectName: "interfaceInspector"; anchors.fill: parent; designer: root }
        }
    }
}
