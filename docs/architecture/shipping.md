# Building and shipping

Desktop builds, the web player and Android devices. Split out of `CLAUDE.md`; the index there lists every page.

## Building a game

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
with is a player. Host exports use the newer of the staged player and that
runtime, so a stale staged copy cannot undo a recent editor/runtime fix. Scripts are native too, so a project with one can only be
built for a platform this machine's rustc can compile for; `script::compile_for`
cross-builds them into `.blockloom/build/<triple>/`, leaving Play's own build
where the runtime has always looked for it. `build::targets` answers both
questions at once, which is what the Build dialog lists, with the reason
attached to every platform it can't offer. A guest-language script is one wasm
module whatever the platform, so a build copies it into
`.blockloom/build/guest/` rather than cross-building anything; a `.wasm` script
ships inside `assets/`, where the runtime loads it from. Android runs native
libraries only, so a guest script stops an Android build with that said plainly.

A build folder is named for the project and the platform - `Pond Game (Linux
x64)` - because one output folder holds a build per platform, and three folders
called the same thing would be three chances to ship the wrong one.

## The web player

The Web build target (`wasm32-unknown-unknown`) makes one `.html` holding
everything: `build::build_web` lays the game folder out as a native build
does, then `web_build::page` gzips the staged player (the wasm and its wasm-bindgen glue
under `players/wasm32-unknown-unknown/`, which `just web-player` stages), the
pack, every game file and each script's wasm module
into one archive, base64'd into the page. The page unpacks it with
`DecompressionStream`, imports the glue from a blob URL, compiles the script
modules and, on a click (audio needs the gesture), calls `web::start_game`.
So it opens from disk as well as any static host. Always SDR, always the VM.

The player renders through WebGPU (Bevy's `webgpu` feature, set for wasm in
the runtime's `Cargo.toml`; WebGL2 has no compute for the sky, fog, cloud and
luminance passes). `start_game` mounts the files in `blockloom_core::vfs`,
which every direct read a web run can reach goes through (`vfs::read` rather
than `std::fs::read`), and `web::GameFiles` serves the same table to Bevy's
asset server; the game folder is the empty path. Saves go to localStorage,
pointer lock is asked for on a click on the canvas (`web::want_pointer_lock`),
and the canvas follows its parent's size. `web::game_actors` reports actor
positions, which is what `just web-smoke` checks, since headless Chromium
can't screenshot a WebGPU canvas.

A script on the web is its own wasm module, not part of the player: a module
can't call another through a function pointer, so there the script imports
the three host calls from `abi::WASM_MODULE`, each taking a pointer to an
`abi::WasmCall` in its own memory, and the entry points get a null `HostApi`
(the prelude's `web` module swaps in its trampolines). `script.rs`'s
`browser` module instantiates each module once per file and answers the
imports with the same `number_for`/`text_for`/`act_for` the native
`extern "C"` calls use. A panic aborts on wasm, so the prelude's panic hook
logs the message first; the trap then stops that script for the rest of the
game, and its error names the panic.

Chrome's WGSL compiler holds shaders to the spec where naga doesn't, so
`pbr_patch.rs` also rewrites the `let texture = pbr_bindings::...` lines in
Bevy's material shaders on wasm (`patch_handle_lets`).

## Android devices and the emulator

The Devices tab (`DevicesPanel.qml`, `DevicePanel.qml`) lists what adb sees,
boots emulators and shows a device's screen. `blockloom-core/src/android.rs`
holds the adb/AVD plumbing and `android_tuning.rs` what makes a boot fast.

An embedded emulator boot is `-no-window`, and `-gpu auto` falls back to
software drawing once the window is hidden, so `android::start_emulator`
always passes `-gpu host` (`BLOCKLOOM_EMULATOR_GPU` picks another mode from
`tuning::gpu_mode`'s list), plus `-cores`/`-memory` raised (never lowered)
from the AVD's own `config.ini`, `-no-metrics`, and `-grpc <port>`. Its output
goes to `emulator-<avd>.log` in the data dir and the child is tracked, so a
boot that dies (no hypervisor, a bad GPU mode) shows up as
`EmulatorState.failure` on the next status read instead of a five minute
wait. `EmulatorStatus.acceleration` is `emulator -accel-check`'s answer
(`/dev/kvm` hints on Linux), asked once when good and every 30 s when not.
The x86_64 image can't be accelerated on an arm64 host; that is reported, not
worked around.

The screen is a push stream, not polled commands. `blockloom-app/src/screen/`
is one `Session` per watched device, owned by `AppBridge` (`watchScreen`,
`screenInput`, `screenJson`) rather than `Backend::dispatch`, so a slow command
on the backend's single worker never delays a tap. A session picks its
transport on its own thread: the emulator's gRPC service (`grpc.rs`:
`streamScreenshot` scaled by the emulator, `sendTouch` for real pointers;
messages are declared by hand with the field numbers of
`emulator_controller.proto`) when the editor started that emulator and wrote
its port (`android::emulator_grpc_port`), else one long-lived `screencap -p`
loop over a single adb pipe (`stream.rs`, PNGs framed by their `IEND`, the
newest kept and older dropped). Frames leave as JPEG data URLs
(`frames.rs`). Touch reaches the device through `Routes`: gRPC once its stream
is up, else one persistent `adb shell` (`shell.rs`) typing `input motionevent
DOWN/MOVE/UP` (or tap/swipe where the device lacks it), which also takes keys
and counts finished commands so moves coalesce instead of queueing. The
`android_mirror_*` commands stay for the shell and MCP, one grab per call.
