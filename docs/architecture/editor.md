# Editor frontend and Hub

The QML editor's structure, docks, actions, asset tray, projects and Hub coordination. Split out of `CLAUDE.md`; the index there lists every page.

## Frontend (`blockloom-qt/qml/`)

`Start.qml` is the entry point: it adds the Lucide icons blockstitch doesn't
carry (`Icons.add`) and then creates `Main`. Trigger (hat) labels are
uppercased in `buildRows()`. `Main.qml` holds the one copy of backend state (`appState`, parsed from
`bridge.stateJson`) and `invoke(command, args, done, failed)`, which every
other file calls. `Blocks.qml` (a singleton) is the single wiring point into
blockstitch and where a block's row comes from: every block is described once
as a list of pieces (a label, a value slot, a dropdown, a text field) and
registered with blockstitch's `BlockRegistry`, which both the canvas and the
palette read. **A new block is a `VerbSpec` row in `blockloom-core/src/verbs.rs`
(see Block verbs above) plus `BLESS=1 cargo test -p blockloom-core --test
verbs`.** The older blocks are each a variant of `InstructionKind`, a row
and an icon in `Blocks.qml`'s `buildRows()`, a label in its `labels`, an
entry in one of its `baseSections` (the palette's categories and titled
sections, drawn by blockstitch's `PalettePanel` in `BlockSidebar.qml`), a
field id if it has value slots, and a `Step`/`Effect`; add to that set only
for something a verb can't express (a hat, a body, a dropdown fed by the
project). A new sensing reporter's fresh value comes from its `default_args` in
`blockloom-core/src/value.rs`: `Blocks.qml` carries those as a generated
`hostDescriptors` table, so rerun `BLESS=1 cargo test -p blockloom-core --test
operator_descriptors` after adding or changing one. `BlockHeader` and `CallBlock` take their row from
a `BlockDef` rather than their type, which blockstitch draws itself.

Keep the canvas's inputs identity-stable, since any change rebuilds what is
bound to it. `Main.qml`'s `reuse` swaps every unchanged part of a new
snapshot for the old object, and `Blocks.qml` bumps `BlockRegistry.revision`
only when `optionSource` (the names its dropdowns list) changes.

`MakeBlockDialog.qml` is blockstitch's shared `BlockDefinitionEditor` in a
dialog, branches on any shape; the backend's `create_block`/`edit_block`
take its draft as is, icon, identifier and description included, refuse more
than `MAX_BRANCHES` branches, and turn every call into a `BranchCallBlock` or a
plain `CallBlock` to match the new pieces before blockstitch reconciles the
bodies. Canvas edits inside a reporter's branch name a `@reporter-branch:`
strand and go through blockstitch's `edit_reporter_branch`
(`commands::reporter_branch_edit`).

The editor page (`EditorPage.qml`) is a shell of three `PanelDock`s
(blockstitch `DockRegion`s): actors on the left, the inspector on the right,
and Assets, Run log, Problems, Profiler and every plugin panel as tabs along the bottom.
The Run log (`RunLog.qml`) keeps every line the backend holds in `all` and
shows those that pass its kind buttons (All, Errors with a count, Says,
Other) and its find field, which matches a line or its actor. The Profiler
(`ProfilerPanel.qml`) lists every `render_metrics` row the runtime reports,
grouped by the name before the first `/`, with the average and peak over the
last 30 status updates; it can hold its numbers and filter by name. The fps
chip's popup in the top bar stays as the short version. The actor list's
find field shows a flat list of actors whose name or a component's name holds
the words, wherever they sit in the tree. The asset tray's find field asks
`find-assets` (`assets::find`, every folder, at most 500) and shows each match
by its path; opening a folder ends the search. `Tour.qml` walks a first-time user round the
window: each step rings a part found by `objectName` and explains it. It
starts once (QML `Settings` category `tour`) and again from Help, and takes no
clicks outside its card. Each panel lives once in a hidden pool and is parented into the region its
id is placed in; a region's move button sends the current panel to another
edge, and the layout is kept in QML `Settings`. Code and Game sit
side by side when `split` is on. Everything the window can do is an id in
`EditorActions.qml`: a dispatch command (`undo`) or the window's own
(`editor.*`, carried out by `EditorPage.editorAction`). The top bar's menu
bar, `CommandPalette.qml` (Ctrl+Shift+P, which also lists every shell
command that takes no arguments) and the window's shortcuts are all built
from it, and the keys come from `blockloom-app/src/keymap.rs` through
`editor-commands`, so a new binding goes in `KEYMAP` and a new menu entry in
`EditorActions.menus`. A user's own bindings override `KEYMAP` in
`<data dir>/blockloom/keymap.json` (`set-keybinding`, `reset-keymap`, the
Keyboard shortcuts dialog `KeymapDialog.qml`; a key another action holds is
refused). The top bar keeps only the project's identity, the menus and the
run controls, and with one menu open hovering another switches to it; plugin
menu items join the Plugins menu.
`ProblemsPanel.qml` reads `problems` (`commands/problems.rs`: plugin
preflight and audit, script build and load errors, stale imports, asset
warnings). `ProjectSettingsDialog.qml` is a page tree: the project's own
pages, one page per world setting in the type registry (`RegistryForm`,
written through `set-property target=world`), and one page per plugin with
resource types (`PluginSettingsPage.qml`), with a filter over every label.

The asset tray (`AssetTray.qml`, the bottom dock's Assets tab) is a file manager
over the project folder: it lists, makes, imports, renames, moves and deletes
files, and an asset dragged out of it lands on any
`AssetField.qml` (the Look component's Image, the Script component's path).
Asset paths are relative to the project folder with forward slashes
(`assets/sprites/player.png`), the same spelling `Script` uses, and
`Project::repoint_asset` follows a renamed or moved file through the document
so the actor using it doesn't end up pointing at nothing. The listing is not
part of the state snapshot - a folder changes for reasons the editor never
hears about - so the tray re-lists after each of its own changes. Scenes are
tray files too: one `.blockscene` per scene, made from the tray's New scene
item, renamed like any other file (the filename is the scene name), and opened
with a double-click (or the Open scene menu item). Project settings names the
default scene, which is what a fresh open - and a built game - boots into;
the actor list's scene box only shows which scene is open.

A project is a folder, not a file: `<name>/project.blockloom` beside an
`assets/`. The folder sits wherever the New Project dialog was pointed
(`~/Blockloom/projects` by default), and the app owns the folder's name - it
follows the project's, so renaming a project moves its folder.
`<data dir>/blockloom/projects.json` remembers which folders the Dashboard
lists; names and dimensions are read back off disk, never cached there. With
`BLOCKLOOM_DATA_DIR` set, both that file and the default project location move
under it. `blockloom-core/src/library.rs` is that list, including the one-time
migration of pre-folder `<id>.blockloom` files.

The app opens on the Dashboard (`Dashboard.qml`); the editor
appears once a project is open, and `state.project` being null is what decides
which of the two shows. The window saves on idle (`Backend::save_when_idle`,
1.5 s): `auto_save` only marks an edit owed (and the copy dirty, so a live
reload waits), `save_if_due` writes it once edits go quiet or 10 s after the
first, and Play, Build, Close, quit and reopening write it at once
(`flush_save`). The shell and tests keep saving every edit, so a folder
attached from another process lags the window by up to that delay; one
attached over the socket shares its backend and sees every edit.

## Hub coordination

`blockloom-core/src/hub.rs` holds the Hub's say over the editor. The running
installation comes from `BLOCKLOOM_INSTALLATION` (set by the Hub) or the
manifest beside the executable; a project's binding is `.blockloom/hub.json`.
`open_project` refuses a mismatch with a `wrong-editor:` error unless
`other_editor=true`. Closing paths return a `save-failed:` error instead of
dropping unsaved work unless `discard=true`. The project list
(`library::list`) never prunes: missing folders come back with
`available: false`, and `locate_project`/`forget_project` recover them. The Hub
reads and writes the same `projects.json`.

The editor's Dashboard re-reads the library on a timer and on window
activation (`refresh_library`). `ProjectEntryDto` carries
`installation_label` and `wrong_editor` so a card names the editor and flags a
mismatch. `open_project`'s `other_editor` copies the project file to
`.blockloom/backups/` first and `rebind` makes the running editor the
project's. The Hub's service has `backups` and `restore-backup`; `focus`
reports `acknowledged`; polling goes through `HubService::poll`, which never
sets busy. The editor/Hub file formats are pinned by
`blockloom-core/tests/fixtures/hub-contract/`, read by both
`blockloom-core/tests/hub_contract.rs` and `blockloom-hub-state/tests/contract.rs`.

`blockloom-hub-state` (Qt-free) is the whole Hub service: the project list,
who holds a project, binding, snapshots, focus, starting or reusing editors,
and installing, downloading and building them (`manage.rs`: the operation
lock, release and dev installs, `tools.rs` the shared tool cache,
`fetch.rs` bounded HTTPS downloads that resume and retry plus safe zip/tar
extraction, `toolchain.rs` Android Rust targets, Java and the Android SDK/NDK
from their official sources, `github.rs` private catalogs through `gh`,
`run.rs` cancellable builds and copies, `stage.rs` the one-rename swap into
a slot). The Hub window runs it as `blockloom-hub --hubctl <command>`, so it
needs no Python; `just hub` and `scripts/hub.py` forward to
`blockloom-hubctl`, and the Python left beside it (`hub_install.py`,
`hub_download.py`, `hub_process.py`) only serves `just replace` and release
packaging. A cancel is the file named by `BLOCKLOOM_HUB_CANCEL_FILE`. Open and New
project reuse an idle editor: the editor's `hub_poll` (every 1.5 s, GUI-only)
publishes `<data dir>/blockloom/editors/<pid>.json`, the Hub drops
`<pid>.request.json` beside it and waits for `<pid>.reply.json`, and a new
process starts only when nothing idle answers.
