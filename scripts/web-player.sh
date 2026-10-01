#!/usr/bin/env bash
# The web player, staged beside the editor where the Build dialog looks for
# it (players/wasm32-unknown-unknown/: the wasm and its JS glue). `dist` is
# what games ship; `release` links much faster for trying things out.
set -euo pipefail
profile="${1-dist}"
# Separate host-tool outputs as well as intermediates from native builds.
web_target_dir="${CARGO_TARGET_DIR:-target}/web-build"
export CARGO_BUILD_BUILD_DIR="${CARGO_BUILD_BUILD_DIR:-$web_target_dir}"
case "$(uname -s)" in
    Linux) python3 scripts/prune-target.py --target-dir "${CARGO_TARGET_DIR:-target}" --run ;;
    MINGW* | MSYS* | CYGWIN*) python scripts/prune-target.py --target-dir "${CARGO_TARGET_DIR:-target}" --run ;;
esac
cargo build -p blockloom-runtime --lib --no-default-features --target wasm32-unknown-unknown --profile "$profile" --target-dir "$web_target_dir"
command -v wasm-bindgen >/dev/null || { echo "need wasm-bindgen-cli: just web-tools"; exit 1; }
out="${CARGO_TARGET_DIR:-target}/release/players/wasm32-unknown-unknown"
mkdir -p "$out"
case "$profile" in
    dev) subdir=debug ;;
    *) subdir="$profile" ;;
esac
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
    --out-dir "$out" "$web_target_dir/wasm32-unknown-unknown/$subdir/blockloom_runtime.wasm"
ls -l "$out"
case "$(uname -s)" in
    Darwin) ;;
    *) just prune-target ;;
esac
