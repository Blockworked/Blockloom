# Cinematics

A cutscene is a named camera reel that plays on the wall clock: cuts, dolly
moves, signal markers, slow motion and volume grades. It keeps playing while
`pause game` freezes world strands, the way interface strands do.

Reels live on the scene (`World.cutscenes`, up to 32) and are authored as
JSON through the `set-scene-component` shell/MCP command with
`{"component": "Cutscenes", "cutscenes": [...]}`. There is no visual
timeline editor. A reel looks like this:

```json
{
  "name": "Opener",
  "duration": 10.0,
  "shots": [
    {
      "camera": "CamA",
      "fov": 55.0,
      "dur": 3.0,
      "path": [
        {"at": 0.0, "pos": [0, 2, 8], "look": [0, 1, 0], "roll": 0.0},
        {"at": 3.0, "pos": [4, 2, 6], "look": [0, 1, 0], "roll": 0.0}
      ]
    },
    {"camera": "CamB", "dur": 7.0, "path": []}
  ],
  "signals": [{"at": 3.0, "name": "beat"}],
  "slowmo": [{"at": 3.0, "scale": 0.4}],
  "volumes": [{"at": 0.0, "volume": "Cave", "weight": 0.8}]
}
```

Shots play back to back. A shot frames its camera actor's pose, or travels
its dolly path when one is given (positions are absolute; an empty path is a
hard cut). `look` is the world point the camera faces, `roll` the Dutch
angle in degrees, `fov` optional per key. A shot naming no actor holds the
last frame and reports itself once in the run log. 2D shots frame position
only, never FOV.

## Blocks and scripts

- `play cutscene` starts a reel by name; the strand carries on. Playing a
  second reel cuts the first without firing its end strands.
- `skip cutscene` jumps to the end marker: remaining signals fire in order,
  then `when cutscene ends` runs. Quiet with nothing playing.
- `when cutscene signal` runs when the reel passes the named marker (empty
  matches any); `when cutscene ends` runs when the reel gets there.
- `shake camera by` kicks trauma 0-1 higher on top of what is shaking; the
  view settles as it decays. Independent of any reel.
- `set time scale to` slows the world for the rest of the run (1 is normal,
  0 freezes world strands). The reel's slow-motion keys only move it while
  no block has. `hitstop` freezes world strands for a number of render
  frames; interface strands and the reel clock tick on through both.
- `set letterbox to` shows the bars for nonzero, hides them for zero;
  `fade screen to` fades to `black` or `white`, or clears for `none`.
- `is cutscene playing?` and `cutscene time` (seconds in) read the reel.

Scripts get `play_cutscene`, `skip_cutscene`, `shake_camera`,
`set_time_scale`, `hitstop`, `set_letterbox`, `fade_screen`,
`cutscene_name`, `cutscene_time`, and hear both events through `event`.

Volume keys borrow weights: whatever they touch is restored when the reel
ends, ends early, or is cut. Letterbox, fade and trauma are independent
presentation state and stay as left. A reel never moves the sun or weather
tracks; drive those from the time-of-day director beside it.
