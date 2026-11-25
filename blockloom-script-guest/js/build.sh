#!/usr/bin/env bash
# Builds one JavaScript or TypeScript script into a component with jco.
#
#   js/build.sh [source.mjs|source.ts] [out.wasm]
#
# TypeScript is type stripping only: jco componentizes the JavaScript, so a
# .ts source must be plain ES modules with types, not namespaces or decorators.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$(realpath "${1:-$here/../templates/minimal.mjs}")
out=$(realpath -m "${2:-minimal.wasm}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
entry=$work/guest.mjs
case "$src" in
  *.ts) npx --yes esbuild@0.25.0 "$src" --format=esm --target=es2022 --outfile="$entry" >/dev/null ;;
  *) cp "$src" "$entry" ;;
esac
npx --yes @bytecodealliance/jco@1.4.0 componentize "$entry" --wit "$here/../wit" --world-name script --out "$out"
echo "wrote $out"
