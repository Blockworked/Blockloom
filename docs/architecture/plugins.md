# Plugins

The plugin platform: api, host, SDK, modules, blocks, scene tools, services, engine plugins and the voxel plugin. Split out of `CLAUDE.md`; the index there lists every page.

## Plugins

Design: `docs/plugin-system-and-voxel-plan.md`; decisions, measurements and
deviations of the first implementation: `docs/plugin-adr-0001.md`.

- **`blockloom-plugin-api`** has no Qt, Bevy or I/O: `manifest.rs`
  (`plugin.json`, tiers, capabilities, per-target runtime entries), `schema.rs`
  (component/resource/block/command/hook schemas, field validation,
  migrations), `record.rs` (`PluginRecord`), `abi.rs` (the C ABI v1) and `id.rs`.
  `blockloom-core` depends on this crate and never on the host, which has
  libloading and git and must not reach the wasm build. Bump
  `versions::PLUGIN_ABI` only when `abi.rs` changes.
- **`blockloom-plugin-host`** is everything that touches disk or code:
  `package` (verify, seal, zip), `source` (`path:`, `archive:`, `git:url#commit`,
  `registry:`), `registry` (folder registries), `cache` (immutable shared
  cache), `lock` (`plugins.json`, `plugins.lock`, `plugins.local.json`,
  `.blockloom/plugin-history`), `resolver`, `install` (plan/apply/rollback),
  `active` (what a project loads, record audit, migrations, ship plan),
  `hooks`, `lifecycle` and `native` (library loader, capability gate).
- **Data**: a plugin's data is an opaque `PluginRecord` in the document, either
  `ActorComponent::Plugin` on an actor or `Project.plugin_resources`. Core never
  reads a payload, so a missing, newer or broken plugin loses nothing on open
  and save. `ActivePlugins::audit` classifies each record, and
  `commands::plugins::preflight` refuses Play and Build while one that is not
  `editor_only` has no working plugin, naming the record and the fix. A
  migration (`plugin-migrate`) is all records or none and snapshots first.
- **Commands** live in `blockloom-app/src/commands/plugins.rs`: install, remove,
  update, pin, sync, rollback, registry, gc, seal, publish, migrate, check,
  list, plus `add_plugin_component`, `set_plugin_component`,
  `set_plugin_resource`, `plugin_inspect` (verify a folder and say what it is)
  and `plugin_call`. Package changes are transactions on
  `plugins.json` and `plugins.lock` only. A command a plugin declares is run as
  `plugin-id/name key=value` in the shell (`Action::Plugin`, forwarded as
  `plugin_call`) and through the MCP `plugin-call` tool. A command's action is
  `add_component`, `set_field`, `set_resource`, `set_resource_field` or
  `module`. `StateDto.plugins` is the snapshot slot.
- **Builds**: a pack that carries plugins is `PACK_VERSION` 2 (without them it
  is still written as 1), each plugin's `plugin.json` and shipped files go to
  `game/plugins/<id>/`, and Android builds refuse plugins that have code. A web
  build ships portable ones (the page's mounted files hold `plugins/<id>/`,
  `Launch::Web` sends the same `Plugins` loadout for target
  `wasm32-unknown-unknown`, and `blockloom_plugin_host::files::set_reader` points
  package reads at `vfs`); a code plugin with no portable module is refused for
  web by the ship plan. The desktop player hosts that code: `Launch::Player` hands the world
  an `EditorMessage::Plugins` before `Start`, built by
  `plugins::shipped_loadout` from the pack's `PackedPlugin`s through
  `blockloom_plugin_host::shipped` (`Package::load_shipped` checks the manifest
  against the pack's content hash and each shipped file against its declared
  hash; a missing or altered file, or no artifact for the target, stops the
  game with the reasons, since it must not run without its plugin). A Build
  accepts plugin blocks the world can run itself (`block_runs_in_world`: a
  module-op statement or reporter, any hat) and refuses a statement whose
  command runs in the editor. Plugin blocks compile too (see below), so a
  build keeps native logic.
- **Native modules** load in the editor on a command's first use
  (`commands::plugins::Modules` on `OpenProject`, keyed by package hash, dropped
  when the package changes or the project closes): a `module` command action
  calls `NativeModule::call_json`, the module's logs go to the run log, and a
  panic is an error, not a crash. A module may be called from any thread but
  one at a time, hence the `Mutex`.
- **Portable modules** (`runtime.portable`, `blockloom-plugin-host/src/portable.rs`,
  contract in `blockloom-plugin-api/src/wasm.rs`) run a `.wasm` through wasmi
  with portable dispatch: one capped memory, only the `blockloom.log`/`call`
  imports, and `call_limit_ms * FUEL_PER_MS` fuel per call. A call that runs out
  or traps stops the module and `Modules::get` loads a fresh one next time.
  `Modules` holds a `Module::{Native, Portable}`; a plugin with a native library
  for the target uses it, else its portable module. Editor only so far.
- **Plugin statement blocks**: the document holds one generic
  `InstructionKind::PluginBlock { plugin, block, args }` (args in the block
  schema's slot order, `FieldId::PluginArg(i)`), compiled to
  `Action::PluginCall` and run by the VM as `Effect::PluginCall`, whole numbers
  as JSON integers. The world forwards it as `RuntimeMessage::PluginCall`, and
  the editor runs it (`commands::plugins::run_block`: slots named by the schema,
  an `actor` argument filled from the running actor, an actor slot's name
  resolved to an id, then the same `plugin_call` path). Codegen emits it as
  `Act::PluginCall` (slots read into `let`s in order, then one host act; over
  the logic ABI as `ACT_PLUGIN_CALL` with the slots as a JSON array in `c`,
  whole numbers as integers, which `logic.rs` turns back into the same
  `Effect::PluginCall`), and `preflight` stops Play on a
  block no installed plugin provides or whose slot count changed. A Build is
  refused for a statement whose command runs in the editor, since a built game
  has none (module-op blocks ship, see Builds). `plugin-run-block` runs one from the shell and MCP, and the snapshot's
  `plugins.blocks` lists every available block. The palette draws the
  statement ones (`Blocks.qml`'s plugin section, grouped by category): the
  one `PluginBlock` row's `head` is a function of the instruction that splits
  the schema's `label` into label pieces and value pieces (`key: "args"`,
  `index: i`, field `PluginArg:i`), cached so the canvas keeps its controls.
  `head` as a function and `index` are blockstitch row features.
- **Plugin code in the game world** (`blockloom-runtime/src/plugins.rs`,
  `blockloom-plugin-host/src/world.rs`, `blockloom-plugin-api/src/loadout.rs`).
  Play sends `EditorMessage::Plugins { loadout }` between `Load` and `Start`
  (`PROTOCOL_VERSION` 26): per plugin its code runtime, hooks and the blocks
  whose commands are module ops (`ActivePlugins::loadout`). `world::begin_run`
  opens each module (`WorldPlugins::load`) and calls `world.start` with the
  plugin's records and resources; `end_run` calls `world.stop` and drops them,
  so a module lives exactly as long as a run. Hooks are named by a `Stage`
  (`Input`, `PreSimulation`, `FixedSimulation`, `EffectApplication`,
  `PostPhysics` in the fixed step, `RenderExtraction` and `Presentation` per
  frame), ordered by their `before`/`after`, and called once per stage as op
  `hook.<name>` with `{stage, hook, tick, dt}`; a paused world skips the fixed
  stages. A module answers `{"effects": [...]}` of `say`, `broadcast` and
  `error`, which the world applies in order - a plugin never touches the
  world itself. An `Unsupported` answer means "nothing to do" (once, as an
  error, for a hook). A faulted portable module is dropped for the rest of the
  run. A plugin block whose command is a module op runs in the world
  (`plugins::run_block`); any other still goes to the editor as
  `RuntimeMessage::PluginCall`. The runtime's `plugins` feature (default) pulls
  in the host crate, which builds for wasm32 without `libloading` (a browser
  has no native libraries; the portable executor is wasmi running inside the
  player's wasm). The web player is built with `--features plugins`; Android
  builds without it and ignores the loadout.
  `plugins/examples/tally` answers `world.stop` with a final tally line.
  A plugin whose manifest says `editor.preview` is also hosted while nothing
  plays (`plugins::preview`, 3D only): the world opens it on `Load`, `Plugins`
  and the end of a run, starts it with `preview: true` and draws what it
  answers, but runs no hooks or blocks and drops `say`s. The module is kept
  while its loadout and records are unchanged and started over when they
  change; Play replaces it with the run's own. The editor resends the loadout
  with every idle sync, so installing or removing a plugin updates the view.
- **Plugin reporters and hats** are generic like the statement. A reporter is
  the value `Op::Ext("PluginRead")` (`value::PLUGIN_READ`) with args `[plugin,
  block, ...slots]`; its operator calls `sense::plugin_read`, which asks a
  thread-local reader that the world installs while a run has modules open
  (`plugins::begin`/`end`). The editor has none, so there a reporter errors
  with "only answers while the game is running". `plugins::read` turns the
  slots into JSON by the schema (an actor slot's name into its id), calls the
  block's module op as the asking actor (`sense::current_actor`) and takes the
  answer's `value` as the schema's `returns` type; a mismatch is an error.
  `WorldPlugins::read` remembers answers per module until the module is called
  for anything else or a frame starts (`forget_reads`, from the Input and
  Presentation stages), and a read may log but not act. A hat is
  `InstructionKind::WhenPlugin { plugin, block, event, args }`, compiled to
  `Trigger::Plugin` and started by `Event::Plugin { plugin, event, args, actor
  }`, which a module raises with an `event` effect (`{name, actor?, args}`).
  Hat slots are literal text, blank for any, and match as text or as numbers.
  Both compile: a reporter is the generic sensing read (`sense(h, &me,
  "PluginRead", ..)`, answered by the host's `ext_operator`, so the world's
  installed reader answers it exactly as for the VM), and a hat is an
  `Entry` whose `detail` is `plugin_detail` (plugin, event and slots joined by
  U+001F), started by `fire("Plugin", actor, detail, "")` - with an actor only
  that actor's strand, without one every copy's - and matched as the VM does
  (`plugin_hat_matches`). `tests/codegen.rs` holds statement, reporter and hat
  against the VM line for line. A script
  reaches the same three through the script ABI:
  `Actor::plugin_call(plugin, block, &[PluginArg])` is `ACT_PLUGIN_CALL`
  (`c` = slots as JSON), `plugin_number`/`plugin_text` are `READ_PLUGIN`/
  `TEXT_PLUGIN` (`a` = plugin, `b` = block, U+001F, slots as JSON; answered by
  the thread's plugin reader, failures go to the run log), and `Event::Plugin`
  is `EVENT_PLUGIN` (subject the event, detail plugin then slot texts joined
  by U+001F), sent to every script or only the actor the event names.
  `Project::plugin_blocks` reports all three shapes (`PluginBlockShape`) so
  `preflight` checks each one (kind, slot count, and that a reporter's command
  is a module op), and a Build is refused only for a statement whose command
  runs in the editor. In the
  palette the operator `PluginRead` has a `layout` and `result` that are
  functions of the value, and the `WhenPlugin` header's `head` uses text pieces
  with an `index` (both blockstitch features). `plugins/examples/tally` has
  "tally of" and "when tally changes" blocks; its `add` raises `changed` once
  a run hosts it.
- **Plugin SDK** (`blockloom-plugin-sdk`): a plugin author implements `Plugin`
  (`start`, then `call` over bytes or `call_json` over `serde_json::Value`) and
  names it with `export_plugin!`. The macro expands to the C entry symbol on a
  desktop target and to the portable module's four exports on
  `wasm32-unknown-unknown`, so one source builds as a `cdylib` or a `.wasm`
  with no pointers in sight. `Host` gives `log` and `call` (host services); an
  `Error` carries the `Status` the host sees and is logged. A panic is caught
  on native and answers `Panicked`; on wasm a panic hook logs it before the
  module traps. `NativeModule::from_entry` starts a plugin's entry function
  in-process, which is how the SDK's tests (`blockloom-plugin-sdk/tests`) and
  `plugins/examples/tally` run the real ABI without a library on disk. The
  tally example's package source is `plugins/examples/tally/package`; `just
  example-plugin` stages and seals it with its wasm into `target/plugins/`.
  A change to `abi.rs` or `wasm.rs` goes through the SDK too.
- **Plugin Manager**: `PluginManagerDialog.qml`, opened from the top bar's plug
  button. It lists `plugin_list`/`plugin_check`/`plugin_commands` and runs the
  same `plugin_*` commands the shell does (install with a dry-run preview,
  update, remove, sync, undo the last change, clean the cache). A "Settings"
  section edits each plugin resource in place. It has QML tests
  (`tests/qml/tst_Plugins.qml`).
- **Schema-generated inspectors** (`PluginRecordForm.qml` over `RegistryForm`, and
  `PluginValueEditor.qml` for scene tool options):
  `plugin_list` and the state snapshot's `plugins.types` list every component
  and resource type (`name`, `displayName`, `kind`, `version`, `fields`,
  `defaults`), built by `types_json` from the installed schemas. A form has a
  row per field drawn by blockstitch's `PropertyEditor` (`bool`, `int`,
  `number`, `text`, `color`, `vec3`, `choice`, `asset`, `actor`, and `list` of
  any of those, recursively), clamped to the schema's bounds; a sourced
  `choice` lists `plugins.choices` (`RegistryForm.choices`). An edit reports the whole
  next payload with any key the schema doesn't name kept, so an older plugin
  never trims a newer document. `InspectorPanel.qml` draws a `Plugin` component
  as that form (`pluginCard`), says so when the plugin is missing or the record
  needs a migration and keeps the data, and lists installed component types in
  Add component (`plugin:<name>` -> `add_plugin_component`). Editing goes
  through `set_plugin_component`/`set_plugin_resource`, so validation and undo
  are the backend's. QML tests: `tests/qml/tst_PluginInspector.qml`. A new
  `FieldType` must be taught to blockstitch's `PropertyEditor.qml`,
  `PluginValueEditor.qml` and `src/plugins.ts`.
  Drawing hints are declarative and never touch a payload: a field's `ui`
  (`label`, `widget` of `slider` for a bounded int/number or `multiline` for
  text, `unit`, `step`, `visible_when` of `{field, equals | not_equals}` or
  truthy) and a component's `inspector.groups` (`label`, `fields`, `collapsed`).
  `check_ui` in `schema.rs` refuses a hint that names a missing field, suits the
  wrong type, or draws a field twice; the form draws ungrouped fields first,
  then each group under a foldable heading, and hides a row while its
  `visible_when` fails (the value stays in the record).
- **Plugin panels** (`PluginPanelView.qml`, `Contributions::panels`): a
  package's `panels` list a name, title and items - `text`, `resource` (a
  `PluginRecordForm` over one of its resources, written through
  `set_plugin_resource`) and `command` (a form over the command's arguments and
  a button that runs `plugin_call`). `check_definition` refuses an unknown
  resource or command. The snapshot's `plugins.panels` carries each panel with
  its owner and the commands its buttons run; each panel is a dock tab
  (bottom by default) like the built-in panels. QML test: `tests/qml/tst_PluginPanels.qml`.
- **Plugin scene tools** (`ToolSchema`, `Contributions::tools`): a package's
  `tools` name a `cast` module op, a `reach`, a `command`, that command's `args`
  (`$hit.cell.0` reads the cast's answer, `$option.name` an option, anything
  else is text) and typed `options`. `check_definition` refuses an unknown
  command, argument or option. The snapshot's `plugins.tools` lists them and the
  scene view's toolbar gets a toggle per tool in 3D, with the options drawn by
  `PluginValueEditor` under it (kept in the QML `Settings` as JSON per tool).
  A `choice` with a `source` takes its list from the plugin: when a preview
  starts, the world asks each module's `choices` op (`{source: [values]}`) and
  sends `RuntimeMessage::PluginChoices` (`PROTOCOL_VERSION` 40), kept as the
  snapshot's `plugins.choices`; `options` stay the fallback and any text
  validates (the voxel material fields list the project's blocks this way).
  `SceneTool::Plugin` plus `SceneView::plugin_tool` (`PluginToolView`: plugin,
  tool, cast op, reach, outline, drag, option values; `PROTOCOL_VERSION` 28) tell the world
  which tool is out. A left click casts the pointer ray through the hosted
  preview module (`plugins::tool_click`, `WorldPlugins::query`: `{x y z dx dy dz
  reach}` in, `{hit, ...}` out, read only) and a hit goes back as
  `RuntimeMessage::PluginTool`; the editor resolves the arguments
  (`ToolSchema::resolve_args`) and runs the command (`commands::plugins::run_tool`,
  `plugin-run-tool` in the shell and MCP), one undo step, which reloads the
  world and so redraws the preview. A tool with `drag` paints a stroke: the
  button down casts every pointer move (`plugins::tool_stroke`, a cell once per
  stroke) and the release sends all the hits as one `PluginTool`, which
  `run_tool` runs batched as a single undo step (Escape or losing focus drops
  it). A tool with `outline` (a `$`-less path into the cast's answer holding
  `[x0 y0 z0 x1 y1 z1]`, read by `schema::box_at`) gets a gizmo box under the
  pointer that follows the hover cast (`plugins::tool_outline`,
  `edit::follow_plugin_tool`). The voxel tools use both.
- **Mesh service** (`blockloom-plugin-api/src/mesh.rs`, `blockloom-runtime/src/
  plugin_meshes.rs`): a module answers with `{"effect": "mesh", name,
  positions, normals, colors, indices, origin, emission, roughness, collider}`
  (flat arrays, so any tier builds one) or `{"effect": "remove_mesh", name}`.
  The host checks a mesh (`MeshData::check`: array lengths agree, indices in
  range, finite, at most `MAX_VERTICES`) and reports a bad one as an error.
  `blend` (opaque, cutout as `AlphaMode::Mask(0.5)`, translucent as `Blend`)
  picks the alpha mode, and `texture_layers` plus a `layers` index per vertex
  draw through `plugin_arrays.rs` (an `ArrayMaterial`: the layer rides
  `UV_1`, the array is filled once every source image settles, at the
  largest one's size, a missing one white; its shader has run on no GPU here).
  `plugins::apply` queues a `MeshOp` on `engine.plugins.meshes` (a 2D game
  reports that only 3D can show one) and `plugin_meshes::sync` (Update, 3D)
  carries them out: one entity per (plugin, name) with `Mesh3d`, a standard
  material shared by roughness and emission (vertex colors multiply the base),
  and a fixed trimesh collider when `collider` is set. A same-named mesh
  replaces the old entity. `mesh_visibility` changes drawing and retains the
  collider, including on replacement; `plugins::end` queues `Clear`. These are not
  actors: no batching, LOD or occlusion, and nothing else reads them yet.
- **Voxel plugin** (`plugins/voxel`, `com.blockworked.voxel`, phase 3's first
  slice): a finite world of cubes, SDK-built. Since 0.4.0 it is engine-tier
  only (adapter, no portable module, so desktop builds only): the workspace
  crate keeps its tests, and `just voxel-plugin` stages `package/` with
  `src/` copied under `engine/`, whose standalone manifest builds the same
  sources with the `engine` feature, where `engine::VoxelModule` links the
  plugin's `export_plugin!` entry. Its `world`
  resource (preset `island|caves|flat|empty`, seed, size, voxel size, origin,
  solid, palette colors/emission) is read at `world.start`, its origin measured
  from a Voxel world anchor (`voxel_world`) actor's start when one exists; `grid.rs` holds
  32-cell sections of 16-bit block ids (`Block`, palette-compressed by `section.rs`
  at 0 to 16 bits a cell; `KEEP` 65535 marks an unchanged cell in checkpoints and
  `.blockworld`, older checkpoints say 256) with exact logical bounds and full-height
  32 by 32 columns (air sections unallocated, a boundary edit
  dirties the neighbour), `terrain.rs` generates from hashes of each cell's own
  coordinates (the same seed always gives the same cells, trees and ore
  included), `mesher.rs` is the greedy cube mesher (visible faces only, faces
  across chunk borders ask the grid, each glowing material its own group), and
  `lib.rs` answers every op with the meshes that changed (`chunk/x/y/z`, and
  `.../glow<id>` for emissive ones, whose mesh `emission` is color times the
  palette's intensity) plus `remove_mesh` for ones that went away. Ops `set`,
  `fill`, `sphere`, `generate` (blocks and commands), reporters `get`/`height`,
  and rays in world units (`ray.rs`, a cell-grid walk: `cast` reports the cell,
  the empty cell before it, the face normal and distance; `break` and `place`
  edit along one); edits last for the run. The package schema declares the
  resource, commands and nine blocks (a material is a dropdown of the built-in names; ids work
  too). Measured: the default 64x32x64 island is drawn from the wasm module in
  under a second, inside its 10 s call budget. `surface=smooth` selects the
  CPU density mesher (`smooth.rs`): fixed tetrahedra, shared vertices and
  central-gradient normals across chunk halos. Signed density overrides in
  `grid.rs` are quantized to 1/256 cell; sphere brushes union or carve them,
  saved sphere lines replay them, and rays intersect the same triangles as
  collision (including actual shaped faces). Cube mode remains the default.
  Runtime edits can persist through `save_voxels` or `persistent: true`, using
  player storage and versioned compressed checkpoints. `streamed: true` uses
  bounded section residency around Voxel viewer components (`viewer`, formerly
  `invoker`) or `stream_voxels`, with
  horizontal column radius, vertical radius and a dense-cell byte budget; canonical
  queries and sparse edits survive eviction. `gpu_meshing: true` adds GPU vertex
  generation with CPU collision and fallback meshes. `fracture_voxels` detaches
  unsupported six-connected regions into editable convex bodies, retaining cells
  when fragment budgets are exhausted. See the voxel milestone in
  `docs/plugin-adr-0001.md` for settings, limits and validation. Render jobs stay
  in bounded 16-cell tiles; checkpoint v2 migrates v1 pages by cell coordinate.
  `lod.rs` caches derived samples at levels 1 through 4, bounded to 8192 records,
  with shape occupancy proxies, averaged smooth densities and edit invalidation
  along the ancestor chain. `voxel_lod_sample` inspects them in level lattice
  coordinates. `lod_mesh.rs` builds cube/smooth coarse CPU previews in 8-sample
  tiles, with two-sample halos, edit dependencies, a 65536-base-visit poll budget
  and four cached jobs. `voxel_lod_mesh` polls them until ready; these mesh data
  have no collision and do not replace full-detail renderer meshes yet.
  `voxel_lod_select` previews bounded perspective tile selection with frustum
  and distance checks and split/merge hysteresis. Camera inputs are explicit;
  selection does not publish meshes or change gameplay residency. Optional
  `visual_lod` uses presentation camera snapshots to stage a complete visual cut
  before replacing the previous one. Fine meshes are hidden through
  `mesh_visibility`, retaining their colliders; distant visuals have no collider.
  Pending cuts have a 2 MiB geometry budget and advance one tile job per frame.
  Edits invalidate intersecting halos, retaining unrelated tiles and jobs;
  generation changes restart jobs. `lod_seam.rs` clips visual tiles and emits
  planar solid-difference joins for mixed cube/shape/smooth resolutions, keeping
  boundary caps within the cut budget. Seam clipping advances one tile per frame.
  Renderer GPU mesh copies wait for a visible entity or instance, retaining hidden
  fine buffers and colliders. Ready copies use FIFO order under 64-mesh/10-MiB
  per-frame limits; CPU fallback remains until a whole-mesh copy completes.
  GPU cubes and rectangular cube LOD seams draw persistent eight-byte records
  through `plugin_quads.rs` (PBR/depth/shadow/motion/deferred), retaining only four
  vertex IDs and six indices per quad. Byte coordinates cover 128-cell LOD tiles;
  fractional/nonrectangular geometry keeps the CPU path. Smooth GPU meshes retain
  prefix sums and packed triangle spans; interpolation stays on the GPU.
  Visual cuts use 150 ms complementary dither crossfades, retaining one old cut.
  `plugin_lod.rs` traverses at most 2048 GPU-backed instances against the depth
  pyramid, requesting up to 512 deduplicated triangle uploads with two asynchronous
  readbacks. Stale/unsupported/overflow feedback falls back to CPU visibility.
  This does not select voxel tiles or request missing mesh jobs. Interpolated density
  seam collars and GPU voxel job requests remain unfinished. Run GPU tests serially.
  See `docs/voxel-lod-research.md` for the LOD implementation stages.
  Its manifest asks for `editor.preview`, `project-storage`, `gpu-compute` and
  `trusted-editor`; preview does not load or write player saves. Its trusted
  editor module `editor/BlockTypes.qml` edits the `block_set`'s `.blockdef`
  files (name, id, color, model, layer, tags, properties, variants, the states
  they make) through `read-text-asset`/`write-text-asset`, which only touch
  files under `assets/` (not scenes) and reload the world. A preview whose
  modules read a file through `assets.read` starts again once that file
  changes (`services::AssetReads`). QML test: `tst_VoxelBlockTypes.qml`. With
  `lighting` on, `light.rs` flood-fills block light (a definition's `light`,
  else 15 for a glowing palette entry) and sky light 0 to 15 over each chunk
  plus a 16-cell margin, so a face's shade is exact; a face takes the light of
  the cell it faces, `daylight` dims only the sky, and an edit remeshes the
  neighbouring chunks and every chunk below them (a block tick that changes
  no light, such as growth or water levels, remeshes only its own chunk).
  `set_daylight` (op `daylight`) relights at run time one chunk a stream
  frame, and only when the 0-15 step changes. LAN sync (M5): `sync.sections
  {generation, since}` answers the world's identity and every section edited
  since (`Grid::take_edited`, `section_record`: runs with `KEEP` for a
  streamed world's generated cells, shapes, densities); a regenerate, load or
  restore starts a new generation. A guest's own copy, started from the same
  settings, takes them with `sync.apply` (`apply_section`), regenerating
  first for a new generation, and stops its own block ticks. The LAN guest
  view (`lan/guest.rs`, `GuestPlugins`) hosts a local build's plugins (`--game`,
  or the build's own player; `BlockloomModule::link` links engine-tier code
  without a world) and draws what `sync.apply` answers.
  `tests/sync.rs` holds a copy to the host cell for cell. Cutout and translucent
  blocks let light and hidden faces through and mesh as their own groups with
  `MeshData.blend` (translucent vertex alpha is the definition's `opacity`,
  0.6 by default); GPU meshing stays opaque only. `texture_arrays` draws
  textured cube faces from one array per block set (`Registry::texture_layers`,
  sorted paths), uvs tiling across merged faces. `ticks.rs` steps block ticks
  on the `tick` hook (fixed simulation, runs only, never preview): scheduled
  ticks for a definition's `fluid` (a `level` property 0 source to `spread`,
  falling before spreading, two sources over ground making a third) and
  `falls`, and `random_tick_speed` random picks per 16-cell cube for its
  `random` rules (`becomes`, `grow` a property, `spreads_to`, with `when`
  covered/open and `unless_near`), all hashed from seed, step and cell. Only
  edits schedule (`World::touch`), so generated water rests. A fluid has no
  collider, rays pass it (`Grid::set_passable`) and placing replaces it; see-
  through states of one type hide shared faces (`Registry::families`).
  `entities.rs` keeps block entities (`entity: {kind, slots, stack}`): made on
  first use, dropped when their block changes with an `entity_broken` event
  (`when a {kind} block is broken`); slots, data and pending ticks ride the
  checkpoint. `graphgen.rs` is the `graph` preset: a `worldgen_file`
  (`.blockgen`, after Hytale's Worldgen V2, `docs/voxel-worldgen.md`) of
  biomes picked by `biome_map` ranges and blended over `transition`, each a
  terrain density tree (positive is solid; `exports` copied in by `import`),
  a material provider (`solidity`, `queue`, `depth`, `range`, ...) and props
  on the surface; y-independent nodes are cached per column, columns per
  graph, and `Grid::set_graph` samples it for streamed pages.
  `plugins/voxel/samples/mini-survival` is the reference sample (a graph
  world, a survival block set, `setup.py` for a first-person project);
  `tests/sample.rs` plays it and holds its release budget. Saved edits are the `world` resource's
  `edits` lines (`set X Y Z material`, `fill X1 Y1 Z1 X2 Y2 Z2 material`,
  `sphere X Y Z radius material`), applied in order over the generated terrain
  at `world.start` (a bad line is reported by number and skipped), so the scene
  view and every run show them; `add_voxel_edit` appends one and
  `clear_voxel_edits` forgets them, both through the `set_resource_field`
  command action (sets one resource field, or with `append` pushes `value` onto
  its list; one undo step; a `template` builds the value from the command's
  arguments, `sphere {x} {y} {z} {radius} {material}`, so no `value` is
  needed). `paint_voxel`, `paint_voxel_box`, `paint_voxel_sphere` and
  `paint_voxel_shape` are those templates, and the package's four scene tools
  (paint, erase, ball, shape) click them into the world (see Plugin scene tools).
  A `world_file` names a `.blockworld` asset (`blockworld.rs`: a JSON header with
  preset, seed and size, then 32-cell sections as `(count, material)` runs where
  256 keeps the generated cell, plus shapes and densities), read through
  `assets.read` and laid over the terrain before the `edits` lines; a file made
  over another preset or seed is refused. `blockloom-voxel-convert <project>`
  (`tools/convert.rs`, over `convert_edits`) moves a 0.3.x world's edit lines
  into one; infinite worlds keep their lines.
  A solid cell may be a slab, top slab, post, stair or ramp (`Shape` in
  `shape.rs`, a sparse table in `grid.rs`; a stair or ramp has a facing, the
  side it rises towards): the greedy mesher takes only whole cubes, shaped
  cells are a few boxes (and a wedge) meshed on their own from `Shape::faces`,
  with faces on a cell's edge hidden by a whole-cube neighbour, `shape` is the op, `shape_voxel` the command and block
  (ten blocks now), `shape X Y Z name` a saved edit line (`stair west`, `ramp north`). Writing a cube or air
  takes a shape away; cube-world rays treat a shaped cell as the whole cell,
  while smooth-world rays intersect its actual faces.
  The `infinite` preset (0.3.x) streams edgeless terrain: editable `noise`
  layers shape the land, `biomes` lines carve temperature/humidity rectangles
  with their own surface and hills, and `features` lines spawn ponds, trees
  and rocks, all canonical per cell so paging never changes the world.
  `render_distance` (default 256 blocks, steps of 32) drives the streamed
  radius, `distant_chunks` (default 128) drives the LOD reach, `pregen_distant`
  warms far tiles in budgeted slices. `pages_per_tick` limits nearest-first
  page work per call; portable infinite cube worlds process at most one
  8-cell mesh tile, skipping empty tiles.
  `chunk_workers` does not launch concurrent workers yet. Terrain features are
  computed per column with a bounded 4096-column derived cache. Checkpoint
  loads reattach the generator configuration before sampling unloaded cells.
  The `blocks` list is the block registry: one JSON object per line, after the
  `.blockdef` files a `block_set` asset (`.blockset`, `{"blocks": [...]}`) lists,
  with id, color, textures, model (cube, shape or inline mesh with optional uvs),
  tags and layer. A definition with `properties` is a block type: each state takes
  an id counted from `id` (last property fastest), `variants` (`{when, ...}`) lay
  looks and models over matching states, and `name[prop=value]` names a state
  wherever a material is named (the bare name is the default). A state whose model
  is a shape is shaped wherever the grid answers (`Grid::set_block_shapes`); the
  `blocks` op lists blocks and types. The cube mesher groups faces
  by material and per-face texture (`top`, `bottom`, `side`, `all`) with planar
  uvs, `MeshData` carries uvs plus a texture path the
  runtime loads once and shares, and registry names resolve everywhere a
  material does. Smooth terrain, coarse LOD, GPU meshing and fracture debris
  stay vertex-colored.
- **Importers and build hooks** (`blockloom-plugin-api/src/assets.rs`,
  `blockloom-plugin-host/src/imports.rs`): a package's schema may list
  `importers` (name, extensions, `limit_ms`) and `build` hooks (name,
  `limit_ms`). Both are module ops over raw bytes, `importer.<name>` and
  `build.<name>`: a request is one JSON line (`Request`) then the source
  file's bytes, an answer one JSON line then each produced file's bytes in
  listed order (`Produced`, with `warnings`, `errors` and `dependencies`).
  The host does every read and write, so a module needs no capability: output
  paths are relative plain names (`check_output_path`), sizes are checked
  against the bytes, at most `MAX_FILES`/`MAX_OUTPUT_BYTES`, and a portable
  module gets the schema's `limit_ms` of fuel for the one call
  (`CodeModule::call_bytes`). An import of `assets/x.gpl` writes
  `assets/x.gpl.imported/...` (never imported again) and records source,
  output and dependency hashes in `.blockloom/imports.json`, so
  `imports::status` says `fresh`, `source_changed`, `dependency_changed`,
  `output_missing`, `output_edited` or `source_missing`; a re-import replaces
  exactly what the last one wrote, and deleting the source in the tray forgets
  it. `import_assets` runs the first importer that takes a file's extension
  (a failure is logged, the copy stays); `plugin-importers`, `plugin-imports`
  and `plugin-import path=... importer=...` are the shell/MCP side. Before a
  desktop or web build `plugins::run_build_hooks` runs every hook with the
  target and the asset list, stages the files under `.blockloom/cooked/`, and
  `BuildOptions::extras` (`ExtraFile`) copies them to
  `game/plugins/<id>/cooked/` (refusing an escape or an overwrite); an error
  from a hook stops the build, and Android builds ship them in the APK's assets
  like desktop ones. `plugin-reimport` imports again what a source, dependency or
  output change left stale (an output edited by hand only when named); Play and
  Build call it first (`plugins::refresh_imports`, owner only), and the asset
  tray marks imported files and offers it. A package
  that is only importers and hooks is an editor tool and is not shipped.
  `plugins/examples/palette` imports GIMP `.gpl` palettes as a one-row
  `palette.png` (stored-deflate PNG, no compressor) plus `palette.json`, and
  its `cook` hook fails a build that has an unimported palette.
- **HTTP registries** (`blockloom-plugin-host/src/registry.rs`): a `plugins.json`
  registry value that starts with `https://` (or `http://` for localhost,
  127.0.0.1 and ::1) is an `HttpRegistry`, which reads a published registry
  folder (`index.json`, `archives/`) from any static host through `ureq`
  (not on wasm32). The index is read once per value, size-capped, and every
  archive is checked against the hash in it before it is unpacked, so only the
  index has to arrive over a trusted channel; an index naming a path that is
  not plain-relative is refused. `open_registry(base, value)` is the one place
  a value becomes a registry. Publishing stays a folder (`plugin-publish`):
  upload the folder. After a package change the owner bumps the folder's
  revision (`plugins::reload(s, true)`), so an attached copy's live reload
  also reloads its active plugins.
- **Trusted editor modules** (`PluginEditorsDialog.qml`, `blockloom-plugin-host/src/trust.rs`):
  a package may list `editor.modules` (`.qml` files, which need the
  `trusted-editor` capability and are left out of every build). Their QML runs
  inside the editor with the editor's own access, so a module loads only after
  the user trusts that exact package. `TrustLedger` (`<data dir>/blockloom/trusted-plugins.json`,
  per user, never in the project) maps a plugin id to the package's
  `content_hash`; an update changes the hash and asks again. `plugin_trust` and
  `plugin_untrust` exist in dispatch for the window only: they have no shell or
  MCP command and the attach socket refuses them (`GUI_ONLY`). The snapshot's
  `plugins.editorModules` lists each such plugin with `trusted`, `changed` and
  its modules, and gives a module's `file` only while trusted. The top bar's
  app-window button opens the dialog, which asks for trust, lists the modules
  and loads one into a `Loader` with `host` as an initial property: `host.plugin`,
  `host.project`, `host.resource(name)`, `host.setResource(name, payload)`,
  `host.call(command, args)` and `host.app` (the editor's `invoke` and state, i.e.
  host-level). `plugins/examples/com.example.notes` is the example. QML test:
  `tests/qml/tst_PluginEditors.qml`. A package may also list
  `editor.inspectors` (`{component, module}`, one `.qml` per component, same
  capability and trust): `InspectorPanel.qml`'s `pluginSection` loads it in place
  of the schema form once the package is trusted (the form stays otherwise),
  with `host.write(next)`, `host.call(...)`, `host.payload`, `host.type`,
  `host.app` and `host.project`. `com.example.notes` has a `sticky` component
  with one. Native compiled editor modules are deliberately not offered: they
  would run host-level code inside the Qt process with no ABI to bound it, so a
  plugin that needs custom UI uses trusted QML.
- **Plugin code on Android**: Android loads only portable (wasmi) modules, like
  the browser (`loads_native_libraries` is false for both). The runtime `.so` is
  built with `--features plugins`, the plugin host skips `ureq` there, builds
  ship plugins in the APK's assets (the same `game/plugins/<id>/` layout) and
  `android.rs` points the host's file reader at the APK. Compile-checked for
  `aarch64-linux-android`; never run on a device.
- **Host services** (`blockloom-plugin-host/src/services.rs`): a module's
  `host.call` reaches one `HostServices` hub, named `area.verb`: `host.version`,
  `rng.*` (hashes of `{seed, index}`, so a replay matches), `storage.*`,
  `save.*` and `assets.read` (need `project-storage`; the last reads a file under
  the project's or game's `assets/` as base64 `data`), `diag.*`, `jobs.*`, and whatever a
  `ServiceProvider` chained on adds (the world's `physics.*` and `nav.*`,
  `blockloom-runtime/src/plugin_services.rs`, answered from the published
  sensor snapshot and the nav bake). The editor and the world each build one
  hub; the capability gate stays in `native.rs`.
- **Storage** (`storage.rs`): two stores namespaced by plugin id, project
  blobs (`.blockloom/plugin-data`, read-only in a built game) and player saves
  (per project, written while a game plays). `BlobStore` is a flat key space
  with atomic multi-key `commit`; `DiskStore` writes through a temp file and a
  rename, `MemoryStore` serves tests. Content blobs live under
  `blobs/<sha256>` (`put`/`get`) and every write is held to `StoreLimits`
  (16 MiB a blob, 256 MiB and 4096 keys a plugin). `KvStore<B: KvBackend>` keeps
  a plugin's saves in any key-value backend (base64 values under a per-project
  prefix, a failed commit puts back what it changed); the web player's backend
  is localStorage (`web::LocalKv`). `plugin-data-gc [dryRun=true]`
  (`storage::collect_blobs`, `commands::plugins::plugin_data_gc`) removes content
  blobs nothing names: a `blob:<sha256>` string in a record, another stored key
  or a reachable blob, over the project's data and its saves; refused while a
  game runs.
- **Jobs and generation** (`jobs.rs`, `generation.rs`): `jobs.start` runs
  `job.<name>` in slices under a per-frame budget (`WorldPlugins::run_jobs`),
  highest priority first. A slice answers `{progress, state}`, `{done,
  result}` or `{error}`; `state` comes back next slice, cancel calls the op
  once with `cancel: true`, and a job's `event` is raised when it ends. A
  package's `nodes` are typed graph nodes (`blockloom-plugin-api/src/
  generation.rs`, `GraphDef::plan` checks types, one source per input and
  cycles); the `graph.evaluate` job evaluates one node per slice through a
  tile cache (`GraphCache`) with per-node margins, so an edit recomputes only
  what it feeds.
- **Diagnostics** (`diagnostics.rs`): `diag.count|gauge|time|marker` plus the
  host's own call counts, timings, errors and faults per plugin.
  `plugins::report` sends them from the world about every fifth status report
  (`RuntimeMessage::PluginDiagnostics`, and `plugin/*` render metrics for the
  profiler); `StateDto.plugin_diagnostics`, `plugin-diagnostics` in the shell
  and the Plugin Manager's rows read them.
- **Editor surfaces** (`blockloom-plugin-api/src/surfaces.rs`): a package's
  `menus`, `shortcuts` and `overlays`. A menu item or shortcut runs one of
  the package's commands with fixed arguments (so it is that command's
  validated, undoable action): the top bar's plugin menus and `Main.qml`'s
  `Instantiator` of `Shortcut`s call `plugin_call`. An overlay is a scene-view
  toggle that asks the hosted preview module for `overlay.<name>` and draws
  the answered `Shape`s as gizmos (`SceneView::plugin_overlays`,
  `PROTOCOL_VERSION` 29); overlays only look. `surfaces_json` in
  `commands/plugins.rs` builds the snapshot. `conflicts` in a manifest, two
  packages providing one service and hook orders that cannot be met are
  `ActivePlugins::conflicts()`; a blocking one stops Play and Build and is
  listed by `plugin-check` and the manager.
- **Rendering service** (`blockloom-plugin-api/src/rendering.rs`): an
  `instances` effect draws many copies of one of the plugin's meshes (position,
  yaw, scale each) as entities sharing the mesh's buffers and material, which
  Bevy batches (`plugin_meshes.rs`, `PluginInstance`); a mesh may be a
  `convex_hull` or `aabb` collider as well as a trimesh. A package's `shaders`
  are `.wesl` files registered as `blockloom::plugin_<id>_<name>`
  (`rendering::module_name`): core's `shader_lib` keeps a global extra-module
  registry (`set_extra`, so the editor's surface check can link them and a
  plugin cannot replace a built-in module) and the runtime's
  `passes::sync_plugin_shaders` adds each as a Shader asset when the loadout's
  shaders change.
- **GPU compute** (`blockloom-plugin-api/src/compute.rs`, the `blockloom-plugin-gpu`
  crate, `blockloom-runtime/src/plugin_compute.rs`; ADR 0001's twenty-third
  batch): a plugin never holds the device. A package's `kernels` (`.wgsl`, an
  entry, a workgroup size, named group-0 bindings `read|read_write|uniform`)
  need the `gpu-compute` capability, and `check_kernel` (naga, no optional
  capabilities) refuses a kernel whose bindings differ from its schema or whose
  loops aren't `for`s over a constant-bounded counter (total iterations within
  `MAX_STATIC_COST`), at seal, install and load. A module answers `gpu_buffer`,
  `gpu_write`, `gpu_dispatch`, `gpu_read` and `gpu_free` effects
  (`Effect::gpu_command` -> `GpuCommand`; no capability is an error), a read
  comes back as the module's `gpu.result` op. `ComputeEngine` (feature `engine`,
  wgpu) owns the buffers and queues per plugin; each frame it runs commands in
  order in one encoder under `FRAME_INVOCATIONS`, round-robin, writes through a
  staging copy so they stay ordered, and reads back through mapped staging
  buffers. The runtime's `ComputeLink` carries commands to the render world
  (`run` in `RenderSystems::Cleanup`, on Bevy's device) and reports back;
  `Loadout::kernels` carries the kernels, `PluginHost::kernels_serial` says when
  they changed, and a run's end clears buffers. The SDK's `gpu` module builds the
  effects. Tests: `cargo test -p blockloom-plugin-gpu --features engine --
  --include-ignored` with `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`.
  Not offered: textures, push constants, indirect dispatch, WESL imports.
- **Reload and isolation**: a changed loadout reaches the running world as
  `WorldPlugins::reload`: a portable module answers `world.save`, is replaced
  and gets `world.restore`; a native change needs the run restarted
  (`ReloadReport::restart_needed`). With `BLOCKLOOM_PLUGIN_ISOLATION=process`
  a native library is loaded by the `blockloom-plugin-worker` binary beside
  the editor (`BLOCKLOOM_PLUGIN_WORKER` names another; `all` hosts portable
  modules there too): `isolated.rs` speaks length-prefixed JSON frames over its
  stdin/stdout, host services stay in the host and cross the pipe, and a call
  past its wall-clock limit or a worker that dies stops the module, not the
  game. The worker ships beside the editor (the Linux install, the CI editor
  archives and the Windows installer carry it); a built game does not.
- **Authoring kit**: `plugin-new path=... id=... template=declarative|portable|native`
  (`blockloom-app/src/scaffold.rs`) writes a starter package: a declarative
  one is sealed at once, a portable one is a crate (`src/lib.rs` is
  `blockloom-plugin-sdk/templates/lib.rs`, which the SDK's `tests/scaffold.rs`
  compiles and runs), schemas, `build.sh` (wasm build plus `plugin-seal`) and
  a CI workflow. A native one is the same crate plus a native library: its
  `build.sh` also builds this machine's `cdylib` and runs `plugin-add-native
  path=... target=... library=...` (`scaffold::add_native`: records the
  library under its target triple, makes the tier `native`, adds the
  `native-execution` capability and seals again), so each platform's library
  is one more run of it. The SDK's `testing` feature adds `Harness` and `harness!(T)`
  (`harness!(T, [Capability::ProjectStorage])`): the plugin runs in-process
  through the host's loader with memory stores. `docs/plugin-api.md` is the
  author guide. CI runs `cargo test -p blockloom-plugin-sdk --features testing`
  and the host with `test-fixtures` (the isolation test).
- **Engine plugins** (tier `adapter`, Tier E in the review): a package whose
  `runtime.adapter` names a crate (`crate_dir`) implementing
  `BlockloomModule` against `blockloom-engine` (the facade: Bevy, core and
  `blockloom_runtime::api`), named with `export_module!`. The crate is
  standalone and asks for `blockloom-engine = "<version>"` plus `bevy` at the
  engine's exact version without default features (Bevy's derives look for
  `bevy` in the crate's own manifest). `build-project-runtime`
  (`blockloom-app/src/project_runtime.rs`) writes `.blockloom/runtime/`, a
  crate whose `main` adds each module (`api::add_engine_module`) and calls
  `run_process`, carrying the engine's toolchain, lock, profiles and patches
  (which point `blockloom-engine` at the engine tree), and builds it with
  Cargo into the engine's own target directory, so only the plugin and the
  link are new; the binary lands in `.blockloom/runtime/bin/`. While the
  project has one, the editor starts that binary as a child instead of the
  embedded world (on Linux drawing into the Game view, see Game view), and refuses Play until it is built and whenever a package
  changed since (`bin/plugins.json` stamps each plugin's content hash). A
  package not installed from a `path:` folder must be in the `TrustLedger`
  at its hash before it builds (`project_runtime::needs_trust`, granted by
  the Plugin Manager's Engine plugins section, which also builds; the
  snapshot's `plugins.engine`). A desktop build for this machine ships the
  built project runtime as its player (`BuildRequest::engine_player`; an
  adapter package ships only its manifest and schemas, its crate being linked
  in). A web or Android build compiles the same generated crate's `lib.rs`
  for that target (`project_runtime::build_for`, `--no-default-features
  --features portable`, which is the runtime's `plugins` plus `embedder`; the
  engine has no default features so plugin crates don't unify desktop in):
  wasm-bindgen output whose `register_engine_modules` the page calls before
  `start_game`, or a `.so` whose `android_main` adds the modules first. It
  lands in `.blockloom/runtime/bin/<triple>/`, stamped like the desktop one,
  and needs the engine source tree (an install's kit links desktop only). The
  engine is a source tree (`BLOCKLOOM_ENGINE_SOURCE`, else the one the editor
  was built from) or, in an install, the engine kit beside the editor
  (`engine/`, `BLOCKLOOM_ENGINE_KIT`), written by `engine-kit out=...`
  (`package-release.py` runs it; `engine_kit.rs`): tracked sources and
  `cargo vendor`ed crates for an author's own `cargo test`, plus the
  engine's compiled crates from one Cargo build of a runtime with no
  plugins and the rustc flags Cargo used for them (`kit.json`). Cargo's
  target can't move (its fingerprints hold absolute paths), so an install
  never runs Cargo: each plugin crate is one rustc run against the kit's
  rlibs (its `Cargo.toml` read for name, edition and dependencies, which
  must be crates the kit has, no build script) and the runtime one more,
  in `<data dir>/blockloom/engine-build/`, with the install's `tools/rust`
  rustc at exactly the kit's version. The kit is only read, so it may sit
  anywhere. `plugin-new template=engine` writes a sealed starter from
  `blockloom-engine/templates/lib.rs` (compiled by that crate's
  `tests/template.rs`); `docs/plugin-api.md` has the author's side.
  `plugins/examples/tally-engine` counts a run's ticks and says the total.
  An engine module reads its data the ECS way: every authored actor entity
  carries `PluginComponents` (its plugin records) beside `ActorId`, and
  `SceneDocument::plugin_resource` gives the project's resources. A module
  that hosts an SDK `Plugin` calls `link_plugin(id, entry)` with its
  `export_plugin!` entry: an adapter package's loadout entry is then
  `CodeRuntime::Linked`, which `blockloom_plugin_host::linked` starts through
  the same C ABI in-process, so its blocks, hooks, scene tools and preview
  work as a library's would (the editor has no such entry, so its `module`
  commands answer that they run only in the project runtime). Only one
  linked crate may use `export_plugin!` per runtime, since the symbol is
  fixed. `engine` re-exports the SDK and API as `sdk` and `plugin_api`.
- **Not yet**: a faster browser host (the page's own WebAssembly instead of wasmi
  in wasm; it needs a fuel substitute, since a browser instance has no budget to
  stop a runaway call), a headless browser proof and native editor modules.
  `plugins/examples/com.example.health` is the sealed proof package;
  `plugins/examples/tally` is the SDK one, with code, and
  `plugins/examples/palette` the importer/build-hook one.
