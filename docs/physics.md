# Physics guide

How to build with the physics model: bodies, colliders, characters, joints and
the tools that explain what happened. For the design and the reasons behind it
see `physics-and-character-controller-plan.md`; for what the backend can and
cannot do see `physics-compatibility-ledger.md`; for what is finished and what
is not see `physics-api-coverage.md`.

## The shape of it

An actor is a list of components. Physics is made of these:

| Component | What it is |
| --- | --- |
| Rigidbody | A body that moves (dynamic), is moved by you (kinematic) or never moves (2D static). Mass, drag, gravity, sleep, freeze axes, detection mode. |
| Collider | One shape. An actor may have several; they share the actor's body. A collider with no Rigidbody is static scenery and needs no Look. |
| CharacterController | A capsule that moves with step, slope and ground rules. It is moved by `move` blocks, not by forces. |
| CharacterMotor | Turns intent (walk, jump, crouch) into controller moves. Input actions or AI can drive it. |
| PlayerCamera | Configures the actor's Camera: look, zoom, wall avoidance. |
| Constraint | A joint to another actor or to the world. An actor may have several. |

Units are metres in 3D and pixels in 2D (50 pixels to a metre). Angles are
degrees. Everything is simulated on the fixed step (`Project Settings > fixed
rate`), whatever the display does.

## Making things

1. **Scenery**: add a Collider, shape *From look* or a box. No Rigidbody.
2. **A prop**: add a Rigidbody and a Collider.
3. **A compound prop**: add several Colliders. Mass is the sum of their
   densities unless the Rigidbody sets one.
4. **A trigger**: tick *Trigger* on a Collider. It reports enter, stay and exit
   (`when I touch`, `when I stop touching`) and never pushes anything.
5. **A player**: select an actor and use *Make this a player* in the inspector
   (first person, third person, top down, platformer). It adds a controller, a
   motor, input actions and a camera in one undoable step, with a preview first.
   Finished setups save as profiles.

Layers: 32 named layers with a matrix of which pairs collide (*Project Settings >
Physics*). A collider's layer and its *include* and *exclude* lists refine a pair.

## Joints

Add *Constraint* to an actor, choose a kind and, if you want, the other actor.
Leaving *Connected to* on *The world* pins it in place.

| Kind | Frees | Typical use |
| --- | --- | --- |
| Fixed | nothing | Welded parts that can snap |
| Hinge | one turn | Doors, wheels, levers |
| Ball (3D) | three turns | Shoulders, chains |
| Slider | one slide | Drawers, lifts |
| Spring | one slide, pulled to a rest length | Suspension |
| Distance | everything inside a range | Ropes and tethers |
| Wheel | a slide and a turn | Vehicle wheels |
| Configurable | whichever axes you free | Anything else |

*Auto configure* works the other end's anchor out from where the actors stand,
so nothing jumps when you press Play. A hinge, slider or wheel can have a limit
and a motor (speed or target). *Break force* and *Break torque* snap the joint
when it carries more; set *When it breaks* to broadcast a message. Both actors
need Rigidbodies (the world needs none).

Blocks and scripts reach a joint by its name (or, for an unnamed one, its place
among the actor's constraints): `joint [door] [motor speed] value 90`, and the
`joint` reporter (angle, position, speed, force, torque, enabled, broken) for a
named joint. In a script: `actor.joint("motor speed", "door", 90.0)` and
`actor.joint_number("door", "position")`.

## Blocks, scripts, shell and MCP

- Forces and motion: *add force*, *add torque*, *apply impulse*, *set velocity*.
- Queries: *cast ray*, *cast ball*, *overlap ball*, *find closest*, and the `hit`
  reporters. Queries see the same shapes physics does, filtered by layer.
- Character: *controller move*, *set controller*, the `controller` reporters,
  the motor blocks (*set intent*, *jump*, *crouch*) and the `motor` reporters.
- Joints: *joint ...* and the `joint` reporter.

Everything the editor can do is a shell and MCP command: `add-collider`,
`set-rigidbody`, `add-constraint`, `apply-player-preset`, `physics-plan`,
`physics-check`, `physics-cook`, `migrate-physics` and so on
(`just shell`, then `help`).

## Seeing what happened

- **Physics debug**: the box button in the Game view's toolbar draws collider
  shapes, bounding boxes, contacts, joints and body axes over the world, while
  stopped and while playing.
- **Profiler**: rows for bodies, active and sleeping bodies, colliders, joints
  and the physics step time.
- **Checks**: `physics-check` names what would stop Play (a body with no
  collider, a joint with no body, a layer that does not exist) before you try.

## Older projects

A project from before components keeps its legacy *Body* and plays exactly as it
did. Nothing changes by opening it. To move to the new model, open the Body card
on an actor and use *Upgrade this actor* (or *Upgrade all*), or run
`physics-migration-preview` and `migrate-physics` from the shell. The preview
shows what each actor becomes; the upgrade is one undo step, copies the project
file to `.blockloom/backups/` first, and running it twice does nothing the
second time. Legacy blocks and commands keep working until you remove them.

## The playground

*New project > Start from > Physics playground* (or `create-project name=Demo
sample=physics-playground`) makes a project with a player, a stack of crates, a
pendulum on a rope, a motor wheel, a door, a plank held by a joint that snaps
and a trigger zone, all made of components with no custom code. Open the
physics debug view and press Play to see how it fits together.

## Shipping

A built game carries the physics settings, the components, the layer matrix and
the cooked collision data in its pack. The desktop player and the editor's
process runtime play it the same way; the web player carries the same pack.
Cooked meshes are rebuilt only when their model changes (`physics-cook` shows
what is stale).

## Limits worth knowing

- Joints are not part of vehicles or ragdoll rigs; those are built from joints
  by hand. Animation does not hand a ragdoll its pose yet.
- AI and navigation steer through their own movement; they do not drive a
  CharacterMotor yet.
- Collision data is as far away as visuals are: collision streaming does not yet
  load on its own distance.
- A clone made while playing gets its body, colliders and constraints, but not
  mesh colliders or a character controller.
