# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Blockloom is a Qt/QML desktop app (Windows/Linux/macOS) for building games out
of blocks - a Scratch-style block editor driving a real game world, in 2D or
in 3D. A project is a world plus a set of actors, each with its own block
canvas; pressing Play runs those blocks against a Bevy world, shown in the
editor's Game view.

## Common commands

```bash
just build              # cargo build --release --workspace (the normal build)
just run                # build, then launch target/release/blockloom
cargo build --workspace && target/debug/blockloom   # debug build/run - faster iteration
just test               # cargo test --workspace (blockloom-core has the bulk of them)
cargo bench -p blockloom-core --bench vm   # block VM ns/tick over a few canvases
just player             # stage the hard-optimized player a built game ships
just web-check          # runtime check-build for wasm32-unknown-unknown (Phase 8)
just web-build [out] [pack=game-dir]  # wasm player folder; serve with just web-serve
```

Build the whole workspace, not just `-p blockloom`: off Linux (or with
`BLOCKLOOM_RUNTIME=process`) the editor starts the `blockloom-runtime` binary
sitting next to it, and a stale or missing runtime is exactly what `cargo run -p
blockloom` would leave you with. The editor says so in a banner rather than
letting Play do nothing.

`rust-toolchain.toml` pins the Rust version (rustup installs it on first
build). There's no clippy.toml/rustfmt.toml - just `cargo clippy`/`cargo fmt` with
defaults.

## Style

- NEVER use em-dashes (-) in comments, code, docs, or commit messages. Use a normal hyphen (-) instead.
- Write comments like a human: short and terse, 1-2 lines. State what and why briefly. No multi-paragraph essays, no history lessons ("Previously..."), no dangling "see X docs" pointers unless X exists.

### Frontend (`blockloom-qt/qml/`, QML)

The QML is compiled into the binary by `blockloom-qt/build.rs` (cxx-qt's
`CxxQtBuilder` + qmlcachegen), so a plain `cargo build` picks up every edit.
It needs Qt 6.10 or newer with Quick, QuickControls2, QuickDialogs2 and Multimedia; use
Qt 6's `qml`/`qmlls` (`/usr/lib/qt6/bin` on Arch), not Qt 5's. A new `.qml`
file must also be listed in `build.rs`'s `QmlModule`.

### Local `blockstitch` development

`blockstitch` is a separate sibling repo shared with any other app built on
the same block editor. Two halves of it are used here:

- `crates/blockstitch-core`, the Rust backend (value system, document model,
  editor operations);
- the QML block canvas and controls (`blockstitch-qml` at the repo root,
  module `com.blockworked.Blockstitch`), which lives on blockstitch's `qml`
  branch.

Both are git dependencies in the root `Cargo.toml`, pinned to the same
commit. To build against a local checkout instead:

```bash
just blockstitch-local [path]        # default path: ../../blockstitch, relative to ui/
just blockstitch-published [commit]  # restore the pinned commit, or move to a new one
```

The old Vue frontend (`ui/`) and its Tauri shell (`src-tauri/`) are still in
the tree but out of the workspace, as are the browser dev bridge recipes
(`just dev-backend`/`dev-ui`) that serve it. Nothing new goes there.

### AI/CLI shell (`blockloom-shell`)

A standalone text interface that speaks every `Backend::dispatch` command as
a single text line, returning JSON responses. Useful for AI agents, scripts
and manual testing outside the GUI.

```bash
just shell                              # builds workspace, launches REPL
just shell --eval 'create-project name=Demo mode=TwoD'   # one-shot
echo 'add-actor name=Ball shape=Circle' | just shell      # piped script
```

**Syntax:** `command key=value key2="value with spaces"` - bare words and
numbers keep their type; quoted strings and JSON values (`[1,2]`, `{"a":1}`)
are preserved as-is. `# comments`, blank lines, and `help` / `exit` are
understood. Every response is `{"ok":bool,"result":...,"error":...,"state":...}`.

**Architecture:** a separate binary (`blockloom-app/src/bin/shell.rs`) that
hosts its own `Backend` (like `devserver`). The command registry lives in
`blockloom-app/src/shell.rs` (`COMMANDS`), one `CommandSpec` per dispatch
command. `parse` turns a line into `Action`, `run` turns `Action` into a
JSON response. `--eval` for one-shot, `--no-state` to suppress the state
snapshot (useful for bandwidth with large projects). Piped mode exits 1
if any command errors. Project folders carry an owner lock
(`blockloom-core/src/sync.rs`) and a revision counter bumped on every save:
`open-project` attaches to a live owner's copy (or takes it over with
`force`), idle backends reload the folder's saves before each command,
`sync-status` / `reload-project` / `take-over-lock` manage the rest, and
`--watch` streams revisions. `--attach` drives the editor's own backend over
the attach socket (`blockloom-app/src/attach.rs`, served by the editor) so
there is one copy at all.

#### MCP server (`mcp/`)

AI agents can also reach this through the Model Context Protocol: `mcp/` is a
pnpm/Node package (`@blockworked/blockloom-mcp`, entry `src/index.ts`) that
spawns `blockloom-shell --no-state` and exposes every `--specs` command as an
MCP tool, with schemas derived from the same registry (see `src/registry.ts`,
which maps every `ArgSpec.ty` prose string to a zod schema - a new prose type
must be taught there). Tool calls are one shell line, responses are the
`{ok, result, error}` shape, plus `blockloom://state` and `blockloom://blocks`
resources. Each session is its own backend unless started with `--attach` (see
above), so without it the window-and-shell sharing rules apply: the shell
attaches to the live owner's files and follows their saves, and one MCP server
process and the editor should still not write the same project at once. The shell resolves as `target/debug|release/blockloom-shell`
beside the repo, `BLOCKLOOM_MCP_SHELL`, or `--shell`; `just mcp` builds
everything and prints the client config line for the built `dist/index.js`.
Build/test with `cd mcp && pnpm install && pnpm run build && pnpm test`.

## Architecture

### Cargo workspace

The editor, and the game world. On Linux the world runs on a thread inside the
editor and draws straight into its Game view (see Game view below); elsewhere
it is a child process.

- **`blockloom-qt`** (package `blockloom`) - the editor window and the app's
  entry point: Qt Quick over cxx-qt. It owns `blockloom-app` directly - there
  is no daemon. `src/app_bridge.rs` is the one QObject QML talks to:
  `invokeCommand` runs a `Backend::dispatch` command on a worker thread, in
  order, and answers with `replied(token, {ok, result, error})`; state the
  backend publishes lands in the `stateJson` property, coalesced per burst.
  `src/preview.rs` follows a child runtime's MJPEG preview stream into
  `previewFrame`. `src/game_view.rs`/`.cpp` are the in-process Game view, and
  `src/pointer_lock.cpp` its pointer lock.
- **`blockloom-app`** - the backend: `src/commands.rs` holds every command,
  `src/dispatch.rs` maps command names + JSON args onto them, `src/state.rs`
  holds `SharedState`/`AppState` and the snapshot the frontend gets, and
  `src/runtime.rs` is the game runtime's leash. No Qt dependency, so the
  dev bridge (`src/bin/devserver.rs`, feature `dev-bridge`) hosts the same code.
  Commands publish changes through `AppHandle`, a plain callback the host
  supplies.
- **`blockloom-runtime`** - the game world: a Bevy app that renders one project
  and runs its blocks. A library plus a thin binary: `run_process` is the child
  process and the built player, `embed::run` the windowless in-editor world,
  and both build the same world through `add_world`. `--mode 2d|3d`
  decides which physics/render pipeline is built, so the editor restarts it when
  a project switches dimension. `src/world.rs` holds the dimension-agnostic
  systems, `src/dim2.rs`/`src/dim3.rs` the sprite/`bevy_rapier2d` and
  mesh/`bevy_rapier3d` halves. The same binary is what a built game ships:
  with a pack beside it, it loads that instead of waiting for an editor, and
  `src/player.rs` is the whole of the difference (see Building a game below).
- **`blockloom-protocol`** - the messages between them: newline-delimited JSON
  over a child's stdin/stdout, or the same enums over channels in-process. No
  sockets, no ports; the pipe or channel closing is the whole shutdown
  handshake. Bump `PROTOCOL_VERSION` whenever a message changes.
- **`blockloom-core`** - the engine library both processes share: `scene.rs`
  (the value types a world is built from - looks, bodies, placements, camera),
  `components.rs` (what an actor is made of - see below), `blocks.rs` (the
  block vocabulary and its `BlockKind` impl), `fields.rs` (what the frontend
  calls each value slot), `project.rs` (the saved document and the folder it
  lives in), `library.rs` (the project folders the Dashboard lists),
  `assets.rs` (the files inside one of those folders, which the asset tray
  manages and the runtime loads images and fonts from), `pipeline/` (what
  import makes of each of those files - see Import roles below), `pack.rs` (that
  document again, as a built game carries it) with `build.rs` (what lays a
  build out), `vm/` (the block VM), `codegen/` (the same blocks as Rust
  instead),
  `script/` (compiling a project's Rust scripts, and the ABI they talk over),
  `sense.rs` (the world state reporter blocks read), `ui.rs` (the screen-space
  interface a game builds out of blocks - see Interface below), and `wire.rs`
  (the one shape difference between documents and the frontend).
- **`blockstitch-core`** (sibling repo, see above) - the shared block-editor
  backend. `value` is the `Value`/`Op` expression system, extended by an app
  through `register_operators` (Blockloom registers its sensing reporters in
  `blockloom-core/src/value.rs`); `graph` is the document model (`BlockGraph` of
  strands, instructions, comments, variables, custom blocks), generic over the
  app's own instruction enum via the `BlockKind` trait; `editor` is canvas
  addressing (`PathStep`/`ValueLocation`), undo `History`, and every structural
  edit a drag/drop/typed character performs. Blockloom implements `BlockKind`
  for `InstructionKind` (`blocks.rs`), names its value slots in `fields.rs`, and
  gives every actor a `BlockGraph` of its own.

### Components

An actor is a list of components, not a fixed set of fields
(`blockloom-core/src/components.rs`). `Place`, `Look`, `Render` and `Body`
each become the Bevy component they name when the runtime spawns the actor,
so removing `Body` really does leave it without a rigid body, and removing
`Look` leaves a positioned, scriptable actor with nothing to draw. Only
`Place` can't be removed: there is nowhere for an entity without a transform
to be. `actor.visual()` is therefore an `Option`, while `placement()`,
`physics()` and `visible()` fall back to a default.

Four components have no fixed-field ancestor. `Camera` attaches the world
camera to that actor - follow, first person or third person - and one project
has one of them, so adding it takes it off whoever had it. `Script` names a
Rust file (see below). `Parent` names another actor this one hangs off (see
Actors below). `Custom` is a named bag of values the project invented
(`Health { hp, armour }`); the runtime carries it on the entity as
`CustomComponents`, publishes it through `sense::ActorSense`, and
`set <field> of <component> to` writes it back. Like a position and unlike a
variable, those writes last exactly as long as the run.

Components come and go mid-run, from a block (`attach`/`detach`) or from a
script. `engine.attached` is the one record of what an actor is carrying right
now - the document says what it *started* with - and
`world::apply_component_effects` is the only place that writes it. A `Body` or
a `Look` needs the dimension's own pipeline, so `dim2`/`dim3` pick those two
out of the same effect list and do the ECS half. Re-attaching brings back what
the editor authored, or that component's defaults if the project never had one.

Pre-component documents kept `visual`/`placement`/`physics`/`visible` flat on
the actor, and the world camera named the actor it followed. Both still load:
`Actor` deserializes through `ActorRepr`, and `Project::normalize` moves the
old `follow` onto its actor as a camera component.

### Actors that come and go

An actor's id is what everything keys it by, and a run can mint ids the
document never had. `engine.spawned` holds those runtime-only actors and
`Engine::actor` looks there before the project, so one question finds any
actor at all - a clone answers about its shape, its physics and its components
exactly as the actor it was copied from does. Those ids are counted rather
than random - `~1`, `~2` - so one project run makes the same ids however it
was scheduled, which is what lets `tests/codegen.rs` hold a compiled run's
clones against the VM's. An id the host mints itself, for a script's clone in
a build whose scheduler is the compiled program, is `~h1` instead, so the two
counters can never land on the same name.

`create a clone of` copies a running actor. The VM does the scheduling half:
`register_clone` gives the copy the template's compiled `Program` (an `Rc`
clone - one program, many actors), its name, its custom-block inputs, and its
own copy of the template's variables as they stand. It then queues
`Event::Cloned`, which starts the copy's `when I start as a clone` strands at
the top of the next tick - by which time `apply_lifetimes` has built the
entity those blocks read through. The host does the world half: the clone is
the template as the editor authored it, standing where the template stands
now, carrying its live custom-component values and hanging off whatever it
hangs off. A clone shares its template's name, so `when I touch Ball`,
`broadcast` and `how many Ball there are` all reach every copy; `the actor I
made` reports an id, which is how a block means one clone in particular.
A compiled program does the same halves in the same order: its own `Actors`
table mints the id and queues the copy, and the host is handed the entity to
build (see Compiling the blocks below).

`create actor` makes something the document never had: a `Place`, a plain
`Look`, and no blocks at all. `delete` takes an actor out of the run - the
document is untouched, so Play puts an authored one back - and stops its
scripts. Deleting yourself ends the strand that asked where it stands, the
way `stop all` ends everything. Both blocks name an actor through a value
slot, so `the actor I made` can be dropped straight into one; `create a clone
of` names an authored actor, so it stays a dropdown.

The parent/child hierarchy is `engine.parents`, one entry per child, seeded
from every `Parent` component on a rebuild and moved after that by `set my
parent to` or a script. Rather than reparenting Bevy's transforms - which
would make every position in the engine relative to somebody and leave rapier
owning half of them - `world::apply_parenting` moves each child by exactly the
change its parent underwent this step: `child = (parent now / parent then) *
child`. A child that moved itself keeps that motion, a parent's turn swings
its children around it, and parents are walked roots-first so one pass carries
a move the whole way down a chain. It runs in `FixedPostUpdate` before
`record_poses`, so the parent's change includes what physics wrote and the
pose the renderer interpolates towards is where the child ended up. Loops are
refused where they would be made - `Project::prune_parents` on load,
`commands::check_parent` in the editor, `world::set_parent` at run time -
because a cycle has no root to start the pass at.

A `Parent` may also carry an `offset`, which is where the child stands in its
parent's frame. `Place` stays the one thing the world is built from, so the
offset is resolved into a world placement before anything is spawned -
`world::place_authored_children`, parents before their children so an offset
down a chain is measured against a parent that has already moved - and read
again whenever `set my parent to` hangs the actor off someone new, which
places it at the offset rather than leaving it where it stood. A `Parent`
without one - which is what a document written before offsets says - keeps
the world position its own `Place` gives it, at build and at run time both.
The local-position reporters (`my local x position`, `<actor>'s local x
position`, and the matching script reads) answer that frame live: the
parent's world transform inverted onto the child's world position, or the
world position itself for an actor hanging off nothing.

### Scripts

An actor's `Script` component names a real `.rs` file under the project's
`assets/scripts`. It is a normal crate root: it links against one generated
crate, `blockloom`, and names its entry points with `blockloom::export!`.

`blockloom-core/src/script/` owns the build. `abi.rs` is the C boundary and is
compiled *twice* - once into `blockloom-core` as the host's view, and once, as
text, into the `blockloom` crate the script links against - so editing it
changes both halves at once. `ABI_VERSION` must go up whenever it does, since
a script built against an older one is still sitting in somebody's project
folder; the runtime checks it before calling in. `prelude.rs` is the API over
that boundary and is script-side only. Building is two `rustc` runs and no
network - the `blockloom` rlib, then the script as a `cdylib` - cached in the
project's `.blockloom/build` against the source, the toolchain and the ABI.
There is no Cargo, so a script gets `std` and nothing else.

The editor compiles scripts on Play (`commands::build_scripts`) so rustc's
errors land in the run log against the script's own line numbers; the runtime
only ever loads what it finds. **Scripts therefore need `rustc` on the machine
that presses Play.** The boundary is three calls, not one per verb, so adding
something a script can do is a new constant in `abi.rs` rather than a new
field in `HostApi` - which would break every script already built.

A script reads the world through the same frame snapshot the reporter blocks
read (`sense`) and everything it does comes back as a `vm::Effect`, applied by
the same systems. So a script and a canvas can drive one actor between them,
and reading straight back after a write gives the old value, exactly as it
does in the block editor. A panic inside a script is caught by `export!` and
logged rather than being allowed to cross the C boundary, which would abort
the whole game window.

### Game view

`Backend::start_embedded` takes an `EmbeddedRuntime` host, and
`RuntimeHandle` then starts the world on a `blockloom-world` thread instead of
spawning a child - same messages, over channels. `bridge::attach` routes the
world's `send` into that channel. A panic in the world is caught and reported
as `Fatal`, ending the run rather than the editor; a native crash (a script
library, a GPU fault) still takes the editor down.

Bevy runs headless there: no winit, one update per frame the view presents
(`embed::paced` waits on `FrameExchange::presented`, which the view calls on
every swap and keeps asking for while a world runs; 50 ms at most, so a hidden
view still hears the editor), and synchronous pipeline compilation, since async compile tasks outliving the
device crash NVIDIA at exit. Cameras render at the view's real pixel size
(`FrameExchange::resize`, debounced by the view, since each size is a new
ring; a fixed resolution picked in the Game tab is sent as is at scale 1),
with `UiScale` and the 2D projection following the view's pixel density, into a ring of three Vulkan images
exported as dma-bufs (`embed.rs`, raw `ash` under wgpu). `FrameExchange` hands
slots between the world and the view: the world never draws into the one
being shown or waiting to be. The C++ `GameView` item imports the ring through
EGL. This is why Qt is forced onto its OpenGL renderer, and onto EGL on X11.

The ring's layout is negotiated. The view offers (`FrameExchange::accept`)
the tiled DRM format modifiers its EGL samples as a plain texture; the world
turns on `VK_EXT_image_drm_format_modifier` through Bevy's `raw_vulkan_init`,
allocates the ring in one of those, and has the cameras draw straight into it
(`aim_cameras` swaps the claimed slot in as their output attachment) - no copy
on either side. With no common modifier, or once the view `refuse`s one it
couldn't import, it falls back to a linear system-memory ring the frame is
copied into, which NVIDIA only samples as an external texture, so the view
copies it again through a small shader into a plain texture.

Status (positions, variables) and the run log reach QML through their own
`statusJson` and `logJson`, not the whole state snapshot - re-evaluating every
binding on each status or `say` stuttered the view. `RunLog.qml` appends by
`log.total` rather than rebuilding its list.

Input is `PreviewInput`, as it is for the MJPEG preview: keys, buttons,
position, text, scroll, touch, plus `focus` (which, once sent, decides whether
the game is focused, and releases held keys and fingers when lost) and
`mouse_delta`. Keys travel by physical position: the view turns Qt's
`nativeScanCode` into a `KeyCode` name through `blockloom_protocol::keys`,
falling back to Qt's key name where there is no scan code (macOS). Scroll and
touch become Bevy's own `MouseWheel`/`TouchInput` messages in
`preview::drain_preview_inputs`. A windowless
world answers `lock mouse` with `RuntimeMessage::PointerLock`; the view then
locks the pointer while it has the keyboard - Wayland pointer constraints and
relative pointer (the generated glue is vendored in `blockloom-qt/src/wayland/`),
cursor warping on X11 - and forwards raw motion. Escape always releases it and
a click takes it back.

HDR frames can't go through the 8-bit ring or Qt's 8-bit window. On Wayland
the view makes a subsurface *under* its window (`makePlane` in
`game_view.cpp`, input passing straight through, sized by `wp_viewporter`)
and offers it through `FrameExchange::offer_hdr_surface`; the world makes a
swapchain on it (`embed::HdrPlane`), which is what `DisplayOffers` then
reports. While the frame resolves HDR, the cameras' view takes the
swapchain's format and `aim_plane` points them at its texture instead of a
ring slot, and `hdr_live` tells the view to draw a zero-alpha texture Qt
thinks is opaque, which shows through to the plane while overlays still
draw on top. That is why the window has an alpha buffer on Wayland.
`BLOCKLOOM_HDR_VIEW=0` turns the plane off.

`BLOCKLOOM_RUNTIME=process` forces the child process and MJPEG preview on
Linux too. Windows and macOS have no GPU sharing yet.

### Scene view

While nothing runs, the Game view is a scene view over the same world:
loaded, never started, and seen through an editor camera instead of the
game's. A run ends the same way whether the Stop button or a `stop all`
ended it (`world::end_run`): the world is rebuilt from the document, so the
scene view never keeps where the run left things. `commands::open_world` brings a world up for it when the tab is shown
(embedded, or with the preview on), so editing never waits for Play.
`blockloom-runtime/src/edit.rs` is all of it: `SceneEditor` holds the camera
(a flying one in 3D, pan and zoom in 2D), the selection and any drag.

Input is the same forwarded `PreviewInput` a game reads, but `edit::interact`
takes it in order ahead of the game's systems, so a press and its release in
one frame still make a click. A click picks the actor under the pointer
(nearest along the ray in 3D, topmost in 2D) and reports `Picked`; a drag on
it or on the gizmo's handles moves the entity's pose live, and the release
reports `Placed`. `commands::place_from_view` writes that into the document
as one undo step, which reloads the world like any other edit. A child placed
in its parent's frame reports its new `offset` too, since that, not `Place`,
is what the world builds it from, and such children follow a dragged parent
live.

The tool, snapping and steps are `blockloom_protocol::SceneView`, an editor
preference kept in QML `Settings` and re-sent to every world that comes up,
as is the selection (`EditorMessage::Select`). `Place` has a uniform
`scale` (the size blocks read and write) and a per-axis `stretch` in the
actor's own frame; the scale tool's axis handles move the stretch and its
centre handle the size. Handles are gizmo lines in their own `HandleGizmos`
group, drawn over the world. In 3D the editor holds the view's pointer lock
while the right button is down, so looking around reads raw motion.

### Instancing and batching

`blockloom-runtime/src/batching.rs`, 3D only. A plain surface (no graph
shader, no box projection, not a tilemap) draws through `InstancedMaterial`:
the shared material is keyed by `surface_key`, the surface minus color and
UV transform, and those two ride per actor in one storage buffer indexed by
its `MeshTag` (`InstanceTable`, slot 0 the identity). So `set color` writes a
slot rather than making a material, and same-mesh actors stay one draw.

`batch_meshes` (PostUpdate) merges what instancing can't group: actors with
no body or a fixed one that hold still for `settle_frames` merge per
streaming cell and surface, on a `CellTasks` background task (members draw
themselves until it lands), and one that moves after merging draws itself for
the rest of the run; small movers merge per surface every frame. A batched
actor keeps its entity and only loses its own draw, through
`RenderLayers::none()`. Merged meshes bake tint and UVs into vertex colors and
UVs. `BatchPolicy` holds every threshold. Counts reach the profiler as
`batching/*` render metrics.

### LOD and occlusion

`blockloom-runtime/src/culling.rs`, 3D only. `LodGroup` is the one LOD
selector: levels of a screen-size threshold (bounding diameter over viewport
height) plus an optional mesh, picked with a hysteresis band in `select_lod`
and announced by `LodChanged`; below the last level the entity leaves the main
view. Content hands it levels (`RenderCache::lod` does spheres and capsules)
rather than selecting its own, and batching leaves LOD'd actors instanced.

`cull_views` runs between Bevy's frustum pass and
`MarkNewlyHiddenEntitiesInvisible`, removing LOD-culled and occluded meshes
from the world camera's `VisibleEntities` only, so shadows are untouched.
Occlusion is a software Hi-Z: the camera-facing faces of solid, opaque
`Occluder` boxes (cuboid and plane actors) are rasterized, eroded a texel and
reduced to a max-depth pyramid. Bevy's GPU `OcclusionCulling` and GPU frustum
culling (`NoCpuCulling`) are camera toggles in `OcclusionPolicy`. Counts reach
the profiler as `culling/*`.

### GPU measurement

`blockloom-runtime/src/gpu.rs`. Bevy's `RenderDiagnosticsPlugin` times each
pass; `gpu.rs` adds one timestamp pair around the whole frame (an encoder
pushed in `RenderGraphSystems::Begin` and one before `Submit`, read back a few
frames later) as `gpu/frame`. Every `MEMORY_EVERY` frames it sorts the wgpu
allocator's report into `Kind`s by allocation label (`Kind::of`), so a new
render target needs a label that sorts, not code. Without a report (Metal) it
sizes the views' own textures instead. All of it reaches the profiler as
`gpu/*` and `memory/*`.

### Loading and streaming

`blockloom-runtime/src/streaming.rs`. `StreamingCells` is the one cell system:
XZ cells around the world camera with a hysteresis band, admitted nearest
first under a per-frame budget and announced as `CellEntered`/`CellLeft`
messages (a rebuild sends every cell as left). Content streams by being a
payload: it reacts to those messages and does its work through `CellTasks`,
which runs on the async compute pool and holds the cell as loading until the
task lands. Terrain chunks and the like belong there, not in a second system.

A look whose files are still loading gets `Loading`: a flat `Placeholder`
child in 3D (Bevy draws nothing for a material whose texture isn't in yet),
and a `FadeIn` on the sprite in 2D once the image lands. A model's own box is
its placeholder already.

Every rebuild opens a `Warmup` window: everything with bounds is drawn
unculled (`NoFrustumCulling`, and `cull_views` stands down), so every pipeline
compiles now rather than when a thing first turns up on screen. It closes
after a few quiet frames with no loads, no loading cells and no pipelines in
the render world's backlog (`PipelineBacklog`), or at its timeout. Start
doesn't press the green flag itself in a rendering world: `engine.starting`
holds it (reported to the editor as running) and `warm_up` calls
`world::begin_run` when the window closes, so a built player's first frame
is warm too. A bare test world (`engine.prewarm` false) starts on the spot.
At build time `build::check_shaders` compiles every `.wesl` surface file the
project draws with, and a broken one fails the build. Counts reach the
profiler as `streaming/*`.

### Environment

`blockloom-runtime/src/environment.rs`. `Environment` is the one resource
the background, sun, ambient, AO, exposure and post come from: each frame
`blend_environment` starts from the project's `World` settings, lays
`EnvironmentVolumes` over them by weight (numbers and colors lerp, switches
flip at half weight), and resolves exposure through `ExposureClaims`
(director beats auto-exposure beats the manual EV). `apply_environment`
writes it onto the world camera, the sun and `ClearColor` when it changes or
a rebuild spawns new ones, and it is extracted to the render world. Passes
read `Environment`, never `project.world.lighting`/`post`; the rebuild only
spawns a bare camera and sun.

### Environment volumes

`blockloom-core/src/volume.rs` is the model: a `Volume` component
(`VolumeSpec`) makes an actor a box, a sphere or a global layer with a
priority, a blend distance (outside the shape, in world units) and a weight.
Every property carries an HDRP-style override checkbox (`Override { on,
value }`, the value kept while unchecked), so a volume only changes what it
ticks. `coverage` is the shape maths, flat in 2D; `blend_order` sorts lowest
priority first so the highest wins.

`blockloom-runtime/src/volumes.rs` is the rest. `gather_volumes` weighs every
carried volume at the world camera (the editor camera in the scene view),
fills `EnvironmentVolumes` for `blend_environment`, and keeps the list in
`VolumeBlend`, which the atmosphere sample turns into `active volumes` and the
status reports as `Status.volumes`. `enable volume` and `set weight of volume`
(and a script's `enable_volume`/`set_volume_weight`) name a volume the way
`set my parent to` names a parent and last for the run
(`engine.volume_enabled`/`volume_weight`). The debug views are
`SceneView.volumes`: bounds with their blend feather, a heat map, and freeze,
which holds the blend and sends each property's lerp as
`Status.volume_trace`. The heat map is `volume_heat.rs`, a pass after
tonemapping that puts each pixel back in the world (the depth prepass in 3D,
the screen plane in 2D) and runs the same coverage maths in
`shaders/volume_heat.wesl` over at most `MAX_VOLUMES`; change the two
together. A selected volume shows grips in the scene view (`edit.rs`): a box
face moves with the opposite face held, a sphere's edge sets the radius, and
the outer ring the blend distance. The drag updates the world's copy of the
spec live, and lands as `Placed` with `volume` set, one undo step. A
property the Phase 5 systems add is one field on `VolumeOverrides`,
`EnvironmentOverride` and `Environment`, plus its row in the inspector's
`volumeProperties`.

### HDR frame and lights

Every world camera carries `Hdr` (`environment::apply_environment`), bloom or
not: the scene renders linear FP16 and only the end of the chain makes display
values, so lights, sky and emissives can pass 1.0. A build made with HDR off
(`GamePack.hdr`, the Build dialog's switch) renders 8-bit instead. Exposure
stays the one EV on `Environment`; `set exposure to` (and a script's
`set_exposure`) takes `ExposureClaims::director` for the rest of the run, and
the atmosphere slot's `exposure` reading reports the resolved value.

`World.display` is the project's output: SDR, HDR10 (PQ) or scRGB, peak
brightness and paper white in nits. `hdr::HdrFrame` resolves it, with `set HDR
output`/`set peak brightness` laid over it for the run, against what the
window's display offers (`DisplayOffers`), and is extracted so both worlds
switch on the same frame. Only the windowed player can leave SDR:
`display.rs` makes the window's surface itself in the render world, keeps it
when the display offers an HDR color space, and hands Bevy's views its
texture each frame; otherwise Bevy keeps the window. On an HDR frame the
tonemapper stands aside for `HdrTone*` (a curve to the display's headroom)
and `HdrEncode*` (scRGB or PQ, after the UI so the HUD sits at paper white).
An HDR swapchain also gets HDR10 static metadata
(`display::send_metadata`: `VK_EXT_hdr_metadata`, turned on by
`add_vulkan_extensions`, or DXGI), resent whenever the swapchain is remade.
The Game view's ring is 8-bit, so on Wayland an HDR frame bypasses it (see
Game view).

`hdr.rs` also holds the Game view's debug views, a `FullscreenMaterial` per
dimension over the exposed image before tonemapping (`shaders/hdr.wesl`): false
color, a clipping zebra past paper white (past the headroom when the project
wants HDR), a histogram, a waveform, calibration patches and an HDR preview.
The choice is `SceneView::debug_view`, an editor preference that applies while
a game runs too. `luminance.rs` meters the world camera's exposed image with a
compute pass and reads it back; the atmosphere sample turns it into the
`scene luminance` reporter's nits on the fixed tick, beside `is HDR display?`
and `peak brightness`. `capture.rs` answers `EditorMessage::CaptureExr`: a
second camera renders the same view untonemapped into FP16, read back and
written as OpenEXR (`capture_exr`, the Game view's camera button).

The sky (below) is drawn and lights the world in the same FP16 frame. An
HDR asset's exposure bias (`set_exposure_bias`) is applied wherever it is
loaded.
`set my glow to` gives an actor its own material with the emissive scaled
(`dim3::set_glow`).

A `Light` component is a point or spot light in lumens with a range in
metres, 3D only. `lights::sync_lights` reconciles each actor's light against
`engine.attached`, the authored spec and `engine.light_intensity` (what
`set my light to` wrote this run), and hangs it on a child entity, since
batching hides a merged actor through an empty `RenderLayers`.

The GPU half is checked by the ignored tests in `embed.rs` (`cargo test -p
blockloom-runtime -- --ignored embed`), which read pixels back from a real
world: false color in both dimensions, a lamp that still lights the floor
after batching, and an EXR capture that stays linear.

### Sky

`World::sky` (`blockloom-core/src/sky.rs`) is one of four kinds: `Flat` (the
background color and flat ambient, the default), `Physical` (Rayleigh, Mie
and ozone single scattering, a limb-darkened sun and a phased moon, a night
tint ramped on the sun), `Gradient` (three stops that warm near a low sun)
and `Hdri` (a panorama or strip, turned and tilted, tinted, optionally
blurred). `SunPlacement` decides where the sun stands for every kind - the
lighting's own direction, azimuth and elevation, or NOAA's solar position
from latitude, longitude, day and hour - and `Environment::from_world`
takes it from there, so volumes still override it. A physical sky's air
then reddens and dims the light (`Environment::through_air`, the CPU half of
the same model, kept as `Sun::above_air` for the sky to scatter). Old
documents' `lighting.sky` path becomes an HDRI sky in `Project::normalize`.

`blockloom-runtime/src/sky.rs` renders it, 3D only. `SkyParams` (the WESL
struct in core's `shaders/sky.wesl`, the `blockloom::sky` library module,
field for field) is resolved each frame from the sky and the blended
`Environment`. When it changes, `shaders/sky_cube.wesl` writes the sky's
light, disks left out, into a 256 cube and a 32 cube scaled by the ambient
dimmer; Bevy's `GeneratedEnvironmentMapLight` filters them on two helper
entities (`SkyProbe`), and the world camera's `EnvironmentMapLight` takes
reflections from one and diffuse from the other, or a black cube for
whichever the sky's toggles leave out. The filter stops a few frames after
the render world reports the cubes written (`SkyRender::written`) and the
pipeline backlog is empty, so a still sky is filtered once. Cubes store
nits over `SKY_UNIT` to stay inside FP16. The background is
`shaders/sky_background.wesl`, drawn per pixel in Bevy's opaque pass
through the skybox slot (`SkyView` supplies `SkyboxPipelineId`/
`SkyboxBindGroup`, never a `Skybox`): gradients per pixel with dither, the
physical sky from its cube plus analytic disks, an HDRI sharp from its image
or blurred from the filtered mips. Probe faces and EXR captures copy
`SkyView` (probe faces without the disks). A build bakes an HDRI to a BC6H
cube with its mip chain (`build::bake_sky`, seam fix applied) and drops the
source. `sky_exposure` and `ambient_dimmer` are volume properties too.

### Fog, space and lightning

`World::fog` (`blockloom-core/src/fog.rs`) is three media, 3D only: an
analytic exponential height fog, a froxel volumetric fog and aerial haze.
`blockloom-runtime/src/fog.rs` resolves them from the blended `Environment`
into `FogRender` each frame (so volumes, `set fog density to` and the time
of day reach them the way they reach the sun) and draws them in one pass
after the main passes and before any post. `shaders/fog_froxels.wesl`
injects each froxel's medium (height-falling density, FBM noise drifting
with `noise_wind`, and up to `MAX_LOCAL_FOG` local fog volumes) and its
in-scattered light: Bevy's directional lights read straight from its own
`GpuLights` and cascade shadow maps (the sun and moon carry
`VolumetricLight` while volumetric fog asks for them, which is what makes
light shafts), plus up to `MAX_FOG_LIGHTS` unshadowed point and spot
`Light`s whose `volumetric` switch is on. It blends with last frame's grid
reprojected, and a second entry point integrates each column front to back.
`shaders/fog_composite.wesl` then reads each pixel's distance from the depth
prepass: haze on surfaces, height fog along every ray (the sky's too, so
the horizon melts into it, glowing with the sky's horizon light from the
diffuse cube), then one froxel fetch. The shared WESL is the
`blockloom::fog` library module; `fog::FogUniforms` matches it field for
field. Fog properties on `Environment` (`fog_density`, `fog_colors`,
`fog_height`, `volumetric_density`, `volumetric_albedo`, `haze`) are volume
properties too, and a `Volume` can also add local fog in its own shape
(`VolumeSpec::fog`).

A light's `beam` (`fog::Beam`) is extra medium only that light scatters:
density, its own g, a falloff curve over the range and near/far fades,
mirrored by `beam_fade` in `blockloom::fog`. In the froxels it rides
`FogLight.beam`, and a beam alone runs the pass. `fog::pick_fog_lights` keeps
beams first, then the nearest, up to `MAX_FOG_LIGHTS`. `Environment.beams`
multiplies every beam (a volume property too), and `set fog density`
(`Environment::set_fog_density`) scales it and `volumetric_density` by the
asked density over the project's own, so a clear day has no beams.
`blockloom-runtime/src/beams.rs` is the geometry: a `BeamMode::Auto` beam
becomes an additive fresnel-faded shaft cone (`shaders/beam_shaft.wesl`,
spots only) while volumetric fog is off or Low, and `Motes` are GPU-placed
billboards (`shaders/beam_motes.wesl`) in a beam or, as
`VolumetricFog::dust`, in a box wrapped round the camera.

Stars, the Milky Way and aurora (`Sky::stars`, `Sky::aurora`) are drawn by
the sky's background pass in the main view only, never in probe faces or
the light cubes, from `space::SpaceRender` (`blockloom::space`). They sit in
their own uniform rather than `SkyParams` because they move every frame,
and a `SkyParams` change rewrites and refilters the cubes. So there are no
stars over a flat sky. `space.rs` also hangs the moon's own
`DirectionalLight` (`MoonLight`) where the physical sky puts it, dimmed by
its phase, and applies `set aurora to KP`.

`World::lightning` (`blockloom-core/src/lightning.rs`) is both dimensions.
`StormDirector` is plain arithmetic on the fixed tick from a seed, so one
storm replays the same. `blockloom-runtime/src/lightning.rs` turns strikes
(the director's and `strike lightning at`'s) into a decaying `PointLight`
above the ground (3D), a `LightningFlash` that pulses the environment's
ambient and background and the sky pass, and thunder late by distance at
the speed of sound: the project's sound, or `thunder_wav`, a rumble made in
code so no asset is needed. The fog, the aurora and the flash fill their
`AtmosphereSources` readings (`fog density`, `aurora`, `lightning`).

The GPU half is the ignored `embed` tests: height fog, volumetric glow,
sunlit fog and a roof's shadow in it, a volume's local fog, aurora, stars
and a lightning block.

### Wind

`World::wind` (`blockloom-core/src/wind.rs`) is the one wind, both
dimensions: a direction (degrees clockwise from north, -Z in 3D and up the
screen in 2D), a speed at the reference height, gusts on seeded 1D gradient
noise that also veer the direction, a log-law profile towards the ground (3D
only), and a storm dial that scales the rest (`StormScale`). A `Volume` can
carry a `LocalWind` zone (override, add or swirl round the actor's up axis,
plus turbulence), blended in `blend_order` like the rest of a volume.
`CloudDrift` is how the clouds ride it: the wind at their altitude times
`follow`, their own drift, an erosion drift and a time-lapse; `CloudOffsets`
integrates it. The public API speaks arrays because core's glam isn't
Bevy's.

`blockloom-runtime/src/wind.rs` steps it on the fixed tick's own clock
(`step_wind`, before `sample_atmosphere`), so a replay gusts the same and
the atmosphere slot's `wind speed`, `wind direction` and `storm` are the
wind at the camera that tick. Everything that moves with the air reads the
resulting `WindField`: particles ease into `field.at(position)` scaled by
their emitter's `wind`, the fog's noise scrolls by `field.drift`, and the
cloud passes are meant to take `field.clouds`. `set wind [dial] to` and
`set cloud drift to` (and a script's `set_wind`/`set_cloud_drift`) land in
`engine.wind` for the run.

### Lighting rig

`Light` (`LightSpec`) is a point, spot, rect or disk light. Rect and disk are
Bevy `RectLight`s with LTC speculars. `intensity` is lumens or, with `unit:
Candela`, the brightest direction's candela (x 4 pi to lumens). A cookie
(tiled over a spot's beam, or on each face of a point light) and an IES
profile are baked on the CPU into one R8 mask in `lights.rs`
(`spot_mask`/`point_mask`, which invert Bevy's own texture lookups) and
handed over as `SpotLightTexture`/`PointLightTexture`. Masks are cached in
`LightMasks` by file, tiling and cone.

`pbr_patch.rs` edits Bevy's own PBR shaders as they load, for what Bevy has
no switch for. A disk is sent with both extents negative (the side of its
equal-area square) and integrates as a 12-gon of the same area. A shadowed
area light gets a `ShadowTwin` child, a black point light at exactly its
position, whose cube shadow the rect loop looks up in the cluster; black
point lights are skipped in the point loop. The sun's shadows fade over the
last `ShadowSettings::fade` of the distance (a shader constant, so changing
it recompiles). Each patch is exact text against the pinned Bevy; one that
no longer matches reports an error and leaves Bevy's shader alone. Per-light contact shadows,
PCSS and biases ride the same spec; `turn my light's shadows` lands in
`engine.light_shadows`, which `wanted` lays over the authored spec, and `casts
shadows?` reads `ActorSense::casts_shadows`.

`Lighting::shadows` (`ShadowSettings`) is the sun and camera half, applied by
`shadows.rs`: cascades (count, first split, blend, distance - which `set
shadow distance` overrides as `engine.shadow_distance`), normal bias, a PCSS
sun size, the fade, the camera's `ShadowFilteringMethod` and `ContactShadows`
(16 steps).
`Lighting::sun_cookie` is a tiled `DirectionalLightTexture` whose tile size is
the sun's scale, which is safe because cascades read only its rotation. The
depth bias and everything volumes blend stay in `environment.rs`.

A `Probe` component (`blockloom-core/src/probe.rs`) is a reflection probe (a
box-projected cubemap, `ParallaxCorrection::Auto`) or an irradiance volume (a
brick grid of ambient cubes in Bevy's `(Rx, 2Ry, 3Rz)` atlas). Its box turns
with the actor but isn't scaled by it. `light_probes.rs` bakes through the
capture service: a reflection capture is rotated with the probe, since Bevy
samples a probe's cube in its own frame, while bricks are world-aligned, since
the irradiance lookup uses the world normal. Captures are divided back out of
the camera's exposure into radiance. `BakeProbes` (the inspector's Bake,
`bake-probes`) writes `.blockloom/probes/<actor>.{dds,irr,json}`; the json
holds `probe::stamp`, a hash of the probe and every lit component around it,
so `probe-status` and the scene view know a bake is stale, and an auto-bake
probe rebakes itself after the next rebuild once warm-up closes. A bake
leaves out its own actor (`ProbeRequest::hide`, applied by
`probes::hide_from_captures`), so a probe actor (`ProbeOwner`) is never
batched. Volumes scale probe light through `reflections` (reflection probes
and the sky's light) and `indirect` (irradiance volumes); probes blend with
each other by their own falloff. `capture
probes` does the same capture mid-run and keeps it in memory. Builds copy the
bakes. A reflection bake loads as BC6H where the GPU samples BC, and is
decoded to FP16 elsewhere, as the sky is. `pipeline::bc6h` encodes all 14
modes, and its decoder matches bcdec bit for bit. The GPU half is the ignored `embed` tests (rect and disk lights, area
light shadows, cookie, both probe kinds, a bake leaving out its own actor).
Those tests start their worlds one at a time: concurrent Vulkan instance
creation crashes in the loader.

### Ray-traced lighting

`blockloom-runtime/src/ray_tracing.rs`, 3D only, behind the runtime's default
`ray_tracing` cargo feature (Bevy Solari, pinned with Bevy).
`Lighting::ray_tracing` (`RayTracingSettings`) turns it on; `turn ray
tracing`, `set GI bounces to` and `set GI samples to` (and a script's
`set_ray_tracing`, `set_gi_bounces` and `set_gi_samples`) override it for the
run as
`engine.ray_tracing`/`gi_bounces`/`gi_samples`. `RayTracingState` probes the
device once for `SolariPlugins::required_wgpu_features`; without them the
raster rig carries on and the editor hears why once. The atmosphere sample
copies `active`/`available` on the fixed tick, so `is ray tracing on?` and
`ray tracing available?` agree between the VM and compiled logic.

`RayTracingSettings::mode` picks the realtime tracer. `Hybrid` puts
`SolariLighting` on the world camera (ReSTIR direct light plus a world cache
for GI). `PathTraced` puts `traced::TracedPaths` there instead: fresh paths
from every G-buffer pixel each frame (`shaders/traced_paths.wesl`, `paths`
per pixel, light sampling MIS'd against the BRDF), with Bevy's deferred
lighting skipped. Either way the camera also gets `Msaa::Off`, `Hdr` and a
storage-capable main texture, and sun shadow maps turn off. Tracing lights
the G-buffer, so while it is on `DefaultOpaqueRendererMethod` is deferred and
every standard and instanced material is touched to re-prepare; off, both go
back to forward.

`Denoiser` picks the cleanup: ReSTIR's reuse (`reuses`, Hybrid only) and
`traced::TracedDenoiser` (`filters`), an SVGF-style filter
(`shaders/denoise.wesl`) that runs after the opaque pass: demodulate by the
G-buffer albedo, reproject along motion vectors, five a-trous passes steered
by the variance, remodulate. Glossy surfaces keep a shorter history and a
tighter blur. It only touches pixels with a G-buffer (forward surfaces and
the background are left alone) and drops NaNs and clips fireflies first,
since its moments are half floats.

Solari ignores the environment map, so `solari_patch.rs` edits its shaders
as they load (the `pbr_patch` rules): a realtime bounce, a world cache GI ray
past its reach and a reference path tracer bounce that escape all see the
camera's `EnvironmentMapLight`. Under a flat sky `TracedAmbient` hangs a
one-texel cube of the ambient there instead, and a sky that lights but
doesn't reflect shows traced rays its diffuse cube. `instanced_pbr.wesl` is therefore
the instanced material's deferred shader too. Box-projected and graph
surfaces stay forward and keep the raster lights.

Solari only traces `RaytracingMesh3d` + `StandardMaterial` in one vertex
layout, and only the sun and emissive meshes light. So `sync_traced_scene`
(PostUpdate, after propagation) keeps a copy: one proxy per visible drawn
mesh (no `Mesh3d`, `traceable` converting the mesh, a standard material
standing in for instanced, box and graph surfaces) and one emissive stand-in
per `Light` whose `ray_traced` is on (a sphere for a point, a disk down a
spot's beam, the rect or disk itself), glowing with the light's power. A
spot narrower than `HOOD_WIDEST` also gets a black flared hood
(`TracedHood`), so its disk only lights the cone. Merged batches,
placeholders and particles are left out.

The path tracer is `SceneView::path_tracer`, an editor preference: the world
camera gets `Pathtracer` instead, which traces the same copy and starts over
when it or the camera moves (`drive_path_tracer`); progress and availability
reach the editor as `RuntimeMessage::RayTracing` and `state.ray_tracing`. An
EXR capture copies the camera's tracing (`trace_like`) and, under the path
tracer, waits for the sample or time budget. The GPU half is the ignored
`embed` tests (traced vs. untraced lamps, switching mid-run, the path tracer
and its EXR, sky and ambient on escaped rays, realtime path tracing, the
denoiser's grain, a spot's cone). They run on lavapipe too, which has ray
queries but no dma-bufs: `BLOCKLOOM_TEST_OPAQUE_FD=1
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`, and
`BLOCKLOOM_TEST_DUMP=<dir>` saves the frames some of them read as PNGs.

### Volumetric clouds

`World.clouds` (`blockloom-core/src/clouds.rs`) persists shape, altitude,
lighting and quality, edited by `set_clouds` and Project Settings. The blended
`Environment.clouds` carries volume coverage, density and type overrides, and
`set clouds [coverage/density/type] to` (a script's `set_clouds`) lays
`engine.clouds` over them for the run.
`blockloom-runtime/src/clouds.rs` bakes seeded, repeating 128³ shape and 32³
erosion volumes on the GPU, raymarches at half resolution before fog, and
uses the shared bilateral upsampler over the scene. WindField supplies drift.
`shape_volume`/`detail_volume` swap either bake for an authored volume asset,
read through `pipeline::load_volume`; `clouds::bake_noise` is the CPU twin of
`cloud_bake.wesl` (hash included), which `bake_cloud_noise` writes out as
strips. Change the two together.
Each view keeps two color/depth histories, invalidated on edits, resize,
skipped frames and fast motion. Sun and moon have separate shadow switches.
The 2048² ground transmittance map is a screen-space approximation over the
combined surface lighting; it does not isolate the PBR direct-light term.
Profiler readback reports primary steps and contributing samples per pixel;
render diagnostics time the march. GPU tests cover sky, terrain preservation
and ground shadows (`cargo test -p blockloom-runtime volumetric_clouds --lib
-- --ignored`).

### Shader library and pass plumbing

`blockloom-core/src/shader_lib.rs` holds Blockloom's own WESL modules
(`src/shaders/`): `hash` (PCG), `noise` (value, gradient, Worley), `fbm`,
`scattering` (phase functions, Beer, per-step integral) and `frame`, the
standard per-view `FrameUniforms`. They live in core so the editor's shader
check can link them; `shader_lib::validate` links a module against them and
runs naga, which is how their tests (and the runtime's shader tests) check
them without a GPU. The runtime registers each as `blockloom::<name>`
(`blockloom-runtime/src/passes.rs`), so built-in passes and surface files
import them like Bevy's.

`passes.rs` is also the shared pass plumbing, both dimensions. A camera
asks for it with `WorkingTargets` (like `DepthPrepass`); the render world then
gives the view a `WorkingTargetSet` (one full-res `Rgba16Float` target and a
half-res scratch pair, labelled `working_*`), `ViewFrameUniforms` into the one
`FrameUniformBuffer` filled from the extracted `Environment`, and
`ViewUpsample`'s bilateral upsample pipelines, queued up front so warm-up
compiles them. `passes::upsample` draws scratch onto a full-res target,
guided by the 3D depth prepass (plain bilinear in 2D). A half-res texel
stands for the full-res pixel at twice its coordinate (`frame::full_texel`).
Nothing asks for `WorkingTargets` yet; Phase 5's passes are the consumers.

### Import roles, probes and the atmosphere slot

The asset pipeline (`blockloom-core/src/pipeline/`) inspects, plans and
fingerprints every file. An `ImportRole` says what a file imports *as*:
texture, HDR (`.hdr`/`.exr`, planned as BC6H), volume (`.cube` LUTs, or an
image strip of slices), heightmap (16-bit images, `.r16`/`.r32`), IES profile,
or light cookie. The extension picks one; a PNG heightmap or cookie is an
override kept in `.blockloom/pipeline.json` (`set-import-role`), and changing
a role dirties the asset. Each role's decoder is its own submodule, and a pass
reads files through `pipeline::load_volume`/`load_heightmap`/`load_ies` rather
than parsing them itself. A new format extends a role here, not a second
importer.

`blockloom-runtime/src/probes.rs` is probe capture as a service, 3D only:
`ProbeService::request` spawns six 90° FP16 face cameras (`FACES` is cubemap
order, with Bevy's z flip), announces `ProbeCaptured` after a couple of
frames, and with `readback` assembles the faces into a cube image. HDRI
baking, reflection probes and water reflections are requests with different
defaults (`ProbeRequest::hdri`/`reflection`/`water`); none owns cameras.

`Sensors::atmosphere` is the snapshot's sun/wind/fog/weather slot, versioned
by `ATMOSPHERE_VERSION`. `atmosphere::sample_atmosphere` writes it at the head
of every fixed tick, before any scheduler, from the blended `Environment` and
`AtmosphereSources` (which Phase 5's wind and weather systems fill), and
`publish_sensors` copies that sample rather than resampling, so a frame
between ticks reads what the tick read. Every reader goes through
`AtmosphereSense::field`: the `Atmosphere` reporter (VM, and compiled logic
through `sense`) and a script's `atmosphere()` (`READ_ATMOSPHERE`). A new
reading is one arm there, one entry in `ATMOSPHERE_FIELDS` and the reporter's
dropdown in `Blocks.qml`.

### Models, tilemaps and surface shaders

A `Visual::Model` draws its glTF/GLB file's first scene as a child of the
actor (`blockloom-runtime/src/model.rs`), scaled by the look's `scale`, and
loops the animation the look names (the file's first when empty). The
authored box stands in until the scene is ready, stays for OBJ/FBX or a file
that won't load, and is always what the actor collides as. `ModelCache` keeps
loaded files alive across rebuilds so an edit doesn't flash the box.

A solid tilemap collides per tile, as the merged rects of
`Tilemap::solid_rects` (minus its `passable` tiles), and nav blocks the same
rects. Animated tiles cycle their frames on the wall clock;
`materials::animate_tiles` rewrites only the mesh's UVs when a frame turns.

A custom effect's `GraphEffect::starter_graph` is the uniform path spelled as
graph nodes, so `export_shader` writes it to a `.wesl` asset that draws the
same thing. An effect whose `source` names a `.wesl` file draws with that
file's `graph_main(uv, time)`: `material::surface_module` wraps it in the
dimension's head, `SURFACE_BINDINGS` and fragment tail, hoisting the file's
own imports up beside the head's (WESL wants every import before the first
declaration), and `materials::surface_shader` swaps that in through the
material's `specialize`. `check_surface_wesl` parses the same module, refuses
names the wrapper already has and imports the dimension hasn't loaded, and
runs naga's full type check when the file has no imports, so the editor and
the GPU agree on what compiles.

The built-in shaders (`src/shaders/*.wesl`) and that wrapper are WESL, since
Bevy 0.20 hands plain WGSL to wgpu untouched: imports are `import
bevy_pbr::render::...`, shader defs are `@if(DEF)`, and the bind group is
`constants::MATERIAL_BIND_GROUP`. A user's surface file is WESL too and may
import Blockloom's library (`blockloom::fbm`, ...) and Bevy's own modules
(`bevy_pbr` in 3D, `bevy_sprite_render` in 2D);
a project's other files aren't modules, so `package::`/`super::` are refused.

### How a project runs

1. Play hands the runtime the whole project (`EditorMessage::Load`) and starts
   it. Nothing is shared but that message - even in-process, the world gets its
   own copy - and the runtime owns the world from then on.
2. `vm::compile` flattens each actor's canvas into a `Vec<Step>` with jumps -
   a nested tree can't be suspended mid-body, but a program counter can. Header
   strands become entry points keyed by their trigger.
   Each value slot is lowered the first time it runs (`vm/lower.rs`): variable
   names become interned slots in `Variables`, and pure operators over
   constants fold away, so evaluating one hashes nothing.
3. Simulation runs on Bevy's `FixedUpdate`: a constant-rate step (`FixedMain`
   catches up whatever the display does) that pulls the project's `world.fixed_rate`
   - set in Project Settings and applied by `pump_editor`/`dim2|dim3::sync_timestep` -
   so blocks and physics advance on the same tick whatever the frame rate. The
   per-frame `Update` publishes the sensor snapshot (`sense::publish`), turns input
   and rapier contacts into `vm::Event`s (at most once a frame, so a few sunk fixed
   steps never repeat a keypress), and reads `Vm::tick`'s results next step.
4. The runtime applies those effects to the ECS - actors made and unmade in
   `world::apply_lifetimes`, shared ones in `world::apply_common`, physics and
   material ones in the dimension's own module - and reports says, errors and a
   periodic status back to the editor.
   Every actor carries a `PhysicsPose`/`PrevPose` pair, and `record_poses`
   (`FixedPostUpdate`) + `interpolate_poses` (`Update`) draw each between fixed
   steps so fast displays don't see them - physics bodies from the pose physics
   wrote, and a `move`/`glide` sprite from the pose its step's effects pushed
   it to, both just as smooth as a rolling ball. `restore_poses` puts the
   settled pose back at the head of every fixed step, so simulation builds on
   where the actor actually is rather than on the frame the renderer drew.

Scripts yield the way Scratch's do: at a `wait`, and once per loop iteration.
That one rule is why `forever` costs one step per fixed tick instead of hanging
the process, and it's checked directly in `blockloom-core/tests/vm.rs`.

The VM holds `Rc`s, so it is a `!Send` Bevy resource - which is exactly right:
every system touching it is therefore scheduled on the main thread, the same
thread the thread-local sensor snapshot lives on. Keep it that way.

### Sound

`blockloom-core/src/sound.rs` is the model: a `SoundBus` (`Master`, `Music`,
`Sfx`), a `SoundMixer` of one linear gain per bus saved on the `World`, and
the 0-100 block scale both ways (`user_to_gain`/`gain_to_user`). Blocks are
`play sound` (global), `play sound at` (positional: follows an actor, panned
by Bevy spatial audio with per-frame distance falloff), `stop sound` (empty
stops all), `set sound volume`/`pitch` (live voices of one file), and `set
bus volume` (window-global like gravity). `blockloom-runtime/src/sound.rs`
owns the voices: one entity per play with an `AudioPlayer`, handles cached by
path, oldest stolen past 8 of a file or 128 total. Sounds ignore the pause
freeze (a menu click still clicks), and `stop all` silences them. Reporters
are `is playing?` and `bus volume`, read off the published snapshot.

### Building a game

Build is not Export. Export writes a `.blockloom` file for somebody else's
editor; Build makes a runnable folder and a ZIP somebody can share without
Blockloom at all. Windows keeps the player and `game/` together. Linux adds a
portable shell launcher, a `.desktop` entry and a PNG icon. macOS uses the
native bundle layout:

```text
Pond Game/
  Pond Game.exe        the player: `blockloom-runtime`, renamed
  Pond Game.ico        the project icon, also embedded in the executable
  game/
    game.pack          the document, and the format version it was written at
    assets/...         the project's assets, minus the script sources
    .blockloom/atlas.* the Image looks baked into one sprite sheet
    .blockloom/build/  native blocks and script libraries
```

```text
Pond Game.app/
  Contents/
    Info.plist
    MacOS/Pond Game
    Resources/
      GameIcon.icns
      game/...
```

`blockloom-core/src/build.rs` lays that out and `commands::build_game` drives
it. The player finds `game/game.pack` beside its own executable, or under the
app's `Contents/Resources` on macOS. That `game/` is handed to the runtime as
the project folder, which is why assets and native libraries keep the spelling
they have inside a project.

Project Settings holds one image asset for build branding. Packaging converts
it into a multi-size Windows ICO, a macOS ICNS and a Linux PNG; an empty setting
uses Blockloom's bundled icon. The ZIP contains the platform-named build folder
as its top-level entry and preserves executable bits for Linux and macOS.

`player::Launch` is the one fork: a pack beside the binary means player mode,
which takes its dimension from the document rather than `--mode`, presses its
own green flag, never reads stdin, reports nothing (`bridge::attached` is
false, so the status corner and the editor handshake are skipped), and exits
when the world stops, since nothing can press Play again - `stop all` is how a
built game quits. `--play <folder>` runs a build without renaming anything.

A build always ships the document so the VM remains a fallback. With "Compile
blocks for maximum speed" enabled, it also ships one optimized native logic
library and the player schedules that instead. The option defaults on when the
project and target toolchain support it. Scripts ship as the libraries the
editor already compiled, and one that won't compile fails the build rather
than shipping an actor that quietly does nothing.

Which platforms an install can build for is a question about what it has beside
it. The player is a native binary Blockloom can't produce, so one per platform
is staged under `players/<triple>/` next to the editor: `just player` puts this
machine's there (the `dist` profile - fat LTO, one codegen unit, stripped -
while `just build` keeps the release profile's quicker link for the edit-run
loop), and `just stage-player <triple> <file>` takes one built elsewhere. The
machine doing the building always has one, since the runtime the editor plays
with is a player. Scripts are native too, so a project with one can only be
built for a platform this machine's rustc can compile for; `script::compile_for`
cross-builds them into `.blockloom/build/<triple>/`, leaving Play's own build
where the runtime has always looked for it. `build::targets` answers both
questions at once, which is what the Build dialog lists, with the reason
attached to every platform it can't offer.

A build folder is named for the project and the platform - `Pond Game (Linux
x64)` - because one output folder holds a build per platform, and three folders
called the same thing would be three chances to ship the wrong one.

### Compiling the blocks

`blockloom-core/src/codegen/` emits a project's blocks as Rust source: an
expression becomes an expression, a variable read becomes a call rather than a
hash lookup, and an actor's canvas becomes a function. It is meant for the same
no-Cargo `rustc` pipeline the scripts use, so a built game can carry native
logic beside its native scripts.

A strand can be suspended, so it isn't a straight run of Rust: it comes out as
a `match` over the program counter, over the very same flattened `Vec<Step>`
the VM walks. Straight-line blocks fuse into one arm, and every point the VM
can give the frame back - a `wait`, a `glide`, each iteration of a loop - is an
arm of its own that picks up where it left off. What the VM works out at run
time on a frame stack is known here while emitting, so a `repeat` counts down
in a flat slot the entry table sized and an `escape loop` is just a jump.

One function covers a whole actor rather than one strand, because every strand
and every custom block body live in one step list with one set of numbers, and
a custom block called as a statement is a jump into somebody else's region with
a return address pushed. The VM's `immediate` flag - a reporter body run to
completion in place - becomes a second function over the same steps, with its
own state, so a reporter may call another or itself and the strand's program
counter never moves. Each actor is therefore emitted at most twice, however
many custom blocks it has.

The rule it has to keep is that a compiled program and the VM ask the world for
exactly the same things in the same order, on the same tick - including the
mistakes, since a bad slot reports itself once and stands a zero in its place,
while a slot that evaluated fine but isn't a number is silently zero.
`tests/codegen.rs` is that rule: it runs a project both ways off one clock,
prints what each one asked for and which tick it asked on, and compares line
for line. Add a block to the emitter and add a case there, or the two halves
drift and a compiled game stops meaning what the played one meant.

The one difference on purpose is the VM's per-tick step budget, which compiled
strands don't count against. Every back edge belongs to a loop and every loop
yields, so a compiled strand can't spin; the budget only catches ten thousand
straight-line blocks in a row, and a counter on every block would cost what
compiling was for. A reporter body does count, since nothing in one yields and
the budget is the only thing that ends a runaway.

`codegen/runtime.rs` is the support code a generated program is built on - the
value type, the operators over it, the `State` a suspended strand is kept in,
and the `Host` trait it asks through. Like `script/abi.rs` it is compiled
twice, once into `blockloom-core` so the tests can hold it against `Evaluated`
and against the VM's own limits, and once as text into every emitted program.
Anything that reads the world or the clock is a question for the host, so
`random` and `current time` go the same way sensing blocks do: one run of a
game has one source of each rather than two that disagree.

Three things shape the emitted code. Every slot is read into a `let` before the
act that uses it, because reading a slot borrows the host and so does handing
it something to do. `and`/`or` take their second operand as a closure, because
the VM's short circuit is observable: `false and <a bad slot>` reports nothing.
And every variable, parameter and reporter call is hoisted into a `let` ahead
of the expression, because the VM runs each of them before the operator over
it: a reporter on the side `and` never reads still runs, and still does
whatever it does to the world.

The actor is a value rather than a constant, which is what lets one emitted
function cover an authored actor and every clone of it: a `State` carries the
id it is running under, and `Entry` says which authored actor's strand it is.
The generated program keeps its own `Actors` table, the same one the VM keeps
and for the same reason - `delete` and `create a clone of` name an actor the
way every block does, and both have to be answerable before the host has done
anything about them. So the program mints the id, copies the scheduling and
queues the copy's `when I start as a clone` strands for the top of the next
tick, and the host is left with the entity. `NAMES` lists every actor the
document has, blocks or none, because an empty canvas still answers to its
name. A clone or a deletion from outside the program - a script's - comes in
through `fire` as a `Cloned`, `Created` or `Deleted` kind instead.

What it won't compile is a custom block that can reach itself through
statement calls: its loops would share one set of counters where the VM gives
every invocation a frame. `Unsupported` refuses the whole project rather than
emitting half of one, so the Build dialog can disable native logic and name
what sent it there.

`vm::Variables` is the live variable home shared by either scheduler. Generated
logic exports one runner behind the ABI in `codegen/runtime.rs`, and
`blockloom-runtime/src/logic.rs` loads it, translates events and effects, and
answers variable and sensing callbacks. Editor Play stays on the VM as the
reference behavior. A packaged player uses the native runner whenever its
build carries one, and falls back to the VM when it does not.

### Interface

A game builds its HUD and its menus out of blocks, in screen space, over the
world. `blockloom-core/src/ui.rs` is the vocabulary: seven element kinds
(`Panel`, `Label`, `Button`, `Image`, `Input`, `Slider`, `Toggle`), a 9-point
anchor, and the properties `set [prop] of (id) to` can write. An element is
named by an id string the project invents, and `show` makes one *or updates
the one that id already names* - so a HUD strand can rebuild itself every
frame without piling up. `hide` takes one off the screen without forgetting
it, children and all; `delete` forgets it. A parented element flows after its
siblings inside its parent's vertical stack and its own placement is ignored,
which is what makes Resume / Settings / Quit a three-block menu.

Every `show` block spells its id, caption and placement the same way, so the
element kinds share one set of field ids (`UiId`, `UiContent`, `UiX`, ...). The
JSON field is `element` rather than `id`, because a flattened instruction
already carries its own `id` on the wire (see `wire.rs`).

Three properties have no `show` row and so live beside the style rather than
on the element: a slider's `step`, and a text input's `allow` and `max
length`. A HUD strand that re-shows its own slider every frame would
otherwise wipe them. `step` rounds a slider's number to a multiple of itself
measured from `min`, which is also applied to a number written straight in,
so a step that doesn't divide the span leaves the far end short. `allow`
names a character set (`any`, `numbers`, `digits`, `letters`) by a word
somebody types, and anything else reads as `any` - a typo shouldn't deaden a
field. A refused keystroke is simply not there: no error, since somebody
holding a key down means no harm by it.

Five reporters read the interface: `value of (id)` and `text of (id)`
(which is a label's words, or an empty input's placeholder), `is (id)
shown?`, `does (id) exist?` and `the focused element`. `hide` leaves an
element existing but not shown; only `delete` takes it out of both. An id
nothing answers to is an error for the first two, the way a missing actor
is, and plainly false for the other two.

`blockloom-runtime/src/ui.rs` is the id map and the rules over it - the
hit test, the subtree walk, the anchoring - Bevy-free but for the entity
handle, so they unit-test without a window. `overlay.rs` is the Bevy half:
`apply_ui_effects` folds the fixed step's effects into the map, and `draw_ui`
spawns, despawns and restyles. An element's anchor is a percentage plus a
`UiTransform` of its own size rather than a worked-out pixel offset, because
an auto-sized label isn't measured until Bevy has laid it out - and a window
resize then recomputes for free.

Clicks route through the interface first (`world::detect_clicks`): the
topmost visible element whose rectangle covers the pointer wins, and only
what nothing wanted reaches the world picks. A visible modal element swallows
the rest, so clicking beside a pause menu never fires the gun behind it. A
click on a text input hands it the keyboard; while it holds it, game strands
see no keys at all. `focus (id)` and `clear focus` move the keyboard without
a click, and the input holding it is drawn brighter, since otherwise nothing
would say where the typing is going.

`pause game` freezes the world: no world strand advances, no physics steps,
no key or collision event queues, so resuming never bursts. The one fork is
that strands a UI event started keep ticking - otherwise a pause menu's own
buttons would be dead - and they run on the wall clock, so a `wait 1` blink
on a frozen menu still blinks. A `pause game` in a world strand stops that
strand where it stands, the way `delete myself` does; in a UI strand it
doesn't. `world::set_paused` is the one place that flips it, whether the
editor's Pause button or the block asked.

Pointer lock is fully manual: game code unlocks around a menu and re-locks on
close. In the Game view the editor holds it on the world's behalf (see Game
view above). The one safety net is that showing a modal while the pointer is locked
logs a warning, since a locked hidden cursor can't press anything.

### Frontend (`blockloom-qt/qml/`)

`Main.qml` holds the one copy of backend state (`appState`, parsed from
`bridge.stateJson`) and `invoke(command, args, done, failed)`, which every
other file calls. `Blocks.qml` (a singleton) is the single wiring point into
blockstitch and where a block's row comes from: every block is described once
as a list of pieces (a label, a value slot, a dropdown, a text field) and
registered with blockstitch's `BlockRegistry`, which both the canvas and the
palette read. **Adding a block means adding a variant to `InstructionKind`, a
row and an icon in `Blocks.qml`'s `buildRows()`, a label in its `labels`, a
field id if it has value slots, and a `Step`/`Effect` if it does something
new** - not a new QML file. `BlockHeader` and `CallBlock` take their row from
a `BlockDef` rather than their type, which blockstitch draws itself.

The asset tray along the bottom (`AssetTray.qml`) is a file manager
over the project folder: it lists, makes, imports, renames, moves and deletes
files, and an asset dragged out of it lands on any
`AssetField.qml` (the Look component's Image, the Script component's path).
Asset paths are relative to the project folder with forward slashes
(`assets/sprites/player.png`), the same spelling `Script` uses, and
`Project::repoint_asset` follows a renamed or moved file through the document
so the actor using it doesn't end up pointing at nothing. The listing is not
part of the state snapshot - a folder changes for reasons the editor never
hears about - so the tray re-lists after each of its own changes.

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
which of the two shows. Every edit is still written to disk right after it
lands.

### Known gaps

- The interface has three global themes plus per-element overrides and
  scrollable UI lists. Variables persist only when a `save variable` block or
  the matching script call writes them to per-player save data.
- A text input is basic: no selection, no cursor, no IME. Backspace rubs out,
  Escape and Enter let go, and every other character key appends - subject to
  the input's own `allow` and `max length`, which is all the validation there
  is. Backspace ignores both, so a rule written after the typing doesn't trap
  what is already in the field.
- A clone copies the template as the editor authored it, standing where the
  template stands now. What `attach`/`detach` did to the template since Play
  doesn't carry over - re-attaching a component has always meant the authored
  one.
- Building for another platform needs its player staged by hand, and a scripted
  project also needs that target's `std` and a linker for it.
- Recursive statement-shaped custom blocks fall back to the VM because their
  loop counters still need to move onto each call frame.
- A script needs a Rust toolchain on the machine that presses Play, which a
  packaged install can't assume. The script editor is a plain textarea, and a
  script's errors only show in the run log.
- A script can't be attached mid-run: its library is opened when the world is
  built.
- `say` shows as a camera-projected speech bubble over its actor in both 2D and
  3D, and is also recorded in the editor log. Bubble styling is saved on the
  world with an optional font asset path, which no inspector row exposes yet.
- A reporter-shaped custom block runs to completion in place, so a `wait` inside
  one passes straight through.
- The scene view's camera starts over whenever the world does (a dimension
  switch, reopening a project).
