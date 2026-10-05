# Native LAN spectator slice

Status: implemented internal slice, 2026-10-05. This attaches a native QUIC
endpoint to the existing run. Guests receive read-only actor replicas; player
ownership, remote input, per-player UI and prediction remain Phase 2 work.

## Use

Build the workspace with `just build`. Desktop runtime builds enable the
`multiplayer` feature by default; web and Android do not link the native transport.

In **Project settings → Multiplayer**, enable **LAN spectators** and choose
a guest limit (1 to 16, default 8). Older projects stay disabled. These settings
are saved with the project and included in exports and built packs. Changes apply
on the next run, including changes made while hosting.

Press Play, then **LAN spectators** in the editor toolbar. Enter the numeric IP
of the intended LAN interface and a port, for example `192.168.1.5:7777`, and
choose Open to LAN. A port of zero chooses an available port. Wildcard and
multicast addresses are refused. The session dialog shows the private invite,
connected guest ids, errors and disconnect/close controls. Opening and closing
LAN neither presses the green flag nor recreates actors. Loading edited content
while LAN is open is refused; close LAN before applying it to the runtime.

The same controls are available through the attached shell and MCP registry:

```text
set-multiplayer settings={"enabled":true,"max_guests":4}
# Start a new run after changing the capability.
open-lan bind="192.168.1.5:7777" max_guests=4
session-status
kick-guest id=1
close-lan
```

Opening is asynchronous: the next settled simulation tick captures the baseline
and binds the socket. Read `session-status` for the result. Invites contain a
certificate fingerprint, a 256-bit admission secret and a content compatibility
hash; share them privately. Nothing is broadcast, and no firewall or router
configuration is changed.

On another desktop, launch the primitive spectator view:

```bash
target/release/blockloom-runtime --join "$LAN_INVITE" --trusted-build "$CONTENT_HASH"
```

Supply the content hash through a trusted channel. It is the invite's final
hex field. This viewer does not execute game code or load remotely supplied
assets. It draws rectangles/circles in 2D and cuboids/spheres/capsules/planes in
3D using a fixed spectator camera. Asset-backed looks are explicitly refused.
It is a diagnostic spectator view, without the host's camera, UI, lighting,
material graphs, animation or other presentation effects.

The text inspector accepts all look types and reports replicated actor poses:

```bash
target/release/blockloom-lan "$LAN_INVITE" "$CONTENT_HASH"
```

Stop/Start/Load and runtime shutdown close the listener. Close LAN disconnects
guests while the host continues; the endpoint drains close packets for up to a
second. Reopening makes a new identity and admission token.

## Replication contract

The schema version is independent of the editor protocol. A guest first passes
pinned TLS, then schema/content-hash and constant-time admission-token checks.
Only acknowledgements and heartbeats are accepted from guests. They cannot
submit game input or administrative commands.

At a settled fixed-step boundary the host captures scene epoch, logical step,
game time in nanoseconds, pause state, dimension and live actors. Actors carry
stable ids, clone template ids, settled translation/rotation/scale, linear and
angular velocity, visibility, authored name/look and live parent id. Appearance
metadata is inert JSON inside a bounded binary frame. Script graphs, private
variables, save data, plugin state and executable libraries are never sent.
Runtime visual-effect changes, component mutations, terrain/tile changes and
animation are not covered by this first registry.

A reliable stream sends one baseline/update at a time. The next delta compares
to the state the guest acknowledged applying, with explicit creation/update and
removal records. Updates coalesce while a guest is behind; there is no unbounded
lifecycle tail. A scene epoch change forces a full baseline. Clients validate and
apply whole frames atomically, then acknowledge. The native transport repairs
loss/reordering. Datagram prediction and interest management are later work.

Limits: 16 guests, 1024 actors, 1 MiB per complete frame, 256-byte identifiers,
32 KiB per actor's appearance, finite pose/velocity fields. The transport also
bounds queued reliable bytes. Admission or acknowledgement stalls time out;
heartbeats cannot prolong an unacknowledged state transfer. Exceeding replication
limits closes LAN and reports the error, leaving the authoritative game running.
These are implementation bounds, not measured capacity claims.

The content hash covers the serialized authored project and a spectator-schema
engine identifier. It is not yet a complete asset/script/plugin build manifest.
That stronger compatibility gate is required before executable guest gameplay.
Discovery, packaged host menus, controllable guests,
dedicated-server shipping and full gameplay replication remain planned.

## Verification

Loopback UDP tests cover baseline/delta reconstruction, spawn/delete, dimension
and scene epochs, wrong admission/build, failed binds, slow-reader coalescing,
and graceful close under 5% loss with delay/jitter. Codec tests reject oversized,
truncated, non-finite and mismatched-base frames. Runtime integration runs both
dimensions privately, attaches a guest, and closes LAN while checking that the
run clock, script cadence and host movement continue. GPU rendering and a second
physical LAN machine require manual qualification.

Checked on this Linux host: `just build` for the whole workspace, 522 runtime unit tests (null audio; 69 GPU/manual tests ignored), 23 shell tests, and 18 transport/session tests (one network-matrix benchmark ignored).

Project capability tests cover legacy defaults, folder/wire/pack persistence,
invalid limits and undo/redo. The runtime fixes the capability and maximum guest
count at run start; opening LAN cannot exceed that limit.
