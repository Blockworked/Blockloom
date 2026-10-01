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
