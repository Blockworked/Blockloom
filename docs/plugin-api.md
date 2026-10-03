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
`template=native` writes a crate that builds a native `cdylib` (and still the
portable module, so web and Android work); `plugin-add-native path=./greeter
target=aarch64-apple-darwin library=native/libgreeter.dylib` adds a library for
another target to the manifest and seals again.

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
`menus`, `shortcuts`, `overlays`, `shaders` and `kernels`. Everything is validated when
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
- **Kernels** are GPU compute (see below).

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
| `world.preview` | an editor preview frame; optional, does not run game hooks |
| `world.start`, `world.stop` | a run begins and ends (also a preview's start) |
| `world.save`, `world.restore` | hot reload: save state before the module is replaced, restore it after |
| `hook.<name>` | once per stage a hook names (`input`, `pre_simulation`, `fixed_simulation`, `effect_application`, `post_physics`, `render_extraction`, `presentation`) |
| `job.<name>` | a slice of a job the plugin started |
| `gpu.result` | a GPU read finished: `{tag, buffer, offset, values}` |
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
| `mesh` | `name`, `positions`, `normals`, `colors`, `indices`, `origin`, `emission`, `roughness`, `collider`, `collider_kind`, `gpu?`, `body?` |
| `remove_mesh` | `name` |
| `instances` | `name`, `mesh`, `positions`, `yaw?`, `scales?` (many copies of one of your meshes, drawn in one batch) |
| `remove_instances` | `name` |
| `nav_dirty` | `min?`, `max?` (the ground changed: bake the navigation mesh again) |
| `gpu_buffer`, `gpu_write`, `gpu_dispatch`, `gpu_read`, `gpu_free` | GPU compute, see below (needs `gpu-compute`) |

A mesh is checked (array lengths, indices in range, finite, at most
`MAX_VERTICES`). `collider: true` makes the mesh solid; `collider_kind` is `trimesh` (exact, for
fixed things), `convex_hull` or `aabb` (cheaper, for movers).

`world.save` answers `{"state": ...}`; `world.restore` receives that same state
under `state`. This is module hot reload, separate from player save storage.

A mesh may name `gpu: {buffer, vertices}` with `gpu-compute`. The buffer contains
10 f32 words per vertex: position, normal, RGBA. The host copies those vertices
into its prepared render mesh without CPU readback. `vertices` must be a positive
multiple of three, cover the CPU index count and stay within `MAX_VERTICES`.
CPU arrays remain the collision geometry and renderer fallback. Queued compute
must finish before copying; a failed compute batch retains the CPU mesh.

`body: {mass, velocity, angular_velocity?, rotation?}` creates a dynamic body.
Mass is positive kilograms, velocity uses world units per second, angular velocity
uses radians per second, and rotation is an XYZW unit quaternion. A body requires
`collider: true` and `convex_hull` or `aabb`. Rotation defaults to identity;
angular velocity defaults to zero.

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
| `world.actor` | `{actor}` returns `{found, position?}` from the sensed actor snapshot |
| `world.mesh_pose` | `{name}` returns your mesh body position, rotation, velocity and angular velocity, or null |
| `nav.available`, `nav.path` | the navigation mesh |

Keys are per plugin, plain relative names; a write over the store's limits is
refused and the whole commit is atomic. Content blobs (`put`) are referenced
by the string `blob:<sha256>`; `plugin-data-gc` (try `dryRun=true` first)
removes the ones no record, other key or other blob names, so keep a hash in
that form if you want it to count. Player saves live in the browser's localStorage on
the web, atomically per commit like on disk.

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

## GPU compute

You never hold the GPU. You ship a kernel, and the world runs it on buffers
it owns, in order, under a budget. Needs `"capabilities": ["gpu-compute"]`.

A kernel is a `.wgsl` file and a schema entry:

```json
{"kernels": [{
  "name": "scale", "file": "kernels/scale.wgsl", "workgroup_size": [64, 1, 1],
  "bindings": [
    {"name": "input",  "binding": 0, "kind": "read"},
    {"name": "output", "binding": 1, "kind": "read_write"}
  ]
}]}
```

```wgsl
@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&input)) { output[id.x] = input[id.x] * 2.0; }
}
```

Then answer effects (the SDK's `gpu` module builds them):

```rust
use blockloom_plugin_sdk::gpu;
Ok(json!({"effects": [
    gpu::buffer("input", 1024),
    gpu::buffer("out", 1024),
    gpu::write_f32("input", 0, &values),
    gpu::dispatch("scale", &[("input", "input"), ("output", "out")], [16, 1, 1]),
    gpu::read("out", 0, 1024, "scaled", gpu::As::F32),
]}))
```

The read's words come back a few frames later as op `gpu.result` with
`{tag, buffer, offset, values}`; nothing waits on the GPU. Commands run in the
order you gave them, so a write after a dispatch is not seen by it.

The rules, all checked when the package is sealed, installed and loaded:

- Plain WGSL, no textures, samplers, push constants or optional features;
  only group 0 buffers, named, numbered and typed (`read`, `read_write`,
  `uniform`) exactly as the schema says; at most 8 bindings, 256 invocations
  a workgroup and 16 KiB of workgroup memory.
- Every loop is a `for` over a local counter that starts at a constant, is
  compared with a constant and steps by a constant (`for (var i = 0u; i < 16u;
  i++)`). A `while`, a `loop`, a limit read from a variable or `arrayLength`,
  or a counter changed in the body is refused. All loop iterations in one
  invocation, nesting and calls included, must be within 2^20. Out-of-range
  buffer indexing is clamped by the device, so a kernel cannot read or write
  outside its buffers.
- Buffers are 32-bit words: at most 64 buffers and 128 MiB a plugin, 64 MiB a
  buffer. A dispatch is at most 65535 workgroups an axis and 2^26 invocations,
  the world runs 2^27 invocations a frame across all plugins, and what does
  not fit waits for the next frame, in order. At most 4096 commands may wait
  and 16 reads be in flight, 4 Mi words each. A refused command is an error in
  the run log and the ones after it still run.
- The same buffer may not be both written and bound twice in one dispatch.
- A run's end drops your buffers; kernels are loaded again with the project.

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
