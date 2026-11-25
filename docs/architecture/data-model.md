# Document and data model

The saved document: resources, prefabs, linked assets, game settings, components, the type registry and actors that come and go. Split out of `CLAUDE.md`; the index there lists every page.

## Saving

`project::save_project` writes every scene file, then the index, each through
`files::write_atomic` (temp file, fsync, rename), so a crash leaves each file
whole; a document writer should use it rather than `std::fs::write`.
Bookkeeping that is rebuilt if lost (revision, owner lock) uses
`files::replace_file`, which skips the fsyncs. `tests/golden.rs` holds every
project under `blockloom-core/tests/fixtures/golden` to its saved bytes; a
deliberate format change is `BLOCKLOOM_BLESS=1 cargo test -p blockloom-core
--test golden`, and a new component kind goes in its generator. The edit
cost baseline is in `docs/performance.md`.

Commands edit `open.project` in place; nothing describes its own edit.
`blockloom-app/src/history.rs`'s `Tracker` keeps the last committed copy, and
`commit` (run by every save and every publish) diffs against it with
`blockloom_core::change::diff`: a `Change` is the parts an edit replaced
(the index fields, a scene's settings, single actors, the order of scenes
and actors), each before and after, so it is its own inverse. `push_undo`
marks the next commit undoable (a session joins one step), undo and redo
apply a change backwards or forwards, `save_open` writes only the scenes and
index the commits since the last save touched (`save_project_parts`; an
explicit Save writes everything), and `stream.rs` sends the window only what
changed: `{"full": state}` once, then `{"patch": ...}` with the top-level
fields whose JSON moved and, for the project, the changed fields and actors
of the active scene. The window's copy lists scenes without their actors
(`FrontendView`); `Main.qml`'s `applyMessage` folds each message in, and
`tests/stream.rs` replays the stream against whole snapshots. The shell,
MCP and the attach socket still read whole `state_dto` snapshots. Instruction
`==` ignores ids, so the diff hashes them as well (`same_actor`). A document
read off disk (live reload) is committed with nothing owed to the disk.

## Resource database

`blockloom-core/src/resources.rs`. Every file under `assets/` (not scenes,
which carry their own ids, nor `*.imported` output) has a stable id in `assets.index` beside
`project.blockloom`, keyed by path so diffs stay small. The tray's rename,
move, delete, create and import keep it current, `open_project` reconciles
it with the folder (a file moved outside the editor keeps its id when its
name is unique among what vanished), and `ResourceRef { id, path }` repairs
its path from the id. The authored import choices (role, exposure bias) live
on the entry too; `.blockloom/pipeline.json` keeps only fingerprints and
reads older manifests' roles for migration. `resources::uses` walks the whole
document's JSON for strings naming an indexed path or id, which is "used by"
(`asset-used-by`, `list_assets`'s `uses`, the tray's delete dialog), and
`Project::repoint_asset` follows a rename by the same walk. The plugin import
ledger (`.blockloom/imports.json`) is still its own file.

## Prefabs

`blockloom-core/src/prefab.rs`, commands in `blockloom-app/src/commands/
prefabs.rs`. A prefab is one actor saved as a `.blockprefab` resource. An
instance is a whole actor carrying a `Prefab` component (`PrefabLink`: the
source `ResourceRef` and the sorted property paths it overrides), so the
runtime and everything else see a plain actor. A path is one top-level
field (`Look.visual`, `Collider#1.collider`, a graph key like `strands`);
`Place`, `Parent` and the name are always the instance's own.
`prefabs::mark_overrides` runs before every commit and compares each touched
instance with the tracker's committed copy: a changed path that differs from
the prefab becomes an override, one set back to the prefab's value stops
being one. `prefab::rebuild` is the prefab plus the instance's overrides,
which `apply-prefab` (instance into file, then every instance), `revert-
prefab` and `open_project` (a file edited outside the editor) use. The
inspector draws the link as a card with Apply and per-override Revert, the
actor list has Make prefab and the tray Place in scene.

A prefab carries the actors hanging off its root (`children` in the file,
format 2; a single-actor prefab is still written as format 1). Each scene
actor of an instance links to the file: the root with an empty `member`, the
others with their `member` id and the root's id as `instance`, and each keeps
its own overrides. A child that is itself an instance is nested by
reference: the file keeps its link and overrides, and `prefab::read_members`
expands it (members `nested/child`, at most 16 deep, a cycle refused).
`sync_instance` adds members a file gained and removes ones it lost; an
instance that deleted a member records `removed:<member>` on its root so a
sync leaves it out until reverted. Apply works from any actor of an
instance and also takes in actors hung off it since; Unpack unlinks the
whole instance. No prefab editing mode yet.

## Material and environment assets

`blockloom-core/src/linked.rs` is what prefabs, materials and environments
share: an `AssetLink` (source `ResourceRef`, sorted override paths), `mark`
(a changed path that differs from the file becomes an override, one put back
stops being one) and `merge` over JSON, paths at most two keys deep.
A `.blockmaterial` (`material_asset.rs`) is one `SurfaceMaterial`; an actor's
`Material` component carries `source` while it follows one, and keeps its own
copy so the runtime never reads the file. A `.blockenv`
(`environment_asset.rs`) is a scene's look, the `World` fields in `KEYS`; a
scene follows one through `World.environment`, saved as an `Environment`
scene component only while there is one. `commands/materials.rs` and
`commands/environments.rs` mirror `prefabs.rs`: create, assign, apply (file
then every follower), revert, unlink, `mark_overrides` before every commit
and `sync_*` on open, so a file edited outside the editor reaches its
followers. `.blocklighting` still links just a scene's lighting. The 2D look
(`Lighting2d`, `Post2d`, `Camera2d`) is a scene component too.

## Game settings

Quality, input actions, the sound buses and the display output hold for the
whole game: `Project.settings` (`blockloom-core/src/settings.rs`,
`GameSettings`), saved in the index. Older documents kept them on every
scene's `World`; those load into `World::legacy_*` (never written) and
`Project::adopt_scene_settings` takes the default scene's values, so the next
save drops them from the scene files. A `set_scene_component` of one of the
four writes the project's settings. Project Settings draws them on its Game
page.

## Components

An actor is a list of components, not a fixed set of fields
(`blockloom-core/src/components.rs`). `Place`, `Look`, `Render` and `Body`
each become the Bevy component they name when the runtime spawns the actor,
so removing `Body` really does leave it without a rigid body, and removing
`Look` leaves a positioned, scriptable actor with nothing to draw. Only
`Place` can't be removed: there is nowhere for an entity without a transform
to be. `actor.visual()` is therefore an `Option`, while `placement()`,
`physics()` and `visible()` fall back to a default.

Four components have no fixed-field ancestor. `Camera` attaches the world
camera to that actor - follow, first person or third person - and one project
has one of them, so adding it takes it off whoever had it. `Script` names a
Rust file (see Scripts in `blocks-and-scripts.md`). `Parent` names another actor this one hangs off (see
Actors below). `Custom` is a named bag of values the project invented
(`Health { hp, armour }`); the runtime carries it on the entity as
`CustomComponents`, publishes it through `sense::ActorSense`, and
`set <field> of <component> to` writes it back. Like a position and unlike a
variable, those writes last exactly as long as the run.

Components come and go mid-run, from a block (`attach`/`detach`) or from a
script. `engine.attached` is the one record of what an actor is carrying right
now - the document says what it *started* with - and
`world::apply_component_effects` is the only place that writes it. A `Body` or
a `Look` needs the dimension's own pipeline, so `dim2`/`dim3` pick those two
out of the same effect list and do the ECS half. Re-attaching brings back what
the editor authored, or that component's defaults if the project never had one.

Pre-component documents kept `visual`/`placement`/`physics`/`visible` flat on
the actor, and the world camera named the actor it followed. Both still load:
`Actor` deserializes through `ActorRepr`, and `Project::normalize` moves the
old `follow` onto its actor as a camera component.

## Type registry

Every built-in component and setting describes itself. `blockloom-schema-derive`
is a `#[derive(Schema)]` that reads the serde attributes (renames, tags,
`flatten`, `default`, `skip`) and emits a `blockloom_plugin_api::schema::Schema`
impl, so a type's schema is the shape serde writes. The field types are the
plugin ones plus `struct`, `map`, `optional`, `enum` (internally tagged or
external) and `opaque` (a type with no `Schema`, picked by autoref
specialization in `schema::__private`). `#[schema(...)]` adds drawing hints:
`label`, `unit`, `min`/`max`/`step`, `slider`, `multiline`, `color`, `asset`,
`actor`, `group`, `advanced`, `only_2d`/`only_3d` (the other dimension's
registry leaves the field out), `opaque`, `skip`, and `show_if`/`hide_if`
(`kind = Spot`, `kind != Rect`, `kind in [Point, Spot]`, joined with `&&`; a
field starting with `/` reads from the top of the component). A field's
default is the container's `Default` where it has one, else its serde
default.

`blockloom-core/src/registry.rs` lists them: `COMPONENTS` (name, label,
dimension, repeatable), `default_component(name, mode)`, the world settings
(`world_fields`), the project's game settings (`settings_fields`), `to_json(mode)` (what `type-registry` returns, ids blanked so
it is stable) and checked writes (`set_component_property`,
`set_world_property`, `set_settings_property`, paths like `light.beam.motes.count`). The app's
`commands/registry.rs` adds the open project's plugin types (`plugins`) and
the generic commands: `type-registry`, `set-property
target=world|settings|actor:<id>/<Component> path=... value=...` (a built-in component
through `set_actor_component`, a plugin one through `set_plugin_component`, a
world setting through `write_world`, a game setting through `write_settings`, coalesced per path into one undo step)
and `add-component` (registry defaults; Script and Collider keep their own
commands). The typed `set_*` setters for a world or game setting
(`set_sky`, `set_fog`, `set_quality`, ...) stay as wrappers: each normalizes its
value and writes through `write_world` or `write_settings_with`, the same undo,
save and sync path `set-property` takes. MCP gets a `blockloom://types` resource, and `src/plugins.ts`'s
`typeSchema` turns any registry or plugin field into zod
(`integration.test.ts` checks every default fits).

The editor fetches the registry once per project and dimension (`Main.qml`'s
`typeRegistry`, `componentFields`, `worldField`) and draws schema-built cards
with blockstitch's `PropertyForm`/`PropertyEditor` through `RegistryForm.qml`,
which supplies the editor's color swatch, asset box and actor picker. The
Light and Emitter cards, the Fog settings, the CharacterController,
CharacterMotor and PlayerCamera cards, most of the Rigidbody card (body type,
mass, centre, inertia and freezes stay hand-drawn) and every plugin record
(`PluginRecordForm.qml`) are drawn this way, and the add-component menu asks
`add_component`, so their defaults are Rust's only. Collider and Constraint
cards are still hand-written. `blockloom-qt/tests/qml/fixtures/Registry3d.js`
and `Registry2d.js` are the registries the QML tests (`tst_RegistryCards.qml`,
`tst_PhysicsInspector.qml`) read; `BLESS=1 cargo test -p blockloom-core --test
registry_fixture` regenerates it after a schema change. A new hint or
condition form is taught to `PropertyForm.qml` (blockstitch) and `typeSchema`
in `mcp/src/plugins.ts`. `docs/reference/` (components, world, settings) is
generated from the registry by `blockloom-core/src/reference.rs`; rerun
`BLESS=1 cargo test -p blockloom-core --test reference_docs` after a schema change.

## Actors that come and go

An actor's id is what everything keys it by, and a run can mint ids the
document never had. `engine.spawned` holds those runtime-only actors and
`Engine::actor` looks there before the project, so one question finds any
actor at all - a clone answers about its shape, its physics and its components
exactly as the actor it was copied from does. Those ids are counted rather
than random - `~1`, `~2` - so one project run makes the same ids however it
was scheduled, which is what lets `tests/codegen.rs` hold a compiled run's
clones against the VM's. An id the host mints itself, for a script's clone in
a build whose scheduler is the compiled program, is `~h1` instead, so the two
counters can never land on the same name.

`create a clone of` copies a running actor. The VM does the scheduling half:
`register_clone` gives the copy the template's compiled `Program` (an `Rc`
clone - one program, many actors), its name, its custom-block inputs, and its
own copy of the template's variables as they stand. It then queues
`Event::Cloned`, which starts the copy's `when I start as a clone` strands at
the top of the next tick - by which time `apply_lifetimes` has built the
entity those blocks read through. The host does the world half: the clone is
the template as the editor authored it, standing where the template stands
now, carrying its live custom-component values and hanging off whatever it
hangs off. A clone shares its template's name, so `when I touch Ball`,
`broadcast` and `how many Ball there are` all reach every copy; `the actor I
made` reports an id, which is how a block means one clone in particular.
A compiled program does the same halves in the same order: its own `Actors`
table mints the id and queues the copy, and the host is handed the entity to
build (see Compiling the blocks in `docs/architecture/blocks-and-scripts.md`).

`create actor` makes something the document never had: a `Place`, a plain
`Look`, and no blocks at all. `delete` takes an actor out of the run - the
document is untouched, so Play puts an authored one back - and stops its
scripts. Deleting yourself ends the strand that asked where it stands, the
way `stop all` ends everything. Both blocks name an actor through a value
slot, so `the actor I made` can be dropped straight into one; `create a clone
of` names an authored actor, so it stays a dropdown.

The parent/child hierarchy is `engine.parents`, one entry per child, seeded
from every `Parent` component on a rebuild and moved after that by `set my
parent to` or a script. Rather than reparenting Bevy's transforms - which
would make every position in the engine relative to somebody and leave rapier
owning half of them - `world::apply_parenting` moves each child by exactly the
change its parent underwent this step: `child = (parent now / parent then) *
child`. A child that moved itself keeps that motion, a parent's turn swings
its children around it, and parents are walked roots-first so one pass carries
a move the whole way down a chain. It runs in `FixedPostUpdate` before
`record_poses`, so the parent's change includes what physics wrote and the
pose the renderer interpolates towards is where the child ended up. Loops are
refused where they would be made - `Project::prune_parents` on load,
`commands::check_parent` in the editor, `world::set_parent` at run time -
because a cycle has no root to start the pass at.

A `Parent` may also carry an `offset`, which is where the child stands in its
parent's frame. `Place` stays the one thing the world is built from, so the
offset is resolved into a world placement before anything is spawned -
`world::place_authored_children`, parents before their children so an offset
down a chain is measured against a parent that has already moved - and read
again whenever `set my parent to` hangs the actor off someone new, which
places it at the offset rather than leaving it where it stood. A `Parent`
without one - which is what a document written before offsets says - keeps
the world position its own `Place` gives it, at build and at run time both.
The local-position reporters (`my local x position`, `<actor>'s local x
position`, and the matching script reads) answer that frame live: the
parent's world transform inverted onto the child's world position, or the
world position itself for an actor hanging off nothing.
