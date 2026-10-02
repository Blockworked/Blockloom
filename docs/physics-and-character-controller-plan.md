# Physics, collision and character controller overhaul

Status: proposed implementation plan. Phase 0 (probes and the compatibility ledger) is done for 3D: see `physics-compatibility-ledger.md` and the `blockloom-physics-probes` crate. Its open rows (2D, joints, cooking, per-body solver overrides) carry into the phases that own them. Phase 1 (document and ownership foundation) is done: `blockloom-core/src/physics/`, the shell commands and the gate fixtures; the runtime still plays the legacy `Body` and migration is preview only. No runtime changes yet.

Date: 2026-10-01.

## 1. Product goal and completion contract

Make physics a complete component system that creators can put on any actor, independently of how it is drawn. Use Unity 6's Rigidbody, Collider and CharacterController authoring model and public behavior as the reference. A creator familiar with Unity should recognize the components, properties, force modes, collision events and movement APIs immediately.

Ship a reusable player stack as part of this work. Adding a player preset must produce working input, collision, locomotion, jumping and a camera without requiring a custom Rust script, an authored block loop or a collection of game-specific helper actors. The same movement foundation must accept AI and scripted commands.

This is the complete target, not a proposal to stop after separating two inspector cards. Every phase below is required for completion. Low-level physics, authoring, migration, block/script access, player dependencies, debugging and shipped-player verification are all deliverables.

### What "nearly exactly like Unity" means

- Match component composition, ownership, inspector concepts and documented API semantics where applicable.
- Keep Rigidbody, Collider and CharacterController distinct. A built-in player motor is an additional Blockloom component with its own settings.
- Pin the behavioral reference to Unity 6.0 documentation, rather than chasing an unspecified changing Unity release.
- Compare representative scenes against that reference. Publish an explicit compatibility table for every property and operation.
- Numerical trajectories, contact manifolds, mesh cooking and solver behavior need not be bit-identical to PhysX. Existing Blockloom physics uses Rapier. A differently named engine setting must never masquerade as an equivalent Unity feature.
- Missing required behavior is implementation work, not permission to silently drop the feature. Prove adapters in phase 0; use a bounded backend extension or revisit the backend if a required behavior cannot be supported correctly.
- Unity's 3D and 2D physics have separate contracts. Provide a corresponding complete 2D family and a Blockloom CharacterController2D; clearly identify the latter as an extension, because Unity's built-in CharacterController reference is 3D.

The component split follows [Unity's Rigidbody/collider composition](https://docs.unity.com/en-us/engine/6000.6/manual/physics-section/physics-overview/rigidbody/rigidbody-configure-colliders). The capsule movement baseline follows [Unity's CharacterController reference](https://docs.unity3d.com/6000.0/Documentation/Manual/class-CharacterController.html). All additional contracts below are proposed Blockloom requirements unless explicitly identified as Unity behavior.

## 2. Current implementation and the gaps to close

The following findings come from the current repository:

| Area | Current behavior | Required change |
| --- | --- | --- |
| Document | `scene.rs::Physics` stores body kind, gravity scale, rotation lock, material values, mass/density, trigger, one-way, controller toggle and eight-layer filtering together. `ActorComponent::Body` owns this value. | Separate body motion, collision geometry, surface response and character movement. |
| Geometry | `dim2.rs::collider_for` and `dim3.rs::collider_for` derive collision from `Visual`. Model collision uses a placeholder box; solid tilemaps use merged boxes. | Author independent primitive, mesh and generated geometry, including invisible colliders. |
| Body installation | Dimension-specific `attach_physics` requires both a collider and a body. Removing `Body` also removes the collider/controller. | A body can exist without a collider; a collider can exist without a body. |
| Controllers | A boolean installs `KinematicCharacterController::default()` on a kinematic actor. Existing movement effects accumulate translation. | An explicit controller component, configured dimensions, movement results, grounding and a reusable motor. |
| Contacts | Rapier start/stop messages feed `world::note_contact`; `engine.touching` is an actor-pair set and starts `Event::Collision`. Terrain has a special owner remap. | Stable collider identity, separate collision/trigger channels, contact payloads and compound-safe aggregation. |
| Queries | `physics_query.rs` tests sensor-snapshot boxes, balls and box parts and requires `has_body`. | Exact authoritative queries over all enabled colliders, including static and invisible geometry. |
| Timing | VM/scripts/effects run in `FixedUpdate`; contact relay and sensor publishing currently run in `Update`. | Physics events and gameplay snapshots have a defined fixed-step boundary, independent of render rate. |
| Parenting | `world::apply_parenting` applies parent transform deltas in `FixedPostUpdate`. | Compound ownership and body motion must be resolved before physics, without moving attached shapes a second time. |
| Authoring | `InspectorPanel.qml` exposes one Body card and a kinematic character-control switch. `Components::get` addresses by component name. | Repeatable Collider instances, property metadata, shape gizmos and dependency-aware player presets. |
| Existing dependencies | `input.rs` already has named actions, keyboard/gamepad bindings and starter actions. Camera, Joint, Brain, navigation and animation systems already exist. | Extend these foundations and integrate them; do not build competing input or AI systems. |

Audit terrain, tilemaps, fracture/debris, streamed models, particles, selection, navigation and runtime attach/detach while implementing the split. Each currently generated physics shape needs the same identity, filtering and lifecycle rules as a hand-authored collider.

## 3. Component and document architecture

### 3.1 Components

| Component | Responsibility | Cardinality per actor |
| --- | --- | --- |
| `Rigidbody` / `Rigidbody2D` | Motion, mass, forces, constraints, integration and sleeping. | At most one active physics motion owner. |
| `Collider` / `Collider2D` | Independent shape, local pose, surface material, trigger and collision/query filters. | Zero or more; every instance has a persistent ID. |
| `CharacterController` / `CharacterController2D` | Collision-constrained character displacement and movement results. Owns its primary capsule shape. | At most one; mutually exclusive with a Rigidbody on the same actor. |
| `CharacterMotor` | Reusable locomotion policy, gravity, jumping, crouching, platform carry and external movement. | At most one active pose-driving motor. |
| `PlayerInput` | Action-map and device assignment; converts input into movement intent. | At most one active input driver per motor. |
| `PlayerCamera` | First-person, third-person, top-down or side-view camera policy. | Optional; refers to the existing Camera owner. |
| `Joint` / `Joint2D` | Constraint between Rigidbody owners. | Repeatable with IDs. |
| Physics material asset | Shared friction, restitution and combination policy. | Referenced by colliders, with optional local overrides. |

An actor with only Place and Collider is static collision geometry. Rigidbody without Collider still integrates gravity and forces but has no physical contact surface. Look and Render never determine whether physics exists. Disabling drawing never disables collisions. Collider visibility in the editor is an overlay preference.

CharacterController needs no authored Rigidbody or additional capsule Collider. Internally the backend may use a kinematic body and shape; these are implementation details and cannot appear as a second creator-owned motion component. Extra trigger colliders may be attached to a controller actor for interactions. Extra solid shapes do not silently change the controller's swept capsule; the inspector explains this limitation.

`CharacterMotor` initially supports the controller backend and a separately tested dynamic Rigidbody backend. The latter requires a suitable solid collider and implements force-driven movement, not Unity CharacterController semantics. Authors choose the backend explicitly; presets resolve dependencies atomically.

### 3.2 Stable identity and editing

Introduce immutable `ComponentId` and `ColliderId` values. Generated terrain/tilemap/mesh parts have stable logical collider IDs plus internal subshape IDs. Clone references remap instance-local component IDs while keeping asset references. IDs survive save/load, undo/redo, reorder and rename.

Retain name-based lookup for singleton components. Repeatable components must be addressed by ID across commands, inspector selection, runtime effects, blocks, scripts and MCP. A name-based operation on an ambiguous repeated component returns a useful error; it never edits the first collider accidentally.

Keep saved specifications separate from runtime state. Velocity, contacts, ground support, sleeping, jump timers and transient ignored pairs are run state unless explicitly stored by gameplay. Play/Stop restore the document; scene editing cannot bake a simulated fall into Place.

Add versioned document types under a new `blockloom-core/src/physics/` module. Use property metadata for units, bounds, defaults, dimensions and dependency checks. Serialize finite values only. Validate mass > 0, valid shapes, valid material assets, legal body combinations, references and nonnegative tolerances before saving or applying runtime changes.

### 3.3 Units and scale

- 3D uses metres, kilograms, seconds, degrees in the editor and radians where the backend requires them.
- Preserve existing 2D authored coordinates. Add an explicit project pixels-per-metre conversion and show physical units in physics panels. Conversion of displacement, velocity, acceleration, density and inertia must share one utility.
- Capsule `height` means total end-to-end height, including caps. Backend half-segment length is `max(0, height / 2 - radius)`. Existing visual capsule height uses a different construction and must be converted during migration.
- Primitive colliders support local center and rotation independently of Look. Full actor scale/stretch affects collider dimensions under documented rules. Reject zero scale and unsupported shear with actionable errors.
- For nonuniform sphere/capsule scale, use explicit Unity-compatible scaling rules proved by fixtures. Arbitrarily turning a capsule into a hull changes character behavior and is not a silent fallback.
- Changing collider size on a live dynamic body recalculates automatic mass properties and wakes it. Mesh recooking is asynchronous and revision-tagged; the previous valid shape stays active until replacement is ready.

## 4. Collider system

### 4.1 Geometry and capabilities

Required 3D shapes: box, sphere, capsule with selectable axis, convex hull, triangle mesh and terrain heightfield. Required 2D shapes: box, circle, capsule, polygon, edge/chain and composite tilemap geometry. Provide collider-only actors, explicit collision mesh assets and primitive fitting tools.

Every collider has an ID, enabled flag, name, shape, local center/rotation, material reference, trigger flag, layer selection/inheritance, include/exclude overrides, override priority, contact offset and query participation. Advanced settings are collapsed by default.

The initial shape is a saved shape. "Fit to visual" is an explicit authoring command. "Generated from visual" is a separate opt-in provider, necessary for legacy compatibility and procedurally changing tilemaps/models. Replacing Look does not overwrite manually authored shapes.

Use static triangle meshes/heightfields for scenery. Require convex geometry or convex decomposition on dynamic bodies; validate kinematic mesh support against the backend and Unity reference. Reject unsupported concave combinations before Play. Support holes, seams, chunk boundaries, face orientation and query backface policy. Two-sided rendering does not imply two-sided collision.

Mesh cooking includes welding, degenerate triangle cleanup, validation and optional convex decomposition. Cache by source hash, transform policy, cooking settings, cooker version and target format. Show progress, cancellation, memory/triangle cost and errors. Store reproducible cooked data with builds so shipped games need no cooker or network. Define decomposition hull/vertex limits and report approximation error.

Animated or skinned meshes do not recook every frame automatically. Use authored primitive/hull attachments to bones or explicit expensive recooking. Bone hitboxes have collider identity and can be used as triggers. Collider cooking and shape fidelity never follow render LOD or visual batching decisions.

### 4.2 Compound ownership

Colliders on an actor attach to its Rigidbody, or the nearest ancestor Rigidbody if the actor has none. A nested Rigidbody begins a separate physics body. A collider without a body ancestor is static. Compound geometry is authored from shapes on one actor or shapes on child actors; all forms use the same ownership resolver. This follows [Unity's compound collider composition](https://docs.unity3d.com/6000.0/Documentation/Manual/create-compound-collider.html).

Create a `PhysicsOwnership` table mapping collider ID to collider actor, body actor, local pose and backend handle. Do not infer ownership from renderer parentage. Refactor the current post-physics parent-delta pass so it excludes physics-owned compound shapes. Collider-only child transforms are resolved before backend synchronization; dynamic child bodies remain independently simulated in world space.

Reparenting, adding/removing a body and changing a child's pose update ownership in one fixed-step transaction. Preserve world pose unless the operation explicitly requests a local placement. Recalculate mass, close removed contact pairs and synchronize queries. Ignore collisions between shapes of the same body. A body moving carries its colliders exactly once. A parented dynamic body must not also be carried by the generic parenting pass.

### 4.3 Materials

Author reusable 3D materials with static friction, dynamic friction, bounciness and independent friction/bounce combine settings: Average, Minimum, Multiply and Maximum. Mixed-mode priority is Maximum > Multiply > Minimum > Average. These concepts follow [Unity's Physics Material reference](https://docs.unity3d.com/6000.0/Documentation/Manual/class-PhysicsMaterial.html) and [combine-mode rules](https://docs.unity3d.com/6000.0/Documentation/Manual/collider-surfaces-combine.html). Give 2D materials a separately specified friction/bounce contract; 3D material options are not automatically Unity 2D equivalents.

Rapier's ordinary coefficient is not sufficient evidence of separate static/dynamic friction support. Phase 0 must prove an appropriate contact/solver adapter, including stick/slip transitions, or identify the necessary backend work. Mapping both controls to one coefficient is unacceptable. Keep controller traction/slope policy separate from contact material friction.

Provide Default, Ice, Rubber and No Bounce materials. Migrated bodies retain their existing coefficients through generated material values, rather than adopting new defaults.

## 5. Rigidbody system

### 5.1 Inspector and state

Expose mass; automatic/custom center of mass and inertia tensor; linear/angular damping; Use Gravity plus Blockloom gravity scale; Is Kinematic; collision participation; interpolation; collision detection; translation/rotation axis constraints; maximum linear/angular velocity; sleep threshold; solver iteration overrides; and maximum depenetration velocity.

Public state includes linear/angular velocity, world center of mass, effective inertia, sleeping and enabled state. Body mass is body-owned. Density-derived mass is an explicit alternate authoring mode that aggregates eligible compound shapes. Triggers do not add mass by default. Collider edits cannot accidentally multiply an explicit mass by the number of shapes.

Rigidbody2D additionally exposes its own Dynamic/Kinematic/Static body type, Simulated flag, 2D constraints and applicable 2D detection/interpolation settings. Validate its behavior against Unity Rigidbody2D independently. A 3D static collider normally has no Rigidbody; the old Static Body label remains only in migration/compatibility handling.

The familiar 3D property baseline comes from [Unity's Rigidbody reference](https://docs.unity3d.com/6000.0/Documentation/Manual/class-Rigidbody.html); density authoring, aggregate mass rules and the state synchronization contract above are Blockloom requirements.

### 5.2 Operations and pose ownership

Support AddForce, AddForceAtPosition, AddTorque, velocity reads/writes, point velocity, MovePosition, MoveRotation, teleport, sleep/wake and reset automatic mass properties. All operations target the body owner, not an arbitrary child collider.

The 3D [Unity ForceMode](https://docs.unity3d.com/6000.0/Documentation/ScriptReference/ForceMode.html) semantics are fixed:

| Mode | Linear velocity change for a fixed step of duration `dt` |
| --- | --- |
| Force | `force * dt / mass` |
| Acceleration | `acceleration * dt` |
| Impulse | `impulse / mass` |
| VelocityChange | Direct delta velocity, independent of mass and `dt`. |

Torque uses the analogous inertia-aware rules. Forces at a position also create torque about the world center of mass. Define zero-force wake policy and force lifetime: a submitted continuous force applies for the specified fixed tick, not indefinitely. The Unity-compatible 2D profile exposes Force and Impulse; optional Acceleration and VelocityChange helpers are labeled Blockloom extensions.

Dynamic bodies own their simulated poses. Kinematic targets go through the backend target API and contribute contact velocity. Static geometry moved by a script is synchronized explicitly; repeated moving scenery should use a kinematic Rigidbody. Teleport is a separate unswept operation with a choice to preserve or reset velocity, and resets interpolation history.

Old position/rotation/glide/tween blocks need explicit adapters. Preserve legacy teleport-style effects for migrated projects; new physics-aware movement blocks use the documented operation. Warn when animation, a motor, parenting and a body simultaneously try to own the same pose. Do not resolve competing pose writers by schedule accident.

### 5.3 Collision detection and solver robustness

Expose Discrete, Continuous, Continuous Dynamic and Continuous Speculative for 3D only after their effective semantics are proved. Unity distinguishes sweep-based and speculative detection, including differences for rotating bodies and moving targets. Use [Unity's mode-selection reference](https://docs.unity3d.com/6000.0/Documentation/Manual/choose-collision-detection-mode.html) as the behavioral fixture source.

Rapier CCD/soft CCD flags are implementation candidates, not a complete mapping. Phase 0 tests static/dynamic/kinematic targets, primitive and convex geometry, rotational tunneling and trigger sweeps. If pair eligibility or rotational behavior differs, implement the missing policy and expose documented restrictions. Never present several names that activate the same backend flag without distinctions.

Provide configurable fixed rate, bounded substeps, contact/rest tolerances, solver position/velocity iterations, restitution threshold and sleep behavior. Presets are Stable, Balanced and Fast, with settings visible. Physics quality is independent of visual adaptive quality. Do not skip simulation steps, drop colliders or change gameplay tolerances because rendering is over budget.

Overload policy bounds catch-up work and reports lost simulation time. Physics state must remain finite under extreme mass ratios, large impulses and overlapping spawn positions. Bound depenetration; report unrecoverable overlap. There is no claim of cross-platform bitwise determinism.

## 6. Filtering, collision events and queries

### 6.1 Layers and pair rules

Replace eight unnamed slots with 32 named layer slots and a symmetric project collision matrix, with separate 2D and 3D settings. Preserve old membership and mutual-mask behavior during migration. Support per-collider include/exclude overrides and pair ignores, with documented precedence and stable-ID addressing. Pair ignores clear when an endpoint is deleted and can be explicitly restored after disable/enable.

Filters are resolved centrally for contacts, triggers, controller sweeps and gameplay queries. A query can intentionally use a different mask, but this must be requested. Provide explicit trigger inclusion (`UseGlobal`, `Ignore`, `Include`), self exclusion, body exclusion, enabled-state handling and optional backface policy.

For ordinary 3D collider pairs, the default Unity-style event matrix is:

| Pair | Solid collision enter/stay/exit | Trigger enter/stay/exit if either shape is a trigger |
| --- | --- | --- |
| Static / static | No | No |
| Static / kinematic | No | Yes |
| Static / dynamic | Yes | Yes |
| Kinematic / kinematic | No | Yes |
| Kinematic / dynamic | Yes | Yes |
| Dynamic / dynamic | Yes | Yes |

Triggers do not produce solid contact response. Kinematic solid bodies are not automatically stopped by static or kinematic geometry; creators needing collision-constrained movement use the controller or a sweep API. Controller pairs use the dedicated controller hit/trigger contract rather than pretending every controller is a dynamic Rigidbody. Test the 2D matrix separately, including its full kinematic-contact option.

This table is based on [Unity's collider interaction matrix](https://docs.unity3d.com/6000.0/Documentation/Manual/collider-types-interaction.html). The 2D full-contact option follows [Rigidbody2D.useFullKinematicContacts](https://docs.unity3d.com/6000.0/Documentation/ScriptReference/Rigidbody2D-useFullKinematicContacts.html): it adds kinematic/static and kinematic/kinematic callbacks without automatic collision response. Compatibility mode may keep legacy trigger/static touch behavior; new projects use the stated matrix.

### 6.2 Contact lifecycle and payload

Track unordered collider pairs, with actor/body ownership attached. Maintain actor-level touching as a reference count over active collider pairs. If two compound shapes touch a wall and one separates, touching stays true until the final pair ends.

Collision events contain phase, tick, both actor IDs, both body IDs, both collider IDs, normal orientation, contact points/separations, relative velocity and aggregate impulse. Trigger events contain identity/phase but no fabricated contact force. Controller hits have their own movement payload. Expose payloads to event-local blocks and scripts, including safe nested-event access.

Deliver Enter once, Stay at most once per eligible fixed tick and Exit once. Stable ordering is tick, pair ID and phase; events generated in tick N are available to gameplay in tick N+1. Multiple fixed steps during one rendered frame preserve every transition. Sleeping contacts follow a documented Unity-compatible Stay policy and remain visible to touching reporters.

Explicitly handle disable, destroy, detach, layer change, ownership change, shape replacement, teleport, scene unload and streaming unload. Blockloom emits synthetic Exit with a reason for removed active pairs, even where Unity lifecycle callback behavior differs. Dead endpoint IDs remain readable but cannot be mutated. Scene reset clears queues without delivering old events into the new world. Geometry replacement must not flood identical logical contacts with fake enter/exit cycles.

Keep legacy `when I touch` and `touching?` through adapters over actor aggregation. Introduce separate collision and trigger hats plus collider-selective events. Deduplicate delivery when collider owner and body owner are the same actor; child-level and body-level listeners can opt into their respective scopes.

### 6.3 Authoritative queries

Provide raycast, linecast, box/sphere/capsule casts, overlaps, closest point, bounds, distance/penetration queries and body sweep tests. Add the corresponding 2D operations. Support closest-hit, all-hit and bounded-buffer variants, with explicit overflow reporting and reusable allocations.

Hit records contain actor/body/collider/subshape identity, point, normal, distance and fraction, plus triangle/barycentric data when supported. Sort by distance then collider ID for stable ties. Define starting-inside, zero-distance, zero-direction and trigger behavior. Queries must include invisible and collider-only actors.

Do not leave a second approximate physics engine in `physics_query.rs`. Preserve cheap geometry utilities for explicitly approximate visual effects, but gameplay queries use the same shapes and filters as simulation. Define two access modes:

1. Synchronous host queries for scripts and controller internals, against the synchronized fixed-step world.
2. Block query instructions that issue a request, suspend the strand, and resume with a result bound to a tick. Pure reporter expressions read a stored result or contact snapshot; they do not secretly block or query a different world.

Compiled blocks use the same request/result contract. Scene editing uses a separate query context for the authored preview. Streaming queries expose incomplete-region status rather than claiming empty unloaded space is safe.

## 7. CharacterController

### 7.1 Unity-compatible low-level contract

Required 3D properties: enabled, center, radius, total height, slope limit, step offset, skin width, minimum move distance, Detect Collisions and Enable Overlap Recovery. Expose velocity, collision flags (`Sides`, `Above`, `Below`) and `isGrounded` from the last completed move. Additional ground details are Blockloom extensions.

`Move(displacement)` takes world-space displacement and applies no gravity. `SimpleMove(velocity)` takes horizontal velocity in units per second, ignores the vertical input component and applies gravity. These distinctions follow [Unity Move](https://docs.unity3d.com/6000.0/Documentation/ScriptReference/CharacterController.Move.html) and [Unity SimpleMove](https://docs.unity3d.com/6000.0/Documentation/ScriptReference/CharacterController.SimpleMove.html). Define how project gravity maps to the vertical axis; the Unity profile remains Y-up.

Implement slide along walls, bounded stair stepping, slope rejection, skin tolerance, overlap recovery and ceiling/floor flags. The controller is not accelerated by incoming rigid-body forces and does not automatically push bodies in its Unity-compatible profile. Optional pushing belongs to the motor or a hit callback. Wall sliding and slope behavior follow the [Unity component reference](https://docs.unity3d.com/6000.0/Documentation/Manual/class-CharacterController.html).

`OnControllerColliderHit` equivalent events contain collider/body IDs, point, normal, attempted motion direction and length, effective displacement and tick. They arise from actual movement hits, not merely persistent nearby overlaps. Reference: [Unity controller hit callback](https://docs.unity3d.com/6000.0/Documentation/ScriptReference/CharacterController.OnControllerColliderHit.html).

Each movement command returns a result after execution. Execute multiple low-level moves sequentially in submission order, not as an indistinguishable summed vector. A script can synchronously read its last move result through a bounded host call; block instructions suspend until that move has executed. The motor submits one composed move per tick. Raw moves and motor moves cannot silently compete.

Expose total effective velocity separately from support-relative motor velocity. Zero-move grounding, thresholds and parameter edits have explicit tested semantics. A new controller's ground result is unknown until the first move/probe; do not invent grounded state from its authored position.

### 7.2 Backend adaptation and robustness

Start with Rapier's character move/shape-cast facilities and autostep primitives. [Rapier's controller guide](https://rapier.rs/docs/user_guides/bevy_plugin/character_controller/) establishes these facilities, but the API used must be checked against the workspace's pinned Rapier revision `09ca067a`.

Implement a runtime adapter with these stages:

1. Validate the capsule, orientation, filters and ownership; synchronize nearby geometry.
2. Recover initial overlap within a capped iteration count and displacement budget.
3. Sweep requested displacement with skin-aware tolerances and collect hits.
4. Attempt a step only when support, clearance and landing tests allow it; sweep up, forward and down.
5. Resolve wall/corner sliding without repeatedly adding upward energy.
6. Classify final ground, ceiling and side contacts; reject surfaces over the slope limit.
7. Commit effective movement, interpolation history and the move result.

Rapier offset is not automatically Unity skin width. Test clearance, allowed penetration and seam traversal before deciding the mapping. Probe ground using capsule-foot geometry and manifold normals; a single center ray is insufficient at edges. Keep unwalkable slope rejection separate from optional motor sliding.

Bound recovery/sweep iterations and allocations. Handle thin walls, corners, tiny ledges, large displacement, near-zero moves, wedged capsules, overlap spawn, height changes and crouch headroom. Reject impossible dimensions, negative widths and step offset greater than height. Continuous translation does not imply swept arbitrary rotation; the standard character capsule stays aligned to its up axis.

Default Detect Collisions and Overlap Recovery are enabled. Distinguish interaction with incoming bodies from the movement sweep's obstacle policy; prove the Unity reference behavior instead of interpreting Detect Collisions as an unexplained noclip switch.

Provide CharacterController2D with the same result and motor integration model, appropriate XY geometry, configurable up direction, one-way platforms and drop-through. Its detailed collision contract is Blockloom-defined and tested independently.

## 8. Reusable player and movement dependencies

### 8.1 CharacterMotor

The motor is the reusable gameplay layer. It owns locomotion state and uses the low-level controller or the explicitly selected Rigidbody motor backend. Input, AI, blocks and scripts all submit the same typed `MovementIntent`.

Required configuration:

- Movement speed, sprint speed, grounded acceleration/braking, air acceleration/control and facing/turn speed.
- World-relative, actor-relative or camera-relative input, normalized diagonal input and analog speed.
- Gravity scale, terminal fall speed, ground stick/snap distance and optional steep-slope sliding.
- Jump height, variable jump release, maximum jump count, coyote time and buffered jump input.
- Standing/crouching dimensions, crouch speed and swept headroom checks before standing.
- Moving-platform translation/rotation carry, departure velocity inheritance and explicit crushing/blocking behavior.
- External velocity, knockback, additive impulses and optional body pushing with mass/force limits.
- One-way platform filtering and timed drop-through for 2D; ignore only the selected support pair and restore when clear.
- Enabled state, player/AI/scripted ownership, cutscene override and optional root-motion intent.

Use `jump_speed = sqrt(2 * abs(gravity_along_up) * jump_height)` for constant-gravity profiles. Handle zero gravity explicitly. Coyote and input-buffer clocks run in simulation time and pause with the game. Collision results cancel blocked velocity along their normals; ceiling contact cancels upward speed.

Publish grounded/rising/falling/landing/crouching state, actual speed, desired speed, jump availability, support ID/normal, slope angle and ground distance. Expose jump, land, leave ground, head hit and stance-changed events. Provide animation bindings through the existing animation system, without requiring an animator graph for basic movement.

Support motion is measured at the contact point, including platform rotation. Sweep carried motion so a platform cannot transport the player through a wall. Do not parent the player to its support as an invisible workaround. Define platform teleport policy, support disappearance and detachment. Crush policy can emit an event and stop movement or apply configured damage; it must not launch the capsule with an unbounded correction.

Pushing is opt-in, so the low-level controller stays compatible with Unity. Apply bounded impulses to dynamic body owners only; do not push static scenery, triggers or apply downward push to the support. Rigidbody locomotion must also handle stairs, slopes, grounding and jumps through its own tested force/contact policy; it cannot be a controller with a fake mass field.

### 8.2 PlayerInput and action maps

Extend `blockloom-core/src/input.rs`. Add typed scalar/vector2 actions, composites, action maps, binding processors, dead zones, sensitivity and device assignment. Existing named button/directional actions remain valid.

Supply Move, Look, Jump, Sprint, Crouch and Interact mappings for keyboard/mouse, gamepad and optional touch controls. Rebinding and saveable per-player overrides are required. UI focus, pause menus, lost preview focus and pointer-lock loss must release/cancel held actions without causing a jump or movement burst on resume.

Render-frame input edges are latched and consumed once by the relevant fixed tick. Held analog values persist across catch-up ticks; mouse deltas are distributed or consumed under a defined policy, not replayed in full for each tick. Support local player indices and independent device assignment without assuming a single global player. Split-screen rendering can remain a separate feature; device/input ownership cannot.

### 8.3 PlayerCamera

Extend the current Camera component and pointer-lock path. Provide first-person pitch/yaw with limits; third-person orbit/follow with collision avoidance; top-down follow; and side-view follow with dead zones/look-ahead. Include sensitivity, smoothing, zoom distance, offsets and optional camera-relative movement.

Third-person avoidance uses a swept camera volume and explicit layer/trigger filters, excluding the player's own shapes. Smooth camera motion uses interpolated poses while movement basis uses a stable sampled orientation. First-person view anchors track standing/crouching height. Camera/pointer-lock policy works in embedded, process, native-player and browser modes.

PlayerCamera selects or configures the existing Camera owner; it never silently creates a competing world camera. Preserve the current single-world-camera restriction until viewport support expands.

### 8.4 Presets and reusable assets

Ship first-person 3D, third-person 3D, top-down 3D, platformer 2D and top-down 2D presets. A preset transaction installs required components, default bindings, a compatible collision shape/controller, motor and camera. Missing compatible dependencies are added; incompatible existing motion owners produce a concrete conversion choice with an undoable preview.

Provide a dependency-free visual capsule/sprite so every preset works before importing art. Models and animation are optional. Default scale and collision dimensions are displayed and editable.

Allow creators to save motor/controller/input/camera configurations as reusable player profiles and instantiate them in another project. Copy required physics materials and binding assets with explicit conflict resolution. Profiles have versioned defaults, instance overrides and a reset-to-profile operation; updating a profile does not overwrite intentional overrides.

Acceptance: create an empty project, add a ground Collider, add a Player preset, press Play, then move and jump successfully. No Rust source, manual physics helper scripts or authored movement blocks are necessary.

## 9. Joints and integration with other systems

Expand existing Fixed/Hinge/Rope support into useful 3D fixed, hinge, spring, distance/rope and configurable constraints, and corresponding 2D fixed, hinge, distance, spring, slider and wheel constraints. Expose both anchors, axes, limits, motors, spring/damping, break force/torque, connected-body collision and break events. Stable joint IDs allow multiple constraints on one actor.

Joint endpoint validation resolves body owners, supports world anchoring and safely handles deletion, disable and reparenting. Test ragdolls, doors, pendulums, ropes and platforms. Advanced articulation and vehicle simulation are separate future systems; they do not replace any required Rigidbody/Collider behavior in this plan.

Integrations required for completion:

- Terrain/tilemaps: stable chunk/subshape IDs, seam-safe movement, edits with bounded cooking and contact replacement.
- Model streaming: load collision at gameplay distance independently of visual LOD. Prevent entry into unloaded required collision regions through a defined blocking/load policy.
- Fracture/debris: inherit filters/materials, derive new mass/inertia and preserve momentum when splitting.
- Buoyancy: apply forces to body owners and use body mass/volume rules; do not require Look for eligibility.
- AI/navigation: generate movement intent for the same motor; navigation obstacle geometry comes from colliders, not renderer bounds.
- Animation/root motion: submit motor intent or an explicit kinematic target. Dynamic ragdoll handoff preserves poses and velocities.
- Particles/VFX: explicitly choose approximate visual collision or authoritative queries; both document their cost and fidelity.
- Cloning/runtime attach: clone collider definitions and remap identities; install/remove dependencies transactionally at a tick boundary.
- Scene editing: show authored collision geometry without simulation; selection remains possible for invisible collider actors.

## 10. Runtime schedule and public APIs

### 10.1 Fixed-step contract

Create named physics sets rather than relying on a long tuple chain. Check actual Bevy/Rapier synchronization points against the pinned sources before installing the ordering.

1. Latch input and restore the previous authoritative simulation pose.
2. Publish the tick-start gameplay snapshot and dispatch events/results from the previous tick.
3. Run VM, scripts, AI and motor intent producers.
4. Apply spawn/despawn/component transactions and resolve physics ownership.
5. Flush ECS commands and synchronize body/collider definitions and query geometry.
6. Execute ordered query/controller requests and motor movement; synchronize targets and changes needed by the solver.
7. Apply force/impulse queues and advance the physics solver.
8. Read back authoritative body poses, controller results and collider-pair contacts.
9. Queue events/results, publish end-of-tick state and record interpolation poses.

Support carry must read valid platform motion for this tick, with an explicit strategy for predicted kinematic targets and solved dynamic supports. Add a post-solver correction pass only if it is swept and its query world is synchronized. A stale target transform is not valid platform motion.

`Update` handles render interpolation, editor status throttling and UI, not contact lifecycle. Pause stops physics, motor timers and gameplay query progression. Scene inspection continues through its own editor context. Reset/unload cancels outstanding requests with a typed cancellation result.

Bound all request queues and per-tick query budgets. Gameplay queries required for controllers have reserved capacity; overflow produces diagnostics rather than a missing collision. Snapshot IDs and result ticks identify exactly which world a read describes.

### 10.2 Blocks, scripts, shell and MCP

Expose all core settings and operations across supported gameplay entry points. Beginner palettes show common operations; advanced groups expose collider IDs, detailed hits, body properties, query buffers and movement results.

Add block families for body forces/motion/state, collider enable/shape/material/filter, collision/trigger events, queries/results, controller moves/results and motor/player intent/state. Use existing `InstructionKind`, `fields.rs`, VM steps/effects, codegen and `Blocks.qml` registration conventions.

Extend script `abi.rs`/`prelude.rs` with body, collider, query and controller APIs. Use bounded numeric records and text/collection handles appropriate for both native and wasm hosts. Bump `ABI_VERSION` when the boundary changes; invalidate cached libraries. Native pointers or Rapier entity handles never escape to scripts. Nested callbacks cannot retain borrowed solver data.

Add component-instance commands, shape edits, cooking operations, layer/material operations, preset/profile creation and physics inspection to `commands.rs`, `dispatch.rs` and `shell.rs::COMMANDS`. Update `mcp/src/registry.ts` for new prose types. Shell/MCP expose the same validation/errors as QML. Bump `PROTOCOL_VERSION` for changed messages and tag replies with revision/tick/world generation.

VM, compiled block programs and Rust scripts submit the same effects and query requests. Verify logical outcome parity; compiler selection cannot change contact routing or controller results.

## 11. Editor and diagnostics

Provide separate Rigidbody, repeatable Collider, CharacterController, CharacterMotor, PlayerInput and PlayerCamera cards. Add-component search describes dependencies and conflicting pose owners. Inspector fields show units and effective inherited values. Common workflows use controls and asset pickers, not JSON fields.

Collider editing includes center/radius/size/height/axis handles, mesh preview, fit-to-visual, duplication and collider list selection. A drag previews a draft and commits one undo transaction; Escape cancels. Draw trigger volumes distinctly and show body ownership, center of mass and controller foot/head clearance.

Add a Physics Debug view with collision shapes, AABBs, layer exclusions, contacts/normals, sleeping bodies, CCD sweeps, controller attempted/effective motion, support probes, step attempts and rejected slopes. Clicking a contact resolves both collider IDs and body owners. Support pause, single fixed-step and reproducible fixture replay.

Profiler rows include body/collider counts, active/sleeping bodies, pair/contact counts, solver/CCD/query/controller milliseconds, cooking time/cache memory and query/event overflow. Record bounded traces with tick, input, movement commands and settings; traces are for diagnosis, not a promise of deterministic network replay.

Show actionable diagnostics for conflicting components, missing collision assets, unsupported shape/body combinations, tiny skins, scale/shear problems, invalid layer references and expensive recooking. The shipped runtime logs the same fatal configuration errors instead of quietly making objects non-colliding.

## 12. Migration and compatibility

Introduce an explicit physics schema version and compatibility profile. Load old projects without writing them until an intentional save/migration. Preserve originals through the existing backup flow or add a migration backup if no such flow exists. Migration is idempotent and covers every scene and reusable actor asset.

| Legacy state | Migration |
| --- | --- |
| No Body / Body None | No Rigidbody or Collider, preserving non-colliding decoration. |
| Static Body | Collider-only geometry with converted trigger/material/filter values. |
| Dynamic Body | Rigidbody plus collider provider matching the previous visual-derived shape. |
| Kinematic Body | Kinematic Rigidbody plus the matching collider provider. |
| Character-control toggle | Explicit legacy controller profile preserving old shape/movement behavior; offer an undoable conversion to the new capsule controller/motor. |
| Eight layers and masks | Preserve slot mapping and old per-object masks using compatibility overrides; do not lose asymmetric masks when creating the project matrix. |
| Friction/restitution/mass/density | Material values plus the corresponding explicit or density mass mode. |
| Rotation lock | Convert to the appropriate dimension's locked rotation axes. |
| One-way static 2D body | Collider2D plus one-way surface policy/effector with matching old behavior. |
| Body attach/detach and old touch blocks | Compatibility adapter preserves whether the old operation added/removed both body and shape and how touches were aggregated. |

Legacy auto-shapes must keep following Look changes where that was observable behavior. Offer "Make independent colliders" as an explicit upgrade. Do not reinterpret a model placeholder as an exact mesh, change capsule dimensions or change collision callbacks simply by opening a project.

Retain old block serialization and shell commands as aliases until a documented deprecation migration exists. Keep imported/unknown version errors clear. New projects and new presets use the new model; an old project may opt in per actor and then commit a project-wide upgrade after previewing differences.

## 13. Implementation phases and gates

Estimates should be made after phase 0. This spans core, runtime, authoring and build formats; a fixed date before the backend probes would be unreliable. Phases are dependency ordered and each produces a reviewable fixture set.

| Phase | Deliverables | Exit gate |
| --- | --- | --- |
| 0. Behavioral and backend probes | Compatibility ledger; pinned-source audit; Unity reference fixtures; prototypes for compounds, skin/step behavior, CCD modes, material stick/slip, exact queries and fixed-step result delivery. | Every required feature has a proved mapping or a concrete backend-extension design and test. Resolve backend choice before broad schema work. |
| 1. Document and ownership foundation | Versioned specs, stable repeated-component IDs, property metadata, validation, editing transactions and legacy migration; new collider/body registry. | Save/load/undo/clone/reparent preserve identities and legacy scenes; invisible static colliders and body-without-shape fixtures pass. |
| 2. Bodies, geometry and solver policy | Independent body/collider installation, primitive shapes, compounds, forces/mass/inertia, constraints, sleep, material response, filtering and CCD. | Body/shape/mode fixtures pass, including multiple collider removal and unsupported-combination errors. |
| 3. Events, queries and cooking | Fixed-step event lifecycle, complete payloads, exact queries with VM/script/codegen adapters, mesh/terrain/tilemap cooking/cache and streaming integration. | No render-rate-dependent missing events; query hits match physical surfaces; builds contain required collision data. |
| 4. Character controllers | Explicit 3D/2D controller specs, sequential Move/SimpleMove, flags/hits, skin/recovery, steps/slopes and overlap/ceiling handling. | Obstacle-course and Unity low-level behavior fixtures pass without motor assistance. |
| 5. Reusable motors and input | Controller and dynamic-body motor backends, intent API, gravity/jump/crouch/platform/one-way/push policy, action maps/device routing/rebinding. | Movement fixtures pass across frame/fixed rates, pause/focus loss, input devices and AI drivers. |
| 6. Cameras, presets and authoring | Camera policies, player profiles/presets, component inspectors, collision gizmos and dependency conversion. | Empty-project player acceptance passes entirely through QML and shell/MCP, with undoable setup. |
| 7. Joints and full integration | Expanded constraints, break/motor support, fracture/buoyancy/AI/animation/streaming integration, debug tools and profiler. | Integration fixtures and bounded-performance tests pass; no duplicate pose owners or render-dependent collision LOD. |
| 8. Compatibility and shipping | Upgrade previews, documentation, sample games, native/web packs, platform verification and complete API coverage audit. | Every required compatibility-ledger row is implemented and verified; all product acceptance scenarios pass. |

Do not call a phase complete with a placeholder that is only available through Rust or only works in the editor. Backend foundations can land before their QML polish, but product completion requires usable authoring and shipped-game support.

## 14. Verification and acceptance suite

### 14.1 Automated verification

- Core tests: validation, serialization/versioning, migration idempotence, component identity, layer precedence, force units and profile overrides.
- Headless runtime fixtures: real Rapier body/contact/query/controller tests in both dimensions with no renderer required.
- VM/codegen/script fixtures: same forces, hit results, compound contact aggregation and controller outcomes through all three execution paths.
- QML tests: repeated component selection, collider property edits, atomic preset dependencies, draft cancel and undo/redo.
- Shell/MCP tests: new schemas, aliases, ID addressing, errors and preset creation.
- Golden migration projects: old shapes, triggers, parenting, character toggles, model placeholders, tilemaps and runtime attach/detach.

Record scene scale, fixed rate, substeps and tolerances with every fixture. Compare positions, velocities, flags and event timelines using scale-aware tolerances, not screenshots alone. Unity comparisons establish public behavior and pass/fail thresholds; Rapier-specific numerical differences are recorded explicitly.

Run the repository's workspace tests and required QML/MCP tests for changed surfaces. Build the entire workspace for native verification. Prepare patched dependencies before direct Cargo builds and prune the target cache afterward. Validate wasm with `just web-check`, then build a web player/sample and run `just web-smoke`; native-player behavior must also be tested outside the editor. Track Windows/macOS checks as required release gates when their runners are available.

### 14.2 Required acceptance scenarios

| Scenario | Observable pass condition |
| --- | --- |
| Invisible wall and empty dynamic actor | Wall blocks bodies without Look; actor without Collider falls without contacts. |
| Compound prop | Child shapes use one body, preserve local pose, contribute correct mass and never self-collide. |
| Nested dynamic body | Parent motion does not move it twice; its colliders belong to its own body. |
| Multiple contacts | Removing one of several touching shapes preserves actor touching; final removal emits one aggregate exit. |
| Trigger lifecycle | Correct enter/stay/exit and no impulse; disable/delete/filter changes leave no stale pairs. |
| Force modes and constraints | Mass and fixed-rate changes produce the specified force/impulse results; locked axes remain locked. |
| Ice, rubber and bounce | Static/dynamic friction and mixed combine modes match the documented policy with stable stack behavior. |
| Fast projectile and rotating obstacle | Supported CCD modes prevent misses in their stated range and report known restrictions. |
| Mesh/terrain/tile seams | Physics queries match collision surfaces; controller traversal does not snag at chunk/triangle seams. |
| Steps/slopes/corners | Below-limit steps/slopes pass; above-limit geometry blocks; wall/corner sliding does not climb or add energy. |
| Controller API | Move adds no gravity; SimpleMove applies it; flags and hit results correspond to the actual completed move. |
| Jump/ceiling/ledge | Buffered/coyote/variable jumps work, ceiling cancels upward velocity and leaving a ledge clears support. |
| Crouch | Feet stay anchored and standing is refused under insufficient clearance. |
| Moving/rotating platforms | Player stays on support without jitter, is swept against walls and inherits configured departure velocity. |
| Overlap/crush | Recovery is bounded, state stays finite and unrecoverable configurations emit diagnostics. |
| One-way/drop-through | Player passes from below, lands from above and drops only through the chosen support. |
| Input and camera | Keyboard/gamepad/touch movement and focus/pointer-lock/UI transitions produce no duplicate or stuck inputs. |
| Timing | At 30/60/144 render FPS and 30/60/120 fixed Hz, input edges and event counts remain correct; movement stays within stated tolerances. |
| Streaming/fracture/AI | Collision remains available independently of visual LOD, debris preserves momentum and AI uses the same movement rules. |
| Creator workflow | Every shipped preset works in an empty project and can be saved/reused without custom code. |
| Shipped games | Embedded editor, process runtime, native player and web player agree on the logical fixture outcomes. |

Performance fixtures cover sparse static worlds, dense contact stacks, many compound bodies, simultaneous controller movement and collider recooking. Phase 0 establishes hardware-specific budgets and scene sizes; release gates reject regressions against those baselines. Queries use spatial acceleration and work proportional to candidates, not a scan of every actor per character. Report memory and worst-frame/tick time as well as averages.

## 15. File-level work map

| Surface | Expected work |
| --- | --- |
| `blockloom-core/src/physics/` (new) | Specs, IDs, property metadata, materials, filters, requests/results and validation. |
| `blockloom-core/src/components.rs`, `scene.rs`, `project.rs`, `pack.rs`, `wire.rs` | Component model, compatibility conversion, versions and asset references. |
| `blockloom-core/src/input.rs`, `sense.rs`, `physics_query.rs` | Typed input/maps, authoritative results, compatibility adapters and removal of misleading approximate gameplay queries. |
| `blockloom-core/src/blocks.rs`, `fields.rs`, `value.rs`, `vm/`, `codegen/` | Physics/controller/motor operations, events, suspended queries and reporters. |
| `blockloom-core/src/script/abi.rs`, `prelude.rs` | Versioned body/collider/query/movement API across native and wasm. |
| `blockloom-runtime/src/physics/` (new), `dim2.rs`, `dim3.rs` | Shared ownership/lifecycle policy plus dimension-specific Rapier adapters. |
| `blockloom-runtime/src/character/` (new), `world.rs`, `engine.rs`, `lib.rs` | Controllers, motors, named schedule sets, snapshot/event queues and pose ownership. |
| Runtime terrain/tiles/model/streaming/destruction/AI/animation modules | Unified generated colliders, cooking and intent integration. |
| `blockloom-app/src/commands.rs`, `dispatch.rs`, `shell.rs`, `state.rs` | Transactional editing, profiles/presets, cooking state and diagnostics. |
| `blockloom-protocol/src/lib.rs` | Versioned physics debug/status and editor shape-edit messages. |
| `blockloom-qt/qml/InspectorPanel.qml`, `Blocks.qml`, scene editor/runtime gizmos | New cards, repeated IDs, block palette, physics settings and debugging. |
| `blockloom-qt/build.rs` | Register any new QML files. |
| `mcp/src/registry.ts` and tests | New command schemas and repeated-component addressing. |
| Build/pipeline scripts, sample projects and docs | Collision cooking/packing, reusable profiles, reference playground and creator documentation. |

## 16. Definition of done

- Rigidbody and Collider are independently authorable on any actor, including invisible actors and compound children.
- Every required body property, shape, material, event and query is implemented with explicit supported semantics and diagnostics.
- CharacterController has its own complete low-level API and passes the obstacle suite independently of CharacterMotor.
- CharacterMotor, PlayerInput and PlayerCamera provide reusable working players in both dimensions without mandatory game-specific code.
- Authoring, undo, migration, blocks, scripts, compiled execution, shell/MCP and runtime attach/detach use the same model.
- Old projects retain their behavior until an explicit upgrade, and new projects use the Unity-style composition.
- Physics tools explain what happened, expose real collision geometry and let creators inspect a fixed step.
- Native and web builds contain all collision dependencies and pass the shipping acceptance suite.
- The compatibility ledger contains no unimplemented required rows. Any unavoidable numerical or deliberate lifecycle differences are documented beside the feature, not hidden behind a blanket "Unity-like" claim.
