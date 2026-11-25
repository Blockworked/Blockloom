#!/usr/bin/env bash
# Builds one Go script into a component with TinyGo and wasm-tools.
#
#   go/build.sh [source.go] [out.wasm]
#
# wasm-unknown keeps WASI out of the component: wasip2 would import wasi:cli,
# which the frozen world does not declare.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$(realpath "${1:-$here/../templates/minimal.go}")
out=$(realpath -m "${2:-minimal.wasm}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -r "$here/../wit" "$work/wit"
cp "$src" "$work/main.go"
cp "$here"/go.mod.template "$work/go.mod"
(cd "$work" && go tool wit-bindgen-go generate --world script --out internal ./wit >/dev/null)
(cd "$work" && tinygo build -target=wasm-unknown -o core.wasm .)
wasm-tools component embed "$work/wit" "$work/core.wasm" --world script -o "$work/embedded.wasm"
wasm-tools component new "$work/embedded.wasm" -o "$out"
echo "wrote $out"
