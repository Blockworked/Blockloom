# Blocks, scripts and compiled logic

Declared block verbs, Rust and guest-language scripts, the sandbox, and blocks compiled to native code. Split out of `CLAUDE.md`; the index there lists every page.

## Block verbs

`blockloom-core/src/verbs.rs` declares a block once: a `VerbSpec` with a
snake_case `name` (saved in documents and called by scripts, so never
renamed), a `label` with `{arg}` where each slot goes, typed `VerbArg`s
(`Number`, `Text`, `Bool`, `Actor`, `Choice`, `Any`, each with a default),
the palette `category` and `section`, and a shape: a statement, or a
reporter with a `read` fn over the published snapshot. Every surface reads
that row. A statement is the one `InstructionKind::Verb { verb, args }`
(slot ids `PluginArg:i`), `Action::Verb`, `Act::Verb` (`ACT_VERB` 132 in the
logic ABI) and `ACT_VERB` 162 in the script ABI; all of them hand their
evaluated args to `verbs::effect`, which checks and coerces them (an
`Actor` becomes an id, a missing arg its default, a bad one an error) into
`Effect::Verb { actor, verb, args }`. A reporter is the `Verb` operator
(first arg the verb's name), answered by `verbs::read` for the VM, compiled
logic (through the generic sense read) and scripts (`READ_VERB` 76,
`TEXT_VERB` 36). Scripts also get a typed method per verb, generated into
`script/verbs_gen.rs`; the guest crate has `verbs::call/read_number/
read_text` and C has `BL_ACT_VERB`. The vocabulary (`blocks` in the shell,
`blockloom://blocks` in MCP) lists them under `verbs`, and the palette draws
them from the generated `builtinVerbs` table in `Blocks.qml` with the plugin
block row builders (`tst_Verbs.qml`). The runtime acts on a statement by
name: `crate::verbs::calls(&effects, "name")` in a system of the module that
owns it. So a new block is one row, one runtime system, and `BLESS=1 cargo
test -p blockloom-core --test verbs`; no ABI constant. `logic::tests::
declared_verbs_match_the_vm_through_the_player_boundary` holds the VM and
compiled halves together.

The first verbs are generic component access: `set_property` and
`property` take a registry path (`Light.intensity`, the wrapper key of a
one-spec component may be left out, `registry::resolve_component_path`).
A write lands in the run's copy of the actor (`Engine::set_live`, looked up
before the document by `Engine::actor`, cleared on every rebuild) and
re-attaches components built only when attached (Look, Body, Sprite,
Camera, Render); the rest read their spec each frame. Reads go through
`ActorSense::spec`, an `Arc` of the actor's components shared from
`Engine::spec_of`, whose `ActorSpec` serializes each component once, so a
read is about 0.5 us against 25 ns for `my x position` (release,
`verbs::cost`); keep typed verbs for hot reads. `screen_open` answers `is screen open?` from
`Sensors::screens`. Place, Script and Parent refuse a property write.

## Scripts

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
that presses Play** - a packaged install carries one in `tools/rust` beside the
editor (`script::bundled_rustc`, or `BLOCKLOOM_RUSTC`), which wins over PATH; a
linker is still the machine's own. The boundary is three calls, not one per verb, so adding
something a script can do is a new constant in `abi.rs` rather than a new
field in `HostApi` - which would break every script already built.

A script reads the world through the same frame snapshot the reporter blocks
read (`sense`) and everything it does comes back as a `vm::Effect`, applied by
the same systems. So a script and a canvas can drive one actor between them,
and reading straight back after a write gives the old value, exactly as it
does in the block editor.

A script has three entry points, any of which `export!` fills in empty:
`start`, `tick` and `event`. `event` hears what the hat blocks start on:
`Engine::fire` queues each event while scripts run (`script_events`), and
`step_scripts` hands them over before `tick` as a `ScriptEvent`, a kind
(`abi::EVENT_*`), four numbers, and words read back through `TEXT_EVENT`,
since a web host can't hand a string into the script's memory. Messages,
keys, actions, touches and interface events reach every script; clicks,
collisions, particles and animation events only the actor they name.

A panic inside a script is caught by `export!` and
logged rather than being allowed to cross the C boundary, which would abort
the whole game window.

## Script sandbox and guest languages

Besides the native `cdylib`, a script can run as a core wasm module under
wasmi (`blockloom-runtime/src/script_wasm.rs`, the shape web builds already
ship): only the three `blockloom.read_number/read_text/act` imports (WASI and
anything else is refused at load), a 64 MiB ceiling, fuel per call, and a trap
or empty tank stops that script with an error effect, not the run. Play builds
the wasm beside the native library when the machine has the target's std;
`ScriptBackend` (`engine.rs`) loads native first and falls back to wasm
(`BLOCKLOOM_SCRIPT_BACKEND=wasm` flips it); wasm32 and Android only have the
one browser/native variant. The frozen `blockloom:script@0.2.0` WIT world is
`blockloom-core/src/script/wit.rs`, and drift tests refuse an ABI verb with no
mapping (a `wit-parser` test also resolves the text). Guests live in
`blockloom-script-guest`: the Rust crate (`src/`, `templates/minimal.rs`), a
freestanding C binding (`c/blockloom.h`, `templates/minimal.c`, built with
`clang --target=wasm32 -nostdlib`; its verb numbers are held to `abi.rs` by
`c_header_covers_every_abi_verb`) and a Python component
(`templates/minimal.py`, componentize-py), a JavaScript component
(`templates/minimal.mjs`, jco; TypeScript compiles to it first) and a Go one
(`templates/minimal.go`, TinyGo wasm-unknown + wasm-tools), a C# one (`csharp/`, Mono on wasi-wasm,
`templates/minimal.cs`) and a Kotlin one (`kotlin/`, Kotlin/Wasm WASI with a hand-written
canonical-ABI binding, `templates/minimal.kt`; the host engine enables WasmGC and exceptions for
it). Each has a `build.sh`. Runtime tests build the Rust and C
templates and hold their effects to the WAT fixture's.

Component-model guests (Python, TypeScript, Go, ...) emit wasm components,
which wasmi cannot run, so desktop builds host them under wasmtime
(`script_component.rs`, cargo feature `components`, default on, absent on
wasm32 and Android). The guest imports the typed interfaces and exports
`entry`; each import is a thin wrapper over the same `number_for`/`text_for`/
`act_for_asked` the other backends use. Sandbox: 256 MiB, fuel per call, empty
WASI context. A script path ending `.wasm` is prebuilt: Play does not run rustc
on it and `ScriptBackend` loads a component under wasmtime or a core module
under wasmi. Compiled components are cached per process and under
`.blockloom/build/components` (first load of CPython or QuickJS is seconds),
and `wasi:http` is defined as traps. wasmtime is why web and Android guests stay Rust and C only.

A script's language is its extension (`blockloom-core/src/script/lang.rs`):
`.rs`, `.c`, `.py`, `.ts`, `.mjs`, `.go`, `.cs`, `.kt`, plus `.wasm` for one
already built. Only Rust goes through rustc. Every other language is a guest
build: Play partitions the actors' scripts, hands the guests to
`lang::build_guest`, and that runs the language's own toolchain through the
guest kit's `build.sh` (C is one direct `clang --target=wasm32`) into
`.blockloom/build/guest/<name>.wasm`. A stamp beside the module (ABI, language,
source hash) reuses the last build. `Language::available` checks each
toolchain's programs on PATH and says how to get a missing one
(`install_hint`), reported once per language in the run log rather than once per
script, and Play carries on: blocks still run. `ScriptBackend::is_built`/`load`
look for a guest script's module at `lang::guest_wasm_path`, opening a component
under wasmtime and a core module under wasmi, so nothing about running one
differs from a prebuilt `.wasm`. `create-script` takes a `language` (its name or
its extension, Rust by default) and copies that language's
`blockloom-script-guest/templates/minimal.<ext>`; the kit is
`BLOCKLOOM_SCRIPT_GUEST`, else `script-guest/` beside the editor, else the
repo's own folder. A runtime test builds `templates/minimal.c` through
`build_guest` and opens it through `ScriptBackend`, holding its effects to the
WAT fixture's.

## Compiling the blocks

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

A block's Branch inputs work the same way in both halves. A
`BranchCallBlock` compiles to a `Call` carrying where each branch body
starts, a jump past them, and the bodies inline in the caller's region, each
ending in `BranchEnd`. `RunBranch` (its index settled per block at compile
time) runs the body of the call in scope as a frame of its own, which reads
the scope below that call: the caller's inputs, the caller's branches, and a
`return` in it leaves the block the body was written in. `escape loop` stops
at a branch body's edge, as at a call's. A reporter call keeps its bodies as
canvas JSON in `Value::Call::branches`; `vm::compile` places them in front of
the instruction holding the value (the value then keeps `{"~start": pc}` per
body) and lifting turns the call into an `Invoke` with those starts, so the
bodies run on the strand's own frames exactly like a statement call's.

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
tick, and the host is left with the entity. Each scene gets its own names
and entries tables (`NAMES_n`/`ENTRIES_n`) under one `SCENES` index, because
an empty canvas still answers to its name. The runner runs the active
table and `load_scene` swaps it the way `Vm::load_scene` does, so a
compiled game switches scenes without falling back to the VM. A clone or a
deletion from outside the program - a script's - comes in
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
