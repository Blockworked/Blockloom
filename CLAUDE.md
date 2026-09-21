# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Blockloom is a Tauri desktop app (Windows/Linux/macOS) for building games out
of blocks - a Scratch-style block editor driving a real game world, in 2D or
in 3D. A project is a world plus a set of actors, each with its own block
canvas; pressing Play opens the world in a Bevy window and runs those blocks
against it.

## Common commands

```bash
just build              # cargo build --release --workspace (the normal build)
just run                # build, then launch target/release/blockloom
cargo build --workspace && target/debug/blockloom   # debug build/run - faster iteration
just test               # cargo test --workspace (blockloom-core has the bulk of them)
```

Build the whole workspace, not just `-p blockloom`: the editor starts the
`blockloom-runtime` binary sitting next to it, and a stale or missing runtime is
exactly what `cargo run -p blockloom` would leave you with. The editor says so
in a banner rather than letting Play do nothing.

There's no clippy.toml/rustfmt.toml - just `cargo clippy`/`cargo fmt` with
defaults.

## Style

- NEVER use em-dashes (-) in comments, code, docs, or commit messages. Use a normal hyphen (-) instead.
- Write comments like a human: short and terse, 1-2 lines. State what and why briefly. No multi-paragraph essays, no history lessons ("Previously..."), no dangling "see X docs" pointers unless X exists.

### Frontend (`ui/`, Vue 3 + TypeScript + Vite, pnpm)

You almost never need to build the frontend by hand: `src-tauri/build.rs` runs
`pnpm install && pnpm run build` before *every* `cargo build`/`cargo run`
(debug or release), so `ui/dist` is always current. Manual commands, if needed:

```bash
cd ui && pnpm run build   # vue-tsc --noEmit && vite build
cd ui && pnpm run dev     # Vite alone (see dev-bridge below to make it functional)
```

### Local `blockstitch` development

`blockstitch` is a separate sibling repo with two halves, both shared with any
other app built on the same block editor and both pinned to the same commit:

- the Vue component/theming library, a git dependency in `ui/package.json`;
- `crates/blockstitch-core`, the Rust backend (value system, document model,
  editor operations), a git dependency in the root `Cargo.toml`.

To iterate on either half locally without hand-patching `node_modules`:

```bash
just blockstitch-local [path]        # default path: ../../blockstitch, relative to ui/
just blockstitch-published [commit]  # restore the pinned commit, or move both halves to a new one
```

### Browser-driven dev workflow (`dev-bridge`)

```bash
just dev-backend    # the real backend + an HTTP/WS bridge on 127.0.0.1:4128
just dev-ui         # Vite alone; then open http://localhost:1420
```

`ui/src/bridge.ts` picks between the real Tauri `invoke` and this HTTP bridge
based on whether `window.__TAURI_INTERNALS__` exists, so the same frontend code
runs against the real backend in a plain browser tab - no relaunching the CEF
window after every UI tweak. Both paths end in `Backend::dispatch`
(`blockloom-app/src/dispatch.rs`), so a new command added to
`blockloom-app/src/commands.rs` only needs an arm there (argument names in
camelCase, as the frontend sends them) to be reachable from either. Play works
from the browser too: the backend spawns the same runtime process.

## Architecture

### Cargo workspace

Two processes: the editor window, and the game world.

- **`src-tauri`** (package `blockloom`) - the editor window and the app's entry
  point. Uses a custom CEF runtime (`tauri-runtime-cef` + the `cef` crate, both
  forks pinned via git `rev`/`[patch.crates-io]` in the root `Cargo.toml`)
  instead of Tauri's default wry/webview. It owns `blockloom-app` directly -
  there is no daemon - and forwards every frontend command through its one
  `call` command. Command responses include the resulting state directly;
  runtime-only changes are also sent through the `state-updated` event. Only
  window-local things live here (`reset_zoom`, the `.blockloom` file dialogs,
  the pre-paint background color in `theme.rs`).
- **`blockloom-app`** - the backend: `src/commands.rs` holds every command,
  `src/dispatch.rs` maps command names + JSON args onto them, `src/state.rs`
  holds `SharedState`/`AppState` and the snapshot the frontend gets, and
  `src/runtime.rs` is the game runtime's leash. No Tauri dependency, so the
  dev bridge (`src/bin/devserver.rs`, feature `dev-bridge`) hosts the same code.
  Commands publish changes through `AppHandle`, a plain callback the host
  supplies.
- **`blockloom-runtime`** - the game world: a Bevy app that renders one project
  and runs its blocks. A separate process because Bevy needs its own window and
  event loop and the editor's CEF runtime already owns one. `--mode 2d|3d`
  decides which physics/render pipeline is built, so the editor restarts it when
  a project switches dimension. `src/world.rs` holds the dimension-agnostic
  systems, `src/dim2.rs`/`src/dim3.rs` the sprite/`bevy_rapier2d` and
  mesh/`bevy_rapier3d` halves.
- **`blockloom-protocol`** - the wire format between them: newline-delimited
  JSON over the child's stdin/stdout. No sockets, no ports; the pipe closing is
  the whole shutdown handshake.
- **`blockloom-core`** - the engine library both processes share: `scene.rs`
  (the value types a world is built from - looks, bodies, placements, camera),
  `components.rs` (what an actor is made of - see below), `blocks.rs` (the
  block vocabulary and its `BlockKind` impl), `fields.rs` (what the frontend
  calls each value slot), `project.rs` (the saved document and the folder it
  lives in), `library.rs` (the project folders the Dashboard lists),
  `assets.rs` (the files inside one of those folders, which the asset tray
  manages and the runtime loads images and fonts from), `vm/` (the block VM),
  `script/` (compiling a project's Rust scripts, and the ABI they talk over),
  `sense.rs` (the world state reporter blocks read), and `wire.rs` (the one
  shape difference between documents and the frontend).
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

Three components have no fixed-field ancestor. `Camera` attaches the world
camera to that actor - follow, first person or third person - and one project
has one of them, so adding it takes it off whoever had it. `Script` names a
Rust file (see below). `Custom` is a named bag of values the project invented
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

### How a project runs

1. Play hands the runtime the whole project (`EditorMessage::Load`) and starts
   it. Nothing is shared but that message: the runtime owns the world from then
   on.
2. `vm::compile` flattens each actor's canvas into a `Vec<Step>` with jumps -
   a nested tree can't be suspended mid-body, but a program counter can. Header
   strands become entry points keyed by their trigger.
3. Simulation runs on Bevy's `FixedUpdate`: a constant-rate step (`FixedMain`
   catches up whatever the display does) that pulls the project's `world.fixed_rate`
   - set in Project Settings and applied by `pump_editor`/`dim2|dim3::sync_timestep` -
   so blocks and physics advance on the same tick whatever the frame rate. The
   per-frame `Update` publishes the sensor snapshot (`sense::publish`), turns input
   and rapier contacts into `vm::Event`s (at most once a frame, so a few sunk fixed
   steps never repeat a keypress), and reads `Vm::tick`'s results next step.
4. The runtime applies those effects to the ECS - shared ones in
   `world::apply_common`, physics and material ones in the dimension's own
   module - and reports says, errors and a periodic status back to the editor.
   Physics bodies carry `PhysicsPose`/`PrevPose`, and `record_poses`
   (`FixedPostUpdate`) + `interpolate_poses` (`Update`) draw them between fixed
   steps so fast displays don't see the steps.

Scripts yield the way Scratch's do: at a `wait`, and once per loop iteration.
That one rule is why `forever` costs one step per fixed tick instead of hanging
the process, and it's checked directly in `blockloom-core/tests/vm.rs`.

The VM holds `Rc`s, so it is a `!Send` Bevy resource - which is exactly right:
every system touching it is therefore scheduled on the main thread, the same
thread the thread-local sensor snapshot lives on. Keep it that way.

### Frontend (`ui/`)

Vue 3 + TypeScript + Vite, package-managed with pnpm. `src-tauri/tauri.conf.json`
points `frontendDist` at `ui/dist`. Key files: `store.ts` (the one copy of
backend state), `tauri.ts` (one function per command) over `bridge.ts` (the
Tauri/dev-bridge switch), `blockstitchSetup.ts` (the single wiring point into
blockstitch), and `blockFields.ts`, which is where a block's row comes from:
every block is described once as a list of pieces (a label, a value slot, a
dropdown, a nested body) and two factories turn that into the canvas component
and the sidebar-prefab component. **Adding a block means adding a variant to
`InstructionKind`, a row to `BLOCK_SPECS`, an icon and label in `icons.ts`, a
field id if it has value slots, and a `Step`/`Effect` if it does something
new** - not a pair of `.vue` files. The
two blocks whose row comes from a `BlockDef` rather than their type
(`BlockHeader`, `CallBlock`) are still hand-written, in `components/fields/`.

The asset tray along the bottom (`components/AssetTray.vue`) is a file manager
over the project folder: it lists, makes, imports, renames, moves and deletes
files, and an asset dragged out of it lands on any input wrapped in
`AssetDrop.vue` (the Look component's Image, the Script component's path).
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

The app opens on the Dashboard (`ui/src/components/Dashboard.vue`); the editor
appears once a project is open, and `state.project` being null is what decides
which of the two shows. Every edit is still written to disk right after it
lands.

### Known gaps

- No clones (`create clone of myself`), no sounds, no lists.
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
