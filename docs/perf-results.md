# Measured performance

`just perf` runs, newest first. Frame and GPU columns are milliseconds,
release build, 1920x1440 window, vsync off. Raw summaries are in
`tests/perf/<label>/` when a run is checked in.

## 2026-11-27, master d94e38d, sirstrokes (round 2)

Same machine. GPU occlusion now waits for 2000 meshes, and the capture adds
main-world CPU. Rounded from the run's summaries.

| GPU | quality | 3D fps | 3D GPU ms | 3D main CPU ms | 2D fps |
|---|---|---|---|---|---|
| RTX 5080 Laptop | default (High) | ~270 | 0.67 | - | ~500 |
| Intel Graphics (ARL) | default (High) | 89-96 | 7.1-9.1 | 5.6-6.2 | 190-230 |
| Intel Graphics (ARL) | Medium, Low | about the same | about the same | - | - |

### What it showed

- Dropping occlusion halved iGPU 3D frame times (16 ms to 7-9 ms of GPU).
- The round-1 third-person result (78 fps on the RTX) was noise: it ran at
  ~270 like the other 3D templates.
- Medium and Low barely moved 3D GPU time, since `main_opaque_pass_3d` stayed
  at 2.7-3.3 ms at every tier: presets didn't touch MSAA. They now cap it
  (Low off, Medium 2x, High and Ultra 4x).
- The largest main-world span in 3D is PostUpdate at 1.6-2.4 ms.
- Whichever window was moved between screens held exactly 60 fps. Both
  displays hang off the NVIDIA card, so an iGPU frame crosses GPUs, and
  this looks like the compositor's path rather than Blockloom's. It also
  explains round 1's 60 fps rows.
- Tonemapping (~0.45 ms) and upscaling (~0.4 ms) still lead 2D on the iGPU.

### Budgets

`scripts/perf.py` checks each entry against its GPU class (`--check` makes
it exit 1): integrated mean 16.7 ms and p95 25 ms (60 fps), discrete mean
6.9 ms and p95 16.7 ms (144 fps). Round 2 holds both on this machine.

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
