import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

Item {
    id: root
    required property var app
    property var document: ({widgets: [], styles: {}, prefabs: {}, reference_size: [960,720], safe_area: [0,0,0,0], theme: "Dark", scale: "ConstantPixel"})
    property int selected: -1
    property string error: ""
    readonly property var widget: selected >= 0 && selected < document.widgets.length ? document.widgets[selected] : null
    readonly property var kinds: ["Panel","Label","Button","Image","Input","Slider","Toggle","List","VerticalBox","HorizontalBox","Grid","Canvas","WrapBox","SizeBox","Spacer","Progress","RadialProgress","ListView","Tabs","Select","Scrollbar","RichText","Tooltip"]
    property int previewWidth: 960
    property int previewHeight: 720
    property real zoom: Math.min((stage.width - 32) / previewWidth, (stage.height - 32) / previewHeight)
    function copy(v) { return JSON.parse(JSON.stringify(v)); }
    function refresh() {
        const saved = app.appState.project ? app.appState.project.world.interface : null;
        if (saved) document = copy(saved);
        if (selected >= document.widgets.length) selected = -1;
    }
    Component.onCompleted: refresh()
    Connections { target: root.app; function onAppStateChanged() { root.refresh(); } }
    function save(next) {
        document = next;
        app.invoke("set_interface", {document: next}, function() { root.error = ""; }, function(e) { root.error = String(e); root.refresh(); });
    }
    function change(field, value) {
        if (!widget) return;
        const next = copy(document); next.widgets[selected].element[field] = value; save(next);
    }
    function extra(field, value) {
        if (!widget) return;
        const next = copy(document); next.widgets[selected][field] = value; save(next);
    }
    function add(kind, x, y) {
        const next = copy(document);
        let id = kind.toLowerCase(), n = 1;
        while (next.widgets.some(w => w.element.id === id + n)) ++n;
        next.widgets.push({element: {id: id+n, kind: kind, content: ["Label","Button","RichText","Toggle"].indexOf(kind) >= 0 ? kind : "", anchor: "TopLeft", offset: [Math.round(x),Math.round(y)], size: [180, kind === "Panel" || kind === "ListView" ? 180 : 40], parent: "", modal: false, range: [0,100], value: {Number: 0}}, style: {}, bindings: [], items: []});
        // Values use the core's tagged representation; default values can be omitted.
        delete next.widgets[next.widgets.length-1].element.value;
        selected = next.widgets.length-1; save(next);
    }
    function removeSelected() {
        if (!widget) return;
        const next = copy(document); let gone = [widget.element.id];
        for (let i=0;i<gone.length;++i) next.widgets.forEach(w => { if(w.element.parent === gone[i] && gone.indexOf(w.element.id)<0) gone.push(w.element.id); });
        next.widgets = next.widgets.filter(w => gone.indexOf(w.element.id)<0); selected = -1; save(next);
    }
    function rect(w, depth) {
        if (depth > 40) return {x:0,y:0,w:100,h:40};
        const e=w.element, parent = document.widgets.find(w => w.element.id === e.parent);
        const p=parent ? rect(parent,depth+1) : {x:0,y:0,w:previewWidth,h:previewHeight};
        let width = e.size && e.size[0] || (parent ? p.w-24 : 180), height=e.size && e.size[1] || 36;
        const l=w.layout || {};
        if(l.width && l.width.Percent !== undefined) width=p.w*l.width.Percent/100;
        if(l.width && l.width.Px !== undefined) width=l.width.Px;
        if(l.height && l.height.Percent !== undefined) height=p.h*l.height.Percent/100;
        if(l.height && l.height.Px !== undefined) height=l.height.Px;
        let x=p.x+(e.offset ? e.offset[0] : 0), y=p.y+(e.offset ? e.offset[1] : 0);
        if(parent && parent.element.kind !== "Canvas" && !l.absolute) {
            const siblings=document.widgets.filter(w=>w.element.parent===e.parent), i=siblings.indexOf(w);
            const index=siblings.findIndex(w=>w.element.id===e.id), gap=(parent.layout || {}).gap || 8;
            x=p.x+12; y=p.y+12;
            if(parent.element.kind === "HorizontalBox" || parent.element.kind === "Tabs") x+=index*(width+gap);
            else if(parent.element.kind === "Grid" || parent.element.kind === "WrapBox") {
                const columns=(parent.layout || {}).columns || Math.max(1,Math.floor(p.w/(width+gap)));
                x+=(index%columns)*(width+gap); y+=Math.floor(index/columns)*(height+gap);
            } else y+=index*(height+gap);
        } else {
            const anchors={TopLeft:[0,0],Top:[0.5,0],TopRight:[1,0],Left:[0,0.5],Center:[0.5,0.5],Right:[1,0.5],BottomLeft:[0,1],Bottom:[0.5,1],BottomRight:[1,1]};
            const a=anchors[e.anchor || "Center"]; x+=a[0]*(p.w-width); y+=a[1]*(p.h-height);
        }
        return {x:x,y:y,w:width,h:height};
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
                                    if (p.x>=0 && p.y>=0 && p.x<canvas.width && p.y<canvas.height) root.add(parent.modelData,p.x,p.y);
                                }
                            }
                        }
                    }
                }
            }
            Label { text: "Hierarchy"; font.bold: true; padding: 8 }
            ListView {
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                model: root.document.widgets
                delegate: ItemDelegate {
                    required property var modelData
                    required property int index
                    width: ListView.view.width; height: 30
                    text: (modelData.element.parent ? "    " : "")+modelData.element.id
                    highlighted: root.selected === index
                    onClicked: root.selected=index
                }
            }
            Button { text: "Delete widget"; enabled: !!root.widget; onClicked: root.removeSelected() }
        }
        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true
            RowLayout {
                ComboBox { model: ["960 × 720","1280 × 720","1920 × 1080","720 × 1280"]; onActivated: { const sizes=[[960,720],[1280,720],[1920,1080],[720,1280]]; root.previewWidth=sizes[currentIndex][0]; root.previewHeight=sizes[currentIndex][1]; } }
                ComboBox { model: ["Dark","Light","HighContrast"]; currentIndex: model.indexOf(root.document.theme || "Dark"); onActivated: { const d=root.copy(root.document); d.theme=currentText; root.save(d); } }
                ComboBox { model: ["ConstantPixel","ScaleWithSize"]; currentIndex: model.indexOf(root.document.scale || "ConstantPixel"); onActivated: { const d=root.copy(root.document); d.scale=currentText; root.save(d); } }
                Item { Layout.fillWidth: true }
            }
            Rectangle {
                id: stage; Layout.fillWidth: true; Layout.fillHeight: true; color: "#171a21"; clip: true
                Rectangle {
                    id: canvas; anchors.centerIn: parent
                    width: root.previewWidth; height: root.previewHeight
                    scale: Math.max(0.05,root.zoom); color: root.document.theme === "Light" ? "#d9dfe8" : "#252d3a"
                    Repeater {
                        model: root.document.widgets
                        delegate: Rectangle {
                            id: tile
                            required property var modelData
                            required property int index
                            readonly property var bounds: root.rect(modelData,0)
                            x: bounds.x; y: bounds.y; width: bounds.w; height: bounds.h
                            color: ((modelData.style || {}).normal || {}).background || (modelData.element.kind === "Label" ? "transparent" : "#39465a")
                            radius: ((modelData.style || {}).normal || {}).radius || 4
                            border.width: root.selected === index ? 2 : 1
                            border.color: root.selected === index ? "#70baff" : "#65738a"
                            Text { anchors.fill: parent; anchors.margins: 6; text: tile.modelData.element.content || tile.modelData.element.kind; color: ((tile.modelData.style || {}).normal || {}).text_color || "white"; font.pixelSize: ((tile.modelData.style || {}).normal || {}).text_size || 16; wrapMode: Text.Wrap; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
                            MouseArea {
                                anchors.fill: parent
                                property point start
                                onPressed: mouse => { root.selected=tile.index; start=Qt.point(mouse.x,mouse.y); }
                                onReleased: mouse => {
                                    const dx=mouse.x-start.x, dy=mouse.y-start.y;
                                    if(Math.abs(dx)+Math.abs(dy)>2) { const offset=tile.modelData.element.offset || [0,0]; root.change("offset",[Math.round(offset[0]+dx),Math.round(offset[1]+dy)]); }
                                }
                            }
                        }
                    }
                }
            }
            Label { text: root.error || "Drag widgets from the palette. Play to preview the runtime layout and interactions."; color: root.error ? "#ff8888" : Theme.textDim; wrapMode: Text.Wrap; Layout.fillWidth: true }
        }
        ScrollView {
            Layout.preferredWidth: 245; Layout.fillHeight: true
            ColumnLayout {
                width: 225; spacing: 8
                Label { text: "Widget inspector"; font.bold: true }
                Label { text: root.widget ? root.widget.element.id : "Select a widget" }
                ComboBox { Layout.fillWidth: true; model: root.kinds; currentIndex: root.widget ? root.kinds.indexOf(root.widget.element.kind) : -1; enabled: !!root.widget; onActivated: root.change("kind",currentText) }
                TextField { Layout.fillWidth: true; placeholderText: "Text or image asset"; text: root.widget ? root.widget.element.content : ""; enabled: !!root.widget; onEditingFinished: root.change("content",text) }
                ComboBox { Layout.fillWidth: true; model: [""].concat(root.document.widgets.filter(w=>!root.widget || w.element.id!==root.widget.element.id).map(w=>w.element.id)); currentIndex: root.widget ? model.indexOf(root.widget.element.parent || "") : 0; enabled: !!root.widget; onActivated: root.change("parent",currentText) }
                ComboBox { Layout.fillWidth: true; model: ["TopLeft","Top","TopRight","Left","Center","Right","BottomLeft","Bottom","BottomRight"]; currentIndex: root.widget ? model.indexOf(root.widget.element.anchor || "Center") : 0; onActivated: root.change("anchor",currentText) }
                Repeater {
                    model: ["X","Y","Width","Height"]
                    delegate: RowLayout {
                        required property string modelData
                        required property int index
                        Label { text: modelData; Layout.preferredWidth: 55 }
                        TextField { Layout.fillWidth: true; text: root.widget ? (index<2 ? root.widget.element.offset || [0,0] : root.widget.element.size || [0,0])[index%2] : "0"; validator: DoubleValidator {}
                            onEditingFinished: { if(!root.widget) return; const field=index<2?"offset":"size", pair=(root.widget.element[field] || [0,0]).slice(); pair[index%2]=Number(text); root.change(field,pair); } }
                    }
                }
                CheckBox { text: "Modal"; checked: root.widget ? root.widget.element.modal === true : false; onToggled: root.change("modal",checked) }
                TextField { Layout.fillWidth: true; placeholderText: "Tooltip"; text: root.widget ? root.widget.tooltip || "" : ""; onEditingFinished: root.extra("tooltip",text) }
                TextField { Layout.fillWidth: true; placeholderText: "World actor id"; text: root.widget ? root.widget.world_actor || "" : ""; onEditingFinished: root.extra("world_actor",text) }
                Label { text: "Items (one per line)" }
                TextArea { Layout.fillWidth: true; Layout.preferredHeight: 70; text: root.widget ? (root.widget.items || []).join("\n") : ""; onActiveFocusChanged: if(!activeFocus && root.widget) root.extra("items",text ? text.split("\n") : []) }
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
                Label { text: "Reusable menus"; font.bold: true }
                TextField { id: prefabName; Layout.fillWidth: true; placeholderText: "Prefab name" }
                Button { text: "Save interface as prefab"; enabled: !!prefabName.text; onClicked: { const d=root.copy(root.document); if(!d.prefabs)d.prefabs={}; d.prefabs[prefabName.text]=root.copy(d.widgets); root.save(d); } }
                ComboBox { id: prefab; Layout.fillWidth: true; model: Object.keys(root.document.prefabs || {}) }
                Button { text: "Insert prefab"; enabled: prefab.currentIndex>=0; onClicked: {
                    const d=root.copy(root.document), prefix="copy"+Date.now()+"_";
                    const widgets=root.copy(d.prefabs[prefab.currentText]); widgets.forEach(w=>{w.element.id=prefix+w.element.id;if(w.element.parent)w.element.parent=prefix+w.element.parent;});
                    d.widgets=d.widgets.concat(widgets);root.save(d);
                } }
            }
        }
    }
}
