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

CEF_DIR := "target/release"
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
player:
    cargo build --profile dist -p blockloom-runtime
    {{mkdir-players}}
    {{copy-player}}

test:
    cargo test --workspace

clean:
    cargo clean

# Work on blockstitch locally: link the Vue half and point cargo at the same
# checkout, without touching the committed lockfiles.
blockstitch-local path="../../blockstitch":
    cd ui && npm pkg set dependencies.blockstitch="link:{{path}}" && pnpm install
    git update-index --skip-worktree ui/package.json ui/pnpm-lock.yaml
    {{mkdir-cargo}}
    {{ if os() == "windows" { 'echo paths = ["' + replace(clean(justfile_directory() / "ui" / path), "\\", "/") + '/crates/blockstitch-core"] > .cargo\config.toml' } else { "printf 'paths = [\"" + "%s/crates/blockstitch-core" + "\"]\\n' \"$(realpath ui/" + path + ")\" > .cargo/config.toml" } }}

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
    # The binary's RUNPATH is `$ORIGIN`, so the CEF runtime payload (libcef.so,
    # the GL/Vulkan shims, *.pak, icudtl.dat, locales/, ...) has to live beside
    # it in a private libdir rather than in /usr/bin.
    sudo install -Dm0755 {{TARGET}} {{LIBDIR}}/blockloom
    sudo install -Dm0755 {{CEF_DIR}}/blockloom-runtime {{LIBDIR}}/blockloom-runtime
    sudo install -Dm0755 {{CEF_DIR}}/libcef.so {{LIBDIR}}/libcef.so
    sudo install -Dm0755 {{CEF_DIR}}/libEGL.so {{LIBDIR}}/libEGL.so
    sudo install -Dm0755 {{CEF_DIR}}/libGLESv2.so {{LIBDIR}}/libGLESv2.so
    sudo install -Dm0755 {{CEF_DIR}}/libvk_swiftshader.so {{LIBDIR}}/libvk_swiftshader.so
    sudo install -Dm0755 {{CEF_DIR}}/libvulkan.so.1 {{LIBDIR}}/libvulkan.so.1
    sudo install -Dm0755 {{CEF_DIR}}/chrome-sandbox {{LIBDIR}}/chrome-sandbox
    sudo install -Dm0644 {{CEF_DIR}}/vk_swiftshader_icd.json {{LIBDIR}}/vk_swiftshader_icd.json
    sudo install -Dm0644 {{CEF_DIR}}/icudtl.dat {{LIBDIR}}/icudtl.dat
    sudo install -Dm0644 {{CEF_DIR}}/v8_context_snapshot.bin {{LIBDIR}}/v8_context_snapshot.bin
    sudo install -Dm0644 {{CEF_DIR}}/chrome_100_percent.pak {{LIBDIR}}/chrome_100_percent.pak
    sudo install -Dm0644 {{CEF_DIR}}/chrome_200_percent.pak {{LIBDIR}}/chrome_200_percent.pak
    sudo install -Dm0644 {{CEF_DIR}}/resources.pak {{LIBDIR}}/resources.pak
    sudo rm -rf {{LIBDIR}}/locales
    sudo cp -r {{CEF_DIR}}/locales {{LIBDIR}}/locales
    sudo ln -sf {{LIBDIR}}/blockloom /usr/bin/blockloom
    sudo install -Dm0644 res/blockloom.desktop /usr/share/applications/blockloom.desktop
    sudo install -Dm0644 res/icons/blockloom.png /usr/share/icons/hicolor/256x256/apps/blockloom.png

[linux]
uninstall:
    sudo rm -rf {{LIBDIR}}
    sudo rm -f /usr/bin/blockloom /usr/share/applications/blockloom.desktop /usr/share/icons/hicolor/256x256/apps/blockloom.png

[linux]
replace: build uninstall install
