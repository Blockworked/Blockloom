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
  (actors, looks, bodies, camera, 2D or 3D), `blocks.rs` (the block vocabulary
  and its `BlockKind` impl), `fields.rs` (what the frontend calls each value
  slot), `project.rs` (the saved document and its `.blockloom` files), `vm/`
  (the block VM), `sense.rs` (the world state reporter blocks read), and
  `wire.rs` (the one shape difference between documents and the frontend).
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

### How a project runs

1. Play hands the runtime the whole project (`EditorMessage::Load`) and starts
   it. Nothing is shared but that message: the runtime owns the world from then
   on.
2. `vm::compile` flattens each actor's canvas into a `Vec<Step>` with jumps -
   a nested tree can't be suspended mid-body, but a program counter can. Header
   strands become entry points keyed by their trigger.
3. Each frame the runtime publishes a sensor snapshot (`sense::publish`), turns
   input and rapier contacts into `vm::Event`s, and calls `Vm::tick`, which runs
   every live script until it yields and returns a list of `vm::Effect`s.
4. The runtime applies those effects to the ECS - shared ones in
   `world::apply_common`, physics and material ones in the dimension's own
   module - and reports says, errors and a periodic status back to the editor.

Scripts yield the way Scratch's do: at a `wait`, and once per loop iteration.
That one rule is why `forever` costs one step per frame instead of hanging the
process, and it's checked directly in `blockloom-core/tests/vm.rs`.

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
`InstructionKind`, a row to `BLOCK_SPECS`, a field id if it has value slots, and
a `Step`/`Effect` if it does something new** - not a pair of `.vue` files. The
two blocks whose row comes from a `BlockDef` rather than their type
(`BlockHeader`, `CallBlock`) are still hand-written, in `components/fields/`.

Projects are saved as `.blockloom` JSON in `<data dir>/blockloom/projects`
(override with `BLOCKLOOM_DATA_DIR`), one file per project, written after every
edit.

### Known gaps

- No clones (`create clone of myself`), no sounds, no lists.
- `say` shows in the runtime's corner overlay and the editor's log rather than
  as a per-actor speech bubble.
- A reporter-shaped custom block runs to completion in place, so a `wait` inside
  one passes straight through.
