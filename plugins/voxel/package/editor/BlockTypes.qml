import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The voxel world's block types: the .blockdef files its block set lists,
// edited in place. `host` is handed in by the editor.
Item {
    id: root
    property var host
    readonly property var world: host ? (host.resource("world") || {}) : ({})
    readonly property string setPath: String(world.block_set || "")
    readonly property string folder: "assets/blocks"
    // The set's entries: {file, def} for a .blockdef, {file: "", def} inline.
    property var entries: []
    property int current: -1
    property string problem: ""
    property bool dirty: false

    // ─── Pure helpers (tested) ──────────────────────────────────────────────
    // `facing: north, east` lines into {facing: ["north", "east"]}.
    function parseProperties(text) {
        const out = {};
        const lines = String(text).split("\n");
        for (let i = 0; i < lines.length; i++) {
            const line = lines[i].trim();
            if (!line) continue;
            const colon = line.indexOf(":");
            if (colon < 1) throw new Error("line " + (i + 1) + " needs \"name: value, value\"");
            const name = line.slice(0, colon).trim();
            const values = line.slice(colon + 1).split(",").map(v => v.trim()).filter(v => v);
            if (!values.length) throw new Error(name + " lists no values");
            out[name] = values;
        }
        return out;
    }
    function propertiesText(props) {
        return Object.keys(props || {}).map(k => k + ": " + props[k].join(", ")).join("\n");
    }
    function stateCount(props) {
        return Object.keys(props || {}).reduce((n, k) => n * props[k].length, 1);
    }
    // The first few state names, last property fastest, as the registry counts.
    function stateNames(name, props, limit) {
        const keys = Object.keys(props || {});
        if (!keys.length) return [name];
        const out = [];
        const total = stateCount(props);
        for (let i = 0; i < Math.min(total, limit); i++) {
            let rest = i;
            const parts = [];
            for (let k = keys.length - 1; k >= 0; k--) {
                const values = props[keys[k]];
                parts.unshift(keys[k] + "=" + values[rest % values.length]);
                rest = Math.floor(rest / values.length);
            }
            out.push(name + "[" + parts.join(",") + "]");
        }
        return out;
    }
    // The form laid over the definition it started from; unknown keys stay.
    function buildDef(original, form) {
        const def = JSON.parse(JSON.stringify(original || {}));
        const name = String(form.name).trim();
        if (!/^[a-z0-9_]+$/.test(name)) throw new Error("a name is lowercase letters, digits and _");
        def.name = name;
        const id = String(form.id).trim();
        if (id === "") delete def.id;
        else {
            const n = Number(id);
            if (!Number.isInteger(n) || n < 1 || n > 65534) throw new Error("an id is a whole number from 1 to 65534");
            def.id = n;
        }
        const set = (key, value) => { if (value === "" || value === null || value === undefined) delete def[key]; else def[key] = value; };
        set("color", String(form.color).trim());
        set("model", String(form.model).trim());
        set("layer", form.layer === "opaque" ? "" : form.layer);
        const tags = String(form.tags).split(",").map(t => t.trim().toLowerCase()).filter(t => t);
        if (tags.length) def.tags = tags; else delete def.tags;
        const props = parseProperties(form.properties);
        if (Object.keys(props).length) def.properties = props; else delete def.properties;
        const variants = String(form.variants).trim();
        if (variants) {
            const parsed = JSON.parse(variants);
            if (!Array.isArray(parsed)) throw new Error("variants are a JSON list");
            def.variants = parsed;
        } else delete def.variants;
        return def;
    }
    function setText(list) {
        return JSON.stringify({ blocks: list }, null, 2) + "\n";
    }

    // ─── Backend ────────────────────────────────────────────────────────────
    function invoke(command, args, done) {
        root.host.app.invoke(command, args, done, function (e) { root.problem = String(e); });
    }
    function reload() {
        root.problem = "";
        root.dirty = false;
        if (!root.host || !root.setPath) { root.entries = []; root.current = -1; return; }
        invoke("read_text_asset", { path: root.setPath }, function (text) {
            let listed;
            try { listed = JSON.parse(text).blocks || []; } catch (e) { root.problem = root.setPath + ": " + e.message; return; }
            const loaded = listed.map(e => typeof e === "string" ? { file: e, def: null } : { file: "", def: e });
            root.entries = loaded;
            root.current = loaded.length ? Math.min(Math.max(root.current, 0), loaded.length - 1) : -1;
            loaded.forEach(function (entry, i) {
                if (!entry.file) return;
                invoke("read_text_asset", { path: entry.file }, function (body) {
                    const next = root.entries.slice();
                    try { next[i] = { file: entry.file, def: JSON.parse(body) }; }
                    catch (e) { next[i] = { file: entry.file, def: {}, error: e.message }; }
                    root.entries = next;
                    if (i === root.current) form.fill();
                });
            });
            form.fill();
        });
    }
    function writeSet(list, done) {
        invoke("write_text_asset", { path: root.setPath, text: setText(list) }, done);
    }
    function listed() {
        return root.entries.map(e => e.file ? e.file : e.def);
    }
    function makeSet() {
        const path = root.folder + "/blocks.blockset";
        invoke("write_text_asset", { path: path, text: setText([]) }, function () {
            const payload = JSON.parse(JSON.stringify(root.world));
            payload.block_set = path;
            root.host.setResource("world", payload, function () { Qt.callLater(root.reload); },
                                  function (e) { root.problem = String(e); });
        });
    }
    function addType(name) {
        const clean = String(name).trim();
        if (!/^[a-z0-9_]+$/.test(clean)) { root.problem = "a name is lowercase letters, digits and _"; return; }
        if (root.entries.some(e => e.def && e.def.name === clean)) { root.problem = clean + " is already in the set"; return; }
        const file = root.folder + "/" + clean + ".blockdef";
        const def = { name: clean, color: "#888888" };
        invoke("write_text_asset", { path: file, text: JSON.stringify(def, null, 2) + "\n" }, function () {
            const next = listed().concat([file]);
            root.entries = root.entries.concat([{ file: file, def: def }]);
            root.current = root.entries.length - 1;
            form.fill();
            writeSet(next);
        });
    }
    function removeCurrent() {
        if (root.current < 0) return;
        const next = root.entries.slice();
        next.splice(root.current, 1);
        root.entries = next;
        root.current = Math.min(root.current, next.length - 1);
        form.fill();
        writeSet(listed());
    }
    function saveCurrent() {
        const entry = root.entries[root.current];
        if (!entry) return;
        let def;
        try { def = buildDef(entry.def, form.values()); } catch (e) { root.problem = e.message; return; }
        root.problem = "";
        const next = root.entries.slice();
        next[root.current] = { file: entry.file, def: def };
        root.entries = next;
        root.dirty = false;
        if (entry.file) invoke("write_text_asset", { path: entry.file, text: JSON.stringify(def, null, 2) + "\n" });
        else writeSet(listed());
    }

    onSetPathChanged: reload()
    Component.onCompleted: reload()

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            Label {
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                text: root.setPath ? "Block set: " + root.setPath : "This world has no block set yet."
            }
            Button {
                visible: !root.setPath
                text: "Make a block set"
                onClicked: root.makeSet()
            }
        }
        Label {
            Layout.fillWidth: true
            visible: root.problem !== ""
            text: root.problem
            color: "#d04040"
            wrapMode: Text.Wrap
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: root.setPath !== ""
            spacing: 12

            ColumnLayout {
                Layout.preferredWidth: 200
                Layout.fillHeight: true
                ListView {
                    id: list
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: root.entries
                    currentIndex: root.current
                    delegate: ItemDelegate {
                        required property var modelData
                        required property int index
                        width: ListView.view.width
                        highlighted: index === root.current
                        text: (modelData.def && modelData.def.name ? modelData.def.name : modelData.file)
                              + (modelData.file ? "" : "  (inline)")
                        onClicked: { root.current = index; form.fill(); }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    TextField {
                        id: newName
                        Layout.fillWidth: true
                        placeholderText: "new_block"
                        onAccepted: addButton.clicked()
                    }
                    Button {
                        id: addButton
                        text: "Add"
                        enabled: newName.text.trim() !== ""
                        onClicked: { root.addType(newName.text); newName.text = ""; }
                    }
                }
            }

            ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                enabled: root.current >= 0
                GridLayout {
                    id: form
                    width: parent.width
                    columns: 2
                    columnSpacing: 8
                    function fill() {
                        const entry = root.entries[root.current];
                        const d = entry && entry.def ? entry.def : {};
                        nameField.text = d.name || "";
                        idField.text = d.id === undefined || d.id === null ? "" : String(d.id);
                        colorField.text = d.color || "";
                        modelField.text = typeof d.model === "string" ? d.model : "";
                        layerBox.currentIndex = Math.max(0, layerBox.model.indexOf(d.layer || "opaque"));
                        tagsField.text = (d.tags || []).join(", ");
                        propsArea.text = root.propertiesText(d.properties);
                        variantsArea.text = d.variants ? JSON.stringify(d.variants, null, 2) : "";
                        root.dirty = false;
                    }
                    function values() {
                        return { name: nameField.text, id: idField.text, color: colorField.text, model: modelField.text,
                                 layer: layerBox.currentText, tags: tagsField.text, properties: propsArea.text,
                                 variants: variantsArea.text };
                    }
                    function edited() { root.dirty = true; }

                    Label { text: "Name" }
                    TextField { id: nameField; Layout.fillWidth: true; onTextEdited: form.edited() }
                    Label { text: "Id" }
                    TextField { id: idField; Layout.fillWidth: true; placeholderText: "next free"; onTextEdited: form.edited() }
                    Label { text: "Color" }
                    RowLayout {
                        Layout.fillWidth: true
                        Rectangle {
                            width: 20; height: 20; radius: 3
                            color: /^#[0-9a-fA-F]{6}$/.test(colorField.text) ? colorField.text : "transparent"
                            border.color: "#80808080"
                        }
                        TextField { id: colorField; Layout.fillWidth: true; placeholderText: "#8a6a3a"; onTextEdited: form.edited() }
                    }
                    Label { text: "Model" }
                    TextField { id: modelField; Layout.fillWidth: true; placeholderText: "cube, slab, stair north, ..."; onTextEdited: form.edited() }
                    Label { text: "Layer" }
                    ComboBox { id: layerBox; Layout.fillWidth: true; model: ["opaque", "cutout", "translucent"]; onActivated: form.edited() }
                    Label { text: "Tags" }
                    TextField { id: tagsField; Layout.fillWidth: true; placeholderText: "stairs, wood"; onTextEdited: form.edited() }
                    Label { text: "Properties"; Layout.alignment: Qt.AlignTop }
                    TextArea {
                        id: propsArea
                        Layout.fillWidth: true
                        placeholderText: "facing: north, east, south, west"
                        onTextChanged: if (activeFocus) form.edited()
                    }
                    Label { text: "States"; Layout.alignment: Qt.AlignTop }
                    Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        opacity: 0.7
                        text: {
                            let props;
                            try { props = root.parseProperties(propsArea.text); } catch (e) { return e.message; }
                            const n = root.stateCount(props);
                            const names = root.stateNames(nameField.text || "block", props, 4);
                            return n + (n === 1 ? " state: " : " states: ") + names.join("  ") + (n > 4 ? "  ..." : "");
                        }
                    }
                    Label { text: "Variants"; Layout.alignment: Qt.AlignTop }
                    TextArea {
                        id: variantsArea
                        Layout.fillWidth: true
                        font.family: "monospace"
                        placeholderText: "[{\"when\": {\"facing\": \"north\"}, \"model\": \"stair north\"}]"
                        onTextChanged: if (activeFocus) form.edited()
                    }
                    Item { width: 1; height: 1 }
                    RowLayout {
                        Button { text: "Save"; enabled: root.dirty; onClicked: root.saveCurrent() }
                        Button { text: "Revert"; enabled: root.dirty; onClicked: form.fill() }
                        Button { text: "Remove from set"; onClicked: root.removeCurrent() }
                    }
                }
            }
        }
    }
}
