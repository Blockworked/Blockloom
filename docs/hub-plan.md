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
Python build tooling. The Qt shell can invoke its commands on worker processes
and consume JSON; package the service runtime with the Hub rather than requiring
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
the development slot. Optional tool downloads and graphical checkboxes are part
of milestone 3, not the initial CLI.
Directory components must declare a `version` in their manifest entries. Rust
still needs a working host linker: shipping must bundle or provision the matching
platform linker and libraries, especially MSVC on Windows, and test scripts on a
machine without an existing developer environment. Until process tracking is
implemented, close editors using a development slot before rebuilding it.

`bind` records a selection only. Existing projects without a selection are
shown as unassigned and must be explicitly assigned before `open`. The first
milestone does not offer project creation, backup/migration UI, automatic release
discovery, removal or a graphical Hub window yet.
