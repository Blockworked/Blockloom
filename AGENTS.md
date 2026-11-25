# AGENTS.md

This file provides guidance to coding agents (Codex, opencode and others) when working with code in this repository.

## Project

Blockloom is a Qt/QML desktop app (Windows/Linux/macOS) for building games out
of blocks - a Scratch-style block editor driving a real game world, in 2D or
in 3D. A project is a world plus a set of actors, each with its own block
canvas; pressing Play runs those blocks against a Bevy world, shown in the
editor's Game view.

## Common commands

Patched Bevy/wgpu sources are generated into gitignored `.patched-deps/` from
the small diffs under `patches/`. `just` Cargo recipes prepare them automatically.
Before invoking Cargo directly on a fresh checkout, run `just prepare-patched-deps`.
See `patches/README.md` for prerequisites and updating patches.

```bash
just build              # cargo build --release --workspace (the normal build)
just run                # build, then launch target/release/blockloom
just qml-preview        # Qt 6.12 live QML edits, with the real backend
just replace-fast       # Linux: rebuild/reinstall editor and runtime, reuse staged players
just replace [jobs]     # build editor/web concurrently, reuse runtime, reinstall
just prune-target --dry-run          # inspect the 20 GiB build-cache budget
cargo build --workspace && target/debug/blockloom   # debug build/run - faster iteration
just test               # cargo test --workspace (blockloom-core has the bulk of them)
just qml-test           # every QML test against the pinned blockstitch, no build
just smoke              # shell flows: edit, undo, Play, Build, run the player, a scripted project (needs a display)
just corpus [--bless]   # play corpus projects to a fixed tick, held to tests/corpus/
cargo bench -p blockloom-core --bench vm   # block VM ns/tick over a few canvases
just player             # stage the hard-optimized player a built game ships
just web-check          # runtime check-build for wasm32-unknown-unknown (Phase 8)
just web-player [profile]            # stage the WebGPU player the Web build target ships
just web-player release              # faster web development build without dist's fat LTO
just web-build <project> [out]       # a project as one self-contained .html
just web-smoke <page.html> [--scripts N] [--moves ACTOR]  # headless run of a built page
```

`just replace` divides the CPU count across its two builds by default;
`just replace 4` uses four Cargo jobs per build. `CARGO_BUILD_JOBS` overrides
the default per-build limit. Web players use `target/web-build` for both final
and intermediate artifacts to avoid the native player's host-tool output locks;
their first build with this cache recompiles dependencies. Staged player paths
stay the same. The native player reuses the optimized runtime from the workspace
build, avoiding a second native compilation.
`BLOCKLOOM_NATIVE_PROFILE=dist just replace` builds the fat-LTO native player alongside the editor and
web player, dividing CPUs across three builds. `just player` still uses `dist`.
Replacement fetches native and web dependencies once before starting the builds
offline, so network access does not serialize the parallel compilation steps.
It keeps one status row below build output in an interactive terminal, showing
each build's state and elapsed time. Redirected output gets periodic plain lines.
Replacement uses the incremental `release` profile for web builds by default;
`BLOCKLOOM_WEB_PROFILE=dist just replace` requests the fat-LTO shipping profile.
`just web-player` still defaults to `dist`. Web builds compile only the library
that wasm-bindgen loads, skipping the unused runtime executable.

Build recipes use `scripts/prune-target.py` before and after compilation
to keep the whole target tree within a 20 GiB budget. Python 3 is required.
`BLOCKLOOM_TARGET_LIMIT_GIB` changes the budget. Cleanup evicts old incremental
caches first, then older profile caches, preserving binaries and staged players.
It locks Cargo profile directories and skips active builds. Temporary overshoot
and protected outputs are allowed; this is not a filesystem quota. Direct Cargo
commands need `just prune-target` afterward. See README.md's Build Cache Budget.

Build the whole workspace, not just `-p blockloom`: off Linux (or with
`BLOCKLOOM_RUNTIME=process`) the editor starts the `blockloom-runtime` binary
sitting next to it, and a stale or missing runtime is exactly what `cargo run -p
blockloom` would leave you with. The editor says so in a banner rather than
letting Play do nothing.

`rust-toolchain.toml` pins the Rust version (rustup installs it on first
build). There's no clippy.toml/rustfmt.toml - just `cargo clippy`/`cargo fmt` with
defaults.

## Style

- `CLAUDE.md` and `AGENTS.md` are the same text (only their first lines differ): change both together.
- NEVER use em-dashes (-) in comments, code, docs, or commit messages. Use a normal hyphen (-) instead.
- Write comments like a human: short and terse, 1-2 lines. State what and why briefly. No multi-paragraph essays, no history lessons ("Previously..."), no dangling "see X docs" pointers unless X exists.

### Frontend (`blockloom-qt/qml/`, QML)

The QML is compiled into the binary by `blockloom-qt/build.rs` (cxx-qt's
`CxxQtBuilder` + qmlcachegen), so a plain `cargo build` picks up every edit.
It needs Qt 6.12 or newer with Quick, QuickControls2, QuickDialogs2 and Multimedia; use
Qt 6's `qml`/`qmlls` (`/usr/lib/qt6/bin` on Arch), not Qt 5's. A new `.qml`
file must also be listed in `build.rs`'s `QmlModule`.

For live edits, `just qml-preview` builds the workspace with the development-only
`blockloom/qml-preview` feature and watches both QML modules through the selected
Qt kit's `qmlpreview`. It needs Python 3. Use `restart` for structural edits that
lose state; Rust/C++ changes and new QML files still need a rebuild. Run `just
build` before installing or shipping. See README.md's QML Hot Reload section.

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
just blockstitch-local [path]        # default path: ../blockstitch, relative to the repo root
just blockstitch-published [commit]  # restore the pinned commit, or move to a new one
```

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
hosts its own `Backend`. The command registry lives in
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
resources. A project's plugin commands are tools too: after `open-project`, `plugin-*`
and undo/redo the server re-reads `plugin-commands` and registers (or removes)
one tool per command, named `plugin-id__name` with a schema built from the
command's typed arguments (`src/plugins.ts`; a new plugin field type must be
taught there). Each session is its own backend unless started with `--attach` (see
above), so without it the window-and-shell sharing rules apply: the shell
attaches to the live owner's files and follows their saves, and one MCP server
process and the editor should still not write the same project at once. The shell resolves as `target/debug|release/blockloom-shell`
beside the repo, `BLOCKLOOM_MCP_SHELL`, or `--shell`; `just mcp` builds
everything and prints the client config line for the built `dist/index.js`.
Build/test with `cd mcp && pnpm install && pnpm run build && pnpm test`.

## Architecture

### Cargo workspace

The editor, and the game world. On Linux the world runs on a thread inside the
editor and draws straight into its Game view (see Game view in `docs/architecture/runtime.md`); on
Windows it runs on a thread too, but reads its target back and the view
uploads those bytes as an image. Elsewhere it is a child process.

- **`blockloom-qt`** (package `blockloom`) - the editor window and the app's
  entry point: Qt Quick over cxx-qt. It owns `blockloom-app` directly - there
  is no daemon. `src/app_bridge.rs` is the one QObject QML talks to:
  `invokeCommand` runs a `Backend::dispatch` command on a worker thread, in
  order, and answers with `replied(token, {ok, result, error})`; the
  backend's change stream lands in the `stateJson` property as a JSON array
  of the messages since the last burst (see Saving in `docs/architecture/data-model.md`).
  `src/preview.rs` follows a child runtime's MJPEG preview stream into
  `previewFrame`. `src/game_view.rs`/`.cpp` are the in-process Game view, and
  `src/pointer_lock.cpp` its pointer lock.
- **`blockloom-app`** - the backend: `src/commands.rs` holds every command,
  `src/dispatch.rs` maps command names + JSON args onto them, `src/state.rs`
  holds `SharedState`/`AppState` and the snapshot the frontend gets, and
  `src/runtime.rs` is the game runtime's leash. No Qt dependency.
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
  `src/player.rs` is the whole of the difference (see Building a game in `docs/architecture/shipping.md`).
- **`blockloom-protocol`** - the messages between them: newline-delimited JSON
  over a child's stdin/stdout, or the same enums over channels in-process. No
  sockets, no ports; the pipe or channel closing is the whole shutdown
  handshake. Bump `PROTOCOL_VERSION` whenever a message changes. Today it is
  40, the script `ABI_VERSION` 53 and `LOGIC_ABI_VERSION` 42; a version named
  further down is the one a feature arrived in.
- **`blockloom-core`** - the engine library both processes share: `scene.rs`
  (the value types a world is built from - looks, bodies, placements, camera),
  `components.rs` (what an actor is made of - see Components in `docs/architecture/data-model.md`), `blocks.rs` (the
  block vocabulary and its `BlockKind` impl), `fields.rs` (what the frontend
  calls each value slot), `project.rs` (the saved document and the folder it
  lives in), `library.rs` (the project folders the Dashboard lists),
  `assets.rs` (the files inside one of those folders, which the asset tray
  manages and the runtime loads images and fonts from), `pipeline/` (what
  import makes of each of those files - see Import roles in `docs/architecture/rendering.md`), `pack.rs` (that
  document again, as a built game carries it) with `build.rs` (what lays a
  build out), `vm/` (the block VM), `codegen/` (the same blocks as Rust
  instead),
  `script/` (compiling a project's Rust scripts, and the ABI they talk over),
  `sense.rs` (the world state reporter blocks read), `ui.rs` (the screen-space
  interface a game builds out of blocks - see Interface in `docs/architecture/interface.md`), and `wire.rs`
  (the one shape difference between documents and the frontend).
- **`blockloom-plugin-api`**, **`blockloom-plugin-host`**, **`blockloom-plugin-sdk`**, **`blockloom-plugin-gpu`** - the plugin platform:
  manifests, schemas, records and the C ABI (api, wasm-safe), resolver, cache,
  install transactions and the native loader (host). See Plugins in `docs/architecture/plugins.md`.
- **`blockloom-physics-probes`** - not part of the game: Phase 0 of the physics
  and character controller plan (`docs/physics-and-character-controller-plan.md`).
  Test-only probes against the pinned Rapier (`rapier2d`/`rapier3d` at the rev
  `bevy_rapier3d` uses) answering the rows of `docs/physics-compatibility-ledger.md`:
  `materials.rs` (static/dynamic friction through a `modify_solver_contacts` hook),
  `compounds.rs` (ownership, mass), `events.rs` (the Unity pair matrix, per-step
  event delivery), `queries.rs`, `ccd.rs` (what each CCD flag set holds) and
  `controller.rs` (Rapier's `KinematicCharacterController` against Unity's skin
  width, step offset, slopes, overlap recovery and platform carry). Findings to
  keep in mind before touching physics: queries are stale until
  `detect_collisions` runs, and syncing a never-stepped velocity-based kinematic
  body freezes it until `wake_up`; Rapier's `offset` is Unity's skin width but its
  autostep is not the step offset, and overlap recovery is capped per call; parry's
  capsule-vs-box `intersect_shape`/`contact` flicker within a centimetre of touching
  (use `distance`); Rapier's own kinematic carry lags and double counts with an
  explicit one. Unity values come from Unity's documentation, not an editor. CI
  runs it with the rest of the workspace; `cargo test -p blockloom-physics-probes
  -- --nocapture` prints the quoted numbers.
- **`blockloom-net`** - native multiplayer transport (Phase 0 of
  `docs/multiplayer-and-embedded-server-plan.md`, findings in
  `docs/multiplayer-phase0.md`). Quiche over a nonblocking UDP socket, driven
  by `poll()` from one thread, carrying opaque bytes: `Server` and `Client`
  with ALPN `blockloom-game/1`, a self-signed in-memory `Identity` pinned by
  its SHA-256 `Fingerprint` (a BoringSSL custom verify callback, never a
  disabled check), stateless Retry address validation, reliable streams with a
  4 MiB per-peer queue cap (`QueueFull`), datagrams, a peer cap and `kick`.
  `Impairment` (loss, duplication, delay, jitter, seeded) wraps the send path
  for tests, and Quiche's pacing times are honoured there. Server connection
  IDs must be full length (a short header does not carry one). `game.rs` is
  the LAN spectator session over it (`docs/multiplayer-lan.md`): `LanHost`
  sends each guest acknowledged deltas of a `Snapshot` (actors, plus plugin
  `Region`s keyed `plugin/key` such as voxel sections), staging regions that
  don't fit one 1 MiB frame over several. The runtime's `lan/native.rs` fills
  it, asking hosted modules `sync.sections` every `SYNC_EVERY` ticks. It
  cannot build for wasm32 (BoringSSL), so keep it out of the web and Android
  runtimes. `cargo test -p blockloom-net`; the RTT/loss matrix
  is `cargo test -p blockloom-net --test loopback -- --ignored --nocapture
  matrix`.
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

### Coordinates and units

World space is right-handed and Y-up in both dimensions (Bevy's, which glTF
shares, so models import without a flip). Rotations are degrees, positive
counterclockwise about +Z in 2D; a 2D actor faces +x and a 3D one -Z (a spot
light's forward). A 2D `z` is only draw order (`Place` z plus the Render
layer) and means nothing to physics.

The scale is the one deliberate difference: **3D is in metres, 2D is in
pixels**, a hundred to the metre (`scene::PIXELS_PER_METRE`, also rapier's
physics scale in 2D). At zoom 1 one 2D unit is one logical screen pixel
(`Camera2d`'s projection scale is `1 / zoom`), so a sprite's size is its
image's size and pixel art lines up. That is why 2D defaults are bigger
numbers: gravity -981 against -9.81, a 60 px starter player against a 0.5 m
ball, `PIXELS_PER_CELL_UNIT` 16, audio falloff `1/500`. Physical settings
(gravity, wind speed, fetch for the sea state) are in world units a second,
so in 2D they read in pixels; metre-based maths (`sea_target`, thunder delay)
divide by `PIXELS_PER_METRE` first.

Why it isn't Unity's or Unreal's: 3D already is Unity's (1 unit = 1 m, Y-up),
and Unreal's centimetres and Z-up would only add a conversion at every glTF
and Bevy boundary. Unity's left-handedness is not worth fighting Bevy's maths
and glTF for. For 2D, Godot, GameMaker and Scratch work in pixels too;
Unity's 2D is the outlier (metres plus a pixels-per-unit on each
sprite), and moving to it would change every saved position and size, plus
numbers typed into blocks and scripts that can't be migrated automatically.

Crossing dimensions converts lengths and nothing else. `Mode::length_from`
is the arithmetic; `Actor::convert_units` applies it to an actor's position,
look, parent offset and joint, and runs on a scene's `switch_mode` and on
`Persist` survivors carried by `switch scene to` (their live position is in
the old scene's units). Not converted, since the right answer isn't a plain
scale: tilemap tile sizes (32 px by default, never under 1), emitters, water,
volumes, rooms, parallax and the 3D camera component. Re-tune those after
switching.

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

### Architecture docs

The rest of the architecture lives in `docs/architecture/`, one page per area. Read the page for the area you touch before changing it, and keep it current in the same commit.

- `docs/architecture/data-model.md` (Document and data model): Saving, Resource database, Prefabs, Material and environment assets, Game settings, Components, Type registry, Actors that come and go.
- `docs/architecture/physics.md` (Physics): Physics document, Physics runtime, Physics queries, Physics inspector, Character controller, Character motor and input actions, Player camera, presets and profiles, Constraints and integration, Upgrade, samples and shipping.
- `docs/architecture/plugins.md` (Plugins): Plugins.
- `docs/architecture/blocks-and-scripts.md` (Blocks, scripts and compiled logic): Block verbs, Scripts, Script sandbox and guest languages, Compiling the blocks.
- `docs/architecture/runtime.md` (Runtime): Game view, Scene view, Simulation and presentation registration, Per-frame cost, Sound, Cinematics.
- `docs/architecture/rendering.md` (Rendering): Instancing and batching, LOD and occlusion, Quality and scaling, GPU measurement, Loading and streaming, Shader library and pass plumbing, Import roles, probes and the atmosphere slot, Models, tilemaps and surface shaders.
- `docs/architecture/environment.md` (Environment and lighting (3D)): Environment, Environment volumes, HDR frame and lights, Post stack, Sky, Fog, space and lightning, Wind, Water, Particles and VFX, Decals, Lighting rig, Ray-traced lighting, Volumetric clouds, World-space texturing, Terrain and vegetation, Cloud layers.
- `docs/architecture/2d.md` (2D): 2D lighting, 2D weather, 2D post, Normal-map authoring, 2D camera, 2D movement helpers, 2D screen feedback, 2D animation and sprites, Tilemaps and levels.
- `docs/architecture/shipping.md` (Building and shipping): Building a game, The web player, Android devices and the emulator.
- `docs/architecture/interface.md` (Interface): Interface, Interface editor, Interface effects and animation preview.
- `docs/architecture/editor.md` (Editor frontend and Hub): Frontend, Hub coordination.

### Known gaps

- The interface has three global themes plus per-element overrides and
  scrollable UI lists. Variables persist only when a `save variable` block or
  the matching script call writes them to per-player save data, in the run's
  save slot (`switch save slot to`, `save slot`/`save slots`; one file per
  slot, the default keeping the legacy file). Interface strings come from
  the project's string table (`set language to`, `language`, `text for
  key`), edited through `set-locale` and friends; scripts drive both through
  ABI 47 (`switch_save_slot`, `save_slot`/`save_slots`, `set_language`,
  `language`, `text_for`).
- A text input has a caret (`ui::TextCaret`): arrows, Home, End, Delete,
  Shift-select, Ctrl/Cmd+A and IME composition, all through the input's own
  `allow` and `max length`. The caret shows as `|` and a selection as `[..]`
  in the text; there is no pointer placement or drawn selection yet. Backspace
  and Delete ignore the rules, so a rule written after the typing doesn't trap
  what is already in the field.
- A clone copies the template as the editor authored it, standing where the
  template stands now. What `attach`/`detach` did to the template since Play
  doesn't carry over - re-attaching a component has always meant the authored
  one.
- Building for another platform needs its player staged by hand, and a scripted
  project also needs that target's `std` and a linker for it. The web player
  is staged with `just web-player`, and web builds need the wasm target's
  `std` for scripts (rust-lld ships with it).
- A web build loads whole before it starts: no streaming, and base64 costs a
  third over the gzip'd parts. Blocks run on the VM there, and `scene
  luminance`, GPU readbacks and the profiler behave as the browser allows.
- Recursive statement-shaped custom blocks fall back to the VM because their
  loop counters still need to move onto each call frame.
- A script needs its language's toolchain on the machine that presses Play
  (rustc for `.rs`, clang/componentize-py/TinyGo/... for a guest language),
  which a packaged install can't assume; nothing is bundled or downloaded on
  demand. The script editor is a plain textarea that only knows Rust, and a
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
- Rooms are axis-aligned boxes: turning a room's actor doesn't turn the
  room.
- 2D rigs draw Spine regions and meshes; DragonBones meshes, clipping and
  path attachments draw nothing. A rig's or stack's outline is one
  silhouette per piece, so a translucent outline color darkens where pieces'
  silhouettes overlap.
