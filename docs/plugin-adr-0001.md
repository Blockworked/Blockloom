# ADR 0001: plugin platform foundation (plan phases 0, 1 and the data half of 2)

Status: accepted for the first implementation PR. Follows
`plugin-system-and-voxel-plan.md`; this file records what was built, what was
measured, and where the build differs from the plan.

## Decisions

**Crates.** `blockloom-plugin-api` (manifest, schemas, records, C ABI types;
serde and semver only) and `blockloom-plugin-host` (resolver, cache, lock
files, install transactions, native loader, lifecycle). `blockloom-core`
depends on the api crate only. The host crate pulls in libloading, zip and git
handling, none of which belong in a crate that must still build for
`wasm32-unknown-unknown`.

**Identity and reproducibility.** A plugin is a reverse-domain id plus a semver
version. `plugin.json` lists a sha256 for every file; the package's content
hash covers the manifest bytes and that table, and a version published to a
registry or the cache is immutable. A project holds three files:
`plugins.json` (what the author asked for), `plugins.lock` (the whole resolved
graph, with hashes, sources and per-target library hashes) and the optional,
uncommitted `plugins.local.json` (development overrides). Only an explicit
update ever moves a locked version.

**Installs are transactions.** A change resolves the whole graph first, fetches
and verifies into a staging directory, publishes into the shared immutable
cache, then rewrites the two project files atomically after snapshotting the
old pair into `.blockloom/plugin-history`. `plugin-rollback` restores the last
snapshot and needs no network. Resolution is a backtracking search that
prefers locked versions and answers failures with the chain of requirements
that caused them.

**Data is never lost for lack of code.** A plugin's data in the document is a
`PluginRecord { plugin, type_id, schema_version, payload }`: an actor component
(`ActorComponent::Plugin`) or a project resource (`Project.plugin_resources`).
Core never interprets a payload. A record whose plugin is missing, too new or
invalid is kept byte for byte through open and save. Whether that stops Play
or a build is decided by the schema (`editor_only` records do not), and the
refusal names the records and the command that fixes it. Migrations run on a
copy, all records or none, and snapshot the old payloads first.

**Tiers.** Declarative packages (schemas, commands, blocks, hook ordering) work
everywhere and need no code. Native packages are C-ABI shared libraries per
target triple; a build that cannot carry a package's code for the target is
refused (web and Android builds refuse code plugins for now). Portable (WASM)
packages run in the editor under an interpreter (see "Portable modules"
below). The source-linked adapter tier is described in the manifest and
validated, but nothing executes it yet.

**Native ABI v1.** One exported symbol, `blockloom_plugin_entry_v1`. Every
struct is size and version prefixed so fields can be added; buffers are
`Slice`/`Buffer` pairs with explicit ownership; calls return a `Status`;
objects cross the boundary as generational handles from a `HandleTable`, so a
stale handle is an error and never a dangling pointer. The host function table
a plugin receives is gated by the capabilities its manifest declared.

**Lifecycle.** `Lifecycle` hands out generation tickets so work started before
a plugin reload cannot land after it; `order_hooks` sorts contributed hooks by
stage and explicit before/after edges and rejects cycles.

## Measurements (container, release build, no GPU)

The ignored `abi_call_cost` test in `blockloom-plugin-host` calls a
rustc-compiled C-layout fixture library.

| Shape | Cost |
| --- | --- |
| One call per item through the C ABI | about 43 ns per call |
| One call per batch of items | about 0.21 ns per item |

That is a 200x spread, which settles the plan's open question: any plugin hot
path (voxel pages, mesh jobs) must be a bulk API, and per-item calls are for
rare events. GPU and browser costs were not measured: there is no GPU in the
build container. The portable executor's costs are under "Portable modules".

## Proof package

`plugins/examples/com.example.health` is a sealed declarative package outside
engine source: a versioned `Health` component with a schema migration, a
`Difficulty` resource, three commands and one block. `blockloom-app/tests/plugins.rs`
installs it from a `path:` source, runs its commands through `plugin-call`,
removes it, shows the records survive and the project refuses to run, and
rolls the removal back. `plugin-seal` and `plugin-publish` are the author
tools.

## Deviations from the plan

Implemented from phase 2 only the data, command and migration parts. Not done,
and tracked in `TODO.md`:

- The Plugin Manager is a dialog over the backend commands. Plugin components
  and resources are edited through schema-generated forms (see the thirteenth
  batch); panels and dynamically loaded trusted editor modules do not exist yet,
  and QML for those would have to be loaded at run time, which the compile-time
  `QmlModule` list cannot do.
- The portable tier runs through a native interpreter in the editor only. There
  is no browser host, so no browser proof, and a built game does not carry the
  executor.
- Native modules load in the editor (a `module` command action calls them,
  tested with a rustc-built C-layout fixture) but not yet in the runtime's
  world or the built player, so plugin code cannot act on a running game.
- Plugin statement blocks run in the editor only (see "Plugin blocks" below).
  Reporters, hats, the script ABI, palette entries and built games are not
  done.
- Registry transport is a local folder (`DirRegistry`) or, since the twelfth batch, the same folder served over HTTP (`HttpRegistry`).
- Shell and MCP expose plugin commands through `plugin-id/command` shell lines
  and the `plugin-call` tool. MCP does not yet list each plugin command as a
  tool of its own.
- Package changes (install, remove, update) refuse to run on a copy attached
  to another editor's files, so they never race the owner's; they do not
  bump the folder's revision counter yet.
- Voxel work (plan part II) was not started; it needs phase 2 services that
  do not exist yet.
- Pack format is v2 only for packs that carry plugins; others are still
  written as v1 so older players read them.

## Plugin blocks (second batch)

A plugin's `BlockSchema` becomes one generic instruction,
`PluginBlock { plugin, block, args }`, rather than a variant per block, so a
document holding one loads in an editor that lacks the plugin. The VM lowers it
to `Effect::PluginCall`; the world forwards it to the editor
(`RuntimeMessage::PluginCall`, protocol 21), because the editor already owns the
plugin host, and the editor runs the block's command through `plugin_call`.
That keeps native modules out of the world for now, at the price of a message
round trip, so these blocks are for rare events, not per-frame work. The result
of the command is dropped (a statement has none), failures reach the run log,
and Play is refused when a block's plugin is missing or its slot count changed.

Deviations: codegen returns `Unsupported` naming the block, so a project with
plugin blocks plays on the VM, and a Build refuses them until the player can
run plugin code. Palette entries need blockstitch to draw a row from a schema
at run time, which this batch does not touch.

## Portable modules (third batch)

`blockloom-plugin-api/src/wasm.rs` is the contract and
`blockloom-plugin-host/src/portable.rs` the executor, on `wasmi` 2.0. It is the
native ABI's contract over one linear memory: a core module with no WASI that
imports `blockloom.log` and `blockloom.call` and exports `memory`,
`blockloom_abi`, `blockloom_alloc`, `blockloom_free` and `blockloom_call`.
Payloads are JSON or raw little-endian bytes, copied in and out through the
module's own allocator. A package with a `runtime.portable` entry and no native
library for the host target runs its `module` commands here
(`ActivePlugins::code_runtime`).

- **Isolation.** The module gets one memory capped at `memory_limit_mib` (a grow
  past it answers -1, a start past it refuses to load), one instance, and only
  the two host functions, so a module that imports anything else fails to
  start. Host services pass the same capability gate as native modules.
- **Time.** A call has `call_limit_ms * FUEL_PER_MS` units of interpreter fuel
  (about one per instruction, 500,000 per ms). It is a budget of work, not a
  clock. A call that spends it, or traps, stops the module: the owner drops it
  and the next call loads a fresh one, since its memory may be half-updated.
- **Dispatch.** wasmi's default tail-call dispatch relies on LLVM sibling-call
  optimisation and overflowed the stack on a runaway module under a
  debug-assertions build, so the workspace asks for `portable-dispatch`.
- **Cost** (container, release build): about 790 ns per empty call, against
  about 43 ns through the C ABI, and about 20 ns per item for a naive summing
  loop over a batch (that is interpreted work, not boundary cost). The spin
  loop burns fuel at about 530 million per second, which is where
  `FUEL_PER_MS` comes from. Run `wasm_call_cost` to repeat it. So portable
  hot paths must still batch, and anything per-voxel belongs to a native
  module or a compiled-in service. No JIT was tried.

Not done at the time: a browser host that runs the same modules (it needs the
player to load a wasm and cross a second boundary), the portable tier in a
built game (since added, see the browser host batch), and cancellation of a call from outside (only the fuel budget stops one).

## Plugin SDK (fourth batch)

`blockloom-plugin-sdk` is what an author depends on instead of the two ABI
documents. `Plugin::start` runs once per loaded module; `call` takes raw bytes
and by default hands JSON to `call_json`, so a bulk op overrides `call` and
keeps its own layout. The macro is the only target-specific thing a plugin
sees: the native expansion calls `sdk::entry::<P>` and fills the `PluginApi`
table with `call`, `free_buffer` and `shutdown` over a boxed instance (the
handle is its address, so one library can hold several modules); the wasm
expansion defines the four exports over one `static` slot, since a module is a
single instance on a single thread.

- **Panics.** Native catches them at the boundary, logs the message and answers
  `Panicked`; the module stays usable. A wasm std build aborts, so a panic hook
  logs first and the trap then stops the module (which the host reloads).
- **Errors.** `Error { status, message }` returns the status and logs the
  message at error level, so a failing command's reason reaches the run log.
- **Example.** `plugins/examples/tally` is one struct and one macro call, run
  four ways in its tests: the entry in-process, the built `cdylib`, the built
  wasm in the portable executor, and the sealed package (`Package::load`). Its
  wasm is about 140 KB with the default release profile (no `opt-level = "s"`
  or LTO tried). The wasm tests skip without the `wasm32-unknown-unknown`
  target unless `BLOCKLOOM_REQUIRE_WASM` is set, which CI does.
- **Not done:** a manifest generator (the author still writes `plugin.json`
  and the schemas by hand and seals with `plugin-seal`), a `cargo` subcommand
  that builds and stages both tiers, typed op helpers, and crates.io
  publication of the SDK (it is a path dependency for now, so the manifest's
  `sdk` range has nothing to check against yet).

## Plugin blocks in the palette (fifth batch)

A plugin's statement blocks show in the sidebar under their schema `category`
and draw on the canvas from the schema `label`. There is still one
`PluginBlock` instruction type, so blockstitch's one-row-per-type registry
needed two small additions on the blockstitch side: a row's `head` may be a
function of the instruction, and a value piece may carry `index` to read and
write one entry of an array held under its key (here `args`). Blockloom pins
the blockstitch commit that has them; until it is merged to blockstitch's
`qml` branch the pin points at the thread branch's commit. An uninstalled
plugin's block still draws, as its `plugin/block` name with "(not installed)"
and no slots, and Play refuses it (preflight). The QML half is unrun here (no
Qt in the container); the Rust half pins the snapshot shape the QML reads.

## Plugin code in the running world (sixth batch)

A plugin's module now runs inside the game world, not only the editor. The
editor sends a serializable `Loadout` (runtime, hooks, module-op blocks)
before `Start`; the world opens the modules at `begin_run` and drops them at
`end_run`, so state never outlives a run. Decisions:

- Plugins reach the world only through effects in their answers (`say`,
  `broadcast`, `error`). That keeps the ABI small and the world single-owner;
  richer effects are added by name as the voxel plugin needs them.
- Hooks are per `Stage` rather than per Bevy system, so a plugin cannot
  depend on engine internals. Within a stage they are ordered by their own
  `before`/`after`, and a cycle is reported once and the run goes without hooks.
- A block whose command is a module op runs in the world with no editor round
  trip; a block whose command edits the project still goes to the editor.
- `world.start`/`world.stop` are ordinary ops: a module that does not know
  them answers `Unsupported`, which the world treats as nothing to do.
- Reporters and hats were not here yet; the seventh batch adds them.

The GPU and windowed paths are unchecked here (no GPU in the container); the
host crate's tests drive the lifecycle with native fixtures, and the runtime
has no-op path tests.

## Plugin reporters and hats (seventh batch)

A reporter is a value, a hat is a header. Decisions:

- The plan was to publish reporter answers as snapshots at a stage, so the
  fixed tick never paid a call per read. That is replaced by on-demand reads:
  the VM already evaluates a slot only when a block runs, a read is one op call
  on a module that is already open, and a snapshot would have forced every
  plugin to guess which values a project reads. The cost is bounded instead by
  memoizing a module's answers until it is called for anything else or a
  frame starts, which makes a loop asking the same question each step one
  call. Modules are the same single-threaded objects the hooks use, so a read
  mid-tick cannot race a hook.
- A read may log but not act: effects in a reporter's answer are reported as
  an error and dropped, so evaluating a slot can never change the world.
- A reporter in the editor, outside a run, is an error that says so. There is
  no module to ask, and answering a stale or default value would hide that.
- One generic operator (`PluginRead`) and one generic header (`WhenPlugin`)
  stand for every plugin block, as `PluginBlock` does for statements, so the
  document stays readable when the plugin is missing and the VM, wire format
  and blockstitch need no per-plugin knowledge. The cost is two blockstitch
  features (an operator `layout` and `result` that are functions of the value,
  and `index` on text and dropdown pieces).
- A hat's slots are literal text, blank for any, because a header has no value
  slots. A module raises its event with an `event` effect naming the plugin's
  own event, optionally an actor, and the args the slots match.
- Both are VM only. Codegen refuses them by name, and scripts do not hear
  plugin events yet.

The tally example gains a "tally of" reporter and a "when tally changes" hat;
the host and example tests drive both through the real ABI, the VM tests
through a fake reader, and the QML half is unrun here (no Qt in the container).

## Plugin code in a built game (eighth batch)

The desktop player now hosts the plugin code a build ships. Decisions:

- The build ships `plugin.json` with the files. The manifest was never
  copied, so a player could not tell what it had; it is the one file the player
  needs to check everything else.
- The player does not resolve or install anything. It trusts only the pack's
  record: `Package::load_shipped` requires the manifest to hash (with its
  declared file table) to the pack's `hash`, and each shipped file to match the
  hash the manifest declares. Files a player has no use for are legitimately
  absent, so unlike an installed package a shipped one is not required to
  contain every declared file.
- A plugin that cannot load (damaged, wrong target, no artifact) stops the
  game with the reasons. Running without it would play a different game.
- The editor's Play and the player build the same `Loadout` from the same
  per-plugin code (`loadout_plugin`, `code_runtime_of`), so what runs in a
  build is what ran in Play.
- A build accepts what the world can run by itself: statements and reporters
  whose commands are module ops, and hats. A statement whose command edits the
  project still needs the editor and still stops the build. A project with
  plugin blocks still ships native logic: statements, reporters and hats
  compile (`ACT_PLUGIN_CALL`, the generic sensing read, a `Plugin` entry
  keyed by plugin, event and slots), and `tests/codegen.rs` compares each with
  the VM.
- Web and Android builds still refuse plugin code: the first needs a browser
  host for portable modules, the second a player built with the host crate.

The tally example's test seals the package, lays it out as the build does,
loads it through the player's path, reads a reporter and runs a block, and
checks a wrong hash and an altered file are refused. The player's launch path
itself (a window, a real game folder) is unrun here.

## Mesh service and the voxel plugin (ninth batch)

The first piece of the plan's phase 3: plugins can put geometry in the world,
and `com.blockworked.voxel` uses it. Decisions:

- A mesh is flat arrays in an effect, not a handle to GPU memory. It is the
  simplest thing a portable module can build, it crosses the ABI as JSON like
  everything else, and the world stays the owner of the entity and its buffers.
  The cost is the JSON size: numbers are rounded to five decimals, and a
  greedy-meshed chunk is a few hundred floats. The plan's compute-to-buffer
  and instanced paths are later services, not replacements.
- A mesh is identified by (plugin, name) so an edit replaces one chunk's
  entity. A plugin that wants fewer entities merges before it submits.
- Collision is the mesh's own triangles on a fixed body. The plan's pending
  barrier and collision-ready events wait for revisioned jobs.
- Emission is per mesh, not per vertex: a standard material has one emissive
  color. The voxel plugin therefore splits each glowing material into its own
  group. A per-vertex emissive channel needs a custom material and is left for
  the render service.
- The voxel plugin keeps the cells on the CPU in 16-cell chunks of material
  ids. The plan's 32-cell bricks, halos and quantised density belong with
  smooth terrain; 16 keeps a remesh to 4096 cells, and measuring 16/32/64
  stays an open item.
- Generation is deterministic from (seed, coordinates) only, so load order
  cannot change a world. Edits are run-local and the next Play starts from the
  generated base, matching how the engine treats play mutations.
- Rays take world units, not cells: a block author aims from an actor's
  position, so the plugin converts through the origin and voxel size. A cell
  walk (not a physics query) keeps the answer exact for the grid, including
  cells whose chunk meshes have not been rebuilt, and it works without a
  collider (`solid` off). `place` builds only against a face, so a ray that
  starts inside a cube does nothing.
- The scene-view preview is opt-in (`editor.preview` in the manifest) and runs
  only `world.start` with `preview: true`: no hooks, no blocks, no reporters,
  and anything the module says is dropped. A plugin that would act on the world
  must not do so from a preview, so the flag is in the manifest where a
  reviewer sees it. It is keyed by the loadout plus the plugin's records, so
  an unrelated edit does not pay the module's start again, and it is 3D only
  since the mesh service is.
- Portable first: the default 64x32x64 island is generated, meshed and
  serialised by the wasm module in about 0.9 s on this machine (release wasm,
  debug host), with a 10 s manifest budget. A larger world wants the native
  tier or streaming.

The plugin's tests cover the mesher (face counts, chunk borders, winding,
glow groups), terrain determinism, the palette, the host path (a run draws the
world, edits redraw only touched chunks, bad input is refused with a reason)
and the sealed wasm package producing the same meshes as the native build. The
runtime test takes a real voxel module through `plugins::install` to meshes
with colliders in the ECS and back to none at the end of the run. The pixels
(a window, lighting on the meshes) are unrun here.

## Browser host for portable modules (tenth batch)

A web build now ships portable plugins and the web player hosts them. The
choice was to run the module under wasmi inside the player's own wasm, not on
the page's `WebAssembly`:

- One executor, one set of limits. Fuel, the memory cap, the import whitelist
  and the stop-on-fault rule are the code the editor and the desktop player
  already run, so a module cannot behave differently in a browser by
  construction, and no JS glue or second ABI shim exists to keep in step.
- The host crate builds for wasm32 by leaving out `libloading`; a native
  library in a browser is a plain error. Files go through
  `files::set_reader`, which the web player points at `vfs`, so
  `Package::load_shipped` verifies a plugin against its pack hash exactly as
  on desktop.
- The cost is speed: an interpreter inside an interpreter. The browser's own
  engine would run the same module several times faster, and a plugin that
  does per-voxel work should not rely on wasmi in a page. That is the next
  step, behind the same `CodeModule` handle; this batch settles the semantics
  first, as the plan asks.
- The ship plan already says which targets a plugin supports: a native plugin
  with a portable fallback ships its portable module for web, and one without
  is refused with the reason. Android still refuses code.

Checked: the web build test (plugin files and pack record in the page's
archive), an integration test that loads the sealed voxel package for the web
target from an in-memory table with the package gone from disk and draws its
world, and clippy for the runtime on wasm32 with the feature. Not run here:
the page in a browser (WebGPU has no software path in this container), so
`just web-build` on a project with a portable plugin and `just web-smoke` are
the remaining proof.

## Importers and build hooks (eleventh batch)

- The plan asks for importers in workers with deadlines and project-scoped IO.
  Here the host is the only thing that touches files: a module is handed the
  source's bytes and answers with the files it made, so an importer needs no
  capability and cannot read or write anything it was not given. The wire form
  is a JSON header line followed by raw bytes, not base64, since sources and
  products can be large images or models.
- The deadline is the portable module's fuel for one call (`limit_ms` in the
  schema, at most two minutes); a native library is trusted and untimed, as
  for every other native call. A call that runs out stops the module, which
  the editor replaces on the next use.
- Outputs go beside the source in `<file>.imported/` and never import again.
  `.blockloom/imports.json` keeps hashes of the source, any `dependencies` the
  module named and each output, so staleness is answered from the disk and a
  hand-edited output is told apart from a changed source. Re-importing deletes
  what the previous run wrote and the new one did not.
- A build hook sees the target and every asset path and may add files, which
  ship under `game/plugins/<id>/cooked/` through `BuildOptions::extras`, or
  fail the build. Android builds ship extras too (fourteenth batch). A package that only
  has importers and hooks is not shipped: it runs in the editor.
- Not yet: importer-declared thumbnails, and hooks whose output a
  shipped plugin reads (there is no host service to open a cooked file).

## HTTP registry and attached copies (twelfth batch)

- The registry protocol was already files (an index and immutable archives),
  so the HTTP transport is a reader of a published folder: any static host
  works and there is no server to run. Every archive is checked against the
  sha256 in the index, so a mirror can't swap a package; trust rests on the
  index's channel, which must be https (plain http only for this machine).
  Redirects are limited to https for the same reason. The archive path in an
  index must be plain-relative so an index can't send the client elsewhere.
- Not yet: authentication, an index signature, a search or yank API, and
  publishing over HTTP (upload the published folder with your own tooling).
- A package change only writes `plugins.json` and the lock, which the revision
  counter did not cover, so an attached copy kept its old active set. The
  owner now bumps the revision after every package change and an attached
  copy reloads its plugins whenever it reloads the folder.

## Schema-generated inspectors (thirteenth batch)

- The inspector needs no plugin code for a declarative plugin: the backend
  sends each type's fields and defaults (`types` in `plugin_list` and the
  snapshot), and two compile-time QML files draw them. Every field type has an
  editor, lists nest, and numbers are clamped to the schema's bounds, but the
  backend still validates and is what undo records, so a form can't write
  something the schema refuses.
- A form's edit is the whole next payload built from the held one, so keys a
  newer plugin wrote survive an older plugin's edit. A record whose plugin is
  missing shows a note and stays untouched, and one at another schema version
  asks for `plugin-migrate` rather than being edited in a shape the form
  doesn't know.
- Not yet: a plugin drawing its own inspector section, panels, and loading
  trusted editor QML at run time. The QML tests were written without a Qt
  install in the build container, so run `just qml-test` locally.

## Import freshness and Android hooks (fourteenth batch)

- An import records hashes, so staleness is a comparison, not a watcher: a
  source changed outside the editor is noticed when the tray refreshes and is
  imported again before Play or a Build (`plugin-reimport`). Output the user
  edited by hand is never overwritten by that sweep; naming the file does it.
  An attached copy never sweeps, since it can't write the owner's files.
- Android builds copy build hook files into the APK's assets next to the
  game, and the Android cache fingerprint hashes their bytes (the staged
  `.blockloom/cooked` folder is rewritten every build, so its mtimes are not
  part of it). Plugin code is still refused on Android.

## Inspector layout hints (fifteenth batch)

- A schema may say how a component is drawn without a plugin shipping UI
  code: per-field `ui` (label, slider or multiline widget, unit, step,
  `visible_when` against another field of the record) and `inspector.groups`.
  Hints are checked when the package is verified and carry no data, so a
  record validates and migrates exactly as before. A hidden field keeps its
  value. Plugin-drawn panels and trusted editor modules are still open.

## Saved voxel edits (sixteenth batch)

- Edits that outlive a run are data in the `world` resource: a list of text
  lines, replayed over the generated terrain at `world.start`. Lines keep the
  document diffable and need no new schema type; a bad line is reported and
  skipped, so one typo does not lose the rest.
- Writing them needed one general command action, `set_resource_field`
  (replace one field of a resource, or `append` to a list field), so any
  package can build a log or a list from commands. `set_world` still replaces
  the whole resource, which includes its edits.
- Editor brushes that write these lines from the scene view are still open.

## Declarative panels (seventeenth batch)

- Editor panels are data, like inspectors: a package lists items (text, a
  resource form, a command button) and the editor draws them, so a plugin
  can have a settings and tools window with no UI code and nothing to trust.
  Running QML a plugin ships needs a trust decision per package and a way to
  keep it from the document; that stays open.

## Shaped voxel cells (eighteenth batch)

- A cell is still one material byte; its shape lives in a sparse table, so a
  world of cubes costs nothing extra and the greedy mesher is untouched (it
  only sees whole cubes). Shaped cells come from a short face list (boxes, plus
  a wedge for ramps, turned about the vertical axis by facing), which covers
  slabs, posts, stairs and ramps; faces two boxes press together are dropped.
- Rays and the break/place blocks work in whole cells, so they treat a slab as
  a cube. Collision is the mesh, so it is the real shape.


## Scene tools and brushes (nineteenth batch)

- A plugin tool is data, like a panel: the plugin names a cast op, a command
  and where each command argument comes from (`$hit.path`, `$option.name`).
  The world only casts and reports the hit, and the editor runs the command, so
  a tool's edit is an ordinary undoable command and the preview redraws by the
  usual reload. Nothing a plugin ships runs in the editor.
- The cast is a read-only module call (`WorldPlugins::query`) on the module the
  scene-view preview already hosts. Clicks cost one call; drag painting and a
  hover outline would cost one per frame and were left out.
- `set_resource_field` gained a `template`, so a brush command is the voxel
  edit line it saves with its arguments filled in. `paint_voxel*` need no
  module op, so they work from the shell and MCP as well as from a click.
- `SceneView` grew a boxed `plugin_tool` rather than a field per tool, so the
  protocol never needs to know a plugin's tools (`PROTOCOL_VERSION` 27).

## Trusted editor modules (twentieth batch)

- QML a plugin ships runs in the editor's engine, so it can do anything the
  editor can. The manifest already forced a `trusted-editor` capability; the
  new part is that nothing loads until the user says so, once per package
  content. The ledger is outside the project (a project cannot trust itself)
  and names the content hash, so updating the package, which is how code
  changes, asks again.
- The one command that grants trust has no shell or MCP name and is refused on
  the attach socket, so an agent that can install a package cannot also make
  the editor run it. The dev bridge serves raw dispatch names and is out of
  the workspace; a person at a keyboard on that machine could always edit the
  ledger file, which is the same trust boundary as the editor itself.
- Modules get a small `host` rather than the module system's imports, but
  `host.app` is the editor, so this is a convenience and not a sandbox. Only
  QML is accepted; native editor modules stay a later question. Builds never
  carry editor modules.
- Not run here: there is no Qt in the build container, so the dialog and its
  test have not been run; the backend (ledger, snapshot, commands, example
  package) is tested.

## Scene tool strokes, inspector sections and Android (twenty-first batch)

- A stroke is the hits of every pointer move between press and release, sent
  once on release, so the editor keeps one undo step however long the drag.
  The cast answers a `cell_box` for the outline and the stroke dedupes by
  cell. `PROTOCOL_VERSION` 28.
- An inspector section is a trusted QML module for one component, so it rides
  the existing trust ledger and hash; without trust the schema form shows.
  Nothing new is exposed beyond what `host.app` already gave a module.
- Native compiled editor modules are not planned: they would execute inside
  the Qt process with nothing like the portable fuel and memory limits, and
  trusted QML already reaches the editor. Revisit only with a real need.
- Android is portable-only, as the browser is: no library is read out of an
  APK. The host compiles for `aarch64-linux-android` (HTTP registries off) and
  so does the runtime with `--features plugins`; there is no NDK or device
  here, so the runtime build and a real run are unverified.
- A faster browser host is not built. A browser instance has no fuel, so a
  runaway call could hang the page; a design that fell back to wasmi per
  plugin needs an opt-in flag and a watchdog, and cannot be proven without a
  browser here.
- Not run here: QML tests and the runtime's own tests (no Qt, no linkable
  Bevy); the plugin api, host, voxel and app tests pass.

## Services, surfaces, rendering and the authoring kit (twenty-second batch)

- One `HostServices` hub answers every module's `host.call` in the editor and
  in the world. A provider chain lets the world add `physics.*`/`nav.*`
  without the host knowing about Bevy, and the capability gate stays where the
  call enters (`native.rs`), so a service is open or closed the same for
  native, portable and isolated modules.
- Storage is two namespaced stores with atomic multi-key commits and hard
  limits. Saves are separate from project blobs because a built game's
  project data is read-only while saves are written during play. Web saves
  were memory only here and moved to localStorage in the twenty-third batch.
- Jobs run in slices inside a per-frame budget and carry their state through
  the answer, so a plugin never needs a thread and a runaway job costs one
  slice. Generation graphs are data checked before they run; the cache is
  keyed by node, parameters and inputs so a parameter edit recomputes only the
  downstream nodes. `graph.evaluate` is a host job, not a module op.
- Surfaces are commands with fixed arguments (menus, shortcuts) or a
  read-only op (overlays), so none of them can change a project outside a
  validated, undoable command. Conflicts block a run only when the pair truly
  cannot work together (declared, shared service, impossible hook order); key
  clashes are reported but never block.
- Rendering stays in effects: instances reuse one mesh's buffers and the
  material so Bevy batches them, and a shader module is only importable by
  surface shaders, never a replacement for a Blockloom module. Plugin-owned
  compute passes were left out of this batch because a plugin holding the
  device has no budget, ordering or memory limit the host can enforce; the
  twenty-third batch offers compute with the host holding the device.
- Live reload is `world.save`/`world.restore` for portable modules only. A
  native library cannot be unloaded safely while its code may be on a stack,
  so a native change says a restart is needed.
- Isolation is opt-in (`BLOCKLOOM_PLUGIN_ISOLATION`). The worker owns the
  library; host services run in the host, so the capability gate cannot be
  bypassed from the worker. A call past its wall-clock limit kills the worker
  and the module is reloaded on the next call. The cost is a frame per call
  over a pipe, which is why it is not the default. The worker binary must
  ship beside the editor (packaged in the twenty-third batch).
- Kit: the scaffold's portable source is the SDK's own tested template, so
  what `plugin-new` writes compiles. A portable scaffold cannot be sealed
  until its wasm exists, so `build.sh` seals. The harness reuses the host's
  loader, so a plugin's tests see the same errors and statuses as a run, but
  there is no world: effects come back in the answer.
- Verified here: plugin api, host (including the isolation test with a real
  child process), SDK harness and scaffold, app and runtime plugin tests,
  wasm32 and aarch64-linux-android checks of the host. Not run here: QML
  (menus, shortcuts, overlays, manager rows), the GPU half of instancing and
  shader modules, and the Android runtime.

## GPU compute, blob GC, web saves, native template, worker packaging (twenty-third batch)

### GPU compute: the host holds the device

A plugin never gets a device, queue or pipeline. Compute is a conversation in
effects, like meshes:

- The package ships **kernels** (`kernels` in a schema file): a `.wgsl` file,
  an entry point, a workgroup size and the named bindings it uses (`read`,
  `read_write` or `uniform` buffers in group 0). The `gpu-compute` capability
  is required to ship one or to ask for any GPU effect, so the Plugin Manager
  and the install preview show it like any other capability.
- The module answers `gpu_buffer`, `gpu_write`, `gpu_dispatch`, `gpu_read` and
  `gpu_free` effects. Buffers are host-owned, named per plugin and made of
  32-bit words; a dispatch binds the kernel's binding names to buffer names.
  A read is answered later through the module's `gpu.result` op, so a plugin
  never blocks on the GPU and a frame never waits on a readback.
- **Validation** is `blockloom-plugin-gpu::check_kernel`, run when a package
  loads (so an install, a Play and a build all refuse a bad kernel): naga
  parses and validates with no optional capabilities (no textures, subgroup,
  ray or f16 features), the schema's entry point and workgroup size must
  match, group 0 buffers must match the schema's names, slots and access,
  and workgroup memory is capped. The device already clamps out-of-range
  buffer accesses, so the one way left to hang a GPU is a loop. Every loop
  must therefore be a `for` over a local counter that starts at a constant,
  is compared with a constant and steps by a constant, and the sum of all
  iterations (nested loops multiply, a call costs its callee) must stay within
  `MAX_STATIC_COST` (2^20). A `while`, `loop`, a limit read from a variable or
  `arrayLength`, a counter written in the body, or a counter that could wrap
  is refused, with the message showing the accepted shape. This is stricter
  than WGSL needs and that is intended: it is a proof, not a heuristic, and it
  was cheaper to write over naga's IR than a watchdog the platform cannot
  offer (wgpu has no kernel timeout and a hung GPU takes the editor's window
  with it).
- **Budget and ordering.** Commands wait in a per-plugin queue (at most 4096)
  and run in order inside one encoder per frame. A frame has an invocation
  budget (2^27, a single dispatch at most 2^26); a dispatch that does not fit
  waits for the next frame, in order, and queues are served round-robin so one
  plugin cannot starve the others. A write goes through a staging copy so it
  lands in order with the dispatches around it, not at the start of the frame
  as `queue.write_buffer` would. Each plugin may hold 64 buffers and 128 MiB,
  a buffer is at most 64 MiB, a read at most 4 Mi words with 16 in flight,
  and a buffer both written and bound twice in one dispatch is refused (it
  would be a wgpu usage conflict). Every refusal is an error in the run log;
  the commands after it still run.
- **Where it runs.** The engine (`ComputeEngine`) lives in the render world
  and runs once a frame in `RenderSystems::Cleanup` on Bevy's own device, so
  there is one device, no second adapter and no cross-device copy. The two
  worlds meet at `plugin_compute::ComputeLink`, a mutex of commands going in
  and reports coming out, because modules run on the main thread and the
  device does not. A world with no renderer says so once instead of dropping
  commands silently. A run ending drops every plugin's buffers and queued
  commands; kernels stay.
- **Not offered, on purpose.** Textures and samplers (a kernel cannot read or
  write what is drawn), push constants, indirect dispatch, shared buffers
  between plugins, and WESL imports in kernels (a kernel is one self-contained
  WGSL file; a library module would have to be linked and re-checked, which is
  worth doing only once a real kernel wants one). Portable (wasm) plugins use
  the same effects, so compute works for web and Android players wherever the
  world has a device.
- Verified here on lavapipe (a CPU Vulkan device): the engine runs a kernel
  over host buffers and reads the result back, an overwritten buffer is seen
  in order, bad commands are reported while the rest run, memory limits hold
  and free returns words, and work over the frame budget carries to the next
  frame (`cargo test -p blockloom-plugin-gpu --features engine -- --ignored`
  with `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`). Not run: a
  real GPU, the web player, and a plugin kernel inside a running game (the
  render-world system is compile-checked, and the effect path is tested up to
  the commands it produces).

### Blob GC

`plugin-data-gc [dryRun=true]` removes content blobs (`blobs/<sha256>`) that
nothing reaches, from the project's plugin data and from its player saves
(blobs only: a save's keys are the player's). A blob is reachable when a record
or resource payload in the document, or any other key its plugin has stored,
holds the string `blob:<sha256>` (the shape `put` returns); a reachable blob is
read too, so blobs may name blobs. It refuses while a game runs, because a blob
written but not yet named anywhere is garbage until it is, and it refuses to
collect from a read-only store while still reporting a dry run. It is
conservative about keys and aggressive about blobs: a plugin that keeps a hash
in some other shape loses the blob, so `dryRun=true` first is the habit and the
author guide says what counts as a reference.

### Web saves

`KvStore` stores player saves in any key-value backend (`KvBackend`, base64
values under a per-project prefix) and undoes the writes of a commit that
fails part way, so the browser gets the same atomic multi-key commit the disk
store has. The web player's backend is localStorage. Quota errors surface as
a failed commit, not a lost write.

### Native template and worker packaging

`plugin-new template=native` writes the portable crate plus a `build.sh` that
also builds this machine's `cdylib` and records it with
`plugin-add-native path=... target=... library=...` (manifest tier `native`,
the `native-execution` capability, the library under its target triple, sealed
again). Each platform's library is one more run of the script, and a target
with no library falls back to the portable module. The editor install, the CI archives and the Windows installer
now carry `blockloom-plugin-worker` beside the editor, which is where
isolation looks for it. A built game does not ship the worker: isolation is an
editor and developer setting, and a built game runs the plugin where it
always did.
