//! The game world as its own process: the editor's child, or a built game.
//!
//! Usage: `blockloom-runtime [--mode 2d|3d] [--play <build or game folder>]`.

fn main() {
    blockloom_runtime::run_process();
}
