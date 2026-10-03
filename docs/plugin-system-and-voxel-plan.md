# Plugin platform and procedural voxel worlds

Status: proposed architecture and delivery plan. No plugin platform or voxel implementation is claimed by this document.

Date: 2026-10-01.

## Goal and definition of done

Give Blockloom a production plugin ecosystem comparable in scope to Unity's: installable packages, dependency resolution, editor extensions, custom components and blocks, runtime systems, importers, rendering extensions, target-specific builds, debugging, compatibility policy and an SDK that third parties can use without editing the engine.

Ship `com.blockworked.voxel` as the first substantial plugin. It must support deterministic procedural worlds, caves and overhangs, smooth and block-shaped geometry, GPU instancing, compute acceleration, emissive materials, structural fracture, persistent runtime edits and visual authoring. A package manager with one hardcoded voxel component does not meet this goal.

An author must be able to install the package, create a world from a preset, change generation visually, sculpt and paint it, drive mining/building/explosions through blocks, and ship a standalone game. A plugin developer must be able to reproduce the same integration using only the published SDK.

### Reference scope

Unity's [project manifest](https://docs.unity.com/en-us/engine/6000.0/manual/packages-list/managing-packages-manifest) and [native plugin support](https://docs.unity.com/en-us/engine/6000.5/manual/scripting/compilation-and-code-reload/plug-ins/native) establish useful package and native-extension precedents. Blockloom needs its own contracts for Qt, Bevy, blocks and players.

Voxel Plugin Pro is an authoring and performance reference, not a dependency or an exact feature checklist. Its current [overview](https://docs.voxelplugin.com/getting-started/working-with-voxel-plugin/) describes deterministic graph/stamp terrain generation and integration with Unreal's PCG. Its [runtime-edit documentation](https://docs.voxelplugin.com/knowledgebase/blueprints/runtime-edits-and-sculpting) distinguishes stamps from sculpt data and currently lists limitations around detached-piece physics and replication. Blockloom's shaped blocks, fracture and persistent edit pipeline are explicit requirements even where that reference differs. Do not assume access to Unreal's Nanite or copy proprietary implementation.

## Starting point in this repository

| Existing system | Reuse | Required change |
| --- | --- | --- |
| `blockloom-core` project, component and block models | Serialization, validation, authored state | Namespaced extension records and schemas alongside built-in enums |
| `blockloom-app` commands, history, revisions, owner lock | One authoritative edit path | Dynamic command registration and plugin transactions |
| `blockloom-qt` QML modules and scene tools | Editor chrome, game viewport, selection and gestures | Runtime contribution registry and plugin workspace/tool hosting |
| `blockloom-runtime::add_world` | Shared embedded/process/player world setup | Plugin lifecycle and stable scheduled extension hooks |
| `blockloom-core/src/terrain/` and runtime terrain | Tiled storage patterns, asynchronous jobs, brushes, vegetation | A separate volumetric store and mesher; heightfields cannot represent caves |
| Runtime batching, culling, streaming and quality | Instance grouping, residency, task scheduling, budgets | Plugin service interfaces, vertical chunk addressing and custom draw accounting |
| Shader library, materials and pass plumbing | WESL validation, common shader modules, pipeline warm-up | Versioned shader/pass contracts and compute resource management |
| Destruction and `Fracture` | Debris lifetime, effects, physical shard budgeting | Volumetric connectivity, material damage and detached voxel bodies |
| VM, codegen and script ABI | Equivalent gameplay execution paths | Generic plugin calls, reporters, events and asynchronous completion |
| Pack/build, shell and MCP | Shipping and automation | Plugin dependency closure, artifacts and generated command schemas |

Current Bevy registration is engine-internal Rust integration, not a distributable plugin API. Rust scripts also have a separate, narrow ABI and a no-Cargo build path. Neither can be treated as a general plugin loader.

Keep existing Terrain and Fracture projects working. Expose adapters rather than migrating every built-in subsystem in the first release. Extend the shared `blockstitch` editor where dynamic block definitions require it, keeping both pinned halves compatible. Work in the Qt frontend.

## Part I: complete plugin platform

### 1. Package identity, installation and reproducibility

Use reverse-domain IDs, semantic versions and immutable content hashes. A package contains a manifest, contribution schemas, runtime artifacts, editor artifacts, shaders, assets, documentation, examples and license notices. Split editor-only dependencies from runtime dependencies.

Proposed package layout:

```text
com.blockworked.voxel/
  plugin.json
  schemas/
  editor/
  runtime/<target-triple>/
  portable/
  shaders/
  assets/
  examples/
  docs/
  licenses/
```

`plugin.json` declares identity/version, engine and SDK ranges, plugin ABI version, dependencies, optional features, capabilities, supported targets, artifact hashes, entry points, schema versions and migration entry points. A target entry describes its required GPU features/limits and fallback, not just its operating system. Verify the actual payload against the declaration.

Project-local `plugins.json` records direct dependencies; `plugins.lock` records the resolved graph, hashes, source and per-target artifact identities. Proposed filenames are new contracts, not existing files. Packages live in a shared immutable cache, with explicit local-development overrides recorded separately. Opening a project never silently upgrades its lockfile.

Support registry, local folder, archive and pinned Git sources. Resolve the complete graph before activation; reject cycles, incompatible versions, duplicate IDs and conflicting providers with an actionable dependency explanation. Initially select one version of each plugin per project. Optional features cannot silently change serialized behavior.

Installation is transactional: fetch into staging, verify hashes/signatures where available, validate manifests and target support, then publish the cache entry and lockfile atomically. A failed install/update leaves the previous project usable. Offer offline cache installs, rollback, pinning, removal, dependency inspection and a preview of migration effects. Uninstall retains project data unless the author explicitly deletes it.

Provide a Plugin Manager with search, Installed/Available/Updates, compatibility, dependency trees, target availability, licenses, capability declarations, diagnostics and restart requirements. Build a registry client and publishing protocol; marketplace payments and commercial licensing services can follow independently of the technical platform.

### 2. Execution models and ABI boundary

Use three documented tiers. A manifest selects the appropriate tier; the host never loads a Rust dynamic library and assumes its Bevy or Rust layouts are stable.

| Tier | Suitable work | Contract and shipping behavior |
| --- | --- | --- |
| Declarative package | Components, inspectors, blocks, graph nodes, assets, supported shader passes | Schemas and host-owned services; works wherever its required capabilities exist |
| Portable code module | Generators, importers, tools, bounded gameplay logic | Versioned WebAssembly host calls, isolated memory and execution/memory limits; native and browser hosts |
| Native module | High-throughput geometry, SDK integrations, specialized runtime services | Versioned C ABI, batched buffers and opaque handles; prebuilt library per supported native target |

Add a separate source-linked engine adapter tier for extensions that genuinely require private Bevy render/ECS APIs. It is pinned to the exact engine build and compiled into custom players. Label it as requiring that build toolchain; do not market it as portable binary compatibility. The voxel plugin should use public services wherever possible and keep any initial engine adapter small and visible.

The C ABI exposes function tables with size/version fields, fixed-width primitives, tagged values, explicit buffer ownership and host-owned opaque handles. No Rust references, trait objects, `Vec`, Qt objects or Bevy resources cross it. Specify allocator/free pairs, byte encoding, thread affinity, callback lifetime, handle generation and error codes. Batch chunk buffers and instances rather than making one foreign call per voxel.

Portable modules run in a native WASM executor and a browser WebAssembly host implementing the same API. Begin with a simple versioned linear-memory ABI and bulk copies; do not depend on browser support for an emerging component model. Prove interpreter/JIT costs, cancellation and browser scheduling in phase 0. CPU reference generation remains available in the portable artifact even when native SIMD and GPU paths accelerate the same algorithm.

The first voxel release must supply native artifacts for Windows/Linux/macOS and a portable browser implementation, with documented browser quality limits. There is no native DLL loading on web. Android follows a separate artifact/device qualification gate; unsupported targets fail build preflight. Players consume prebuilt package artifacts by default and do not need Rust installed.

### 3. Public extension services

Define narrow, versioned services with contract tests. The initial SDK must cover the following extension surfaces, even if advanced implementations arrive in later phases.

| Surface | Public contribution |
| --- | --- |
| Data | Namespaced component/resource schemas, defaults, validation, migrations, references |
| Editor | Inspector sections, asset pickers, panels, menus, commands, tools, gizmos, overlays, shortcuts |
| Blocks | Categories, statement/reporter/hat definitions, typed slots, effects, events and help |
| Gameplay | Fixed/update hooks, snapshots, actor/component queries, batched effects, deterministic RNG |
| Assets | Importers, processors, thumbnails, dependency discovery, export/cook hooks |
| Generation | Typed graph nodes, worker jobs, tiled data, bounds and cache dependencies |
| Rendering | Material definitions, mesh/instance submission, compute jobs, render passes, shader imports |
| Physics/navigation | Collider cooking and replacement, shape queries, dirty navigation regions |
| Storage | Project blobs, player saves, content hashes, transactions and reference enumeration |
| Build | Runtime dependency closure, target artifacts, validation, asset cooking and licenses |
| Diagnostics | Logs, timings, memory, GPU markers, counters, job traces and debug overlays |

Runtime hooks use named stages with documented snapshot visibility: input, pre-simulation, fixed simulation, effect application, physics/post-physics, render extraction and presentation. Resolve declared ordering constraints within allowed stages and reject cycles. Plugins submit effects rather than modifying authored data or bypassing runtime component ownership.

A render service owns the GPU device, queue, resource lifetimes and submission. Plugins register validated WGSL/WESL kernels and declare bindings, resource accesses, dispatch bounds and pass dependencies. The host controls barriers, allocation, warm-up and budget admission. Basic mesh, instanced mesh, compute-to-buffer and supported material paths form the stable API; arbitrary custom pipeline internals require a source-linked adapter until a supported descriptor exists.

Expose compute work through the engine's render schedule, not a second unmanaged GPU device. Measure graphics contention before promising overlap: compute dispatch does not imply a separate asynchronous hardware queue. Handle device loss and re-create resources from authoritative data.

QML has two levels: schema-generated inspectors/panels for ordinary packages, and explicit trusted editor modules for advanced interfaces. Add dynamic QML loading and a stable bridge for the latter; the existing compile-time `QmlModule` list cannot load arbitrary installed packages. Trusted QML/native modules have host-level access and cannot be sandboxed by a manifest declaration. Sandboxed packages use host widgets and capability-mediated services. Runtime builds omit all editor modules.

### 4. Persistence, missing plugins and migration

Add generic plugin-owned records to components, project/scene resources, assets and block graphs. A record carries `plugin_id`, stable `type_id`, `schema_version` and a validated payload with explicit asset/actor references. Names and palette labels are display metadata, not persistent identities.

Preserve unknown records losslessly. A missing plugin produces an unresolved component, asset or block with its original payload and a repair/install action. Opening and saving cannot erase that data. Missing runtime behavior stops Play/build with a precise dependency report; it never becomes a silent no-op. Explicit editor-only contributions do not block a player build.

Migrations operate on copied project data and plugin blobs in a transaction. Validate the entire result before committing the new lockfile and schema versions. Keep a rollback snapshot. Reject downgrades without a supported reverse migration. Garbage collection uses references from current documents, undo history, migration snapshots and in-flight edits; it must not prune a voxel revision needed by undo.

### 5. Blocks, compiled gameplay, scripts, shell and MCP

Add generic plugin instruction/reporter/event representations alongside the current closed enums. Define a registry ID and typed argument layout once. Dynamic statements need asynchronous job handles and explicit wait/cancel behavior; reporters read published snapshots and never block for GPU readback or generation.

Initially support ordinary statements, reporters and hats through the generic bridge. Third-party custom control-flow blocks require an explicit VM/codegen lowering contract and verification before activation. Do not allow an arbitrary callback to create invisible loops or suspension semantics.

VM and compiled code call the same registry with equivalent argument evaluation, error behavior, effect ordering, job IDs and event delivery. Generated Rust uses a stable generic host ABI rather than linking the plugin crate. Extend the script ABI with batched plugin operations and job/event access; bump `ABI_VERSION` when it changes. Maintain VM/codegen parity fixtures for plugin statements, reporters, suspension and errors.

Backend commands are namespaced and derived from structured schemas. QML, shell and MCP discover the same registry at runtime. Extend `--specs`, dispatch, shell parsing and `mcp/src/registry.ts` so installing a plugin makes its commands available without hand-editing the MCP server. Validate permissions, argument bounds and project revisions centrally.

### 6. Lifecycle, reload, reliability and compatibility

Lifecycle: discover, resolve, validate, register, open project, create world, start run, stop run, close world, close project, unregister. World recreation and cross-dimension scene changes must not leave jobs, colliders or GPU buffers behind. Tag callbacks and jobs with project/world/plugin generations so late replies cannot revive old state.

Hot reload begins with assets, shaders and declarative UI. Portable code reload requires quiescing jobs and transferring versioned state. Native runtime libraries reload only by restarting the world host initially; in embedded Linux mode that may require restarting the editor. Do not unload a library while any callback, worker or render command can reference it. Report the actual restart requirement in the manager.

Declare capabilities such as project storage, network, external files, subprocesses and native execution. Mediate these for portable code. Run importers in workers with deadlines and project-scoped IO. Native plugins are trusted code; a C ABI prevents layout coupling, not native crashes. Offer process-runtime mode for native-plugin fault isolation, while preserving the high-performance embedded path.

Keep engine version, SDK version, plugin ABI version, editor API version, data schema versions, shader API version and target artifact identity distinct. Publish supported compatibility ranges and deprecation policy. Verify plugin pairs and competing hook registrations, not just plugins individually.

Provide an SDK scaffold, manifest validator, local-development mode, package builder, publishing CLI, mock host, native/WASM examples, integration harness and API documentation. CI builds artifacts and validates schemas, license notices, ABI conformance and supported targets. A tiny independent importer/component plugin is required to prove generality alongside voxels.

## Part II: first plugin, `com.blockworked.voxel`

### 7. User-facing capability contract

| Requirement | Planned implementation | Acceptance demonstration |
| --- | --- | --- |
| Full procedural generation | Seeded graph, biome rules, volumetric fields, stamps, structures and scatter | Stream a world with caves, overhangs, biomes, ores and placed structures |
| Smooth terrain | Signed density field, surface extraction and transition geometry | Enter a sculpted cave through multiple LOD rings without seams |
| More than cubes | Shape palette and rotations, SDF primitives, splines, mesh stamps | Cubes, wedges, slabs, stairs and smooth arch/tunnel in one project |
| GPU instancing | Shared shape/prop geometry with compact per-instance state | Large repeated-shape and vegetation scene with grouped draws |
| Compute shaders | Generation, classification, scans, meshing, culling where profitable | Profile the same seed/edit workloads against CPU reference paths |
| Emissiveness | HDR emission per material, bloom and optional lighting integration | Paint glowing ore, mine it and preserve the exposed material |
| Fracturing | Damage/support graph, detached connected regions, bounded rigid bodies | Remove bridge supports and watch detached sections collide |
| Live manipulation | Async add/remove/paint/smooth/stamp/damage operations | Mine/build continuously across chunk borders and save/reload edits |

The complete release includes all rows. Smooth terrain alone or a cube demo is an intermediate milestone.

### 8. World data and coordinates

Expose `VoxelWorld`, `VoxelGenerator`, `VoxelMaterialPalette`, `VoxelShapePalette`, `VoxelStamp`, `VoxelInteractor` and `VoxelFractureSettings` through plugin schemas. World settings specify seed, base voxel size, generator identity/version, finite or streamed bounds, LOD policy, edit/persistence policy and resource budgets.

Keep authoritative data on the CPU in sparse, paged bricks. Begin with 32 cubed cells per brick and benchmark 16/32/64 before fixing the storage format. A smooth brick needs boundary samples and stencil halos in addition to its cells; explicitly account for these in memory estimates. Represent uniform empty/solid bricks compactly, use palettes for materials/shapes, and compress inactive edited pages. Meshes, colliders and GPU buffers are disposable caches.

Use two explicit representations behind one world service:

- Smooth regions store quantized signed density and material weights/metadata. They support caves, overhangs, rounded terrain and volumetric CSG.
- Shaped-cell regions store occupancy, shape ID, orientation, material and damage metadata. They support cubes, wedges, stairs, slabs and user-defined cell meshes with collision templates and face-coverage masks.

Do not force arbitrary cell meshes into a single scalar SDF. Define region composition precedence, contact boundaries, occlusion and edit semantics when smooth and shaped content overlap. Smooth terrain and shaped construction may render as separate compatible layers initially. Importing a mesh as an SDF stamp is distinct from registering that mesh as a repeatable cell shape. Non-watertight mesh imports require an explicit shell/fill policy.

Address pages by signed 64-bit integer world coordinates; render and simulate near a floating origin. Smooth movable voxel objects and detached fragments use local grids with actor transforms. Extend the existing streaming service to represent vertically stacked pages and invokers, while keeping it the sole owner of residency/task admission. Large-world origin shifts must update physics, cameras, water, particles and plugin queries consistently; finite worlds can ship before that engine-wide gate passes.

Logical world extent can exceed resident memory, but active bricks, edit journals, save storage and generation work always have finite budgets. Never describe streaming as literally unlimited resources.

### 9. Deterministic procedural generation

Build a visual typed graph with reusable subgraphs, exposed parameters, previews, error locations and per-node timings. Reuse blockstitch interaction controls where useful, but field generation is a typed dataflow graph, not a time-stepped gameplay strand.

Initial node families: coordinate transforms, integer-seeded noise, domain warp, arithmetic/masks, height and volume fields, CSG, slope/altitude/climate rules, biome blending, caves/tunnels, ore distribution, material assignment, shape placement, splines, imported heightmaps/meshes and structure templates. Scatter consumes surface/material/biome queries and connects to existing vegetation and actor placement services.

Compile graphs to a versioned intermediate representation with CPU and WGSL backends. Validate types, cycles, sample radii, output bounds, unsupported GPU operations and execution cost. Hoist uniform values, fold constants, cache reusable subgraphs and skip provably empty/solid pages. Graphs and stamps declare influence bounds so an edit invalidates only dependent bricks.

Generation phases are explicit: macro climate/biome fields, density/caves, materials/resources, cross-page structures, then surface scatter. Rivers/erosion and other nonlocal simulations use deterministic coarse region tiles with fixed halos and bounded iteration counts or baked assets, rather than pretending they are independent point samples. Water placement integrates with existing Water; full volumetric fluid simulation is a separate extension.

Randomness derives from world seed, stable node ID, integer coordinates and generator version, independent of worker scheduling or load order. Structures spanning pages have a canonical owner and deterministic overlap priority. Scatter samples coarse cells deterministically and invalidates local support when terrain changes.

Persist generator version/hash and exposed parameters with every save. Generator upgrades create a new base revision and require an explicit rebase of edits. Provide CPU-authoritative quantized outputs for gameplay. GPU floating-point evaluation is not assumed bit-identical across devices; compare with tolerance for visual generation and use canonical data for collision, persistence and future replication.

Include presets for an island, biome terrain with caves/ores, a shaped-block building world and a destructible voxel prop. A user can customize each through graph controls without writing Rust.

### 10. Meshing, LOD and rendering

Use distinct rendering strategies rather than creating one entity/draw per cell:

1. Solid cubic regions use greedy surface meshing with material/AO-aware merge rules. Emit visible faces only.
2. Smooth regions begin with Marching Cubes plus a consistent ambiguity policy and transition cells. [Transvoxel](https://transvoxel.org/Transvoxel.pdf) is the reference for bridging different sampling resolutions. Review algorithm/table licenses before integrating code or tables. Balanced neighboring LODs, shared samples and coherent boundary updates are required.
3. Repeated non-cubic cell shapes, props and compatible detached objects use instanced mesh groups keyed by geometry and material pipeline. Per-instance records carry transform, palette selection, tint/emission and damage flags. Cull hidden faces using shape coverage where valid; partially occluded shapes must retain required geometry.

Instancing does not reduce unique smooth terrain to one repeated mesh. Smooth chunks use pooled vertex/index arenas and grouped material bindings; shape instances use instance buffers. Avoid routing transient voxel meshes into static actor batching that assumes authored mesh ownership.

Compute acceleration proceeds in measured steps: field evaluation, active-cell classification, prefix scans, output allocation, surface extraction and visibility compaction. Clamp dispatch sizes, guard buffer bounds, detect output overflow and resubmit/split work within a budget. Keep mesh output GPU-resident for rendering; do not read back every generated mesh to upload it again.

CPU jobs build collision from canonical samples separately. GPU readback is reserved for diagnostics or explicitly requested data and is asynchronous. Keep a tested CPU mesher/generator as a correctness oracle and a reduced-quality fallback. Precompile the initial graph/material/shape variants during warm-up; newly introduced runtime variants use placeholders or queued compilation rather than blocking a frame.

LOD uses screen-space error, hysteresis, distance and invokers. Keep collision detail near players even when visual detail drops. Publish neighboring seams as a coherent set while preserving previous valid geometry. Coarse shaped-cell LOD needs an explicit silhouette/occupancy proxy; thin structures cannot disappear under a naive majority vote. Pin gameplay-critical construction at sufficient detail.

Register bounds and actual chunk/instance/triangle/allocated-byte counts with culling and quality services. Custom draws must participate in shadows, depth/prepass, picking, fog, HDR, probes and motion history where the host supports them. Unsupported ray-tracing paths use an explicit fallback until geometry registration exists. Reset velocity/history when remeshing or shifting origin to avoid TAA artifacts.

### 11. Materials and emissive lighting

Materials carry base color, roughness, metallic, normal/detail textures, opacity mode, emission color/intensity and gameplay metadata such as hardness, density and fracture strength. Use bounded palette tables and texture arrays with triplanar mapping for smooth surfaces; shaped meshes can use authored UVs. Blend a bounded number of surface materials per vertex, preserving stable IDs during edits.

Opaque terrain ships first. Cutout and transparent materials require separate draw groups, sorting policy and limits; they cannot silently enter the opaque pass. Integrate water through the existing Water service before considering voxel fluids.

Emission is scene-linear HDR and works with the existing bloom/exposure pipeline. Emission alone does not automatically illuminate neighbors. Offer a bounded cluster-light approximation on baseline devices and integrate emissive geometry with existing GI/ray-traced lighting when supported. Profile light counts and emissive updates after mining. No light or shadow map per glowing voxel.

### 12. Live edits, queries and persistence

One edit API serves brushes, blocks, scripts, shell/MCP and explosions. Operations include add/subtract sphere/box/capsule, smooth/flatten, paint material, place/remove shape, stamp mesh/spline, damage region and copy/paste volume. Accept world ID, coordinate frame, operation bounds, parameters, expected revision and persistence policy.

Pipeline:

```text
validate and order operation
  -> update canonical pages and edit revision
  -> invalidate touched pages plus dependency/meshing halos
  -> schedule render, collision, navigation and scatter jobs
  -> publish coherent revisioned results
  -> emit completion and save eligible changes
```

Coalesce overlapping dirty pages; prioritize nearby interactions; cancel superseded jobs. Results carry world generation, page revision, generator hash and LOD identity. A late GPU/CPU result cannot overwrite a newer edit or resurrect an unloaded page.

Queries define what they observe: density/material reads use the latest committed canonical revision, while raycasts use a published collision revision unless a canonical voxel query is explicitly requested. Job events report committed, visual-ready and collision-ready states separately. Gameplay can wait for collision readiness before teleporting or placing a body into edited terrain.

Use a safe temporary collision policy during large edits: retain old colliders with a pending-region barrier or conservative replacement until the new set is ready. Set a maximum lag and expose it to gameplay. Swap related collider pages between physics steps, not halfway through a contact update.

Editor gestures preview a private overlay, commit one transaction on release and cancel on Escape. Play mutations are run-local by default and Stop restores authored data, matching existing engine behavior. An explicit Apply Run Changes action commits selected edits to the project. Player persistence writes to save data, separate from authored package blobs.

Save base generator identity plus ordered edits and periodic compressed page checkpoints. Bound journal replay by compaction; never regenerate an entire world on save. Use content-addressed pages, atomic manifests, checksums and crash recovery. A checkpoint records the operation sequence it covers. Editing an unloaded region loads only required pages or queues bounded work; large operations expose progress and cancellation.

### 13. Fracture and physical voxel objects

Treat fracturing as persistent topological change plus bounded physical simulation. Do not replace it with disappearing cubes or cosmetic particles.

Maintain material-dependent damage and structural support information. For shaped cells, use adjacency/coverage contacts; for smooth fields, derive a coarser support grid from canonical occupied samples. Anchor flags identify foundations and fixed structures. After damage, update local connectivity and cross-page support links, then find disconnected regions using an incremental connectivity graph.

A region touching an unloaded boundary has unknown support and remains attached until enough boundary information is available. Bound graph searches per frame and persist progress; do not run a full-world flood fill for each explosion. Document that this is a controllable game structural model, not engineering-grade stress simulation.

Detach qualifying islands into movable voxel objects with local data, mass/inertia from material density, approximate compound/convex colliders and momentum. Edit static source pages and publish the detached object in one logical transaction so material is neither duplicated nor lost. Emit fracture events with region ID, removed volume/material summary and spawned body IDs.

Reuse destruction/debris pooling, lifespan, sleep and quality throttles. Cap active rigid bodies, collider complexity, connected-region work and retained voxel data separately. Large pieces retain geometry and collide; tiny pieces can become particles. Exceeding physical budgets leaves regions pending/static or converts explicitly disposable debris according to authored policy. Persistent construction must not vanish merely because the debris cap was reached.

Support continued mining/painting on a moving voxel object, conservative remeshing/collider replacement, sleeping and optional bake-back into static terrain. Replay authoritative edit/fracture decisions deterministically; Rapier trajectories are not assumed cross-platform deterministic. Provide seams and progressive damage effects before adding more costly stress propagation.

### 14. Editor and gameplay integration

Add a Voxel workspace/toolset with world hierarchy, generator graph, material/shape palette, stamp browser, inspector and diagnostics. Use the real runtime viewport for preview, picking and brushes. Preview changes should update pages, not restart the entire world or rebuild the whole document per pointer move.

Tools: add/remove/smooth/flatten/paint/damage, shape placement with rotation, spline editing, mesh stamp placement, region selection, copy/paste, undo/redo and presets. Display brush bounds, pending collision, LOD rings, support regions and residency only through optional overlays. Graph previews expose density slices, surfaces, biome/material fields and per-node cost.

Initial blocks/reporters/events:

- Create/configure voxel world; set exposed generator parameter; preload region.
- Add/remove/paint/stamp/place shape/damage/fracture; save/load voxel edits.
- Sample density/material/shape; voxel raycast; region-ready and job-progress reporters.
- When edit completes/fails, region loads, material is mined or a fragment detaches.
- Wait for voxel job; cancel voxel job.

Every destructive operation returns a job ID, revision and completion payload. Provide counts/volume by material for mining rewards rather than estimating them from brush radius. Expose the same typed operations to scripts and namespaced shell/MCP commands. Immediate reporters read snapshots; costly scans are jobs.

## Part III: budgets, validation and delivery

### 15. Performance model and proposed acceptance budgets

These are initial targets to validate in phase 0, not claims about current performance. Record exact hardware, driver, backend, resolution, graph complexity, voxel size, residency, triangle counts, material counts and collision policy for every result. Select named physical reference machines before accepting a performance milestone.

Proposed desktop baseline: 8-core CPU, 16 GiB RAM, discrete GPU with 8 GiB VRAM, 1080p. Add a named integrated-GPU machine, a macOS Metal machine and a browser WebGPU profile with smaller residency. Use release builds and warmed pipelines; report cold start separately.

| Metric | Initial desktop gate |
| --- | --- |
| Steady traversal | p95 frame at or below 16.7 ms, p99 at or below 25 ms in the agreed reference scene |
| Plugin main-thread work | p95 at or below 2 ms/frame for scheduling and publication |
| Small brush | 2 m radius at 0.25 m cells, up to eight dirty pages: p95 edit-to-visible at or below 50 ms |
| Nearby collision | Same brush: p95 collision-ready at or below 100 ms, with safe interim behavior |
| Large edit | 20 m carve/explosion: first response within 100 ms, progressive completion and cancellation; no blocking frame-length job |
| Memory | Reference scene at or below 2 GiB voxel CPU memory and 1.5 GiB voxel VRAM, including caches and staging |
| Streaming | Agreed 10 m/s traversal stays ahead of the player; teleport admits a safe coarse view and nearby collision before movement |
| Stability | 30-minute traverse/edit/fracture soak reaches a bounded memory plateau |

A dense 32 cubed brick with an illustrative 8 bytes/cell costs 256 KiB before halos, indices and caches. 4,096 such bricks already cost 1 GiB. Measure real layouts; sparse compression does not remove worst-case budgeting.

Manage independent limits for CPU pages, GPU density/geometry, collision, staging buffers, queued jobs, edits, scatter and debris. Pressure first cancels obsolete jobs and lowers distant visual detail, then shrinks optional residency. Protect canonical edits and nearby safety-critical collision. Publish `plugins/<id>/*` and `voxel/*` counters for memory, generation, meshing, queue age, GPU dispatch, upload bytes, draws, collision lag, journal size and fracture work.

Browser gates use explicit reduced distance/resolution budgets, validated limits and CPU fallback behavior. Do not promise desktop scene parity before profiling. Procedural generation can page logically without network, but the current self-contained web build loads its packaged content upfront; external asset streaming needs a separate engine/build feature.

### 16. Verification matrix

Platform tests cover package resolution/rollback/offline installs, invalid artifacts, ABI mismatch, target rejection, duplicate contributions, lifecycle teardown, stale jobs, capability enforcement for portable code, schema migration, missing-plugin round trips and project-lock behavior.

Voxel tests cover negative coordinates and origin shifts, page borders/halos, LOD transitions and ambiguity cases, shape rotations/occlusion, graph determinism under changed worker/load order, cross-page structures, CPU/GPU numerical tolerances, bounds/overflow and degenerate geometry. Check raycasts and collision readiness against canonical data.

Edit tests cover overlapping strokes, cancellation, undo across compaction, save/load and crash recovery, generator rebase, edits to unloaded pages, job supersession and a run stopping during a pending fracture. Fracture fixtures cover anchored bridges, cross-page islands, unknown support, moving fragments and body-cap exhaustion without mass duplication or lost persistent material.

Render fixtures cover glowing caves, smooth/shaped boundaries, shadows/depth, transparency, origin shifts, device loss, GPU allocation exhaustion and fallback paths. Performance scenes separately stress unique smooth surfaces, repeated shapes, dense materials, worst-case noisy density, continuous edits and many fragments. Report CPU, GPU and storage bottlenecks independently.

Run the same plugin gameplay fixtures through VM and native codegen and through scripts. Validate browser VM/portable modules and native embedded/process players. Test low GPU limits and forced CPU paths, not just the developer's GPU. Keep generated shell/MCP schemas and editor actions in end-to-end checks.

### 17. Implementation sequence and gates

| Phase | Deliverable | Exit gate |
| --- | --- | --- |
| 0. Architecture/performance spikes | ABI prototype, dynamic component/block, runtime QML hosting, browser portable module, compute-to-mesh path, CPU smooth mesher, shaped instancing and fracture connectivity experiment | A package outside engine source installs, edits data, runs a block and ships; measured results settle tier/mesher/page choices |
| 1. Package foundation | Manifests, resolver, lockfile, cache, local development, manager, compatibility checks and transactional install | Conflict, rollback, offline and missing-plugin fixtures pass without data loss |
| 2. SDK and complete extension path | Services, data records, migrations, lifecycle, editor contributions, commands, VM/codegen/script bridge, importer/build hooks | Independent tiny plugin works using only SDK; native and web player artifacts execute the same plugin calls |
| 3. Voxel vertical slice | Finite smooth/shaped world, CPU generation/meshing, palette, brushes, collision, undo and player save | Sculpt a cave, place shaped blocks, paint emission, mine through blocks and ship/save/reload the project |
| 4. Procedural worlds and streaming | Graph editor, biomes/caves/ores, stamps/structures/scatter, sparse pages, LOD transitions, invokers, large-world origin integration | Traversal/teleport and seam fixtures pass; identical generation independent of load order |
| 5. GPU production path | Compute generation/meshing, arenas, instance culling, warm-up, quality feedback and CPU fallback | Desktop budgets pass; browser/iGPU profiles degrade predictably; no routine mesh readback |
| 6. Live worlds and fracture | Revisioned jobs, collision/nav invalidation, checkpointing, cross-page support, movable editable fragments and debris policies | Continuous edits and support removal remain responsive; persistent material survives budgets and save/reload |
| 7. Ecosystem and shipping | Registry publish flow, SDK docs/examples, compatibility CI, profiling tools, recovery, platform qualification and full authoring polish | Third-party author ships an independent plugin without engine edits; voxel feature contract and target matrix pass |

Dependencies: phase 2 requires phase 1's lifecycle/package identities; phase 3 requires the minimum phase 2 edit/build/block services. CPU voxel work and SDK expansion can proceed concurrently once those contracts are stable. Phase 5 accelerates established canonical semantics. Phase 6 prototypes start in phase 0 because fracture can change storage requirements, but its production gate follows stable edits/streaming. Do not postpone browser/ABI proof until the end.

The first implementation task is phase 0, not a full marketplace UI. Produce an architecture decision record with benchmarks and one installable proof package. The first usable voxel milestone is phase 3; the complete requested plugin requires phases 4-7 as well.

### 18. Proposed code boundaries

New names below are proposals:

- `blockloom-plugin-api`: ABI definitions, schemas, handles and compatibility rules without Qt/Bevy dependencies.
- `blockloom-plugin-host`: resolver/cache, lifecycle, portable/native loaders, service dispatch and diagnostics.
- `blockloom-plugin-sdk`: developer scaffold, bindings, conformance harness and packaging tools.
- `plugins/voxel/`: independently packaged voxel core, native/portable entry points, shaders, editor declarations, samples and tests. It must not rely on private sibling paths to publish.
- `blockloom-core`: generic extension records, references, plugin instructions/events, storage reachability and pack metadata.
- `blockloom-app`: authoritative plugin transactions, dynamic dispatch/specs, history and project lock integration.
- `blockloom-protocol`: versioned registration/capability and revisioned world-job messages where editor/runtime transport requires them.
- `blockloom-runtime`: service adapters in world, streaming, rendering, physics, quality, script and player paths.
- `blockloom-qt`: Plugin Manager, contribution host, tools and Voxel workspace. List new built-in QML files in `build.rs`.
- `mcp/`: dynamic structured schema discovery and plugin command exposure.

Keep authored data processing independent of Qt and GPU state. Bump protocol, pack, script and plugin ABI versions only for changes to their respective contracts. Prepare patched dependencies before direct Cargo on a fresh checkout, build the whole workspace for editor/runtime integration, and follow the existing target-cache budget recipes.

### 19. Risks and decisions to close before expansion

| Risk | Decision or mitigation |
| --- | --- |
| Engine-private rendering APIs undermine plugin compatibility | Prove a small stable render service; isolate exact-build adapters and publish their constraints |
| WASM call/copy costs undermine voxel throughput | Bulk page/job APIs, native fast path and browser profiling in phase 0 |
| Smooth/shaped representations multiply complexity | Explicit storage/composition semantics and shared transaction/query services |
| GPU meshing competes with frame rendering | Budget dispatches, profile end-to-end latency and keep CPU fallback |
| Collision/navigation trails destructive visuals | Revisioned readiness, conservative interim policy and prioritized cooking |
| Fracture searches become global | Incremental cross-page support summaries, bounded jobs and unknown-boundary policy |
| Persistent edits grow indefinitely | Checkpoints, compaction, quotas, crash recovery and history-aware retention |
| Large-world rebasing breaks existing systems | Engine-wide origin contract and finite-world release until qualified |
| Scope produces a demo instead of a platform | Independent plugin conformance fixture and final feature/target gates |

Network multiplayer is an extension track after the local system: authoritative operation sequencing, generator/palette identity, chunk checkpoints, bandwidth limits, late-join and fracture decisions. Reserve revision/operation identities now, but do not claim existing networking or promise bit-identical physics. Advanced fluid simulation, planetary terrain, custom dual contouring and commercial marketplace billing are later features; they do not replace any requested capability.

This is a multi-subsystem program. Estimate staffing and delivery dates after phase 0 establishes native/browser costs and SDK boundaries. Track platform and voxel gates separately so a working voxel prototype cannot be mistaken for a complete Unity-level plugin ecosystem.
