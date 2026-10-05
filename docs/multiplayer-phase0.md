# Multiplayer Phase 0: findings

Status: Phase 0 of [the multiplayer plan](multiplayer-and-embedded-server-plan.md). Date: 2026-10-02.

Phase 0's gate is "native transport builds; a realistic list of extraction blockers and compatibility decisions exists". This page records both halves. The original Phase 0 added only the isolated `blockloom-net` transport. Later runtime and LAN work is recorded in section 7 and the linked slice notes.

What was **not** measured: anything that needs a GPU (this container has none), a Windows/macOS/Android host, or a built player. Those rows say so.

## 1. Quiche transport spike (`blockloom-net`)

`blockloom-net` is a workspace member (not a default member) that carries opaque bytes over Quiche 0.30 and BoringSSL. It has no game protocol yet. `cargo test -p blockloom-net` runs 9 tests over real loopback UDP; they all pass.

| Plan requirement | Result |
| --- | --- |
| Application ALPN separate from the editor | `blockloom-game/1`; a client offering anything else never establishes (test). |
| No global certificate-verification bypass | The client installs a BoringSSL custom verify callback that accepts only a certificate whose SHA-256 equals the invite's fingerprint. A wrong fingerprint fails inside the TLS handshake and the server never sees `Established` (test). The identity lives in memory (`Identity`), no PEM files. |
| Address validation | Stateless Retry. The token is an HMAC over the client address and original DCID; the post-Retry server connection ID is derived from the same HMAC, so no state exists until a client echoes a valid token. Forged tokens and random garbage change nothing (test). |
| Reliable streams | 768 KiB client-to-server arrives intact across several flow-control windows. |
| Datagrams | Negotiated; `dgram_max_writable_len` is 1310 bytes at a 1350 byte UDP payload. A larger send is refused with `DatagramTooLarge`; a full datagram queue returns `Ok(false)` so replaceable state can be dropped. |
| Bounded queues, no blocking send | `send_stream` refuses with `QueueFull` past 4 MiB queued per peer (test). The endpoint is driven by a nonblocking `poll()`; nothing blocks. |
| Peer caps and kick | `max_peers` refuses extra clients (test); `kick` closes with a reason the client reads (test). |
| Reproducible loss harness | `Impairment` (loss, duplication, delay, jitter, seed) wraps the send path. Pacing times from Quiche (`SendInfo::at`) are honoured by the same queue. |

### Network matrix

The plan's matrix, one thread, debug build, loopback, 256 KiB reliable transfer plus 200 datagrams at 5 ms spacing. Each side delays what it sends by half the RTT. Run it with `cargo test -p blockloom-net --test loopback -- --ignored --nocapture matrix`. Treat the numbers as shape, not capacity.

| RTT | Loss | Handshake | 256 KiB, jitter 20 ms | 256 KiB, no jitter | Datagrams of 200 (jitter 20 ms) |
| --- | --- | --- | --- | --- | --- |
| 0 | 0% | 2 ms | 6 ms | 8 ms | 200 |
| 0 | 5% | 4 ms | 45 ms | 47 ms | 184 |
| 50 | 0% | 154 ms | 1667 ms | 233 ms | 200 |
| 50 | 5% | 400 ms | 2483 ms | 1531 ms | 195 |
| 100 | 0% | 254 ms | 2367 ms | 457 ms | 200 |
| 100 | 5% | 701 ms | 4956 ms | 2810 ms | 200 |

### 1.1 `tokio-quiche` spike

`blockloom-net/tests/tokio_quiche.rs` (dev-dependencies only) builds the same pinned-identity connection on `tokio-quiche` 0.20's `ApplicationOverQuic`: a handshake, a 768 KiB stream, datagrams both ways, and a wrong-fingerprint client. Both tests pass (handshake 3 ms and 768 KiB in about 190 ms on loopback; the raw driver's loopback figures are not directly comparable because that spike reports them through the matrix).

What fit:
- The pin works through `ConnectionHook::create_custom_ssl_context_builder`, so the same BoringSSL verify callback and in-memory `Identity` carry over. No verification is disabled.
- ALPN, datagrams and stream flow control are plain `QuicSettings`.
- Address validation (Retry) is on by default.

What did not:
- **Retry control is global.** The only switch is `disable_client_ip_validation`; there is no per-connection or per-token decision, and quiche has no client API to present a token in an Initial anyway.
- The hook is only invoked when certificate file paths are supplied, so the in-memory identity needs dummy paths.
- The application callbacks own the quiche connection, so the `Impairment` harness (which wraps our send path and honours `SendInfo::at` pacing) cannot sit in front of it. Seeded loss tests would need a UDP proxy instead.
- It brings tokio and `foundations` into the dependency tree: about 6 extra minutes on a cold dev build here, and a second runtime next to Bevy's task pools. Stream writes need our own pending buffer, as in the raw driver, because `stream_send` is partial.
- It would not remove any code we need to keep: Retry-less admission, the peer cap, kick and queue bounds are still ours.

**Recommendation: keep the raw `poll()` driver.** It is about 600 lines, runs on the simulation host's own thread with no second runtime, and keeps the impairment seam the Phase 3 matrix needs. Revisit `tokio-quiche` only if the dedicated server (Phase 5) wants many thousands of connections, where its multi-socket listener is the real gain. The spike stays as a dev-only test so that check is cheap.

### Findings and decisions this forces

1. **The Retry round trip is a visible join cost.** Without jitter a handshake is about 2 RTT (Retry, then TLS). Quiche only needs Retry for amplification protection. The owner decided (section 6) to skip it when the invite carries the admission token, but that cannot be done at the QUIC layer, see section 6 decision 1.
2. **The server sends two Retries per connection attempt**, most likely because BoringSSL's ClientHello spans two Initial packets (not verified at the packet level). The client uses the first and drops the second, so it is harmless; the test asserts `>= 1`.
3. **Reordering hurts far more than the plan's loss figures.** With 20 ms of jitter (which reorders packets) a 50 ms RTT transfer takes 1667 ms instead of 233 ms at 0% loss, apparently because reordered packets read as loss to the congestion controller. This was only run with Quiche's default CUBIC. Phase 3 should try BBR and the relaxed loss threshold before the join baseline is sized, and the matrix should keep a jitter column.
4. **Short-header packets do not carry a connection ID length**, so the server must issue full length (20 byte) connection IDs. A 16 byte ID silently broke every post-handshake packet in the first draft.
5. **`tokio-quiche` 0.20 was spiked and works, but is not adopted.** See section 1.1.
6. **Build requirements.** `boring-sys` needs cmake, a C/C++ compiler and libclang (bindgen). A clean `cargo build -p blockloom-net` took 2m55s here, and clippy with `-D warnings` is clean. It cannot build for `wasm32-unknown-unknown`, so the crate must stay out of the web player (the plan's `multiplayer` feature gate) and out of the Android runtime until that target has its own qualification.

Not covered: Windows, macOS and Android builds of Quiche/BoringSSL; IPv6; mDNS discovery; 0-RTT (the spike never enables early data); connection migration (disabled in the config); WebTransport.

## 2. Headless extraction inventory

Evidence is `blockloom-runtime/src/lib.rs::add_world` and the files it registers. The runtime is 85 files and about 73,800 lines; 13 files (about 6,900 lines: `ai`, `atmosphere`, `logic`, `script`, `volumes`, `wind`, `player`, `bridge`, `plugin_services` and the shader-patch files) mention no render, audio, window, UI or mesh API. Everything else touches one. `add_world` registers dozens of rendering and presentation modules unconditionally, and both entry points build `DefaultPlugins` (`lib.rs:211`, `embed.rs:131`).

### 2.1 Simulation work that runs in the frame schedule (`Update`)

| System | Why it must move | Source |
| --- | --- | --- |
| `dim2/dim3::relay_collisions` | Was reading Rapier contact messages once per frame, so several fixed steps in one frame coalesced into one batch of `touch` events. The physics work replaced it with per-step tracking (`dim2/dim3::track_contacts`, `contacts.rs`). | `dim3.rs:943` (before the physics work) |
| `world::publish_sensors` | Rebuilds the snapshot every frame. Reporters therefore see frame-time state, not tick state, except for the fields explicitly sampled on the tick (atmosphere, water, level, scene, which the code comments call out). | `lib.rs` Update chain |
| `world::detect_clicks`, `type_into_focused_input` | Pointer picking and UI hit tests read `PrimaryWindow` and the camera. They are intent input and belong in the client. | `world.rs:1780`, `1932` |
| `world::rebuild_world`, `pump_editor` | Scene build and control messages share one frame system with presentation setup. | `lib.rs` Update chain |
| `streaming::update_streaming_cells` | Cell residency follows the world camera, so looking away unloads authoritative state. The plan requires residency around every player. | `streaming.rs` |
| `volumes::gather_volumes`, `environment::blend_environment` | The blend is weighed at the world camera and feeds `atmosphere::sample_atmosphere`, which runs in `FixedUpdate` and publishes `time of day`, wind and fog. A headless server has no camera, and two clients would disagree. | `atmosphere.rs:86` |

### 2.2 Presentation work that runs in the fixed step (`FixedUpdate`)

These are in the chain after `step_vm` and need to become client-bound effects with an explicit audience: `overlay::apply_ui_effects`, `ui_systems::bindings`, `fx::apply_fx_effects`, `sound::apply_sound_effects`, `lights::apply_light_effects`, `hdr::apply_hdr_effects`, `environment::apply_exposure_effects`, `ray_tracing::apply_ray_tracing_effects`, `volumes::apply_volume_effects`, `world::apply_cursor_lock`, `world::apply_rumble`, `world::apply_input_effects`. `step_vm` itself sends `RuntimeMessage::Say`/`Error`/`PluginCall` straight to the editor through `bridge::send` (32 call sites in `world.rs`, 7 in `plugins.rs`), so the simulation currently talks to a client without an intermediate queue. Animation stepping (`anim2d::step_animations`) fires gameplay events and stays in the simulation.

### 2.3 `Engine` is three things in one `NonSend` struct

`engine.rs::Engine` has about 70 public fields. Reading them against the plan:

- **Owned by the simulation thread** (must stay `!Send`): `vm`, `variables`, `lists`, `dicts`, `logic`, `scripts`, `plugins`, `script_events`, `spawned`, `clones`, `parents`, `attached`, `last_created`, `touching`, `pending_scene`, `project`, `save_data`, `driven`, `physics_filter`.
- **Client or device state**: `window_focused`, `wants_cursor_locked`, `preview_inputs`, `incoming` (the editor channel), `veil`, `speech`, `hdr_output`, `peak_nits`, `ray_tracing`, `gi_bounces`, `gi_samples`, `light_shadows`, `shadow_distance`, `no_point_shadow_maps`, `capture_probes`, `terrain_previews`, `cine_*`.
- **Run overrides that are presentation today but sensed by blocks**: `fog_density`, `aurora_kp`, `lightning_rate`, `wind`, `clouds`, `surface`, `water`, `weather`, `director_time`, `time_scale`. Each needs a decision: server-authoritative shared state, or a per-client view.

### 2.4 Window and device coupling in the shared systems

Window access is narrow: `world.rs` reads `PrimaryWindow` in five places (focus at `:1244`, pointer at `:1780` and `:1932`, cursor lock at `:4336`) and nowhere in `dim2.rs`/`dim3.rs` beyond cameras. GPU access outside the render modules is limited to the profiler and capture (`gpu.rs`, `capture.rs`, `luminance.rs`, `probes.rs`). Mesh-derived colliders, navigation and terrain heightfields are built next to their draw meshes (`model.rs`, `terrain/`), which is where the "cook without GPU uploads" work lands.

### 2.5 Existing headless footing

Several tests already build a bare `App` with `MinimalPlugins` plus a hand-picked set of runtime systems (`world.rs:4762`, `dim2.rs:1071`), including a `TimeUpdateStrategy::ManualDuration` fixed-step harness. The Phase 1 headless harness should grow from that rather than from `DefaultPlugins`.

## 3. Reporter execution domains

75 extension reporters live in `blockloom-core/src/value.rs` (`OPERATORS`), and the sensing snapshot (`sense.rs::Sensors`) is single-player throughout (`keys`, `mouse`, `touches`, `actions`, `gamepad_*`, `ui_focus` are one set each). By what they read:

| Domain | Reporters (extension ops) | Phase 2 treatment |
| --- | --- | --- |
| Authoritative actor and world state | `MyPosition`, `MyRotation`, `MyLocalPosition`, `MyParent`, `IsClone`, `NewActor`, `ComponentField`, `ActorCount`, `ActorPosition`, `ActorLocalPosition`, `Touching`, `IsTrigger`, `CastsShadows`, `CollisionLayer`, `RoomContaining`, `RayHit`, `RayDistance`, `CircleHit`, `DistanceTo`, `TileAt`, `WaterHeight`, `Underwater`, `CurrentScene`, `SceneNames`, `GamePaused`, `Timer`, `IsTweening`, `CurrentClip`, `CurrentFrame`, `AnimationPlaying` | Server. `DistanceTo` also reads `mouse`, and `Timer` picks its clock by strand kind (section 4); both need an owner rule. |
| Atmosphere (tick-sampled) | `TimeOfDay`, `SunElevation`, `CurrentWeather`, `ActiveVolumes`, `Atmosphere` | Server, once the camera-weighted blend in 2.1 is replaced. |
| Per-player input | `KeyDown`, `MouseDown`, `MouseButtonDown`, `MouseX/Y`, `MouseDeltaX/Y`, `MouseLocked`, `Touch*`, `Action*`, `Gamepad*` | Read the owner's input (plan section 7.1). Unowned actors must be refused by preflight. |
| Client interface and audio | `UiExists`, `UiFocus`, `UiShown`, `UiValue`, `UiText`, `UiSelectedIndex`, `SoundPlaying`, `BusVolume` | Client view models; the server gets only validated widget events. |
| Presentation output (GPU readback) | `ParticleCount`, `ParticleEventCount`, `ParticleEventPosition` | Not deterministic across machines (GPU particles). Keep on the client or give the server a CPU counterpart. |
| Device and GPU | `SceneLuminance`, `IsHdrDisplay`, `PeakBrightness`, `IsRayTracing`, `RayTracingAvailable`, `FrameTime`, `DrawCalls`, `CurrentQuality`, `DlssAvailable`, `IsCutscenePlaying`, `CutsceneTime` | Never authoritative. Diagnose in server gameplay; offer as local telemetry. |

The 75 extension operators are all assigned above (classified by reading each one's `eval`). The core built-in reporters outside `OPERATORS` were not enumerated; the Phase 1 audit should add a test that every operator names a domain so a new reporter cannot arrive unclassified.

## 4. Clocks

Facts about the clocks that matter for the plan's section 6:

- The VM takes two numbers per tick: `now` (frozen while paused) and `wall` (`Vm::tick_at`, `vm/exec.rs:747`). A UI strand sleeps against `wall`, everything else against `now`.
- `step_vm` derives both from `time.elapsed_secs()`, which inside `FixedUpdate` is the fixed clock, so `wall` is **not** wall time: it advances only with fixed steps. The comment in `Sensors` ("the unfrozen clock") is true only relative to `pause`.
- `elapsed_secs()` is `f32`. Casting to `f64` after the fact keeps the rounding: at one hour the resolution is about 0.25 ms and at twelve hours about 4 ms, against a 16.7 ms step. The plan's "integer durations" requirement should start with `elapsed_secs_f64()` at the call sites in `world.rs` (`:271`, `:620`, `:1266`, `:2163`, `:3869`).
- Game speed is `Time<Virtual>::set_relative_speed` (`cinematic.rs:312`), clamped 0 to 2. Bevy's virtual clock also caps one frame's delta (250 ms by default), so a stall silently loses simulation time rather than carrying the debt the plan asks for. The server driver needs its own accumulator, or an explicit `max_delta`.
- The fixed rate is `world.fixed_rate` applied by `sync_timestep` (`world.rs:269`, 1 to 1000 Hz). Physics is `RapierPhysicsPlugin::in_fixed_schedule` with one substep at 1/60 s, so the 30 Hz profile validation in plan 6.2 needs a substep decision for both dimensions.

## 5. Baseline

- VM cost, `cargo bench -p blockloom-core --bench vm`, 50 actors, release build, shared 4-core cloud container (so noisy, and not comparable to the numbers in `docs/performance.md`):

| Canvas | ns/tick | ns/actor |
| --- | --- | --- |
| arithmetic | 27,144 | 543 |
| effects | 17,590 | 352 |
| custom_blocks | 28,687 | 574 |
| lists | 112,770 | 2,255 |
| sensing | 88,630 | 1,773 |
| sensing_crowd | 82,782 | 1,656 |
- Sensing publish cost: the ignored `sensor_publish_cost` test in `world.rs:7273` needs a runtime build; not run here.
- Input latency, resident memory and per-frame CPU of the existing player: not measured. They need a GPU and a built player, and the plan's gate for Phase 1 ("measure resident memory, local input latency and allocation cost against the existing player") needs them. A local run of `just player` plus `docs/performance.md`'s harness is the way to collect them.

## 6. Decisions (answered 2026-10-02 by the project owner)

1. **Retry:** the owner chose to skip it when the invite's admission token is present and keep it for direct IP joins. **This cannot be built at the QUIC layer:** the token is an application secret in the invite, but Retry happens before any application data, quiche has no client API to put a token in the Initial, and `tokio-quiche` only has a global off switch. The workable form is adaptive Retry: skip it while half-open handshakes are under a cap and per-source rate is low, and require it above that (or always, if the extra round trip is acceptable). The invite token then gates admission after the handshake. Needs the owner's pick before Phase 3.
2. **Transport driver:** try `tokio-quiche`. Done (section 1.1): it works, but the raw `poll()` driver stays because of the global Retry switch, the lost impairment seam and the extra runtime. Recommendation, awaiting the owner's confirmation.
3. **Run overrides:** wind, water, weather and time of day (and the other overrides gameplay reporters read) are shared server state. Fog, clouds, aurora and lightning become per-client looks. The first replication schema follows this split; any override a reporter reads from that second group needs a server counterpart or moves to the first.
4. **"Wall" time:** becomes real wall time for UI strands. This changes existing games slightly under time scaling or a stall, so it needs a compatibility note and a test, and should land with the clock work in Phase 1 (together with `elapsed_secs_f64`). **Landed in Phase 1:** `Engine::wall_time` reads `Time<Real>` from the green flag. Compatibility note: before, `wall` was the fixed clock minus the run start, and a resume moved the run start forward, so `wall` jumped back by the paused span (a UI strand sleeping across a pause waited that long again); it now never rewinds. Under game speed below 1, or after a stall, UI waits now finish in real seconds. Tests: `the_wall_clock_*` and `step_vm_hands_the_scheduler_real_time_as_the_wall_clock` in `world.rs`.
5. **Logical rate:** v1 multiplayer uses each project's existing fixed rate. Lower-rate profiles and their physics substep policy are deferred.

## 7. Where Phase 1 starts

Status as of the first Phase 1 slices (the same branch as Phase 0):

1. **Done, partly:** `add_world` is split into `simulation::add_simulation` and the presentation registration. The fixed step and physics are separated with unchanged order (each simulation system is a `SimStep`, presentation systems order against the steps they sat between). The `Update` chain and `rebuild_world`/`apply_lifetimes` are still interleaved with presentation and are the next extraction.
2. **Done, by the physics work:** contacts are tracked per fixed step (`dim2/dim3::track_contacts`, `contacts.rs`) and delivered on the next tick (`world::deliver_contacts`). Actor sensing now runs after authoritative poses are restored and before the fixed-step schedulers, including tile/room state when available. Frame publication retains those actor samples while running; stopped editor previews still sample their live transforms. The headless batching regression checks that position-dependent blocks see every logical tick. Run time, real wall time, pause state and scene names now refresh before each gameplay tick, after any pending scene switch. Frame publication retains the logical run time and advances the UI wall clock. Client input/UI and presentation telemetry still need separate contracts.
3. **Done:** `volumes::VolumeEye` lets a world with no camera weigh volumes at an actor or a point. The server still needs to set it, and presentation blends per client later.
4. **Done for the fixed step:** `simulation::tests` run a 2D and a 3D project headless over `MinimalPlugins`, check determinism, and check native logic against the VM. Needs the asset stores (CPU-only collections) until `rebuild_world` is split.
5. **Done:** clocks (real `wall`, `elapsed_secs_f64`). Still open: Bevy's 250 ms virtual delta cap and the server's own accumulator.

Verification: the 420 runtime unit tests, plus the ignored GPU suite on lavapipe (`BLOCKLOOM_TEST_OPAQUE_FD=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo test -p blockloom-runtime --lib -- --ignored --test-threads=1`, about 30 minutes). Before the split 54 passed and 11 failed on this container's lavapipe (the failing set is in the PR description); the same suite after the split is recorded there too.

Fixed-step sensing slice (2026-10-04): 516 runtime unit tests and 10 core sensing tests passed. The runtime suite used an ALSA null output because the host audio-device probe stalled; 69 GPU/manual tests remained ignored. New regressions cover position-dependent blocks under batched ticks, settled poses while paused, editable stopped previews, and actor-name cache invalidation without replacing input.

Run-context sensing slice (2026-10-04): 519 runtime unit tests and 11 core sensing tests passed, with the same null-audio setup and 69 GPU/manual tests ignored. Regressions exercise timer-driven movement under individual and batched ticks, headless scene metadata, paused game time with advancing real time, frame retention of logical time, and preserving actors and client state when run context is refreshed.

LAN spectator slice (2026-10-05): see [multiplayer-lan.md](multiplayer-lan.md) for the endpoint, replica protocol, editor/shell controls and native spectator clients. This is read-only actor replication, not completion of the player-ownership or full-gameplay replication gates.

The LAN spectator slice now includes saved project opt-in and a run-scoped guest
limit. Old projects remain private, and capability edits apply on the next Play.
Player ownership and per-player input are still unimplemented.
