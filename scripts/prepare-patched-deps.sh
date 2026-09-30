#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="${BLOCKLOOM_PATCHED_DEPS_DIR:-$root/.patched-deps}"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
mode="${1:-prepare}"
if [[ "$mode" != prepare && "$mode" != --check ]]; then
    echo "Usage: bash scripts/prepare-patched-deps.sh [--check]" >&2
    exit 2
fi

hash() {
    if command -v sha256sum >/dev/null; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}

mkdir -p "$output"
if ! mkdir "$output/.prepare-lock" 2>/dev/null; then
    echo "Dependency preparation is already running (lock: $output/.prepare-lock)." >&2
    exit 1
fi
work=""
cleanup() {
    if [[ -n "$work" ]]; then rm -rf "$work"; fi
    rmdir "$output/.prepare-lock"
}
trap cleanup EXIT

while read -r name version checksum directory; do
    [[ -z "$name" || "$name" == \#* ]] && continue
    patches=("$root/patches/$directory/"*.patch)
    fingerprint="$({
        printf '%s\n' "$name" "$version" "$checksum"
        cat "${BASH_SOURCE[0]}" "${patches[@]}"
    } | hash)"
    destination="$output/$directory"
    if [[ -f "$destination/Cargo.toml" && -f "$destination/.blockloom-patch-stamp" ]] &&
        [[ "$(cat "$destination/.blockloom-patch-stamp")" == "$fingerprint" ]]; then
        continue
    fi
    if [[ "$mode" == --check ]]; then
        echo "Patched dependency $name is missing or stale; run just prepare-patched-deps." >&2
        exit 1
    fi
    if [[ -e "$destination" && ! -f "$destination/.blockloom-patch-stamp" ]]; then
        echo "Refusing to replace unmanaged directory $destination." >&2
        exit 1
    fi

    work="$(mktemp -d "$output/.prepare.XXXXXX")"
    archive="$name-$version.crate"
    cached="$output/.archives/$archive"
    mkdir -p "$output/.archives"
    if [[ ! -f "$cached" ]]; then
        for candidate in "$cargo_home"/registry/cache/*/"$archive"; do
            if [[ -f "$candidate" ]]; then
                cp "$candidate" "$work/$archive"
                break
            fi
        done
        if [[ ! -f "$work/$archive" ]]; then
            curl --fail --location --retry 3 \
                "https://static.crates.io/crates/$name/$archive" -o "$work/$archive"
        fi
        if [[ "$(hash < "$work/$archive")" != "$checksum" ]]; then
            echo "Checksum mismatch for $archive." >&2
            exit 1
        fi
        mv "$work/$archive" "$cached"
    fi
    if [[ "$(hash < "$cached")" != "$checksum" ]]; then
        echo "Checksum mismatch for $cached; remove the damaged archive and retry." >&2
        exit 1
    fi
    tar -xzf "$cached" -C "$work"
    source="$work/$name-$version"
    for patch_file in "${patches[@]}"; do
        # Patch files use LF; normalize only the source files they change.
        while IFS= read -r file; do
            if [[ "$file" != src/* || "$file" == *..* ]]; then
                echo "Unexpected patch path: $file" >&2
                exit 1
            fi
            sed "s/$(printf '\r')$//" "$source/$file" > "$work/normalized"
            mv "$work/normalized" "$source/$file"
        done < <(sed -n 's|^--- a/||p' "$patch_file")
        patch -t -N -F 0 -p 1 -d "$source" -i "$patch_file"
    done
    printf '%s\n' "$fingerprint" > "$source/.blockloom-patch-stamp"
    if [[ -d "$destination" ]]; then mv "$destination" "$work/previous"; fi
    if ! mv "$source" "$destination"; then
        if [[ -d "$work/previous" ]]; then mv "$work/previous" "$destination"; fi
        exit 1
    fi
    rm -rf "$work"
    work=""
    echo "Prepared $name $version in $destination"
done < "$root/patches/dependencies.txt"
