# Mini survival

The voxel plugin's reference sample: a `.blockgen` world of meadows, dunes
and sea, a block set with falling sand, spreading grass, growing wheat,
flowing water, a chest and a torch, and a first-person player who mines
(left click), builds planks (right click) and puts down chests (E) while
the sky turns from day to night and back.

```bash
just voxel-plugin                      # seal the plugin into target/plugins
python3 plugins/voxel/samples/mini-survival/setup.py ~/Blockloom/projects
```

Then open the project, build its runtime in the Plugin Manager's Engine
plugins section (or pass `--build-runtime`) and press Play.

- `world.json` is the voxel `world` resource the script installs.
- `assets/blocks/` holds the block set, `assets/worlds/survival.blockgen`
  the world (see `docs/voxel-worldgen.md`).
- `cargo test -p blockloom-voxel --test sample` plays it headless: mining,
  building, a chest's items, water flow, wheat growth and nightfall. In a
  release build it holds a 16 ms budget for a block tick and for each
  frame of a relight, and 2 s to bring the world up.

The voxel plugin is engine-tier, so the sample plays on desktop only, not
in the web player.
