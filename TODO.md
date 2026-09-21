# Long-term TODO

Obvious gaps already identified in the project notes:

- [ ] Add clones so blocks can create and manage copies of an actor.
- [ ] Add sound playback and sound-related blocks.
- [ ] Add lists and blocks for creating, reading, and changing list items.
- [x] Add asset management UI for importing, organizing, previewing, replacing, and removing project assets.
- [x] Show `say` as a speech bubble over its actor in the game world.
- [ ] Let reporter-shaped custom blocks suspend and resume when they contain `wait`.
- [x] Handle actors whose visual shape does not match the project's dimension, including a way to convert or replace the shape.
- [x] Add project packaging so a finished game can be shared and run independently.
- [x] Finish platform packaging with Windows executable icons, Linux launchers,
      macOS `.app` bundles, project-selected icon assets, and shareable ZIPs.
- [x] Build for platforms other than the one doing the building: stage a player
      payload per target under `players/<triple>/`, and let the Build dialog
      pick between the targets an install actually has one for.
- [x] Compile a project's blocks into optimized native logic for a fast build,
      with a shared variable store, a stable C boundary, a player-side native
      scheduler, and a VM fallback when compilation is unavailable.
- [ ] Compile recursive statement-shaped custom blocks. They are refused for
      now because each invocation needs its own loop-counter slots. Recursive
      reporter-shaped blocks already compile.
- [ ] Add a save-data system so a finished game can persist the player's progress across runs, with a block API and a matching Rust script API.
- [x] Redesign projects as folders: the app always starts on a Dashboard page for creating new projects and opening existing ones, with each project stored as a folder so it can hold assets.
- [x] Add a component system built on Bevy's ECS components that turns all properties into components, with support for custom components and camera-attach components (for first-person / third-person cameras).
- [x] Add a script component that runs Rust, so a project can drop out of blocks where it needs to.
- [x] Let blocks attach and detach whole components at runtime, not just write their fields.
- [ ] Ship the script toolchain, or degrade well without one: a script needs `rustc` on the machine that presses Play, and a packaged install can't assume it.
- [ ] Give the script editor real Rust editing - highlighting, and errors shown against the line they're on rather than only in the run log.
- [ ] Give Rust scripts a real rust-analyzer experience: syntax highlighting, completion, `export!` macro expansion, and go-to-source on the API, the way Unity hands Rider its project folder. Today a script is a bare textarea, and the `blockloom` crate it links against exists only in memory inside the build, so rust-analyzer has nothing to index.
      - Generate a `Cargo.toml` at the project root, next to `project.blockloom` and `assets/`. It is the Rust analog of the `.sln`/`.csproj` Unity generates for an IDE: the entry point an editor (VS Code, Zed, RustRover) opens, while the scripts stay in place under `assets/scripts/`. An "Open in editor" action just points at that folder.
      - The root `Cargo.toml` declares one target per script in `assets/scripts/*.rs`, plus a path dependency on the assembled `abi.rs` + `prelude.rs` kept as a `blockloom` crate under `.blockloom/`, so `use blockloom::*` and `export!` resolve against real source. Play keeps the fast direct-`rustc` compile, so Cargo is analysis-only; regenerate the root `Cargo.toml` whenever scripts or the ABI change to keep it in sync.
      - Later: feed `cargo check` output back into the editor so errors also show inline in Blockloom's own script editor, reusing the same project.
- [x] Add a shell command system with full control over the app, so an AI agent can create and edit projects in any way a user can, at the user's request.
- [ ] Add a Model Context Protocol (MCP) server so AI agents can use blockloom directly instead of shelling out through `blockloom-shell`. The shell's command registry is the natural surface to expose: each dispatch command becomes an MCP tool, so an agent can inspect, create, and edit a project to match a user's request. Probably a pnpm/Node-based `mcp` host running the existing `blockloom-shell` binary via stdio, or a Rust stdio server implementing the MCP protocol against the same `Backend::dispatch`. Needs: an initial `list projects`/`read project` tool for context, tool schemas derived from each `CommandSpec`'s known keys, per-command JSON responses that already come out of the shell, and coexistence rules so an MCP session and the editor window don't fight over one in-memory project (the same warning the shell already carries).
