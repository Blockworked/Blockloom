import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// The selected widget in foldable sections, or the document when nothing is
// selected. Every edit goes through the designer's typed edits.
ScrollView {
    id: root
    required property var designer
    readonly property var d: designer
    readonly property var w: d.widget
    readonly property bool has: !!w
    readonly property bool busy: !!d.gesture
    clip: true
    contentWidth: availableWidth
    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

    function styleOf(field) { return w ? (((w.style || {})[d.styleStateName] || {})[field]) : undefined; }
    function ownText(field) { const v = styleOf(field); return v === undefined || v === null ? "" : String(v); }
    function actorVariables(actorId) {
        const p = d.app.appState.project;
        if (!p) return [];
        const names = list => (list || []).map(v => v.name);
        if (!actorId) return names(p.globals);
        const a = (p.actors || []).find(x => x.id === actorId);
        return a ? names(a.variables) : [];
    }
    readonly property var actorOptions: {
        const p = d.app.appState.project;
        return [{value: "", label: "Global"}].concat(p ? (p.actors || []).map(a => ({value: a.id, label: a.name})) : []);
    }

    ColumnLayout {
        width: root.availableWidth
        spacing: 0

        // ── Nothing selected: the document ───────────────────────────────
        ColumnLayout {
            visible: !root.has
            Layout.fillWidth: true; Layout.margins: 10; spacing: 6
            Text { text: "Nothing selected"; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
            Text {
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
                text: "Click a widget in the viewport or the tree to edit it. Drag out of the palette to add one."
            }
        }
        UiSection {
            designer: root.d; title: "Document"; visible: !root.has
            InspectorRow {
                label: "Theme"; Layout.fillWidth: true
                ChoiceField {
                    options: ["Dark", "Light", "HighContrast"].map(v => ({value: v, label: v}))
                    value: root.d.document.theme || "Dark"
                    onChosen: v => root.d.structuralEdit({kind: "SetDocument", theme: v}, root.d.selectedId, root.d.extraIds.slice())
                }
            }
            InspectorRow {
                label: "Scaling"; Layout.fillWidth: true
                ChoiceField {
                    options: [{value: "ConstantPixel", label: "Constant pixel size"}, {value: "ScaleWithSize", label: "Scale with window"}]
                    value: root.d.document.scale || "ConstantPixel"
                    onChosen: v => root.d.structuralEdit({kind: "SetDocument", scale: v}, root.d.selectedId, root.d.extraIds.slice())
                }
            }
            Text {
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
                text: "Reference size " + (root.d.document.reference_size || [960, 720]).join(" x ") + ". Safe area " + root.d.safe.join(", ") + " (left, top, right, bottom)."
            }
        }
        UiSection {
            designer: root.d; title: "Interface files"; visible: !root.has
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchTextField { id: assetName; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "menu.json" }
                StitchButton { text: "Save as asset"; implicitHeight: 30; font.pixelSize: 12; enabled: !!assetName.text; onClicked: root.d.app.invoke("save_interface_asset", {name: assetName.text}, function() { root.d.error = ""; }, function(e) { root.d.error = String(e); }) }
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchTextField { id: assetPath; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "assets/ui/menu.json" }
                StitchButton { text: "Load"; implicitHeight: 30; font.pixelSize: 12; enabled: !!assetPath.text; onClicked: root.d.app.invoke("load_interface_asset", {path: assetPath.text}, function() { root.d.error = ""; }, function(e) { root.d.error = String(e); }) }
            }
        }

        // ── A widget ─────────────────────────────────────────────────────
        RowLayout {
            visible: root.has
            Layout.fillWidth: true; Layout.margins: 10; spacing: 8
            LucideIcon { name: root.w ? root.d.kindIcon(root.w.element.kind) : "square"; color: Theme.text; width: 18; height: 18 }
            ColumnLayout {
                Layout.fillWidth: true; spacing: 0
                Text { Layout.fillWidth: true; elide: Text.ElideRight; text: root.w ? root.w.element.id : ""; color: Theme.text; font.pixelSize: 13; font.weight: Font.DemiBold }
                Text {
                    visible: root.d.selectionIds.length > 1
                    text: root.d.selectionIds.length + " selected"; color: Theme.textDim; font.pixelSize: 11
                }
            }
            StitchComboBox {
                objectName: "interfaceKind"
                implicitWidth: 116; implicitHeight: 28; font.pixelSize: 12
                model: root.d.kinds
                currentIndex: root.w ? root.d.kinds.indexOf(root.w.element.kind) : -1
                enabled: root.has && !root.busy
                onActivated: root.d.change("kind", currentText)
                Accessible.name: "Widget kind"
            }
        }

        UiSection {
            designer: root.d; title: "Layout"; visible: root.has
            InspectorRow {
                label: "Parent"; Layout.fillWidth: true
                StitchComboBox {
                    Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                    model: [""].concat(root.d.document.widgets.filter(x => !x.world_actor && !root.d.descendant(x.element.id, root.d.selectedId)).map(x => x.element.id))
                    displayText: currentText === "" ? "(none)" : currentText
                    currentIndex: root.w ? model.indexOf(root.w.element.parent || "") : 0
                    enabled: root.has && !root.w.world_actor && !root.busy
                    onActivated: root.d.reparent(currentText)
                    Accessible.name: "Parent widget"
                }
            }
            InspectorRow {
                label: "Anchor"; Layout.fillWidth: true
                StitchComboBox {
                    Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                    model: ["TopLeft","Top","TopRight","Left","Center","Right","BottomLeft","Bottom","BottomRight"]
                    currentIndex: root.w ? model.indexOf(root.w.element.anchor || "Center") : 0
                    enabled: root.has && !root.busy
                    onActivated: root.d.change("anchor", currentText)
                    Accessible.name: "Anchor"
                }
            }
            Text {
                visible: root.has && !root.d.editable(root.w)
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.warning; font.pixelSize: 11
                text: "This widget is placed by its parent's layout. Position and size come from the layout settings below."
            }
            GridLayout {
                columns: 2; columnSpacing: 8; rowSpacing: 6; Layout.fillWidth: true
                Repeater {
                    model: ["X", "Y", "Width", "Height"]
                    delegate: InspectorRow {
                        id: dimRow
                        required property string modelData
                        required property int index
                        label: modelData; labelWidth: 42; Layout.fillWidth: true
                        NumberField {
                            objectName: "interfaceDim" + dimRow.modelData
                            value: root.w ? (index < 2 ? root.w.element.offset || [0,0] : root.w.element.size || [0,0])[index % 2] : 0
                            enabled: root.has && root.d.editable(root.w) && !root.busy
                            onCommitted: n => root.d.editDimension(index, n)
                        }
                    }
                }
            }
            UiLayoutInspector {
                Layout.fillWidth: true
                enabled: root.has && !root.busy
                kind: root.w && root.w.element.kind ? root.w.element.kind : ""
                layoutValue: root.w ? root.w.layout || null : null
                onEdited: value => root.d.propertyEdit("layout", value)
            }
            StitchCheckBox {
                visible: root.d.applies("element.modal")
                text: "Modal (blocks clicks behind it)"
                checked: root.w ? root.w.element.modal === true : false
                onToggled: root.d.change("modal", checked)
            }
        }

        UiSection {
            designer: root.d; title: "Content"; visible: root.has
            InspectorRow {
                label: root.w && root.w.element.kind === "Image" ? "Image" : "Text"; Layout.fillWidth: true
                StitchTextField {
                    visible: !(root.w && root.w.element.kind === "Image")
                    Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                    text: root.w ? root.w.element.content || "" : ""
                    enabled: root.has && !root.busy
                    onEditingFinished: if (root.w && text !== (root.w.element.content || "")) root.d.change("content", text)
                }
                AssetField {
                    visible: root.w && root.w.element.kind === "Image"
                    app: root.d.app; accept: ["image"]
                    value: root.w ? root.w.element.content || "" : ""
                    placeholderText: "Drag an image here"
                    onCommitted: p => root.d.change("content", p)
                }
            }
            InspectorRow {
                visible: root.d.applies("element.range"); label: "Range"; Layout.fillWidth: true
                NumberField { value: root.w ? (root.w.element.range || [0, 100])[0] : 0; onCommitted: n => root.d.propertyEdit("element.range", [n, (root.w.element.range || [0, 100])[1]]) }
                NumberField { value: root.w ? (root.w.element.range || [0, 100])[1] : 100; onCommitted: n => root.d.propertyEdit("element.range", [(root.w.element.range || [0, 100])[0], n]) }
            }
            ColumnLayout {
                visible: root.d.applies("items"); Layout.fillWidth: true; spacing: 2
                Text { text: "Items, one per line"; color: Theme.textDim; font.pixelSize: 11 }
                StitchTextField {
                    id: itemsField
                    Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                    placeholderText: "Separate items with a comma"
                    text: root.w ? (root.w.items || []).join(", ") : ""
                    onEditingFinished: {
                        const next = text.trim() === "" ? [] : text.split(",").map(s => s.trim()).filter(s => s !== "");
                        if (root.w && JSON.stringify(next) !== JSON.stringify(root.w.items || [])) root.d.propertyEdit("items", next);
                    }
                }
            }
            InspectorRow {
                label: "Tooltip"; Layout.fillWidth: true
                StitchTextField {
                    Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                    text: root.w ? root.w.tooltip || "" : ""
                    onEditingFinished: if (root.w && text !== (root.w.tooltip || "")) root.d.extra("tooltip", text)
                }
            }
            InspectorRow {
                label: "World actor"; Layout.fillWidth: true
                ChoiceField {
                    options: root.actorOptions.map(o => o.value === "" ? {value: "", label: "None"} : o)
                    value: root.w ? root.w.world_actor || "" : ""
                    onChosen: v => root.d.extra("world_actor", v)
                }
            }
            InspectorRow {
                visible: root.d.applies("scroll_target"); label: "Scrolls"; Layout.fillWidth: true
                ChoiceField {
                    options: [{value: "", label: "Nothing"}].concat(root.d.document.widgets.map(x => ({value: x.element.id, label: x.element.id})))
                    value: root.w ? root.w.scroll_target || "" : ""
                    onChosen: v => root.d.extra("scroll_target", v)
                }
            }
            InspectorRow {
                label: "Tab page"; Layout.fillWidth: true
                NumberField {
                    allowEmpty: true
                    value: root.w && root.w.tab_index !== undefined ? root.w.tab_index : null
                    onCommitted: n => root.d.extra("tab_index", n === null ? null : Math.max(1, Math.round(n)))
                }
            }
        }

        UiSection {
            designer: root.d; title: "Appearance"; visible: root.has
            InspectorRow {
                label: "State"; Layout.fillWidth: true
                ChoiceField {
                    options: ["normal", "hover", "pressed", "disabled", "focused"].map(v => ({value: v, label: v}))
                    value: root.d.styleStateName
                    onChosen: v => root.d.styleStateName = v
                }
            }
            InspectorRow {
                label: "Class"; Layout.fillWidth: true
                ChoiceField {
                    objectName: "interfaceClass"
                    options: [{value: "", label: "None"}].concat(Object.keys(root.d.document.styles || {}).map(v => ({value: v, label: v})))
                    value: root.w ? root.w.class || "" : ""
                    enabled: root.has && !root.busy
                    onChosen: v => root.d.extra("class", v)
                }
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchTextField { id: className; objectName: "interfaceClassName"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "New class name" }
                StitchButton { objectName: "interfaceSaveClass"; text: "Save class"; implicitHeight: 30; font.pixelSize: 12; enabled: root.has && !!className.text.trim() && !root.busy; onClicked: root.d.saveClass(className.text.trim()) }
                IconButton { objectName: "interfaceDeleteClass"; iconName: "trash"; tip: "Delete this widget's class"; implicitWidth: 30; implicitHeight: 30; enabled: root.has && !!root.w.class && !root.busy; onClicked: root.d.deleteClass(root.w.class) }
            }
            Repeater {
                model: [
                    {key: "background", label: "Background"}, {key: "text_color", label: "Text colour"},
                    {key: "border_color", label: "Border colour"}, {key: "shadow", label: "Shadow"}
                ]
                delegate: InspectorRow {
                    id: paint
                    required property var modelData
                    readonly property string own: root.ownText(modelData.key)
                    readonly property string inherited: root.d.inheritedPaint(modelData.key) || ""
                    label: modelData.label; Layout.fillWidth: true
                    ColorField {
                        value: /^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/.test(paint.own || paint.inherited) ? (paint.own || paint.inherited) : "#FFFFFF"
                        opacity: paint.own ? 1 : 0.5
                        onPicked: c => root.d.paint(paint.modelData.key, c)
                    }
                    StitchTextField {
                        Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                        placeholderText: paint.inherited ? "class " + paint.inherited : "#RRGGBB"
                        text: paint.own
                        onEditingFinished: if (text !== paint.own) root.d.paint(paint.modelData.key, text || null)
                    }
                }
            }
            Repeater {
                model: [{key: "text_size", label: "Text size"}, {key: "border_width", label: "Border"}, {key: "radius", label: "Corner radius"}]
                delegate: InspectorRow {
                    id: sizeRow
                    required property var modelData
                    label: modelData.label; Layout.fillWidth: true
                    NumberField {
                        allowEmpty: true
                        value: { const v = root.styleOf(sizeRow.modelData.key); return v === undefined ? null : v; }
                        onCommitted: n => root.d.paint(sizeRow.modelData.key, n === null ? null : Math.max(0, n))
                    }
                }
            }
            InspectorRow {
                visible: root.w && root.w.element.kind === "Image"; label: "Image fit"; Layout.fillWidth: true
                ChoiceField {
                    objectName: "interfaceImageFit"
                    options: ["auto", "stretch", "sliced", "tiled"].map(v => ({value: v, label: v}))
                    value: root.d.imageFit().mode
                    onChosen: v => root.d.setImageFit(v, root.d.imageFit().border)
                }
            }
            RowLayout {
                visible: root.w && root.w.element.kind === "Image" && root.d.imageFit().mode === "sliced"
                Layout.fillWidth: true; spacing: 4
                Repeater {
                    model: ["left", "right", "top", "bottom"]
                    delegate: NumberField {
                        required property string modelData
                        required property int index
                        objectName: "interfaceSlice" + modelData
                        Layout.fillWidth: true
                        value: root.d.imageFit().border[index]
                        onCommitted: n => { const b = root.d.imageFit().border.slice(); b[index] = Math.max(0, n); root.d.setImageFit("sliced", b); }
                    }
                }
            }
            InspectorRow {
                label: "Font"; Layout.fillWidth: true
                AssetField {
                    app: root.d.app; accept: ["font"]
                    value: { const f = root.styleOf("fonts"); return f && f.length ? f[0] : ""; }
                    placeholderText: "Drag a font here"
                    onCommitted: p => root.d.paint("fonts", p ? [p] : [])
                }
            }
            InspectorRow {
                label: "Fade (s)"; Layout.fillWidth: true
                NumberField { value: root.w ? root.w.transition || 0 : 0; onCommitted: n => root.d.extra("transition", Math.max(0, n)) }
            }
            Text { text: "Effects (visual only)"; color: Theme.textDim; font.pixelSize: 11 }
            Repeater {
                model: [
                    {key: "opacity", label: "Opacity", fallback: 1}, {key: "offset_x", label: "Shift X", fallback: 0},
                    {key: "offset_y", label: "Shift Y", fallback: 0}, {key: "rotation", label: "Rotation", fallback: 0}
                ]
                delegate: InspectorRow {
                    id: fxRow
                    required property var modelData
                    label: modelData.label; Layout.fillWidth: true
                    NumberField {
                        fallback: fxRow.modelData.fallback
                        value: root.d.fxValue(fxRow.modelData.key)
                        onCommitted: n => root.d.setFx(fxRow.modelData.key, n)
                    }
                }
            }
        }

        UiSection {
            designer: root.d; title: "Bindings"; visible: root.has
            badge: root.w && root.w.bindings && root.w.bindings.length ? String(root.w.bindings.length) : ""
            Repeater {
                model: root.w ? root.w.bindings || [] : []
                delegate: RowLayout {
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true; spacing: 4
                    Text { Layout.fillWidth: true; elide: Text.ElideRight; text: root.d.describeBinding(modelData); color: Theme.text; font.pixelSize: 12 }
                    IconButton { objectName: "interfaceRemoveBinding" + index; iconName: "x"; tip: "Remove binding"; implicitWidth: 24; implicitHeight: 24; enabled: !root.busy; onClicked: root.d.removeBinding(index) }
                }
            }
            InspectorRow {
                label: "Property"; Layout.fillWidth: true
                StitchComboBox { id: bindingProperty; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; model: ["Text", "Value", "Visible", "SelectedIndex"] }
            }
            InspectorRow {
                label: "Actor"; Layout.fillWidth: true
                ChoiceField { id: bindingActor; options: root.actorOptions; value: ""; onChosen: v => { value = v; } }
            }
            InspectorRow {
                label: "Variable"; Layout.fillWidth: true
                StitchComboBox {
                    id: bindingName
                    objectName: "interfaceBindingVariable"
                    Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                    model: root.actorVariables(bindingActor.value)
                    displayText: currentIndex >= 0 ? currentText : "Choose a variable"
                }
            }
            Text {
                visible: bindingName.count === 0
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
                text: "No variables here yet. Make one in the Code tab first."
            }
            StitchCheckBox { id: bindingWrite; text: "Write input back to the variable" }
            StitchButton {
                objectName: "interfaceAddBinding"
                text: "Bind variable"; implicitHeight: 30; font.pixelSize: 12
                enabled: root.has && bindingName.currentIndex >= 0 && !root.busy
                onClicked: root.d.addBinding(bindingProperty.currentText, bindingActor.value, bindingName.currentText, bindingWrite.checked)
            }
        }

        UiSection {
            designer: root.d; title: "Events"; visible: root.has
            badge: root.w && root.w.actions && root.w.actions.length ? String(root.w.actions.length) : ""
            Repeater {
                model: root.w ? root.w.actions || [] : []
                delegate: RowLayout {
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true; spacing: 4
                    Text { Layout.fillWidth: true; elide: Text.ElideRight; text: root.d.describeAction(modelData); color: Theme.text; font.pixelSize: 12 }
                    IconButton { objectName: "interfaceRemoveAction" + index; iconName: "x"; tip: "Remove event"; implicitWidth: 24; implicitHeight: 24; enabled: !root.busy; onClicked: root.d.removeAction(index) }
                }
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchComboBox { id: actionOn; objectName: "interfaceActionOn"; Layout.preferredWidth: 90; implicitHeight: 30; font.pixelSize: 12; model: ["Click", "Change"] }
                StitchComboBox { id: actionKind; objectName: "interfaceActionKind"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; model: ["Broadcast", "Set variable", "Play animation", "Stop animation"] }
            }
            StitchTextField {
                id: actionMessage; objectName: "interfaceActionMessage"
                visible: actionKind.currentIndex === 0
                Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Message to broadcast"
            }
            StitchComboBox {
                id: actionAnimation
                visible: actionKind.currentIndex >= 2
                Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                model: root.d.animationNames()
                displayText: currentIndex >= 0 ? currentText : "Make an animation first"
            }
            ChoiceField { id: actionActor; visible: actionKind.currentIndex === 1; options: root.actorOptions; value: ""; onChosen: v => { value = v; } }
            StitchComboBox {
                id: actionVariable
                objectName: "interfaceActionVariable"
                visible: actionKind.currentIndex === 1
                Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                model: root.actorVariables(actionActor.value)
                displayText: currentIndex >= 0 ? currentText : "Choose a variable"
            }
            StitchTextField {
                id: actionValue; objectName: "interfaceActionValue"
                visible: actionKind.currentIndex === 1
                Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                placeholderText: "Value ($value is the new value on Change)"
            }
            StitchButton {
                objectName: "interfaceAddAction"; text: "Add event"; implicitHeight: 30; font.pixelSize: 12
                enabled: root.has && !root.busy && (actionKind.currentIndex === 0 ? !!actionMessage.text.trim()
                    : actionKind.currentIndex === 1 ? actionVariable.currentIndex >= 0 : actionAnimation.currentIndex >= 0)
                onClicked: {
                    const text = actionKind.currentIndex === 0 ? actionMessage.text.trim() : actionAnimation.currentText;
                    root.d.addAction({on: actionOn.currentText, run: [
                        {Broadcast: {message: text}},
                        {SetVariable: {actor: actionActor.value, name: actionVariable.currentText, value: actionValue.text}},
                        {PlayAnimation: {name: text}},
                        {StopAnimation: {name: text}}][actionKind.currentIndex]});
                }
            }
        }

        UiSection {
            designer: root.d; title: "Animation"
            badge: root.d.animationNames().length ? String(root.d.animationNames().length) : ""
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchComboBox {
                    id: animationPick; objectName: "interfaceAnimationPick"
                    Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                    model: root.d.animationNames()
                    displayText: currentIndex >= 0 ? currentText : "No animations"
                    onCurrentTextChanged: { root.d.animName = currentIndex >= 0 ? currentText : ""; root.d.stopAnimationPreview(); }
                }
                IconButton { objectName: "interfaceDeleteAnimation"; iconName: "trash"; tip: "Delete animation"; implicitWidth: 30; implicitHeight: 30; enabled: animationPick.currentIndex >= 0 && !root.busy; onClicked: root.d.deleteAnimation(animationPick.currentText) }
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchTextField { id: animationName; objectName: "interfaceAnimationName"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "New animation name" }
                StitchButton { objectName: "interfaceNewAnimation"; text: "New"; implicitHeight: 30; font.pixelSize: 12; enabled: !!animationName.text.trim() && !root.busy; onClicked: { root.d.newAnimation(animationName.text.trim()); animationName.text = ""; } }
            }
            UiTimeline {
                visible: animationPick.currentIndex >= 0
                Layout.fillWidth: true
                designer: root.d
                name: animationPick.currentIndex >= 0 ? animationPick.currentText : ""
            }
            Text {
                visible: root.d.animPreviewing
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
                text: "Previewing: editing the widgets is paused and nothing is saved."
            }
            Text {
                visible: animationPick.currentIndex >= 0
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
                text: "Play it from an event: add a Click event with Play animation."
            }
        }

        UiSection {
            designer: root.d; title: "Responsive"; visible: root.has
            badge: root.w && root.w.variants && root.w.variants.length ? String(root.w.variants.length) : ""
            Text {
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
                text: "Preview " + root.d.previewWidth + " x " + root.d.previewHeight + ". The first matching variant wins."
            }
            Repeater {
                model: root.w ? root.w.variants || [] : []
                delegate: RowLayout {
                    required property var modelData
                    required property int index
                    readonly property bool active: root.d.variantActive(modelData)
                    Layout.fillWidth: true; spacing: 4
                    Rectangle { width: 8; height: 8; radius: 4; color: active ? Theme.accent : Theme.border }
                    Text { Layout.fillWidth: true; elide: Text.ElideRight; text: root.d.describeVariant(modelData); color: Theme.text; font.pixelSize: 12 }
                    IconButton { objectName: "interfaceRemoveVariant" + index; iconName: "x"; tip: "Remove variant"; implicitWidth: 24; implicitHeight: 24; enabled: !root.busy; onClicked: root.d.removeVariant(index) }
                }
            }
            Flow {
                Layout.fillWidth: true; spacing: 4
                Repeater {
                    model: [{label: "Phone", minW: "", maxW: "599"}, {label: "Tablet", minW: "600", maxW: "1199"}, {label: "Desktop", minW: "1200", maxW: ""}]
                    delegate: StitchButton {
                        required property var modelData
                        text: modelData.label; implicitHeight: 26; font.pixelSize: 11
                        onClicked: { varMinW.text = modelData.minW; varMaxW.text = modelData.maxW; varMinH.text = ""; varMaxH.text = ""; }
                    }
                }
            }
            GridLayout {
                columns: 4; Layout.fillWidth: true; columnSpacing: 4
                StitchTextField { id: varMinW; objectName: "interfaceVariantMinWidth"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "min w"; validator: DoubleValidator { bottom: 0 } }
                StitchTextField { id: varMaxW; objectName: "interfaceVariantMaxWidth"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "max w"; validator: DoubleValidator { bottom: 0 } }
                StitchTextField { id: varMinH; objectName: "interfaceVariantMinHeight"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "min h"; validator: DoubleValidator { bottom: 0 } }
                StitchTextField { id: varMaxH; objectName: "interfaceVariantMaxHeight"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "max h"; validator: DoubleValidator { bottom: 0 } }
            }
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchComboBox { id: varVisible; objectName: "interfaceVariantVisible"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; model: ["Unchanged", "Shown", "Hidden"] }
                StitchCheckBox { id: varLayout; objectName: "interfaceVariantLayout"; text: "Use current layout"; enabled: !!(root.w && root.w.layout) }
            }
            StitchButton {
                objectName: "interfaceAddVariant"; text: "Add variant"; implicitHeight: 30; font.pixelSize: 12
                enabled: root.has && !root.busy
                onClicked: {
                    const num = f => f.text.trim() === "" ? null : Number(f.text);
                    const when = {min_width: num(varMinW), max_width: num(varMaxW), min_height: num(varMinH), max_height: num(varMaxH)};
                    root.d.addVariant(when, varLayout.checked, [undefined, true, false][varVisible.currentIndex]);
                }
            }
        }

        UiSection {
            designer: root.d; title: "Component"
            badge: root.w && root.w.instance_of ? root.w.instance_of : ""
            ColumnLayout {
                visible: !!(root.w && root.w.instance_of)
                Layout.fillWidth: true; spacing: 4
                Text { Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.text; font.pixelSize: 12; text: root.w ? "Instance of " + root.w.instance_of : "" }
                Text {
                    visible: root.d.overrides.length === 0
                    color: Theme.textDim; font.pixelSize: 11; text: "Matches its component."
                }
                Repeater {
                    model: root.d.overrides
                    delegate: RowLayout {
                        required property string modelData
                        required property int index
                        Layout.fillWidth: true; spacing: 4
                        Text { Layout.fillWidth: true; elide: Text.ElideRight; text: modelData; color: Theme.warning; font.pixelSize: 11 }
                        IconButton {
                            objectName: "interfaceResetOverride" + index
                            iconName: "rotate-ccw"; tip: "Reset this field to the component"
                            implicitWidth: 24; implicitHeight: 24
                            enabled: !root.busy && modelData.slice(-6) !== ": added" && modelData.slice(-8) !== ": removed"
                            onClicked: root.d.resetOverride(modelData)
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true; spacing: 6
                    StitchButton { objectName: "interfaceResetInstance"; text: "Reset to component"; implicitHeight: 30; font.pixelSize: 12; enabled: !root.busy; onClicked: root.d.resetInstance() }
                    StitchButton { objectName: "interfaceDetachInstance"; text: "Detach"; implicitHeight: 30; font.pixelSize: 12; enabled: !root.busy; onClicked: root.d.detachInstance() }
                }
                Repeater {
                    model: root.d.exposedRows()
                    delegate: InspectorRow {
                        id: exposedRow
                        required property var modelData
                        label: modelData.name; Layout.fillWidth: true
                        StitchTextField {
                            objectName: "interfaceExposed_" + exposedRow.modelData.name
                            visible: !exposedRow.modelData.slot
                            Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                            text: typeof exposedRow.modelData.value === "string" ? exposedRow.modelData.value : JSON.stringify(exposedRow.modelData.value)
                            onEditingFinished: {
                                const current = typeof exposedRow.modelData.value === "string" ? exposedRow.modelData.value : JSON.stringify(exposedRow.modelData.value);
                                if (text !== current) root.d.setExposed(exposedRow.modelData.name, text);
                            }
                        }
                        StitchButton {
                            objectName: "interfaceFillSlot_" + exposedRow.modelData.name
                            visible: exposedRow.modelData.slot; text: "Place selection here"
                            implicitHeight: 30; font.pixelSize: 12
                            enabled: !root.busy && root.d.extraIds.length > 0
                            onClicked: root.d.extraIds.forEach(id => root.d.fillSlot(exposedRow.modelData.name, id))
                        }
                    }
                }
            }
            Text { text: "Make a component from the selection"; color: Theme.textDim; font.pixelSize: 11 }
            RowLayout {
                Layout.fillWidth: true; spacing: 6
                StitchTextField { id: componentName; objectName: "interfaceComponentName"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Component name" }
                StitchButton { objectName: "interfaceSaveComponent"; text: "Save"; implicitHeight: 30; font.pixelSize: 12; enabled: root.has && !root.w.world_actor && !!componentName.text.trim() && !root.busy; onClicked: root.d.saveComponent(componentName.text.trim(), true) }
            }
            Text { text: "Library"; color: Theme.textDim; font.pixelSize: 11; visible: componentPick.count > 0 }
            StitchComboBox { id: componentPick; objectName: "interfaceComponentPick"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; visible: count > 0; model: Object.keys(root.d.document.prefabs || {}) }
            RowLayout {
                visible: componentPick.count > 0
                Layout.fillWidth: true; spacing: 6
                StitchButton { objectName: "interfaceInsertComponent"; text: "Insert"; implicitHeight: 30; font.pixelSize: 12; enabled: componentPick.currentIndex >= 0 && !root.busy; onClicked: root.d.insertComponent(componentPick.currentText) }
                StitchButton { objectName: "interfaceDeleteComponent"; text: "Delete component"; implicitHeight: 30; font.pixelSize: 12; enabled: componentPick.currentIndex >= 0 && !root.busy; onClicked: root.d.structuralEdit({kind: "DeletePrefab", name: componentPick.currentText}, root.d.selectedId, []) }
            }
            ColumnLayout {
                visible: componentPick.count > 0 && root.has
                Layout.fillWidth: true; spacing: 4
                Text { text: "Expose the selected widget's property or slot on " + componentPick.currentText; color: Theme.textDim; font.pixelSize: 11; Layout.fillWidth: true; wrapMode: Text.Wrap }
                RowLayout {
                    Layout.fillWidth: true; spacing: 6
                    StitchTextField { id: exposeName; objectName: "interfaceExposeName"; Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12; placeholderText: "Name" }
                    StitchComboBox { id: exposePath; objectName: "interfaceExposePath"; Layout.preferredWidth: 130; implicitHeight: 30; font.pixelSize: 12; model: Object.keys(root.d.propertyKinds).filter(p => root.d.applies(p)) }
                }
                RowLayout {
                    Layout.fillWidth: true; spacing: 6
                    StitchButton { objectName: "interfaceExposeProperty"; text: "Expose property"; implicitHeight: 30; font.pixelSize: 12; enabled: !!exposeName.text.trim() && !root.busy; onClicked: root.d.exposeProperty(componentPick.currentText, exposeName.text.trim(), exposePath.currentText) }
                    StitchButton { objectName: "interfaceExposeSlot"; text: "Expose as slot"; implicitHeight: 30; font.pixelSize: 12; enabled: !!exposeName.text.trim() && !root.busy; onClicked: root.d.exposeSlot(componentPick.currentText, exposeName.text.trim()) }
                }
            }
        }

        UiSection {
            designer: root.d; title: "Advanced (JSON)"; key: "advanced"; visible: root.has
            Text {
                Layout.fillWidth: true; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: 11
                text: "Edit one property as raw data. Prefer the sections above."
            }
            StitchComboBox {
                id: advancedPath
                objectName: "interfaceAdvancedPath"
                Layout.fillWidth: true; implicitHeight: 30; font.pixelSize: 12
                textRole: "label"
                model: root.d.propertyInfo.filter(p => root.d.applies(p.path)).map(p => ({path: p.path, label: p.group + ": " + p.label, ty: p.ty, choices: p.choices || []}))
            }
            TextArea {
                id: advancedText
                objectName: "interfaceAdvancedText"
                Layout.fillWidth: true; Layout.preferredHeight: 110
                wrapMode: TextEdit.Wrap; font.pixelSize: 11; color: Theme.text
                background: Rectangle { color: Theme.field; radius: 5; border.color: advancedText.activeFocus ? Theme.accent : Theme.border }
                readonly property var info: advancedPath.currentIndex >= 0 ? advancedPath.model[advancedPath.currentIndex] : null
                text: info && root.w ? root.d.formatProperty(info.ty, root.d.propertyValue(info.path)) : ""
            }
            StitchButton {
                objectName: "interfaceAdvancedApply"
                text: "Apply"; implicitHeight: 30; font.pixelSize: 12
                enabled: root.has && !!advancedText.info && !root.busy
                onClicked: {
                    try { root.d.propertyEdit(advancedText.info.path, root.d.parseProperty(advancedText.info.ty, advancedText.text)); }
                    catch (e) { root.d.error = String(e); }
                }
            }
        }
        Item { Layout.preferredHeight: 24 }
    }
}
