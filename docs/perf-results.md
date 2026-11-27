# Measured performance

`just perf` runs, newest first. Frame and GPU columns are milliseconds,
release build, 1920x1440 window, vsync off. Raw summaries are in
`tests/perf/<label>/` when a run is checked in.

## 2026-11-27, master f9bc965, sirstrokes

Intel Core Ultra 9 275HX, 30 GiB, KDE Wayland, Mesa 27.0.0-devel /
NVIDIA 620.73.04, Vulkan. Quality preset: the template default (High).

### NVIDIA GeForce RTX 5080 Laptop GPU

| entry | fps | frame mean | p95 | p99 | GPU mean | GPU p95 | fixed mean |
|---|---|---|---|---|---|---|---|
| playground-3d | 227 | 4.40 | 8.67 | 11.21 | 1.66 | 3.49 | 0.41 |
| playground-2d | 515 | 1.94 | 3.64 | 6.16 | 0.19 | 0.21 | 0.12 |
| platformer | 471 | 2.12 | 4.26 | 6.47 | 0.20 | 0.22 | 0.14 |
| top-down | 530 | 1.89 | 3.43 | 5.45 | 0.21 | 0.23 | 0.11 |
| first-person | 244 | 4.09 | 7.97 | 10.18 | 1.63 | 3.35 | 0.36 |
| ui-puzzle | 521 | 1.92 | 3.47 | 5.41 | 0.22 | 0.24 | 0.11 |
| third-person | 78 | 12.79 | 20.92 | 23.19 | 1.75 | 3.38 | 1.49 |

### Intel Graphics (ARL), integrated

| entry | fps | frame mean | p95 | p99 | GPU mean | GPU p95 | fixed mean |
|---|---|---|---|---|---|---|---|
| playground-3d | 54 | 18.56 | 26.25 | 35.20 | 16.40 | 16.97 | 2.39 |
| playground-2d | 86 | 11.62 | 22.62 | 28.41 | 2.49 | 3.45 | 1.44 |
| platformer | 171 | 5.84 | 17.71 | 20.35 | 2.04 | 3.65 | 0.52 |
| top-down | 60 | 16.68 | 21.13 | 24.12 | 4.41 | 4.89 | 1.83 |
| first-person | 56 | 17.91 | 24.25 | 29.34 | 15.81 | 16.02 | 2.04 |
| ui-puzzle | 60 | 16.67 | 20.13 | 22.61 | 3.61 | 4.35 | 1.52 |
| third-person | 52 | 19.22 | 25.18 | 31.78 | 17.10 | 17.50 | 2.24 |

### What it showed

- With vsync on (the first pass) every template sat at the 160 Hz display's
  6.27 ms, so `profile.rs` now turns vsync off while it measures.
- The 3D templates are GPU-bound on the iGPU at about 16 ms. Bevy's GPU
  occlusion depth pyramid (`mip_generation::early_downsample_depth`) was 3.4 ms
  of it against 0.15 ms on the RTX, for scenes of a few dozen meshes. GPU
  occlusion now waits for 2000 meshes. The opaque pass is 2.9-3.5 ms. The timed
  passes sum to about 7 ms, so about 9 ms goes to passes without timestamps.
- Third-person is CPU-bound on the RTX (12.8 ms frames against 1.75 ms of GPU)
  but was under 6.27 ms with vsync on. The capture now records main-world CPU
  and its `Update` spans to find out where.
- top-down and ui-puzzle held exactly 60 fps on the iGPU with 3.6-4.4 ms of
  GPU and vsync off, while platformer ran at 171. Not explained yet.
- 2D on the iGPU spends 0.5-1.2 ms in tonemapping and 0.4-0.6 ms in upscaling,
  more than drawing the sprites.
