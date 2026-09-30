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
- Qt **6.10 or newer**, including development headers and build tools for
  Core, Gui, Qml, Quick, Quick Controls 2, Quick Dialogs 2 and Multimedia.
  QtGui private headers are needed for the Linux Wayland HDR integration.
  Use a Qt kit matching your compiler and architecture. Set `QMAKE` to its
  `qmake` executable if it is not discoverable as `qmake6` or `qmake` on PATH.
- Bash, curl, tar, patch, and `sha256sum` (or `shasum`) for preparing patched
  dependencies. On Windows, put the Git Bash tools on PATH.
- On Linux: `pkg-config` and development libraries for ALSA, udev, Wayland,
  xkbcommon and EGL, plus a working Vulkan loader and GPU driver.

The first build needs network access for Rust dependencies, the pinned
`blockstitch` Git dependency, and patched dependency archives. A sibling
`blockstitch` checkout is not required.

### Editor And Runtime

Run from the repository root:

```bash
just build                  # release build of the entire workspace
./target/release/blockloom   # Windows: target\release\blockloom.exe
just run                    # build, then launch the editor
```

Always build the **whole workspace**, not just `-p blockloom`. On Windows,
macOS, or Linux with `BLOCKLOOM_RUNTIME=process`, the editor launches the
`blockloom-runtime` executable beside itself. Linux normally renders in-process.

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
QML is compiled into the editor, so QML changes also require a rebuild.
For QML tests, put Qt 6's `qmltestrunner` on PATH or set `QMLTESTRUNNER`
to its executable.
The legacy `ui/` and `src-tauri/` applications are not part of this build;
Node.js is not needed to build the Qt editor.

### Game Players

`just player` builds the optimized native player and stages it under
`target/release/players/<host-target>/` for exporting games from the editor.
Build and stage other native platforms' players on those platforms.
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
