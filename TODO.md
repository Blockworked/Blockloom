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
- [ ] Save slots/profiles plus localization: builds on save system we have.
- [ ] Multiple scenes plus loading between scenes (menu, level 1, level 2):
  - [ ] Document: `Project` holds a scene list (each with its own actors and
        `World` settings); one active scene; old single-scene docs migrate as
        scene one; scene add/rename/duplicate/delete with undo.
  - [ ] Mode: v1 supports mixed 2D/3D scenes; `Mode` lives per scene and a
        scene switch across dimensions rebuilds the dim2/dim3 pipeline plus
        rapier backend the way a project dimension switch does today.
  - [ ] Blocks and scripts: `switch scene to _` (plus `with transition _`),
        reporters `current scene`, `scene names`, events `when scene
        starts/ends`; globals plus save data cross scenes, actor locals do
        not; opt-in survivors later; fixed-tick sampling so VM and codegen
        agree, with parity cases in `tests/codegen.rs`.
  - [ ] Runtime: unload the current world, load the scene doc the way
        `EditorMessage::Load` does now, rebuild and warm up before the green
        flag continues; transitions run on the wall clock like UI strands;
        rooms stay intra-scene camera zones, not scenes.
  - [ ] Editor and tooling: scene picker plus per-scene actor list/canvas and
        World settings; project folder, pack, build and web carry all scenes;
        shell/MCP commands (`add-scene`, `switch-scene`, ...).

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
        Not covered: DLSS Ray Reconstruction (no build carries the SDK);
        converging stills in the inspector; cookies and IES profiles aren't
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
  - [ ] Decals (transient marks only; lasting stains live in the destruction map
        below): deferred projected (albedo/normal/roughness/emissive, atlas pages,
        angle fade, depth reject to avoid floating edges), pool with LRU steal plus
        per-decal lifetime/fade, blood/footprint/fresh-scorch presets. Blocks:
        `spawn decal _ at`, `fade decals in radius`. No persist toggle: anything
        that must survive reload goes through the scorch/wetness map instead.
  - [ ] Destruction and fluids lite: fracture-on-hit (Voronoi cell count, interior
        cap material, impulse threshold, shard lifetime/sleep/pool cap), debris
        impulse inheritance plus bounce sounds, 2D shallow-water ripple grid for
        puddles/ponds (rain rings, footstep rings, shore reflect), smoke advection
        grid for stylized chimneys and dust puffs (no full 3D sim), persistent
        scorch/wetness map (world-space RT, dries over time, darkens albedo and
        raises specular): the sole owner of lasting surface state, read by the
        material mask stack. Blocks: `fracture _`, `splash at`, `puff smoke at`.
  - [ ] Post volumes (full HDR chain, volume-blended): exposure (auto spot-meter
        with min/max and speed writes `Environment.exposure` when enabled, else the
        manual EV stands; both lose to the director track per precedence), bloom (threshold/knee, 5-mip scatter chain,
        dirt texture), tonemap (ACES/Neutral/AgX select, toe/shoulder), white balance
        plus LUT/grading (lift/gamma/gain, saturation, contrast), vignette, depth of
        field (autofocus target or fixed distance, bokeh blades/circular, near/far),
        motion blur (shutter angle, per-object toggle), SSAO (HBAO, radius/intensity),
        SSR toggle with roughness cutoff, chromatic aberration, film grain, sharpen.
        Order fixed HDR-first; debug splits (bloom mip, CoC, AO only).
  - [ ] Performance and scalability (whole-frame budgets for the stack above):
        - Draw policy (numbers on the Phase 4 mechanisms, no new machinery):
          which meshes instance (vegetation, props, debris, decals) and at what
          density, indirect-draw batch membership, per-system draw-call and
          triangle budgets surfaced in the profiler. A system over budget loses
          density or distance before it loses features.
        - LOD and throttle policy (thresholds, not selectors): screen-size and
          distance cutoffs for terrain chunks, trees, water tiles and VFX, cloud
          step counts by distance and weather weight, probe and shadow update
          throttling (staggered refresh, frozen static probes, cascade shrinking).
        - Content streaming (payloads on the Phase 4 cell system, not a second
          one): terrain chunk data, noise volumes, HDRI mips and probe captures
          register as streamable payloads with per-type priority and eviction
          policy. No new hysteresis or prewarm logic here.
        - Resolution scaling: dynamic resolution driven by frame-time feedback,
          spatial upscaler plus temporal anti-aliasing path, half-res volumetrics,
          fog and SSR with bilateral upsample, reflection and shadow resolution
          budgets per quality preset.
        - DLSS (Bevy `dlss` path on NVIDIA RTX): configurable mode (DLAA, Quality,
          Balanced, Performance, Ultra Performance) plus sharpness, driven by the
          same dynamic-resolution signal. Vendor the DLSS redistributable in player
          builds, probe capability at startup, fallback chain DLSS to TAA plus
          spatial to spatial-only, per-platform toggle (off on WASM and weak
          targets), editor override with a warning when unavailable. Shares the
          jittered-camera and motion-vector plumbing with the TAA path and denoises
          Solari output where Bevy exposes ray reconstruction.
        - Memory: texture streaming with distance-based mip bias, BC/BC6H compression
          defaults, noise and LUT atlasing, pool caps for particles/decals/shards
          with LRU steal. One quality preset maps onto every dial above, plus an
          auto-drop rule shared with the editor scaling panel.
        - Blocks and scripts: `set quality/resolution scale/upscaler/DLSS mode to`, reporters
          `frame time`, `draw calls`, `current quality`, `is DLSS available?`, event `when quality drops`.
  - [ ] Time-of-day and weather director (the thing that makes it shippable):
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
  - [ ] Cinematics: timeline tracks (camera cut, transform, FOV, volume weight, signal
        fires block at marker), dolly/crane spline path with look-at target plus roll,
        camera shake (trauma 0-1, Perlin translation/rotation noise, decay), letterbox
        bars plus fade to black/white, slow-mo (timeScale curve) plus hitstop frames
        blocks, skip support (`skip cutscene` jumps to end marker). Plays on wall clock
        even when `pause game` freezes world strands, like UI strands do.
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
  - [ ] 2D lights: point/spot/ambient per sorting layer, color times intensity,
        range in pixels/meters, normal-map toggle for beveled sprites. Shadows
        as projected 2D occluders from solid tiles and circle/rect actors.
  - [ ] Day/night and glow: ambient tint ramp tied to time-of-day director,
        emissive/glow sprites that pass 1.0 into bloom, light flicker noise for
        torches and neon.
  - [ ] 2D post: pixelation (fixed pixel size), palette quantize, outline/edge
        detect, CRT scanline/vignette preset, dither toggle. Order fixed after
        tonemap; debug splits per effect.
  - [ ] Normal-map authoring: height-to-normal bake on import, strength dial,
        preview thumbnail with a movable light dot.
  - [ ] Blocks and scripts: `set ambient light to`, `set light intensity of _ to`,
        `set pixelation to`, reporters `light level at x y`, `is night?`.
- [ ] 2D camera (pixel-correct, deterministic):
  - [ ] Follow: target actor, deadzone rect, lookahead by velocity, smoothing time,
        axis locks. One camera per project like 3D, attached through the existing
        Camera component.
  - [ ] Bounds and zoom: confine rect plus soft edge push-in, zoom by height in
        world units with pixel-snap toggle for pixel art, rotation for top-down
        tilt effects.
  - [ ] Shake and kicks: trauma 0-1 with Perlin offset/rotation noise and decay,
        impulse `shake camera by _`, hitstop freeze frames that pause world strands
        but not UI strands (same rule as `pause game`).
  - [ ] Parallax and split: camera drives parallax layers above, pixel-perfect
        toggle that snaps to whole pixels at integer zoom, split-screen for two
        players as two viewports over one world (later; single camera first).
  - [ ] Blocks and scripts: `set camera target to`, `set camera bounds/zoom/shake
        to`, reporters `camera x/y/zoom`, event `when camera reaches bounds`.
- [ ] 2D physics and movement (builds on rapier2d, joints and one-way platforms
      from Phase 4; this is feel plus helpers):
  - [ ] Platformer controller tuning: run accel/decel, air control factor, jump
        velocity plus variable jump height, coyote time, jump buffering, slope
        slide limit, step-up height for stairs.
  - [ ] Helpers: moving platforms that carry riders (parent-space delta like the
        actor hierarchy, not parenting), ladders/climb volumes, conveyor belts
        by surface tangent speed, top-down friction/acceleration preset.
  - [ ] Water and hazards: buoyancy volumes with drag and splash hook, spike/hurt
        volumes with knockback and invulnerability frames.
  - [ ] Blocks and scripts: `set move speed/jump height/coyote time to`, `is _
        grounded/on wall/in water?`, `launch _ by x y`, event `when _ lands`.
        Sampled on fixed tick so replays stay deterministic.
- [ ] 2D effects and juice (the 2D path through the Phase 4 particle/trail blocks
      plus screen feedback):
  - [ ] Particles in 2D: sprite-sheet flipbook particles, spawn burst/rate shapes
        (point/line/box/circle), velocity plus drag plus gravity scale, color and
        size over life curves, soft-edge fade near tile collision.
  - [ ] Trails and feedback: ribbon trails behind fast actors, ghost afterimages
        with lifetime (reuse the existing ghost path), floating damage text,
        squash-and-stretch scale pops on land/hit.
  - [ ] Screen transitions: fade/wipe/circle wipes between rooms, flash frames,
        slow-mo timeScale curve plus hitstop (same clock rule as cinematics: wall
        clock when paused, fixed tick when running).
  - [ ] Weather lite in 2D: rain/snow/leaf particle presets tied to the weather
        director coverage value, splash rings on ground hit, wind push from the
        global wind asset.
  - [ ] Blocks and scripts: `burst particles _ at`, `trail _ on/off`, `flash
        screen _`, `pop _`, reporters `particle count`, `is screen shaking?`.
- [ ] Editor, preview and scaling for 2D:
  - [ ] 2D inspector tabs: Sprite/Anim/Tiles/Light/Camera, live aspect preview
        thumbnail, flipbook strip viewer (scrub frames, see hitboxes), parallax
        layer stack view.
  - [ ] Viewport: pixel grid plus onion-skin ghosts for animation, tile collision
        overlay, light radius gizmos, camera bounds plus deadzone rect, parallax
        depth ruler.
  - [ ] Quality: per-platform sprite atlas budget, particle and decal pool caps
        with LRU steal shared with Phase 5, resolution scale for 2D post, auto-drop
        rule (if frame over N ms for M frames, drop particle density one step).
        Build dialog lists which target keeps 2D lights/shadows and why.

### Phase 7 - Scale and ecosystem, do last
- [ ] Multiplayer: headless server, replication, lobbies, rollback.
- [ ] Deploy: Web/WASM (see Phase 8 player and Phase 9 editor), Android/iOS signing, console path, auto-updater/DLC/addressables.
- [ ] Ecosystem: analytics/crash, achievements/IAP hooks, plugin API, asset store, collab/VCS, docs/LTS.

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

- [ ] Goal: edit block projects in a browser. Qt cannot go to web, so the
      path is the existing browser frontend (`ui/` Vue plus the `dev-bridge`
      backend), with the Phase 8 web player as its preview canvas. No new
      editor stack: reuse `Backend::dispatch` commands and the shell/MCP
      command surface.
- [ ] Scope: block canvas and inspectors in Vue, web-player preview with
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
