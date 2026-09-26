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

# Web player (Phase 8): the runtime builds for wasm32-unknown-unknown with
# --no-default-features - no Solari ray tracing, no Basis/KTX2 C++ codecs
# (web v1 ships PNG/JPEG), no `dlopen` scripts or native logic (blocks run
# on the VM there). Needs the target once: `rustup target add
# wasm32-unknown-unknown`, plus a matching glue generator once:
# `cargo install wasm-bindgen-cli --version <lock's wasm-bindgen> --locked`.
web-check:
    cargo check -p blockloom-runtime --no-default-features --target wasm32-unknown-unknown

# A runnable browser folder: the wasm player, its JS glue, and a host page
# that feeds it a pack. Pass a game folder (one holding `game.pack`) to play
# a real game; without one the page reports the missing pack through the
# error overlay. `profile=release` for a small file that boots fast;
# `dev` (the default) iterates faster but boots slowly. Assets resolve
# against the server root, so serve the folder `web-serve` makes - the
# single-file `.html` with everything inlined is the next packaging step,
# not this one.
web-build out="web-dist" pack="" profile="dev":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p blockloom-runtime --no-default-features --target wasm32-unknown-unknown --profile {{profile}}
    if ! command -v wasm-bindgen >/dev/null; then
        echo "need wasm-bindgen-cli (once): cargo install wasm-bindgen-cli --version $(cargo metadata --format-version 1 --filter-platform wasm32-unknown-unknown 2>/dev/null | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "wasm-bindgen"))') --locked"
        exit 1
    fi
    rm -rf "{{out}}/pkg"
    mkdir -p "{{out}}/pkg"
    wasm-bindgen --target web --out-dir "{{out}}/pkg" "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/{{ if profile == "dev" { "debug" } else { profile } }}/blockloom_runtime.wasm"
    if [ -n "{{pack}}" ]; then cp -r "{{pack}}"/. "{{out}}"/; fi
    cat > "{{out}}/index.html" <<'PAGE'
    <!DOCTYPE html>
    <html lang="en">
    <head><meta charset="utf-8"><title>Blockloom</title></head>
    <body style="margin:0;background:#1b212c;display:flex;justify-content:center;align-items:center;min-height:100vh;">
    <canvas id="blockloom-canvas" width="960" height="720"></canvas>
    <script type="module">
    import init, { start_game } from "./pkg/blockloom_runtime.js";
    const packUrl = new URLSearchParams(location.search).get("pack") ?? "game.pack";
    await init();
    let pack;
    try {
        const response = await fetch(packUrl);
        if (!response.ok) throw new Error(response.status + " " + response.statusText);
        pack = await response.text();
    } catch (e) {
        document.body.insertAdjacentHTML("beforeend",
            `<div style="position:fixed;left:16px;right:16px;bottom:16px;padding:12px 16px;background:#3a1414;color:#ffd7d7;font:14px sans-serif;border:1px solid #a33;border-radius:8px;">Blockloom: couldn't load ${packUrl}: ${e}</div>`);
        throw e;
    }
    start_game(pack, "#blockloom-canvas");
    </script>
    </body>
    </html>
    PAGE
    echo "web player in {{out}}/ - serve it with: just web-serve {{out}}"

# Static host for the `web-build` folder: right MIME types for wasm/JS, so a
# smoke test (or a browser) can load the player with no real server.
web-serve dir="web-dist" port="8080":
    cd "{{dir}}" && python3 -c 'import functools, http.server; http.server.SimpleHTTPRequestHandler.extensions_map.update({".wasm": "application/wasm", ".js": "text/javascript"}); http.server.test(functools.partial(http.server.ThreadingHTTPServer, ("127.0.0.1", {{port}}), http.server.SimpleHTTPRequestHandler), bind=None)'

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

[linux]
uninstall:
    sudo rm -rf {{LIBDIR}}
    sudo rm -f /usr/bin/blockloom /usr/share/applications/com.blockworked.Blockloom.desktop /usr/share/applications/blockloom.desktop /usr/share/icons/hicolor/256x256/apps/blockloom.png

[linux]
replace: build uninstall install
