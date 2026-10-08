# Rust script reads

Scripts read the same world snapshot as blocks. Writes queue effects for the
runtime to apply after callbacks finish, so a read immediately after `go_to`
still returns the previous position.

## Reading a pose

```rust
use blockloom::*;

fn tick(me: &Actor, _dt: f32) {
    let pose = me.pose();
    let [x, y, z] = pose.position;
    let [rx, ry, rz] = pose.rotation;
    let scale = pose.scale;

    if let Some(target) = me.pose_of("Ball") {
        me.go_to(target.position[0], y, z);
    }
}

blockloom::export!(tick = tick);
```

`Pose` contains world position, Euler rotation in degrees and uniform scale.
Position uses pixels in 2D and metres in 3D. `pose_of` accepts an actor id or a
case-insensitive name and returns `None` for a missing actor. An empty name
refers to this actor. `pose()` falls
back to zero position/rotation and scale 1 if its actor is missing.

## Cost in a tick

Each scalar read (`x`, `rotation`, `scale`, `position_of`) crosses the host
boundary and borrows the snapshot separately. Reading XYZ, all three rotations
and scale takes seven calls. `pose()` gets those seven values in one call;
keep the returned value in a local variable when using several fields.

Native calls use a function pointer; web calls cross a WebAssembly import and
copy the answer into the script's memory. No fixed nanosecond cost is promised:
it depends on the platform and host. Batch reads reduce the call count, without
changing when writes become visible.

The pose travels as a fixed 28-byte buffer, with seven little-endian `f32`s.
Self pose reads and scalar position/rotation/scale reads allocate no strings
and do not clone the actor's components. `pose_of` borrows its argument; actor
ids take a direct lookup, while names use the existing name resolver. The ABI
keeps its three callbacks on desktop, Android and web.

## Reading motion

```rust
use blockloom::*;

fn tick(me: &Actor, _dt: f32) {
    let vx = me.velocity(Axis::X);
    let spin = me.angular_velocity(Axis::Z);
    let weight = me.mass();

    if me.grounded() && vx > 0.0 {
        // Running along the ground.
    }
    if me.grounded_of("Ball") {
        // The ball has landed.
    }
}
```

`velocity` is linear motion in world units a second (pixels in 2D, metres in
3D); `angular_velocity` is spin in radians a second about an axis (a 2D body
turns about z only). `mass` is the body's kilograms, from its shapes. All
three read zero with no body. `grounded` reports a solid contact holding the
actor up, with no controller move required; `is_grounded` instead reports the
last controller move (slope-limited, unknown before the first move). The
`_of` variants read another actor by name or id, with an empty name meaning
this actor; a missing actor reads zero or false. The matching block reporters
are `velocity`, `angular velocity`, `mass of` and `is grounded?`, so blocks
and scripts agree on the same fixed tick.

## Keeping per-actor state

```rust
use blockloom::*;

fn tick(me: &Actor, dt: f32) {
    let cooldown = me.data("cooldown") - dt as f64;
    if cooldown <= 0.0 {
        me.set_data("cooldown", 1.0);
        me.set_data_text("mood", "hot");
    } else {
        me.set_data("cooldown", cooldown);
    }

    if me.has_data("mood") {
        // ... read me.data_text("mood") ...
    }
    me.clear_data("mood"); // me.clear_all_data() forgets every key.
}
```

One script file is shared by every actor running it, clones included, so a
`static` would leak state between them. The host keeps one map per running
actor id instead: each clone gets its own keys, and nothing is saved. A run
starts empty; scene switches inside the run keep what is stored. Writes land
at once, so a read straight after a write sees it (unlike a world write,
which waits for the step's end). An unset number reads 0.0 and unset text
reads empty; `has_data` tells a stored zero apart from nothing stored. Keys
are trimmed and an empty key stores nothing. ABI 43 requires older script
libraries and packaged players to be rebuilt together.

## Sharing block variables and lists

```rust
use blockloom::*;

fn tick(me: &Actor, _dt: f32) {
    me.set_variable("score", me.variable("score") + 1.0);
    me.change_variable("lives", -1.0);
    me.set_variable_text("mode", "hot");

    me.list_add("bag", 3.0);
    me.list_add_text("bag", "four");
    let count = me.list_len("bag");
    let first = me.list_number("bag", 1);
    let second = me.list_text("bag", 2);
    me.list_replace("bag", 1, first + 1.0);
    me.list_delete("bag", count);
    me.list_clear("bag");
    me.save_variable("score");
}
```

Scripts read and write the same working memory the canvas uses: this actor's
own variables over the shared ones, and the same for lists. Writes land at
once through the same stores the VM writes, so a canvas counter and a script
counter stay one counter, and a read straight after sees the write.
`save_variable` persists them the way it does for a block. An unknown
variable reads 0.0 as a number and `"0"` as text; an unknown list reads empty
and writes nothing, exactly as the list blocks treat one. List indexes are
1-based, like the blocks: inserts allow one past the end, and out-of-range
reads and writes miss quietly. ABI 45.

## Save slots and interface language

```rust
use blockloom::*;

fn tick(me: &Actor, _dt: f32) {
    me.switch_save_slot("Slot 2");
    me.delete_save_slot("Slot 1");
    me.say(&me.save_slot());
    me.say(&me.save_slots());

    me.set_language("fr");
    me.say(&me.language());
    me.say(&me.text_for("greeting"));
}
```

Slot and language blocks have script spellings that produce the same
effects: switching loads that slot's saved variables into the run (a name
with no file yet starts fresh), deleting is quiet for a missing file, and
`save slot` reads `"default"` until the first switch. `save slots` answers
every slot with a file as a JSON list, so `load json into list` takes it.
`text_for` answers the key in the run's language, falling back to the
default language and then to the key itself, exactly like the blocks. ABI 47.

## Driving another actor

```rust
use blockloom::*;

fn tick(me: &Actor, _dt: f32) {
    let ball = me.world().actor("Ball");
    if let Some(pose) = ball.pose() {
        ball.go_to(pose.position[0] + 10.0, pose.position[1], pose.position[2]);
    }
    ball.set_color("red");
    ball.push(1.0, 0.0, 0.0);
}

blockloom::export!(tick = tick);
```

`me.world().actor("Ball")` names another actor by id or name (empty for this
actor, `me`/`myself` also mean this one). Reads use the snapshot, so they
miss as `None`, zero or false when nothing answers to the name. Writes become
the same effects a self-write would, with that actor's id: transform
(`go_to`, `change_position`, `move_forward`, `turn`, `set_rotation`,
`set_scale`, `point_towards`), appearance (`say`, `set_visible`,
`set_color`) and velocity (`push`, `set_velocity`, `add_force`,
`add_torque`). A missing target is an error for the running actor, and a read
straight after one of these still sees the old value, exactly as for self.
`rotation` and `scale` read through `pose()`; `exists()` probes presence and
`target()` returns the name as spelled. ABI 44.

## Sharing code and stacking scripts

```rust
// assets/scripts/shared/health.rs: plain Rust, no `export!`.
pub fn clamp(hp: f64) -> f64 {
    hp.clamp(0.0, 100.0)
}
```

```rust
// assets/scripts/player.rs: one of several scripts on one actor.
use blockloom::*;

#[path = "shared/health.rs"]
mod health;

fn tick(me: &Actor, _dt: f32) {
    me.set_variable("hp", health::clamp(me.variable("hp")));
}

blockloom::export!(tick = tick);
```

Any file under `assets/scripts/shared/` is library code, never a runnable
script: it gets no build of its own, no script component, and no IDE target.
A script pulls it in with `mod` (usually `#[path]` as above), and rustc
reads it as part of that script's own build on desktop, web and Android
alike. Any edit, add or delete under `shared/` rebuilds every script, since
each one may include it. The IDE still checks it as part of the scripts that
include it.

An actor may also run several scripts: Add component again while it already
has one. They run in list order, top to bottom: every `start`, then every
`event`, then every `tick` in the same order. They share the actor's script
storage, block variables and lists, so two scripts incrementing one counter
stay one counter; a later script reads what an earlier one wrote the same
tick. The inspector's Up/Down moves a script among its own; detaching one
leaves the rest running.

## Timing: fixed, frame and UI ticks

```rust
use blockloom::*;

fn tick(me: &Actor, dt: f32) {
    // Fixed step: physics. Same `dt` every step.
    me.push(0.0, 0.0, 0.0);
}

fn frame(me: &Actor, dt: f32) {
    // Once per rendered frame while unpaused: camera and UI motion on the
    // variable frame delta.
    let (_, dy) = me.mouse_delta();
    me.set_camera_pitch(dy * dt * 10.0);
}

fn ui(me: &Actor, _dt: f32) {
    // Once per rendered frame even while paused: menus. Paused or not, this
    // is where a settings screen lives, the way UI strands do for blocks.
    if me.action_pressed("confirm") {
        me.hide_ui("menu");
    }
}

fn stop(me: &Actor) {
    // Once when the run ends: flush a save, log a line. Movement is moot;
    // the world is going away.
    me.save_variable("score");
}

fn destroy(me: &Actor) {
    // Once when this actor is deleted mid-run (or its script detached).
    me.log("going away");
}

blockloom::export!(tick = tick, frame = frame, ui = ui, stop = stop, destroy = destroy);
```

`tick` is the fixed-step physics clock; `frame` is the per-render-frame
presentation clock. Keep forces and contacts in `tick`, camera and interface
motion in `frame`. `ui` runs in both modes so script menus answer while
paused, matching block UI strands; `frame` and fixed-step `event` delivery
pause with the world. `stop` runs on run end (speech, errors and saves land;
movement does not) and `destroy` when the actor goes away. All four are
optional: an old three-entry `export!` still builds, and missing entries are
no-ops. `Event::SceneEnded` still fires for scene-end cleanup between runs.

## Waiting without blocking

```rust
use blockloom::*;

fn tick(me: &Actor, dt: f32) {
    // A wait keyed in this actor's storage: true on the tick 2 seconds
    // elapse, then it starts over. Every clone keeps its own countdown.
    if me.after("hello", 2.0, dt) {
        me.say("two seconds later");
    }
    // Repeating work: how many whole beats elapsed (catching up after a
    // hitch), and a cooldown gating a trigger.
    for _ in 0..me.every("beat", 1.0, dt) {
        me.change_variable("ticks", 1.0);
    }
    me.cooldown_tick("jump", dt);
    if me.action_pressed("jump") && me.cooldown_ready("jump") {
        me.cooldown_trigger("jump", 0.5);
        me.push(0.0, 300.0, 0.0);
    }
}
```

There is no `wait` that blocks: a strand yields at waits, but a script
function must return every step. Feed the step's `dt` to a keyed helper and
act when it fires. `after` answers true on the tick the wait ends, then
starts over (guard with a flag or `clear_data` for fire-once); `every`
answers how many beats elapsed; `cooldown_ready`/`cooldown_trigger`/
`cooldown_tick` gate a trigger so holding the button does not restart the
clock. Keys live in per-actor script storage, so two actors (or clones)
running this file do not share countdowns.

The same helpers exist as plain structs - `Timer`, `Every`, `Cooldown` -
for a file only one actor runs, held in a `static` and fed `dt` by hand.
Prefer the keyed methods whenever the file could run twice.

Tween and animation completion use the same poll-or-event split as blocks.
`is_tweening()` polls a glide or `tween ...` in flight; a `Once` clip's end
arrives as `Event::AnimationEnded` and its markers as
`Event::AnimationMarker`, both in the fixed-step `event` entry point. A
`wait`-style sequence over either is a small state machine: start the tween
or clip, then poll or match the event, then move to the next step on the
following tick.

## Typed dials

String dials still build, but prefer the checked enums: a typo fails compile
instead of silently doing nothing at runtime. Each maps to the same word the
host parses, so old and new calls land on the same effect.

```rust
use blockloom::*;

fn tick(me: &Actor, dt: f32) {
    me.add_force_mode(ForceMode::Impulse, 1.0, 0.0, 0.0);
    me.set_wind_dial(WindDial::Speed, 3.0);
    me.set_water_dial(WaterDial::Chop, 0.5);
    me.set_clouds_dial(CloudDial::Coverage, 0.5);
    me.set_precipitation_kind(Precipitation::Snow, 0.5);
    me.set_color_rgb(Color::RED);
    me.tween_scale_eased(2.0, 1.0, Easing::EaseOut);
}
```

| String | Checked |
| --- | --- |
| `add_force("Impulse", ..)` / `add_torque(..)` | `add_force_mode(ForceMode::Impulse, ..)` / `add_torque_mode(..)` |
| `set_wind("speed", ..)` | `set_wind_dial(WindDial::Speed, ..)` (`Direction`, `Gust`, `Storm`) |
| `set_water("chop", ..)` | `set_water_dial(WaterDial::Chop, ..)` (`Level`, `Foam`) |
| `set_clouds("coverage", ..)` | `set_clouds_dial(CloudDial::Coverage, ..)` (`Density`, `Type`) |
| `set_precipitation("snow", ..)` | `set_precipitation_kind(Precipitation::Snow, ..)` (`Rain`) |
| `tween_scale(f, s, "EaseOut")` and siblings | `tween_scale_eased(f, s, Easing::EaseOut)`, `tween_rotation_eased(..)`, `tween_color_eased(..)` |
| `set_color("#FF0000")` | `set_color_rgb(Color::RED)` - or `Color::new(r, g, b)`, `Color::parse("#ff0000")` |

The cross-actor handle (`me.world().actor(..)`) has the same checked
spellings where it applies: `add_force_mode`, `add_torque_mode`,
`set_color_rgb`.

## Project symbols

`blockloom::symbols` holds this project's names as constants, generated at
build time: actor names and ids, input actions, custom component fields and
shared variables/lists. A rename rebuilds every script, and a stale name
fails compile instead of silently reading zero.

```rust
use blockloom::*;

fn tick(me: &Actor, _dt: f32) {
    // Instead of me.variable("score") and me.action_pressed("jump"):
    if me.action_pressed(symbols::actions::JUMP) {
        me.change_variable(symbols::variables::SCORE, 1.0);
    }
    if let Some(pose) = me.pose_of(symbols::actors::BALL) {
        me.go_to(pose.position[0], pose.position[1], pose.position[2]);
    }
    let hp = me.field(
        symbols::components::health::NAME,
        symbols::components::health::HP,
    );
}
```

Sections: `symbols::actors` (one constant per actor name, e.g. `BALL`,
`PLAYER_1`), `symbols::actor_ids`, `symbols::actions`, `symbols::components`
(one module per custom component, e.g. `symbols::components::health::HP`),
`symbols::variables` and `symbols::lists`. The module is always there with
no import or setup; a project with nothing to name gets an empty one, so
scripts still build. See the [Rosetta](script-rosetta.md) for the full
block-to-script map.

## Editor help

The script dialog checks what is on screen, not just what was saved: every
save re-runs diagnostics, and two quiet seconds do too, so inline errors
follow typing. `Check` compiles through `rustc`; `Lint` runs the opt-in
`cargo clippy` pass (needs `rustup component add clippy`) and its rows land
where check's do; `Format` runs Save+`rustfmt` over the file (needs
`rustup component add rustfmt`) and shows what it did. All three read the
same analysis project `sync_script_ide` writes, so an external editor with
rust-analyzer sees the same API and `symbols::` names.

`Ctrl+Space` completes from the same two sources: every script API method,
type and typed-enum variant with its doc line, plus this project's
`symbols::...` paths. `Tab`/`Enter` takes the picked row; the line under the
list is that row's doc (`script_hover`, also over `write_script` agents'
`script-completions`/`script-hover` commands). Symbols complete from what the
last build generated, so a rename shows up after the next check.

The run log is script-aware: every line carries a level (`info` for a `say`,
`error` for a failure) and, when one names it, the script file plus the
first `path:line` span. A line with a file offers `Open script`, which opens
the actor running it on that line. The render profiler lists one `script/…`
row per script file with its smoothed milliseconds per tick, so a heavy
file stands out from a busy project.

## Sandboxed scripts

Play builds each script twice when this machine has the wasm target's std
(`rustup target add wasm32-unknown-unknown`): the native library the run
loads, plus the same wasm module a web build ships. The run prefers native
for speed and falls back to the sandbox when the library is missing or won't
load, so a wasm-only project still plays. `BLOCKLOOM_SCRIPT_BACKEND=wasm`
prefers the sandbox instead, which is how to check a script behaves the same
trapped behind fuel and a 64 MiB ceiling before trusting anyone else's.

Rust guests bind the frozen world through `blockloom-script-guest`
(`wit/world.wit` is `blockloom:script@0.1.0`, byte for byte): typed
`lifecycle`, `state`, `vars`, `lists`, `physics` and `plugins` modules over
the same three imports, with `export_script!` naming the entry points.
`templates/minimal.rs` is the whole contract in one file;
`cargo build --target wasm32-unknown-unknown` makes the module the sandbox
loads.
