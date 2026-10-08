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
- [ ] Multi-language script guests (see `docs/script-plan.md` Phase 5): Rust and C guests are in and tested against the sandbox, and a desktop-only wasmtime component host (WIT 0.2.0) runs Python components. Python, JavaScript and Go components run. Open: C# (needs the .NET SDK) and Kotlin (needs a canonical-ABI binding generator for Kotlin/Wasm) guests, and the ship-a-toolchain question for packaged installs (`rustfmt`/`clippy`/`clang` are not bundled).
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
- [x] Tweens plus sprite animation plus animation player/state machine.
- [x] In-game UI framework (UMG/UI-Toolkit grade, builds on the 7 widgets we have):
  - [x] Layout engine: measure/arrange pass with desired-size bubbling, containers
        (vertical/horizontal box, grid, overlay/canvas, scroll box, wrap box, size
        box, spacer), padding/margin, 9-point anchors plus offsets, fill/align
        policies. Retained mode with dirty-flag reflow plus batched draws over a
        shared atlas, so static HUDs cost nothing per frame.
  - [x] Widgets: progress bars (linear and radial), virtualized list view with row
        recycling for long inventories, tabs, dropdown/select, scrollbar, tooltip,
        rich-text label (markup, wrapping, auto-size), world-space widgets
        (nameplates, prompts projected from 3D actors).
  - [x] Style and themes: stylesheet assets over the existing three global themes,
        state styles (normal/hover/pressed/disabled/focused), font fallback chain,
        borders/shadows/rounded corners, per-element overrides kept.
  - [x] Data binding: bind widget text/value/visibility to variables and custom
        components with converters, one-way plus two-way (inputs write back), so
        HUDs update without per-frame rebuild blocks.
  - [x] Events and navigation: click/press/hover/drag/scroll events with bubbling,
        gamepad directional focus navigation plus focus memory, touch and wheel
        routing, existing modal and focus rules kept.
  - [x] Polish: hover/press transitions and tweens on UI properties, show/hide
        animations, screen-safe areas, DPI and canvas scaling (scale-with-size vs
        constant-pixel), resolution-independent sizes.
  - [x] Editor: visual UI designer (drag widgets onto a canvas, live resolution
        previews, hierarchy outliner, style inspector), UI prefabs for
        menus/dialogs shared across scenes.
  - [x] Blocks and scripts: `show/hide/delete` kept, plus `bind _ to _`, `set items
        of _ to`, `scroll _ to`, `set theme of _ to`, reporters (`value of`,
        `is shown?` kept) plus `selected index of _`. Fixed-tick sampling like
        other reporters so VM and codegen agree.
  Implementation and usage: [Interface guide](docs/interface.md).
- [x] Save slots/profiles plus localization: builds on save system we have.
  Done: named save slots (`switch save slot to`, `delete save slot`,
  `save slot`/`save slots` reporters) over `save::slot_path` (the default
  slot keeps the legacy file, the rest sit beside it; same shape on web
  localStorage and Android), and a project string table (`set language
  to`, `language`, `text for key` with default-language fallback; shell
  `set-locale`/`remove-locale`/`remove-language`/`set-default-language`/
  `list-locales`, `save-slots`/`delete-save-slot`). VM and compiled logic
  held together in `tests/codegen.rs`; scripts drive both through ABI 47
  (`switch_save_slot`, `save_slot`/`save_slots`, `set_language`,
  `language`, `text_for`), and there is no settings UI or QML run here.
- [x] Multiple scenes plus loading between scenes (menu, level 1, level 2):
  - [x] Document: `Project` holds a scene list (each with its own actors and
        `World` settings); one active scene; old single-scene docs migrate as
        scene one; scene add/rename/duplicate/delete with undo.
        Done: `Scene { id, name, world, actors }` in `project.rs`, `Project`
        holds `scenes` plus `active_scene` with compat `world`/`actors` in the
        JSON so old files and the current QML keep working. `Deref` to the
        active scene keeps existing `project.world`/`project.actors` code
        compiling; globals, input actions, repoint and normalize run across
        all scenes.
  - [x] Mode: v1 supports mixed 2D/3D scenes; `Mode` lives per scene and a
        scene switch across dimensions rebuilds the dim2/dim3 pipeline plus
        rapier backend the way a project dimension switch does today.
        Done: each scene has its own `World` (including `mode`), `Scene` and
        active-scene `switch_mode` convert units and visuals, and both
        dimensions' pipelines register at launch with the live `Dimension`
        following the active scene - so `set_active_scene`, `set_mode` and
        `set_scene_component` reload instead of respawning. Builds bake skies,
        atlases, probes, scripts and terrain from all scenes.
  - [x] Blocks and scripts: `switch scene to _` (plus `with transition _`),
        reporters `current scene`, `scene names`, events `when scene
        starts/ends`; globals plus save data cross scenes, actor locals do
        not; opt-in survivors later; fixed-tick sampling so VM and codegen
        agree, with parity cases in `tests/codegen.rs`.
        Done: `SwitchScene { scene, transition }` (`none`/`fade`/`wipe`/
        `circle`, unknown reads as `none`) ends its strand like `delete
        myself`; `WhenSceneStarts`/`WhenSceneEnds` headers with matching
        `Trigger`/`Event` pairs; `CurrentScene`/`SceneNames` sensing
        reporters (names as a JSON list, like `active volumes`) sampled from
        `Sensors::current_scene`/`scene_names`; scripts get `switch_scene`,
        `current_scene`, `scene_names` plus `SceneStarted`/`SceneEnded`
        events (ABI 32, LOGIC_ABI 29); `Vm::load_scene` plus
        `Variables`/`Lists`/`Dicts::load_scene` keep globals and shared
        collections while resetting actor locals; parity cases in
        `tests/codegen.rs` (`switch-scene`, reporter rows) and `tests/vm.rs`.
        Native logic carries every scene: `compile` emits one function per
        actor numbered across scenes plus a `NAMES_n`/`ENTRIES_n` table per
        scene under one `SCENES` index, and `Runner` runs the active table -
        `new(SCENES, ACTIVE_SCENE)`, `load_scene(id)` swapping the way
        `Vm::load_scene` does (ABI 29, `blockloom_logic_scene`). The host
        switches the program instead of dropping it, falling back to the VM
        for a stale build only.
        Done: opt-in survivors via a `Persist` component (`components.rs`,
        `BUILT_IN_NAMES`, inspector card, attach/detach palette): authored or
        runtime-attached carriers survive with live placement, variables,
        lists, dicts and attached components, while clones still die with the
        old scene. `Vm`/`Variables`/`Lists`/`Dicts::load_scene_keep` preserve
        survivor scopes, programs, names and running strands; the host carries
        survivor docs in `spawned` (`survivor_keep` through the rebuild, which
        respawns and converts them across dimensions) and reseeds their
        attached/parents/filters. A switch carrying survivors drops native
        logic to the VM (the runner holds one active table), so survivors run
        correct before they run native; parity case
        `survivors_keep_live_locals_lists_and_dicts_across_scenes` in
        `tests/vm.rs` plus `persist_marks_its_actor_as_a_scene_survivor`.
  - [x] Runtime: unload the current world, load the scene doc the way
        `EditorMessage::Load` does now, rebuild and warm up before the green
        flag continues; transitions run on the wall clock like UI strands;
        rooms stay intra-scene camera zones, not scenes.
        Done: switches unload non-survivors (spawned actors, clones, speech,
        touches), set the new active scene, reload the VM with globals kept
        (`load_scene_keep` for survivors), flag a rebuild (which reopens
        scripts including survivors', reseeds the level and reopens the
        warmup window) and fire `SceneStarted` after firing `SceneEnded` two
        ticks earlier so ended strands run first; unknown scenes report, the
        requesting strand ends, and the first ask per tick wins (blocks and
        scripts share the path). Rooms reseed per scene and stay intra-scene.
        Named transitions run on the wall clock: `Engine.veil` covers first
        (`transition.rs` - fade as an alpha wash, wipe as a left-to-right
        panel, circle as a centered iris disc growing to cover and shrinking
        to reveal), the swap waits for cover, and the reveal holds until the
        rebuild's warmup closes. `none` (and typos) keep the immediate cut.
        Cross-dimension switches rebuild live: both rapier plugins and both
        pipelines register at launch (gated `is_2d`/`is_3d` chains, shared
        systems run once), `rebuild_world` syncs the live `Dimension` plus
        audio scale from the new scene and converts survivor visuals, so
        editor, player and web all swap 2D/3D mid-run with no fresh process.
  - [x] Editor and tooling: scene picker plus per-scene actor list/canvas and
        World settings; project folder, pack, build and web carry all scenes;
        shell/MCP commands (`add-scene`, `switch-scene`, ...).
        Done: pack, build and web already carry all scenes (the `Project`
        JSON holds them, builds collect assets from every scene), and the
        shell/MCP has `add-scene`, `duplicate-scene`, `rename-scene`,
        `remove-scene`, `set-active-scene` (`switch-scene` alias),
        `set-default-scene` plus `scene-components`, `set-scene-component`,
        `remove-scene-component` and `import-scene`. The block palette
        (`Blocks.qml` rows, `vocabulary.rs` specs) now carries `switch scene
        to`, `when scene starts/ends`, `current scene` and `scene names`, plus
        `Persist` in attach/detach. Scenes live in the asset tray like normal
        files: the tray's New scene item makes one where listed, the filename
        is the scene name (renaming the file renames the scene, deleting it
        deletes the scene with undo), double-click or Open scene loads it, and
        project settings names the default scene a fresh open - and a built
        game - boots into plus which scene is open (World rows edit that
        scene). The actor list keeps a scene box showing which scene is open
        (no add button); per-scene canvas routing and World settings editing
        go through the active scene's compat fields, which is what makes them
        per-scene (switching scenes clears selection and swaps the canvas).
        The inspector adds a `Persist` card; dimension changes reload instead
        of respawning.
  - [x] Scene assets (Unity-style): each scene becomes its own asset file
        under the project folder (one file per scene, referenced by the
        project), so scenes can be shared, duplicated and versioned like any
        other asset. Each scene asset carries its settings as components on
        the scene itself - lighting, sky, fog, wind, water, post and physics
        settings become component records rather than fixed `World` fields -
        with the inspector editing the scene's components the way it edits
        an actor's. Loading a scene loads its asset; the in-document scene
        list becomes an index over scene assets with migration for old files.
        Done: name-based scene files (`assets/scenes/<name>.blockscene`,
        `SCENE_EXTENSION`, `SCENES_DIR`), `project.blockloom` as a
        `ProjectFile` index over `SceneRef`s with old embedded folders and
        id-named files migrating on load; `Project.default_scene` naming the
        boot scene (fresh opens and built players start there); `SceneFile`
        carries `SceneComponents`
        (Dimension, Background, Physics, Camera, SpeechBubble, Lighting,
        Sound, Input, Post, Display, Navigation, Sky, Fog, Clouds,
        CloudLayers, Lightning, Wind, Surface, Vfx, Interface) with
        `from_world`/`to_world`, the world staying the runtime's in-memory
        shape; `scene-components`, `set-scene-component`,
        `remove-scene-component`, `import-scene` and `set-default-scene` in
        shell/MCP/dispatch; the tray owns scene files outright (New scene,
        rename-to-rename, delete-with-undo, double-click to open, external
        moves followed by id on load); builds skip `.blockscene` anywhere
        (the pack embeds); `AssetKind::Scene` plus the actor list's scene box
        (no add button) and the settings default-scene row.

### Phase 3 - Dev productivity, before API surface explodes
- [x] Script toolchain: ship rustc or graceful degrade plus highlight plus inline errors plus rust-analyzer Cargo project.
- [x] VM/codegen correctness: suspendable reporter `wait`, recursive statement blocks.
- [x] Game view: the world runs in the editor and draws into a docked Game view, with input forwarding, pause/step and pointer lock. Started as a sidecar MJPEG stream beside the runtime's own OS window; on Linux it is now in-process with GPU frame sharing and no extra window (see "Qt6 rewrite" below for what's left there).
  - [x] Headless mode for the sidecar: hide the OS window while it keeps rendering the stream. Moot in-process, where there is no window.
  - [ ] MJPEG fallback (Windows, macOS, `BLOCKLOOM_RUNTIME=process`): the resolution switch still resizes the OS window, pointer lock only gets absolute positions, and the stream is a fixed ~15fps JPEG-60 regardless of preset or pause state. Most of this goes away once those platforms share GPU frames.
- [x] Visual world editor: edit-mode 2D/3D viewport with selection sync to ActorList/Inspector, drag to move plus rotate/scale gizmos, snapping, camera pan/zoom/orbit. Shares the Game view panel: Edit manipulates placement directly, Play runs the world.
- [ ] Editor: gizmos/snapping, prefab mode, scene search, log filter, frame stepper, profiler (draw calls, CPU/GPU/memory), playmode tests.
- [x] Editor/headless project sync: shell and MCP sessions share live state with an
      open editor instead of forking a silent second copy that last-writer-wins
      over the user's work.
  - [x] Attach mode: shell/MCP drives the editor's own Backend over the existing
        command channel instead of booting a second in-memory project;
        `open-project` attaches when the folder is already open, owns only when
        it is not. One copy, no merge problem by construction.
  - [x] Lock file: per-folder lock with owner PID, session id and heartbeat; a
        second owner-mode opener warns or takes over explicitly, never silently.
  - [x] Live reload (covers every non-attached reader): file-watch
        project.blockloom plus assets, and since every edit already hits disk on
        landing, reload idle backends straight off disk. Conflict prompt when both
        sides hold unsaved in-memory work (keep mine / take theirs); undo history
        stays per side and is never merged.
  - [x] Revision feed: counter on every save; shell `--watch` streams revisions so
        agents poll cheaply, MCP state resources re-read on revision bump.

### Phase 4 - Look and depth, uses Bevy leverage
- [x] Asset pipeline: glTF/FBX rigs, atlases, texture/audio compression, reimport tracking.
- [x] Materials/custom WGSL plus shader graph lite, particles/trails, post-process, shadows/HDR, 2D sorting layers, tilemap/terrain.
- [x] Load glTF scenes for Model looks (a ModelSource loader with rig playback from the parsed animations) instead of placeholder boxes.
- [x] Bake atlas layouts into sheets at build time - pack_atlas is plan-only today - and let a tilemap animate tiles and collide per-tile rather than as one slab.
- [x] Close the custom-shader loop: export a shader graph to a .wesl asset, and let hand-authored WESL drive the live material instead of only the uniform path.
- [x] World-space material texturing (fixes stretched textures on large brushes,
      first-person walls and floors first): per-material texture transform
      (tiling X/Y, offset, rotation), sampler choice (Repeat/Mirror/Clamp plus
      anisotropy), normal and roughness map slots beside albedo, and a
      triplanar/box-projection toggle with world-scale texel density so big
      surfaces tile evenly at any size. Scale-corrected UVs for Cuboid/Plane
      primitives, applied in `surface_standard` and the graph ubershaders, with
      inspector rows for the new dials. Old projects normalize to the legacy
      mapping (tiling 1x1, clamp) so nothing already shipped changes look.
  - [x] Advanced pass (for Phase 5 terrain): stochastic texture bombing to hide
        tiling, macro variation plus micro detail maps, mask stack (slope, height,
        cavity plus snow/wetness fed by the weather director and the persistent
        wetness map), blend debug view. Builds on the triplanar toggle and texel
        density above, not a second implementation. Not covered: no weather
        director or persistent wetness map exists yet, so snow and wetness come
        from World.surface, volumes and the run's overrides.
- [x] Particle/trail blocks (burst, emitter dials), plus ghost trails for custom-shaded and tilemap actors.
- [x] Advanced physics: fixed, hinge and rope joints, character controller, one-way platforms, and ragdoll chains built from hinged bodies.
- [ ] Physics, collision and character controller overhaul (`docs/physics-and-character-controller-plan.md`, ledger in `docs/physics-compatibility-ledger.md`):
  - [x] Phase 0: probes against pinned Rapier for materials, compounds, events, queries, CCD and the controller; compatibility ledger.
  - [ ] Phase 0 leftovers: 2D probes, joints, mesh cooking, per-body solver overrides, a platform-carry backend switch, Unity-editor confirmation of the documented values.
  - [x] Phase 1: document and ownership foundation (specs, ids, ownership, materials, validation, transactions, migration preview, shell/MCP commands). Not yet: inspector UI, runtime reading the components, applying migration.
  - [x] Phase 2: bodies, geometry and solver policy (planner in core, per-collider child entities, mass rules, constraints, sleep, materials with stick/slip, layer matrix and exact pair filtering, CCD mapping, speed caps, Play/Build preflight, layer/plan commands). Force blocks and per-body solver overrides landed with Phase 3. Contact offset, queryable and interpolation now act at runtime, and Rigidbody/Collider have inspector cards.
  - [x] Phase 3: contact lifecycle, authoritative 2D and 3D queries (ray, rays, ball/box/capsule cast and overlap, closest) over the Rapier world with layer and trigger filtering, collision cooking (hulls, decomposition, trimesh) for Play and builds, query blocks and `hit` reporters in the VM, compiled logic and script ABI. Open: no GPU-checked run, no inspector UI for cooking settings, no collider handles in the scene view yet.
  - [x] Phase 4: character controllers (CharacterController component and document ops, Unity-style move/slide/step/slope/ground/recovery over Rapier in 2D and 3D, move and settings blocks with `controller` reporters across the VM, compiled logic and scripts, hit events, inspector card). Open: no GPU-checked run, no platform carry, QML untested here (no Qt in the container), no capsule gizmo in the scene view yet.
  - [x] Phase 5: CharacterMotor (intent/jump/crouch/slide/coyote/buffer/carry core, runtime driver, blocks, reporters, scripts, inspector card) and input action maps (vector actions, composites, processors, per-player copies, mouse delta). Open: no `when I land` hat, no input rebinding UI or persistence of overrides, no GPU or Qt run, platform carry lags one tick, action maps are toggled from code only.
  - [x] Phase 6: PlayerCamera component (look, zoom, wall avoidance, 2D follow), player presets with preview and conversion, profiles, commands in shell/MCP, setup card and camera form. Open: no collision gizmos, no profile overrides or reset-to-profile, no Qt or GPU run.
  - [x] Phase 7: Constraint component (fixed, hinge, ball, slider, spring, distance, wheel, configurable; several per actor, world anchor, limits, motors, break force/torque and message), joint blocks/reporters/script calls/shell/MCP, inspector card, colliders size buoyant bodies, shards inherit filters/surface/mass, clones get physics and their own constraints, Physics Debug toggle and profiler rows. Open: no joint gizmos, no animation ragdoll handoff, AI does not drive the motor, collision streaming is not independent of visual LOD, no single-step or replay, no pair/contact counts, no Qt or GPU run.
  - [ ] Phase 8: compatibility and shipping. Done: `migrate-physics` with preview, backup, per-actor and undo plus an upgrade card; the physics playground sample (shell/MCP `sample=`, New Project dialog); creator guide `docs/physics.md`; coverage audit `docs/physics-api-coverage.md`; ledger rows 37, 39, 40 closed; joint blocks held against the compiled program; pack round trip. wasm `cargo check` of the runtime passes. Open: `just web-smoke`, native and web player runs on a real display, Windows and macOS checks, 14.2 scenarios that need a GPU or a course (see the audit), Phase 0 leftovers, no Qt or GPU run.
- [x] AI: live polyanya rebake, navigation cost areas and layer masks, off-mesh links, crowd separation, steering, behavior trees and sight perception.
- [ ] Performance foundation (do before Phase 5 needs it): engine-wide footing for
      the Phase 5 environment stack; Phase 5 adds the rendering budgets on top.
  - [x] First renderer pass: shared primitive mesh and PBR material handles for
        Bevy instancing, sphere screen-size LOD, depth-pyramid occlusion,
        hysteretic XZ cell activation, and live render timings, mesh allocation
        and Game view target footprint.
  - [x] Batching and instancing (the mechanism; Phase 5 sets the numbers): static
        batching for level geometry, GPU instancing for repeated meshes (one draw
        per mesh, per-instance data in storage buffers, batch keys independent of
        material slot layout so texturing changes never re-key), dynamic batching
        for small meshes. Phase 5 decides what scatters by the thousand; this is
        how the thousand draws become one.
        Done in `blockloom-runtime/src/batching.rs`: an instanced PBR surface with
        tint and UV transform per `MeshTag` slot, static merges per streaming
        cell, per-frame dynamic merges, all tuned by `BatchPolicy`. 3D only;
        Bevy's sprite batcher already covers 2D.
  - [x] LOD and occlusion (framework plus hooks; Phase 5 plugs policy in):
        screen-size LOD selection with hysteresis bands and a per-level swap API
        for meshes, software Hi-Z or query-based occlusion culling for interiors
        and caves, GPU frustum culling where Bevy does not already do it. Terrain
        chunks, trees and VFX supply thresholds and levels, never new selectors.
        Done in `blockloom-runtime/src/culling.rs`: `LodGroup` levels with
        `LodChanged`, a CPU Hi-Z over solid `Occluder` boxes, and GPU occlusion
        and GPU frustum culling as camera toggles, tuned by `LodPolicy` and
        `OcclusionPolicy`. Merged dynamic batches now carry real bounds.
  - [x] Bevy 0.20 migration (do right after LOD, before anything else below is
        written): bump the workspace from 0.19.1 to 0.20.0-rc.1 now and follow to
        final on release, so async streaming, GPU measurement and the Phase 5
        refactors below target the new APIs instead of being migrated twice.
        Translate custom shaders to WESL (naga_oil is gone; plain WGSL without
        preprocessor directives keeps working), move 2D sprites onto the
        SpriteMesh/Mesh2d backend, update observer syntax (`On<Add<A>>`), pointer
        events (`PointerPress`), exclusive-system code paths, `bevy_math` imports
        (`bevy_shape`/`bevy_curve` split), tonemapping paths plus `Linear` for the
        old `None` behavior, and extraction generics (`bevy_extract`). Re-add
        `ScreenSpaceTransmission` on 3D cameras that need it (now opt-in).
        Confirm bevy_rapier has a 0.20-compatible release first; if it lags the
        RC, timebox on the RC and land the upgrade when rapier lands.
        Done on 0.20.0-rc.1: shaders are WESL, the surface wrapper too, `None`
        tonemaps as `Linear`, sprites draw through `SpriteMesh` (a look swap
        drops its `SpriteMeshMaterial`). No material uses transmission, so no
        camera needed `ScreenSpaceTransmission`. bevy_rapier has no 0.20
        release: it moved into the rapier monorepo, pinned by git rev and
        patched onto the RC's Bevy in the root `Cargo.toml`.
  - [ ] Finish Bevy 0.20: move to 0.20.0 final, swap the rapier git pin and
        its `[patch]` for a crates.io bevy_rapier release once one exists.
  - [x] Async loading and streaming (owns the cell system; Phase 5 content only
        registers into it): background asset loads with placeholder or fade-in,
        world streaming cells with hysteresis so borders never thrash, shader
        prewarm on Play and at build time so first frames never hitch. Terrain
        chunks, noise volumes, HDRI mips and probe captures arrive as payload
        types on this system, not as a second one.
        Done in `blockloom-runtime/src/streaming.rs`: budgeted nearest-first
        cells with `CellEntered`/`CellLeft` and `CellTasks` for background
        payload work (static batch merges are the first payload), flat
        placeholders for loading 3D looks and sprite fade-in in 2D, and a
        warm-up window after every rebuild that draws everything unculled and
        holds the green flag until loads and pipeline compiles settle. Builds
        validate every `.wesl` surface file first.
  - [x] GPU measurement: per-pass timestamp queries plus render-target memory
        accounting, surfaced in the profiler (completes the render half of the open
        Phase 3 profiler item). No Phase 5 budget is enforceable without it.
        Done in `blockloom-runtime/src/gpu.rs`: a whole-frame timestamp span
        (`gpu/frame`) beside Bevy's per-pass `elapsed_gpu`, render-target bytes
        by kind (color, depth, prepass, shadow, post, game view) plus images
        and everything else, sorted from the wgpu allocator's report by label
        so a new pass is counted without code of its own. Backends without a
        report (Metal) fall back to sizing the views' own textures. The
        profiler says when an adapter has no timestamps.
- [ ] Refactors to clear the path for Phase 5 (do these first, not mid-stack):
  - [x] One blended environment resource: volume blending writes a single
        `Environment` render resource (sky, fog, light, exposure deltas) that the
        dim2/dim3 passes read, instead of each pass reading the project. Sky,
        clouds, fog, water and post then consume the same blended values.
        `Environment.exposure` is the single EV value every pass reads; writers
        resolve by precedence (director track beats post auto-exposure beats
        manual EV), so the four exposure dials below never fight.
        Done in `blockloom-runtime/src/environment.rs`: the project's settings
        plus weighted `EnvironmentVolumes` blend into `Environment` every
        frame, exposure resolves through `ExposureClaims`, and
        `apply_environment` is the only writer of camera exposure, tonemapping,
        bloom, vignette, AO, the sun, ambient and the clear color. It is
        extracted to the render world for the Phase 5 passes. Nothing fills
        the volume list or the claims yet; the volume framework does.
  - [x] Shared shader library and pass plumbing: common WESL chunks (hash, noise,
        FBM, scattering helpers, standard UBO layout) plus one FP16 working-target
        set with a half-res scratch pair and bilateral upsample, used by both
        dimensions. Stops volumetrics, fog and SSR from each rolling their own.
        Done: the modules are `blockloom::hash`, `noise`, `fbm`, `scattering`
        and `frame` (`blockloom-core/src/shader_lib.rs`), importable from
        surface files too. `blockloom-runtime/src/passes.rs` gives any camera
        carrying `WorkingTargets` the FP16 set, `FrameUniforms` from the
        blended `Environment`, and prewarmed upsample pipelines (depth-guided
        in 3D, bilinear in 2D) behind `passes::upsample`.
  - [x] Asset and snapshot extension points: the importer grows 3D/LUT volumes,
        HDR/EXR with BC6H, heightmaps, IES/cookies (extend the Phase 4 pipeline,
        not a parallel one); probe capture becomes a service reused for HDRI
        baking, reflection probes and water reflections; the sense snapshot gains
        a versioned slot for sun/wind/fog/weather so VM, codegen and scripts stay
        in sync on fixed ticks.
        Done: `ImportRole` in `blockloom-core/src/pipeline/` (hdr, volume,
        heightmap, ies, cookie) with per-asset overrides in the manifest and
        `load_*` decoders for passes; HDR plans as BC6H but the encode itself
        waits for the build step that applies texture plans. Probe capture is
        `blockloom-runtime/src/probes.rs` (`ProbeService`, six FP16 faces,
        optional cube readback). `Sensors::atmosphere` is sampled per fixed
        tick in `atmosphere.rs` and read by the `Atmosphere` reporter and a
        script's `atmosphere()`; nothing writes wind, fog or weather yet.

### Phase 5 - Environment and AAA look (HDRP/Unreal parity)
- [ ] AAA environment stack (HDRP-grade, Bevy leverage where it exists, ordered
      bottom-up: frame, volumes, light, sky, air, wind, clouds, world, effects,
      post, perf, direction, camera, audio, tooling): a World Environment asset on the
      project plus Environment volumes in the scene, with HDR, lighting, sky, fog,
      clouds, water, terrain, VFX and post blending by volume weight.
  - [x] Full HDR pipeline (linear FP16 end to end, HDRP-style units and output):
        - Rendering: FP16 HDR render targets from sky through lights to post, linear
          working space, tonemap and OETF only at final output. All lights, sky sun,
          clouds and emissives can exceed 1.0 without clipping. Bloom threshold works
          in HDR (1.0-plus for real glints, below for stylized glow).
        - Physical units (the scene-referred side; the EV value itself lives in
          `Environment.exposure`): sun in lux with real sun/sky ratios, punctual
          lights in lumens/candela with range falloff in meters, emissive as color
          times intensity multiplier (HDR color picker with exposure-invariant
          swatch). Exposure is the single bridge from scene lux to display nits,
          written by the director track or post auto-exposure per the precedence
          on the blended environment resource, never stored per system.
        - Display output: SDR sRGB fallback everywhere plus true HDR where the OS
          offers it (Windows Advanced Color scRGB/HDR10, macOS EDR, Vulkan HDR
          swapchain, Wayland color-management when present). Selectable output
          Rec709/sRGB, Rec2020 with ST2084 PQ, peak brightness 100 to 10000 nits
          plus paper-white setting. UI composited after tonemap so HUD stays SDR
          crisp under HDR scene.
        - Assets and import: EXR and Radiance HDR import for skies and emissives,
          BC6H compression for HDR textures, HDR cubemap and equirect support,
          per-texture exposure bias. Built games carry an HDR-or-SDR flag per
          target so WASM and weak mobile clamp to LDR at build time.
        - Editor: HDR Game view toggle (native HDR when display allows, else
          simulate-paper-white), histogram plus waveform plus false-color and
          clipping-zebra debug views, peak-brightness calibration pattern, EXR
          screenshot export. Profiler line for HDR target memory and tonemap cost.
        - Blocks and scripts: `set exposure/HDR output/peak brightness to`,
          `set light intensity/emissive strength to`, reporters `scene luminance`,
          `is HDR display?`, `peak brightness`. Fixed-tick sampling like the
          weather director so VM and codegen agree.
        Done: world cameras are always `Hdr` (FP16 to the tonemapper, bloom or
        not, unless a build is SDR-only); `Light` components in lumens; `set
        exposure`, `set my light`, `set my glow` (emissive strength), `set HDR
        output`, `set peak brightness` in blocks, compiled logic and scripts;
        `scene luminance` (a compute meter in `luminance.rs`, read back and
        sampled on the fixed tick), `is HDR display?` and `peak brightness`
        reporters plus atmosphere readings. `World.display` holds the output
        space (SDR, HDR10 PQ, scRGB), peak and paper white; `hdr::HdrFrame`
        resolves it against what the window offers, and `display.rs` takes the
        player window's surface over in an HDR color space, with a tone curve
        to the headroom and an scRGB/PQ encode after the UI. Game view debug
        views: false color, zebra, histogram, waveform, calibration patches,
        HDR preview. EXR screenshots (`capture.rs`), Radiance/EXR skies as
        cubes from a panorama or strip (`sky.rs`), baked to BC6H at build
        time, per-asset exposure bias, the build dialog's HDR switch
        (`GamePack.hdr`), and `memory/targets/hdr` plus `hdr/tonemap` in the
        profiler.
        Then: the swapchain path run on real HDR hardware (KDE Wayland,
        NVIDIA, HDR10 and scRGB both adopted, clean under the validation
        layer); HDR10 static metadata (primaries, mastering peak, MaxCLL at
        the peak, MaxFALL at paper white) sent per swapchain through
        `VK_EXT_hdr_metadata` or DXGI `SetHDRMetaData`
        (`display::send_metadata`); an HDR Game view on Wayland, where the
        world presents to its own swapchain on a subsurface under the editor
        window and the view shows through to it (`embed::HdrPlane`,
        `game_view.cpp`'s plane); a BC6H codec over all 14 modes, the
        encoder searching one-region modes and the two-region ones over the
        best-fitting shapes, checked bit for bit against bcdec, with
        reflection probe bakes now sampled as BC6H on the GPU like the sky;
        `HdrColorField` for emissive and volume fog glow (exposure-invariant
        swatch, intensity in stops); and a per-target HDR default in the
        Build dialog (ARM64 Linux SDR, the web player always SDR).
        Not covered: the DX12 metadata path is uncompiled here (no Windows
        toolchain) and Metal EDR takes no metadata; the HDR Game view is
        Wayland only, so X11 keeps HDR preview; MaxFALL is a fixed paper
        white rather than measured.
  - [x] Volume framework (the backbone everything below plugs into): global default
        plus box/sphere volumes with priority, blend distance and weight. Every
        property has an override checkbox HDRP-style, so a cave volume can take fog
        and exposure without touching sky. Debug views: volume heatmap, active blend
        list, frozen-frame lerp inspector. Block API: `enable volume _`, `set weight
        of volume _ to`, reporter `active volumes`.
        Done: the project's settings are the global default and a `Volume`
        component (`blockloom-core/src/volume.rs`) adds box, sphere or global
        layers with priority, outer blend distance, weight and a checkbox per
        property (background, sun, ambient, AO, exposure, tonemapper, bloom,
        vignette). `blockloom-runtime/src/volumes.rs` weighs them at the camera
        into `EnvironmentVolumes`. Blocks, compiled logic and scripts share
        `enable volume`, `set weight of volume` and `active volumes` (a JSON
        list, sampled on the fixed tick). The Game view's volumes panel has
        bounds with their feather, a per-pixel heat map (depth-reconstructed
        in 3D), the live blend list and freeze, which holds the blend and
        shows each property's lerp. A selected volume has scene-view grips
        for its faces (or radius) and blend distance.
        Not covered: the heat map measures at most 32 volumes, and leaves
        out global ones.
  - [x] Lighting rig (makes interiors and nights look right): irradiance/light-probe
        volumes (brick grid like HDRP APV, bake button plus auto-dirty on move),
        reflection probes (box-projected cubemaps, capture on demand, blend by volume),
        rect/disk area lights with LTC speculars, light cookies (projected texture
        with tiling), IES profiles for spotlights (import file, intensity in candela),
        contact shadows (screen-space 16-tap raymarch under feet and clutter),
        shadow tuning (PCF/PCSS toggle, cascade splits, normal/slope bias, fade).
        API: `capture probes`, `set shadow distance to`, per-light `casts shadows?`.
        Done: a `Probe` component (`blockloom-core/src/probe.rs`,
        `blockloom-runtime/src/light_probes.rs`) as a box-projected reflection
        probe or an irradiance brick grid, baked through the capture service
        into `.blockloom/probes` from the inspector or `bake-probes`, stamped
        so a move marks it stale and auto-bake redoes it in the scene view,
        shipped by builds, and overlapping probes blend through their falloff.
        `Light` gains rect and disk area lights (Bevy's LTC `RectLight`),
        candela, spot cookies with tiling and IES masks (spot and point)
        baked into Bevy light textures, per-light contact and PCSS shadows and
        biases. `ShadowSettings` tunes the filter (hardware, Gaussian,
        temporal), cascades and their split and blend, distance, normal bias,
        PCSS sun size and contact shadows; the sun can take a tiled cookie.
        Blocks, compiled logic and scripts share `capture probes`, `set
        shadow distance to`, `turn my light's shadows` and `casts shadows?`.
        Later: Bevy's PBR shaders are patched as they load (`pbr_patch.rs`)
        for true disks (a 12-gon of the disk's area), area light shadows
        (a black point light twin's cube shadow, PCSS by the light's size)
        and a sun shadow fade over the last `fade` of the distance. A point
        light repeats its cookie on each face. Volumes gained `reflections`
        and `indirect` multipliers over probes and the sky's light, and a
        bake leaves out its own actor.
        Not covered: an area light's shadow is cast from its centre, so its
        penumbra is PCSS's guess rather than the true area's; a disk is a
        12-gon; area lights take no contact shadows; the fade is a shader
        constant, so changing it recompiles the PBR pipelines; the patches
        are exact text against Bevy 0.20 and need redoing on a bump.
  - [x] Ray-traced lighting (bevy_solari, experimental upstream, RTX-class GPUs):
        - Realtime (`bevy_solari::realtime` via `SolariPlugins`): alternate high-end
          backend beside the raster rig, with ReSTIR direct lighting plus GI,
          ray-traced sun/spot/point shadows, and ray-traced reflections where the
          raster SSR falls off. Per-light RT toggles, bounce and sample-count
          dials, denoiser (DLSS ray reconstruction where available, else the
          temporal/spatial path). Falls back per light and per platform, and is
          excluded from WASM and weak-mobile builds.
        - Path tracer (`bevy_solari::pathtracer`): progressive reference mode, not
          for gameplay. Converging stills in the inspector and viewport for
          validating a scene's lighting, with time/sample budget and EXR export.
          Used to check raster and realtime-RT looks against ground truth.
        - Gating: cargo feature plus runtime capability probe (ray-tracing capable
          adapter, DX12/Vulkan), pinned Bevy version since the API is experimental.
          Missing hardware logs once and quietly uses the raster rig.
        - Blocks and scripts: `enable ray tracing`, `set GI bounces/samples to`,
          reporters `is ray tracing on?`, `ray tracing available?`. Fixed-tick
          sampling like the weather director so VM and codegen agree.
        Done: `blockloom-runtime/src/ray_tracing.rs` behind a default
        `ray_tracing` cargo feature, probing the device for Solari's features
        once and saying why in the editor when it can't. Project Settings has
        a Ray tracing section (on, bounces, samples, denoiser, GI reach); the
        world camera then takes `SolariLighting`, surfaces go deferred for its
        G-buffer (and back to forward when it is off, mid-run too), and sun
        shadow maps give way to traced shadows. The traced world is a copy:
        a proxy per drawn mesh and an emissive stand-in of the same power per
        light, which each light's Traced switch leaves out. The path tracer
        is a Game view button with a sample/time budget and progress, and the
        EXR camera waits for the budget. Blocks, compiled logic and scripts
        share `turn ray tracing`, `set GI bounces/samples to`, `ray tracing
        on?` and `ray tracing available?`, sampled on the fixed tick.
        Then: a realtime path tracer mode beside Hybrid ReSTIR (paths per
        pixel, NEE with MIS), an SVGF-style spatiotemporal denoiser (Auto is
        ReSTIR's reuse plus the filter; Filter, ReSTIR and None pick one or
        neither), traced rays that escape see the sky (Solari's shaders
        patched to read the environment map) or a flat sky's ambient, and a
        hooded spot emitter that only lights its cone.
        Not covered: converging stills in the inspector; cookies and IES profiles aren't
        traced; box-projected and shader surfaces stay forward with raster
        lights; skinned meshes trace in their bind pose; the denoiser has no
        separate specular signal, so glossy reflections only get a shorter
        history and blur; the realtime path tracer has no reuse, so it wants
        the filter; only run here on Intel Arc through Mesa and on lavapipe,
        not yet on RTX or DX12.
  - [x] Sky types (all feed background, ambient probe and reflections together):
        - Procedural physical sky: sun disk (size, limb darkening, intensity) plus
          moon disk (size, phase 0-1, halo power), Rayleigh RGB scattering, Mie
          anisotropy g plus directional intensity, ozone absorption, Rayleigh and Mie
          altitude scales, ground albedo tint, horizon-to-zenith blend curve, planet
          radius and atmosphere thickness for limb curvature, night tint ramp, and
          a sky-local exposure bias (added after `Environment.exposure`, never a
          second EV). Sun position driven by lat/long plus time, or
          manual azimuth/elevation.
        - Gradient sky: top/middle/bottom stops, horizon offset and softness,
          horizon warmth tied to sun elevation, dither toggle to kill banding,
          exposure multiplier. Cheap stylized path, also used as fallback under
          volumetrics on low-end.
        - HDRI sky: equirect or cubemap asset, rotation Y plus tilt, tint, exposure
          multiplier, mip blur for ambient vs sharp for background, horizon seam fix.
          Import bakes diffuse/roughness mip chain once, reused by probes.
        - Shared: background vs reflection vs lighting contribution toggles, ambient
          probe dimmer, sky-to-fog blend at horizon so fog never hard-edges.
        Done: `World::sky` (`blockloom-core/src/sky.rs`) is flat, physical,
        gradient or HDRI, with a shared sun placement (the light direction,
        azimuth/elevation, or NOAA's solar position from lat/long, day and
        hour) that moves the sun light too; a physical sky's air reddens and
        dims it by the same model. `blockloom-runtime/src/sky.rs` writes the
        sky's light into a cube and a small dimmed one on change
        (`shaders/sky_cube.wesl`), Bevy filters them once per change, and
        the camera takes reflections from one and diffuse from the other, or
        black where a toggle says no. The background is drawn per pixel in
        the skybox slot: dithered gradients, the physical sky with sharp
        limb-darkened sun and phased, haloed moon, HDRIs sharp or blurred from
        the filtered mips, turned, tilted and tinted. Builds bake an HDRI to
        BC6H with its mip chain, seam fix applied. Sky exposure (EV on top of
        the camera's) and the ambient dimmer are volume properties.
        Not covered: sky-to-fog blend, since there is no fog yet (the fog
        item reads the sky); the physical sky is single scattering only, so
        twilight zeniths run dark; the roughness chain is filtered on the GPU
        when the sky changes rather than baked at import; the seam fix blends
        a panorama's wrap seam, not its horizon; no blocks drive the sun or
        time of day yet; 2D worlds have no sky.
  - [x] Atmosphere, fog and space:
        - Height fog: base height, falloff, extinction distance, inscatter color
          keyed to sun elevation (warm at dusk, gray at noon), sun disk inscatter
          boost for god-ray-ish horizon.
        - Volumetric fog (froxel grid like HDRP, e.g. 128x72x64 over camera range):
          density, height falloff, anisotropy g, colored albedo and emissive for
          light shafts and neon smog, per-light inscatter toggle, noise scroll for
          drifting mist. Local fog volumes add density with box falloff.
        - Aerial perspective: distance blue-shift plus desaturation curve out to
          10km-plus, height-tinted so mountain tops stay crisp and valleys haze.
        - Stars: procedural hash field (density, magnitude distribution, temperature
          tint variation), twinkle amplitude plus speed, horizon fade, sun dimming
          curve, optional Milky Way band texture with intensity. Moon lights the
          scene as a real directional at night.
        - Aurora: curtain sheets (2-3 layers, altitude, width, ray structure noise
          scale), color ramps (green bottom to purple top), flow speed, coverage KP
          0-9 dial, horizon glow. Block-settable for sci-fi skies.
        - Lightning: `strike lightning at x y` block spawns flash light (intensity,
          color, decay) plus sky ambient pulse plus thunder sound with delay by
          distance. Random-strike director with rate and region box for storms.
        Done: `World::fog` (`blockloom-core/src/fog.rs`) and one pass after
        the main passes (`blockloom-runtime/src/fog.rs`). Height fog is
        integrated exactly along each ray from base, falloff, see-through
        distance and start, colored day/dusk/night by the sun's elevation
        with a sun glow lobe, and covers the sky so the horizon blends into
        it with the sky's own horizon light. Volumetric fog is a froxel grid
        (Low/Medium/High, 128x72x64 by default, squared depth slices over a
        range): density with height falloff, anisotropy, albedo, emissive,
        FBM noise drifting with the wind, lit by the sun and moon through
        Bevy's own cascade shadow maps (light shafts), by point and spot
        lights with a per-light `Lights fog` switch, and by the ambient;
        blended with last frame's grid reprojected, then integrated per
        column. A `Volume` adds local fog inside its shape, fading over its
        blend distance. Aerial haze blue-shifts, desaturates and fades far
        surfaces into the sky's horizon color, thinning with height. Fog
        density, color, base, volumetric density and albedo and haze distance
        are volume properties. Stars (hashed field with density, magnitude
        slope, temperature spread, twinkle, horizon and sun fades, an
        optional Milky Way panorama) and aurora (1-3 flowing curtain layers
        with ray structure, bottom/top color ramp, KP coverage from the pole,
        horizon glow) draw in the sky pass (`blockloom::space`), and the moon
        is a real directional light by phase. Lightning (`World::lightning`,
        both dimensions): a decaying flash light, an ambient and sky pulse,
        and thunder late by distance (a built-in rumble made in code, or a
        sound asset); a seeded storm director strikes at a rate inside a
        region on the fixed tick. Blocks, compiled logic and scripts share
        `set fog density to`, `set aurora to KP`, `strike lightning at` and
        `set lightning storm to`; `aurora` and `lightning` join the
        atmosphere readings and `fog density` is filled. Project Settings has
        Fog and Lightning sections and star, aurora and moonlight rows.
        Not covered: point and spot lights light the froxels unshadowed, and
        at most 16 of them and 16 local fog volumes are read; the composite
        reads opaque depth, so transparent surfaces are fogged as if they
        were what is behind them; froxel history has no neighbourhood clamp,
        so very fast motion can ghost; haze uses a Rayleigh-like spread over
        a height falloff rather than the physical sky's own scattering, and
        skips sky pixels; stars and aurora need a sky other than flat, show
        only in the main view (not in reflections or probes) and light
        nothing; aurora assumes flat ground; the flash light casts no
        shadows, there is no bolt to see, and thunder is not positional; no
        fog in 2D. GPU-checked on
        Intel Arc through Mesa only.
  - [x] Volumetric light volumes: per spot/point cone inscatter (density, anisotropy,
        falloff curve, near/far fade) for visible beams, dust motes (billboard points
        drifting in beam, size/alpha/twinkle), fake shaft cones (additive fresnel-faded
        geometry with noise scroll) for cheap beams, height-dust global toggle. Tied
        to fog density so beams thicken in fog and vanish on clear days.
        A light's `beam` adds medium in the froxels (beams first, then nearest,
        16 lights) or, in Auto with volumetric fog off or Low, a shaft cone;
        `set fog density` scales beams and volumetric fog, and volumes carry a
        `beams` multiplier. Not covered: beams are unshadowed like the other
        fog lights; shafts are spots only and ignore occluders past depth
        testing; motes and height dust are unshadowed and don't collide.
  - [x] Movement and animation (one wind system drives clouds, layers, vegetation,
        water and particles so a storm reads as one storm):
        - Global wind asset: direction, base speed, gust strength plus gust frequency
          (1D Perlin over time), vertical log-law profile (calm at ground, fast aloft),
          storm factor 0-1 scaling everything.
        - Local wind zones (box/sphere volumes): direction/speed override or additive
          swirl, turbulence amplitude, blend falloff. Used for tornado funnels,
          valley fog drift, interior stillness.
        - Cloud specifics: volumetric advection vector plus separate erosion drift,
          layer UV scroll, time-lapse multiplier (1x to 1000x for demo skies),
          seed shuffle button for instant new sky with same dials.
        - Blocks and scripts: `set wind direction/speed/gust/storm to`,
          `set cloud drift to`, reporters `wind speed`, `wind direction`, `storm`.
          All sampled on fixed tick so replays stay deterministic.
        Done: `World::wind` (`blockloom-core/src/wind.rs`) holds direction,
        speed, gust strength and frequency (two octaves of seeded 1D gradient
        noise, which also veer the direction), a log-law profile from a
        roughness length to a reference height, and a storm dial that
        triples the speed, quadruples gusts and triples turbulence at 1.
        Volumes carry local wind zones (blow instead, blow on top, or swirl
        round the actor's up axis with inflow and updraft), each with
        turbulence and the volume's own blend falloff, weight and priority.
        `CloudDrift` takes the wind at the clouds' altitude times a follow
        factor plus its own advection, an erosion drift, a layer scroll
        multiplier and a 1-1000x time-lapse, with a cloud seed the Project
        Settings Shuffle button rerolls. `blockloom-runtime/src/wind.rs`
        steps it on the fixed tick's clock into `WindField`; particles ride
        it (per-emitter `wind` factor), volumetric fog's noise drifts with
        it, and the cloud offsets are integrated there for the cloud passes.
        Blocks, compiled logic and scripts share `set wind [direction/speed/
        gust/storm] to` and `set cloud drift to`; `wind direction` and
        `storm` join the atmosphere readings beside `wind speed`.
        Not covered: vegetation and water have no renderer yet;
        volumetric clouds now consume the cloud offsets;
        wind doesn't push rigid bodies; dust motes and beam motes keep
        their own drift rather than the wind; there are no wind arrows in
        the viewport (the editor item below).
  - [x] Volumetric clouds (the hero feature, raymarched in sky pass at half res with
        temporal reprojection and depth-aware upsample):
        - Shape: tileable 128 cubed Worley plus Perlin-Worley FBM asset (authored or
          baked in editor), coverage remap curve (toe for wisps, shoulder for
          overcast), density multiplier, cloud type 0-1 (stratus to cumulus) driving
          height-gradient profile, bottom/top altitude plus thickness in meters,
          horizontal tiling in km, vertical wind shear vector.
        - Detail: erosion noise (second 32 cubed curlish set), micro detail scale,
          erosion strength per type, detail scroll speed multiplier, anvil head
          widening for cumulus tops, bottom billow and top feather curves.
        - Lighting: Beer plus powder scattering, dual-lobe Henyey-Greenstein phase
          (forward silver lining lobe plus back scatter lobe), sun lightbleed
          (powder through thin tops), ambient skylight tint top vs bottom occlusion
          darkening, height-based tint (warm lit edges, blue shadowed bellies),
          per-pixel analytic ambient from sky LUT so dusk clouds go pink.
        - Shadows: cloud-to-ground shadows via downsampled cloud depth rendered to a
          2k shadow map reprojected over terrain range, plus lightmarch self-shadow
          (5-8 taps jittered) inside the raymarch. Toggle per light, density-scaled.
        - Quality: presets Ultra/High/Medium/Low mapping to primary steps
          (e.g. 64/48/32/16) and light steps (8/6/5/3), transmittance threshold for
          early out, blue-noise jitter plus TAA. Stats line in profiler: steps,
          overdraw, ms.
        Implemented: `World.clouds` with project settings and `set-clouds`,
        normalized on load and edit, plus coverage/density/type volume overrides.
        `clouds.rs` bakes seeded tileable 128³ Perlin-Worley and 32³ erosion
        textures on the GPU, then marches at half resolution with blue-noise
        jitter, wind advection, sun/moon lightmarching and sky-cube ambient.
        Per-view history rejects edits, resize, skipped draws and large drift;
        the shared bilateral upsampler preserves the scene under clear rays.
        A 2048² sun/moon transmittance map projects onto visible terrain, with
        independent celestial shadow switches. This is a screen-space shadow
        approximation: it attenuates the combined surface lighting, including
        emissives, rather than only the direct-light term inside PBR.
        Profiler: average primary steps and contributing samples per pixel
        (`clouds/steps`, `clouds/overdraw`) plus GPU `cloud_march` timing.
        Shape and erosion noise are baked from the cloud seed on the GPU, or
        read from authored volume assets (image strips or `.cube`); Project
        Settings' Bake noise to assets (`bake-cloud-noise`) writes the seed's
        own noise to `assets/clouds/*.png` through a CPU twin of the bake.
        Blocks, compiled logic and scripts share `set clouds [coverage/density/
        type] to`, laid over the project's clouds and any volume for the run.
  - [x] Cloud layers (planar 2D cover above and below volumetrics, also the full
        fallback when volumetrics are off):
        - Up to 4 layers, each: coverage texture or procedural FBM (seed, scale,
          octaves), coverage amount plus contrast curve, tiling km, opacity,
          altitude in meters, normal offset for parallax vs camera, tint plus sun
          scatter tint plus powder edge tint, horizon fade start/end angles, day to
          sunset to night tint ramps.
        - Per-layer wind scroll vector plus global wind multiplier, optional flow map
          asset for cyclone swirl, rotation pivot for storm spin. Layers receive fog
          and aerial tint with distance so high cirrus still sits in atmosphere.
        - Editor paint mode: paint coverage into a layer texture with cloud/eraser
          brushes, blur and advect tools. Import any grayscale image as coverage.
        Done: `World.cloud_layers` (`blockloom-core/src/cloud_layers.rs`, at
        most four) with every dial above, normalized on load and edit, set by
        `set-cloud-layers` and Project Settings. Coverage is an image asset's
        luma or tileable gradient FBM baked from the layer's seed, scale and
        octaves, remapped live by coverage and contrast. The runtime
        (`blockloom-runtime/src/cloud_layers.rs`) draws them full resolution,
        farthest first: layers beyond the volumetric slab before its composite,
        layers between it and the camera after, and all of them alone when
        volumetrics are off. Parallax is a second tap along the view ray and a
        sun tap for self-shadow; sky-cube ambient, sun and moon light, the
        day/sunset/night ramp, a backlit edge glow and aerial haze towards the
        horizon's color, with height fog laid on by the fog pass after. Layers
        scroll on the wind's layer offset times their wind factor plus their
        own scroll, on cloud time (time-lapse included), turn round a pivot,
        and follow a two-phase flow map. Layers raise the `cloud cover`
        reading. Project Settings paints strokes into
        `assets/clouds/layer-N.png` (`paint-cloud-layer`: cloud, eraser, blur,
        advect, wrapping at the tile's edges), started from what the layer drew,
        so an imported image is never overwritten; every stroke keeps a
        snapshot under `.blockloom/cloud-paint`, so undo and redo take strokes
        back. Layers shadow the ground towards the sun with their own
        strength. `set cloud layer _ [coverage/opacity/contrast/altitude/
        spin] to` (and a script's `set_cloud_layer`) lays run-time dials over
        a layer. The scene view draws each layer's altitude plane and spin
        pivot.
        Not covered: layers stay out of the sky's light cubes, so reflections
        and sky ambient don't see them (the cubes are rewritten only when the
        sky changes, and moving layers would refilter them every frame, the
        same reason stars and aurora stay out).
  - [x] Terrain and vegetation: heightmap terrain (1k to 4k, import PNG/RAW, sculpt
        raise/lower/smooth/flatten/noise/terrace with radius/falloff/strength, paint
        albedo/normal/roughness layers with slope/height/curvature rules, holes for
        caves), auto collision plus LOD (quadtree or chunked, crack fix, pixel-error
        metric), grass (instanced blades or cross quads, density map, color variation,
        wind bend plus gust flutter, distance fade, cull distance), trees/rocks
        (instanced LOD0/LOD1/billboard, scatter brush with density/clumping noise and
        slope/altitude/collision filters, per-instance tint/scale jitter). Editor:
        sculpt/paint/scatter brushes, erosion filter preview, stats (tris, instances).
        Not covered: RAW import is 16/32-bit .r16/.r32 only, and the GPU half
        has no ignored embed test yet.
  - [x] Water (ocean, lake, river actor): wave model (8-12 summed Gerstner waves
        with amplitude/chop/steepness plus Phillips-spectrum normal detail, wind
        fetch param), depth color (shallow tint, deep tint, Beer absorption distance),
        transparency plus refraction offset, foam (shoreline depth fade, crest foam
        threshold with noise breakup, flow scroll), reflections (planar probe or SSR
        fallback plus GGX sun glint with roughness), underwater fog volume plus
        caustics texture projection, buoyancy component (sample height/normal/velocity
        for physics blocks and scripts), interaction hooks (splash particle, ripple
        decal, sound). Blocks: `set water level/chop/foam to`, `water height at x y`,
        `is _ underwater?`.
        Done: a `Water` component (lake, river with a current, or an ocean out
        to the horizon round the camera) with seeded Gerstner waves whose sea
        state follows the wind through a fetch-limited fit, Phillips detail
        normals, shallow/deep color with Beer absorption and clarity,
        refraction of the opaque frame, shore and crest foam broken up by
        noise and carried by the current, screen-space reflections falling
        back to a refreshing probe at the surface and then the sky color, the
        sun's glint through Bevy's own lighting, an underwater pass (fog,
        absorption, caustics, built in or from an image) and Snell's window
        from below. `Buoyancy` floats dynamic bodies at 1, 4 or 8 sample
        points with drag and spin damping; anything with a rigid body that
        drops in splashes droplets, a spreading ripple and the water's sound.
        The CPU sample and the drawn surface sum the same waves. Scripts get
        `water_at` (height, normal, velocity, foam), `is_underwater` and
        `set_water`. 2D water is a strip with the same waves, depth color,
        caustics and a foam line.
        Also done: a `Planar` reflection mode (a mirror camera with an
        oblique near plane at the surface, blurred by roughness), simulated
        ripples (a wave-equation height field per body that splashes and
        floating bodies' wakes disturb, and that buoyancy rides), far ocean
        waves calming on the CPU the way the drawn ones do, a swell that
        turns with the live wind by fading between headings over
        `waves.turn` seconds, and fog measured to the water surface.
        Not covered: the mirror is a flat plane per body, so tall waves
        bend its image rather than re-reflect it; a body bigger than its
        ripple extent simulates only a patch round the camera, and ripples
        it leaves behind are dropped; a wake is a push per sample point,
        not a Kelvin wave pattern; 2D water has no reflections. The GPU
        tests cover a lake's tint and the mirror, not ripples or fog.
  - [x] VFX graph (Niagara/VFX-Graph lite): GPU sim with spawn modules (rate, burst,
        shape sphere/box/cone/mesh-surface), update modules (velocity, drag, curl noise,
        turbulence, attractor, depth-buffer collide with bounce/friction, kill planes),
        render (flipbook sub-UV, size/color/rotation over life curves, soft particles,
        lit vs unlit), ribbons/trails (length history, tessellation, width curve, face
        camera, HDR color for glow trails), event hooks (`on collide/die/spawn` fires
        blocks). CPU fallback pool for headless/low-end. Editor: curve editor, live
        loop preview, max-particle budget and overdraw meter.
        Done: the `Emitter` component grew the graph (older emitters load
        unchanged): a spawn shape (point, sphere or its surface, box, cone,
        the actor's own mesh surface) launching along the facing or out of
        the shape, timed bursts with cycles on the emitter's clock, an
        ordered update stack (force, drag, curl noise, turbulence, attractor
        towards an actor or the emitter with a kill radius, collide with
        bounce/friction/lifetime loss/kill, kill plane), and render options
        (additive or alpha, facing camera/velocity/flat, velocity stretch,
        flipbook sheets with frame blending, size/opacity/rotation curves and
        a color gradient over life, spin, size jitter, soft particles, lit or
        unlit, HDR intensity). Ribbons follow each particle or the actor
        itself, with a point history, Catmull-Rom tessellation, a width
        curve, a color gradient, face-camera and glow intensity. 3D emitters
        simulate in a compute shader after the depth prepass (up to 65536
        each) and collide with the depth buffer; 2D, devices without compute,
        `Runs on: CPU`, a project's CPU-only switch and actor ribbons use the
        CPU pool in core (up to 4096 each), which collides with actors'
        collision shapes. Both fill one particle buffer the same draw reads.
        `when my particles [spawn/die/collide]` runs at most once a frame per
        event (from the GPU a frame or two late, through a readback) and
        `start/stop my particles` pauses spawning; both compile to native
        logic. The inspector edits all of it with a curve editor, the
        selected actor's emitter loops its duration in the scene view, and
        Project Settings holds the particle budget. The profiler gets
        `vfx/particles`, `vfx/emitters`, `vfx/gpu_emitters`, `vfx/budget` and
        `vfx/overdraw`.
        Then: GPU collisions also test the nearest 128 bodies' boxes and
        balls, so hidden or off-screen actors stop particles; each event's
        count and last position reach blocks (`my particle count`, `how many
        of my particles`, `where my particles last`) and scripts
        (`burst_particles`, `set_emitter`, `set_emitter_playing`,
        `particles()`); overdraw is counted per fragment on the GPU; lit
        particles receive shadows and can be translucent; a mesh surface
        includes a model's parts; embed tests cover collisions with a hidden
        body, ribbons and the overdraw meter.
        Not covered: the emitter stays a module stack laid out like VFX
        Graph's contexts, with no node editor or operator nodes; GPU
        collisions see bodies as boxes and balls only (no capsules, meshes
        or terrain unless on screen). Scripts hear particle events (and every
        other hat block's event) through their `event` entry point.
  - [x] Decals (transient marks only; lasting stains live in the destruction map
        below): deferred projected (albedo/normal/roughness/emissive, atlas pages,
        angle fade, depth reject to avoid floating edges), pool with LRU steal plus
        per-decal lifetime/fade, blood/footprint/fresh-scorch presets. Blocks:
        `spawn decal _ at`, `fade decals in radius`. No persist toggle: anything
        that must survive reload goes through the scorch/wetness map instead.
        Shipped: blood, footprint and fresh-scorch atlas presets, a 256-mark
        visibility-aware LRU pool, lifetime/radius fades, and spawn/fade blocks
        in QML, the VM and compiled logic. Projectors blend albedo, normal,
        roughness and emission with angle/depth rejection; a G-buffer pass
        feeds deferred and ray-traced lighting. Marks pause with the run and
        clear on rebuild. Native texture binding arrays are required; WebGPU,
        unlit graph surfaces and the reference path tracer are not supported.
  - [x] Destruction and fluids lite: fracture-on-hit (Voronoi cell count, interior
        cap material, impulse threshold, shard lifetime/sleep/pool cap), debris
        impulse inheritance plus bounce sounds, 2D shallow-water ripple grid for
        puddles/ponds (rain rings, footstep rings, shore reflect), smoke advection
        grid for stylized chimneys and dust puffs (no full 3D sim), persistent
        scorch/wetness map (world-space RT, dries over time, darkens albedo and
        raises specular): the sole owner of lasting surface state, read by the
        material mask stack. Blocks: `fracture _`, `splash at`, `puff smoke at`.
        Shipped: deterministic 2-64-cell Voronoi fracture for convex 3D
        primitives, authored cap PBR material, contact-impulse threshold,
        inherited linear/angular motion, sleep/timeout retirement and a shared
        256-shard pool. Bounce sounds use the existing capped voice mixer.
        Splash, rain and moving feet disturb the existing shallow-water grid;
        ponds retain the water system's shore/planar reflection path. Smoke
        uses up to 16 vertical 64x64 advected density grids, buoyancy and decay.
        One scene/player surface map owns lasting scorch and wetness, mirrored
        into a world-space RG render target and saved to player data (browser
        localStorage on web). Lightning scorches, splashes/rain wet, and water
        dries during simulation. Terrain/projected mask stacks and instanced
        surfaces sample it. All three blocks have QML, VM and compiled parity.
        Limits: fracture does not cut models, terrain or 2D sprites; smoke is
        a billboard plane, not a 3D simulation. The surface target is a fixed
        256-unit square at the origin, 64x64 cells; graph shaders, model-part
        standard materials and the reference tracer do not sample it.
  - [x] Post volumes (full HDR chain, volume-blended): exposure (auto spot-meter
        with min/max and speed writes `Environment.exposure` when enabled, else the
        manual EV stands; both lose to the director track per precedence), bloom (threshold/knee, 5-mip scatter chain,
        dirt texture), tonemap (ACES/Neutral/AgX select, toe/shoulder), white balance
        plus LUT/grading (lift/gamma/gain, saturation, contrast), vignette, depth of
        field (autofocus target or fixed distance, bokeh blades/circular, near/far),
        motion blur (shutter angle, per-object toggle), SSAO (HBAO, radius/intensity),
        SSR toggle with roughness cutoff, chromatic aberration, film grain, sharpen.
        Order fixed HDR-first; debug splits (bloom mip, CoC, AO only).
        Shipped: spot, center-weighted or average auto-exposure claiming
        `ExposureClaims::auto` between the director and the manual EV, with
        min/max, per-direction speed and compensation; Blockloom's own
        five-level bloom (soft-knee threshold, Karis prefilter, scatter,
        lens dirt image); AgX and Khronos Neutral beside the old tonemappers
        plus a toe/shoulder shape ahead of them; white balance, lift/gamma/
        gain, saturation and contrast in linear light; a `.cube` or strip
        LUT after the tonemapper; film grain with size and highlight
        response; Bevy's vignette, chromatic aberration, CAS sharpening,
        depth of field (fixed distance or an actor's depth, six-bladed or
        circular bokeh, near toggle, far limit, widest blur), motion blur
        (shutter angle, samples), SSAO (radius, intensity) and SSR (roughness
        cutoff, thickness; draws opaque surfaces deferred). Volumes blend
        every numeric and switch property; the Game view shows bloom levels,
        blur size and AO alone. Embed tests cover grading, bloom levels,
        grain and both 3D debug views on lavapipe.
        Limits: SSAO is Bevy's GTAO rather than HBAO, and its intensity is
        the project's (baked into the shader), not a volume property; bokeh
        is Bevy's hexagon or Gaussian, not a blade count; motion blur has no
        per-object toggle (Bevy's motion vectors have no per-mesh switch);
        auto-exposure, depth of field, motion blur, SSAO and SSR are 3D; the
        LUT is skipped under HDR output.
  - [x] Performance and scalability (whole-frame budgets for the stack above):
        - Done: shared Low/Medium/High/Ultra presets with hysteretic frame-time
          auto-drop, spatial `scale_views` plus TAA, per-system draw/triangle
          budgets (`Quality::budget`, `budget/<system>/...`), and quality blocks,
          reporters and `when quality drops`. Local throttles shrink before a
          shared preset drop: terrain/scatter LOD distance, grass density,
          particle/shard caps, water tessellation, prop/model LOD distance
          (spheres/capsules swap coarser then cull; cuboids/planes/models carry
          a batchable cull-only level, models decimated at mid range). Draws
          count the active world camera's visible meshes; rebuilds clear sampled
          costs. Streaming stays on the one `StreamingCells`/`CellTasks`
          hysteresis. UI recomposites natively (SDR) or at paper white (HDR).
          SSR traces half-res through the shared upsampler (`ssr_half`).
          Texture streaming is preset lod bias plus aniso caps with per-texture
          distance steps; LUTs and cloud noise share stacked 3D atlases.
          `indirect.rs` counts binned phases into `indirect/*` with a CPU
          membership mirror. DLSS is Bevy's `dlss` path behind the `dlss`
          feature (`just player-dlss`, SDK via `just dlss-sdk`), Auto default,
          ray reconstruction on Hybrid, warn-once fallback to TAA/spatial.
        - [x] Draw policy: per-system draw/triangle budgets with live
          `estimated_draws`/`draw_budget` and `budget/<system>/...` rows; local
          throttles shrink density/distance before the shared preset drops.
        - [x] LOD and throttle policy: terrain pixel-error LOD, prop/model LOD
          distance, water tessellation, particle/shard density, cloud/fog caps
          by preset, staggered probe captures, shadow distance and map caps.
        - [x] Content streaming: terrain, grass, scatter, tile rooms, batch
          merges, cloud noise, HDRI/probe captures as `CellTasks` payloads on
          the one `StreamingCells` hysteresis; static bakes stay under
          `GLOBAL_CELL` until their key changes.
        - [x] Resolution scaling: frame-time controller drives `scale_views`
          plus TAA with native UI recomposite; clouds march half-res, fog at
          its grid, SSR half-res, reflections/shadows follow budgets.
        - [x] DLSS (NVIDIA RTX, Bevy `dlss` path): Auto/DLAA/Quality/Balanced/
          Performance/Ultra Performance plus CAS sharpness; manual modes step
          down with the resolution signal; ray reconstruction on Hybrid;
          redistributable staged beside players and shipped in builds; off on
          WASM and weak targets.
        - [x] Memory: preset lod bias plus aniso caps with distance steps, BC6H
          sky/probe bakes, stacked noise/LUT atlases, bounded
          particle/decal/shard pools with visibility-based eviction, one
          auto-drop rule driving all of it.
        - [x] Blocks and scripts: `set quality/resolution scale/upscaler/DLSS
          mode to`, reporters `frame time`, `draw calls`, `current quality`,
          `is DLSS available?`, event `when quality drops` (`SetRenderSetting`,
          fixed-tick sampling).
  - [x] Time-of-day and weather director (the thing that makes it shippable):
        - 24h curve editor: tracks for sun azimuth/elevation, moon azimuth/elevation,
          exposure EV (the default writer of `Environment.exposure`; wins over post
          auto-exposure and manual EV while the director runs), temperature/tint, fog density, cloud coverage/type, precipitation,
          wetness, wind, aurora KP, grading LUT weight. Bezier keys, loop toggle,
          keyframe presets (dawn/noon/dusk/midnight).
        - Weather presets as assets: Clear, Overcast, Storm, Sunset, Night, plus user
          presets. Each stores full sky/cloud/fog/light/post deltas. `blend weather
          to _ over _ seconds` lerps with ease curve; stack holds current plus target
          so rapid changes do not pop.
        - Block and script API: `set time of day to`, `advance time by`, `set cloud
          coverage/density/type to`, `set fog density to`, `set precipitation to`,
          `set exposure to` (director value, top precedence), reporters `time of day`, `sun elevation`, `cloud
          coverage`, `current weather`, event `when weather becomes _`. Sensor
          snapshot carries sun/wind/fog so reporters and scripts agree per tick.
        - Determinism: seeded RNG per blend so two runs with same inputs make same
          clouds; fixed-tick sampling so codegen and VM match (same rule as
          tests/codegen.rs line-for-line check).
        Done: `World.director` (`blockloom-core/src/director.rs`, a `Director`
        scene component, normalized on load) holds the clock (time of day,
        day length, loop), one Bezier `TimeTrack` per dial above, and the
        project's presets; `WeatherPreset::builtin` answers the five palette
        names and `Director::keyframe_preset` the four moments. The runtime
        (`blockloom-runtime/src/director.rs`) steps clock and blend on the
        fixed tick ahead of the atmosphere sample: tracks and the blend move
        the sun (ahead of the air tint), fog, clouds, wind, aurora, exposure
        (only where `set exposure to` hasn't claimed the slot), rain, snow,
        wetness (lagged behind rain), temperature, clock and weather name, and
        a finished blend fires `when weather becomes`. Explicit blocks win
        over the blend, which wins over tracks. Blocks, compiled logic and
        scripts share `set time of day to`, `advance time by`,
        `set precipitation to` (rain/snow), `blend weather to _ over _`,
        `time of day`, `sun elevation`, `current weather` and `when weather
        becomes` (ABI 34, LOGIC_ABI 32), with parity cases in
        `tests/codegen.rs` (`weather-director`, `weather-arrives`) and
        `tests/vm.rs`; fixed-tick sampling like the wind and storm directors.
        Project Settings has the clock, day length, loop and keyframe
        buttons, a curve editor (`DirectorTrackField.qml`) that draws each
        dial's Bezier track for click/drag/double-click key editing with
        tangent and loop controls, a preset gallery that starts a saved
        preset from any built-in or saved one (`save-director-preset`) and
        edits its 21 dials in place, and the shell/MCP has `set-director`,
        `apply-director-preset` and `save-director-preset`. Presets carry the whole look: sunlight,
        ambient, sky exposure, bloom and saturation ride beside the
        fog/cloud/wind dials and land on the sun, the ambient dimmer, the sky
        and the post chain. Wetness is a spatial map (`WetnessMap`): a 32x32
        grid round the camera that rain soaks and warm sun and wind dry back
        towards the weather's damp at each cell's own uneven pace, sampled
        into both the reporters and the surface wetness.
        Not covered: the wetness map uploads at 32x32, so dampness varies
        over 2 m cells rather than per texel, and instanced batches, graph
        surfaces and 2D sprites keep the sampled uniform instead of the
        texture.
  - [x] Cinematics: timeline tracks (camera cut, transform, FOV, volume weight, signal
        fires block at marker), dolly/crane spline path with look-at target plus roll,
        camera shake (trauma 0-1, Perlin translation/rotation noise, decay), letterbox
        bars plus fade to black/white, slow-mo (timeScale curve) plus hitstop frames
        blocks, skip support (`skip cutscene` jumps to end marker). Plays on wall clock
        even when `pause game` freezes world strands, like UI strands do.
        Done: `World.cutscenes` (`blockloom-core/src/cinematic.rs`, a
        `Cutscenes` scene component, normalized on load) holds named reels:
        shots of a camera actor, an FOV and a length, each with an optional
        dolly path of look-at/roll/FOV keys eased between, plus signal,
        slow-motion and volume-weight keys. `blockloom-runtime/src/
        cinematic.rs` plays the reel on the real clock ahead of the sensor
        publish: shots drive the world camera after the rig and the room
        confine, signals fire `when cutscene signal` strands, slow-motion
        keys move the virtual clock the fixed clock follows (so blocks and
        physics keep their tick), volume keys borrow weights and restore
        them at the end, and `skip cutscene` fires the remaining signals
        before `when cutscene ends` runs. Trauma shakes the camera on
        seeded gradient noise and decays; letterbox bars (z 45) and a
        black/white fade wash (z 47) ease in over the interface but under
        the scene veil. Blocks, compiled logic and scripts share
        `play cutscene`, `skip cutscene`, `shake camera by`,
        `set time scale to`, `hitstop`, `set letterbox to`,
        `fade screen to`, `is cutscene playing?` and `cutscene time`
        (ABI 35, LOGIC_ABI 33), with parity cases in `tests/codegen.rs`
        (`cutscene-director`, `cutscene-arrives`) and `tests/vm.rs`.
        Reels are authored as JSON through `set-scene-component` (shell/MCP).
        Not covered: no visual timeline editor (reels are JSON); volume keys
        name actors carrying a `Volume`, and a reel never moves the sun or
        weather tracks; 2D shots frame position only, never FOV; the fade
        wash has no wipe/iris shapes.
  - [ ] Soundscape zones: reverb volumes (size/decay/damping/excursion, IR or
        parametric), occlusion (raycast, lowpass plus gain per wall hit) and
        obstruction (edge diffraction lowpass), weather-tied loop mixer (wind gain by
        wind speed, rain gain by precipitation, thunder by lightning director),
        ducking (music dips under dialogue or pause menu), underwater muffle tied to
        water volume. Blocks: `set reverb to`, `muffle _ while underwater?`.
  - [ ] Editor, preview and scaling:
        - Sky/Environment inspector: tabbed Sky/Clouds/Fog/Light/Post, live 16:9
          preview thumbnail rendering the real sky shader, cloud-shape slice viewer
          (scrub altitude, see density slice), coverage curve editor, preset gallery.
        - Viewport: volume gizmos (box/sphere with blend feather), sun path arc for
          time-of-day, wind arrows, cloud layer altitude planes, probe capture points.
        - Quality: per-platform override (volumetrics on desktop, layers-only on
          weak targets and WASM), resolution scale for sky pass, step counts per
          preset, auto-drop rule (if sky pass over N ms for M frames, drop one LOD).
          Build dialog lists which target keeps volumetrics and why.

### Phase 6 - 2D games, parity look and feel (2D-first, uses Phase 2 and Phase 4 footing)
- [x] 2D animation stack (builds on the open Phase 2 tweens/sprite-animation item; this is the 2D-specific half):
  - [x] Flipbooks: image-strip or atlas-page ranges per clip, fps plus per-frame
        durations, loop/ping-pong/once modes, events on frame marker. API: `play
        clip _`, `set animation speed to`, reporters `current clip`, `current frame`.
  - [x] Skeletal/bone 2D rigs: import from common 2D rig formats, bone transform
        hierarchy with IK-lite (two-bone), slot attachments that swap sprites,
        skin tint per slot. Falls back to flipbook when no rig is present.
  - [x] 9-slice/stretchable panels and sprite stacking: borders that do not stretch,
        center tiling modes, per-layer offset for stacked 2.5D sprites.
  - [x] Animation player/state machine for 2D: states with transitions on variable
        or event, blend/crossfade time, root-motion toggle that moves the actor.
        Shared with the Phase 2 player, not a second implementation.
  - [x] Sprite dials: flip X/Y, per-sprite material overrides (tint, palette swap
        index, outline width/color), sorting layer plus order-in-layer plus
        Y-sort toggle for top-down depth. Fixed-tick sampling so VM and codegen agree.
  - [x] Spine mesh attachments: deformed 2D meshes rebuilt per frame, weighted
        skinning and deform keys, so rigs that use meshes draw every part.
  - [x] Palette/outline effect on 9-slice panels (slice in the effect shader),
        and one outline around a whole rig or stack (silhouettes behind every
        piece rather than a texture, so only the outer edge shows).
- [x] Tilemaps and level building (builds on the Phase 4 per-tile collision and
      animated tiles; this is authoring plus runtime):
  - [x] Autotile and brushes: bitmask/edge autotile rules per tileset, scatter
        brush with density and jitter, fill/line/rect tools, animated-tile paint
        with frame sync. Tileset import keeps collision, passable and animated
        flags from Phase 4.
        Done as `blockloom-core/src/tilemap.rs`: 16-case edge and 47-case blob
        sets (Tiled edge/mixed wang sets import as them), a seeded brush, and
        painting an animation's frame paints its base tile. Import is Tiled
        JSON (`.tsj`); TSX and corner wang sets aren't read.
  - [x] Parallax layers: ordered background/foreground layers with scroll factor
        0-2 per axis, wrap/repeat toggle, camera-distance dimming. Layers render
        behind or in front of actors by sorting layer.
        The Parallax component; render-only offsets, and a wrapped layer draws
        its neighbour copies.
  - [x] Level structure: multi-tilemap scenes, room/zone bounds with camera handoff,
        spawn points plus checkpoints, kill zones and ladders/water volumes as
        tile regions. Rooms stream through the Phase 4 cell system as payloads,
        not a second streamer.
        The Room component and `TileRegion`s; 2D runs `StreamingCells` over XY.
        Regions act on a moving body's centre.
  - [x] Editor: tile paint/erase/pick, collision overlay view, parallax preview
        while the camera moves, tileset slice viewer. Stats line: tiles, draw
        batches, colliding rects.
        The Game view's Tiles tool (T) with the sheet under it, region and
        room overlays too; the stats line and autotile/region rows are in the
        Look card.
  - [x] Blocks and scripts: `paint tile _ at`, `tile at x y`, `set parallax of
        layer _ to`, `room containing _`, event `when actor enters room _`.
        Parity cases in `tests/codegen.rs`. ABI_VERSION 29, LOGIC_ABI_VERSION
        25. The hat fires in the actor that entered; scripts read and write
        but don't get the event.
- [ ] 2D lighting and look (the 2D half of the Phase 5 HDR chain; reads the same
      blended `Environment` exposure, never a second EV):
  - [x] 2D lights: `Light2D` component (point/spot, color times intensity,
        range in pixels, falloff, cone, shadows from solid tiles and
        `casts_shadow` circle/rect/image actors via per-light 256-sample
        distance maps), world ambient. Open: lighting is one multiply layer over
        everything under `unlit_above`, not per sorting layer; no normal-map
        lit sprites; GPU path unverified (shader passes naga only).
  - [x] Day/night and glow: ambient tint ramp tied to the time-of-day director
        (`Lighting2d.ramp`, sunrise/sunset), sprite `glow` (0-64) that passes 1.0
        into bloom, seeded light flicker. Unverified on a GPU.
  - [x] 2D post: pixelation (block size), palette or per-channel quantize,
        ordered dither, outline/edge detect, CRT preset (scanlines, curvature,
        vignette, mask), a split to compare per frame. One fullscreen pass after
        the tonemapper, before the UI. Open: per-effect debug toggles beyond
        setting a dial to 0; GPU path unverified (shader passes naga only).
  - [x] Normal-map authoring: height-to-normal bake (`bake-normal-map`, Sobel
        slope with a strength dial, writes `art_n.png` beside the source), a
        lit preview with a movable light dot (`preview-normal-map`, drawn in
        the Sprite card). Open: sprites are not lit through their normal maps
        yet (the lighting layer is a screen multiply with no normal buffer),
        and nothing records which normal map belongs to a sprite.
  - [ ] Blocks and scripts: `set ambient light to` / `set ambient color to`
        (`SetLook2d`, VM and compiled logic, logic ABI 39), `set my light to`
        (existing block, now also works in 2D), reporters `light level at x y`,
        `is night?` done. Open: pixelation, palette levels, dither, outline and CRT dials on the same
        block are done. Open: the script ABI side (`set_look_2d`, `light_level`), left to the script work.
- [x] 2D camera (pixel-correct, deterministic): follow with dead zone, lookahead
      and smoothing already came with PlayerCamera; added axis locks, bounds with
      a soft edge, zoom by view height, pixel snap, roll, trauma shake and
      hitstop, all as `set [dial] to` dials and project settings, plus
      `camera zoom` and `is camera at bounds?` reporters. Open: `set camera target
      to`, `when camera reaches bounds` hat, split-screen, per-actor camera
      rule overrides. Unverified on a GPU.
- [x] 2D physics and movement (builds on rapier2d, joints and one-way platforms
      from Phase 4; this is feel plus helpers):
  - [x] Platformer controller tuning: run accel/decel, air control factor, jump
        velocity plus variable jump height, coyote time, jump buffering, slope
        slide limit, step-up height for stairs.
        (The character controller's step offset covers stairs.)
  - [x] Helpers: moving platforms that carry riders (parent-space delta like the
        actor hierarchy, not parenting), ladders/climb volumes, conveyor belts
        by surface tangent speed, top-down friction/acceleration preset.
        (Carry and `top_down` are Phase 4; the Conveyor component is new. Open:
        ladders are tilemap regions only, no climb-volume actor.)
  - [x] Water and hazards: buoyancy volumes with drag and splash hook, spike/hurt
        volumes with knockback and invulnerability frames.
        (Water/buoyancy are Phase 5; the Hazard component is new. Open: no
        health value, a hazard only throws and broadcasts a message.)
  - [x] Blocks and scripts: `set move speed/jump height/coyote time to`, `is _
        grounded/on wall/in water?`, `launch _ by x y`, event `when _ lands`.
        Sampled on fixed tick so replays stay deterministic.
        (New: `when I land/jump/...` hat, `on wall` reading. Open: no script ABI
        for the hat or the new components.)
- [ ] 2D effects and juice (the 2D path through the Phase 4 particle/trail blocks
      plus screen feedback):
  - [ ] Particles in 2D: sprite-sheet flipbook particles, spawn burst/rate shapes
        (point/line/box/circle), velocity plus drag plus gravity scale, color and
        size over life curves, soft-edge fade near tile collision.
        (Phase 5 VFX already gives 2D CPU particles with shapes, curves and
        flipbooks; left open: soft-edge fade near tile collision.)
  - [x] Trails and feedback: ribbon trails behind fast actors, ghost afterimages
        with lifetime (reuse the existing ghost path), floating damage text,
        squash-and-stretch scale pops on land/hit.
        (Trail and ghost exist from Phase 5; `set sprite Pop` and `FloatNumber` are new.)
  - [x] Screen transitions: fade/wipe/circle wipes between rooms, flash frames,
        slow-mo timeScale curve plus hitstop (same clock rule as cinematics: wall
        clock when paused, fixed tick when running).
        (Flash and cover dials are new; slow-mo, hitstop and the scene veil
        already existed. The iris cover closes to a hole.)
  - [x] Weather lite in 2D: rain/snow/leaf particle presets tied to the weather
        director coverage value, splash rings on ground hit, wind push from the
        global wind asset.
        (Rain, snow and wind-driven leaf motes, with splash rings.)
  - [x] Blocks and scripts: `burst particles _ at`, `trail _ on/off`, `flash
        screen _`, `pop _`, reporters `particle count`, `is screen shaking?`.
        (Pop is `set sprite Pop`. The script ABI for the new dials, hat and
        components is left to the script thread.)
- [ ] Editor, preview and scaling for 2D:
  - [x] 2D inspector tabs: Sprite/Anim/Tiles/Light/Camera, live aspect preview
        thumbnail, flipbook strip viewer (scrub frames, see hitboxes), parallax
        layer stack view.
        (All written in QML with no Qt build here, so unrun. The strip shows
        frames, timings and markers; the first enabled collider is outlined on each frame.)
  - [x] Viewport (pixel grid, light radius, camera bounds, parallax marks, onion skin): pixel grid plus onion-skin ghosts for animation, tile collision
        overlay, light radius gizmos, camera bounds plus deadzone rect, parallax
        depth ruler.
  - [ ] Quality: per-platform sprite atlas budget, particle and decal pool caps
        with LRU steal shared with Phase 5, resolution scale for 2D post, auto-drop
        rule (if frame over N ms for M frames, drop particle density one step).
        Build dialog lists which target keeps 2D lights/shadows and why.
        (Phase 5 already has the auto-drop controller, resolution scale and vfx
        caps in `quality.rs`. Done: per-target atlas sheet size (desktop 4096,
        web and Android 2048) and the Build dialog's 2D note. Decals are 3D
        only, so there is no 2D decal pool. The 2D inspector tabs, strip viewer and
        viewport gizmos are not started: they are QML and need a Qt build.)

### Phase 6.5 - Android games from a desktop PC (player only, do between Phase 6 and Phase 7)

- [ ] Goal: a project builds from Windows/Linux/macOS into an installable
      Android APK and runs standalone on a phone or tablet. No Android editor:
      Qt stays desktop-only (same rule as Phase 9), and Android is a Build
      dialog row, never a Play path.
- [ ] Scope: arm64-v8a devices first (`aarch64-linux-android`), the x86_64
      emulator second (`x86_64-linux-android`) for the dev loop. No 32-bit
      armeabi-v7a in v1. Flat only; Android XR is Phase 10's separate target.
      APK in v1; AAB and store upload stay manual later items.
- [ ] App Settings menu on the main menu (not Project settings):
  - The Dashboard gains a Settings entry (gear button beside the version):
        an App Settings dialog with an Android section - status rows for
        cmdline-tools, platform, build-tools, NDK, platform-tools/adb, JDK
        and Rust targets, plus install/update buttons, an SDK path row and
        an NDK path row (each with a browse button) and license state.
        The paths are settings, not env lookups: Blockloom never reads
        `ANDROID_HOME`, `ANDROID_SDK_ROOT` or `ANDROID_NDK_HOME`.
  - Storage is a new app config file beside `projects.json` under the data
        dir (`library.rs` neighborhood, honors `BLOCKLOOM_DATA_DIR`): SDK/NDK
        paths, the license-accepted stamp, keystore choices (never passwords).
        Per-project rows (applicationId, version, icons) stay in Project
        settings.
  - Backend commands (`commands.rs`/`dispatch.rs`, on the shell/MCP surface
        too): `android-status`, `android-install-sdk`,
        `android-accept-licenses`, `android-device-status`. `just
        android-check` and `just android-sdk-install` are the headless
        equivalents.
- [ ] SDK install flow (the Settings button runs this; headless runs the
      same code):
  - Needs JDK 25 first (the latest LTS): probe `java -version`, else point at a
        download. Installing a JDK silently is not v1.
  - Download Google's cmdline-tools into the SDK path setting (default
        `~/Blockloom/android-sdk`, or under `BLOCKLOOM_DATA_DIR`), then
        `sdkmanager` installs one pinned platform (android-35), matching
        build-tools, platform-tools for adb and a pinned NDK (r27, bumped
        deliberately) into the NDK path setting (default `<sdk>/ndk/<pin>`).
        Pointing either row at an existing install reuses it: the status
        probe validates what is there and only missing pieces download.
  - Show the licenses, then accept on the user's click. Piping yes into
        `sdkmanager --licenses` with no prompt is not allowed: record the
        stamp in the app config. Offline or proxy failure reports what is
        missing and keeps any existing SDK usable.
- [ ] Rust toolchain for Android: `rustup target add aarch64-linux-android`
      (plus `x86_64-linux-android` for the emulator), and NDK linker wiring
      (`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` at the NDK clang wrapper
      per host OS) written to env or a generated cargo snippet that merges
      with - never overwrites - the `.cargo/config.toml` that
      `blockstitch-local` uses. `script::target_installed` already covers the
      std half; add an NDK-link probe beside it so `targets()` can say why
      Android is unavailable. Texture compression needs the NDK C++ toolchain
      for the target (same reason wasm leaves it out); until that links,
      ship PNG/JPEG like web.
- [ ] Build targets and dialog (`build.rs`):
  - `TARGETS` gains `aarch64-linux-android` ("Android (arm64)") and
        `x86_64-linux-android` ("Android Emulator (x64)"). Unlike desktop
        triples these need no staged player under `players/<triple>/`: the
        desktop cross-builds the runtime through the NDK, so `status()`
        checks SDK plus NDK plus JDK plus Rust target instead, with the
        reason attached like every other row. There is no `stage-player`
        for Android; the NDK build replaces it.
  - `hdr_default` is SDR on both with the weak-GPU note (same shape as the
        ARM64 Linux and web rows). The per-platform quality rows already
        planned list Android first: volumetrics off (layers-only fallback
        like WASM), cloud Low, short shadows, small particle/decal pools.
  - `build_android` beside `build_web`: the same game-folder staging (pack,
        assets minus script sources, atlas, baked sky, probes, terrain), then
        an APK assembly from a checked-in template
        (`blockloom-core/src/android-template/`: manifest, activity
        bootstrap, Gradle wrapper pinned to a version that runs on JDK 25) with applicationId,
        versionCode/versionName, adaptive icons and the game files under
        `assets/`, signed and zipaligned with the build-tools on the SDK
        just installed. Scripts and native block logic cross-compile
        (`script::compile_for`, `codegen::compile_for`) into `lib/` .so
        files (see below), never the host's.
- [ ] Runtime compat (`blockloom-runtime`, one `cfg(target_os =
      "android")` pass like the wasm one, no behavior changes):
  - `Launch::Android` like `Launch::Web`: pack and game files from the APK
        (asset-reader path, never `std::fs` beside a binary), synthetic
        `Load`/`Start`, no stdin bridge, no process exit, saves to the app
        data dir, probe bakes ship pre-baked and are never written at
        runtime, SDR only, no Solari, BC6H bakes decode where the GPU cannot
        sample BC (already the web rule).
  - Lifecycle through Bevy/winit suspend/resume: pause strands like `pause
        game` (world freezes, UI strands keep the wall clock), handle
        resize/orientation, route the back button through the UI first (a
        modal swallows it, like clicks) and only then reach world blocks as
        an event. The soft keyboard drives text inputs; safe-area insets feed
        the Phase 2 UI layout.
  - Touch and gamepad are Bevy's own mobile input (multitouch comes from
        Phase 2 already); no second input stack.
- [ ] Scripts and native logic on Android: each script compiles to
      `libscript_<crate>.so` for the Android triple against the same ABI and
      `export!` entry points (linker from the installed NDK, not the host's),
      packaged under the APK's `lib/arm64-v8a/` (or `x86_64/`) since Android
      only loads app lib dirs, and opened by name at world build. A script
      that cannot build for the target fails the build with rustc's error,
      the same as cross-desktop and wasm. Codegen logic ships as one more
      .so on the same path with the VM as fallback.
- [x] Signing and the dev loop:
  - A debug keystore is auto-created with `keytool` on the first Android
        build (data dir, clearly debug-only); release signing takes a
        keystore path plus alias in Project settings and asks for passwords
        on each build (env or OS keyring, never written into the project or
        the app config).
        Done: `ensure_debug_keystore` on first build; `AndroidSettings`
        carries the keystore path plus alias; `resolve_signing` reads each
        password from the typed args, the env, then the OS keyring
        (`android_keyring`: macOS Keychain through `security`, Linux Secret
        Service through `secret-tool`, no new crates; Windows reads as
        unavailable in v1). The Build dialog's remember checkbox (or
        `rememberPasswords` headless) keeps passwords that just signed in
        the keyring for the next build, with status rows, a Forget button
        and `android-forget-passwords`; `create-key` offers the same.
  - The Build dialog gains Install on connected device (behind `adb
        devices` from the installed platform-tools): `adb install -r` plus
        `am start`, with logcat streaming the run log back into RunLog. An
        emulator counts as a device on the x86_64 row.
        Done: `android_install` plus launch with the buffer cleared, then a
        2 s `android_logcat_tail` poll that dumps, clears and appends every
        line to the RunLog (markers as `say`, panics as `error`); emulators
        list as devices on either row.
- [x] Devices tab speed: embedded emulators boot with the host GPU
      (`-gpu host`), more cores/RAM, a log and a failure report; the screen
      is a pushed stream (emulator gRPC, else one `screencap` loop) with
      touch that follows the pointer, instead of 1 fps polled PNGs and an
      `adb` process per tap. Follow-ups: scrcpy-style H.264 for phones
      (`screencap -p` is PNG-compress-bound on the device), arm64 AVDs for
      Apple silicon and arm64 Linux hosts, an Android Studio style
      `-grpc-use-token` handshake.
- [ ] Branding and manifest (`distribution.rs` neighborhood):
  - Adaptive icons generated from the project icon (foreground plus
        background plus monochrome, through the existing `Icons` pipeline),
        app label from the project name, `applicationId` defaulting to
        `com.blockloom.game.<sanitized id>` and overridable in Project
        settings, versionCode/versionName rows, minSdk 29 with the targetSdk
        pinned beside the NDK pin. No permissions in v1 (no INTERNET, no
        VIBRATE): a game that needs none declares none.
- [ ] Tooling and tests:
  - `just android-check` (SDK/NDK/JDK/Rust-target probe, no device needed),
        `just android-build <project> [out]` (through the shell like
        `web-build`), `just android-install` (adb install plus launch). CI
        runs `cargo check -p blockloom-runtime --target
        aarch64-linux-android` once the NDK linker exists there, check-only
        without linking until then.
  - A smoke test beside `web-smoke`: install on an emulator or a connected
        device, launch, watch logcat for the world-built marker and actor
        movement, fail on Rust panics. Unit tests for manifest/template
        substitution, applicationId sanitize, `targets()` Android notes, and
        the app-config round trip.
- [ ] Not in v1, by decision: no editor on the device; no AAB or store
      upload automation (signed APK file only); no 32-bit targets; no ray
      tracing, HDR output or EXR capture on Android.

### Phase 6.6 - UWP / Microsoft Store for Windows (player only, do between Phase 6 and Phase 7)

- [ ] Goal: a project builds from a Windows PC into a sideloadable and
      Store-ready package and runs standalone on Windows 10/11. No editor
      on the device: Qt stays Win32 desktop-only (same rule as Phase 6.5),
      and Store is a Build dialog row, never a Play path.
- [ ] Upfront truth, read before scoping: UWP as a XAML/CoreWindow app
      model is legacy (Microsoft steers new apps to Windows App SDK plus
      MSIX), Rust `*-uwp-windows-msvc` std is broken upstream (uses
      Win32 APIs the store partition forbids), and winit has no WinRT
      backend (Bevy plus wgpu plus audio therefore have no UWP window
      path today). So v1 ships Track A only; Track B stays parked behind
      a spike.
- [ ] Two tracks, do in order:
  - Track A first (this phase): MSIX-packaged Win32 through the Desktop
        Bridge. Same player binary as the Windows x64/ARM64 rows, wrapped
        with an AppxManifest plus tile assets and signed with the Windows
        SDK tools. Store-accepted, sideloadable, no runtime rewrite. This
        is what "UWP for Windows" means in v1.
  - Track B later (gated): true UWP sandbox (`x86_64-uwp-windows-msvc`,
        `aarch64-uwp-windows-msvc`, CoreApplication sandbox, read-only
        install location, suspend/resume lifecycle). Needs a windowing
        backend that does not exist upstream yet.
- [ ] Spike first, before any Track B code: `cargo check -p
      blockloom-runtime --target x86_64-uwp-windows-msvc` on a Windows
      runner, plus a minimal Bevy plus wgpu plus audio probe for window
      creation, swapchain, input, file reads under the install location,
      and suspend/resume. Park Track B unless std plus winit plus Bevy
      all pass; Track A never waits on it.
- [ ] Scope: x86_64 desktop first, ARM64 desktop second (Surface-class
      devices). MSIX in v1; APPX is the same container with the older
      extension, not a second format. No 32-bit in v1. No Xbox (GDK is a
      separate path, not APPX UWP), no HoloLens remoting in v1. Packaging
      runs on a Windows host only in v1: other hosts list the rows as
      unavailable with the reason attached.
- [ ] Project settings plus app config (`uwp.rs` beside `android.rs`,
      Project settings rows like `project.android`):
  - Package identity name (default `com.blockloom.game.<sanitized id>`),
        publisher subject, four-part version, Desktop device family,
        capabilities (none by default; `internetClient` is opt-in, a game
        that needs none declares none).
  - Tile plus splash plus Store logo generated from the project icon
        through the existing `Icons` pipeline.
  - App config file beside `android.json` under the data dir (honors
        `BLOCKLOOM_DATA_DIR`): Windows SDK location, test-cert
        thumbprint, license or Store association state. Keystore and PFX
        passwords never land there.
  - Backend commands (`commands.rs`/`dispatch.rs`, on the shell/MCP
        surface too): `uwp-status`. `just uwp-check` is the headless
        equivalent.
- [ ] Build targets and dialog (`build.rs`):
  - Track A adds Store rows that reuse the `pc-windows-msvc` compilers
        (no new Rust target in v1): e.g. "Windows Store (MSIX x64)" beside
        "Windows x64". Readiness is SDK tools plus staged player plus Rust
        target when the project has scripts, with the reason attached like
        every other row. There is no `stage-player` change: the staged
        Win32 player is the payload.
  - Track B triples (`*-uwp-windows-msvc`) appear only if the spike
        unblocks, with an `is_uwp()` helper beside `is_android()`/`is_web()`.
  - `hdr_default` matches desktop (HDR where the display offers it),
        unlike the Android and web SDR-only rows. The per-platform quality
        rows list Store beside desktop with no weak-GPU caveat.
  - `build_uwp` beside `build_android`/`build_web`: same game-folder
        staging (pack, assets minus script sources, atlas, baked sky,
        probes, terrain), then package assembly from a checked-in template
        (`blockloom-core/src/uwp-template/`: AppxManifest, tile assets,
        `resources.pri` layout) with MakeAppx from the installed Windows
        SDK, signed with SignTool. Scripts and native block logic ride as
        the same `.dll` files desktop ships; whoever calls this compiled
        them first, same as every other target.
- [ ] Runtime compat: Track A needs none (full-trust Win32 inside MSIX,
      one `cfg` check that it is running packaged, no behavior changes).
      Track B is a second `Launch` like `Launch::Web`/`Launch::Android`
      when it happens: pack and game files from the install location
      through an asset-reader path (never `std::fs` beside a binary),
      synthetic `Load`/`Start`, no stdin bridge, no process exit, saves
      to the app data LocalFolder, probe bakes ship pre-baked and are
      never written at runtime, suspend/resume pauses strands like `pause
      game` (world freezes, UI strands keep the wall clock), resize and
      orientation handled, no second input stack.
- [ ] Scripts and native logic on Store: Track A ships what desktop
      ships (same triple, same `.dll` names, VM as fallback). Track B
      cross-compiles each script plus codegen logic for the UWP triple
      against the same ABI and `export!` entry points; a script that
      cannot build for the target fails the build with rustc's error, the
      same as cross-desktop and wasm.
- [ ] Signing and the dev loop:
  - A test cert is auto-created on the first Store build (clearly
        test-only, like the Android debug keystore); release signing takes
        a PFX path plus thumbprint in Project settings and asks for the
        password on each build (env or OS credential store, never written
        into the project or the app config). The Build dialog's remember
        checkbox keeps a password that just signed for the next build,
        with a Forget button.
  - The Build dialog gains Install on this PC (behind `Add-AppxPackage`,
        needs sideloading or dev mode on): install plus launch, with the
        run log tailed back into RunLog from a log file under LocalFolder
        (there is no stdout or logcat in a package). Document the dev-mode
        plus cert-trust steps next to `just player`.
- [ ] Tooling and tests:
  - `just uwp-check` (SDK plus Rust-target probe, no device needed),
        `just uwp-build <project> [out]` (through the shell like
        `web-build`). CI validates the manifest template and runs `cargo
        check -p blockloom-runtime --target x86_64-uwp-windows-msvc` on a
        Windows runner only (check-only without linking until the std
        half unblocks); Linux CI never attempts it.
  - A smoke test beside `web-smoke`: install the MSIX on Windows,
        launch, watch the log tail for the world-built marker and actor
        movement, fail on Rust panics. Unit tests for manifest
        substitution, identity sanitize, quad-version validation,
        `targets()` Store notes, and the app-config round trip.
- [ ] Not in v1, by decision: no editor on the device; no Partner Center
      upload automation (signed `.msix` file only); no 32-bit targets; no
      Xbox GDK path, no HoloLens remoting; no new HDR, ray tracing or EXR
      behavior beyond desktop; Track B true-UWP sandbox stays parked until
      Rust std plus winit plus Bevy unblock upstream.

### Phase 7 - Scale and ecosystem, do last
- [ ] Multiplayer: headless server, replication, lobbies, rollback (`docs/multiplayer-and-embedded-server-plan.md`):
    - [x] Phase 0, transport spike: `blockloom-net` (Quiche, pinned self-signed identity, Retry, streams, datagrams, seeded loss/delay harness) and the findings in `docs/multiplayer-phase0.md` (extraction inventory, reporter domains, clock facts, decisions needed).
    - [x] `tokio-quiche` spike (dev-only test): works, not adopted; raw `poll()` driver stays (`docs/multiplayer-phase0.md` 1.1). Retry-skip-with-token is not possible at the QUIC layer; adaptive Retry proposed.
    - [ ] Phase 0 leftovers: Windows/macOS/Android Quiche builds, VM and sensing baselines against the built player (needs a GPU).
    - [ ] Phase 1: private authoritative simulation:
        - [x] Clocks: `wall` is real time (`Engine::wall_time`), run clocks use `elapsed_secs_f64`.
        - [x] Contacts relayed on the fixed tick (the physics work's `contacts.rs`: `track_contacts` per step, `world::deliver_contacts` at the next tick).
        - [x] `add_world` split: `simulation::add_simulation` (fixed step and physics) with presentation ordered around it; headless 2D/3D harness in `simulation::tests` (determinism, VM vs native logic).
        - [x] `VolumeEye`: volumes weighed at a named actor or point instead of the camera.
        - [ ] Split the `Update` chain and `rebuild_world`/`apply_lifetimes` (simulation spawn vs visuals), move the tick-sampled part of `publish_sensors` into the fixed tick, virtual-delta cap and own accumulator.
    - [ ] Phases 2 to 6 as in the plan (players and authoring, LAN and late join, prediction and dedicated server, plugin/save scope, browser and internet).
- [ ] Deploy: Web/WASM (see Phase 8 player and Phase 9 editor), Android signing (see Phase 6.5), Windows Store signing (see Phase 6.6), iOS signing, console path, auto-updater/DLC/addressables.
- [ ] Ecosystem: analytics/crash, achievements/IAP hooks, asset store, collab/VCS, docs/LTS.
- [ ] Plugin platform (`docs/plugin-system-and-voxel-plan.md`; decisions in `docs/plugin-adr-0001.md`):
    - [x] Phase 0/1: api and host crates, manifests, resolver, lock file, immutable cache, transactional install/rollback/sync/gc, folder registries, C ABI v1 with measured call cost, sealed proof package.
    - [x] Phase 2 data path: namespaced records on actors and the project (lossless when the plugin is missing), schema validation and migrations, declarative components/resources/commands, shell and MCP access, Play/Build preflight, pack v2 plugin payload.
    - [x] Plugin Manager dialog in QML (list, install with preview, update, remove, sync, undo, cache clean, record issues, commands).
    - [x] Schema-generated inspectors for plugin components (every field type, lists, missing/migration notes, Add component) and plugin settings in the Plugin Manager.
    - [x] Declarative inspector layout: field `ui` hints (label, slider, multiline, unit, step, `visible_when`) and component `inspector` groups.
    - [x] Declarative plugin panels (text, resource forms, command buttons) in a top bar dialog.
    - [x] Trusted editor modules: `editor.modules` QML loaded at run time behind a per-user trust ledger bound to the package hash (window-only `plugin_trust`), a `host` bridge, the Plugin editors dialog and `plugins/examples/com.example.notes`.
    - [x] Plugin-drawn inspector sections: `editor.inspectors` trusted QML replaces the schema form for one component (`com.example.notes` sticky).
    - [ ] Native compiled editor modules: left out on purpose (host-level code in the Qt process, no bounding ABI); trusted QML covers custom UI.
    - [x] Native modules load in the editor for `module` commands (`Modules` cache, run-log output, panic containment).
    - [x] Portable (WASM) executor in the editor: wasmi, linear-memory ABI, memory and work limits, stop-and-reload on a fault.
    - [x] Plugin SDK crate (`blockloom-plugin-sdk`): one `Plugin` trait and `export_plugin!` for native and WebAssembly, `NativeModule::from_entry`, and the `plugins/examples/tally` example (`just example-plugin`).
    - [x] Native and portable modules in the running game world: loadout sent before Play, `world.start`/`world.stop` lifecycle, staged hooks, module-op blocks run in the world, effects (`say`, `broadcast`, `error`).
    - [x] Modules in the built desktop player: the build ships each plugin's manifest and files, the player verifies them against the pack and hosts the loadout; module-op blocks, reporters and hats build.
    - [x] A browser host for portable modules: the host crate builds for wasm32 (wasmi, no dlopen), `files::set_reader` reads shipped plugins from the page's mounted files, the web build ships and records portable plugins, the web player hosts the loadout. Checked by an in-memory load test and the wasm32 clippy; no browser run yet.
    - [x] Android plugin code: portable modules only, plugins ship in the APK assets, runtime built with `--features plugins` (compile-checked, not run on a device).
    - [ ] Browser proof (`just web-build` with a plugin project, `just web-smoke`) and a faster host on the browser's own WebAssembly (needs a fuel substitute).
    - [x] Plugin statement blocks in the VM: `PluginBlock` instruction, `Effect::PluginCall`, the editor runs the block's command, `plugin-run-block`, Play preflight; codegen emits them (`Act::PluginCall`, `ACT_PLUGIN_CALL`, logic ABI 34) and Build is refused for a statement whose command runs in the editor.
    - [x] Plugin statement blocks in the palette and on the canvas (one `PluginBlock` row whose head follows the schema label; needs blockstitch's function `head` and `index` pieces).
    - [x] Plugin reporters and hats: `PluginRead` values answered on demand by the world's modules, `WhenPlugin` hats started by `event` effects; palette, canvas, preflight, tally example (needs blockstitch's operator `layout`).
    - [x] Per-plugin MCP tools: `plugin-commands` becomes one typed tool per command (`plugin-id__name`), kept in step after install, remove, open and undo.
    - [x] Codegen for plugin statements, reporters and hats, held against the VM in `tests/codegen.rs` and through the player boundary in `logic.rs`.
    - [x] The script ABI for plugin blocks: `plugin_call`, `plugin_number`/`plugin_text`, `Event::Plugin` (`ABI_VERSION` 36), tested through a built script.
    - [x] Importer/build hooks: schema `importers`/`build`, byte-framed module ops with host-mediated IO and a per-call budget, `<file>.imported/` outputs tracked in `.blockloom/imports.json`, auto-import on asset import, `plugin-importers`/`plugin-imports`/`plugin-import`, build hooks staged into `game/plugins/<id>/cooked/`, `plugins/examples/palette`.
    - [x] Importer follow-ups: asset tray import badges and re-import, `plugin-reimport` (also run before Play and Build), build hook files in Android builds.
    - [x] HTTP registry transport (`HttpRegistry`, https or loopback, index read once, archives verified against the index hash); revision bump so attached copies follow package changes.
    - [x] Mesh submission service: `mesh`/`remove_mesh` effects become named 3D entities with an optional trimesh collider.
    - [x] Voxel plugin first slice (`plugins/voxel`): finite cube world, seeded terrain presets, greedy chunk mesher, glowing materials, live set/fill/sphere/generate, reporters, rays (distance, break, place), schema and sealed package (`just voxel-plugin`).
    - [x] Scene-view preview: a plugin with `editor.preview` is hosted while nothing plays (`plugins::preview`), the voxel world shows in the editor without Play.
    - [x] Voxel edit persistence: saved `edits` lines on the `world` resource, `add_voxel_edit`/`clear_voxel_edits`, and a general `set_resource_field` command action.
    - [x] Voxel shaped cells: slab, top slab, post, stairs and ramps (four facings), `shape_voxel`, and a `shape` saved-edit line.
    - [x] Plugin scene tools: a package's `tools` (cast op, command, `$hit`/`$option` argument sources, typed options) are toolbar tools in the 3D scene view; a click casts the pointer ray through the hosted preview module and runs the command (`plugin-run-tool`). Voxel paint, erase, ball and shape tools, and `paint_voxel*` template commands, use it.
    - [x] Scene tool follow-ups: `drag` strokes (one undo step, batched hits) and `outline` boxes under the pointer; voxel tools use both (`PROTOCOL_VERSION` 28).
    - [x] Host services hub (`host.*`, `rng.*`, `storage.*`, `save.*`, `diag.*`, `jobs.*`, plus chained providers) with atomic project and player-save stores.
    - [x] Jobs (sliced, prioritised, cancellable, event on end) and node-graph generation (`nodes`, `graph.evaluate`, tile cache with margins).
    - [x] Physics and navigation services for plugins (`physics.ray|overlap_point|overlap_sphere`, `nav.available|path`, `nav_dirty` effect, collider kinds).
    - [x] Diagnostics: per-plugin counters, timings, errors and faults in the Plugin Manager, shell (`plugin-diagnostics`) and profiler.
    - [x] Editor surfaces: plugin menus, shortcuts and scene overlays (`PROTOCOL_VERSION` 29), and plugin conflicts (`conflicts`, shared services, hook orders) that block Play and Build.
    - [x] Rendering service: instanced plugin meshes and plugin WESL shader modules (`blockloom::plugin_<id>_<name>`).
    - [x] Reliability: live reload of portable modules (`world.save`/`world.restore`), process isolation of native libraries (`blockloom-plugin-worker`, `BLOCKLOOM_PLUGIN_ISOLATION`), wall-clock call limits.
    - [x] Authoring kit: SDK `testing` harness, `plugin-new` scaffolds (tested), `docs/plugin-api.md`, CI steps for the kit and isolation.
    - [x] Plugin follow-ups (twenty-third batch): GPU compute kernels (host-owned buffers, validated and loop-bounded WGSL, per-frame budget, async reads; `gpu-compute` capability, `blockloom-plugin-gpu`), web persistence of plugin saves (localStorage `KvStore`), `plugin-data-gc`, native template and `plugin-add-native`, and the plugin worker packaged beside the editor.
    - [ ] Plugin follow-ups left: a published SDK crate, QML and real-GPU runs of the new surfaces, instancing and compute (the GPU half was run on lavapipe only), WESL imports and textures for kernels, a plugin kernel inside a running game.
    - [ ] Voxel next: smooth terrain, streaming/LOD, instancing, GPU meshing, fracture.
    - [ ] Phases 4-7 (procedural graph, GPU path, fracture, ecosystem).

### Phase 8 - Web player via WebGPU (single-file build, do before Phase 9)

- [x] Goal: a built game ships as one self-contained file (single `.html`:
      inlined wasm plus the pack plus assets) that runs in a browser over
      WebGPU with no server beyond static hosting. Player only; the editor
      stays native (see Phase 9).
- [x] Web toolchain: `wasm32-unknown-unknown` target for `blockloom-runtime`
      with `--no-default-features` (no `ray_tracing`/Solari on web; `is ray
      tracing on?` and `ray tracing available?` report false). Check-build the
      pinned rapier git rev for wasm early; Bevy 0.20 WebGPU backend plus the
      WESL surface shaders must compile to it (the `shader_lib` naga check
      covers the offline half).
      Done: `just web-check` is green with the same 3 warnings as native,
      the pinned rapier rev compiles for wasm, and `web-time` stands in
      where `std::time::Instant` panics without an OS clock (`preview.rs`,
      `performance.rs`). WebGL2 (not full WebGPU) is what headless browsers
      here offer, and the game boots on it.
- [x] `cfg(target_arch = "wasm32")` gating (one pass, no behavior changes):
      no stdin/stdout bridge thread (`bridge::listen`), no `fatal` process
      exit (canvas error overlay instead), Linux-only embed/`ash`/dma-buf
      stays out, no HDR swapchain takeover in `display.rs` (web builds force
      `GamePack.hdr = false`, SDR only), no EXR capture file writes, probe
      bakes ship pre-baked from the build and are never written at runtime.
      Done, plus two things the headless runs shook out: player-side errors
      go to the devtools console (printing panics on wasm), and a panic hook
      forwards Rust panics there with their message.
- [x] Rust scripts work on web: at build time each `assets/scripts/*.rs` is
      compiled for `wasm32-unknown-unknown` against the same `HostApi`/ABI
      and `export!` entry points, so script behavior matches native. The
      build machine needs the wasm target `std`; a project whose scripts
      cannot build for wasm fails the web build with the rustc error.
      Blocks run on the VM on web v1 (native codegen logic stays native).
      Done differently from the plan: not linked into the player wasm, which
      would mean rebuilding Bevy per game, but one small wasm module per
      script (debug info stripped, ~50 KB) that the player instantiates.
      Modules can't call each other through function pointers, so on wasm
      the prelude reaches the three host calls as imports (`abi::WasmCall`
      in the script's memory; ABI 28) and the runtime's `browser` module
      answers them with the native host code. A panic logs its message
      through the prelude's hook, then the trap stops that script for the
      game. Verified headless: a script moving an actor, reading its
      position and name, and a panicking script stopped with its message.
- [x] Single-file packaging in `build.rs`: the Web target
      (`wasm32-unknown-unknown`) beside the native triples lays out one
      `.html` with the
      wasm player, its glue, `game.pack`, assets, the atlas, the baked sky,
      probe bakes and script modules in one gzip'd, base64'd archive,
      unpacked with `DecompressionStream` and mounted in memory
      (`blockloom_core::vfs` plus a Bevy asset reader), so nothing is
      fetched and the page opens from disk. BC6H sky and probe bakes ship as
      they are, since the runtime already decodes them where the GPU can't
      sample BC. The Build dialog says what it costs (whole file before
      start, a third over gzip) and shows the page's size.
- [x] Runtime compat: `Launch::Web` with the pack from memory and synthetic
      `Load`/`Start` (never stdin); saves keyed by `GamePack::save_id()` go
      to `localStorage`; audio starts behind a click-to-play cover; the
      canvas follows its parent (`fit_canvas_to_parent`); pointer lock is
      asked for on a click on the canvas; touch and gamepads are Bevy's own
      web input. Renders through WebGPU (Bevy's `webgpu` feature; the old
      build was WebGL2-only), so the sky, fog, cloud and luminance compute
      passes run, and a browser without WebGPU gets a plain message. Bevy's
      material shaders are patched on wasm where Chrome's WGSL compiler is
      stricter than naga (`let` of textures and samplers).
      Not covered: touch and gamepad on real devices, and the headless
      runs use a software adapter.
- [x] Tooling and tests: `just web-player` (stage the player), `just
      web-build <project>` (the single file, through the same build as the
      dialog), `just web-serve`, `just web-smoke <page>` (Playwright:
      unpack, click to play, world built, scripts open, actors move, no
      errors - read through the player's `game_actors()`, since headless
      Chromium can't screenshot a WebGPU canvas). wasm-bindgen CLI rather
      than trunk or wasm-pack, since the build dialog, not a bundler, makes
      the page. CI clippy-builds the runtime for wasm and compiles a script
      to wasm. Single-threaded wasm; shared-memory threads plus COOP/COEP
      stay a later opt-in.

### Phase 9 - Editor on web (separate phase, do after Phase 8)

- [ ] Goal: edit block projects in a browser. Qt cannot go to web, so this
      needs a new browser editor (the retired `ui/` Vue frontend and its
      `dev-bridge` backend were removed), with the Phase 8 web player as its
      preview canvas. No new command surface: reuse `Backend::dispatch`
      commands and the shell/MCP command surface.
- [ ] Scope: block canvas and inspectors in the browser, web-player preview with
      input forwarding, project storage (File System Access API locally or
      server-side folders when hosted). Native-only pieces stay native:
      Qt Game-view GPU sharing, `rustc` script builds (scripts compile on a
      server or at export, not in the browser), staged native players.
- [ ] Rule: start only once the single-player loop plus Phase 8 are solid;
      Phase 9 never blocks web-player shipping.

### Phase 10 - Virtual Reality via OpenXR (3D only, do after Phase 7 deploy footing)

- [ ] Goal: a 3D project can run on a connected headset, and pressing Play
      with a headset attached shows the game on the headset while the
      editor's Game view keeps a flat mirror. One OpenXR path covers every
      headset; no per-vendor SDK in the runtime.
- [ ] Platform truth table (OpenXR runtimes decide this, not Blockloom):
      Windows and Linux desktop through the active OpenXR runtime (SteamVR,
      Monado on Linux, Quest Link / Windows Mixed Reality on Windows);
      Android standalone headsets through the Khronos Android loader
      (Quest first, Pico / HTC Focus-class devices follow the same loader
      path where their runtime supports it). macOS has no consumer OpenXR
      runtime and iOS has none either, so both stay flat with a logged
      reason. visionOS is not targeted. WebXR is a later opt-in on top of
      the Phase 8 web player, not v1.
- [ ] Runtime foundation (gated, falls back to flat):
      cargo feature `xr` on `blockloom-runtime` wrapping a community
      OpenXR backend in the `bevy_mod_openxr` / `bevy_oxr` lineage, pinned
      to the workspace Bevy; when the backend lags a Bevy bump the feature
      compiles out and XR reports unavailable instead of breaking the
      build. Desktop binds Vulkan (D3D12 option on Windows only), Android
      binds Vulkan through the Khronos loader (pin >= 1.0.34, the first
      version Quest OS v62+ accepts). Owns session lifecycle
      (create/begin/end, playspace vs seated, eye height), the stereo
      swapchain, and per-eye views fed by the existing world camera and
      blended `Environment`. No XR session means the flat renderer carries
      on and the editor hears why once. 2D worlds stay flat; XR is 3D only.
- [ ] Editor live preview (headset mirror, not a second Game view):
      a "Preview on headset" toggle beside Play opens the XR session on
      the editor machine and mirrors one eye (or side-by-side) into the
      Game view ring on Linux or the MJPEG preview on the child-process
      path, at mirror resolution rather than headset resolution so the
      editor stays interactive. Status reports session state
      (idle/searching/running) plus headset presence; a missing runtime or
      headset logs once and plays flat. Pause/step, run log and status
      keep working; keyboard/mouse still drive flat input while controllers
      drive XR input. Document the Linux SteamVR setup (active runtime
      json or `XR_RUNTIME_JSON`) next to `just player`.
- [ ] Camera rig and input (builds on the Phase 2 input actions, not a
      second input stack): the `Camera` component gains a VR mode
      (head-tracked, seated/standing/room-scale with configurable eye
      height); the HMD pose drives the rig each render frame while blocks
      and physics keep reading the fixed-tick snapshot with the existing
      pose-interpolation rule. Controllers appear as tracked poses with
      buttons, triggers, sticks and grips routed through input actions
      (remappable like gamepad), plus haptics through the Phase 2 rumble
      path. v1 is controllers only; hand tracking, eye tracking and
      passthrough/mixed-reality are later items, not v1.
- [ ] Blocks and scripts (sampled on the fixed tick so VM and codegen
      agree, same rule as other reporters): `is headset connected?`,
      `headset x/y/z`, `controller _ pressed?`, `controller _ position`,
      `rumble controller _ by _`, `move VR rig to`, `snap-turn _ by _`;
      event `when controller _ pressed`. Teleport locomotion is a block
      recipe (raycast from Phase 1 physics queries plus rig move), not a
      built-in locomotion system. Comfort defaults ship in the template:
      snap turn on, smooth turn opt-in, optional vignette while moving.
- [ ] Performance and comfort (numbers on existing machinery, no new
      pipeline): the XR runtime owns frame pacing (72/90/120 Hz by
      headset), sim stays on the fixed tick and renders interpolate as
      they do today. Stereo MSAA, fixed-foveated rendering where the
      extension exists, and dynamic resolution plug into the Phase 5
      scaling policy and quality presets; over budget loses density or
      pixel rate before it loses tracking. Profiler lines for per-eye ms,
      dropped vs reprojected frames, and session state.
- [ ] Build and packaging (extends the Phase 7 deploy path, not a second
      one): desktop players keep the system-loader lookup (nothing
      vendored) with a troubleshooting note in the Build dialog; Android
      XR is a build target beside the flat Android target (arm64 APK with
      OpenXR manifest entries, loader version pin, Quest signing through
      the same signing flow as flat Android). `build::targets` lists which
      targets can do XR and why a missing one cannot (no runtime, no
      loader, scripts need the target `std`). Quest Link stays the desktop
      dev loop; standalone APKs are the shareable artifact.
- [ ] Tooling and tests: `just xr-check` (extension + runtime probe without
      a headset where possible, e.g. Monado null/emulated runtime) plus a
      headset-less CI smoke (world builds with `xr`, session absent means
      flat start, mirror path emits frames). `embed.rs`-style ignored GPU
      tests gain a stereo-mirror case when hardware is present, run one at
      a time like the existing ones.

### Qt6 rewrite - in-process Game view
- [x] Goal: docked Game view with no sidecar video and no extra OS window.
- [x] Step 2 - headless world thread (true in-process, Linux): Bevy runs windowless on its own thread of the editor, `RuntimeHandle` talks to it over channels. A panic ends the run, not the editor.
- [x] Step 3 - GPU texture sharing (Linux): the camera renders offscreen and each frame lands in a ring of dma-bufs, imported into Qt's GL through EGL. Wayland and X11.
- [ ] Step 4 - input parity:
  - [x] Keys, mouse buttons and position, text, focus.
  - [x] Pointer lock plus raw deltas (Wayland pointer constraints, X11 warp fallback)
  - [x] Scroll wheel and touch.
  - [ ] Gamepads: probably already read straight from the system in-process - verify.
  - [x] Keyboard by physical key, not Qt key name (non-QWERTY layouts land WASD elsewhere). Right Shift/Ctrl/Alt arrive as left; numpad, brackets, quote and backtick aren't sent. macOS has no scan code in Qt and still goes by key name.
- [x] Update CLAUDE.md: it still describes the world as a separate process with its own window.
- [x] Frame pacing: the world runs on its own 60 Hz timer, not the display's, so 120/144 Hz screens get uneven frames. Tie it to Qt's frame signal.
- [x] Log stutter: every `say`/error line still emits the whole editor state to QML. Give log lines their own event like status has.
- [x] Zero-copy on NVIDIA: frames are copied to a linear system-memory image, then again through an external-texture pass on the Qt side. Share the image in its native tiled layout (DRM format modifiers) to drop both copies.
- [x] Resolution: the world renders at the Game view's real pixel size, with an aspect ratio (default 16:9, or Free) and a resolution (default Free, or a fixed size the view is sized to) picked in the Game tab. Code and Game are tabs of one editor area.
- [ ] Other platforms: Windows (shared D3D or Vulkan handles) and macOS (IOSurface) still use the child process plus MJPEG, which stays until they're ported.
- [ ] Crash isolation: a native crash (script cdylib, GPU fault) takes the editor down. Decide whether scripted projects should keep the separate process.

Rule: do 1-4 before 5-8, do 9-11 before adding new block surface in 12-15, leave 17-19 until single-player shipping loop is solid.
