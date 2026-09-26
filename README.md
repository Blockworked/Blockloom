# Blockloom

Build games out of blocks, in 2D or in 3D.

Blockloom is a desktop game engine with a Scratch-style block editor. A project
is a world and a cast of actors; each actor has its own canvas of blocks, and
pressing Play opens the world in a real renderer with real physics and runs
them.

- **One vocabulary, two dimensions.** The same `go to`, `push` and `turn` blocks
  drive a sprite in a 2D world or a mesh in a 3D one. A project says which it
  is, and a `z` coordinate is simply ignored in 2D.
- **Real physics.** Every actor can be static, dynamic or kinematic, with
  gravity, bounce and friction, via Rapier.
- **Blocks that sense the world.** `key down?`, `touching?`, `distance to`,
  `my x position`, `timer`, the mouse - all readable inside any value slot.
- **Your own blocks.** Custom commands and reporters with named inputs, as in
  Scratch's "My Blocks".

## Building

```bash
just build     # everything: the editor and the game runtime
just run
```

Requires Rust (pinned in `rust-toolchain.toml`, which rustup picks up) and Qt
6.10 or newer (Quick, Quick Controls 2, Quick Dialogs 2, Multimedia).
`blockstitch` comes in as a git dependency, so no sibling checkout is needed.
Build the whole workspace: the editor launches the `blockloom-runtime` binary
from beside itself.

## Layout

| Crate | What it is |
| --- | --- |
| `blockloom-qt` (`blockloom`) | the editor window, in Qt Quick |
| `blockloom-app` | every command the editor issues, and the state it edits |
| `blockloom-runtime` | the game world: a Bevy app running the blocks |
| `blockloom-core` | the shared engine: scene, blocks, VM, project files |
| `blockloom-protocol` | the editor-to-runtime wire format |

The block editor itself - canvas, dragging, snapping, the value expression
system - is [blockstitch](https://github.com/Blockworked/blockstitch), shared
with [Blockwork](https://github.com/Blockworked/Blockwork).

See [CLAUDE.md](CLAUDE.md) for the architecture in more detail.
