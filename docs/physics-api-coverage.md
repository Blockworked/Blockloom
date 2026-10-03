# Physics API coverage audit

Each required item from `physics-and-character-controller-plan.md` against what
exists, in which surfaces, and what is verified. "Verified" means a test runs it
in the container; the container has no GPU and no Qt, so GPU paths and QML are
written and reviewed but have not been run here.

Surfaces: **Doc** (saved model), **Rt** (runtime), **Blk** (blocks, VM and
compiled), **Scr** (script ABI), **CLI** (shell and MCP), **UI** (inspector).

| Area | Doc | Rt | Blk | Scr | CLI | UI | Verified | Gaps |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Rigidbody (type, mass, damping, gravity, sleep, freeze, CCD, interpolation, solver overrides) | yes | yes | forces, velocity | yes | yes | card | core and runtime tests | - |
| Collider (primitives, compounds, from look, mesh, terrain, tilemap, triggers, layers, materials) | yes | yes | trigger, layer | - | yes | card | core and runtime tests | no collider gizmos or handle editing in the scene view |
| Physics materials, friction and bounce combine, stick and slip | yes | yes | - | - | yes | card | tests | - |
| Layers and pair rules (32 layers, include/exclude, priority) | yes | yes | layer, mask | - | yes | card | tests | - |
| Contact lifecycle (enter, stay, exit, aggregate, payload) | yes | yes | hats | events | - | - | runtime tests | event order of the two ends is the backend's |
| Queries (ray, cast, overlap, closest) | - | yes | blocks, reporters | yes | - | - | VM, codegen, runtime | - |
| Mesh and terrain cooking, cache, packs | yes | yes | - | - | yes | partial | core tests | no inspector UI for cooking settings |
| CharacterController | yes | yes | blocks, reporters | yes | yes | card | runtime tests | no capsule gizmo |
| CharacterMotor and input actions | yes | yes | blocks, reporters | yes | yes | card | runtime tests | no rebinding UI, platform carry lags a tick |
| PlayerCamera, presets, profiles | yes | yes | blocks | - | yes | card, setup | core and runtime tests | no profile overrides |
| Constraints (8 kinds, limits, motors, spring, break, several per actor, world anchor) | yes | yes | blocks, reporter | yes | yes | card | core, runtime, VM, codegen | no joint gizmos |
| Terrain and tilemap colliders | yes | yes | - | - | - | - | runtime tests | seam traversal is untested on a real course |
| Model streaming | - | no | - | - | - | - | - | collision is not loaded independently of visual LOD |
| Fracture and debris | yes | yes | - | - | - | - | runtime tests | shards inherit filters, surface and mass; inertia comes from each shard's shape |
| Buoyancy | yes | yes | - | - | - | card | runtime tests | sized from colliders, falls back to the Look |
| AI and navigation | - | partial | - | - | - | - | - | does not feed CharacterMotor |
| Animation and root motion | - | partial | - | - | - | - | - | no ragdoll handoff |
| Particles and VFX collision | - | yes | - | - | - | - | runtime tests | approximate; documented in `AGENTS.md` |
| Cloning and runtime attach | yes | yes | clone, attach | - | - | - | runtime test | clones skip mesh colliders and controllers |
| Scene editing (invisible colliders selectable, debug view) | yes | yes | - | - | - | toggle | tests | shapes drawn by Rapier's renderer, not run on a GPU here |
| Profiler rows | - | yes | - | - | - | rows | build only | no pair or contact counts, no solver, query or CCD split |
| Migration (legacy Body to components) | yes | yes | - | - | preview, apply | card | core, app | the character toggle converts to a controller profile, not the new capsule |
| Samples | yes | yes | - | - | create-project sample= | dialog | core, app, runtime | - |
| Packs (native, web) | yes | yes | - | - | - | - | core pack round-trip | web player and native player not run on a real display here |

## Acceptance scenarios not yet shown

From section 14.2 of the plan, the ones this container cannot show or that need
a real course: fast projectile and rotating obstacle against real CCD hardware
settings, mesh/terrain/tile seams, input from gamepad and touch, 30/60/144 FPS
timing sweeps, streaming independent of LOD, AI using the motor, animation
ragdoll, and agreement between the embedded editor, the process runtime, the
native player and the web player. Each needs a GPU or a display; none is
claimed.

## Required gates still open

- The runtime's wasm `cargo check` (`wasm32-unknown-unknown`, `plugins` feature)
  passes. `just web-smoke` needs a browser build and was not run here.
- Windows and macOS checks need their runners.
- Phase 0 left 2D probes, a platform-carry backend switch and Unity-editor
  confirmation of documented values open (`TODO.md`).
