# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

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
just player             # stage the hard-optimized player a built game ships
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
if any command errors. **Don't edit the same project from the shell and
the window simultaneously** - each holds its own in-memory copy.

#### MCP server (`mcp/`)

AI agents can also reach this through the Model Context Protocol: `mcp/` is a
pnpm/Node package (`@blockworked/blockloom-mcp`, entry `src/index.ts`) that
spawns `blockloom-shell --no-state` and exposes every `--specs` command as an
MCP tool, with schemas derived from the same registry (see `src/registry.ts`,
which maps every `ArgSpec.ty` prose string to a zod schema - a new prose type
must be taught there). Tool calls are one shell line, responses are the
`{ok, result, error}` shape, plus `blockloom://state` and `blockloom://blocks`
resources. Each session is its own backend, so the window-and-shell warning
above applies to it too - one MCP server process and the editor must not hold
the same project. The shell resolves as `target/debug|release/blockloom-shell`
beside the repo, `BLOCKLOOM_MCP_SHELL`, or `--shell`, and is then staged to a
per-process scratch copy before it is spawned: a running executable locks its
own file on Windows, so driving the build tree's binary in place would fail
every `cargo build` for as long as the server lives. The copy is as fresh as
the last server start, so restart the server after rebuilding to pick up a new
binary. `just mcp` builds everything and prints the client config line for the
built `dist/index.js`.
Build/test with `cd mcp && pnpm install && pnpm run build && pnpm test`.

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
  mesh/`bevy_rapier3d` halves. The same binary is what a built game ships:
  with a pack beside it, it loads that instead of waiting for an editor, and
  `src/player.rs` is the whole of the difference (see Building a game below).
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
  manages and the runtime loads images and fonts from), `pack.rs` (that
  document again, as a built game carries it) with `build.rs` (what lays a
  build out), `vm/` (the block VM), `codegen/` (the same blocks as Rust
  instead),
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
   Every actor carries a `PhysicsPose`/`PrevPose` pair, and `record_poses`
   (`FixedPostUpdate`) + `interpolate_poses` (`Update`) draw each between fixed
   steps so fast displays don't see them - physics bodies from the pose physics
   wrote, and a `move`/`glide` sprite from the pose its step's effects pushed
   it to, both just as smooth as a rolling ball.

Scripts yield the way Scratch's do: at a `wait`, and once per loop iteration.
That one rule is why `forever` costs one step per fixed tick instead of hanging
the process, and it's checked directly in `blockloom-core/tests/vm.rs`.

The VM holds `Rc`s, so it is a `!Send` Bevy resource - which is exactly right:
every system touching it is therefore scheduled on the main thread, the same
thread the thread-local sensor snapshot lives on. Keep it that way.

### Building a game

Build is not Export. Export writes a `.blockloom` file for somebody else's
editor; Build makes a folder somebody can run without Blockloom at all:

```text
Pond Game/
  Pond Game.exe        the player: `blockloom-runtime`, renamed
  game/
    game.pack          the document, and the format version it was written at
    assets/...         the project's assets, minus the script sources
    .blockloom/build/  the script libraries, where the runtime already looks
```

`blockloom-core/src/build.rs` lays that out and `commands::build_game` drives
it. The player finds `game/game.pack` beside its own executable, so renaming
the binary is the whole of the branding, and `game/` is handed to the runtime
as the project folder - which is why the assets and the script libraries keep
the spelling they have inside a project. Nothing in the runtime knows whether
it is playing a folder or a build.

`player::Launch` is the one fork: a pack beside the binary means player mode,
which takes its dimension from the document rather than `--mode`, presses its
own green flag, never reads stdin, reports nothing (`bridge::attached` is
false, so the status corner and the editor handshake are skipped), and exits
when the world stops, since nothing can press Play again - `stop all` is how a
built game quits. `--play <folder>` runs a build without renaming anything.

A build ships the blocks as the document and runs them on the same VM the
editor plays with, so it needs no toolchain and costs a file copy. Scripts are
the exception: they ship as the libraries the editor already compiled, and one
that won't compile fails the build rather than shipping an actor that quietly
does nothing.

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
And everything `Vm::resolve` replaces - a variable, a parameter, a reporter
call - is hoisted into a `let` ahead of the expression, because the VM resolves
a whole tree before one operator runs: a reporter on the side `and` never reads
still runs, and still does whatever it does to the world.

What it won't compile is a custom block that can reach itself through statement
calls - its loops would share one set of counters where the VM gives every
invocation a frame. `Unsupported` refuses the whole project rather than
emitting half of one, so whatever calls it can fall back to the VM knowing what
sent it there. Nothing calls it yet: there is no scheduler on this side and no
home for variables both halves can reach, so a build still ships the document
and the runtime still plays it.

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
- A build carries no icon of its own and is a folder rather than an installer
  or one file; macOS gets that same folder rather than an `.app` bundle.
- Building for another platform needs its player staged by hand, and a scripted
  project also needs that target's `std` and a linker for it.
- A built game's blocks are interpreted, the same way the editor plays them.
  Nothing compiles a project down.
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
