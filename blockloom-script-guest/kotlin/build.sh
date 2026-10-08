#!/usr/bin/env bash
# Builds a Kotlin/Wasm guest into a component:
#   kotlin/build.sh [guest.kt] [out.wasm]     (defaults: ../templates/minimal.kt, minimal.wasm)
# Needs Gradle on JDK 17+, wasm-tools, and the WASI preview1 reactor adapter
# (wasi_snapshot_preview1.reactor.wasm from the wasmtime releases) as $WASI_ADAPTER.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$(realpath "${1:-$here/../templates/minimal.kt}")
out=$(realpath -m "${2:-minimal.wasm}")
adapter=${WASI_ADAPTER:?set WASI_ADAPTER to the preview1 reactor adapter}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$here"/settings.gradle.kts "$here"/build.gradle.kts "$work"/
mkdir -p "$work/src/wasmWasiMain/kotlin"
cp "$here"/src/wasmWasiMain/kotlin/Blockloom.kt "$work/src/wasmWasiMain/kotlin/"
cp "$src" "$work/src/wasmWasiMain/kotlin/Guest.kt"
(cd "$work" && gradle --no-daemon -q compileDevelopmentExecutableKotlinWasmWasi)
core=$work/build/compileSync/wasmWasi/main/developmentExecutable/kotlin/guest.wasm
wasm-tools component embed "$here/../wit" --world script "$core" -o "$work/embedded.wasm"
wasm-tools component new "$work/embedded.wasm" --adapt wasi_snapshot_preview1="$adapter" -o "$out"
echo "wrote $out"
