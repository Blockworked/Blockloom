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

rm-cargo-cfg := if os() == "windows" { 'if exist .cargo\config.toml (del /F /Q .cargo\config.toml)' } else { 'rm -f .cargo/config.toml' }

default: build

# Build everything. The editor launches `blockloom-runtime` from beside itself,
# so the whole workspace has to be built, not just the `blockloom` package.
build *args:
    cargo build --release --workspace {{args}}

run: build
    {{TARGET}}

# The browser dev loop: the real backend behind an HTTP bridge, plus Vite.
# Run these in two terminals, then open http://localhost:1420.
dev-backend:
    cargo build -p blockloom-runtime
    cargo run -p blockloom-app --features dev-bridge --bin blockloom-devserver

dev-ui:
    cd ui && pnpm run dev

# A shell onto the backend: each line is a command, each answer is JSON.
# Builds the whole workspace first so Play has `blockloom-runtime` beside it.
shell *args:
    just build
    cargo run -p blockloom-app --bin blockloom-shell -- {{args}}

# The player a built game ships: the same runtime, compiled as hard as the
# `dist` profile asks, staged where the exporter looks for it. `just build`
# keeps its quicker link, so this is a deliberate step before shipping games.
# Other platforms' payloads come from running this there - see `stage-player`.
player:
    cargo build --profile dist -p blockloom-runtime
    {{mkdir-players}}
    {{copy-player}}

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
web-check:
    cargo check -p blockloom-runtime --no-default-features --target wasm32-unknown-unknown

# The wasm-bindgen CLI matching Cargo.lock's wasm-bindgen, which the glue
# generator has to match exactly.
web-tools:
    cargo install wasm-bindgen-cli --locked --version "$(cargo metadata --format-version 1 --filter-platform wasm32-unknown-unknown | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "wasm-bindgen"))')"

# The web player, staged beside the editor where the Build dialog looks for
# it (players/wasm32-unknown-unknown/: the wasm and its JS glue). `dist` is
# what games ship; `release` links much faster for trying things out.
web-player profile="dist":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p blockloom-runtime --no-default-features --target wasm32-unknown-unknown --profile {{profile}}
    command -v wasm-bindgen >/dev/null || { echo "need wasm-bindgen-cli: just web-tools"; exit 1; }
    out="${CARGO_TARGET_DIR:-target}/release/players/wasm32-unknown-unknown"
    mkdir -p "$out"
    wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
        --out-dir "$out" "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/{{ if profile == "dev" { "debug" } else { profile } }}/blockloom_runtime.wasm"
    ls -l "$out"

# Builds a project folder for the browser: one self-contained .html under
# out/, exactly what the Build dialog's Web target makes. Open it from disk
# or host it anywhere static (`just web-serve`).
web-build project out="web-dist" profile="dist":
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

test:
    cargo test --workspace

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

blockstitch-published commit="":
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

# Reinstall the editor plus the players a built game ships, so an installed
# Build dialog can offer this machine's native target and the web target.
[linux]
replace: build player web-player uninstall install
