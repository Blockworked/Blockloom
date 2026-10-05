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
keeps its three callbacks on desktop, Android and web. ABI 41 requires older
script libraries and packaged players to be rebuilt together.
