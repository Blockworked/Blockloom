# Editor baseline

`trace.json` is a run of canvas edits (nested bodies, a custom reporter,
comments, collections, numeric spellings, value drags, undo/redo, a rejected
edit). `editor_baseline.rs` replays it and holds each step to `expected.json`.
`project/` is the project that run saved; it must load and save back without losing anything.

A deliberate behavior change re-records it:

```sh
BLOCKLOOM_BLESS=1 cargo test -p blockloom-app --test editor_baseline
```
