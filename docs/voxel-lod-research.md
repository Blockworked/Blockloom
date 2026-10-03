# Voxy LOD research and Blockloom design

Research date: 2026-10-03. This is a source review and implementation design;
storage, derived mip samples, coarse mesh jobs and camera selection are
implemented, including opt-in visual publication.

## Requested chunk dimensions

A Blockloom chunk is a column with a 32 by 32 cell footprint in X/Z. Its
height is the world's configured height limit, currently represented by
`world.size[1]`. The height is an exact logical bound, not rounded upward to
a section boundary. A height of 100 means cells 0 through 99 are valid.

Internally, divide columns into sparse 32 by 32 by 32 sections. The last
section is clipped to the height limit; padding is air and cannot be edited.
A height of 256 has eight possible vertical sections per column. Empty
sections need no dense allocation. Sections are storage and meshing units;
they do not change the full-height definition of a chunk.

Keep column addresses `(chunk_x, chunk_z)` distinct from section addresses
`(section_x, section_y, section_z)` and LOD addresses `(level, x, y, z)`.
Column footprint is measured in cells; world-space width is
`32 * voxel_size`. Finite horizontal edge columns can also be clipped.

## What upstream Voxy does

Reviewed upstream [MCRcortex/voxy](https://github.com/MCRcortex/voxy), pinned
to commit `534d58ec8b4aa412ef314b884295552c69d480a6`. Fork-specific world
generation and networking features are not assumed to be upstream behavior.

### Voxel hierarchy

`WorldSection` stores 32 cubed samples at every detail level. `WorldEngine`
sets the maximum LOD layer to 4, giving levels 0 through 4. Each coarser
sample spans twice the distance on every axis. This yields:

| Level | Sample width in base blocks | Section span on each axis |
| --- | --- | --- |
| 0 | 1 | 32 |
| 1 | 2 | 64 |
| 2 | 4 | 128 |
| 3 | 8 | 256 |
| 4 | 16 | 512 |

Eight child sections occupy a parent section's volume. Sections track
non-empty children so traversal can skip absent branches. This is a
volumetric hierarchy, capable of representing caves and overhangs.

Sources:
[WorldSection](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/common/world/WorldSection.java),
[WorldEngine](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/common/world/WorldEngine.java).

### Ingestion and downsampling

Minecraft's loaded sections are converted into voxel records on service
workers. A 16-cubed input section is reduced through 8-cubed, 4-cubed,
2-cubed and 1-sample representations, then inserted into the appropriate
regions of the 32-cubed world sections at each level.

The actual `Mipper` selects a non-air child with the highest mapped opacity,
using a fixed corner ordering to break ties. It carries the selected child's
record. When all eight children are air, it combines their lighting. It
does not currently select the most common material, average block colors,
or use a signed-density surface filter. A single non-air child can therefore
survive reduction, but become visually thicker at coarse levels.

`WorldUpdater` propagates changed data and child-existence state upward,
marks affected sections and boundary neighbors dirty, and stops early when
neither data nor relevant existence state changes. Dirty callbacks and
storage callbacks separate rebuild work from persistence. Stored section
data can survive unloading the corresponding Minecraft chunks. The ingest
path consumes available chunks; distant rendering itself does not imply
generation of unexplored terrain.

Sources:
[VoxelIngestService](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/common/world/service/VoxelIngestService.java),
[section reduction](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/common/voxelization/WorldVoxilizedSectionMipper.java),
[Mipper](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/common/world/other/Mipper.java),
[WorldUpdater](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/common/world/WorldUpdater.java),
[SectionStorage](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/common/config/section/SectionStorage.java).

### Detail selection and visibility

The compute traversal projects each section's bounds and estimates projected
area. If that area exceeds the subdivision threshold, it descends into
children; otherwise it draws the current section mesh. The host normalizes
the configured subdivision size squared by viewport pixel count.

Traversal applies a horizontal render-distance bound, frustum rejection and
hierarchical depth-buffer occlusion checks. GPU request queues ask for missing
detail, with duplicate suppression and queue limits. When finer children
are not ready, the traversal can retain the current coarser mesh, subject
to its render-distance boundary checks. This avoids requiring every fine
section to be loaded before distant terrain is visible.

These are screen-area decisions, not a measured geometric-error metric.
The reviewed traversal does not establish a two-threshold hysteresis policy
or smooth interpolation between LOD meshes.

Sources:
[traversal shader](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/resources/assets/voxy/shaders/lod/hierarchical/traversal_dev.comp),
[screen-space and depth checks](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/resources/assets/voxy/shaders/lod/hierarchical/screenspace.glsl),
[traversal host](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/client/core/rendering/hierachical/HierarchicalOcclusionTraverser.java).

### Geometry production

Mesh generation is performed by CPU service workers. The render factory
culls faces using block-model metadata and neighboring samples, merges
compatible faces with a scanline greedy mesher, and packs quads into compact
records. It distinguishes translucent, double-sided and directional output.
GPU compute traversal and rendering do not mean GPU voxel mesh generation.

Sources:
[RenderGenerationService](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/client/core/rendering/building/RenderGenerationService.java),
[RenderDataFactory](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/client/core/rendering/building/RenderDataFactory.java),
[ScanMesher2D](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/src/main/java/me/cortex/voxy/client/core/util/ScanMesher2D.java).

## Applying this architecture to Blockloom

These are proposed Blockloom decisions, not claims about Voxy:

1. Migrate addressing to 32-cell sections and expose 32 by 32 full-height
   columns. Keep exact world height separate from section allocation size.
   Stream selection groups columns horizontally, with vertical sections
   scheduled within explicit section and byte budgets.
2. Build a sparse, versioned voxel mip hierarchy from canonical generated
   data plus runtime edits. Start with levels 0 through 4 and permit future
   policy expansion. Far nodes should not require all full-detail pages to
   remain resident. Generation, edits, shape changes and fracture removals
   invalidate the affected ancestor chain and boundary dependencies.
3. Select visual detail using projected section area, render distance and
   camera information. Add split/merge hysteresis to limit camera jitter.
   Keep collision and interaction residency driven by invokers at full
   detail. A distant visual node never becomes authoritative gameplay data.
4. Preserve the last valid covering mesh while replacement work is queued.
   Publish a coherent parent/child replacement only after required meshes
   and boundary data are ready. Validate generation and revision identities
   before installing results. Do not draw overlapping parent and child
   coverage or expose missing regions during refinement and edits.
5. Use material/opacity-aware occupancy proxies for cubic and shaped terrain.
   Include silhouette and thin-structure fixtures. Smooth density terrain
   needs its own reduction rule and transition geometry between resolutions;
   the existing tetrahedral extractor alone does not supply LOD seams.
6. Keep mip data and meshes as disposable caches derived from authoritative
   cells. Save cache identity with generator, palette, reduction version and
   world revision if persisted. A stale cache can be rebuilt without losing
   player edits or detached-body saves.
7. Add renderer integration for camera/viewport inputs, GPU visibility and
   compact geometry allocation in stages. The current plugin compute API
   exposes buffers and dispatches, but does not expose Voxy's depth texture,
   traversal queues or compact quad draw pipeline as ready-made services.

### Implementation status

Storage, reduction, coarse mesh production and selection are implemented in
`plugins/voxel`:

- Storage uses sparse 32-cubed sections and distinct X/Z column addresses.
  All world dimensions retain their exact cell bounds. Empty resident sections
  carry residency metadata without a dense cell allocation.
- Checkpoint version 2 records the page width explicitly. Version 1 pages
  decode at their original 16-cell width and repack by cell coordinate.
  Migration clips cells outside the configured logical bounds, including old
  rounded padding. Legacy procedural worlds retain their original generator
  bounds so in-bounds terrain does not change when saved again. Shapes,
  density overrides and detached-body snapshots keep their cell coordinates.
- Render and collision outputs use at most 16-cubed mesh tiles within each
  section. CPU and GPU jobs share clipped tile bounds and canonical halo
  samples. Existing `chunk/x/y/z` mesh names continue to address those tiles;
  they are not column or storage-section addresses. GPU capacity checks retain
  CPU rendering and collision when an output cannot be allocated.
- `stream_radius` measures horizontal distance in 32-cell columns.
  `vertical_radius` independently limits vertical section selection (default 2).
  Selection prioritizes horizontal proximity, then vertical proximity, under
  `max_pages` (resident section count) and `max_resident_bytes` (dense section
  cells, default 16 MiB). `pages_per_tick` budgets section rebuilds, each of
  which can publish multiple tiles. Sparse edits, metadata, temporary jobs and
  GPU allocations are outside the dense-cell byte budget; GPU output retains
  its existing independent 64 MiB budget.
- `count` reports drawn columns (`chunks`), `column_counts`, `resident_columns`,
  `drawn_sections`, `resident_sections`, dense-cell `allocated_bytes`,
  `gpu_allocated_bytes`, `lod_nodes`, `lod_samples`, `lod_sample_limit` and
  `voxel_revision`. Coarse jobs add `lod_mesh_tiles`, `lod_mesh_tile_limit`
  and `lod_generation`.
  `resident` and `pending` still count sections.
- `lod.rs` defines reduction version 1 for levels 0 through 4. Sample addresses
  use each level's lattice; node addresses group 32-cubed samples at that level.
  Queries recursively reduce canonical terrain and edits without loading base
  sections. A FIFO cache retains at most 8192 derived sample records and queue
  entries, including cached air; nodes currently contain only requested samples.
  This is a record-count budget separate from dense-cell and GPU budgets, not an
  exact heap-byte limit. Mip caches are disposable and are omitted from saves.
- Cubic reduction selects the child with greatest shape occupancy proxy:
  cube 255, stair 192, slab/ramp 128 and post 64. Ties keep the first corner in
  `x + 2*y + 4*z` order. All current materials are opaque; this ranking models
  shape coverage, not palette alpha. One occupied child survives, so thin
  structures thicken at coarse levels. An eight-bit mask records occupied child
  octants. Coarse samples are whole-cell proxies, not scaled copies of shapes.
- Smooth reduction averages signed child densities and halves their magnitude
  to express distance in the coarser lattice. Zero maps to positive 1, except
  a negative sum truncated to zero maps to -1. A negative result carries the
  material of the most negative child, with the same corner tie rule. Shaped
  cells remain outside the smooth field. This filter can erase thin smooth
  features and does not provide transition geometry.
- Every material, shape, density and fracture removal invalidates the exact
  cached sample on each ancestor level. Unrelated samples remain usable.
  Regeneration and logical-bound changes discard the cache; restore starts
  with a fresh cache. A grid revision changes on edits for future mesh result
  validation, but is local to that grid lifetime, not a saved generation id.
  Nonprocedural authoritative sections are protected from residency eviction.
- `voxel_lod_sample` (`lod_sample` module op) exposes these samples for inspection.
  Its X/Y/Z arguments are coordinates in the chosen level's lattice, so level 4
  sample `[2,0,0]` covers base X cells 32 through 47. The response includes
  material, opacity proxy, density, density material, child mask, revision and
  reduction version. These queries do not alter rendered or collision meshes.

- `lod_mesh.rs` builds coarse CPU geometry through the existing cube and smooth
  meshers. Coarse render tiles span at most 8 samples per axis, independently of
  the 32-sample hierarchy node. Positions scale by `2^level * voxel_size`;
  emission groups retain their material identity. Smooth jobs use reduced
  density materials and add whole-cell proxies for selected non-cube shapes.
  Outer edge vertices project onto the exact world box; degenerate triangles
  are removed and moved triangles receive geometric normals.
- Mesh jobs sample a conservative two-sample halo, including smooth gradients
  and diagonal neighbors. Their dependency box covers those samples in base
  cell coordinates. Edits invalidate intersecting ready and partial jobs,
  including neighboring tiles across a boundary. Changed cells are accumulated
  into one bounding box per flush, which can invalidate additional tiles when
  separate edits span a large region. Disjoint jobs remain cached. Regeneration
  and successful restore/load clear the cache and advance a plugin-local
  generation identity, retained across world stop/start. The response also
  includes the job's original grid revision.
- A mesh poll debits at most 65536 potential base-cell visits. Each in-bounds
  sample costs `8^level` even when it is already cached; out-of-bounds samples
  cost zero. This bounds sampling work without requiring full-detail residency.
  Sampling continues on the next poll, and geometry is exposed only after its
  entire halo is ready. The cache retains at most four partial/complete tiles,
  evicting the oldest request when full. Each tile has a 1 MiB geometry payload
  limit; oversized output fails the request. Vector capacity, sample storage,
  meshing scratch and serialized responses are outside that payload limit.
- `voxel_lod_mesh` (`lod_mesh` op) polls this pipeline for inspection. X/Y/Z
  address 8-sample tiles at the chosen level, not base cells or hierarchy nodes.
  For example, level 2 tile `[1,0,0]` starts at base cell X=32 and spans up to
  32 base cells. Repeat until `ready` is true; `meshes` is empty while pending.
  The response reports sample progress, the conservative `base_visit_bound`,
  dependencies, revision and generation identities. Mesh names start with
  `lod/<level>/<x>/<y>/<z>`, with an optional glow suffix. These meshes have no
  collision or GPU allocation and are returned as data, not renderer effects.
  Full-detail meshes continue to provide gameplay; drawing defaults to full detail.

- `lod_select.rs` traverses a perspective view from level 4 roots to level 0
  tiles. `voxel_lod_select` (`lod_select` op) takes world-space `position`,
  `forward`, `up`, `viewport_width`, `viewport_height`, vertical `fov_y` in
  degrees, `render_distance` in world units, and `split_pixels`/`merge_pixels`.
  Axes are normalized and orthogonalized. A conservative bounding sphere tests
  the side and eye planes; closest AABB distance tests a spherical render range.
  The squared projected bounding diameter estimates area. This is conservative
  screen coverage, not a measured terrain error or depth occlusion query.
- Previously split tiles use the lower merge threshold; new splits use the
  higher split threshold. History resets on generation, restore/load and world
  restart. Invalid requests leave history intact. Selection reads bounds only,
  leaving canonical cells, residency, mip/mesh jobs and renderer effects alone.
- Traversal permits at most 512 candidate roots, 512 output leaves and 4096
  visits. A view exceeding the root budget fails before changing history.
  A split exceeding either remaining traversal budget retains its parent and
  reports `budget_limited`. Desired leaves do not overlap, and contain clipped
  base-cell bounds, level/tile addresses and projected area in squared pixels
  (reported to three decimal places). Level 0 leaves address 8-cell tiles;
  original fine/collision tiles still span 16 cells and remain separate.
  Selection includes empty terrain and does not request or build meshes.

- Presentation hooks now receive an optional `view` snapshot with the active
  world perspective camera's world pose, logical viewport dimensions and vertical
  FOV in degrees. Fixed-stage calls receive null. The host's existing `run_stage`
  remains available without a view. Preview and orthographic views retain fine
  rendering. No depth or occlusion resources are exposed by this snapshot.
- Set the voxel world's `visual_lod` to true to enable experimental publication.
  `lod_distance` defaults to 256 world units; `lod_split_pixels` and
  `lod_merge_pixels` default to 160 and 120. Runtime selection has its own
  hysteresis history, separate from inspection. Invokers still determine fine
  section/collision residency, while camera selection can draw canonical distant
  terrain without loading those sections. Detached bodies retain their normal
  rendering and physics.
- `lod_publish.rs` stages a complete desired cut, advancing at most one tile job
  per presentation frame. Level-zero jobs use the exact cube/shape or smooth
  extractor, in 8-cell render tiles. Coarse jobs retain their occupancy proxies.
  The target cut accounts for at most 2 MiB of geometry and boundary words,
  independently of the four-tile job cache and renderer allocations. Reused
  installed tiles count toward that limit; only rebuilt tiles retain full
  geometry in staging. Installed tiles retain names, dependency bounds derived
  from their keys, payload size, validity, neighbor layout and solid face caps.
  Face cap coordinates are f64 (two words per coordinate), retaining no full
  CPU mesh copy. Scratch, vector/map metadata and capacities, JSON and temporary
  duplication during publication are outside this payload budget.
- No staged meshes are sent to the renderer until every tile in the cut is
  ready and validated. One effect batch replaces the previous visual set and
  hides fine rendering through `mesh_visibility`, preserving fine colliders and
  GPU buffers. Visibility persists across replacement of the same named mesh.
  The runtime applies the batch before the next render. Empty tiles also count
  as ready, and names absent from the replacement are removed.
- Edits invalidate installed tiles, staged geometry and partial jobs whose
  two-sample dependency halos intersect the accumulated edit box. Joined tiles
  also depend on their neighbor's halo; both sides of a changed join rebuild.
  Unaffected tiles and queued sampling survive, and the target revision advances after
  invalidation. Inspection mesh polls and fine flushes broadcast the same changes
  to both mesh paths. A missing change box forces a conservative full rebuild.
  Generation changes discard all pending work and installed reuse eligibility,
  while preserving the installed cover until replacement.
- Replacement cuts reuse valid installed tiles at the same level/address and
  neighbor layout, including empty tiles, and send renderer effects only for
  rebuilt or removed groups.
  An edit outside all selected halos advances the installed revision without a
  mesh upload. Shared boundary dependencies still rebuild together; no partial
  replacement reaches the renderer. Camera changes can reuse matching installed
  leaves, but abandoned staged leaves are not retained as a second mesh cache.
  A disjoint root selection cancels old work on teleport;
  other camera motion lets the captured cut complete before requesting the
  latest view. This avoids starvation but can temporarily publish an older
  camera selection. A cut exceeding its payload budget or a rejected camera
  retains the installed cover and reports an error. Missing camera input removes
  visual meshes and unhides fine meshes. Stop removes both sets.
- Fragment publication waits for the source collider pages and the replacement
  visual cut; the fragment and new visual set are released in one effect batch.
  `count` reports visual activation, tile/pending counts, installed revision and
  generation, and the cut word limit. Old visual data may remain visible while
  edits rebuild, while gameplay queries always use current canonical cells.

- `lod_seam.rs` joins mixed-resolution faces for cubic, shaped and smooth terrain.
  Visual smooth extraction includes the lower halo and clips triangles to exact
  logical tile bounds before world-edge projection. Coplanar surface triangles
  belong to the tile containing the solid, avoiding duplicate boundary faces.
  Fine collision and inspection meshes retain their existing extraction path.
- Required boundary caps describe each tile's solid cross-section. Cubic proxies
  and exact fine shapes use their convex solids; smooth caps intersect the same
  six tetrahedra and signed densities used by the extractor. Zero-density faces
  use the interior solid limit. At each mixed face, ordinary coplanar faces are
  removed and convex polygon differences emit only the area solid on one side.
  Opposite sides supply opposite outward faces, retaining material and emission.
  Caps handle multiple fine neighbors, cavities, empty neighbors, all axes and
  level differences up to four without requiring a balanced selection tree.
- Caps build from already sampled tile halos, with no extra mip queries or fine
  residency. Only mixed faces retain caps. After all required tiles are ready,
  seam clipping advances at most one tile per frame; `visual_lod_pending` includes
  this phase. Cap construction and stitching each permit at most two million
  polygon clipping work units per tile. Shared render vertices are compacted.
  Cap payload and completed seam geometry count toward tile/cut limits; a budget
  rejection preserves the installed cover. No staged seam reaches the renderer
  before the complete cut validates against its generation and revision.

- The runtime extracts camera/frustum visibility for GPU-backed mesh assets.
  Copies from completed plugin compute buffers into Bevy's vertex allocations
  now wait until an asset has a visible mesh entity or instance. Explicitly
  hidden fine meshes retain their colliders, bindings and completed compute
  buffers; showing them later resumes the pending copy. Shared instances qualify
  their source asset even when the original mesh is outside the camera.
- Ready visible copies follow binding order, with at most 64 meshes and
  10 MiB (262144 vertices at 40 bytes each) copied per render frame. The byte
  limit admits any single API-valid mesh. An oldest ready mesh that does not fit
  waits for the next frame, so later small meshes cannot starve it. Missing
  render allocations or unfinished compute results do not block ready copies.
  CPU fallback geometry remains available until each whole-mesh copy completes.
  Completed copies are retained across visibility changes and retired on removal.
  This bounds GPU-to-GPU copies, not compute dispatches, CPU asset preparation,
  allocated vertex capacity or draw count. No depth occlusion traversal is added.

- Fine GPU meshing now packs output using a CPU prefix sum over sampled topology.
  Cubes reserve six vertices per exposed face in the requested emission group;
  smooth cells reserve three or six vertices for each active tetrahedron using
  the extractor's negative-density and first-inside-material rules. GPU work
  still computes interpolated positions and normals. Empty cells, hidden faces
  and other emission groups reserve no output space. Each cell owns a disjoint
  output span, with no atomics or production GPU readback.
- Output buffers, GPU-to-GPU copies and Bevy's raster vertex allocations use
  the packed vertex count. A lone cube in a 16-cubed tile now needs 1440 output
  bytes rather than the previous 5898240 bytes. Offset buffers are temporary and
  freed after dispatch. Dispatch still visits bounded tile anchors, and smooth
  zero-density degeneracies can leave zero triangles within active spans. Cube
  output still uses individual faces rather than the CPU mesher's greedy quads.
  The existing 64 MiB output budget and CPU collision/fallback meshes remain.
  Visual LOD meshes still use their compacted CPU geometry and seam path.

These are planar, watertight step joins, not interpolated density transitions.
Smooth joins can retain a visible crease or flat ledge where reductions disagree.
As with existing coarse geometry, overlapping shape proxies and density surfaces
can retain internal faces inside the solid union. Visual LOD remains opt-in;
all gameplay queries and collision stay canonical.
The plugin compute API still needs renderer services for depth traversal,
visibility queues and compact quad draw allocation beyond packed triangle buffers.

### Implementation order and qualification

First implement column/section addressing and checkpoint migration, then
bounded meshing for 32-cell sections, then voxel reduction and edit propagation.
Camera inputs and coherent complete-cut publication now connect the selector
to coarse jobs. Dependency-based tile reuse now avoids rebuilding the entire cut
for unrelated edits. Mixed-resolution boundaries now have planar solid-difference
joins. Renderer copies now use camera visibility and bounded FIFO scheduling;
GPU triangle buffers now pack active topology. Next improve transition appearance,
add depth visibility services and a compact quad draw path.

Qualification includes non-multiple heights (1, 31, 33, 100), old checkpoints,
column and section borders, thin structures, caves, shape proxies, smooth
LOD seams, camera threshold jitter, edit propagation, fracture removals,
teleports, delayed jobs, eviction and memory limits. Verify CPU/GPU parity,
native/WASM behavior and the whole workspace build for runtime changes.

Upstream's [license](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/LICENSE.md)
reserves rights and prohibits redistribution. Implement the architecture
independently; do not vendor Voxy source or shaders.
