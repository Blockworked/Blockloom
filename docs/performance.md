# Performance review

First pass, 2026-10-01. Everything here was measured in a CPU-only container
(4 cores, no GPU), so the CPU numbers are real and every GPU statement is
inferred from reading the code. GPU-bound conclusions need the in-editor
profiler (`gpu/frame`, `render/*/elapsed_gpu`) on a machine that shows the
problem.

## How to measure

```bash
cargo bench -p blockloom-core --bench vm
# ns/tick over 50 actors: arithmetic, effects, custom_blocks, lists,
# sensing (own-actor and by-name reporters), sensing_crowd (same, 1000 more
# actors in the world)

cargo test --release -p blockloom-runtime -- --ignored --nocapture sensor_publish_cost
# what one frame's sensor snapshot costs at 100 / 1000 / 4000 actors
```

## Edit cost baseline (2026-11-21)

Every editor edit still copies the whole document several times: an undo
snapshot (`push_undo` clones the project), a save that rewrites every scene
file and the index (`save_project`), a runtime sync that hands the world the
whole project (`sync_runtime`, `EditorMessage::Load`; a child runtime gets it
as one JSON line) and a world rebuild (`rebuild_world`), plus the full state
snapshot the window gets. These are the numbers to beat; Phase 3 of the
architecture review (inverse-op undo, dirty-file saves, per-document patches)
and Phase 4 (runtime deltas) should each move them.

Measured in the same 4-core CPU container, release build, on the kitchen-3d
golden fixture grown to three scenes of N actors, every actor carrying the
editor-baseline canvas. Medians.

```bash
cargo test --release -p blockloom-core --test edit_cost -- --ignored --nocapture
cargo test --release -p blockloom-app --test edit_latency -- --ignored --nocapture
```

| actors | document on disk | sync payload (JSON) | undo clone | sync serialize | save, atomic | save, plain writes | reload from disk |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 300 | 3.0 MiB | 1.6 MiB | 2.3 ms | 3.3 ms | 13 ms | 7.7 ms | 16 ms |
| 1200 | 12 MiB | 6.4 MiB | 8.4 ms | 8.4 ms | 53 ms | 32 ms | 71 ms |
| 4800 | 48 MiB | 26 MiB | 63 ms | 57 ms | 257 ms | 187 ms | 299 ms |

One whole edit through `Backend::dispatch` (no runtime attached, so no
rebuild), including the state snapshot:

| actors | state snapshot | rename an actor | undo |
| --- | --- | --- | --- |
| 300 | 1.6 MiB | 56 ms | 32 ms |
| 1200 | 6.3 MiB | 230 ms | 158 ms |
| 4800 | 25 MiB | 892 ms | 596 ms |

After Phase 3 (change-based undo, dirty-file saves and the patch stream,
2026-11-21), the same 1200-actor fixture: rename an actor 63 ms (was 230),
undo 18 ms (was 158), and the window gets a 4 KB patch instead of a 6.3 MiB
snapshot. The rest of a rename is the dirty scene's atomic save and the
runtime sync, which Phase 4's deltas and J4 (save on idle) take on. The
tables above are the pre-Phase 3 baseline.

Readings:

- Everything scales with the whole project, not with the edit: a one-field
  rename on 1200 actors costs about 230 ms, past the point where typing
  feels laggy. Building and serializing the state snapshot is the biggest
  share after the save; the undo clone is cheap by comparison.
- Atomic saves (temp file, fsync, rename; `blockloom_core::files`) cost
  about 1.4-1.7x plain overwrites here, almost all of it the fsyncs. That is
  the price of a crash never leaving half a scene file. Writing only the
  scenes an edit touched (Phase 3) removes most of it; J4 (save on idle)
  would remove the rest from the edit path.
- `rebuild_world` needs a GPU world to measure, which this container lacks.
  It now times itself: the profiler's `world/rebuild` row (ms, the system's
  own work before its spawn commands flush) and a `debug` log line with the
  actor count. Record a number from a real machine here.

## Fixed in the first PR (CPU, measured)

| Where | Problem | Before | After |
| --- | --- | --- | --- |
| `publish_sensors`, 1000 actors | snapshot rebuilt from nothing every frame: ~15 `String`/set clones per actor, a throwaway map, and 6+ linear `Project::actor` scans per actor (O(N^2)) | 6.7-9.9 ms/frame | 0.6-0.8 ms/frame |
| `publish_sensors`, 4000 actors | same | 80 ms/frame | 5 ms/frame |
| `my x position`, `touching`, `my parent`... | `me()` deep-cloned the whole `ActorSense` (strings, sets, component maps) for every own-actor reporter | sensing bench 200 us/tick | 109 us/tick |
| `distance to`, `actor position`, `how many X` | name lookups scanned every actor, per call | sensing_crowd 379 us/tick | 105 us/tick |
| `sync_navmesh`, every fixed tick | cloned every static `Actor` (block graph included) and its visual just to compare a signature | 60 Hz x all static bodies | borrows, clones only on a change |
| `interpolate_poses`, every frame | wrote every `Transform` even at rest, which marks all of them changed and makes Bevy propagate, re-bound and re-extract the whole scene | all actors dirty | only moved actors |
| compiled logic (built games) | each list/dict reporter copied every list in scope | O(total list size) per read | copies the one list it names |
| VM events | every event walked every program | O(actors) per collision | O(1) for events about one actor |
| `with_script` | two `String` allocations per strand step | 2 | 0 once warm |

## Still open, in priority order

1. **GPU defaults (inferred).** A default 3D camera is FP16 HDR with 4x MSAA
   (`Msaa::default()`), a 4-cascade 150 m sun shadow, full resolution, and
   `dynamic_resolution`/`auto_drop` off (`quality::Settings::default`). On a
   weak or integrated GPU that is the likeliest cause of "does not run well".
   Cheapest levers, in order: dynamic resolution on by default, FXAA/SMAA
   instead of 4x MSAA, 2 cascades, a shorter shadow distance. Needs a profiler
   dump from a slow scene before choosing.
2. **Fixed-step catch-up has no cap.** `Time<Virtual>` keeps Bevy's 250 ms
   `max_delta`, so a hitch runs up to 15 fixed steps in the next frame. If a
   step costs more than 1/60 s that spirals. A cap turns it into slow motion,
   but it would also slow a GPU-bound game whose sim is cheap, so it should
   adapt to the measured step cost. Not changed here.
3. **VM strand bookkeeping.** `Vm::start` scans every script and clones two
   strings per start; `tick_at` clones the actor id per strand per tick.
   Reporter arguments allocate a `Vec<Evaluated>` and `String`s per call
   (about 350 ns each in the sensing bench).
4. **Component lookup by name.** `Components::get("Body")` compares strings
   per component; `Project` derefs through `active_scene_ref` (a scene scan)
   on every `project.world...` access. Both want a discriminant match and a
   cached active scene.
5. **`publish_sensors` remainder** (about 0.6-1.3 us per actor per frame):
   `filter_of`, `casts_shadows` and `collider_shape` recomputed for every
   actor every frame although they only change on a tick or an effect.
6. **Per-frame systems to profile next** (not yet read closely):
   `apply_common` (per-effect lookups), `batch_meshes` (small movers merge
   every frame), `cull_views` (CPU Hi-Z raster), `tiles::redraw_maps`,
   `ai::tick`, `sync_lights`.

## Interface designer

`UiDocument::validate` walks each parent chain once (an id to parent map and a
set of ids already known to reach a root), and `UiDocument::with_edit` clones
the document once per edit. A drag no longer clones the project: a transaction
keeps only the interface document it started from, and staleness is judged on
that document and the saved revision.

Measured on a debug build, one `Move` on a chain-heavy document (chains eight
deep), `cargo test -p blockloom-core scale_tests::timings -- --ignored
--nocapture`:

| Widgets | `validate` | `with_edit` (clone, edit, validate) |
| --- | --- | --- |
| 1,000 | 2.0 ms | 2.7 ms |
| 10,000 | 19.9 ms | 20.8 ms |

Release builds are several times faster. A QML-side drag still receives the
whole document back per pointer move; sending a patch instead is open.
