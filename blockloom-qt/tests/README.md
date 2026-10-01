# Game view smoke test

Build the real editor with the development QML entry override, then run both
the shared-image path and the readback fallback on a Windows GPU:

```sh
cargo build --release --workspace --features blockloom/qml-preview
python scripts/test-game-view.py
python scripts/test-game-view.py --backend opengl
```

Each run isolates its projects and library under a fresh `target/game-view-*`
folder. It renders a red 2D world, resizes to a non-aligned resolution, destroys
the view, opens a 3D world, resizes again and exits. Logs and four PNG captures
stay in that folder for inspection. The Vulkan run requires four successful
zero-copy ring imports; the OpenGL run requires readback.

The normal release build ignores `BLOCKLOOM_QML_ENTRY`. Run `just build` after
testing to restore the editor without the QML debugging feature.
