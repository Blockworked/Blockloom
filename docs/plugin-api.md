# Writing a Blockloom plugin

This is the author's guide. The design lives in
`docs/plugin-system-and-voxel-plan.md` and the decisions in
`docs/plugin-adr-0001.md`; this page is how to build one.

## Start

```sh
blockloom-shell --no-state --eval 'plugin-new path=./greeter id=com.example.greeter template=portable'
cd greeter
cargo test          # runs the plugin through the SDK harness
./build.sh          # builds portable/greeter.wasm and seals the package
```

`template=declarative` makes a package with schemas and no code. It is sealed
at once and installs with `plugin-install id=com.example.greeter source=path:./greeter`.

## Tiers

| Tier | Has | Runs |
| --- | --- | --- |
| `declarative` | schemas only | everywhere, no code |
| `portable` | a `.wasm` module | sandboxed (wasmi) on every platform, web and Android included |
| `native` | a library per target | desktop only, needs the `native-execution` capability |

One SDK source builds as a native `cdylib` or a portable module. A package
with both uses the library where there is one for the target and the module
elsewhere.

## The manifest

`plugin.json` is a claim the host checks against the disk. The fields that
matter: `id` (reverse domain), `version`, `tier`, `abi`, `sdk`, `runtime`
(`portable` with `module`, `memory_limit_mib` and `call_limit_ms`, or
`native` per target), `contributions` (schema files), `capabilities`,
`provides` and `conflicts`. `plugin-seal` writes every file's sha256 into
`files`; an install refuses a package whose bytes differ.

`conflicts` lists plugin ids this one cannot be active beside. Two packages
that `provide` the same service conflict too. A conflict stops Play and
Build and is shown in the Plugin Manager.

## Schemas (contributions)

A schema file may hold any of: `components`, `resources`, `blocks`,
`commands`, `panels`, `tools`, `hooks`, `importers`, `build`, `nodes`,
`menus`, `shortcuts`, `overlays` and `shaders`. Everything is validated when
the package loads, so a mistake is an install error, not a run-time one.

- **Components and resources** are typed records (`bool`, `int`, `number`,
  `text`, `color`, `vec3`, `choice`, `asset`, `actor`, `list`) with bounds,
  `ui` hints and `migrations` between versions. The inspector draws them.
- **Commands** are the one way a plugin changes a project: `add_component`,
  `set_field`, `set_resource`, `set_resource_field` or `module` (an op in
  your code). Every command is one undo step.
- **Blocks** are statements, reporters or hats. A block names a command (a
  statement or reporter) or an event (a hat). A reporter's command must be a
  module op and returns `{"value": ...}`.
- **Menus and shortcuts** run a command with fixed arguments from the top
  bar's plugin menu or a key sequence.
- **Overlays** are scene-view toggles. While one is on, the preview module is
  asked for op `overlay.<name>` and what it answers is drawn as gizmo shapes.
  Overlays only look.
- **Tools** are scene-view tools: a `cast` op on the preview module and a
  command to run on the hit.
- **Shaders** are `.wesl` files registered as `blockloom::plugin_<id>_<name>`
  (dots in the id become underscores), importable from a surface shader.
  A shader cannot replace a built-in module.

## Ops

Your plugin implements `Plugin`:

```rust
use blockloom_plugin_sdk::{Error, Host, Plugin, Value, export_plugin, json};

struct Counter(i64);

impl Plugin for Counter {
    fn start(_host: &Host) -> Result<Self, Error> { Ok(Counter(0)) }

    fn call_json(&mut self, _host: &Host, op: &str, args: Value) -> Result<Value, Error> {
        match op {
            "bump" => { self.0 += args["by"].as_i64().unwrap_or(1); Ok(json!({"count": self.0})) }
            _ => Err(Error::unsupported(op)),
        }
    }
}

export_plugin!(Counter);
```

A module answers ops the host raises:

| Op | When |
| --- | --- |
| a command's or block's `op` | the command or block runs |
| `world.start`, `world.stop` | a run begins and ends (also a preview's start) |
| `world.save`, `world.restore` | hot reload: save state before the module is replaced, restore it after |
| `hook.<name>` | once per stage a hook names (`input`, `pre_simulation`, `fixed_simulation`, `effect_application`, `post_physics`, `render_extraction`, `presentation`) |
| `job.<name>` | a slice of a job the plugin started |
| `node.<name>` | one node of a generation graph |
| `overlay.<name>` | an overlay is on |
| `importer.<name>`, `build.<name>` | an asset import or a build hook (raw bytes) |

An op answers JSON. `{"value": ...}` is a reporter's answer. `{"effects": [...]}`
asks the world to act, in order. A plugin never touches the world itself.

## Effects

| Effect | Fields |
| --- | --- |
| `say` | `text` (a run-log line) |
| `broadcast` | `message` (fires the message hats) |
| `error` | `message` (reported, the run carries on) |
| `event` | `name`, `actor?`, `args` (starts the plugin's hat blocks) |
| `mesh` | `name`, `positions`, `normals`, `colors`, `indices`, `origin`, `emission`, `roughness`, `collider`, `collider_kind` |
| `remove_mesh` | `name` |
| `instances` | `name`, `mesh`, `positions`, `yaw?`, `scales?` (many copies of one of your meshes, drawn in one batch) |
| `remove_instances` | `name` |
| `nav_dirty` | `min?`, `max?` (the ground changed: bake the navigation mesh again) |

A mesh is checked (array lengths, indices in range, finite, at most
`MAX_VERTICES`). `collider: true` makes the mesh solid; `collider_kind` is `trimesh` (exact, for
fixed things), `convex_hull` or `aabb` (cheaper, for movers).

## Host services

`host.call_json(name, input)` from the SDK. Services are named `area.verb`.
`storage.*` and `save.*` need the `project-storage` capability; the rest are
always open.

| Service | What |
| --- | --- |
| `host.version` | `{engine, abi}` |
| `rng.unit`, `rng.range` | deterministic numbers from `{seed, index}` (`count` for a batch) |
| `storage.*` | project blobs: `read`, `exists`, `write`, `delete`, `list`, `commit` (atomic, several ops), `put`/`get` (content addressed) |
| `save.*` | the same verbs over player saves, kept per player, not in the project |
| `diag.count`, `gauge`, `time`, `marker` | your own metrics, shown in the Plugin Manager and the profiler |
| `jobs.start`, `status`, `cancel`, `take`, `list` | background work (below) |
| `physics.ray`, `overlap_point`, `overlap_sphere` | queries over the world's colliders (a run or a hosted preview) |
| `nav.available`, `nav.path` | the navigation mesh |

Keys are per plugin, plain relative names; a write over the store's limits is
refused and the whole commit is atomic.

## Jobs and generation

`jobs.start {name, args, event?, priority?}` runs `job.<name>` in slices,
each given `budget_ms`, highest priority first. A slice answers
`{progress?, state?}` to carry on, `{done: true, result}` to finish, or
`{error}`. `state` comes back on the next slice, so a slice never has to
finish what it began. `event` is raised when the job ends. `cancel` calls the
op once more with `cancel: true`.

A graph is data: `nodes` (typed ports, parameters, a `margin`) in the schema
and a `GraphDef` in a resource. A job named `graph.evaluate` with
`{graph, tile, seed?, extra?}` evaluates one node per slice with a tile cache,
so a changed parameter recomputes only what it feeds.

## Isolation, limits and reload

- A portable call is limited by fuel from `call_limit_ms`; a trap or an
  overrun stops the module, which loads fresh at the next call.
- A native library runs in the host's process. Set
  `BLOCKLOOM_PLUGIN_ISOLATION=process` to host native libraries in
  `blockloom-plugin-worker` instead (`all` hosts portable modules there as
  well). A call over its wall-clock limit or a worker that dies stops that
  module and the game carries on. The worker binary sits beside the editor
  (`BLOCKLOOM_PLUGIN_WORKER` names another).
- Reloading a changed package during a session: a portable module is saved
  (`world.save`), replaced and restored (`world.restore`); a native change
  needs a restart of the run.

## Test

Add `blockloom-plugin-sdk` with the `testing` feature as a dev-dependency:

```rust
use blockloom_plugin_sdk::{harness, json};

#[test]
fn bumps() {
    let plugin = harness!(Counter).unwrap();           // or harness!(Counter, [Capability::ProjectStorage])
    assert_eq!(plugin.call("bump", json!({"by": 2})).unwrap()["count"], 2);
    assert!(plugin.logs().is_empty());
}
```

The harness runs the plugin in-process through the host's own loader, with
memory stores and the default services, so errors, logs and statuses behave
as in a real run. It does not run a world: effects come back in the answer for
you to assert on.

For the package itself, `plugin-inspect` verifies a folder (hashes, schemas,
capabilities) and `plugin-check` audits a project's plugins. A project's
plugin commands are shell and MCP tools as `plugin-id/name`.

## Publish

`plugin-seal` then `plugin-publish path=<package> registry=<folder>` writes an
archive and an index entry into a registry folder; serve that folder over
HTTPS and add it with `plugin-registry`. Installs check each archive's hash
against the index.
