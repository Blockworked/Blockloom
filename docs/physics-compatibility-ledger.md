# Physics compatibility ledger (Phase 0)

Status: Phase 0 of `physics-and-character-controller-plan.md`. Probes live in
`blockloom-physics-probes` (`cargo test -p blockloom-physics-probes -- --nocapture`
prints the numbers quoted here).

Date: 2026-10-02.

## How to read this

- **Backend decision:** stay on the pinned Rapier (monorepo rev `09ca067a`, rapier
  0.35.3, parry 0.31.1). No row below needs a different engine. Four rows need
  adapter code Blockloom owns and two need a small backend extension; each is
  named in the table. Broad schema work (Phase 1) can start.
- **Proved** means a passing probe in `blockloom-physics-probes/tests/`.
  **Design** means the mapping is specified here but not yet run. **Open** means
  Phase 0 did not reach it, and the phase that owns it must probe it first.
- **Unity values come from Unity's documentation, not from a Unity install.** There
  is no Unity editor in the container, so the fixtures encode the documented
  contract (event matrix, combine priority, ForceMode formulas, skin width,
  step offset). Where Unity's documented behaviour is vague, the row says so.
- **Nothing GPU was run.** These probes are headless physics; they say nothing
  about rendering, the embedded Game view or `blockloom-runtime`.
- Every probe is 3D unless a row says otherwise. The 2D contract is Open (see the
  last section).

## Ledger

| # | Required feature (plan section) | Status | Mapping or extension | Probe |
| --- | --- | --- | --- | --- |
| 1 | Separate static and dynamic friction (4.3) | Proved, adapter | Rapier has one Coulomb coefficient per manifold. A `PhysicsHooks::modify_solver_contacts` hook picks static friction while the tangential speed is under a stick threshold and dynamic friction above it. A block on a 31 degree slope (tan 0.6) with static 0.8 / dynamic 0.3 rests when left alone, slides at the analytic 10.57 m/s after a push and breaks away when shoved. Any single coefficient gets one of these wrong. | `materials.rs` |
| 2 | Combine modes, priority Max > Multiply > Min > Average (4.3) | Proved | Rapier's `CoefficientCombineRule` has the same priority, so Average/Min/Multiply/Max map one to one. Friction and restitution combine independently. | `materials.rs` |
| 3 | Collider with no Rigidbody is static geometry; invisible colliders (4.2, 6.3) | Proved | A `Collider` with no parent body holds a ball at rest and answers rays. Nothing needs a `Look`. | `compounds.rs`, `queries.rs` |
| 4 | Rigidbody with no collider (5.1) | Proved, adapter | Rapier gives it zero mass and it does not fall. The adapter sets additional mass properties on the body (default 1 kg, authored mass otherwise). | `compounds.rs` |
| 5 | Compound mass, center of mass, explicit mass (4.2, 5.1) | Proved | Sum of shape masses with a combined local center of mass. An explicit body mass is exact (5 kg stays 5 kg with any shape count) and scales inertia linearly. Sensors can be massless. | `compounds.rs` |
| 6 | Shapes of one body do not collide with each other (4.2) | Proved | Rapier creates no pair for colliders sharing a parent. | `compounds.rs` |
| 7 | Remove a shape, recompute mass (4.2) | Proved, adapter | Mass is stale after removal until `recompute_mass_properties_from_colliders`; the ownership transaction calls it. | `compounds.rs` |
| 8 | Disable a collider (6.2) | Proved | Contacts end with a `REMOVED` stop event and queries stop seeing it. | `compounds.rs`, `events.rs` |
| 9 | Event matrix for ordinary 3D pairs (6.1) | Proved, adapter | Rapier's default `ActiveCollisionTypes` matches Unity for solids. Unity delivers triggers for kinematic/static and kinematic/kinematic pairs, which Rapier drops by default (static/kinematic and kinematic/kinematic); the adapter gives sensors `ActiveCollisionTypes::all() - FIXED_FIXED`, which reproduces the whole matrix while solids keep Rapier's defaults, so a kinematic body never reports a solid collision with scenery. | `events.rs` |
| 10 | Enter / stay / exit per fixed tick (6.2) | Proved | Rapier raises a start and a stop event; Stay is derived by checking active contacts after each step (`has_any_active_contact`). Transitions alternate and land on their own tick. | `events.rs` |
| 11 | Several fixed steps per rendered frame keep every transition (6.2) | Proved | Events are drained after every step. Polling contact state once per frame saw 1 of 7 touches in the probe (8 steps per frame), so reporters read a per-tick event log, never live state. | `events.rs` |
| 12 | Touching is a reference count over collider pairs (6.2) | Proved | Pair events fold into an actor counter; the actor sees one enter and one exit while the shapes come and go. | `events.rs` |
| 13 | Collision payload (6.2) | Proved | Normal, contact points and aggregate impulse are readable from the contact pair on the start event (impulse 3.05 N s for a 2.6 N s momentum change, so it includes the solver's penetration correction). Relative velocity comes from the bodies' velocities saved before the step (`CollisionEvent` does not carry it). | `events.rs` |
| 14 | Sleeping contacts stay visible (6.2) | Proved | A sleeping body keeps its pair and `has_any_active_contact` stays true. | `events.rs` |
| 15 | Synthetic Exit for removed pairs (6.2) | Proved | Rapier raises `Stopped` with the `REMOVED` flag when a collider is disabled or removed. | `events.rs` |
| 16 | Rays: invisible, body-less, trigger policy, filtering (6.3) | Proved | `QueryFilter` with `exclude_sensors` and groups. Query groups are independent of collision groups, so a query can use its own mask. | `queries.rs` |
| 17 | Stable hit order (6.3) | Proved, adapter | `intersect_ray` returns hits unsorted; the adapter sorts by distance then collider ID (two hits at 9.5 m tie in the probe). | `queries.rs` |
| 18 | Starting inside, zero distance, zero direction (6.3) | Proved, adapter | A ray starting inside a solid reports distance 0 with a zero normal; hollow shapes report the exit. A zero-direction ray returns nothing, so the adapter rejects it with an error before calling. | `queries.rs` |
| 19 | Backface policy and triangle identity (6.3) | Proved | `RayIntersection` has `is_backface` and a trimesh feature id. A one-sided policy is `is_backface` filtered. | `queries.rs` |
| 20 | Shape casts, overlaps, closest point (6.3) | Proved with limits | Accurate away from touching. Within about 1 cm of contact, parry's capsule-vs-cuboid `intersect_shape` and `contact()` flicker (true/false alternates across 1 mm steps). The adapter uses `distance`/`closest_points` for clearance and treats under 1e-3 as touching. | `queries.rs`, `controller.rs` |
| 21 | Queries see current poses (6.3) | Proved, adapter | Queries are stale until the broad phase is synchronized: a body moved onto a ray was invisible until `detect_collisions`. The fixed step gets an explicit query-sync stage. | `queries.rs` |
| 22 | Query cost (6.3) | Proved | 20 000 colliders, 2000 rays: 0.4 microseconds per ray (measured in a debug test build, so a release build should be no slower). Cost follows candidates, not collider count. | `queries.rs` |
| 23 | Sync stage and fresh velocity-kinematic bodies (10.1) | Proved, adapter | `detect_collisions` before the first step of a velocity-based kinematic body drops it from the active set and it never moves. A `wake_up` after the sync (or one step first) fixes it. The sync stage wakes bodies spawned that tick. | `controller.rs` |
| 24 | CCD mode names Discrete / Continuous / Continuous Dynamic / Speculative (5.3) | Proved, adapter | See the CCD table below. No two names may share a backend flag set. | `ccd.rs` |
| 25 | CharacterController: skin width (7.1, 7.2) | Proved | Rapier's `offset` is the gap the controller leaves (0.0101, 0.0801 and 0.2001 for offsets 0.01, 0.08, 0.2, on floor and wall alike), so offset = Unity skin width. | `controller.rs` |
| 26 | Flags Sides / Above / Below and `isGrounded` from the completed move (7.1) | Proved | Derived from the move's collision normals (`up > 0.7` below, `< -0.7` above, else sides). A falling capsule has no flags and is not grounded; landing sets Below. | `controller.rs` |
| 27 | Slide along walls without adding energy (7.2) | Proved | A (6, 0, 6) move into a wall whose face is at x = 2.5 stops at x = 1.92 (face minus radius and skin), keeps the full 6 m of tangent, adds no height and never moves further than requested. | `controller.rs` |
| 28 | Thin walls and large displacement (7.2) | Proved | A single 100 m move stops at a 1 cm wall (x = 29.415: the face at 29.995 minus radius and skin). | `controller.rs` |
| 29 | Slope limit (7.1) | Proved | With `max_slope_climb_angle` 45 degrees the capsule climbs 30 and 44 degree ramps and is stopped by 46 and 60 degree ramps. | `controller.rs` |
| 30 | Step offset (7.1, 7.2) | Proved, adapter | Rapier's autostep is not the Unity step offset: its reach depends on capsule radius and `offset` (a 0.3 m limit climbed 0.19 to 0.47 m across the probe settings). The adapter owns stepping: ray down past the riser, require a walkable top and a rise within the step offset, then sweep up, forward and down. It climbs 0.28 m and refuses 0.34 m at every radius and skin tested. | `controller.rs` |
| 31 | Overlap recovery, bounded (7.2) | Proved with limits | Rapier recovers only on zero-length moves and caps each call at a quarter of the height: a 5 cm overlap is resolved in one call (it moves 0.13 m, the overlap plus the skin), an 80 cm one moves 0.5 m in the first call and finishes in the second. Starting inside the skin zone tunnels through the floor, so the adapter first runs a skin-zone recovery from `closest_points` (with a `project_point` fallback when the witness is unreliable) and never submits a zero-length move (a tiny nudge instead). | `controller.rs` |
| 32 | Crouch and stand-up headroom (7.2) | Proved | Clearance for the standing 2 m capsule is tested with `distance` (not `intersect_shape`, which flickers): a ceiling at 2.02 m allows it, 1.9 m and 1.5 m refuse it. | `controller.rs` |
| 33 | Capsule height is end-to-end (7.1) | Proved | Total height maps to `capsule_y(height / 2 - radius, radius)`. | `controller.rs` |
| 34 | Moving platforms (8.1 platform policy) | Proved, extension needed | Rapier's controller carries a character on a kinematic support itself, but only inside a move that hit the support and with lag: the character trailed a 3 m/s platform by up to 0.42 m, then rode steadily. The carry is swept (a wall stopped it at 4.92 m) and the character never sank. Adding an explicit carry on top double counts (the offset to a position-based platform wandered 0.10 to 0.26 m). Phase 4 must choose one owner. Recommendation: a `carry_kinematic_supports` switch on `KinematicCharacterController` (a patch to the pinned Rapier) so the adapter supplies the support delta as a swept move; fallback is subtracting a measured probe delta. | `controller.rs` |
| 35 | Material stick/slip does not belong to the controller (4.3) | Design | Controller traction and slope policy stay in the adapter; only contact materials use row 1. | - |
| 36 | ForceMode formulas (5.2) | Design | Force: apply `f * dt` impulse; Acceleration: `a * mass * dt` impulse; Impulse: apply directly; VelocityChange: `dv * mass` impulse (or write velocity). All four reduce to `apply_impulse` with the fixed step's `dt`. Rapier's own `add_force` persists until reset, so the adapter uses impulses and never relies on persistent forces. | - |
| 37 | Max depenetration velocity, solver iteration overrides, restitution threshold (5.3) | Design | `IntegrationParameters` has contact softness and iteration counts; they are world wide. Phase 2 outcome: `solver_iterations` and `max_depenetration_velocity` are stored and validated but not applied; Bevy Rapier's `AdditionalSolverIterations` exists per body and is the candidate for the first, and a depenetration clamp still needs a `modify_solver_contacts` pass. Open. | - |
| 38 | Interpolation and pose ownership (5.2) | Design | Existing `PhysicsPose` / `PrevPose` stay; the `PhysicsOwnership` table decides who writes a pose. No probe needed beyond rows 4 to 7. | - |
| 39 | Joints (9) | Open | Not probed. Rapier has fixed, revolute, prismatic, spherical, rope and spring joints, joint motors and limits; break force needs adapter code. Phase 7 probes it. | - |
| 40 | Mesh cooking, convex decomposition, terrain heightfield (4.1) | Open | Rapier has trimesh, convex hull (parry), heightfield and V-HACD decomposition. Not probed; Phase 3. | - |

## CCD matrix (row 24)

A projectile at 300 m/s (5 m per step) is fired at something thin, or a 100 m/s kinematic
wall is driven into it; 60 Hz, 3D, no gravity. A cell says whether the mode **held**
(the projectile did not tunnel or the trigger was seen). Values are the asserts in
`ccd.rs::tunneling_table`.

| Rapier configuration | 4 cm static wall | Trigger seen | Dynamic plate | Spinning bar (40 rad/s) | Fast kinematic wall |
| --- | --- | --- | --- | --- | --- |
| Discrete (`max_ccd_substeps = 0`, no flags) | no | no | no | no | no |
| Automatic (Rapier as shipped, nothing set) | yes | yes | no | no | no |
| Bullet (`ccd_enabled`) | yes | yes | yes | yes | no |
| Speculative (`soft_ccd_prediction`, sweep off) | yes | no | yes | no | yes |
| Bullet and speculative | yes | yes | yes | yes | no |

Findings:

1. Rapier sweeps against fixed colliders by default, so a true Discrete mode needs
   `max_ccd_substeps = 0`, which is a **world** switch, not a body setting.
2. No single flag holds every scenario: only speculative holds a fast kinematic wall,
   only bullet holds a spinning bar and a trigger, and a bullet projectile still
   tunnels through a bullet plate. Those are documented restrictions, not mappings.
3. The held-matrix is the contract the inspector shows beside each mode, so two
   names never silently share a flag set.

Mapping as shipped in Phase 2 (`physics_install.rs`, test `continuous_modes_hold_what_the_table_says`): Discrete and Continuous both leave Rapier's automatic sweep against fixed colliders (the `max_ccd_substeps = 0` world switch is not applied while legacy bodies share the world, so Discrete is documented as sweeping fixed colliders), Continuous Dynamic = `ccd_enabled`, Continuous Speculative = `soft_ccd_prediction` of 0.5 m. Original proposal: Discrete = no flags and the world switch,
Continuous = Rapier's automatic fixed sweep, Continuous Dynamic = `ccd_enabled`,
Continuous Speculative = `soft_ccd_prediction`. Where Unity promises a case a mode does
not hold (rows above), the adapter documents the restriction.

## Pinned-source audit

Read at rev `09ca067a`; items that changed a design are in the ledger above.

- `control/character_controller.rs`: `offset` is the rest gap. `check_and_fix_penetrations`
  only runs when the move is under 1e-5 and moves at most 25 percent of the height per
  call. The kinematic-friction carry is applied inside the move loop after a hit
  (`detect_grounded_status_and_apply_friction`).
- `pipeline/physics_hooks.rs`: `modify_solver_contacts` can change friction per manifold
  and runs only for colliders flagged `ActiveHooks::MODIFY_SOLVER_CONTACTS`.
- `dynamics/coefficient_combine_rule.rs`: priority Max > Multiply > Min > Average.
- `dynamics/ccd/ccd_solver.rs`: the sweep covers fixed colliders for any CCD-capable
  body, `max_ccd_substeps` of 0 disables it; angular speed is capped at pi/4 rad per step
  in the sweep.
- `pipeline/query_pipeline.rs` and `pipeline/physics_world.rs`: queries read the broad
  phase, which `step` and `detect_collisions` refresh.
- parry3d 0.31.1: capsule-vs-cuboid `contact()`, `intersect_shape` and shape casts are
  unreliable within about 1 cm of touching; `distance()` and `closest_points` are
  accurate.

## Still Open after Phase 0

- The 2D contract (Rigidbody2D, CharacterController2D, the 2D event matrix and its full
  kinematic option, one-way platforms): `rapier2d` is a dependency of the probe crate
  but no 2D probe exists yet. The Phase 2 and Phase 4 owners start by running these.
- Rows 37, 39 and 40 above.
- Unity reference fixtures are documentation derived (see "How to read this"). Whoever
  has Unity should confirm rows 9, 25, 30 and 31 against the editor and file any
  differences against this ledger.
- A backend-extension prototype for row 34 and a decision on it.

Phase 0 therefore passes its gate for the 3D contact, event, query, CCD and
controller rows. It does not pass for the open rows above, which carry into the phase
named in each row.

## Phase 1 notes

Phase 1 is the document layer (`blockloom-core/src/physics/`); it changes no runtime
behavior, so no ledger row moves from Design to Verified. What it fixed, for the
phases that read it:

- The Blockloom-chosen defaults (`DEFAULT_MAX_LINEAR_VELOCITY` and the sleep and
  solver values) are not from Unity's documentation and stay Open until a Unity
  editor confirms or replaces them.
- Legacy `Body` fields keep their meaning on migration; a capsule's old height plus
  radius becomes an end to end height, and a plane gets a thin box
  (`LEGACY_PLANE_THICKNESS`). Phase 2 must check both against how `dim2`/`dim3`
  build colliders today before the migration is applied to a real project.
- The 2D material rule (geometric mean of frictions, larger bounce) is the documented
  Unity 2D rule and has no probe yet (see "Still Open").
