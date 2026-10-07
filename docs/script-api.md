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
