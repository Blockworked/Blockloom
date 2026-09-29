#!/usr/bin/env bash
# Headless check of an Android APK beside `web-smoke`: installs on the
# connected device (or the serial in $3), launches, waits for the world-built
# marker plus a second actor snapshot in logcat, and fails on any Rust panic.
# An emulator counts as a device. Usage: android-smoke.sh <apk> <app-id> [serial].
set -euo pipefail

APK="${1:?usage: android-smoke.sh <apk> <applicationId> [serial]}"
APP="$2"
SERIAL="${3:-}"
SHELL_BIN="${CARGO_TARGET_DIR:-target}/release/blockloom-shell"

cargo build --release -p blockloom-app --bin blockloom-shell >/dev/null

if [ -n "$SERIAL" ]; then
  INSTALL_EVAL="android-install apk=\"$APK\" app=\"$APP\" device=\"$SERIAL\""
  LOGCAT_EVAL="android-logcat device=\"$SERIAL\""
else
  INSTALL_EVAL="android-install apk=\"$APK\" app=\"$APP\""
  LOGCAT_EVAL="android-logcat"
fi

echo "installing $APK and launching $APP..."
"$SHELL_BIN" --eval "$INSTALL_EVAL" --no-state | python3 -c "import json,sys; r=json.load(sys.stdin); sys.exit(0 if r['ok'] else 1)"

# The world needs a few seconds: install, first frame, warmup, green flag.
sleep 12

echo "reading logcat..."
OUT="$("$SHELL_BIN" --eval "$LOGCAT_EVAL" --no-state)"
echo "$OUT" | python3 -c "
import json, sys
r = json.load(sys.stdin)
if not r['ok']:
    print('logcat failed:', r.get('error'))
    sys.exit(1)
result = r['result']
panics = result.get('panics', [])
if panics:
    print('RUST PANICS on device:')
    print('\n'.join(panics))
    sys.exit(1)
lines = result.get('lines', [])
started = [l for l in lines if 'run started' in l]
snaps = [l for l in lines if 'actors {' in l]
print(f'markers: {len(started)} run-started, {len(snaps)} actor snapshots')
if not started:
    print('no world-built marker in logcat; full dump:')
    print('\n'.join(lines))
    sys.exit(1)
if len(snaps) < 2:
    print('only one actor snapshot so far; the world built but movement is unproven:')
    print('\n'.join(lines))
    sys.exit(1)
first, second = snaps[0], snaps[-1]
print('first :', first[-220:])
print('second:', second[-220:])
if first.split('actors ', 1)[1] == second.split('actors ', 1)[1]:
    print('note: snapshots identical (a still scene reads the same twice)')
print('android-smoke: PASS')
"
