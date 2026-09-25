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
- [x] Game view: the world runs in the editor and draws into a docked Game view, with input forwarding, pause/step and pointer lock. Started as a sidecar MJPEG stream beside the runtime's own OS window; on Linux it is now in-process with GPU frame sharing and no extra window (see "Qt6 rewrite" below for what's left there).
  - [x] Headless mode for the sidecar: hide the OS window while it keeps rendering the stream. Moot in-process, where there is no window.
  - [ ] MJPEG fallback (Windows, macOS, `BLOCKLOOM_RUNTIME=process`): the resolution switch still resizes the OS window, pointer lock only gets absolute positions, and the stream is a fixed ~15fps JPEG-60 regardless of preset or pause state. Most of this goes away once those platforms share GPU frames.
- [x] Visual world editor: edit-mode 2D/3D viewport with selection sync to ActorList/Inspector, drag to move plus rotate/scale gizmos, snapping, camera pan/zoom/orbit. Shares the Game view panel: Edit manipulates placement directly, Play runs the world.
- [ ] Editor: gizmos/snapping, prefab mode, scene search, log filter, frame stepper, profiler (draw calls, CPU/GPU/memory), playmode tests.

### Phase 4 - Look and depth, uses Bevy leverage
- [x] Asset pipeline: glTF/FBX rigs, atlases, texture/audio compression, reimport tracking.
- [x] Materials/custom WGSL plus shader graph lite, particles/trails, post-process, shadows/HDR, 2D sorting layers, tilemap/terrain.
- [x] Load glTF scenes for Model looks (a ModelSource loader with rig playback from the parsed animations) instead of placeholder boxes.
- [x] Bake atlas layouts into sheets at build time - pack_atlas is plan-only today - and let a tilemap animate tiles and collide per-tile rather than as one slab.
- [x] Close the custom-shader loop: export a shader graph to a .wgsl asset, and let hand-authored WGSL drive the live material instead of only the uniform path.
- [x] Particle/trail blocks (burst, emitter dials), plus ghost trails for custom-shaded and tilemap actors.
- [x] Advanced physics: fixed, hinge and rope joints, character controller, one-way platforms, and ragdoll chains built from hinged bodies.
- [x] AI: live polyanya rebake, navigation cost areas and layer masks, off-mesh links, crowd separation, steering, behavior trees and sight perception.
- [ ] Performance foundation (do before Phase 5 needs it): engine-wide footing for
      the Phase 5 environment stack; Phase 5 adds the rendering budgets on top.
  - [ ] Batching and instancing: static batching for level geometry, GPU instancing
        for repeated props/vegetation/debris (one draw per mesh, per-instance data
        in storage buffers), dynamic batching for small meshes. Covers the actors
        Phase 5 will scatter by the thousand.
  - [ ] LOD and occlusion: screen-size LOD selection for meshes, software Hi-Z or
        query-based occlusion culling for interiors and caves, GPU frustum culling
        where Bevy does not already do it. The LOD hooks Phase 5 terrain chunks,
        trees and VFX plug into rather than reinvent.
  - [ ] Async loading and streaming: background asset loads with placeholder or
        fade-in, world streaming cells with hysteresis so borders never thrash,
        shader prewarm on Play and at build time so first frames never hitch. The
        cell system Phase 5 terrain chunks stream through.
  - [ ] GPU measurement: per-pass timestamp queries plus render-target memory
        accounting, surfaced in the profiler (completes the render half of the open
        Phase 3 profiler item). No Phase 5 budget is enforceable without it.
- [ ] Refactors to clear the path for Phase 5 (do these first, not mid-stack):
  - [ ] One blended environment resource: volume blending writes a single
        `Environment` render resource (sky, fog, light, exposure deltas) that the
        dim2/dim3 passes read, instead of each pass reading the project. Sky,
        clouds, fog, water and post then consume the same blended values.
  - [ ] Shared shader library and pass plumbing: common WGSL chunks (hash, noise,
        FBM, scattering helpers, standard UBO layout) plus one FP16 working-target
        set with a half-res scratch pair and bilateral upsample, used by both
        dimensions. Stops volumetrics, fog and SSR from each rolling their own.
  - [ ] Asset and snapshot extension points: the importer grows 3D/LUT volumes,
        HDR/EXR with BC6H, heightmaps, IES/cookies (extend the Phase 4 pipeline,
        not a parallel one); probe capture becomes a service reused for HDRI
        baking, reflection probes and water reflections; the sense snapshot gains
        a versioned slot for sun/wind/fog/weather so VM, codegen and scripts stay
        in sync on fixed ticks.

### Phase 5 - Environment and AAA look (HDRP/Unreal parity)
- [ ] AAA environment stack (HDRP-grade, Bevy leverage where it exists, ordered
      bottom-up: frame, volumes, light, sky, air, wind, clouds, world, effects,
      post, perf, direction, camera, audio, tooling): a World Environment asset on the
      project plus Environment volumes in the scene, with HDR, lighting, sky, fog,
      clouds, water, terrain, VFX and post blending by volume weight.
  - [ ] Full HDR pipeline (linear FP16 end to end, HDRP-style units and output):
        - Rendering: FP16 HDR render targets from sky through lights to post, linear
          working space, tonemap and OETF only at final output. All lights, sky sun,
          clouds and emissives can exceed 1.0 without clipping. Bloom threshold works
          in HDR (1.0-plus for real glints, below for stylized glow).
        - Physical units: sun in lux with real sun/sky ratios, punctual lights in
          lumens/candela with range falloff in meters, emissive as color times
          intensity multiplier (HDR color picker with exposure-invariant swatch).
          Exposure in EV shared with time-of-day director and post volumes.
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
  - [ ] Volume framework (the backbone everything below plugs into): global default
        plus box/sphere volumes with priority, blend distance and weight. Every
        property has an override checkbox HDRP-style, so a cave volume can take fog
        and exposure without touching sky. Debug views: volume heatmap, active blend
        list, frozen-frame lerp inspector. Block API: `enable volume _`, `set weight
        of volume _ to`, reporter `active volumes`.
  - [ ] Lighting rig (makes interiors and nights look right): irradiance/light-probe
        volumes (brick grid like HDRP APV, bake button plus auto-dirty on move),
        reflection probes (box-projected cubemaps, capture on demand, blend by volume),
        rect/disk area lights with LTC speculars, light cookies (projected texture
        with tiling), IES profiles for spotlights (import file, intensity in candela),
        contact shadows (screen-space 16-tap raymarch under feet and clutter),
        shadow tuning (PCF/PCSS toggle, cascade splits, normal/slope bias, fade).
        API: `capture probes`, `set shadow distance to`, per-light `casts shadows?`.
  - [ ] Ray-traced lighting (bevy_solari, experimental upstream, RTX-class GPUs):
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
  - [ ] Sky types (all feed background, ambient probe and reflections together):
        - Procedural physical sky: sun disk (size, limb darkening, intensity) plus
          moon disk (size, phase 0-1, halo power), Rayleigh RGB scattering, Mie
          anisotropy g plus directional intensity, ozone absorption, Rayleigh and Mie
          altitude scales, ground albedo tint, horizon-to-zenith blend curve, planet
          radius and atmosphere thickness for limb curvature, night tint ramp, and
          shared exposure offset. Sun position driven by lat/long plus time, or
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
  - [ ] Atmosphere, fog and space:
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
  - [ ] Volumetric light volumes: per spot/point cone inscatter (density, anisotropy,
        falloff curve, near/far fade) for visible beams, dust motes (billboard points
        drifting in beam, size/alpha/twinkle), fake shaft cones (additive fresnel-faded
        geometry with noise scroll) for cheap beams, height-dust global toggle. Tied
        to fog density so beams thicken in fog and vanish on clear days.
  - [ ] Movement and animation (one wind system drives clouds, layers, vegetation,
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
  - [ ] Volumetric clouds (the hero feature, raymarched in sky pass at half res with
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
  - [ ] Cloud layers (planar 2D cover above and below volumetrics, also the full
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
  - [ ] Terrain and vegetation: heightmap terrain (1k to 4k, import PNG/RAW, sculpt
        raise/lower/smooth/flatten/noise/terrace with radius/falloff/strength, paint
        albedo/normal/roughness layers with slope/height/curvature rules, holes for
        caves), auto collision plus LOD (quadtree or chunked, crack fix, pixel-error
        metric), grass (instanced blades or cross quads, density map, color variation,
        wind bend plus gust flutter, distance fade, cull distance), trees/rocks
        (instanced LOD0/LOD1/billboard, scatter brush with density/clumping noise and
        slope/altitude/collision filters, per-instance tint/scale jitter). Editor:
        sculpt/paint/scatter brushes, erosion filter preview, stats (tris, instances).
  - [ ] Water (ocean, lake, river actor): wave model (8-12 summed Gerstner waves
        with amplitude/chop/steepness plus Phillips-spectrum normal detail, wind
        fetch param), depth color (shallow tint, deep tint, Beer absorption distance),
        transparency plus refraction offset, foam (shoreline depth fade, crest foam
        threshold with noise breakup, flow scroll), reflections (planar probe or SSR
        fallback plus GGX sun glint with roughness), underwater fog volume plus
        caustics texture projection, buoyancy component (sample height/normal/velocity
        for physics blocks and scripts), interaction hooks (splash particle, ripple
        decal, sound). Blocks: `set water level/chop/foam to`, `water height at x y`,
        `is _ underwater?`.
  - [ ] VFX graph (Niagara/VFX-Graph lite): GPU sim with spawn modules (rate, burst,
        shape sphere/box/cone/mesh-surface), update modules (velocity, drag, curl noise,
        turbulence, attractor, depth-buffer collide with bounce/friction, kill planes),
        render (flipbook sub-UV, size/color/rotation over life curves, soft particles,
        lit vs unlit), ribbons/trails (length history, tessellation, width curve, face
        camera, HDR color for glow trails), event hooks (`on collide/die/spawn` fires
        blocks). CPU fallback pool for headless/low-end. Editor: curve editor, live
        loop preview, max-particle budget and overdraw meter.
  - [ ] Decals: deferred projected (albedo/normal/roughness/emissive, atlas pages,
        angle fade, depth reject to avoid floating edges), pool with LRU steal plus
        per-decal lifetime/fade, blood/scorch/footprint presets. Blocks: `spawn decal
        _ at`, `fade decals in radius`. Persist toggle for scorch that survives reload.
  - [ ] Destruction and fluids lite: fracture-on-hit (Voronoi cell count, interior
        cap material, impulse threshold, shard lifetime/sleep/pool cap), debris
        impulse inheritance plus bounce sounds, 2D shallow-water ripple grid for
        puddles/ponds (rain rings, footstep rings, shore reflect), smoke advection
        grid for stylized chimneys and dust puffs (no full 3D sim), persistent
        scorch/wetness map (world-space RT, dries over time, darkens albedo and
        raises specular). Blocks: `fracture _`, `splash at`, `puff smoke at`.
  - [ ] Post volumes (full HDR chain, volume-blended): exposure (manual EV plus auto
        spot-meter with min/max and speed), bloom (threshold/knee, 5-mip scatter chain,
        dirt texture), tonemap (ACES/Neutral/AgX select, toe/shoulder), white balance
        plus LUT/grading (lift/gamma/gain, saturation, contrast), vignette, depth of
        field (autofocus target or fixed distance, bokeh blades/circular, near/far),
        motion blur (shutter angle, per-object toggle), SSAO (HBAO, radius/intensity),
        SSR toggle with roughness cutoff, chromatic aberration, film grain, sharpen.
        Order fixed HDR-first; debug splits (bloom mip, CoC, AO only).
  - [ ] Performance and scalability (whole-frame budgets for the stack above):
        - Draw efficiency: GPU instancing for vegetation/props/debris, static plus
          dynamic batching for decals and small meshes, indirect draws with GPU
          frustum and occlusion culling (software Hi-Z plus hardware queries),
          per-system draw-call and triangle budgets surfaced in the profiler.
        - LOD and culling: distance plus screen-size LOD for terrain chunks, trees,
          water tiles and VFX, occlusion culling for interiors and caves, cloud
          step-count LOD by distance and weather weight, probe and shadow update
          throttling (staggered refresh, frozen static probes, cascade shrinking).
        - Async and streaming: background load of terrain chunks, noise volumes,
          HDRI mips and probe captures, world streaming cells with hysteresis so
          borders never thrash, shader prewarm on Play and at build time so first
          frames never hitch.
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
          exposure EV, temperature/tint, fog density, cloud coverage/type, precipitation,
          wetness, wind, aurora KP, grading LUT weight. Bezier keys, loop toggle,
          keyframe presets (dawn/noon/dusk/midnight).
        - Weather presets as assets: Clear, Overcast, Storm, Sunset, Night, plus user
          presets. Each stores full sky/cloud/fog/light/post deltas. `blend weather
          to _ over _ seconds` lerps with ease curve; stack holds current plus target
          so rapid changes do not pop.
        - Block and script API: `set time of day to`, `advance time by`, `set cloud
          coverage/density/type to`, `set fog density to`, `set precipitation to`,
          `set exposure to`, reporters `time of day`, `sun elevation`, `cloud
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

### Phase 6 - Scale and ecosystem, do last
- [ ] Multiplayer: headless server, replication, lobbies, rollback.
- [ ] Deploy: Web/WASM, Android/iOS signing, console path, auto-updater/DLC/addressables.
- [ ] Ecosystem: analytics/crash, achievements/IAP hooks, plugin API, asset store, collab/VCS, docs/LTS.

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
