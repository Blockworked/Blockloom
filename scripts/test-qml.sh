#!/usr/bin/env bash
# Run every QML test without building the editor: stage blockstitch's QML
# straight from its source (the pinned commit, or BLOCKSTITCH_SOURCE), with a
# qmldir written from its module.json. Extra arguments go to qmltestrunner.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
runner="${QMLTESTRUNNER:-qmltestrunner}"
if ! command -v "$runner" >/dev/null; then
    echo "Set QMLTESTRUNNER to Qt 6's qmltestrunner executable." >&2
    exit 1
fi
temp="$(mktemp -d)"
trap 'rm -rf "$temp"' EXIT

source_dir="${BLOCKSTITCH_SOURCE:-}"
# `just blockstitch-local` points Cargo at a checkout; test that one too.
if [[ -z "$source_dir" && -f "$root/.cargo/config.toml" ]]; then
    for path in $(sed -n 's/^paths = \[\(.*\)\]/\1/p' "$root/.cargo/config.toml" | tr -d '",'); do
        if [[ -f "$path/qml/module.json" ]]; then source_dir="$path"; break; fi
    done
fi
if [[ -z "$source_dir" ]]; then
    rev="$(sed -n 's/^blockstitch-qml = .*rev = "\([0-9a-f]*\)".*/\1/p' "$root/Cargo.toml")"
    [[ -n "$rev" ]] || { echo "No blockstitch-qml rev in Cargo.toml; set BLOCKSTITCH_SOURCE." >&2; exit 1; }
    source_dir="$temp/blockstitch"
    git init -q "$source_dir"
    git -C "$source_dir" fetch -q --depth 1 https://github.com/Blockworked/blockstitch "$rev"
    git -C "$source_dir" -c advice.detachedHead=false checkout -q FETCH_HEAD
fi

imports="$temp/imports"
mkdir -p "$imports/com/blockworked"
python3 -I - "$source_dir/qml" "$imports/com/blockworked/Blockstitch" <<'PY'
import json, pathlib, shutil, sys
qml, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
manifest = json.loads((qml / "module.json").read_text())
shutil.copytree(qml / manifest["dir"], out)
singletons = set(manifest["singletons"])
major = manifest["version"].split(".")[0]
lines = [f"module {manifest['uri']}"]
for name in manifest["files"]:
    if not name.endswith(".qml"):
        continue
    kind = "singleton " if name in singletons else ""
    lines.append(f"{kind}{pathlib.Path(name).stem} {manifest['version']} {name}")
# Files in subfolders are reached by relative imports, so only the root needs entries.
(out / "qmldir").write_text("\n".join(lines) + "\n")
PY
cp -R "$root/blockloom-qt/tests/qml/fixtures/Blockloom" "$imports/com/blockworked/Blockloom"

QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-offscreen}" \
QT_QUICK_BACKEND="${QT_QUICK_BACKEND:-software}" \
    "$runner" -import "$imports" -input "$root/blockloom-qt/tests/qml" "$@"
