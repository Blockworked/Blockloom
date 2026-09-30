#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
module="$root/target/cxxqt/qml_modules/com/blockworked/Blockstitch"
if [[ ! -f "$module/qmldir" ]]; then
    echo "Build the workspace with just build before running QML tests." >&2
    exit 1
fi
runner="${QMLTESTRUNNER:-qmltestrunner}"
if ! command -v "$runner" >/dev/null; then
    echo "Set QMLTESTRUNNER to Qt 6's qmltestrunner executable." >&2
    exit 1
fi
temp="$(mktemp -d)"
trap 'rm -rf "$temp"' EXIT
mkdir -p "$temp/com/blockworked"
cp -R "$module" "$temp/com/blockworked/Blockstitch"
# The editor links the plugin statically; these controls only need its QML.
sed '/plugin /d; /classname /d; /prefer /d' "$module/qmldir" > "$temp/com/blockworked/Blockstitch/qmldir"
QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-offscreen}" \
QT_QUICK_BACKEND="${QT_QUICK_BACKEND:-software}" \
    "$runner" -import "$temp" -input "$root/blockloom-qt/tests/qml"
