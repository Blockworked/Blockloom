name := "blockloom"
appid := "com.blockworked.Blockloom"

set allow-duplicate-variables

[unix]
set shell := ["sh", "-cu"]

[windows]
set shell := ["cmd.exe", "/Q", "/C"]

[unix]
TARGET := "target/release/blockloom"

[windows]
TARGET := "target/release/blockloom.exe"

LIBDIR := "/usr/lib/blockloom"

mkdir-cargo := if os() == "windows" { 'if not exist .cargo mkdir .cargo' } else { 'mkdir -p .cargo' }

# This machine's target triple, spelled the way `blockloom_core::build` spells
# it - the two have to agree for the exporter to find a staged player.
host-target := if os() == "windows" {
    if arch() == "aarch64" { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
} else if os() == "macos" {
    if arch() == "aarch64" { "aarch64-apple-darwin" } else { "x86_64-apple-darwin" }
} else {
    if arch() == "aarch64" { "aarch64-unknown-linux-gnu" } else { "x86_64-unknown-linux-gnu" }
}

players-dir := if os() == "windows" { 'target\release\players\' + host-target } else { "target/release/players/" + host-target }

mkdir-players := if os() == "windows" { 'if not exist "' + players-dir + '" mkdir "' + players-dir + '"' } else { 'mkdir -p "' + players-dir + '"' }

copy-player := if os() == "windows" { 'copy /Y target\dist\blockloom-runtime.exe "' + players-dir + '"' } else { 'cp target/dist/blockloom-runtime "' + players-dir + '/"' }

# Pinned DLSS SDK (NVIDIA/DLSS tag) matching dlss_wgpu 5.0.0's version chart.
# Fetched sparse by `just dlss-sdk` into gitignored `third-party/dlss`, so
# every clone gets it with one command instead of carrying 750MB.
dlss-version := "v310.7.0"
dlss-sdk-dir := justfile_directory() + "/third-party/dlss"

# Absolute, as dlss_wgpu's build script demands. Only consumed when the
# `dlss` cargo feature builds, so plain builds ignore it, present or not.
export DLSS_SDK := dlss-sdk-dir

rm-cargo-cfg := if os() == "windows" { 'if exist .cargo\config.toml (del /F /Q .cargo\config.toml)' } else { 'rm -f .cargo/config.toml' }

default: build

# Recreate local patched crates without committing dependency source trees.
# The wrapper finds Git Bash on Windows, where it is usually not on PATH.
prepare-patched-deps:
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/run-bash.py scripts/prepare-patched-deps.sh

# Build everything. The editor launches `blockloom-runtime` from beside itself,
# so the whole workspace has to be built, not just the `blockloom` package.
build *args: prepare-patched-deps
    {{if os() == "linux" { "python3 scripts/prune-target.py --run" } else if os() == "windows" { "python scripts/prune-target.py --run" } else { "" }}} cargo build --release --workspace {{args}}

# Trim old caches, preserving final binaries and staged players.
prune-target *args:
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/prune-target.py {{args}}

run: build
    {{TARGET}}

# Hub inspection and installation commands do not need an editor build.
hub *args:
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/hub.py {{args}}

# Build the workspace and open the separate Hub window.
hub-run: build
    target/release/blockloom-hub{{if os() == "windows" { ".exe" } else { "" }}}

# Rebuild and reinstall only the Hub. The optional argument limits Cargo jobs.
hub-replace jobs="": (hub-build jobs) hub-install

hub-build jobs="": prepare-patched-deps
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/prune-target.py --run cargo build --release -p blockloom-hub {{if jobs == "" { "" } else { "--jobs " + jobs }}}

[linux]
hub-install:
    sudo install -Dm0755 target/release/blockloom-hub /usr/lib/blockloom-hub/blockloom-hub
    sudo ln -sf /usr/lib/blockloom-hub/blockloom-hub /usr/bin/blockloom-hub
    sudo install -Dm0644 res/blockloom-hub.desktop /usr/share/applications/blockloom-hub.desktop
    sudo install -Dm0644 res/icons/blockloom.png /usr/share/icons/hicolor/256x256/apps/blockloom-hub.png

[windows]
hub-install:
    python scripts/install-windows.py hub-install

[macos]
hub-install:
    @echo 'Hub build is in target/release/blockloom-hub - run it from there.'

# Build the editor and runtime, then watch QML edits through Qt 6.12.
qml-preview *args: prepare-patched-deps
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/qml-preview.py {{args}}

# The browser dev loop: the real backend behind an HTTP bridge, plus Vite.
# Run these in two terminals, then open http://localhost:1420.
dev-backend: prepare-patched-deps
    cargo build -p blockloom-runtime
    cargo run -p blockloom-app --features dev-bridge --bin blockloom-devserver

dev-ui:
    cd ui && pnpm run dev

# A shell onto the backend: each line is a command, each answer is JSON.
# Builds the whole workspace first so Play has `blockloom-runtime` beside it.
shell *args: prepare-patched-deps
    just build
    cargo run -p blockloom-app --bin blockloom-shell -- {{args}}

# The player a built game ships: the same runtime, compiled as hard as the
# `dist` profile asks, staged where the exporter looks for it. `just build`
# keeps its quicker link, so this is a deliberate step before shipping games.
# Other platforms' payloads come from running this there - see `stage-player`.
player: prepare-patched-deps
    {{if os() == "linux" { "python3 scripts/prune-target.py --run" } else if os() == "windows" { "python scripts/prune-target.py --run" } else { "" }}} cargo build --profile dist -p blockloom-runtime
    {{mkdir-players}}
    {{copy-player}}

# The DLSS SDK, fetched once per clone: a sparse checkout (headers, link
# stubs, redistributable DLLs, programming guides - about 260MB, not the full
# 750MB tree) pinned to `dlss-version`. Cloning it means accepting NVIDIA's
# SDK license (see third-party/dlss/LICENSE.txt once fetched).
dlss-sdk:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="{{ dlss-sdk-dir }}"
    want="{{ dlss-version }}"
    if [ -d "$dir/.git" ]; then
        have="$(git -C "$dir" describe --tags 2>/dev/null || echo unknown)"
        if [ "$have" = "$want" ]; then echo "DLSS SDK $want already at $dir"; exit 0; fi
        echo "DLSS SDK is $have, want $want - re-fetching..."
        rm -rf "$dir"
    elif [ -e "$dir" ]; then
        echo "$dir exists and is not a git checkout - remove it first"; exit 1
    fi
    mkdir -p "$(dirname "$dir")"
    git -c advice.detachedHead=false clone --depth 1 --branch "$want" --filter=blob:none --sparse https://github.com/NVIDIA/DLSS.git "$dir"
    git -C "$dir" sparse-checkout set --no-cone \
        '/include/' '/LICENSE.txt' '/README.md' '/utils/' \
        '/lib/Linux_x86_64/*.a' '/lib/Linux_x86_64/rel/' \
        '/lib/Windows_x86_64/x64/' '/lib/Windows_x86_64/rel/' \
        '/doc/DLSS_Programming_Guide_Release.pdf' '/doc/DLSS-RR Integration Guide.pdf'
    du -sh "$dir"

# The DLSS player: the same runtime with Bevy's `dlss` path linked in, for
# NVIDIA RTX machines on Windows or Linux (macOS has no Vulkan DLSS path).
# Needs the SDK (`just dlss-sdk`), a Vulkan SDK, clang, and agreement to
# NVIDIA's SDK license. The redistributable DLLs are staged beside the
# payload automatically; before shipping games, add the section 9.5 license
# blurb from the programming guide (now at third-party/dlss/doc/) as
# DLSS_LICENSE.txt beside the staged player - builds then carry it along.
# A run without the DLLs falls back to TAA plus spatial. Never for web.
[unix]
player-dlss: dlss-sdk prepare-patched-deps
    @if [ "$(uname -s)" = "Darwin" ]; then echo "DLSS needs Windows or Linux (Vulkan RTX); macOS has no path."; exit 1; fi
    @if [ ! -f "${VULKAN_SDK:-/usr}/include/vulkan/vulkan.h" ]; then echo "Need a Vulkan SDK with headers (VULKAN_SDK, default /usr on Linux)."; exit 1; fi
    VULKAN_SDK="${VULKAN_SDK:-/usr}" cargo build --profile dist -p blockloom-runtime --features dlss
    {{mkdir-players}}
    {{copy-player}}
    cp "{{ dlss-sdk-dir }}/lib/Linux_x86_64/rel/"*.so.* "{{players-dir}}/"
    @echo 'Staged the DLSS player. Add DLSS_LICENSE.txt beside it in {{players-dir}} (section 9.5 blurb), or runs fall back to TAA.'

[windows]
player-dlss: dlss-sdk prepare-patched-deps
    @if "%VULKAN_SDK%"=="" (echo Set VULKAN_SDK to your Vulkan SDK - the Lunarg installer sets it system-wide. && exit 1)
    @if not exist "%VULKAN_SDK%\Include\vulkan\vulkan.h" (echo VULKAN_SDK=%VULKAN_SDK% has no Vulkan headers. && exit 1)
    cargo build --profile dist -p blockloom-runtime --features dlss
    {{mkdir-players}}
    {{copy-player}}
    copy /Y "{{ replace(dlss-sdk-dir, '/', '\') }}\lib\Windows_x86_64\rel\nvngx_dlss.dll" "{{ replace(players-dir, '/', '\') }}\"
    copy /Y "{{ replace(dlss-sdk-dir, '/', '\') }}\lib\Windows_x86_64\rel\nvngx_dlssd.dll" "{{ replace(players-dir, '/', '\') }}\"
    @echo Staged the DLSS player. Add DLSS_LICENSE.txt beside it in {{players-dir}} (section 9.5 blurb), or runs fall back to TAA.

# Put a player built on another machine where the exporter will find it.
# Blockloom can't cross-build a Bevy binary, so this is how another platform's
# payload arrives: run `just player` there, bring the file here, name its
# target triple.
[windows]
stage-player target file:
    if not exist "target\release\players\{{target}}" mkdir "target\release\players\{{target}}"
    copy /Y "{{ replace(file, '/', '\') }}" "target\release\players\{{target}}\{{ if target =~ 'windows' { 'blockloom-runtime.exe' } else { 'blockloom-runtime' } }}"

[unix]
stage-player target file:
    mkdir -p "target/release/players/{{target}}"
    cp "{{file}}" "target/release/players/{{target}}/{{ if target =~ 'windows' { 'blockloom-runtime.exe' } else { 'blockloom-runtime' } }}"

# Web player (Phase 8): the runtime for wasm32-unknown-unknown, drawing
# through WebGPU, with --no-default-features - no Solari ray tracing and no
# Basis/KTX2 C++ codecs (web builds ship PNG/JPEG). Blocks run on the VM;
# scripts compile to wasm modules of their own. Needs, once:
# `rustup target add wasm32-unknown-unknown` and `just web-tools`.
web-check: prepare-patched-deps
    cargo check -p blockloom-runtime --no-default-features --features plugins --target wasm32-unknown-unknown

# The wasm-bindgen CLI matching Cargo.lock's wasm-bindgen, which the glue
# generator has to match exactly.
web-tools: prepare-patched-deps
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/web-tools.py

# The web player, staged beside the editor where the Build dialog looks for
# it (players/wasm32-unknown-unknown/: the wasm and its JS glue). `dist` is
# what games ship; `release` links much faster for trying things out.
web-player profile="dist": prepare-patched-deps
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/run-bash.py scripts/web-player.sh {{profile}}

# Builds a project folder for the browser: one self-contained .html under
# out/, exactly what the Build dialog's Web target makes. Open it from disk
# or host it anywhere static (`just web-serve`).
web-build project out="web-dist" profile="dist": prepare-patched-deps
    just web-player {{profile}}
    cargo build --release -p blockloom-app --bin blockloom-shell
    printf 'open-project path=%s\nbuild-game path=%s target=wasm32-unknown-unknown\n' "$(realpath '{{project}}')" "$(realpath -m '{{out}}')" \
        | BLOCKLOOM_PLAYERS="${CARGO_TARGET_DIR:-target}/release/players" "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --no-state

# Static host for a web build, for browsers that won't run a page from disk.
web-serve dir="web-dist" port="8080":
    cd "{{dir}}" && python3 -m http.server {{port}} --bind 127.0.0.1

# Headless check of a built page: it unpacks, the world builds, scripts open
# and actors move. Needs Playwright (`npm i -g playwright`) and WebGPU in its
# Chromium; software Vulkan (lavapipe) is enough.
web-smoke page *args:
    NODE_PATH="$(npm root -g)" node scripts/web-smoke.cjs "{{page}}" {{args}}

# Android toolchain probe (Phase 6.5): SDK/NDK/JDK/Rust targets, no device
# needed. Same code the App Settings dialog reports.
android-check: prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-status' --no-state

# Android SDK install (Phase 6.5): the bootstrap download plus the pinned
# packages through sdkmanager. Same code the Settings button runs; licenses
# stay unaccepted until `android-accept-licenses accept=true`.
android-sdk-install: prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-install-sdk' --no-state

# Builds a project folder into a signed APK, exactly what the Build dialog's
# Android rows make: the NDK cross-builds the runtime into lib/<abi>/, the
# game folder stages under the APK's assets, and the debug keystore - or the
# project's release key with BLOCKLOOM_ANDROID_STORE_PASS - signs it. First
# run needs the network for the target's crates.
android-build project out="android-dist" triple="aarch64-linux-android": prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    printf 'open-project path=%s\nbuild-game path=%s target=%s\n' "$(realpath '{{project}}')" "$(realpath -m '{{out}}')" "{{triple}}" \
        | "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --no-state

# Forgets whatever the OS keyring keeps for a project's release key. Needs
# an open project, like the Build dialog's Forget button.
android-forget-passwords project: prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    printf 'open-project path=%s\nandroid-forget-passwords\n' "$(realpath '{{project}}')" \
        | "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --no-state

# Makes an AVD on the pinned Android 35 x86_64 image (empty names the
# managed default), exactly what the App settings emulator section makes.
android-avd-create name="": prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-create-avd{{ if name != "" { " name=" + name } else { "" } }}' --no-state

# Boots an AVD and waits up to waitSecs for adb to see it booted: 5 minutes
# when unset, 0 to return right after spawning. Empty names the managed
# default, created on the spot when no AVDs exist at all. headless=true
# hides the host window for the editor's embedded view.
android-emulator-start avd="" wait="300" headless="": prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-start-emulator{{ if avd != "" { " avd=" + avd } else { "" } }}{{ if wait != "" { " waitSecs=" + wait } else { "" } }}{{ if headless != "" { " headless=" + headless } else { "" } }}' --no-state

# Stops the running emulator on serial (empty stops the only running one;
# a physical serial is refused).
android-emulator-stop serial="": prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-stop-emulator{{ if serial != "" { " serial=" + serial } else { "" } }}' --no-state

# Installs an APK on a connected device or emulator and launches it
# (`android-device-status` lists the serials when several are attached).
android-install apk app device="": prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-install apk="{{apk}}" app="{{app}}"{{ if device != "" { " device=" + device } else { "" } }}' --no-state

# One-shot device log for the dev loop: the runtime's blockloom markers plus
# any Rust panic. What `android-smoke` checks and what a developer reads first.
android-logcat device="": prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-logcat{{ if device != "" { " device=" + device } else { "" } }}' --no-state

# Poll the device log the way the Build dialog streams it: dump, clear the
# buffer for the next poll, and append every kept line to the RunLog. Repeat
# for a follow tail; each call reads only what arrived since the last.
android-logcat-tail device="": prepare-patched-deps
    cargo build --release -p blockloom-app --bin blockloom-shell
    "${CARGO_TARGET_DIR:-target}/release/blockloom-shell" --eval 'android-logcat-tail{{ if device != "" { " device=" + device } else { "" } }}' --no-state

# Typechecks the shipped runtime for both Android triples without linking:
# new `cfg(target_os = "android")` code has to compile there, not just here.
# The C build scripts (blake3, ring) need the NDK clang, so this resolves the
# toolchain the same way a build does: the installed SDK's NDK through
# `android-status`, or the `ndk` dir handed in (CI downloads one standalone).
# Needs the Android Rust std (`rustup target add aarch64-linux-android
# x86_64-linux-android`); the SDK licenses don't matter for a check.
android-runtime-check ndk="": prepare-patched-deps
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p blockloom-app --bin blockloom-shell
    SHELL_BIN="${CARGO_TARGET_DIR:-target}/release/blockloom-shell"
    if [ -z "{{ndk}}" ]; then
      NDK="$("$SHELL_BIN" --eval 'android-status' --no-state \
        | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['ndk_path'])")"
    else
      NDK="$(realpath '{{ndk}}')"
    fi
    case "$(uname -s)" in
      Linux) HOST="linux-x86_64" ;;
      Darwin) HOST="darwin-x86_64" ;;
      *) HOST="windows-x86_64" ;;
    esac
    BIN="$NDK/toolchains/llvm/prebuilt/$HOST/bin"
    # The API level on the wrapper only sets the default -target; rustc
    # passes --target itself. 29 is MIN_SDK in blockloom-core/src/android.rs.
    for triple in aarch64-linux-android x86_64-linux-android; do
      stem="$(echo "$triple" | tr '-' '_')"
      upper="$(echo "$triple" | tr '[:lower:]' '[:upper:]' | tr '-' '_')"
      export "CC_$stem"="$BIN/${triple}29-clang"
      export "CXX_$stem"="$BIN/${triple}29-clang++"
      export "AR_$stem"="$BIN/llvm-ar"
      export "CARGO_TARGET_${upper}_LINKER"="$BIN/${triple}29-clang"
      # The shipped .so builds with no default features (SDR-only, no Solari
      # or texture compression), so the check builds the same shape.
      cargo check -p blockloom-runtime --no-default-features --target "$triple"
    done

# Headless check of an APK beside `web-smoke`: installs on a connected device
# or emulator, launches, and watches logcat for the world-built marker and a
# second actor snapshot, failing on any Rust panic. Needs exactly one device
# (or pass its serial) - an emulator counts.
android-smoke apk app device="":
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/run-bash.py scripts/android-smoke.sh "{{apk}}" "{{app}}" "{{device}}"

test: prepare-patched-deps
    {{if os() == "linux" { "python3 scripts/prune-target.py --run" } else if os() == "windows" { "python scripts/prune-target.py --run" } else { "" }}} cargo test --workspace

# Qt Quick interaction and layout tests use the staged blockstitch controls.
qml-test: build
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/run-bash.py scripts/test-devices-qml.sh

# An MCP server standing on `blockloom-shell`: every backend command becomes
# an MCP tool, so an agent can drive a project the way a user does. Builds the
# workspace first so the shell is current, then wires an MCP client to it.
mcp:
    just build
    cd mcp && pnpm install && pnpm run build
    echo Wire your MCP client to:
    echo {"command":"node","args":["{{ justfile_directory() }}\\mcp\\dist\\index.js"]}

clean:
    cargo clean

# Work on blockstitch locally: link the Vue half and point cargo's
# blockstitch-core and blockstitch-qml at the same checkout, without touching
# the committed lockfiles.
blockstitch-local path="../../blockstitch":
    cd ui && npm pkg set dependencies.blockstitch="link:{{path}}" && pnpm install
    git update-index --skip-worktree ui/package.json ui/pnpm-lock.yaml
    {{mkdir-cargo}}
    {{ if os() == "windows" { 'echo paths = ["' + replace(clean(justfile_directory() / "ui" / path), "\\", "/") + '", "' + replace(clean(justfile_directory() / "ui" / path), "\\", "/") + '/crates/blockstitch-core"] > .cargo\config.toml' } else { "printf 'paths = [\"" + "%s\", \"%s/crates/blockstitch-core" + "\"]\\n' \"$(realpath ui/" + path + ")\" \"$(realpath ui/" + path + ")\" > .cargo/config.toml" } }}

blockstitch-published commit="": prepare-patched-deps
    {{rm-cargo-cfg}}
    git update-index --no-skip-worktree ui/package.json ui/pnpm-lock.yaml
    git checkout -- ui/package.json ui/pnpm-lock.yaml
    {{ if os() == "windows" { 'if not "' + commit + '"=="" (cd ui && npm pkg set dependencies.blockstitch="github:Blockworked/blockstitch#' + commit + '")' } else { 'if [ -n "' + commit + '" ]; then cd ui && npm pkg set dependencies.blockstitch="github:Blockworked/blockstitch#' + commit + '"; fi' } }}
    cd ui && pnpm install
    {{ if os() == "windows" { 'if not "' + commit + '"=="" (node scripts/set-blockstitch-rev.js ' + commit + ')' } else { 'if [ -n "' + commit + '" ]; then node scripts/set-blockstitch-rev.js ' + commit + '; fi' } }}
    cargo fetch

[linux]
install:
    # The editor starts `blockloom-runtime` from beside itself, so both live
    # in a private libdir with a symlink on the PATH.
    sudo install -Dm0755 {{TARGET}} {{LIBDIR}}/blockloom
    sudo install -Dm0755 target/release/blockloom-runtime {{LIBDIR}}/blockloom-runtime
    sudo ln -sf {{LIBDIR}}/blockloom /usr/bin/blockloom
    sudo install -Dm0644 res/blockloom.desktop /usr/share/applications/com.blockworked.Blockloom.desktop
    sudo install -Dm0644 res/icons/blockloom.png /usr/share/icons/hicolor/256x256/apps/blockloom.png
    # Staged players a built game ships, beside the editor where the exporter
    # looks for them (see `just player` / `just web-player`).
    if [ -d target/release/players ]; then sudo mkdir -p {{LIBDIR}}/players && sudo cp -a target/release/players/. {{LIBDIR}}/players/; fi

[linux]
uninstall:
    sudo rm -rf {{LIBDIR}}
    sudo rm -f /usr/bin/blockloom /usr/share/applications/com.blockworked.Blockloom.desktop /usr/share/applications/blockloom.desktop /usr/share/icons/hicolor/256x256/apps/blockloom.png

# Install the editor, runtime and staged players for the current Windows
# user, with a Start Menu shortcut and a PATH entry. No admin rights needed.
[windows]
install:
    python scripts/install-windows.py install

[windows]
uninstall:
    python scripts/install-windows.py uninstall

# Reinstall the editor and runtime, keeping the already staged game players.
[linux]
replace-fast: build install

# Reuse the workspace runtime as the native player without another build.
[linux]
_stage-release-player:
    {{mkdir-players}}
    cp target/release/blockloom-runtime "{{players-dir}}/"

[windows]
_stage-release-player:
    {{mkdir-players}}
    copy /Y target\release\blockloom-runtime.exe "{{players-dir}}\"

[macos]
_stage-release-player:
    {{mkdir-players}}
    cp target/release/blockloom-runtime "{{players-dir}}/"

# Build the editor and web player concurrently, then reinstall on success.
# The optional jobs argument sets each build's Cargo job limit.
replace jobs="": prepare-patched-deps
    {{if os() == "windows" { "python" } else { "python3" }}} scripts/replace.py {{jobs}}

# Stage and seal the voxel plugin (com.blockworked.voxel) into target/plugins
voxel-plugin:
    rustup target add wasm32-unknown-unknown
    cargo build --release -p blockloom-voxel --target wasm32-unknown-unknown
    rm -rf target/plugins/com.blockworked.voxel
    mkdir -p target/plugins/com.blockworked.voxel/portable
    cp -r plugins/voxel/package/. target/plugins/com.blockworked.voxel/
    cp target/wasm32-unknown-unknown/release/blockloom_voxel.wasm target/plugins/com.blockworked.voxel/portable/voxel.wasm
    cargo run -p blockloom-app --bin blockloom-shell -- --no-state --eval 'plugin-seal path=target/plugins/com.blockworked.voxel'

# The SDK example plugin (com.example.tally) as a sealed portable package in
# target/plugins/, ready for `plugin-install source=path:...`.
example-plugin:
    rustup target add wasm32-unknown-unknown
    cargo build --release -p blockloom-example-tally --target wasm32-unknown-unknown
    rm -rf target/plugins/com.example.tally
    mkdir -p target/plugins/com.example.tally/portable
    cp -r plugins/examples/tally/package/. target/plugins/com.example.tally/
    cp target/wasm32-unknown-unknown/release/blockloom_example_tally.wasm target/plugins/com.example.tally/portable/tally.wasm
    cargo run -p blockloom-app --bin blockloom-shell -- --no-state --eval 'plugin-seal path=target/plugins/com.example.tally'
