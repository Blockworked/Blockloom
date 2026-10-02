# Blockloom Hub

## Product rules

- The Hub is a separate, lightweight app for Windows, Linux and macOS.
- Projects show their name, dimension, path, last opened time and selected editor.
- Each released version has exactly one installation in the Hub's user data
  directory. Install a new version alongside existing ones, never over them.
- Downloading an editor does not change any project's selected version.
- Changing a project's version is explicit. Keep the old editor available for
  other projects and offer a project backup before opening with a newer editor.
- Developer checkouts are separate named sources, keyed by canonical repo path.
  Each checkout has one mutable development installation. Its identity is the
  checkout, not its Cargo version, so multiple checkouts that say `0.0.1` do not
  masquerade as duplicate released versions.
- Developer Rebuild uses the same builds as `just replace`, but stages into the
  Hub. It never calls system install, uninstall, sudo or platform registration.
- Every release includes its pinned Rust toolchain, including Cargo and host
  standard libraries. Java, Android SDK, Android NDK and Android Rust targets
  are optional checkboxes at installation time. Versions and component downloads
  come from that release's manifest. No global toolchain installation is needed.
  SDK selection requires explicit license acceptance in the shipping installer.

## Architecture

Use a separate Qt Quick executable for the shipping desktop UI, sharing the
editor's visual style but not linking Bevy or starting the editor backend. Keep
installation operations behind an independently testable service. The first
milestone is a Python standard-library service and CLI, alongside the existing
Python build tooling. The Qt shell invokes its commands on worker processes
and consumes JSON; package the service runtime with the Hub rather than requiring
end users to install Python.

The editor's existing `blockloom/projects.json` remains the project list. The Hub
reads it without rewriting it or deleting missing projects. An imported project
can also be inspected and linked directly. Store the selection in the project's
`.blockloom/hub.json`, so renaming its folder preserves the selection. This file
does not change the project document or trigger migration.

Default Hub root: `%LOCALAPPDATA%/Blockloom/Hub` on Windows,
`~/Library/Application Support/Blockloom/Hub` on macOS and
`${XDG_DATA_HOME:-~/.local/share}/blockloom-hub` on Linux.
`BLOCKLOOM_HUB_DIR` overrides it. Editor project discovery honors the existing
`BLOCKLOOM_DATA_DIR` setting separately.

Layout:

```text
Hub/
  installations/
    release-0.1.0/
      blockloom-installation.json
      blockloom[.exe]
      blockloom-runtime[.exe]
      players/
      tools/rust/
      tools/java/          (optional)
      tools/android-sdk/   (optional)
      tools/android-ndk/   (optional)
    dev-<canonical-repo-hash>/
      blockloom-installation.json
      ...
  .operation-lock/
```

Release payloads have a manifest with a version and host target. They must be
portable bundles with the editor, runtime, players and Qt dependencies. Publish
one bundle per version and target. Copy into a temporary sibling, validate, then
rename into place. Refuse an existing release slot. Serialize mutations across
CLI and UI processes. Refuse corrupt manifests rather than silently resetting
state. Failed builds leave the previous development installation intact.

## Milestones

1. **Installation foundation (this change).** Project discovery and metadata,
   explicit binding, installation listing, portable directory import, developer
   repo registration/rebuild, atomic staging and tests. Add
   `BLOCKLOOM_INSTALL_DIR` to `just replace` to bypass the system installer.
   Add `--project <folder>` startup support to the editor for Hub launch.
2. **Desktop shell.** Separate Qt/QML Hub target with Projects and Installations
   tabs, search, import/create/open actions, explicit version chooser, repo folder
   picker and Rebuild button. Run builds asynchronously with progress, persistent
   logs, cancellation and process-tree cleanup. Track running editors and refuse
   replacing/removing their installations. Protect projects with active owners.
3. **Release updates.** Configure an authoritative HTTPS release catalog with
   version, channel, supported targets, archive URL, byte count and SHA-256.
   Checking reports available versions; installation remains explicit. Verify
   hashes, bound download/extraction size, reject archive traversal and links,
   support retry/cancellation and preserve working installs on failure. Feed URL
   and release credentials are release infrastructure decisions, not guessed here.
   Bundle Rust by default, and show Java, SDK, NDK and Android Rust target
   checkboxes with sizes, pinned versions and dependencies. Allow adding optional
   tools later to the same editor slot without replacing its editor binaries.
   Launch uses the slot's Rust and Java on PATH and its SDK/NDK paths through
   Hub-specific overrides, preserving the user's Android settings. Android license
   acceptance stays explicit. NDK and Android Rust targets are both needed for
   native Android compilation; Java and SDK are needed for APK tooling.
4. **Project upgrade workflow.** Explicit selection with backup and compatibility
   checks. Changing the binding selects an editor; the chosen editor performs any
   format migration on open. Do not imply that switching back reverses migration.
   Never edit a project in use. Uninstall refuses versions used by projects or
   running editors; missing/removable-drive projects remain visible.
5. **Shipping.** Portable Qt deployment for all three platforms, macOS signing
   and notarization, Windows signing, OS launchers and file associations. Package
   Hub updates separately from editor installations. CI exercises clean install,
   failed download/build, concurrent operations, project folder moves, paths with
   spaces and Unicode, and both CPU architectures where supported.

## Running the first milestone

Python 3.11 or newer is required for the development CLI. It does not build the
editor just to inspect projects. Run `just hub --help` or
`python scripts/hub.py --help` (`python3` on Unix).

```text
just hub projects
just hub installations
just hub add-dev "path/to/blockloom"
just hub rebuild dev-<id>
just hub install "path/to/portable-bundle"
just hub install "path/to/portable-bundle" --java --android-sdk --android-ndk --android-rust-targets
just hub bind "path/to/project" release-0.1.0
just hub open "path/to/project"
```

Release import currently accepts unpacked portable directories, not network
downloads. The manifest must contain `kind: "release"`, `version: "0.1.0"`
and `target` matching the host triple used by `scripts/replace.py`. It must have
the editor and runtime at its root. This milestone does not deploy Qt libraries;
the bundle producer must include them. Development builds currently preserve the
same dependency requirements as running `target/release/blockloom` directly.
The bundle must also include `tools/rust/bin/{rustc,cargo}` and host standard
libraries under `tools/rust/lib/rustlib/<host>/lib`, with a manifest entry such as
`"tools": {"rust": {"channel": "1.98.1"}}`. Optional component names are `java`,
`android-sdk`, `android-ndk` and `android-rust-targets` (the latter lists
`aarch64-linux-android` and `x86_64-linux-android`). Selected directory components
live under `tools/<name>`. Unselected components are omitted from the imported
installation. Developer rebuilds copy the checkout's pinned rustup toolchain into
the development slot. Development builds first prepare a candidate, then open
an installation options dialog. The desktop window provides graphical checkboxes for
bundle import. Optional tool downloads remain part of milestone 3.
Directory components must declare a `version` in their manifest entries. Rust
still needs a working host linker: shipping must bundle or provision the matching
platform linker and libraries, especially MSVC on Windows, and test scripts on a
machine without an existing developer environment.

`bind` records a selection only. Existing projects without a selection are
shown as unassigned and must be explicitly assigned before `open`. The first
milestone does not offer project creation, backup/migration UI, automatic release
discovery or removal.

## Desktop shell started

`blockloom-hub` is a separate Qt Quick workspace member. It does not link the
editor backend or Bevy. Run `just hub-run` to build the workspace and open it,
or run `target/release/blockloom-hub` directly (`.exe` on Windows) after building.
The window provides searchable projects, explicit editor selection and launch,
prepared release bundle import with optional tool checkboxes, local repository
registration and a Build/Rebuild button. Builds use an asynchronous worker
process with a bounded, dark operation log, selectable text, wrapping and optional
following of output. Automatic refresh preserves that log. Build stdout/stderr
are also saved to `Hub/logs/<installation>-build.log`.

Adding a project remembers its folder in the Hub's own `projects.json`, merged
with the editor registry when listing. Canonical paths prevent duplicate rows;
the editor registry remains read-only. The service sources are embedded in the
Hub executable and extracted to a temporary directory for worker imports.
Development requires Python 3.11+ on PATH, or `BLOCKLOOM_HUB_PYTHON` pointing to
its executable. A packaged runtime at `python/python.exe` (Windows) or
`python/bin/python3` (Unix), next to the Hub executable, takes precedence over
PATH. Runtime packaging is still shipping work.

The window refuses ordinary close requests while a worker is running. A Cancel
button requests cooperative worker cancellation; the worker stops its child
build tree and exits through normal staging and lock cleanup. Windows assigns a
waiting bootstrap to a Job Object before permitting it to spawn build children;
Unix checks owned descendants, including replacement builds' separate sessions.
Cancellation during tool copying or staging stops before installation promotion.
The atomic promotion itself finishes once started.

Hub launches are recorded in `running.json` with process birth identifiers, so
stale PID records do not mistake a reused PID for the same editor. Live project
owner locks also block editor selection, repeat launch and development replacement.
The window refreshes usage indicators while idle. This is best-effort coordination
with externally launched editors, whose own locks do not share the Hub's operation
lock. Editor stdout/stderr go to `Hub/logs/<installation>-editor.log`
so they cannot corrupt the service's JSON responses.

The development Build button uses `prepare-dev`: it keeps the current installation
and saves a candidate under `Hub/pending/<installation>`. When the build finishes,
the Hub opens Install development build. Rust is required; Java, SDK and NDK can
be copied from local folders chosen in the dialog. Saved editor Android paths and
`JAVA_HOME` provide initial folder suggestions. These folders must contain
version metadata and required binaries. SDK copying excludes its NDK folder so
the NDK checkbox remains independent. Missing Android Rust targets are fetched
from the exact pinned Rust distribution into the private installation candidate,
with official manifest and component checksum validation. Cancelling the options dialog leaves the current installation
intact; Install prepared build reopens it without rebuilding. `install-dev`
performs the final validated transaction. The dialog's build ID rejects a candidate
changed by another Hub between preparation and installation.

`rebuild` remains a CLI convenience that prepares and installs with Rust only.
Hub replacement builds exclude the companion Hub target to avoid relinking the
running launcher on Windows; they still build the editor, runtime and players.

Validation: `python scripts/test-hub.py` covers the installation service;
`python scripts/test-hub-ui.py` runs the built Hub offscreen against isolated
fixtures and captures Projects, Installations, tool selection and editor
selection under `target/hub-ui-smoke`. Windows smoke tests load an OS font
explicitly because the offscreen plugin does not discover native fonts.
The UI smoke test also selects a release and verifies its saved project binding.
It checks the operation log, prepared-build options and development installation.
`python scripts/test-hub-process.py` checks real child-tree cancellation and
process birth identity; service tests cover active-owner refusal, stale locks,
optional tool selection and failed/cancelled installation preservation.

## Downloads and build workflows

Get a release supports HTTPS catalogs and a persisted GitHub CLI source setting,
default repository `Blockworked/Blockloom`. GitHub CLI uses the existing
`gh auth login` session to list the latest 100 releases and retrieve each release's
`blockloom-catalog.json`. Downloads use `gh release download` for private assets;
the Hub never reads or saves the login token. Draft releases are hidden.
Rechecking a release validates the selected checksum again before downloading.

Catalog schema 1 contains a `releases` array. Each entry declares `version`,
Rust host `target`, HTTPS `url`, hex `sha256`, compressed `size`, `unpacked_size`,
and `format` (`zip`, `tar.gz`, or `tar.xz`). Optional `tools` drives available
checkboxes. GitHub catalogs also declare the release asset basename in `asset`.
Archive extraction rejects links, special files, escaping paths and duplicate
paths, checks size/member limits and preserves executable permissions. Successful
downloads use the same immutable installation rules as local bundle import.

Rust preparation copies compiler binaries, libraries, linker helpers and config,
skipping the tens of thousands of HTML documentation files and Rust sources.
Progress reports copied files and MiB. Prepared candidates and final private
payloads are promoted by rename instead of copying the entire installation again.
The candidate is retained during final installation for retry after failures.

`build-artifacts.yml` and `release.yml` share `build-packages.yml`. Each platform
builds the editor and web player concurrently using `replace.run_builds`. Shipping
also builds the native player concurrently with `dist`; web uses `dist` too.
Dependencies and wasm-bindgen are fetched before compilation, then builds run
offline. CI never performs a system installation. Windows, Linux and macOS jobs
run independently. Artifacts include Qt and the pinned Rust toolchain, including
wasm and optional Android targets. The Hub archive requires Python 3.11+ on PATH.
Windows builds use the `windows-2025-vs2026` runner and select Visual Studio 18.0
(2026) explicitly. Qt's prebuilt kit is named `win64_msvc2022_64`.
Linux packages require Ubuntu 24.04-compatible system libraries; macOS packages
target macOS 27 on GitHub's `xcode-27` runner and are unsigned for distribution
(ad hoc signing only). Broader platform baselines,
developer signing and a bundled Hub Python runtime remain shipping work.

The release workflow validates its existing tag against the workspace version,
waits for all platform packages, verifies their archive checksums and merges their
catalogs. It creates a draft, uploads complete assets, then publishes. Prerelease
tags are marked as prereleases. An interrupted publish can leave a draft for
manual cleanup; it does not overwrite an existing release.

Protocol references: [GitHub CLI release downloads](https://cli.github.com/manual/gh_release_download)
and [Rust distribution manifests](https://forge.rust-lang.org/infra/channel-layout.html).
`test-hub-download.py`, `test-hub-github.py` and `test-package-release.py` use
controlled fixtures to verify downloads, private release discovery, archives and
CI profile selection. Real hosted workflow execution remains to be verified.
