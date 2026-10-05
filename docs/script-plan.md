# Rust script experience plan

Status: Phase 0 started. Analysis of the current script system against Unity's coded scripts, with gaps and a phased improvement plan.

Goal: make Rust scripts a first-class alternative to blocks for game logic. A script author should get per-instance state, inspector-tunable params, typed APIs over any actor, sane timing primitives, and editor feedback (check, format, docs, debug) without installing a toolchain or leaving Blockloom. Scripts stay another way to drive an actor, not another engine: reads from the same `sense` snapshot, writes as the same `vm::Effect`s.

## Baseline before this plan

- One `.rs` file per actor, named by `Script { path }` (`blockloom-core/src/components.rs:354`) under `assets/scripts/`. `create_script` scaffolds from `script::starter` and attaches the component (`blockloom-app/src/commands.rs:3889`).
- Build is two `rustc` runs, no Cargo/network: the `blockloom` rlib assembled from `script/abi.rs` + `script/prelude.rs`, then the script as a `cdylib` (`blockloom-core/src/script/mod.rs:274`). `std` only, edition 2024, `-O2`. Cache key is ABI + toolchain + target + source length (`mod.rs:260`); host builds reuse Play's output dir, cross builds get a triple-named dir (`mod.rs:107`).
- Boundary is `script/abi.rs:12` (`ABI_VERSION = 40`). Three host calls in `HostApi` (`abi.rs:486`): `read_number`, `read_text`, `act`. New verbs are new constants so old libs keep loading until the version check rejects them (`blockloom-runtime/src/script.rs:110`).
- Runtime opens libs with `libloading`, checks the ABI, and calls `start` once per actor (late clones included), then `tick(dt)` + `event(kind, n0..n3)` each fixed step (`blockloom-runtime/src/world.rs:1081`, `simulation.rs:130`). `step_scripts` is skipped while paused (`world.rs:1087`). Effects join the VM's list; read-after-write returns the old value, same as blocks.
- Web builds compile each script to its own `.wasm` importing `blockloom::{read_number,read_text,act}` with a null `HostApi` (`abi.rs:567`); Android resolves the lib by file name beside the runtime (`script.rs:61`).
- Editor is `ScriptDialog.qml`: a plain `TextEdit` with regex highlight, gutter, on-demand `read/write/check_script` and `script_diagnostics` via `cargo check --tests` or `rustc --error-format=json`. `script/ide.rs:178` synthesizes a root `Cargo.toml` + `blockloom` crate under `.blockloom/ide` so rust-analyzer works in an external editor. `check`/`build_scripts` log raw `rustc` stderr to the run log; a failed script silently does not load and Play carries on (`commands.rs:2608`).

## Unity comparison

| Area | Unity (`MonoBehaviour`) | Blockloom baseline |
| --- | --- | --- |
| Lifecycle | `Awake/OnEnable/Start/Update/FixedUpdate/LateUpdate/OnDisable/OnDestroy`, coroutines (`yield WaitForSeconds`), `Invoke` | `start/tick/event` via `export!` (`prelude.rs:3130`). `tick` is fixed-step only, no frame/late split, no destroy/disable/stop hook, no wait primitive. |
| State | Per-instance fields, `[SerializeField]` Inspector UI, per-prefab overrides | `Actor { ctx, api }` holds no user state (`prelude.rs:128`). No fields, no Inspector rows. One file shared by N actors shares `static`s. Only persistence is `save_variable` with no script-side read/write of block variables. |
| Scope | Multiple components per object, `GetComponent`, `GameObject.Find`, act on any object | One script per actor; almost every `act_for` pins `actor = ctx.actor` (`script.rs:1001`). Scripts move/tint only themselves; cross-actor ops are limited to clone/delete/parent/sound-at plus single-axis `position_of`. |
| API shape | Typed (`transform.position`, `Rigidbody.velocity`, enums), compiler-checked | Stringly typed: key names, dial strings (`"direction"`, `"set radius"`), `"#RRGGBB"` colors, component/field/action names. Typos answer `MISSING`/zero at runtime. |
| Tooling | Roslyn shipped with the editor, IntelliSense, debugger, incremental compile on save | Requires the user's own `rustc`; no in-app completion/hover/signature, no formatter, no breakpoints/watches, diagnostics only on open/`Check`. Panics log `"panicked in {what}"` with no location (`prelude.rs:2868`). |
| Iteration | Compile on save, domain reload, hot state | Compile on Play/`Check`. No attach mid-run, no hot reload. ABI bumps invalidate every build until `.blockloom/build` rebuilds. |

## Background: Pumpkin-MC WASM plugins (research)

Pumpkin-MC moved plugins from native libs to WASM components for sandboxing, cross-platform shipping, crash isolation, marketplace validation, and multi-language support (issue #662; Extism tried and dropped as too heavy, wasmtime preferred).

- Unit is a `wasm32-wasip2` component, not a native lib. The interface is defined in WIT, not in a language API: `pumpkin-plugin-api` calls `wit_bindgen::generate!` with `world: "plugin"`, host exposes typed interfaces (`command`, `server`, `permission`, ...), guest exports `init-plugin`, `on_load`/`on_unload`, `handleCommand`/`handleEvent`, `get_metadata`.
- Each language has a thin guest binding over the same WIT world (`pumpkin-api-go`, `-py`, `-cs`, `-kt`, `-c`, `-d`, `-zig`, `-ts`, listed at `docs.pumpkinmc.org/plugin-dev/introduction`), each with its own componentize toolchain: Rust `cdylib` + build tool, Go via `TinyGo` (`tinygo build -target=wasi`; standard Go does not emit the right component), C# via .NET 10 `wasi-wasm` RID + `Componentize.DotNet` SDK, Kotlin via Gradle `wasmWasi`, C via `wasi-sdk clang -mexec-model=reactor`, TS/Python via bundler + componentize script.
- Host runs them in a WASM runtime with a permission + WASI-sandbox model: `PluginMetadata.permissions` gates `fs.read.data`/`fs.write.data`, network DNS, sys-info fields; storage is plain files via the language's normal `std::fs` under a per-plugin data folder from `Context.get_data_folder`. Lifecycle callbacks are re-entrant: docs warn not to hold a non-reentrant lock across a host call.
- Native-vs-WASM stays an explicit tradeoff in their thread: sandboxing and language reach vs native-only needs (sockets, codecs, GPU libs, AVX/OpenCL, HW video) and bigger debug/perf costs. Plan kept both paths open.

Blockloom already has half of this: portable plugin modules run WASM under `wasmi` with two imports (`blockloom-plugin-host/src/portable.rs:1`, `blockloom-plugin-api/src/wasm.rs`), and web scripts are already per-script `.wasm` modules (`mod.rs:145`). Desktop scripts (`rustc cdylib` + `libloading` + 3-function C ABI) are the outlier this phase would replace or parallel.

## Gaps

1. No per-instance state or inspector params. Forces `static`s (shared across actors using the same file), custom-component hacks, or side files.
2. Self-only writes and no variable bridge. Scripts cannot drive other actors' transforms/physics/appearance or read/write block variables and lists, so mixed block/script logic has no shared working memory.
3. Poor time model. No `wait`/timer/interval helpers, no tween-end or animation-end subscription from `tick`, no fixed-vs-frame split, no pause-aware UI tick, no stop/destroy cleanup.
4. Thin physics reads. Scripts can push/set velocity/add force/query/raycast but cannot read velocity, angular velocity, mass, or contact detail outside events; `position_of` is one axis at a time.
5. Editor feedback. Tokenizer highlighting only; no continuous check, format, clippy, API docs, generated project symbols (actor/component/field names), or debugger/profiler attribution.
6. Iteration and platform cost. Per-file `-O2` `cdylib` builds on Play; cross builds need target `std` plus a linker (Android NDK wrapper, wasm `rust-lld`); failures arrive as raw `rustc` stderr in the run log.

## Plan

### Phase 0: small correctness and debuggability wins

- [x] Log panic location and message from `guard`, keeping the catch-and-log contract so a script still cannot take the window down. Native guards capture the actual panic site through a hook, with `Location::caller` as a fallback; web's abort hook already reports the panic site. Tests cover exported callbacks, non-string payloads, nested guards, concurrent threads and continuing after a panic.
- [x] Invalidate script caches when the prelude changes, and fingerprint source contents so same-length edits cannot reuse stale builds.
- [x] Keep per-script build and load status in editor state. Show missing builds, compiler errors, ABI mismatches and loader errors in `ScriptDialog` and the Inspector card, alongside the run log. Saving clears the previous result; a successful check or load replaces failures.
- [x] Add `pose()` and `pose_of()` batch reads (position + rotation + scale in one call). Fixed 28-byte records use the existing buffer callback on native and web; self reads avoid allocations and cloning the actor snapshot. Document call costs and snapshot timing in the [script read guide](script-api.md). Script ABI is now 41.
- Add missing readback parity the blocks already have: velocity/angular velocity/mass reads, multi-axis `position_of`, grounded flag without requiring a controller move.

### Phase 1: state and scope (the Unity gap)

- Host-owned per-actor script storage (`get_data/set_data` or typed `state::<T>`), keyed by actor id, cleared on rebuild. Unblocks timers, cooldowns, and multi-tick behavior without `static`s.
- `World`/`ActorRef` handle for cross-actor reads and writes (`world.actor("Ball").go_to(..)`), lowering to the existing `Effect`s with an explicit target id. Start with transform/appearance/velocity; keep the deferred-effect semantics (read-after-write still reads old).
- Script access to block variables and lists (read + write + save), so a canvas counter and a script counter can be the same counter.
- Allow shared script modules: a documented `assets/scripts/shared/` tree compiled as additional `--extern`/crate sources, plus support for more than one script component per actor with a defined call order.

### Phase 2: timing and lifecycle

- Split `tick` into fixed and frame variants (physics vs camera/UI motion), and add a UI tick that runs while paused so script-driven menus match block-driven ones.
- Add timer helpers in the prelude (`after`, `every`, cooldowns) built on `dt` accumulation, plus `on_stop`/`on_destroy` and scene-end cleanup.
- Expose tween/animation completion and `wait`-equivalent sequencing without blocking: pollable handles or events, not threads.

### Phase 3: typed API and generated symbols

- Replace dial strings with enums (`WindDial`, `WaterDial`, `ForceMode`, color type), following the existing `Axis`/`CameraView` pattern.
- Generate project symbols for scripts (actor names/ids, custom component fields, input actions) as an optional checked module, so renames break compile instead of silently reading zero.
- Publish a blocks-to-scripts Rosetta page: each block with its `Actor::` equivalent, and keep the `starter` template (`mod.rs:41`) as the canonical example.

### Phase 4: editor experience

- Background `cargo check` on `write_script` with continuous inline diagnostics, plus Save+Format (`rustfmt`) and an opt-in Clippy pass.
- In-app completion/hover from the existing `.blockloom/ide` project (or an LSP client), project-symbol completion, and one-click open in the user's editor with the current flow kept.
- Script-aware run log: log levels, per-script timing rows in the profiler, and failure states that link back to the file/line.

### Phase 5: multi-language scripts via WASM components (Pumpkin-style)

Do scripts second, not first: prove the WIT world in the plugin host, measure per-tick overhead against the `cdylib` path, then decide whether desktop scripts migrate or run side by side (native for trusted local use, WASM for sandbox/marketplace, same split Pumpkin kept open).

- Freeze a script WIT world equivalent to the current `READ_*/TEXT_*/ACT_*` + `start/tick/event` surface. Prefer typed resources/records (`resource actor`, `record hit`, `variant event`, per-domain interfaces) over the current numeric verbs-over-3-calls shape. Keep deferred-effect semantics (snapshot reads, later-applied writes, read-after-write reads old).
- Host components in `wasmtime` (component model + WASI P2) alongside the current `libloading` path, with per-tick fuel/deadline like `portable.rs:40` already does for plugins. Gate WASI imports per script (no file/net/threads in the tick path by default); expose wall-clock, RNG, and async tasks only as host-provided deterministic services, following `blockloom-plugin-host/src/services.rs`.
- Ship one guest binding per language via `wit-bindgen`, starting with Rust plus one GC language (Python or TS via componentize) before promising Go/Kotlin/C#. Each binding is a small repo to own, same cost Pumpkin pays with its `pumpkin-api-*` set. Document each language's componentize toolchain (TinyGo, .NET WASI workload, Kotlin/Wasm, wasi-sdk) the way Pumpkin's per-language quick-starts do.
- Fix the Blockloom-specific costs Pumpkin does not have before committing GC languages to per-tick scripts: batch hot reads (`pose()`, event structs) to amortize canonical-lift overhead, keep the fixed-step determinism story (same tick, same answers), and define the per-language cross-build path (target `std`, linker, strip) next to the existing Rust one in `mod.rs:288`.
- Gate: one non-Rust language driving `start/tick/event` in a test world with fuel limits, at measured overhead vs native. No GC-language promise for per-tick scripts until that number exists; component sizes (bundled interpreters) and per-call lift costs are the known risk from Pumpkin's own thread (#110).

## Open questions

- Ship a toolchain or remove the dependency: bundle `rustc`, offer an optional interpreted scripting tier for edit-time iteration, or keep requiring the user's toolchain and improve the missing-toolchain guidance. Multi-language WASM multiplies this: one toolchain per guest language (TinyGo, .NET WASI, Kotlin/Wasm, wasi-sdk). Packaged installs cannot assume any of them (known gap).
- Hot reload scope: recompile + reload a single script in the scene view without a full rebuild, and define what happens to per-actor storage across reloads.
- Debugging story: headless test harness for scripts (VM-equivalence style, as in `tests/codegen.rs`) vs full debugger attach; pick one before promising breakpoints.
- Web/Android parity: keep the three-call ABI as the portable surface until the WIT world replaces it; any new verb needs the wasm import path and the Android packaging path at the same time. A script WIT world must ship guest bindings and host support on all three targets at once.
- `wasmi` vs `wasmtime`: portable plugin modules use `wasmi` today; script components need the component model + WASI P2, which points at `wasmtime`. Decide whether both runtimes coexist or the host converges on one.
