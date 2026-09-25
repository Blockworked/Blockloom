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
just player             # stage the hard-optimized player a built game ships
```

Build the whole workspace, not just `-p blockloom`: off Linux (or with
`BLOCKLOOM_RUNTIME=process`) the editor starts the `blockloom-runtime` binary
sitting next to it, and a stale or missing runtime is exactly what `cargo run -p
blockloom` would leave you with. The editor says so in a banner rather than
letting Play do nothing.

There's no clippy.toml/rustfmt.toml - just `cargo clippy`/`cargo fmt` with
defaults.

## Style

- NEVER use em-dashes (-) in comments, code, docs, or commit messages. Use a normal hyphen (-) instead.
- Write comments like a human: short and terse, 1-2 lines. State what and why briefly. No multi-paragraph essays, no history lessons ("Previously..."), no dangling "see X docs" pointers unless X exists.

### Frontend (`blockloom-qt/qml/`, QML)

The QML is compiled into the binary by `blockloom-qt/build.rs` (cxx-qt's
`CxxQtBuilder` + qmlcachegen), so a plain `cargo build` picks up every edit.
It needs Qt 6 with Quick, QuickControls2, QuickDialogs2 and Multimedia; use
Qt 6's `qml`/`qmlls` (`/usr/lib/qt6/bin` on Arch), not Qt 5's. A new `.qml`
file must also be listed in `build.rs`'s `QmlModule`.

### Local `blockstitch` development

`blockstitch` is a separate sibling repo shared with any other app built on
the same block editor. Two halves of it are used here:

- `crates/blockstitch-core`, the Rust backend (value system, document model,
  editor operations), a git dependency in the root `Cargo.toml`;
- the QML block canvas and controls (`blockstitch-qml`, module
  `com.blockworked.Blockstitch`), a path dependency on `../blockstitch` in
  `blockloom-qt/Cargo.toml`, so that checkout has to exist beside this one.

To build against a local checkout of the Rust half too:

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
  manages and the runtime loads images and fonts from), `pack.rs` (that
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
device crash NVIDIA at exit. Cameras render into an offscreen texture of
`GAME_SIZE` (960x720) - the whole game, scaled to fit the view - and each
frame is GPU-copied into a ring of three linear Vulkan images exported as
dma-bufs (`embed.rs`, raw `ash` under wgpu). `FrameExchange` hands slots
between the world and the view: the world never draws into the one being
shown or waiting to be. The C++ `GameView` item imports the ring through EGL;
NVIDIA only samples linear buffers as external textures, so there it copies
each frame through a small shader into a plain texture first. This is why Qt
is forced onto its OpenGL renderer, and onto EGL on X11.

Status (positions, variables) and the run log reach QML through their own
`statusJson` and `logJson`, not the whole state snapshot - re-evaluating every
binding on each status or `say` stuttered the view. `RunLog.qml` appends by
`log.total` rather than rebuilding its list.

Input is `PreviewInput`, as it is for the MJPEG preview: keys, buttons,
position, text, plus `focus` (which, once sent, decides whether the game is
focused, and releases held keys when lost) and `mouse_delta`. A windowless
world answers `lock mouse` with `RuntimeMessage::PointerLock`; the view then
locks the pointer while it has the keyboard - Wayland pointer constraints and
relative pointer (the generated glue is vendored in `blockloom-qt/src/wayland/`),
cursor warping on X11 - and forwards raw motion. Escape always releases it and
a click takes it back.

`BLOCKLOOM_RUNTIME=process` forces the child process and MJPEG preview on
Linux too. Windows and macOS have no GPU sharing yet.

### How a project runs

1. Play hands the runtime the whole project (`EditorMessage::Load`) and starts
   it. Nothing is shared but that message - even in-process, the world gets its
   own copy - and the runtime owns the world from then on.
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
And everything `Vm::resolve` replaces - a variable, a parameter, a reporter
call - is hoisted into a `let` ahead of the expression, because the VM resolves
a whole tree before one operator runs: a reporter on the side `and` never reads
still runs, and still does whatever it does to the world.

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
