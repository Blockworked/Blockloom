# Rendering

Batching, LOD, quality scaling, GPU measurement, streaming, shader plumbing, import roles, models and surface shaders. Split out of `CLAUDE.md`; the index there lists every page.

## Instancing and batching

`blockloom-runtime/src/batching.rs`, 3D only. A plain surface (no graph
shader, no box projection, not a tilemap) draws through `InstancedMaterial`:
the shared material is keyed by `surface_key`, the surface minus color and
UV transform, and those two ride per actor in one storage buffer indexed by
its `MeshTag` (`InstanceTable`, slot 0 the identity). So `set color` writes a
slot rather than making a material, and same-mesh actors stay one draw.

`batch_meshes` (PostUpdate) merges what instancing can't group: actors with
no body or a fixed one that hold still for `settle_frames` merge per
streaming cell and surface, on a `CellTasks` background task (members draw
themselves until it lands), and one that moves after merging draws itself for
the rest of the run; small movers merge per surface every frame. A batched
actor keeps its entity and only loses its own draw, through
`RenderLayers::none()`. Merged meshes bake tint and UVs into vertex colors and
UVs. `BatchPolicy` holds every threshold. Counts reach the profiler as
`batching/*` render metrics.

## LOD and occlusion

`blockloom-runtime/src/culling.rs`, 3D only. `LodGroup` is the one LOD
selector: levels of a screen-size threshold (bounding diameter over viewport
height) plus an optional mesh, picked with a hysteresis band in `select_lod`
and announced by `LodChanged`; below the last level the entity leaves the main
view. Content hands it levels (`RenderCache::lod` does spheres and capsules)
rather than selecting its own, and batching leaves LOD'd actors instanced.

`cull_views` runs between Bevy's frustum pass and
`MarkNewlyHiddenEntitiesInvisible`, removing LOD-culled and occluded meshes
from the world camera's `VisibleEntities` only, so shadows are untouched.
Occlusion is a software Hi-Z: the camera-facing faces of solid, opaque
`Occluder` boxes (cuboid and plane actors) are rasterized, eroded a texel and
reduced to a max-depth pyramid. Bevy's GPU `OcclusionCulling` and GPU frustum
culling (`NoCpuCulling`) are camera toggles in `OcclusionPolicy`. Counts reach
the profiler as `culling/*`.

## Quality and scaling

`blockloom-core/src/quality.rs` owns saved settings, preset budgets and frame-time
feedback. `blockloom-runtime/src/quality.rs` applies run-only overrides, LOD and
render-target scaling, and publishes `quality/*` and `budget/*` profiler rows.
Geometry pressure drops density/distance without reducing pixel rate; frame-time
pressure lowers resolution to its floor first. Draw counts estimate mesh and
material groups in the active world camera's visible list, excluding shadow/post
passes and 2D sprites. Rebuilds clear the previous world's sampled costs.
Terrain, vegetation, VFX and debris have independent sustained auto-drop
multipliers for LOD distance, grass density and transient pool capacity. They
reach a local floor before geometry asks for a shared preset drop. These run-only
reductions reset on setting changes or rebuilds. The profiler reports terrain
and vegetation under `budget/<system>/distance_scale`, VFX and debris under
`budget/<system>/density_scale`. Fractured child meshes count toward debris.
Water has its own sustained tessellation throttle, reported as
`budget/water/tessellation_scale`. Preset density and the local multiplier thin
lake/river grids and ocean rings while keeping their full extent. Water floors
count toward water costs and keep their meshes when only tessellation changes.
Props have a local LOD-distance throttle too (`budget/props/distance_scale`),
applied to every prop level. Spheres and capsules swap to coarser meshes and
then leave the view far out; cuboids and planes carry a
cull-only level, so visible ones still merge into batches while culled ones
stay out of merges entirely. Model placeholders carry a cull-only level:
`model::sync_model_lod` keeps visible meshes intact and hides the `ModelChild`
subtree when culled, shedding the scene's draws. If props stay over budget at the local
floor, shared preset feedback takes over. Draw groups serving more than one
visible mesh count as `quality/instanced_draws`, with the absorbed meshes in
`quality/batched_instances`. The render world also counts what batching
actually issued (`indirect.rs`): the world cameras' binned opaque, mask,
deferred and transparent phases after batching, as `indirect/api_draws`,
`commands`, `instances`, `multidraw_sets` and `transparent_draws` with a
`gpu` flag saying the renderer has counted, plus per-system
`indirect/draws/<system>` membership from the CPU's grouping, since bins
don't know which system a mesh belongs to. Before the first render count
the totals mirror that grouping instead. Texture streaming maps the preset onto surface
samplers (a lod clamp plus an anisotropy cap, re-applied on quality change),
laid over per-texture distance steps (`Quality::distance_mip_steps`, tracked
from each variant's nearest material user in `materials::track_texture_distances`):
far walls stream coarser while near ones stay sharp. Baked data textures
(cloud noise volumes, grading LUTs, lens dirt) never register as variants.
Grading LUTs share one depth-stacked 3D atlas (`post::LutAtlas`, paged in the
finish shader; a new grid size rebuilds it), and authored cloud noise shares
one stacked 3D atlas too (`clouds::noise_atlas_texture` over
`pipeline::pack_volume_atlas` sizes): shape under detail in one image, paged
in the march shader through `noise_shape`/`noise_detail`, so the march and
the shadow pass bind once instead of once per volume. Baked pages render
into scratch textures and copy into their slab once.

The main-world camera stays at native size for input. Extracted world-view targets
scale before rendering and Bevy's spatial blit fills the output. Custom sub-viewports
stay native. TAA is 3D only. A DLSS ask runs Bevy's `dlss` path in a build made
with `--features dlss` (`just player-dlss`): `just dlss-sdk` fetches the pinned
SDK once per clone (sparse, about 330MB under gitignored `third-party/dlss`;
cloning it accepts NVIDIA's license), the justfile exports `DLSS_SDK`, and a
Vulkan SDK plus clang complete the build (`VULKAN_SDK`, default `/usr` on
Linux). The `Dlss` component carries the effective perf mode on perspective
HDR world cameras - plain super resolution, or the ray-reconstruction
variant while Hybrid Solari lights the view - and drives its own render resolution while the spatial
`scale_views` stands down. Manual modes step down with the dynamic-resolution
signal; `Auto` (the default) leaves the ratio to the SDK. Otherwise - no SDK
build, no RTX, no Vulkan,
missing DLLs, 2D, or web - it rides TAA plus spatial in 3D and
spatial in 2D and on web. The probe reads `DlssSuperResolutionSupported` where
the feature exists and the adapter elsewhere, reports through the same
`dlss_available` flag the reporters read, and says why once in the run log
when a run asks for it. The redistributable (`nvngx_dlss` / `libnvidia-ngx-dlss`
plus the `dlssd` twin and a license blurb) stages beside the player payload and
ships beside each built game; without it the game falls back to TAA. With scene scaling,
a separate unlit UI camera composites at native resolution after the scene blit,
using premultiplied alpha and no world render layers. It follows the world output
and owns `IsDefaultUiCamera` while active. Under HDR the world's tone curve stays
on the scaled scene while the encode moves onto the UI camera, so the HUD is
encoded at paper white at full resolution instead of blending in linear light.
Script rendering overrides end on a world rebuild.

`ssr.rs` traces only specular light into the shared half-size scratch target,
then adds it to the scene with the depth-guided upsampler. Bevy still supplies
SSR uniforms and view layouts; `pbr_patch` makes its shader emit just the light
under `BLOCKLOOM_HALF_SSR`. Unpatched shaders, compiling pipelines and custom
viewports keep Bevy's full-size pass. The trace timing is `ssr_half`.

`CellTasks` bounds concurrent work per payload type and admits nearest cells first.
Global HDRI/noise jobs use `GLOBAL_CELL`, remain part of warmup accounting across
cell resets, and cancel when their content key changes. HDRI quality caps discard
highest-resolution baked mips before GPU upload, retaining BC6H where supported.
The preset caps authored volumetric quality too: fog grids step down to the
preset's grid and cloud march steps step down to its `CloudQuality`
(`clouds::capped_quality`), so a Low world never marches Ultra steps.

## GPU measurement

`blockloom-runtime/src/gpu.rs`. Bevy's `RenderDiagnosticsPlugin` times each
pass; `gpu.rs` adds one timestamp pair around the whole frame (an encoder
pushed in `RenderGraphSystems::Begin` and one before `Submit`, read back a few
frames later) as `gpu/frame`. Every `MEMORY_EVERY` frames it sorts the wgpu
allocator's report into `Kind`s by allocation label (`Kind::of`), so a new
render target needs a label that sorts, not code. Without a report (Metal) it
sizes the views' own textures instead. All of it reaches the profiler as
`gpu/*` and `memory/*`.

## Loading and streaming

`blockloom-runtime/src/streaming.rs`. `StreamingCells` is the one cell system:
XZ cells around the world camera with a hysteresis band, admitted nearest
first under a per-frame budget and announced as `CellEntered`/`CellLeft`
messages (a rebuild sends every cell as left). Content streams by being a
payload: it reacts to those messages and does its work through `CellTasks`,
which runs on the async compute pool and holds the cell as loading until the
task lands. Terrain chunks and the like belong there, not in a second system.

A look whose files are still loading gets `Loading`: a flat `Placeholder`
child in 3D (Bevy draws nothing for a material whose texture isn't in yet),
and a `FadeIn` on the sprite in 2D once the image lands. A model's own box is
its placeholder already.

Every rebuild opens a `Warmup` window: everything with bounds is drawn
unculled (`NoFrustumCulling`, and `cull_views` stands down), so every pipeline
compiles now rather than when a thing first turns up on screen. It closes
after a few quiet frames with no loads, no loading cells and no pipelines in
the render world's backlog (`PipelineBacklog`), or at its timeout. Start
doesn't press the green flag itself in a rendering world: `engine.starting`
holds it (reported to the editor as running) and `warm_up` calls
`world::begin_run` when the window closes, so a built player's first frame
is warm too. A bare test world (`engine.prewarm` false) starts on the spot.
At build time `build::check_shaders` compiles every `.wesl` surface file the
project draws with, and a broken one fails the build. Counts reach the
profiler as `streaming/*`.

## Shader library and pass plumbing

`blockloom-core/src/shader_lib.rs` holds Blockloom's own WESL modules
(`src/shaders/`): `hash` (PCG), `noise` (value, gradient, Worley), `fbm`,
`scattering` (phase functions, Beer, per-step integral) and `frame`, the
standard per-view `FrameUniforms`. They live in core so the editor's shader
check can link them; `shader_lib::validate` links a module against them and
runs naga, which is how their tests (and the runtime's shader tests) check
them without a GPU. The runtime registers each as `blockloom::<name>`
(`blockloom-runtime/src/passes.rs`), so built-in passes and surface files
import them like Bevy's.

`passes.rs` is also the shared pass plumbing, both dimensions. A camera
asks for it with `WorkingTargets` (like `DepthPrepass`); the render world then
gives the view a `WorkingTargetSet` (one full-res `Rgba16Float` target and a
half-res scratch pair, labelled `working_*`), `ViewFrameUniforms` into the one
`FrameUniformBuffer` filled from the extracted `Environment`, and
`ViewUpsample`'s bilateral upsample pipelines, queued up front so warm-up
compiles them. `passes::upsample` draws scratch onto a full-res target,
guided by the 3D depth prepass (plain bilinear in 2D). A half-res texel
stands for the full-res pixel at twice its coordinate (`frame::full_texel`).
Nothing asks for `WorkingTargets` yet; Phase 5's passes are the consumers.

## Import roles, probes and the atmosphere slot

The asset pipeline (`blockloom-core/src/pipeline/`) inspects, plans and
fingerprints every file. An `ImportRole` says what a file imports *as*:
texture, HDR (`.hdr`/`.exr`, planned as BC6H), volume (`.cube` LUTs, or an
image strip of slices), heightmap (16-bit images, `.r16`/`.r32`), IES profile,
or light cookie. The extension picks one; a PNG heightmap or cookie is an
override kept in `assets.index` (`set-import-role`), and changing
a role dirties the asset. Each role's decoder is its own submodule, and a pass
reads files through `pipeline::load_volume`/`load_heightmap`/`load_ies` rather
than parsing them itself. A new format extends a role here, not a second
importer.

`blockloom-runtime/src/probes.rs` is probe capture as a service, 3D only:
`ProbeService::request` spawns six 90° FP16 face cameras (`FACES` is cubemap
order, with Bevy's z flip), announces `ProbeCaptured` after a couple of
frames, and with `readback` assembles the faces into a cube image. HDRI
baking, reflection probes and water reflections are requests with different
defaults (`ProbeRequest::hdri`/`reflection`/`water`); none owns cameras.

`Sensors::atmosphere` is the snapshot's sun/wind/fog/weather slot, versioned
by `ATMOSPHERE_VERSION`. `atmosphere::sample_atmosphere` writes it at the head
of every fixed tick, before any scheduler, from the blended `Environment` and
`AtmosphereSources` (which Phase 5's wind and weather systems fill), and
`publish_sensors` copies that sample rather than resampling, so a frame
between ticks reads what the tick read. Every reader goes through
`AtmosphereSense::field`: the `Atmosphere` reporter (VM, and compiled logic
through `sense`) and a script's `atmosphere()` (`READ_ATMOSPHERE`). A new
reading is one arm there, one entry in `ATMOSPHERE_FIELDS` and the reporter's
dropdown in `Blocks.qml`.

## Models, tilemaps and surface shaders

A `Visual::Model` draws its glTF/GLB file's first scene as a child of the
actor (`blockloom-runtime/src/model.rs`), scaled by the look's `scale`, and
loops the animation the look names (the file's first when empty). The
authored box stands in until the scene is ready, stays for OBJ/FBX or a file
that won't load, and is always what the actor collides as. `ModelCache` keeps
loaded files alive across rebuilds so an edit doesn't flash the box.

A solid tilemap collides per tile, as the merged rects of
`Tilemap::solid_rects` (minus its `passable` tiles), and nav blocks the same
rects. Ray and overlap queries see them too, as a `ColliderShape::Parts`
that `tiles::publish_level` fills from the live map. Every query shape is in
its actor's frame and turned by its rotation (`physics_query::to_local`). Animated tiles cycle their frames on the wall clock;
`materials::animate_tiles` rewrites only the mesh's UVs when a frame turns.

A custom effect's `GraphEffect::starter_graph` is the uniform path spelled as
graph nodes, so `export_shader` writes it to a `.wesl` asset that draws the
same thing. An effect whose `source` names a `.wesl` file draws with that
file's `graph_main(uv, time)`: `material::surface_module` wraps it in the
dimension's head, `SURFACE_BINDINGS` and fragment tail, hoisting the file's
own imports up beside the head's (WESL wants every import before the first
declaration), and `materials::surface_shader` swaps that in through the
material's `specialize`. `check_surface_wesl` parses the same module, refuses
names the wrapper already has and imports the dimension hasn't loaded, and
runs naga's full type check when the file has no imports, so the editor and
the GPU agree on what compiles.

The built-in shaders (`src/shaders/*.wesl`) and that wrapper are WESL, since
Bevy 0.20 hands plain WGSL to wgpu untouched: imports are `import
bevy_pbr::render::...`, shader defs are `@if(DEF)`, and the bind group is
`constants::MATERIAL_BIND_GROUP`. A user's surface file is WESL too and may
import Blockloom's library (`blockloom::fbm`, ...) and Bevy's own modules
(`bevy_pbr` in 3D, `bevy_sprite_render` in 2D);
a project's other files aren't modules, so `package::`/`super::` are refused.
