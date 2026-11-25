#!/usr/bin/env bash
# Builds one Python script into a component with componentize-py.
#
#   python/build.sh [source.py] [out.wasm]
#
# componentize-py wants the module on its import path under a plain name, so
# the source is copied into a scratch folder as `guest.py` first.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$(realpath "${1:-$here/../templates/minimal.py}")
out=$(realpath -m "${2:-minimal.wasm}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$src" "$work/guest.py"
(cd "$work" && componentize-py -d "$here/../wit" -w script componentize guest -o "$out")
echo "wrote $out"
