# Blockloom

Build games out of blocks, in 2D or in 3D.

Blockloom is a desktop game engine with a Scratch-style block editor. A project
is a world and a cast of actors; each actor has its own canvas of blocks, and
pressing Play opens the world in a real renderer with real physics and runs
them.

- **One vocabulary, two dimensions.** The same `go to`, `push` and `turn` blocks
  drive a sprite in a 2D world or a mesh in a 3D one. A project says which it
  is, and a `z` coordinate is simply ignored in 2D.
- **Real physics.** Every actor can be static, dynamic or kinematic, with
  gravity, bounce and friction, via Rapier.
- **Blocks that sense the world.** `key down?`, `touching?`, `distance to`,
  `my x position`, `timer`, the mouse - all readable inside any value slot.
- **Your own blocks.** Custom commands and reporters with named inputs, as in
  Scratch's "My Blocks".

## Building

### Prerequisites

- Rust installed through `rustup`. The repository pins Rust 1.98.1 in
  `rust-toolchain.toml`; rustup selects and installs it when needed.
- `just`, Git, a C/C++ compiler and the platform's native linker.
  On Windows, use the MSVC Rust toolchain and Visual Studio C++ build tools;
  on macOS, install the Xcode command-line tools.
- Qt **6.12 or newer**, including development headers and build tools for
  Core, Gui, Qml, Quick, Quick Controls 2, Quick Dialogs 2 and Multimedia.
  QtGui private headers are needed for the Linux Wayland HDR integration.
  Use a Qt kit matching your compiler and architecture. Set `QMAKE` to its
  `qmake` executable if it is not discoverable as `qmake6` or `qmake` on PATH.
- Bash, curl, tar, patch, and `sha256sum` (or `shasum`) for preparing patched
  dependencies. On Windows, put the Git Bash tools on PATH.
- On Linux: `pkg-config` and development libraries for ALSA, udev, Wayland,
  xkbcommon and EGL, plus a working Vulkan loader and GPU driver. Python 3
  is needed for automatic build-cache cleanup.
- On Windows: a working Vulkan GPU driver. Dependency preparation downloads
  pinned Vulkan headers; a full Vulkan SDK is optional.

The first build needs network access for Rust dependencies, the pinned
`blockstitch` Git dependency, and patched dependency archives. A sibling
`blockstitch` checkout is not required.

### Blockloom Hub

`just hub-run` builds the workspace and opens the separate Hub window. It lists
projects, saves each project's chosen editor, imports prepared release bundles
and builds local repositories into its per-user installations directory. Release
versions remain separate and projects change versions only through an explicit
selection. Rust is required in each release bundle; Java, Android SDK, NDK and
Android Rust targets have optional import checkboxes.

The development Hub requires Python 3.11+ on PATH, or
`BLOCKLOOM_HUB_PYTHON` set to the Python executable. Get a release accepts an
HTTPS catalog or the GitHub CLI setting for private `Blockworked/Blockloom`
releases using your `gh auth login` session. Shipping Python runtime packaging
is still pending. Local builds open an
installation options dialog before replacing their current editor. The Hub can
cancel builds, keeps build logs, and blocks rebuilding installations or changing
projects that are in use. See
[the Hub plan](docs/hub-plan.md) for current behavior and remaining work.
Changing a project's editor offers a ZIP backup, enabled by default. Backups
preserve the old editor selection and project files under the Hub's `backups`
folder; Show backup opens that folder after the change.

Build artifacts runs the editor and players concurrently on Windows, Linux and
macOS. Release runs for `X.Y.Z` tags matching the workspace version, or a manual
existing-tag dispatch. It builds native and web players with the `dist` profile,
then publishes editor/Hub archives and `blockloom-catalog.json`. Editor archives
include Qt, the pinned Rust compiler, web support and optional Android Rust targets.
The Hub archive currently requires Python 3.11+ installed separately.
macOS packages target macOS 27 using GitHub's `xcode-27` runner.
Windows packages use Visual Studio 2026 on `windows-2025-vs2026`.

### Editor And Runtime

Run from the repository root:

```bash
just build                  # release build of the entire workspace
./target/release/blockloom   # Windows: target\release\blockloom.exe
just run                    # build, then launch the editor
```

Always build the **whole workspace**, not just `-p blockloom`. Linux and
Windows normally run the world in-process. macOS and `BLOCKLOOM_RUNTIME=process`
launch the `blockloom-runtime` executable beside the editor.

On Windows the Game view defaults to Vulkan and shares three GPU images with
Qt through Win32 memory handles. Cameras render directly into those images,
without CPU readback or a frame copy. Ownership barriers and queue completion
waits protect frames while Qt samples them, including during resize and shutdown.
If sharing is unsupported or Qt uses a different GPU, the view falls back to
readback. `QSG_RHI_BACKEND` can select another Qt renderer, which also uses
readback. The shared ring currently displays SDR; it does not provide the Linux
Wayland HDR plane.

`just` Cargo recipes automatically generate patched Bevy/wgpu sources in
gitignored `.patched-deps/` from the small in-repo patches. Before running
Cargo directly on a fresh checkout, prepare them explicitly:

```bash
just prepare-patched-deps
cargo build --workspace
./target/debug/blockloom
cargo fmt --all --check
just test
just qml-test               # Qt Quick interaction/layout tests, after building
```

See [patches/README.md](patches/README.md) for patch preparation and maintenance.
QML is compiled into the editor for normal builds. For live QML iteration,
use the preview workflow below.
For QML tests, put Qt 6's `qmltestrunner` on PATH or set `QMLTESTRUNNER`
to its executable.
The legacy `ui/` and `src-tauri/` applications are not part of this build;
Node.js is not needed to build the Qt editor.

### Build Cache Budget

On Linux and Windows, `just build`, `just player`, `just web-player`,
`just test`, `just replace` and `just qml-preview` clean build caches around
compilation.
The default budget is **20 GiB** for the whole `target` tree, including the
web build. Set `BLOCKLOOM_TARGET_LIMIT_GIB` to change it.

Cleanup removes the oldest incremental crate caches first. If that is not
enough, it removes older profile caches (`deps`, `build`, `.fingerprint` and
`examples`). Final binaries and staged players are preserved. Evicted caches
are recreated by the next build, which may take longer.

```bash
just prune-target --dry-run          # inspect usage and proposed cleanup
just prune-target                   # clean without building
BLOCKLOOM_TARGET_LIMIT_GIB=30 just replace
```

This is a cleanup budget, not a filesystem quota: builds may temporarily exceed
it. Locked profiles are skipped and retried on the next cleanup. Protected
outputs can also keep usage above the budget; the cleaner reports this.
QML preview keeps its current profile's resource manifests. Direct Cargo
commands need a separate `just prune-target` afterward. `CARGO_TARGET_DIR` and
`CARGO_BUILD_BUILD_DIR` are respected; an external build directory counts
toward the same budget.

### QML Hot Reload

With Python 3 and Qt 6.12's QML tooling installed, run:

```bash
just qml-preview                       # build the workspace, then watch QML edits
just qml-preview --profile dev         # use the debug build instead
just qml-preview --output session.qtd  # record input for a later session
just qml-preview --replay session.qtd  # restore a recorded session
just qml-preview --build-only          # build and print the preview command
```

The launcher uses `QMAKE` (or `qmake6`/`qmake` on PATH) to select Qt and runs
that kit's `qmlpreview`, avoiding older tools on PATH. It passes the active
resource manifests for Blockloom and Blockstitch so embedded `qrc:` URLs map
to source files. Local Blockstitch QML edits work after `just blockstitch-local`.
Pass application arguments after `--`.

Save an existing QML file to update the running editor. Colors, dimensions,
bindings and function bodies can update while preserving UI state. Structural
changes can recreate objects; use `restart` at the preview prompt if needed.
Use `help` for commands and `quit` to end the session. Set
`QMLPREVIEW_HOTRELOAD=0` to compare with full scene reloads.

The opt-in `blockloom/qml-preview` Cargo feature enables QML debugging.
Only use it for development. Run `just build` again before installing or
distributing the editor. Rust/C++ edits and new QML module files still need a
rebuild. Verify final changes with a normal build and fresh launch.

### Game Players

`just player` builds the optimized native player and stages it under
`target/release/players/<host-target>/` for exporting games from the editor.
Build and stage other native platforms' players on those platforms.
`just replace` reuses the optimized release runtime built with the editor as
the native player, avoiding a separate compilation and fat LTO. Use
`BLOCKLOOM_NATIVE_PROFILE=dist just replace` for the full shipping optimization,
or stage it separately with `just player`. The status row shows Native waiting
for the editor build, then staging the runtime.
The optional DLSS build is `just player-dlss`; it requires the separately
downloaded NVIDIA SDK and additional Vulkan/clang tooling.

For Android, install **JDK 25** and put `java` on PATH. In App Settings,
configure the SDK/NDK paths, install the SDK packages and accept their
licenses. Blockloom pins platform Android 35, build-tools 35.0.0 and NDK
major 27; SDK/NDK paths come from those settings, not `ANDROID_HOME` or
`ANDROID_NDK_HOME`. Install the Rust targets and check the toolchain:

```bash
rustup target add aarch64-linux-android x86_64-linux-android
just android-check
```

The Devices tab builds, installs and launches on the selected phone or
x86_64 emulator. Android runtime cross-builds prepare patched dependencies
automatically. Debug signing is automatic; release signing is configured
per project. The exported APK requires Android 10 (API 29) or newer.

For the WebGPU player:

```bash
rustup target add wasm32-unknown-unknown
just web-tools
just web-player
```

`just replace` uses `just web-player release` for faster optimized web rebuilds.
The first release build creates its own dependency cache; subsequent builds use
incremental compilation. For the fat-LTO shipping build, run `just web-player`
or `BLOCKLOOM_WEB_PROFILE=dist just replace`. Both profiles build only the wasm
library, avoiding an unused executable. Runtime speed and file size may differ
between profiles; choose `dist` for final shipping builds.

Then select the Web target in the Build dialog. `just web-tools` installs the
`wasm-bindgen` CLI matching Cargo.lock. Built web games require a browser
with WebGPU support.

## Layout

| Crate | What it is |
| --- | --- |
| `blockloom-qt` (`blockloom`) | the editor window, in Qt Quick |
| `blockloom-app` | every command the editor issues, and the state it edits |
| `blockloom-runtime` | the game world: a Bevy app running the blocks |
| `blockloom-core` | the shared engine: scene, blocks, VM, project files |
| `blockloom-protocol` | the editor-to-runtime wire format |

The block editor itself - canvas, dragging, snapping, the value expression
system - is [blockstitch](https://github.com/Blockworked/blockstitch), shared
with [Blockwork](https://github.com/Blockworked/Blockwork).

See [AGENTS.md](AGENTS.md) for the architecture in more detail.
