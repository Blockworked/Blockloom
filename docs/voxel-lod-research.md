# Voxy LOD research and Blockloom design

Research date: 2026-10-03. This is a source review and implementation design;
storage and derived mip samples are implemented, with no visual LOD yet.

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

The storage and reduction stages are implemented in `plugins/voxel`:

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
  `voxel_revision`.
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

Camera-driven selection, coarse mesh boundary invalidation, coherent
parent/child replacement and cross-resolution seams remain to be implemented.
There is no distant visual LOD yet; all gameplay queries remain canonical.
The plugin compute API still needs renderer services for depth traversal,
visibility queues and compact quad draw allocation.

### Implementation order and qualification

First implement column/section addressing and checkpoint migration, then
bounded meshing for 32-cell sections, then voxel reduction and edit propagation.
Next add coarse mesh production, boundary dependencies, screen-space selection
and coherent mesh replacement.
Finish seam handling for both surface modes and renderer visibility/compaction.

Qualification includes non-multiple heights (1, 31, 33, 100), old checkpoints,
column and section borders, thin structures, caves, shape proxies, smooth
LOD seams, camera threshold jitter, edit propagation, fracture removals,
teleports, delayed jobs, eviction and memory limits. Verify CPU/GPU parity,
native/WASM behavior and the whole workspace build for runtime changes.

Upstream's [license](https://github.com/MCRcortex/voxy/blob/534d58ec8b4aa412ef314b884295552c69d480a6/LICENSE.md)
reserves rights and prohibits redistribution. Implement the architecture
independently; do not vendor Voxy source or shaders.
