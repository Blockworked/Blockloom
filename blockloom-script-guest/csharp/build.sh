#!/usr/bin/env bash
# Builds a C# guest (Mono on wasi-wasm) into a component:
#   csharp/build.sh [guest.cs] [out.wasm]     (defaults: ../templates/minimal.cs, minimal.wasm)
# Needs the .NET 10 SDK with the wasi-experimental workload, wasi-sdk 25 (exactly:
# WASI_SDK_PATH, default /opt/wasi-sdk) and wit-bindgen 0.40 on PATH.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$(realpath "${1:-$here/../templates/minimal.cs}")
out=$(realpath -m "${2:-minimal.wasm}")
export WASI_SDK_PATH=${WASI_SDK_PATH:-/opt/wasi-sdk}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$here"/guest.csproj "$here"/shim.c "$here"/Blockloom.cs "$work"/
wit-bindgen c "$here/../wit" --world script --out-dir "$work/gen"
(cd "$work" && dotnet publish -c Release -p:GuestSource="$src" -v q)
cp "$work"/obj/Release/net10.0/wasi-wasm/wasm/for-publish/guest.wasm "$out"
echo "wrote $out"
