# Long-term TODO

Obvious gaps already identified in the project notes:

- [x] Add clones so blocks can create and manage copies of an actor.
- [x] Let blocks and scripts create brand-new actors from scratch and delete existing actors during runtime, not just clone what the editor authored.
- [x] Implement child actors: the parent/child hierarchy most game engines have, so actors can be attached to each other and move together.
- [x] Compile the actor-lifetime blocks, so a project that clones, creates or
      deletes actors can still ship native logic.
- [x] Give a child actor an authored local offset, so the inspector can place
      one relative to its parent rather than in world coordinates.
- [x] Implement the parent/child hierarchy in the editor actor list, so actors
      can be dragged under other actors to reparent (and dragged out to
      unparent, with cycle protection), and child lists can be collapsed per
      parent.
- [x] Let a child be asked about its place in its parent's frame, and let
      `set my parent to` place it there rather than leaving it where it
      stands. A child with an authored offset is put at it, in the parent's
      own frame; `my local` / `<actor>'s local` reporters answer it live.
- [x] Add sound playback and sound-related blocks.
- [x] Add lists and blocks for creating, reading, and changing list items.
- [x] Add asset management UI for importing, organizing, previewing, replacing, and removing project assets.
- [x] Show `say` as a speech bubble over its actor in the game world.
- [x] Let reporter-shaped custom blocks suspend and resume when they contain `wait`.
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
- [x] Compile recursive statement-shaped custom blocks. Each invocation keeps
      its own loop-counter slots now, the way the VM gives every call a frame.
- [x] Add a save-data system so a finished game can persist the player's progress across runs, with a block API and a matching Rust script API.
- [x] Redesign projects as folders: the app always starts on a Dashboard page for creating new projects and opening existing ones, with each project stored as a folder so it can hold assets.
- [x] Add a component system built on Bevy's ECS components that turns all properties into components, with support for custom components and camera-attach components (for first-person / third-person cameras).
- [x] Add a script component that runs Rust, so a project can drop out of blocks where it needs to.
- [x] Let blocks attach and detach whole components at runtime, not just write their fields.
- [x] Ship the script toolchain, or degrade well without one: a script needs `rustc` on the machine that presses Play, and a packaged install can't assume it.
- [x] Give the script editor real Rust editing - highlighting, and errors shown against the line they're on rather than only in the run log.
- [x] Give Rust scripts a real rust-analyzer experience: syntax highlighting, completion, `export!` macro expansion, and go-to-source on the API, the way Unity hands Rider its project folder. Today a script is a bare textarea, and the `blockloom` crate it links against exists only in memory inside the build, so rust-analyzer has nothing to index.
      - [x] Generate a `Cargo.toml` at the project root, next to `project.blockloom` and `assets/`. It is the Rust analog of the `.sln`/`.csproj` Unity generates for an IDE: the entry point an editor (VS Code, Zed, RustRover) opens, while the scripts stay in place under `assets/scripts/`. An "Open in editor" action just points at that folder.
      - [x] The root `Cargo.toml` declares one target per script in `assets/scripts/*.rs`, plus a path dependency on the assembled `abi.rs` + `prelude.rs` kept as a `blockloom` crate under `.blockloom/`, so `use blockloom::*` and `export!` resolve against real source. Play keeps the fast direct-`rustc` compile, so Cargo is analysis-only; regenerate the root `Cargo.toml` whenever scripts or the ABI change to keep it in sync.
      - [x] Later: feed `cargo check` output back into the editor so errors also show inline in Blockloom's own script editor, reusing the same project.
- [x] Add a shell command system with full control over the app, so an AI agent can create and edit projects in any way a user can, at the user's request.
- [x] Add a Model Context Protocol (MCP) server so AI agents can use blockloom directly instead of shelling out through `blockloom-shell`. The shell's command registry is the natural surface to expose: each dispatch command becomes an MCP tool, so an agent can inspect, create, and edit a project to match a user's request. The `mcp/` package is a pnpm/Node host running the existing `blockloom-shell` binary via stdio, with tool schemas derived from `blockloom-shell --specs`, a `block-vocabulary` command for the block palette, and `blockloom://state` and `blockloom://blocks` resources. Each session owns one backend, so an MCP session and the editor window don't fight over one in-memory project.

## Engine parity roadmap (Godot/Unity order)

Phased by dependency and value per cost. Each phase unblocks the next.

### Phase 1 - Unblock real games, low risk
- [x] Lists plus dicts plus JSON: VM and compiled both, everything else needs data structures.
- [x] Sound playback plus buses plus 2D/3D positional: isolated, huge completeness win.
- [x] Parent-space runtime API (`set parent`, local vs world query): finishes hierarchy work already started.
- [x] Physics queries: raycast/shapecast, layers/masks UI, trigger vs solid: required for platformers, AI, UI clicks.

### Phase 2 - Ship a complete single-player game
- [x] Input actions plus remapping, gamepad/rumble, touch/multitouch, mouse lock.
- [ ] Tweens plus sprite animation plus animation player/state machine.
- [ ] In-game UI: button/label/bar/slider/input, anchors/layout, HUD/menus.
- [ ] Save slots/profiles plus localization: builds on save system we have.

### Phase 3 - Dev productivity, before API surface explodes
- [x] Script toolchain: ship rustc or graceful degrade plus highlight plus inline errors plus rust-analyzer Cargo project.
- [x] VM/codegen correctness: suspendable reporter `wait`, recursive statement blocks.
- [x] Embedded preview: sidecar MJPEG runtime plus streamed viewport in editor with input forwarding, pause/step, resolution switch. Keeps separate-process split, no OS reparenting. Additive: the OS window stays up beside the viewport.
  - [x] True headless/offscreen preview mode: a Headless toggle hides the OS window while the hidden window keeps rendering the stream. Windowed mode still keeps it up beside the viewport.
  - [ ] Stop the resolution switch from resizing the OS window: render the stream at its own size offscreen.
  - [ ] Forward scroll-wheel, touch/multitouch and gamepad through the viewport, not just mouse, keys and text.
  - [ ] Honor `lock mouse` inside the preview (pointer lock + raw deltas) - done for the in-process Game view, pending a test; the MJPEG preview still only gets absolute positions.
  - [ ] Adaptive stream rate/quality: fixed ~15fps JPEG-60 today regardless of preset or pause state.
- [ ] Visual world editor: edit-mode 2D/3D viewport with selection sync to ActorList/Inspector, drag to move plus rotate/scale gizmos, snapping, camera pan/zoom/orbit. Shares panel with embedded preview: Edit manipulates placement directly, Play streams runtime.
- [ ] Editor: gizmos/snapping, prefab mode, scene search, log filter, frame stepper, profiler (draw calls, CPU/GPU/memory), playmode tests.

### Phase 4 - Look and depth, uses Bevy leverage
- [x] Asset pipeline: glTF/FBX rigs, atlases, texture/audio compression, reimport tracking.
- [x] Materials/custom WGSL plus shader graph lite, particles/trails, post-process, shadows/HDR, 2D sorting layers, tilemap/terrain.
- [ ] Load glTF scenes for Model looks (a ModelSource loader with rig playback from the parsed animations) instead of placeholder boxes.
- [ ] Bake atlas layouts into sheets at build time - pack_atlas is plan-only today - and let a tilemap animate tiles and collide per-tile rather than as one slab.
- [ ] Close the custom-shader loop: export a shader graph to a .wgsl asset, and let hand-authored WGSL drive the live material instead of only the uniform path.
- [ ] Particle/trail blocks (burst, emitter dials) once block surface reopens, plus ghost trails for custom-shaded and tilemap actors, which leave none today.
- [ ] Advanced physics: joints, character controller, one-way platforms, ragdoll.
- [ ] AI: full nav on top of existing baked polyanya mesh (`navigate to`): runtime rebake, layers/costs, off-mesh links, crowds/separation, plus steering, behavior trees, perception.

### Phase 5 - Scale and ecosystem, do last
- [ ] Performance: batching/instancing, LOD/occlusion, async loading, world streaming.
- [ ] Multiplayer: headless server, replication, lobbies, rollback.
- [ ] Deploy: Web/WASM, Android/iOS signing, console path, auto-updater/DLC/addressables.
- [ ] Ecosystem: analytics/crash, achievements/IAP hooks, plugin API, asset store, collab/VCS, docs/LTS.

### Qt6 rewrite - in-process Game view
- [x] Goal: docked Game view with no sidecar video and no extra OS window.
- [x] Step 2 - headless world thread (true in-process, Linux): Bevy runs windowless on its own thread of the editor, `RuntimeHandle` talks to it over channels. A panic ends the run, not the editor.
- [x] Step 3 - GPU texture sharing (Linux): the camera renders offscreen and each frame lands in a ring of dma-bufs, imported into Qt's GL through EGL. Wayland and X11.
- [ ] Step 4 - input parity:
  - [x] Keys, mouse buttons and position, text, focus.
  - [ ] Pointer lock plus raw deltas (Wayland pointer constraints, X11 warp fallback) - built, awaiting a test on KDE. X11 path untested.
  - [ ] Scroll wheel and touch.
  - [ ] Gamepads: probably already read straight from the system in-process - verify.
  - [ ] Keyboard by physical key, not Qt key name (non-QWERTY layouts land WASD elsewhere). Right Shift/Ctrl/Alt arrive as left; numpad, brackets, quote and backtick aren't sent.
- [ ] Update CLAUDE.md: it still describes the world as a separate process with its own window.
- [ ] Frame pacing: the world runs on its own 60 Hz timer, not the display's, so 120/144 Hz screens get uneven frames. Tie it to Qt's frame signal.
- [ ] Log stutter: every `say`/error line still emits the whole editor state to QML. Give log lines their own event like status has.
- [ ] Zero-copy on NVIDIA: frames are copied to a linear system-memory image, then again through an external-texture pass on the Qt side. Share the image in its native tiled layout (DRM format modifiers) to drop both copies.
- [ ] Resolution: fixed 960x720 scaled to fit - blurry on HiDPI, wrong for widescreen. Add a resolution/aspect setting in Project Settings, or render at the view's real pixel size.
- [ ] Other platforms: Windows (shared D3D or Vulkan handles) and macOS (IOSurface) still use the child process plus MJPEG, which stays until they're ported.
- [ ] Crash isolation: a native crash (script cdylib, GPU fault) takes the editor down. Decide whether scripted projects should keep the separate process.

Rule: do 1-4 before 5-8, do 9-11 before adding new block surface in 12-15, leave 17-19 until single-player shipping loop is solid.
