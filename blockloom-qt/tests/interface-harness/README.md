# Interface designer harness

Runs the real editor (backend, runtime, Game view) and drives the Interface tab
with Qt's own synthetic events from inside the process, so nothing touches the
desktop's pointer or keyboard. It checks that a click picks what the runtime
drew, a drag moves it and commits once, a resize handle resizes, Escape cancels,
an arrow key nudges, a keyed animation scrubs through the runtime, and a rename
lands.

```sh
cargo build -p blockloom --features qml-preview
BLOCKLOOM_DATA_DIR=<dir> \
BLOCKLOOM_QML_ENTRY=file://$PWD/blockloom-qt/tests/interface-harness/Harness.qml \
  target/debug/blockloom --project <project folder> 2>&1 | grep HARNESS
```

It prints `HARNESS DONE` when every step held and `HARNESS ERROR ...` on the
first failure. The project is edited, so point it at a scratch copy. It needs
the embedded Game view (a GPU and a display); `BLOCKLOOM_RUNTIME=process` with
the offscreen platform has no frames to read. Run `cargo build -p blockloom`
afterwards to put the normal editor back.
