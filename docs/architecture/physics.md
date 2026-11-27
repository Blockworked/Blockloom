# Physics

Rigidbody and Collider documents, the Rapier runtime, queries, character controller and motor, player camera, constraints, upgrade and samples. Split out of `CLAUDE.md`; the index there lists every page.

## Physics document (Rigidbody, Collider)

Phase 1 of the physics plan, `blockloom-core/src/physics/`. It is the saved model
and its rules. **Actors with a legacy `Body` still play through the old path**
(unchanged, byte for byte); actors with Rigidbody/Collider components play through
the Phase 2 installer below. Nothing converts a project until a later phase wires
migration into the editor, and an actor may not have both.

- `ActorComponent::Rigidbody { rigidbody }` (`RigidbodySpec`: body type, mass
  or density, damping, gravity, interpolation, collision detection, constraints,
  velocity caps) and `ActorComponent::Collider { collider }` (`ColliderSpec`: a
  `ColliderGeometry` of a saved `ColliderShape` or `FromLook`, center, rotation,
  material, trigger, layer, overrides). One body per actor; **colliders repeat**,
  so `Components::collider(id)`/`colliders()` address them by `ColliderId` and the
  name-based `get`/`remove` refuse an ambiguous repeated name. Ids are generated,
  unique per scene, and kept by undo (snapshots), save/load and reparenting;
  `Actor::refresh_physics_ids` gives a duplicate its own. A capsule's `height` is
  end to end.
- Ownership (`ownership.rs`, `PhysicsOwnership::resolve`): a collider belongs to
  its own actor's Rigidbody or the nearest ancestor's (via `Parent`); no body above
  it makes it static; a nested Rigidbody starts a separate body. The shape's pose in
  the body's frame is derived from the same world matrices `place_authored_children`
  builds, honoring `Parent` offsets. Static colliders need no Look and a body may
  have no shape; both are valid.
- Settings and materials live on `Project.physics` (`PhysicsSettings`: schema
  version, `CompatibilityProfile` Legacy or Unity, `MaterialLibrary`), written only
  when not default so an untouched legacy project re-saves byte for byte. A newer
  `schema_version` fails to load with a clear error. Built-in materials (Default,
  Ice, Rubber, No Bounce) are referenced by name; stored ones by `MaterialRef::Asset`.
  3D combine priority is Maximum, Multiply, Minimum, Average; 2D uses the geometric
  mean of frictions and the larger bounce.
- `meta.rs` holds one property table per component (unit, bounds, world, advanced);
  a test keeps it in step with the serialized defaults, so a new field needs a row.
  `validate.rs` checks specs and a whole scene (shape in the wrong dimension, a
  solid concave shape on a dynamic body, a trigger that is also one way, an
  unresolved material, a legacy `Body` beside the new components, ...).
- Edits (`edit.rs`) run on one actor, validate and roll back that actor on failure;
  only errors on the edited actor or its touched ids refuse an edit, so a
  hand-edited bad actor elsewhere does not lock the scene.
- Migration (`migrate.rs`): `Scene::physics_migration_preview` shows what each legacy
  `Body` becomes (Rigidbody plus `FromLook` collider plus a stored legacy material,
  ids `{actor}-rigidbody`/`{actor}-collider`); `migrate_physics` applies it
  idempotently. Only the preview is exposed (`physics-migration-preview`) because
  the runtime cannot read the result yet.
- Commands (`blockloom-app/src/commands/physics.rs`): `add-collider`,
  `set-collider`, `remove-collider`, `fit-collider-to-look`, `set-rigidbody`,
  `remove-rigidbody`, `set-physics-profile`, `add|set|remove-physics-material`, and
  the reads `physics-check`, `physics-ownership`, `physics-properties`,
  `physics-migration-preview`. A refused edit leaves no undo step. The generic
  `add|set-actor-component` refuse these two components and point at the typed
  commands. The wire spelling is `{"kind": "Shape", "shape": {"kind": "Sphere", ...}}`.
- Not yet: any UI (the inspector shows an empty card for these components) and
  verified Unity defaults for the Blockloom-chosen values
  (`DEFAULT_MAX_LINEAR_VELOCITY` and the sleep and solver defaults).
  Tests: `cargo test -p blockloom-core physics` (`tests/physics_gate.rs` holds the
  gate fixtures) and `cargo test -p blockloom-app --test physics`.

## Physics runtime (bodies, shapes, filtering)

Phase 2 of the physics plan. Core decides, the runtime installs.

- **Core** (`blockloom-core/src/physics/`): `plan.rs` `PhysicsPlan::build(actors,
  mode, settings)` (also `Scene::physics_plan`) turns a scene into `BodyPlan`s and
  `ColliderPlan`s and carries every problem (`issues`, a superset of
  `validate_scene`). `geometry.rs` applies Unity's scale rules (a box scales per axis, a
  sphere by its largest axis, a capsule's radius by the larger of the two axes across it
  and its length by its own axis; negative scale only mirrors) and never turns a
  primitive into a hull. `layers.rs` is the 32-layer matrix per dimension
  (`LayerSettings`, saved on `PhysicsSettings.layers`, written only when changed) plus
  `ColliderFilter`/`pair_collides`: a collider's include/exclude overrides decide first
  (exclude beats include, the higher priority wins between two, a tie excludes),
  then the matrix, then each side's legacy mask. `needs_exact` says when two group
  masks cannot say it (an include, or an exclude with a priority) and the plan sets
  `exact_filtering`. `ops.rs` is the `ForceMode` math (every mode reduces to an
  impulse for one fixed step, so a force never lingers).
- **Mass**: shapes carry it. An explicit body mass becomes density `mass / volume`
  over the shapes that count (enabled, not triggers, with volume), so the total is exact
  whatever the shape count and COM and inertia come from the shapes. A body with
  nothing to carry it holds its own mass (`ExtraMass`; default 1 kg). A custom
  inertia (or center of mass, which needs an inertia too, else an error) moves the whole
  mass onto the body. Static and kinematic bodies carry no density.
- **Positions**: a collider's pose is solved in its frame actor's space (the body actor,
  or its own actor for scenery) from the same world matrices ownership uses, in
  metres (`position`) and as the local translation under the frame entity (`local`,
  which bevy_rapier multiplies by the frame's scale). A rotated shape under a stretched
  frame would shear and is an error. Zero scale is an error.
- **Unsupported on purpose** (errors, not silent): convex hull, triangle mesh,
  terrain and tilemap colliders (Phase 3: cooking), a polygon that is not convex, a
  shape for the other dimension, a concave solid on a dynamic body, a custom center of
  mass without an inertia. A collider's `contact_offset` is Rapier's `ContactSkin`
  (times pixels per metre in 2D), `queryable` filters ray/cast/overlap queries, and a
  body's `interpolation` is a `PoseSmoothing` component read by `world::blend_pose`
  (None draws the fixed-step pose, Interpolate blends, Extrapolate carries the last step on;
  an actor with no Rigidbody keeps blending, and migrated bodies are Interpolate).
- **Runtime** (`blockloom-runtime/src/physics_install.rs`): `install` runs at the end
  of `rebuild_world` for the scene's plan (an erroring plan installs nothing and logs).
  A body gets `RigidBody`, `Velocity`, `ExternalImpulse`, gravity scale (Use Gravity off is
  scale 0), damping, `LockedAxes`, `Sleeping`, CCD, extra mass and
  `RigidBodyDisabled` for Simulated off. **Each collider is its own child entity**
  of its frame entity (`PlannedCollider`), with `ColliderScale::Absolute` (the shape is
  built at its final size), density, `Friction`/`Restitution`, collision groups, `Sensor`
  for triggers (`ActiveCollisionTypes::all() - STATIC_STATIC`), `ColliderDisabled`, and
  one-way (2D). Removing a collider entity is how a shape goes away;
  `refresh_masses` nudges every planned body so the mass totals again (Rapier keeps
  the old one otherwise). Contacts on a collider entity are tracked as touches on the
  actor that owns the component (see Contact lifecycle below).
- **Hooks**: 3D uses `Hooks3` (`RapierPhysicsPlugin::<Hooks3>`) for the stick/slip
  friction (static friction below `STICK_SPEED` of sliding at the contact point,
  dynamic above; only on colliders whose two coefficients differ) and the exact pair test;
  2D reuses `OneWayHooks` (now also the pair test). 2D friction is Rapier's geometric
  mean and bounce its maximum, set per collider. `PhysicsLayers` is the resource the
  hooks read. Velocity caps and frozen axes are enforced by `clamp_velocities` before
  each step (a 2D body has no angular cap; Unity 2D has none comparable).
- **CCD**: Discrete and Continuous both leave Rapier's own sweep against fixed colliders
  (a true Discrete would need `max_ccd_substeps = 0`, a world switch, so it is not
  applied while legacy bodies exist); Continuous Dynamic is `Ccd`; Speculative is
  `SoftCcd` with a 0.5 m prediction. See the ledger's CCD table for what each holds.
- **Solver overrides**: `solver_iterations` becomes Rapier's `AdditionalSolverIterations`
  on the body (Rapier's solver has no per-body iteration count, so this is extra
  substeps, not a Unity iteration count). `max_depenetration_velocity` caps how fast a
  contact pushes the body out of an overlap: Rapier's soft contacts correct position at
  the world's `normalized_max_corrective_velocity` (3 m/s) without touching velocity, and
  the per-body cap clamps each `SolverContact.dist` to at least `-cap * dt` in the
  `modify_solver_contacts` hook of both dimensions (`depenetration_cap`, the slower of
  the two bodies wins). A cap at or above 3 m/s changes nothing and installs no hook.
- **Force blocks**: `add force` and `add torque` (`Action::AddForce`, `Effect::AddForce`,
  `Act::AddForce`, `ACT_ADD_FORCE` 123 in the logic ABI and 101 in the script ABI, a
  script's `add_force`/`add_torque`) pick a `ForceMode` and a vector. `d2/d3::apply_forces`
  reduces the mode to one impulse for the fixed step (`ForceMode::linear_impulse_array`,
  `torque_impulse_3d/2d`, with the body's mass and world inertia from
  `ReadMassProperties`) and adds it to `ExternalImpulse`, so nothing lingers. Only dynamic
  bodies answer; the 2D world is in pixel units, so a 2D impulse is not rescaled.
- **Contact lifecycle** (`physics/contacts.rs`, `blockloom-runtime/src/contacts.rs`):
  `ContactTracker` is pure core. The runtime's `track_contacts` (`dim2`/`dim3`, in
  `FixedPostUpdate` after the writeback) feeds it Rapier's Started/Stopped reports once
  per fixed tick (sensor pairs are `ContactKind::Trigger`, with no payload; solid ones carry
  normal, points, relative velocity and impulse read from the contact pair), closes the tick
  and queues Enter, Stay (at most once per pair and tick, while either body is awake) and
  Exit in tick/pair/phase order. `step_vm` hands the VM what earlier ticks made
  (`world::deliver_contacts`), so an event made in tick N is heard in N+1 whatever the
  render rate. A pair is keyed by its two collider ids; the actor-level answer is a
  refcount over pairs (`touching`, `pairs_between`, mirrored into `engine.touching` for the
  `touching?` reporters), and `ContactEvent::edge` marks the actor-level transition that
  `when I touch` listens to, so a compound wall is one Enter and one Exit. A pair that
  ends because an actor was deleted (`delete_actor`) or a collider was removed/disabled
  (Rapier's REMOVED flag, `ColliderDisabled`) or a filter changed (`engine.filter_touched`)
  gets an Exit with an `ExitReason`; a run reset clears the tracker without delivering.
  `when I touch` gained `phase` (Enter/Stay/Exit, default Enter) and `scope` (Any/Collision/
  Trigger, default Any) so old documents behave as before; scripts hear `Event::Contact`
  (`EVENT_CONTACT` 21) for every phase. Still open: the shape-replacement and teleport
  reasons (a replaced collider entity ends its pairs as Separated).
- **Legacy effects**: `world::is_dynamic` also recognizes a simulated dynamic
  Rigidbody, so `push`, `set velocity` and the like act on planned bodies. The navmesh
  and fracture still read the legacy `Body` only.
- **Commands**: `set-physics-layer-name`, `set-layer-collision`, `physics-plan`;
  `physics-check` now includes plan-level errors, and Play and Build refuse a scene
  with a physics error (`commands::physics::preflight`).
- Tests: `cargo test -p blockloom-core physics`, `cargo test -p blockloom-runtime
  --lib physics_install` (headless mini-apps with a real Rapier world: resting, mass
  totals and removal, modes, triggers, layers, exact filtering, stick/slip on a slope,
  CCD, one-way, caps, forces, depenetration caps, contact events at two frame rates), `cargo test -p blockloom-app --test physics`.

## Physics queries (Phase 3)

`physics/query.rs` holds the request types (`QueryRequest`: ray, cast and overlap of
ball, box and capsule, closest), `QueryFilter` (layer mask, trigger policy, ignored
actor) and `QueryOutcome`. The runtime answers them from the live Rapier world
(`blockloom-runtime/src/queries.rs`) through a `QueryService` installed for the
frame. A query block (`CastRay`, `CastBall`, `OverlapBall`, `FindClosest`) asks on the
spot as the running actor and files the answer as that actor's query result;
`QueryNumber`/`QueryText` read hit N back by field (a miss reads zero or empty). The
VM, compiled logic (`ACT_PHYSICS_QUERY` is the script ABI's) and scripts
land on the same answers;
`tests/codegen.rs` holds them together with a one-wall harness world.
`RayHit`/`RayDistance`/`CircleHit` reporters ask the same service. With no world
installed a query reports an error and reads as a miss.

## Physics inspector

`RigidbodyForm.qml` and `ColliderForm.qml` are the cards for the two components
(`InspectorPanel.qml` loads them; a collider is removed by id with `remove_collider`, the
Rigidbody with `remove_rigidbody`, and Add component gives an actor a Rigidbody or another
Collider). Every edit sends the whole next spec to `set_rigidbody`/`set_collider`, so
validation and undo are the backend's. QML test: `tests/qml/tst_PhysicsInspector.qml`.

## Character controller (Phase 4)

A `CharacterController` component (`ActorComponent::CharacterController`,
`physics/controller.rs`, `CharacterControllerSpec`) is a capsule that blocks sweep
through the world, Unity-style: radius, height, centre, slope limit, step offset, skin
width, minimum move, detect collisions, overlap recovery, layer and up. It needs no
Rigidbody (the runtime gives the actor a position-based kinematic body when it has
none). `Scene::set_character_controller`/`remove_character_controller` edit it and
the plan carries `PhysicsPlan::controllers`; the shell has `set-character-controller`
and `remove-character-controller`.

- **Contract**: core decides, the runtime installs. `controller::move_call(actor,
  MoveMode, vector)` runs through a `ControllerService` (installed per frame by
  `controller::with_service`, the way queries are). `Move` is a displacement without
  gravity, `Simple` a speed with gravity (fall speed builds while airborne and is
  zeroed on a floor or ceiling). Moves run in order on the spot with a `pending`
  displacement; the pose is written once per fixed step by
  `blockloom-runtime/src/controller.rs::apply_motion`. A move below
  `min_move_distance` skips moving but still probes the ground and recovers overlap.
  Hits are classified by normal against the slope limit into sides, above and below
  (`CollisionFlags`, Unity's 1/2/4); triggers are never obstacles.
- **Runtime** (`blockloom-runtime/src/controller.rs`): `KinematicCharacterController`
  (`move_shape`) over a capsule child collider, 3D and 2D. Its hit normal points at the
  controller. `ControllerAccess` is folded into `QueryAccess::scope`, so a VM tick,
  compiled logic or a script can move a controller where it already asks queries.
  Fixed obstacles the controller meets fire `Event::Collision` (Enter, Stay, Exit by
  move); moving bodies are left to the contact lifecycle.
- **Blocks**: `ControllerMove` (mode Move or Simple, x y z) and `SetController`
  (property, value) are both `Action::Controller { op, vector }`, `Effect::Controller`,
  `Act::Controller` (`ACT_CONTROLLER` 125 in the logic ABI, 103 in the script ABI) and
  `controller::run_op`, the one entry every adapter calls. `ControllerNumber` and
  `ControllerText` read `(hit index, field)` of the last move (`grounded`, `flags`,
  `moved x`, `normal y`, ...; see `controller::read_number`). Scripts get
  `move_controller`, `simple_move_controller`, `set_controller`, `controller_number`,
  `controller_text` and `is_grounded` (`READ_CONTROLLER` 55, `TEXT_CONTROLLER` 23).
  `tests/codegen.rs` holds the VM and
  compiled halves together against a flat-floor harness.
- **Inspector**: `CharacterControllerForm.qml` (Add component gives an actor one;
  `tests/qml/tst_PhysicsInspector.qml`).
- Tests: `cargo test -p blockloom-core controller`, `cargo test -p blockloom-runtime
  --lib controller` (headless Rapier worlds in 3D and 2D: floor, step, slope, wall,
  trigger, overlap recovery, idle then move).

## Character motor and input actions (Phase 5)

A `CharacterMotor` component (`ActorComponent::CharacterMotor`, `physics/motor.rs`,
`CharacterMotorSpec`) is the reusable gameplay layer over a `CharacterController`: walk,
sprint and crouch speeds, ground and air acceleration, air control, turn speed, gravity
scale, terminal speed, ground snap, steep-slope sliding, jump height, max jumps, jump
cut, coyote time, jump buffer, crouch height and knockback drag. `owner` says who steers
it (`Player` reads the project's actions, `Script` is blocks and scripts, `Ai` is for
brains), `space` what a direction is measured against (`World`, `Actor`, `Camera`), and
`player` which local player's actions drive it. `validate` and the `set_character_motor`
edit refuse a motor without a controller, a second motor, or bad numbers.

- **Split**: `MotorState::plan` is pure (intent and environment in, displacement and
  events out) and `settle` takes the move's hits back (landing, head hits, slope,
  sliding, support). `motor::drive` is the glue that calls the controller service
  (`probe_move` for crouch headroom, `move_call(Move)` for the displacement, a second
  call for ground snap). It lives in core so the runtime stays thin.
- **Runtime** (`blockloom-runtime/src/motor.rs`): `latch_player_input` (Update, after
  `publish_sensors`) copies the Move/Jump/Sprint/Crouch actions (`Move P2` for the
  second player) into the intent of every player-owned motor; `drive_motors` runs first
  in the fixed chain, before `controller::apply_motion`, with the camera's yaw, the
  actor's facing, platform carry (the support actor's movement since last tick, ignored
  past 5 m as a teleport) and turning towards the move at `turn_speed`.
- **Blocks**: `MotorAct` (steer, jump, let go of jump, sprint/crouch on and off, push,
  stop) and `SetMotor` ride the controller op channel (`"motor <op>"` through
  `controller::run_op`, so no ABI change); `MotorNumber`/`MotorText` read the motor
  (`grounded`, `speed`, `jumps left`, `jumped`, `landed`, ...). Scripts get
  `motor_steer`, `motor_jump`, `motor_sprint`, `motor_crouch`, `motor_push`,
  `motor_stop`, `set_motor`, `motor_number` and `motor_text`. Events are exposed as
  one-tick reporter flags (`jumped`, `landed`, `left ground`, `hit head`); there is no
  `when I land` hat yet.
- **Input** (`input.rs`): actions have a `kind` (Button, Axis, Vector2), a four-key
  `composite`, `processors` (dead zone, scale, invert, normalize), a `map` that can be
  switched off with `set_map_enabled`, and a `player`. `InputBinding::MouseDelta` binds
  look. `InputConfig::add_player_actions` adds the standard Gameplay map (Move, Look,
  Jump, Sprint, Crouch, Interact) and `player_copy` makes per-player copies. The
  runtime publishes each pad on its own, so a second player reads only their pad.
- **Inspector**: `CharacterMotorForm.qml`. Shell: `set-character-motor`,
  `remove-character-motor`.
- Tests: `cargo test -p blockloom-core motor input`, `cargo test -p blockloom-runtime
  --lib controller_tests_2d` (a headless 2D motor walking, braking and jumping).

## Player camera, presets and profiles (Phase 6)

`blockloom-core/src/player_camera.rs` is `PlayerCameraSpec`, the
`ActorComponent::PlayerCamera` that *configures* the actor's existing `Camera`
(it never adds one): look sensitivity and stick speed, invert Y, pitch limits,
`turn_body`, smoothing, wall avoidance (`collision`, radius, `min_distance`),
scroll zoom, a 2D dead zone and look-ahead, and `eye_follows_stance`. The maths
(`look_turn`, `clamp_pitch`, `clear_distance`, `follow_2d`, `lead`) is pure and
tested in core. The generic `add_actor_component`/`set_actor_component` path
carries it and refuses a bad setting by field name (`check_player_camera`).

`blockloom-runtime/src/player_camera.rs` installs it inside `world::drive_camera`.
The stored `rig.pitch` is the look state, so `set camera pitch` blocks and look
input share it; yaw lives in `RigState`. Mouse look applies only while
`mouse_locked`; sticks use the "rightstickx/y" axes (0.1 dead zone). Third-person
pitch is elevation. Wall avoidance is a swept ball `QueryRequest::Cast` from the
pivot that skips the target and triggers. `BodyFacing` (actor to yaw) is how a
first-person `turn_body` camera hands its yaw to `motor::drive_motors`.
`Engine.look_lock_offered`: `apply_cursor_lock` takes the pointer once per run
when a `PlayerCamera` with look sits beside a `Camera`.

`blockloom-core/src/physics/presets.rs` is `PlayerPreset` (first person, third
person, top down 3D, platformer 2D, top down 2D). `Scene::preview_player_preset`
lists what applying would add, replace, keep, remove or convert;
`apply_player_preset` installs a visual, controller, motor, camera, player camera
and the input actions in one undo step. A dynamic Rigidbody or legacy Body is a
conflict that needs `convert`, and a refused apply leaves no trace.
`PlayerProfile` (version 1) captures a finished setup as
`assets/profiles/<name>.profile.json` and applies it to another actor.

Commands (dispatch, shell and MCP): `preview-player-preset`,
`apply-player-preset`, `save-player-profile`, `list-player-profiles`,
`apply-player-profile`, `import-player-profile`. QML: `PlayerSetupCard.qml`
(under the Name row: preset, preview, convert, profiles), `PlayerCameraForm.qml`
(the component card, "PlayerCamera" in Add component). Tests:
`tst_PhysicsInspector.qml`.

Open: no collision gizmos in the scene view, profile instance overrides and
reset-to-profile, QML not run here (no Qt), GPU paths not run.

## Constraints and integration (Phase 7)

`blockloom-core/src/physics/joints.rs` is `ConstraintSpec`, the repeatable
`ActorComponent::Constraint` (addressed by `ConstraintId`; the old single
`Joint` component is untouched). Kinds: Fixed, Hinge, Ball (3D), Slider,
Spring, Distance, Wheel, Configurable. An empty `target` anchors to the world.
Both ends need Rigidbodies. Lengths are document units (2D pixels), angles
degrees. With `auto_configure` the other end's frame is worked out from where
the actors stand, so Play starts with no snap. `blueprint()` is the backend
neutral recipe (locked axes, limits in radians, motors, `coupled_linear`), and
`plan_constraints` validates and resolves frames into `PhysicsPlan.constraints`.
Break force/torque thresholds snap the joint and may broadcast `break_message`.

`blockloom-runtime/src/constraints.rs` maps the recipe onto Rapier's
`GenericJoint`, one child entity per constraint (an `ImpulseJoint` is one per
entity). `drive` runs after `PhysicsSet::Writeback`: queued commands, rebuild on
motor/spring change, break detection off `ImpulseJointImpulses`, and status
(`position`, `speed`, `force`, `torque`) published for reporters.

Blocks/scripts use the op channel, so `ABI_VERSION` and `LOGIC_ABI_VERSION` are
unchanged: `JointAct` lowers to `Action::Controller` op `joint <verb>|<name>`
(`controller::run_op` routes the "joint " prefix) and `JointNumber` is an
ExtOperator over `joints::read_number`. A name is the constraint's `name` or its
1-based place. Scripts: `Actor::joint`, `joint_number`. Commands (dispatch,
shell, MCP): `add-constraint`, `set-constraint`, `remove-constraint`,
`list-constraints`. QML: `ConstraintForm.qml`, "Constraint" in Add component.

Integration: a buoyant body is sized by its colliders (`water/buoy.rs`
`collider_extents`), falling back to its Look, so it needs no Look. Fracture
shards keep the source collider's `CollisionGroups`, `SolverGroups`, friction and
restitution and share its body mass (`destruction.rs` `Sources`). A clone made
mid-run gets its Rigidbody, colliders and constraints through
`physics_install::install_actor` (fresh ids from `Actor::refresh_physics_ids`,
constraints appended with `constraints::install_more`); its mesh colliders and
CharacterController/Motor are not installed.

Physics Debug: `SceneView.physics` (`PhysicsDebug`: shapes, aabbs, contacts,
joints, axes; `PROTOCOL_VERSION` 30) switches Rapier's own debug renderer
(`physics_debug::sync`), off by default. The profiler gets
`physics/{bodies,active_bodies,sleeping_bodies,colliders,joints,step_ms}`.
The scene view has a box button that opens the panel (`PreviewPanel.qml`).

Open: no joint gizmo editing, no ragdoll handoff from animation, AI/navigation
still drives its own movement rather than the motor, collision streaming is not
separate from visual LOD, no single-step or fixture replay, no pair/contact
counts in the profiler, QML and GPU paths not run here.

## Upgrade, samples and shipping (Phase 8)

Docs: `docs/physics.md` (creator guide), `docs/physics-api-coverage.md` (what
each surface does and what is open), `docs/physics-compatibility-ledger.md`.

Upgrading legacy `Body` actors: `physics-migration-preview [actorId]` previews
across every scene; `migrate-physics [actorId]` converts them (`Scene::migrate_physics`
or `migrate_actor_physics`) as one undo step and copies `project.blockloom` to
`.blockloom/backups/` first. A call with nothing to convert files no undo step
and no backup. QML: `PhysicsUpgradeCard.qml` at the top of the Body card.

Samples: `blockloom_core::physics::sample::build` (`physics-playground`: player
preset, crates, rope pendulum, motor wheel, 3D door, snapping weld, trigger zone,
all components). `create-project ... sample=physics-playground` and the New
Project dialog's "Start from" use it. Tests: core (plans clean in both
dimensions, pack round trip), app (opens runnable), runtime (plays 240 ticks).

Genre templates live in `physics/genres.rs` beside it and go through the same
`sample::build`; `sample::modes` says which dimensions each fits, and the dialog
offers only those. `platformer` (2D: platforms a jump apart, coins, a flag),
`top-down` (2D, no gravity: walls, gems, a guard gliding on a kinematic trigger
body who sends the player back and costs a life), `first-person` (3D: orbs,
stairs to a ledge, a door that glides open on the fourth orb) and `ui-puzzle`
(either: a 3x3 lights puzzle of `show button` blocks and `when ui clicked`
strands; `LIGHTS_SOLUTION` solves it). Pickups are triggers whose own strand
counts a global and deletes itself. `simulation::tests` walk the player into
each first pickup and click the puzzle solved; all four are corpus entries.
`third-person` is not: `go limp` starts each body at the bone pose last drawn,
so where the figure falls depends on frame timing.

Packs carry the whole `Project`, so physics settings, components, constraints
and cooked collision data ship in `game.pack` with no extra format.
