#!/usr/bin/env bash
set -euo pipefail
if [[ "$OSTYPE" == msys* || "$OSTYPE" == cygwin* ]]; then
    export PATH="/usr/bin:$PATH"
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
if [[ "$OSTYPE" == msys* || "$OSTYPE" == cygwin* ]]; then
    fixture="$(cygpath -u "$fixture")"
fi
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/patches/example" \
    "$fixture/source/example-1.0.0/src" "$fixture/cargo/registry/cache/test"
cp "$root/scripts/prepare-patched-deps.sh" "$fixture/scripts/"
printf '[package]\nname = "example"\nversion = "1.0.0"\n' > "$fixture/source/example-1.0.0/Cargo.toml"
printf 'pub const VALUE: u32 = 1;\r\n' > "$fixture/source/example-1.0.0/src/lib.rs"
tar -czf "$fixture/cargo/registry/cache/test/example-1.0.0.crate" \
    -C "$fixture/source" example-1.0.0
if command -v sha256sum >/dev/null; then
    checksum="$(sha256sum < "$fixture/cargo/registry/cache/test/example-1.0.0.crate" | awk '{print $1}')"
else
    checksum="$(shasum -a 256 < "$fixture/cargo/registry/cache/test/example-1.0.0.crate" | awk '{print $1}')"
fi
printf 'example 1.0.0 %s example\n' "$checksum" > "$fixture/patches/dependencies.txt"
export CARGO_HOME="$fixture/cargo"
export BLOCKLOOM_PATCHED_DEPS_DIR="$fixture/.patched-deps"
if [[ "$OSTYPE" == msys* || "$OSTYPE" == cygwin* ]]; then
    # Exercise the drive-letter paths supplied by Windows CI and Cargo.
    export CARGO_HOME="$(cygpath -w "$fixture/cargo")"
    export BLOCKLOOM_PATCHED_DEPS_DIR="$(cygpath -m "$fixture/.patched-deps")"
fi
prepare() { bash "$fixture/scripts/prepare-patched-deps.sh" "$@"; }
expect_failure() {
    if prepare "$@" > "$fixture/failure.log" 2>&1; then
        echo "Expected preparation to fail: $*" >&2
        exit 1
    fi
}
write_patch() {
    printf '%s\n' '--- a/src/lib.rs' '+++ b/src/lib.rs' '@@ -1 +1 @@' \
        '-pub const VALUE: u32 = 1;' "+pub const VALUE: u32 = $1;" \
        > "$fixture/patches/example/change.patch"
}

write_patch 2
expect_failure --check
prepare
grep -qx 'pub const VALUE: u32 = 2;' "$fixture/.patched-deps/example/src/lib.rs"
stamp="$(cat "$fixture/.patched-deps/example/.blockloom-patch-stamp")"
prepare --check
prepare
[[ "$(cat "$fixture/.patched-deps/example/.blockloom-patch-stamp")" == "$stamp" ]]

write_patch 3
expect_failure --check
prepare
grep -qx 'pub const VALUE: u32 = 3;' "$fixture/.patched-deps/example/src/lib.rs"
[[ "$(cat "$fixture/.patched-deps/example/.blockloom-patch-stamp")" != "$stamp" ]]

printf '%s\n' '--- a/src/lib.rs' '+++ b/src/lib.rs' '@@ -1 +1 @@' \
    '-missing context' '+bad replacement' > "$fixture/patches/example/change.patch"
expect_failure
grep -qx 'pub const VALUE: u32 = 3;' "$fixture/.patched-deps/example/src/lib.rs"

write_patch 4
printf 'damaged archive\n' > "$fixture/.patched-deps/.archives/example-1.0.0.crate"
expect_failure
grep -q 'Checksum mismatch' "$fixture/failure.log"
grep -qx 'pub const VALUE: u32 = 3;' "$fixture/.patched-deps/example/src/lib.rs"

rm "$fixture/.patched-deps/example/.blockloom-patch-stamp"
expect_failure
grep -q 'unmanaged directory' "$fixture/failure.log"
grep -qx 'pub const VALUE: u32 = 3;' "$fixture/.patched-deps/example/src/lib.rs"
echo 'Patched dependency integration tests passed.'
