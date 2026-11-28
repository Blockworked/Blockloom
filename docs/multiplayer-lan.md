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

On another desktop, launch the primitive spectator view (see Playing as a
guest below for the game itself):

```bash
target/release/blockloom-runtime --join "$LAN_INVITE" --trusted-build "$CONTENT_HASH" --spectate
```

Supply the content hash through a trusted channel. It is the invite's final
hex field. This viewer does not execute game code from the network or load
remotely supplied assets. It draws rectangles/circles in 2D and
cuboids/spheres/capsules/planes in 3D using a fixed spectator camera; an actor
with any other look is left out (logged once). It is a diagnostic spectator
view, without the host's camera, UI, lighting, material graphs, animation or
other presentation effects.

To see plugin worlds (voxels), give the guest a local build of the same game:
run the build's own player with `--join`, or pass `--game <build folder>`.
The view then opens that build's shipped plugins (an engine-tier plugin such as
voxel is linked into the build's player), starts them as a preview with the
pack's records, hands them the host's synced regions through `sync.apply` and
draws the meshes they answer, framing the camera on the first world. Plugin
code only ever comes from the local build; the host sends data, which the
plugin checks (voxel refuses a world whose settings differ from its own).

```bash
"Pond Game (Linux x64)/Pond Game" --join "$LAN_INVITE" --trusted-build "$CONTENT_HASH"
```

The text inspector accepts all look types and reports replicated actor poses:

```bash
target/release/blockloom-lan "$LAN_INVITE" "$CONTENT_HASH"
```

Stop/Start/Load and runtime shutdown close the listener. Close LAN disconnects
guests while the host continues; the endpoint drains close packets for up to a
second. Reopening makes a new identity and admission token.

## Replication contract

The schema version (2) is independent of the editor protocol. A guest first passes
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

Plugins can replicate their own state as regions (schema 2). Every six fixed
ticks the host asks each hosted module `sync.sections` with the generation and
revision it last answered; a module without the op is skipped for the run.
Each answer's identity and sections become regions keyed `plugin/identity` and
`plugin/<key>`, and a new generation drops the plugin's old ones. A guest gets
a region again only when its revision or bytes change, and removals are sent
explicitly. Regions that don't fit one frame beside the actors are staged:
each frame carries as many as fit, the guest acknowledges that partial state,
and the next frame continues from it. The voxel plugin sends only edited
sections, so a guest's copy generates the rest from the same settings
(`sync_apply_calls` turns held regions into its `sync.apply` calls). The
spectator view does this with a local build (above); `blockloom-lan` reports
region counts. The guest runs no hooks, so a streamed world shows what its
start and the synced sections mesh, and textures load from the local build.

Limits: 16 guests, 1024 actors, 16384 regions of at most 384 KiB, 1 MiB per complete frame, 256-byte identifiers,
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

## Seats and input (schema 4)

An admitted guest gets the lowest free player seat (1 to 16; seat 0 is the
host's own player) in a seat frame before its first state. It then sends its
own actions by base name (`Jump`, `Move`) and its view (camera yaw, and the
body yaw a first-person camera turns, each optional) whenever they change and
at least every 250 ms: at most 64 actions, names up to 64 bytes, finite values
within 10000, and a rising sequence; anything else drops the guest. The host
feeds a seat's actions in as that player's action copies (`Jump P2` for seat
1), in place of the local pad, so a character motor or player camera set to
that player follows the guest. A motor set to that player moves relative to
the guest's camera and turns with the guest's first-person view
(`server::SeatViews`), not the host's. An action the guest hasn't sent reads
as released. `when player joins`/`when player leaves` fire on the host with
the player number (2 and up), and `player count`/`player connected?` read the
seats. Scripts don't see these events yet.

## Playing as a guest

A guest with a local build of the same game runs that game as a client of the
host (`lan/remote.rs`, a `ServerLink` like the integrated server): its own
world builds the document, sends its seat's input, applies each tick's effects
the host publishes (frame 7: the tick, then the effects as JSON, saves and
plugin calls left out) and takes the host's poses. Its camera follows the
rig's settings on its own seat's actor: the first actor, by id, whose
player-driven motor or player camera names that seat. Without a build beside
the runtime (or `--game`), or with `--spectate`, `--join` opens the diagnostic
viewer above instead.

```bash
"Pond Game (Linux x64)/Pond Game" --join "$LAN_INVITE" --trusted-build "$CONTENT_HASH"
```

A guest that joins late catches up from the snapshot's roster
(`server::catch_up`): clones the host made earlier are cloned from their
template, and authored actors the host already deleted go. A roster older than
effects already taken, or from another scene, is skipped. Not yet: actors made
by `create actor` before the guest joined aren't rebuilt, and there is no
prediction, so a guest feels the round trip.

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
