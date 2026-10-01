# Patched Dependencies

Only diffs are committed here. Full crate sources are recreated in the ignored
`.patched-deps/` directory from pinned crates.io release archives.

```sh
just prepare-patched-deps
just build
```

Cargo recipes in the justfile prepare dependencies automatically, as does the
editor's Android runtime cross-build. Before running Cargo directly on a fresh
checkout, run `just prepare-patched-deps`. Preparation requires Bash, curl, tar,
patch, and either sha256sum or shasum. On Windows the justfile finds Git Bash
itself (PATH first, then the default install location).
Windows preparation also caches checksum-verified Vulkan 1.3.290 headers from
Khronos. Qt's native texture import needs headers, but no SDK library linkage.

`dependencies.txt` pins the crate name, version, archive SHA-256 and generated
directory name. Preparation first checks Cargo's archive cache, then downloads
missing archives from crates.io. Every archive is checksum-verified. Patch files
apply in filename order, with no fuzz; changed source files are normalized to LF.
Cargo's shared cache is never modified.

Unchanged inputs are a no-op. A changed patch, version, checksum or preparation
script recreates that crate in a temporary directory before replacing its old
generated copy. Failed downloads or patch applications leave the old copy intact.
Do not edit generated sources; edit the patches instead. Delete `.patched-deps/`
and run preparation again to recreate everything. Cached preparation works offline.
If a killed process leaves `.patched-deps/.prepare-lock`, remove that empty lock
directory only after confirming that no preparation process is still running.

```sh
bash scripts/prepare-patched-deps.sh --check
bash scripts/test-patched-deps.sh
```

`--check` fails when a generated crate is missing or its input stamp is stale.
The integration test uses isolated local archives and does not access the network.

## Preserved Workarounds

- wgpu-hal 30.0.1: descriptor allocation retries once in a fresh pool instead of
  panicking on pool exhaustion/fragmentation; Android/Mali command pools request
  resource release every 64 resets. The latter remains experimental.
- bevy_pbr 0.20.0-rc.2: Mali-only view and preprocessing bind-group reuse caches.

These are the changes from the former vendor copies, not new stability fixes.
See [the Android investigation](../docs/android-mali-debugging.md) for evidence
and limitations. To retire an override, remove its Cargo patch entry and its row
in `dependencies.txt`, then remove that crate's patch directory and update Cargo.lock.
