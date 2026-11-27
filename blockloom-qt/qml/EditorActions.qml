import QtQuick

// Everything the editor can be asked to do, by id: the menu bar, the command
// palette and the window's shortcuts all read this. An id is a dispatch
// command (`undo`) or the window's own (`editor.*`), the same ids the
// backend's keymap binds keys to (`editor-commands`).
QtObject {
    id: root
    required property var app
    // The editor page, which carries out `editor.*` actions.
    required property var page
    readonly property var appState: app.appState

    // The shell's command registry and the keymap, fetched once.
    property var specs: []
    property var keymap: ({})
    Component.onCompleted: app.invoke("editor_commands", {}, r => root.load(r), () => {})
    function load(r) {
        if (r.commands) root.specs = r.commands;
        const keys = {};
        for (const b of r.keymap || []) keys[b.action] = b.keys;
        root.keymap = keys;
        const defaults = {};
        for (const b of r.defaults || []) defaults[b.action] = b.keys;
        root.defaultKeys = defaults;
    }
    property var defaultKeys: ({})
    // Rebinds an action for this user (`keys` null puts the default back).
    function rebind(id, keys, done, failed) {
        root.app.invoke("set_keybinding", keys === null ? { action: id } : { action: id, keys: keys },
            r => { root.load(r); if (done) done(); }, e => { if (failed) failed(String(e)); });
    }
    function keys(id) { return root.keymap[id] || []; }
    function keyText(id) { const k = root.keys(id); return k.length ? k[0] : ""; }

    readonly property var plugins: appState.plugins || ({})
    readonly property bool hasProject: !!appState.project
    // Menus in bar order, each a list of action ids ("-" a separator).
    readonly property var menus: [
        { title: "File", items: ["close_project", "save_project", "-", "editor.import", "editor.export", "-", "editor.build", "-", "editor.projectSettings", "editor.appSettings"] },
        { title: "Edit", items: ["undo", "redo", "-", "editor.palette"] },
        { title: "View", items: ["editor.showCode", "editor.showGame", "editor.showInterface", "editor.showDevices", "-", "editor.split", "-", "editor.assets", "editor.runLog", "editor.problems", "editor.profiler"] },
        { title: "Run", items: ["run_project", "stop_project", "editor.pause", "step_project", "-", "run_tests", "-", "close_runtime"] },
        { title: "Plugins", items: ["editor.plugins", "editor.pluginPanels", "editor.pluginEditors"] },
        { title: "Help", items: ["editor.tour", "editor.palette", "editor.shortcuts"] }
    ]
    readonly property var defs: ({
        "close_project": { label: "All projects", icon: "layout-grid" },
        "save_project": { label: "Save", icon: "save" },
        "editor.import": { label: "Import a project…", icon: "upload" },
        "editor.export": { label: "Export this project…", icon: "download" },
        "editor.build": { label: "Build a standalone game…", icon: "package" },
        "editor.projectSettings": { label: "Project settings…", icon: "settings" },
        "editor.appSettings": { label: "App settings…", icon: "sliders-horizontal" },
        "undo": { label: "Undo", icon: "undo-2" },
        "redo": { label: "Redo", icon: "redo-2" },
        "editor.palette": { label: "Command palette…", icon: "search" },
        "editor.shortcuts": { label: "Keyboard shortcuts…", icon: "keyboard" },
        "editor.tour": { label: "Tour the editor", icon: "map" },
        "editor.showCode": { label: "Code", icon: "blocks" },
        "editor.showGame": { label: "Game", icon: "gamepad-2" },
        "editor.showInterface": { label: "Interface", icon: "layout-grid" },
        "editor.showDevices": { label: "Devices", icon: "smartphone" },
        "editor.split": { label: "Game beside the code", icon: "columns-2" },
        "editor.assets": { label: "Assets", icon: "folder" },
        "editor.runLog": { label: "Run log", icon: "scroll" },
        "editor.problems": { label: "Problems", icon: "triangle-alert" },
        "editor.profiler": { label: "Profiler", icon: "gauge" },
        "run_project": { label: "Play", icon: "play" },
        "stop_project": { label: "Stop", icon: "square" },
        "editor.pause": { label: "Pause", icon: "pause" },
        "step_project": { label: "Advance one tick", icon: "step-forward" },
        "run_tests": { label: "Run playmode tests", icon: "flask-conical" },
        "close_runtime": { label: "Close the game window", icon: "monitor-x" },
        "editor.plugins": { label: "Plugin manager…", icon: "plug-zap" },
        "editor.pluginPanels": { label: "Show plugin panels", icon: "panel-left" },
        "editor.pluginEditors": { label: "Plugin editors…", icon: "app-window" }
    })

    function label(id) {
        if (id === "editor.pause") return root.appState.paused ? "Resume" : "Pause";
        const def = root.defs[id];
        return def ? def.label : id;
    }
    function icon(id) {
        if (id === "editor.pause") return root.appState.paused ? "play" : "pause";
        const def = root.defs[id];
        return def ? def.icon : "command";
    }
    function visible(id) {
        if (id === "editor.pluginPanels") return !!(root.plugins.panels && root.plugins.panels.length);
        if (id === "editor.pluginEditors") return !!(root.plugins.editorModules && root.plugins.editorModules.length);
        return true;
    }
    function enabled(id) {
        if (!root.hasProject) return false;
        const running = root.appState.running === true;
        switch (id) {
        case "undo": return !!root.appState.can_undo;
        case "redo": return !!root.appState.can_redo;
        case "run_project": return !running;
        case "stop_project": case "editor.pause": return running;
        case "step_project": return running && !!root.appState.paused;
        case "close_runtime": return !!root.appState.runtime_open;
        }
        return root.visible(id);
    }
    function checked(id) { return root.page.actionChecked ? root.page.actionChecked(id) : false; }

    // A command that fails says so in the run log, where the user looks.
    function report(command, args, done) {
        root.app.invoke(command, args || {}, done || null, e => root.app.invoke("push_log", { kind: "error", text: String(e) }));
    }
    function run(id) {
        if (!root.enabled(id)) return;
        if (id === "editor.pause") root.report("pause_project", { paused: !root.appState.paused });
        else if (id.startsWith("editor.")) root.page.editorAction(id);
        // The verdicts land in the run log; show it.
        else if (id === "run_tests") { if (root.page.showPanel) root.page.showPanel("runLog"); root.report(id); }
        else root.report(id);
    }

    // Commands the palette offers beyond the menus: every shell command that
    // needs no arguments. What one answers goes to the run log.
    readonly property var hidden: ["get_state", "editor_commands", "hub_poll", "shutdown", "refresh_library"]
    readonly property var paletteEntries: {
        const out = [];
        const seen = {};
        for (const menu of root.menus)
            for (const id of menu.items) {
                if (id === "-" || seen[id] || !root.visible(id)) continue;
                seen[id] = true;
                out.push({ id: id, label: root.label(id), detail: menu.title, icon: root.icon(id), keys: root.keyText(id), command: false });
            }
        for (const spec of root.specs) {
            if (seen[spec.cmd] || root.hidden.indexOf(spec.cmd) >= 0) continue;
            if ((spec.args || []).some(a => a.required)) continue;
            seen[spec.cmd] = true;
            out.push({ id: spec.cmd, label: spec.name, detail: spec.summary, icon: "command", keys: root.keyText(spec.cmd), command: true });
        }
        return out;
    }
    function runEntry(entry) {
        if (!entry.command) { root.run(entry.id); return; }
        root.report(entry.id, {}, answer => {
            if (entry.id === "reset_keymap") { root.load(answer); return; }
            if (answer === null || answer === undefined || (typeof answer === "object" && Object.keys(answer).length === 0)) return;
            const text = typeof answer === "string" ? answer : JSON.stringify(answer);
            root.app.invoke("push_log", { kind: "info", text: entry.label + ": " + (text.length > 2000 ? text.slice(0, 2000) + "…" : text) });
        });
    }
}
