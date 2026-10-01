# ADR 0001: plugin platform foundation (plan phases 0, 1 and the data half of 2)

Status: accepted for the first implementation PR. Follows
`plugin-system-and-voxel-plan.md`; this file records what was built, what was
measured, and where the build differs from the plan.

## Decisions

**Crates.** `blockloom-plugin-api` (manifest, schemas, records, C ABI types;
serde and semver only) and `blockloom-plugin-host` (resolver, cache, lock
files, install transactions, native loader, lifecycle). `blockloom-core`
depends on the api crate only. The host crate pulls in libloading, zip and git
handling, none of which belong in a crate that must still build for
`wasm32-unknown-unknown`.

**Identity and reproducibility.** A plugin is a reverse-domain id plus a semver
version. `plugin.json` lists a sha256 for every file; the package's content
hash covers the manifest bytes and that table, and a version published to a
registry or the cache is immutable. A project holds three files:
`plugins.json` (what the author asked for), `plugins.lock` (the whole resolved
graph, with hashes, sources and per-target library hashes) and the optional,
uncommitted `plugins.local.json` (development overrides). Only an explicit
update ever moves a locked version.

**Installs are transactions.** A change resolves the whole graph first, fetches
and verifies into a staging directory, publishes into the shared immutable
cache, then rewrites the two project files atomically after snapshotting the
old pair into `.blockloom/plugin-history`. `plugin-rollback` restores the last
snapshot and needs no network. Resolution is a backtracking search that
prefers locked versions and answers failures with the chain of requirements
that caused them.

**Data is never lost for lack of code.** A plugin's data in the document is a
`PluginRecord { plugin, type_id, schema_version, payload }`: an actor component
(`ActorComponent::Plugin`) or a project resource (`Project.plugin_resources`).
Core never interprets a payload. A record whose plugin is missing, too new or
invalid is kept byte for byte through open and save. Whether that stops Play
or a build is decided by the schema (`editor_only` records do not), and the
refusal names the records and the command that fixes it. Migrations run on a
copy, all records or none, and snapshot the old payloads first.

**Tiers.** Declarative packages (schemas, commands, blocks, hook ordering) work
everywhere and need no code. Native packages are C-ABI shared libraries per
target triple; a build that cannot carry a package's code for the target is
refused (web and Android builds refuse code plugins for now). Portable (WASM)
and source-linked adapter tiers are described in the manifest and validated,
but nothing executes them yet.

**Native ABI v1.** One exported symbol, `blockloom_plugin_entry_v1`. Every
struct is size and version prefixed so fields can be added; buffers are
`Slice`/`Buffer` pairs with explicit ownership; calls return a `Status`;
objects cross the boundary as generational handles from a `HandleTable`, so a
stale handle is an error and never a dangling pointer. The host function table
a plugin receives is gated by the capabilities its manifest declared.

**Lifecycle.** `Lifecycle` hands out generation tickets so work started before
a plugin reload cannot land after it; `order_hooks` sorts contributed hooks by
stage and explicit before/after edges and rejects cycles.

## Measurements (container, release build, no GPU)

The ignored `abi_call_cost` test in `blockloom-plugin-host` calls a
rustc-compiled C-layout fixture library.

| Shape | Cost |
| --- | --- |
| One call per item through the C ABI | about 43 ns per call |
| One call per batch of items | about 0.21 ns per item |

That is a 200x spread, which settles the plan's open question: any plugin hot
path (voxel pages, mesh jobs) must be a bulk API, and per-item calls are for
rare events. GPU, browser and WASM costs were not measured: there is no GPU in
the build container and no WASM executor exists yet.

## Proof package

`plugins/examples/com.example.health` is a sealed declarative package outside
engine source: a versioned `Health` component with a schema migration, a
`Difficulty` resource, three commands and one block. `blockloom-app/tests/plugins.rs`
installs it from a `path:` source, runs its commands through `plugin-call`,
removes it, shows the records survive and the project refuses to run, and
rolls the removal back. `plugin-seal` and `plugin-publish` are the author
tools.

## Deviations from the plan

Implemented from phase 2 only the data, command and migration parts. Not done,
and tracked in `TODO.md`:

- No Plugin Manager or contribution UI in QML. The surface is the backend
  commands, the state snapshot (`plugins`) and the shell/MCP.
- No WASM executor and no browser proof; the portable tier only validates.
- Native modules load in the editor (a `module` command action calls them,
  tested with a rustc-built C-layout fixture) but not yet in the runtime's
  world or the built player, so plugin code cannot act on a running game.
- Plugin statement blocks run in the editor only (see "Plugin blocks" below).
  Reporters, hats, the script ABI, palette entries and built games are not
  done.
- Registry transport is a local folder (`DirRegistry`); no HTTP registry.
- Shell and MCP expose plugin commands through `plugin-id/command` shell lines
  and the `plugin-call` tool. MCP does not yet list each plugin command as a
  tool of its own.
- Package changes (install, remove, update) refuse to run on a copy attached
  to another editor's files, so they never race the owner's; they do not
  bump the folder's revision counter yet.
- Voxel work (plan part II) was not started; it needs phase 2 services that
  do not exist yet.
- Pack format is v2 only for packs that carry plugins; others are still
  written as v1 so older players read them.

## Plugin blocks (second batch)

A plugin's `BlockSchema` becomes one generic instruction,
`PluginBlock { plugin, block, args }`, rather than a variant per block, so a
document holding one loads in an editor that lacks the plugin. The VM lowers it
to `Effect::PluginCall`; the world forwards it to the editor
(`RuntimeMessage::PluginCall`, protocol 21), because the editor already owns the
plugin host, and the editor runs the block's command through `plugin_call`.
That keeps native modules out of the world for now, at the price of a message
round trip, so these blocks are for rare events, not per-frame work. The result
of the command is dropped (a statement has none), failures reach the run log,
and Play is refused when a block's plugin is missing or its slot count changed.

Deviations: codegen returns `Unsupported` naming the block, so a project with
plugin blocks plays on the VM, and a Build refuses them until the player can
run plugin code. Palette entries need blockstitch to draw a row from a schema
at run time, which this batch does not touch.
