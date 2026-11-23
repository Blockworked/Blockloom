# GPU test triage on lavapipe

The ignored `embed` tests read pixels back from a real world. CI can only run
them on lavapipe (Mesa's software Vulkan), which has ray queries but no
dma-buf export or DRM format modifiers. This is where each one stands there,
so a failure on lavapipe can be told apart from a regression. GPU runs on real
hardware happen only when asked for.

```bash
BLOCKLOOM_TEST_OPAQUE_FD=1 BLOCKLOOM_TEST_DEADLINE=300 \
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
cargo test -p blockloom-runtime --lib -- --ignored embed --test-threads 1
```

Without `BLOCKLOOM_TEST_OPAQUE_FD=1` every ring allocation fails on lavapipe
("A device memory allocation has failed"), so that flag is required.

Last run 2026-11-23 (master 34e2737 plus the fixes below): 56 of 67 pass.

## Fixed by this triage

| Test | What it was |
|---|---|
| `quality_scaling_keeps_ui_edges_at_native_resolution` (part) | A real crash. In 2D with `resolution_scale` under 1 and a visible interface, the native UI overlay was a second `Camera2d` on the world's target. Bevy keeps one 2D depth texture per target, so the overlay drew with the world's 4x depth into a 1x color target, a validation error that quit the game. The overlay is now always the other core pipeline (`Camera3d` over a 2D world, `Camera2d` over a 3D one). |
| `decals_project_reject_and_fade_on_instanced_surfaces` | Lavapipe needed more than the 20 s frame deadline. Passes with `BLOCKLOOM_TEST_DEADLINE=300`. |

## Lavapipe can't do it (expected to fail there)

| Test | Why |
|---|---|
| `an_embedded_world_draws_straight_into_a_tiled_ring` | Needs DRM format modifiers; with opaque fds the ring stays linear. |
| `an_embedded_world_publishes_read_back_frames` | Tests the shared-memory fallback, which only engages where GPU sharing fails. With opaque fds sharing works, so no read-back frame is ever published. |
| `a_path_traced_exr_capture_waits_for_its_samples` | The path tracer does not reach its sample budget on a CPU before the test gives up, so no EXR is written. Likely passes on hardware. |

## Open: need a real GPU run to tell

| Test | What lavapipe shows |
|---|---|
| `quality_scaling_keeps_ui_edges_at_native_resolution` (rest) | 2D only, after the crash fix: the HUD draws sharp, but 64 px to the right of where it should (stripe at x 95 instead of 31). 3D places it right. Probably a real 2D layout bug. |
| `a_froxel_beam_glows_between_the_lamp_and_the_floor`, `a_shaft_cone_stands_in_for_the_froxels`, `motes_glitter_in_a_beam_and_dust_in_the_air` | The floor reads `[1, 1, 0]` with and without the beam: nothing from the spot light reaches the frame at all. No errors logged. |
| `the_heat_map_tints_what_a_volume_covers_in_2d` | Reads `[1, 1, 0]`: the heat pass draws nothing in 2D. |
| `gpu_particles_land_on_an_actor_the_camera_cant_see` | The shelf half passes; with no shelf no spark is found at all (row 0). |
| `a_shadowed_rect_light_is_blocked_by_what_hangs_under_it` | The floor under the shade stays lit (245): the shadow twin's cube shadow isn't applied. |
| `cloud_tile_wrap_leaves_no_vertical_seam` | Flaky: failed once (step 83 at column 63), passed on rerun. |
