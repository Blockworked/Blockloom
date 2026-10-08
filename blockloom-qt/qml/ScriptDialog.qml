import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.blockworked.Blockstitch 1.0

// Editing an actor's Rust script without leaving Blockloom. The file on disk
// is the document here - it isn't part of the project - so this reads it on
// open and writes it on save. "Check" hands it to rustc, and diagnostics come
// back pinned to their lines so they show inline too.
BwDialog {
    id: root
    required property var app
    property var actor: null
    property string path: ""
    property string error: ""
    property bool busy: true
    property var diagnostics: []
    property var toolchain: null
    property string ideNote: ""
    property var completions: []
    property bool completionOpen: false
    property int selectedCompletion: 0
    property string hoverDoc: ""
    readonly property var scriptStatus: (app.appState.script_statuses || {})[path] || null
    title: actor ? actor.name + " · " + path : path
    standardButtons: Dialog.NoButton
    width: Math.min(parent ? parent.width - 80 : 900, 900)

    function openFor(a, p, line) {
        actor = a; path = p; error = ""; ideNote = ""; diagnostics = []; toolchain = null; busy = true;
        completionOpen = false; completions = []; hoverDoc = "";
        editor.text = "";
        open();
        app.invoke("read_script", { actorId: a.id, path: p }, source => { editor.text = source; editor.cursorPosition = 0; busy = false; editor.forceActiveFocus(); if (line > 0) jumpTo(line, 1); }, e => { error = String(e); busy = false; });
        refreshDiagnostics();
        app.invoke("script_toolchain", {}, status => toolchain = status, e => error = String(e));
    }
    function refreshDiagnostics() { if (!actor) return; app.invoke("script_diagnostics", { actorId: actor.id, path: path }, list => diagnostics = list, () => diagnostics = []); }
    function save(then) {
        error = "";
        // The write itself clears the last build status; re-check what is on
        // screen now, so diagnostics follow every save without a second click.
        app.invoke("write_script", { actorId: actor.id, path: path, source: editor.text }, () => { refreshDiagnostics(); if (then) then(); }, e => { busy = false; error = String(e); });
    }
    // Saving first, so rustc is told about what's on screen.
    function check() {
        busy = true;
        save(() => app.invoke("check_script", { actorId: actor.id, path: path }, () => { busy = false; refreshDiagnostics(); }, e => { busy = false; error = String(e); }));
    }
    // Save+Format: rustfmt rewrites the file, and the box shows what it did.
    function format() {
        busy = true; error = "";
        save(() => app.invoke("format_script", { actorId: actor.id, path: path }, formatted => { editor.text = formatted; busy = false; refreshDiagnostics(); }, e => { busy = false; error = String(e); }));
    }
    // Opt-in lint pass: clippy's rows land where check's do, same shape.
    function lint() {
        busy = true; error = "";
        save(() => app.invoke("clippy_script", { actorId: actor.id, path: path }, list => { diagnostics = list; busy = false; }, e => { busy = false; error = String(e); }));
    }
    // Completion: Ctrl+Space lists API methods and project symbols starting
    // with the word under the caret; Tab/Enter inserts the picked row.
    function currentPrefix() {
        const before = editor.text.slice(0, editor.cursorPosition);
        const m = /[A-Za-z0-9_:]+$/.exec(before);
        return m ? m[0] : "";
    }
    function requestCompletions() {
        app.invoke("script_completions", { prefix: currentPrefix() }, list => {
            completions = list; selectedCompletion = 0;
            completionOpen = list.length > 0;
            if (completionOpen) updateHover();
        }, () => completionOpen = false);
    }
    function updateHover() {
        const item = completions[selectedCompletion];
        if (!item) { hoverDoc = ""; return; }
        app.invoke("script_hover", { symbol: item.label }, doc => hoverDoc = doc || item.detail, () => hoverDoc = item.detail);
    }
    function acceptCompletion() {
        const item = completions[selectedCompletion];
        completionOpen = false;
        if (!item) { editor.forceActiveFocus(); return; }
        const prefix = currentPrefix(), at = editor.cursorPosition;
        editor.remove(at - prefix.length, at);
        editor.insert(at - prefix.length, item.insert);
        editor.forceActiveFocus();
    }
    function openExternal() {
        busy = true; ideNote = "";
        app.invoke("sync_script_ide", {}, () => app.invoke("open_script_ide", {}, opened => { busy = false; ideNote = "Opened with " + opened.openedWith + ": " + opened.path; },
            e => { busy = false; error = String(e); }), e => { busy = false; error = String(e); });
    }
    function jumpTo(line, column) {
        const rows = editor.text.split("\n");
        let offset = 0;
        for (let i = 0; i < line - 1 && i < rows.length; ++i) offset += rows[i].length + 1;
        editor.forceActiveFocus();
        editor.cursorPosition = Math.min(editor.length, offset + Math.max(0, column - 1));
    }
    readonly property var lineLevels: {
        const out = {};
        for (const d of diagnostics) if (out[d.line] !== "error") out[d.line] = d.level === "error" ? "error" : "warning";
        return out;
    }
    // Continuous check: two quiet seconds re-run diagnostics on what's on
    // screen, so inline errors follow typing, not just Check and Save.
    Timer {
        id: autoCheck
        interval: 2000; repeat: false
        onTriggered: if (!root.busy && root.actor && root.visible) root.refreshDiagnostics()
    }

    // ─── Rust highlighting ─────────────────────────────────────────────────
    // A small tokenizer, not a grammar: good enough to read real Rust by.
    readonly property var keywords: "as,break,const,continue,crate,else,enum,extern,false,fn,for,if,impl,in,let,loop,match,mod,move,mut,pub,ref,return,self,Self,static,struct,super,trait,true,type,unsafe,use,where,while,async,await,dyn,try".split(",")
    readonly property var colors: ({ keyword: "#c678dd", string: "#98c379", comment: "#7f848e", number: "#d19a66", type: "#e5c07b", call: "#61afef", macro: "#56b6c2", attr: "#56b6c2" })
    function esc(t) { return t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"); }
    function highlight(src) {
        let out = "", i = 0;
        const n = src.length;
        const wrap = (cls, text) => "<span style=\"color:" + colors[cls] + "\">" + esc(text) + "</span>";
        while (i < n) {
            const rest = src.slice(i);
            if (rest.startsWith("//")) { const end = src.indexOf("\n", i); const stop = end === -1 ? n : end; out += wrap("comment", src.slice(i, stop)); i = stop; continue; }
            if (rest.startsWith("/*")) { const end = src.indexOf("*/", i + 2); const stop = end === -1 ? n : end + 2; out += wrap("comment", src.slice(i, stop)); i = stop; continue; }
            if (rest[0] === "\"" || rest[0] === "'") {
                const quote = rest[0];
                let j = i + 1;
                while (j < n) {
                    if (src[j] === "\\") j += 2;
                    else if (src[j] === quote) { j += 1; break; }
                    else if (src[j] === "\n" && quote === "'") break;
                    else j += 1;
                }
                const text = src.slice(i, j);
                out += text.length <= 4 || quote === "\"" ? wrap("string", text) : esc(text);
                i = j; continue;
            }
            if (rest[0] === "#") { const end = src.indexOf("]", i); const stop = rest.startsWith("#[") && end !== -1 ? end + 1 : i + 1; out += wrap("attr", src.slice(i, stop)); i = stop; continue; }
            const number = /^[0-9][0-9_]*(\.[0-9_]+)?(e[+-]?[0-9_]+)?(f32|f64|i32|u32|u64|usize)?/.exec(rest);
            if (number) { out += wrap("number", number[0]); i += number[0].length; continue; }
            const word = /^[A-Za-z_][A-Za-z0-9_]*/.exec(rest);
            if (word) {
                const text = word[0], after = src.slice(i + text.length);
                if (keywords.indexOf(text) >= 0) out += wrap("keyword", text);
                else if (after.startsWith("!")) out += wrap("macro", text) + "!";
                else if (after.startsWith("(")) out += wrap("call", text);
                else if (/^[A-Z]/.test(text)) out += wrap("type", text);
                else out += esc(text);
                i += text.length + (after.startsWith("!") ? 1 : 0);
                continue;
            }
            out += esc(rest[0]);
            i += 1;
        }
        return out;
    }

    ColumnLayout {
        width: root.availableWidth; spacing: 8
        ScrollView {
            id: statusScroll
            Layout.fillWidth: true; Layout.preferredHeight: Math.min(120, statusText.implicitHeight)
            clip: true
            Text {
                id: statusText
                width: statusScroll.availableWidth; wrapMode: Text.WordWrap; textFormat: Text.PlainText
                color: scriptStatus && scriptStatus.error ? Theme.danger : Theme.textDim
                text: !scriptStatus ? "Not built yet. Check the script or press Play." : scriptStatus.stage === "build_failed" ? "Build failed: " + scriptStatus.error
                    : scriptStatus.stage === "load_failed" ? "Load failed: " + scriptStatus.error
                    : scriptStatus.stage === "loaded" ? "Script loaded" : "Script compiled"
            }
        }
        Text { visible: root.error.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; text: root.error; color: Theme.danger }
        Text {
            visible: !!root.toolchain && !root.toolchain.available
            Layout.fillWidth: true; wrapMode: Text.WordWrap; color: "#ffb86b"; font.pixelSize: 12
            text: root.toolchain ? "No Rust toolchain found - scripts won't run until one is installed. Blocks still run. " + root.toolchain.help : ""
        }
        Rectangle {
            Layout.fillWidth: true; Layout.preferredHeight: 380
            radius: 6; color: "#1c1f26"; border.color: editor.activeFocus ? Theme.accent : Theme.border; clip: true
            Flickable {
                id: flick
                anchors.fill: parent; anchors.margins: 1
                contentWidth: Math.max(width, gutter.width + editor.implicitWidth); contentHeight: Math.max(height, editor.implicitHeight)
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar {}
                ScrollBar.horizontal: ScrollBar {}
                // Keep the caret in view while typing.
                function ensureVisible(r) {
                    const x = r.x + gutter.width, y = r.y;
                    if (contentX >= x) contentX = Math.max(0, x - 8); else if (contentX + width <= x + 8) contentX = x + 8 - width;
                    if (contentY >= y) contentY = y; else if (contentY + height <= y + r.height) contentY = y + r.height - height;
                }
                Column {
                    id: gutter
                    width: 44; topPadding: editor.topPadding
                    Repeater {
                        model: editor.lineCount
                        delegate: Text {
                            required property int index
                            readonly property string level: root.lineLevels[index + 1] || ""
                            width: 38; horizontalAlignment: Text.AlignRight
                            height: editor.cursorRectangle.height > 0 ? editor.cursorRectangle.height : implicitHeight
                            text: index + 1; font: editor.font
                            color: level === "error" ? "#ff6b6b" : level === "warning" ? "#e5c07b" : "#8b93a3"
                        }
                    }
                }
                TextEdit {
                    id: editor
                    x: gutter.width
                    width: Math.max(flick.width - gutter.width, implicitWidth)
                    topPadding: 8; leftPadding: 8; rightPadding: 8; bottomPadding: 8
                    font.family: "monospace"; font.pixelSize: 13
                    color: "transparent"; selectionColor: "#594c97ff"; selectedTextColor: "transparent"
                    selectByMouse: true; wrapMode: TextEdit.NoWrap; textFormat: TextEdit.PlainText
                    persistentSelection: true; readOnly: root.busy && !text.length
                    onCursorRectangleChanged: flick.ensureVisible(cursorRectangle)
                    onTextChanged: autoCheck.restart()
                    // Ctrl+Space completes; arrows/Enter/Tab drive the popup while open.
                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_Space && (event.modifiers & Qt.ControlModifier)) {
                            root.requestCompletions(); event.accepted = true;
                        } else if (root.completionOpen) {
                            if (event.key === Qt.Key_Escape) { root.completionOpen = false; event.accepted = true; }
                            else if (event.key === Qt.Key_Down) { root.selectedCompletion = Math.min(root.completions.length - 1, root.selectedCompletion + 1); root.updateHover(); event.accepted = true; }
                            else if (event.key === Qt.Key_Up) { root.selectedCompletion = Math.max(0, root.selectedCompletion - 1); root.updateHover(); event.accepted = true; }
                            else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { root.acceptCompletion(); event.accepted = true; }
                        }
                    }
                    // Tab inserts two spaces and stays in the box.
                    Keys.onTabPressed: event => {
                        if (root.completionOpen) { root.acceptCompletion(); event.accepted = true; return; }
                        const at = cursorPosition; remove(selectionStart, selectionEnd); insert(selectionStart, "  "); cursorPosition = at + 2; event.accepted = true;
                    }
                    Text {
                        z: -1
                        x: editor.leftPadding; y: editor.topPadding
                        font: editor.font; color: "#dcdfe4"
                        textFormat: Text.RichText
                        // Not <pre>: that swaps in its own font, whose line height then drifts from the editor's.
                        text: "<div style=\"white-space:pre\">" + root.highlight(editor.text) + "</div>"
                    }
                    Rectangle {
                        z: -2; x: 0; width: editor.width; height: editor.cursorRectangle.height; y: editor.cursorRectangle.y
                        visible: editor.activeFocus; color: "#0cffffff"
                    }
                    Rectangle { visible: editor.activeFocus; x: editor.cursorRectangle.x; y: editor.cursorRectangle.y; width: 2; height: editor.cursorRectangle.height; color: "#eeeeee" }
                }
            }
        }
        ListView {
            visible: root.diagnostics.length > 0
            Layout.fillWidth: true; Layout.preferredHeight: Math.min(120, contentHeight); clip: true
            model: root.diagnostics
            delegate: RowLayout {
                required property var modelData
                width: ListView.view.width; spacing: 8
                BwButton { flat: true; implicitHeight: 24; font.pixelSize: 11; text: modelData.line + ":" + modelData.column; onClicked: root.jumpTo(modelData.line, modelData.column) }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: 12; color: modelData.level === "error" ? "#ff8080" : "#e5c07b"; text: modelData.message }
            }
        }
        Rectangle {
            visible: root.completionOpen
            Layout.fillWidth: true; Layout.preferredHeight: Math.min(190, completionList.contentHeight + hoverDocText.implicitHeight + 16)
            radius: 6; color: "#24272f"; border.color: Theme.accent; clip: true
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 6; spacing: 4
                ListView {
                    id: completionList
                    Layout.fillWidth: true; Layout.preferredHeight: Math.min(120, contentHeight); clip: true
                    model: root.completions
                    currentIndex: root.selectedCompletion
                    highlight: Rectangle { color: "#0cffffff"; radius: 3 }
                    delegate: RowLayout {
                        required property var modelData
                        required property int index
                        width: ListView.view.width; spacing: 8
                        Text { font.family: "monospace"; font.pixelSize: 12; color: "#61afef"; text: modelData.insert }
                        Text { Layout.fillWidth: true; elide: Text.ElideRight; font.pixelSize: 11; color: Theme.textDim; text: modelData.detail }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: { root.selectedCompletion = index; root.updateHover(); }
                            onDoubleClicked: { root.selectedCompletion = index; root.acceptCompletion(); }
                        }
                    }
                }
                Text {
                    id: hoverDocText
                    visible: root.hoverDoc.length > 0
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: 11; color: Theme.textDim
                    text: root.hoverDoc
                }
            }
        }
        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11
            text: "Real Rust, compiled with rustc when you press Play. `std` is there; other crates aren't. Ctrl+Space completes API methods and project names; Tab takes the picked row. Errors land in the run log and on their lines above. The project root holds a Cargo.toml for rust-analyzer, so this file also opens in VS Code, Zed or RustRover with completion and go-to-source on the API." }
        Text { visible: !!root.toolchain && !root.toolchain.rustfmt_version; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: "#ffb86b"; font.pixelSize: 11
            text: "rustfmt isn't installed, so Format is off. Install it with `rustup component add rustfmt`." }
        Text { visible: !!root.toolchain && !root.toolchain.clippy_version; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: "#ffb86b"; font.pixelSize: 11
            text: "Clippy isn't installed, so Lint is off. Install it with `rustup component add clippy`." }
        Text { visible: root.ideNote.length > 0; Layout.fillWidth: true; wrapMode: Text.WordWrap; color: Theme.textDim; font.pixelSize: 11; text: root.ideNote }
        RowLayout {
            Layout.alignment: Qt.AlignRight; spacing: 8
            BwButton { text: "Cancel"; onClicked: { root.completionOpen = false; root.close(); } }
            BwButton { text: "Open in editor"; enabled: !root.busy; onClicked: root.openExternal() }
            BwButton { text: "Lint"; enabled: !root.busy && (!root.toolchain || !!root.toolchain.clippy_version); onClicked: root.lint() }
            BwButton { text: "Format"; enabled: !root.busy && (!root.toolchain || !!root.toolchain.rustfmt_version); onClicked: root.format() }
            BwButton { text: "Check"; enabled: !root.busy; onClicked: root.check() }
            BwButton { text: "Save"; primary: true; enabled: !root.busy; onClicked: root.save(() => root.close()) }
        }
    }
}
