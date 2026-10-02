//! A running game's plugin code.
//!
//! [`WorldPlugins`] opens every module a [`Loadout`] names and then only
//! reacts: the world calls it at lifecycle points ([`start`](WorldPlugins::start),
//! [`stop`](WorldPlugins::stop)), once per stage ([`run_stage`](WorldPlugins::run_stage))
//! and when a plugin block runs ([`run_block`](WorldPlugins::run_block)). Every
//! call answers with [`Outcome`]s the world applies in order, so a plugin
//! submits effects and never touches the world itself.
//!
//! A module that fails to load, or that traps or runs out of budget, is
//! dropped for the rest of the run and reported once; one broken plugin
//! does not stop the others. A module that has no op for something the
//! world asks (a lifecycle call it does not care about) is simply skipped.
//!
//! An op's answer may carry `{"effects": [...]}`, each `{"effect": "say" |
//! "broadcast" | "event" | "mesh" | "remove_mesh" | "error", ...}`; see
//! [`Effect`].
//!
//! A reporter block is answered on demand by [`WorldPlugins::read`]: the
//! module's op returns `{"value": ...}`. Reads are memoized until anything
//! else calls into the module or the world starts a new frame
//! ([`WorldPlugins::forget_reads`]), so a loop asking the same question every
//! step costs one call. A read is a question, so it may log but not act.

use crate::diagnostics::Diagnostics;
use crate::generation::{GraphCache, GraphRun};
use crate::hooks::{HookRef, order_hooks};
use crate::jobs::JobTable;
use crate::module::{CodeModule, is_unsupported};
use crate::services::HostServices;
use blockloom_plugin_api::compute::{GpuCommand, RESULT_OP, ReadAs, Words};
use blockloom_plugin_api::generation::NodeSchema;
use blockloom_plugin_api::loadout::{CodeRuntime, Loadout, LoadoutBlock, LoadoutPlugin, ops};
use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_api::mesh::MeshData;
use blockloom_plugin_api::rendering::InstanceData;
use blockloom_plugin_api::schema::{FieldSchema, FieldType, HookSchema, Stage};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use web_time::Instant;

/// What a plugin may ask the world to do from an op's answer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Effect {
    /// A line in the run log.
    Say { text: String },
    /// Fires the message hats, as the `broadcast` block does.
    Broadcast { message: String },
    /// Reports a problem in the run log; the run carries on.
    Error { message: String },
    /// Fires the plugin's own event: the hats naming it start, on `actor`
    /// when one is named and on every actor otherwise. `args` are what the
    /// hat's slots match against.
    Event {
        name: String,
        #[serde(default)]
        actor: Option<String>,
        #[serde(default)]
        args: Vec<Value>,
    },
    /// Draws a mesh, replacing the plugin's mesh of the same name.
    Mesh(MeshData),
    /// Takes the plugin's mesh of that name out of the world.
    RemoveMesh { name: String },
    /// Draws many copies of one of the plugin's meshes in one batch,
    /// replacing the plugin's set of the same name.
    Instances(InstanceData),
    /// Takes the plugin's instance set of that name out of the world.
    RemoveInstances { name: String },
    /// The ground changed: the navigation mesh is baked again. `min` and
    /// `max` say where, for the run log; the bake covers the whole level.
    NavDirty {
        #[serde(default)]
        min: Option<[f32; 3]>,
        #[serde(default)]
        max: Option<[f32; 3]>,
    },
    /// Makes (or replaces, zeroed) a GPU buffer of `words` 32-bit words.
    /// The GPU effects need the `gpu-compute` capability.
    GpuBuffer { name: String, words: u32 },
    /// Writes `f32`, `u32` or `i32` values into a buffer at a word offset.
    GpuWrite {
        buffer: String,
        #[serde(default)]
        offset: u32,
        #[serde(flatten)]
        data: Words,
    },
    /// Runs one of the plugin's kernels: `bindings` maps each of the kernel's
    /// binding names to a buffer, `groups` is the workgroup count per axis
    /// (missing axes are 1).
    GpuDispatch {
        kernel: String,
        bindings: BTreeMap<String, String>,
        #[serde(default)]
        groups: Vec<u32>,
    },
    /// Reads words back; the plugin's `gpu.result` op gets them a few frames
    /// later, tagged with `tag`.
    GpuRead {
        buffer: String,
        #[serde(default)]
        offset: u32,
        words: u32,
        tag: String,
        #[serde(default, rename = "as")]
        as_type: ReadAs,
    },
    /// Frees a buffer.
    GpuFree { buffer: String },
}

impl Effect {
    /// Whether this is one of the GPU effects.
    pub fn is_gpu(&self) -> bool {
        matches!(
            self,
            Effect::GpuBuffer { .. }
                | Effect::GpuWrite { .. }
                | Effect::GpuDispatch { .. }
                | Effect::GpuRead { .. }
                | Effect::GpuFree { .. }
        )
    }

    /// The command a GPU effect asks the world's compute engine for.
    pub fn gpu_command(&self) -> Option<Result<GpuCommand, String>> {
        Some(match self {
            Effect::GpuBuffer { name, words } => Ok(GpuCommand::Buffer {
                name: name.clone(),
                words: *words,
            }),
            Effect::GpuWrite {
                buffer,
                offset,
                data,
            } => data
                .bits()
                .map_err(|e| format!("write to {buffer}: {e}"))
                .map(|data| GpuCommand::Write {
                    buffer: buffer.clone(),
                    offset: *offset,
                    data,
                }),
            Effect::GpuDispatch {
                kernel,
                bindings,
                groups,
            } => {
                if groups.is_empty() || groups.len() > 3 {
                    return Some(Err(format!("dispatch {kernel}: groups are 1 to 3 numbers")));
                }
                let mut padded = [1u32; 3];
                padded[..groups.len()].copy_from_slice(groups);
                Ok(GpuCommand::Dispatch {
                    kernel: kernel.clone(),
                    bindings: bindings
                        .iter()
                        .map(|(a, b)| (a.clone(), b.clone()))
                        .collect(),
                    groups: padded,
                })
            }
            Effect::GpuRead {
                buffer,
                offset,
                words,
                tag,
                as_type,
            } => Ok(GpuCommand::Read {
                buffer: buffer.clone(),
                offset: *offset,
                words: *words,
                tag: tag.clone(),
                as_type: *as_type,
            }),
            Effect::GpuFree { buffer } => Ok(GpuCommand::Free {
                buffer: buffer.clone(),
            }),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A line the module logged through the host.
    Log {
        plugin: String,
        level: u32,
        text: String,
    },
    /// The module was dropped, a call failed, or an op's answer was malformed.
    Error { plugin: String, message: String },
    /// An effect the module asked for.
    Effect { plugin: String, effect: Effect },
}

/// A module opened by the caller, for [`WorldPlugins::with_modules`].
pub struct Preloaded {
    pub id: String,
    pub module: CodeModule,
    pub hooks: Vec<HookSchema>,
    pub blocks: Vec<LoadoutBlock>,
}

struct Hosted {
    module: CodeModule,
    blocks: BTreeMap<String, LoadoutBlock>,
    /// Hooks whose op the module does not have; not asked again.
    missing_hooks: BTreeSet<String>,
    /// Answers to reads since the module last did anything else.
    reads: BTreeMap<String, Value>,
    /// The plugin declared `gpu-compute`, so its GPU effects are honoured.
    gpu: bool,
}

#[derive(Default)]
pub struct WorldPlugins {
    modules: BTreeMap<String, Hosted>,
    order: Vec<HookRef>,
    pending: Vec<Outcome>,
    diagnostics: Arc<Diagnostics>,
    jobs: Arc<Mutex<JobTable>>,
    /// The graph nodes each plugin computes.
    nodes: BTreeMap<String, Vec<NodeSchema>>,
    graph_runs: BTreeMap<u64, GraphRun>,
    graph_cache: GraphCache,
    /// What the modules were opened with, so a reloaded one gets the same.
    services: Option<HostServices>,
}

/// What a mid-run change of loadout did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReloadReport {
    /// Portable modules replaced by a newer build.
    pub reloaded: Vec<String>,
    /// Plugins that were not hosted and now are.
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Changed plugins that keep running their old code until the run is
    /// restarted, and why.
    pub restart_needed: Vec<(String, String)>,
    /// Reloaded plugins that carried state across with `world.save`.
    pub kept_state: Vec<String>,
}

impl ReloadReport {
    pub fn is_empty(&self) -> bool {
        self.reloaded.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.restart_needed.is_empty()
    }
}

/// The job name a plugin starts to evaluate a node graph: `jobs.start` with
/// `{"name": "graph.evaluate", "args": {graph, tile, seed?, extra?}}`. One node
/// is evaluated per slice, and the job's result is
/// `{"outputs": {"node.port": value}, "evaluated": n, "cached": m}`.
pub const GRAPH_JOB: &str = "graph.evaluate";

impl WorldPlugins {
    /// Opens every module in `loadout`. A plugin that fails to open is left
    /// out and reported by the next call that returns outcomes
    /// ([`WorldPlugins::drain`]).
    pub fn load(loadout: &Loadout, engine: &str) -> WorldPlugins {
        Self::load_with(loadout, &HostServices::new(engine))
    }

    /// [`WorldPlugins::load`] with the services (storage, diagnostics, a
    /// world's queries) each module is opened with.
    pub fn load_with(loadout: &Loadout, host: &HostServices) -> WorldPlugins {
        let mut world = WorldPlugins {
            diagnostics: host.diagnostics.clone(),
            jobs: host.jobs.clone(),
            services: Some(host.clone()),
            ..WorldPlugins::default()
        };
        let mut hooks = Vec::new();
        for plugin in &loadout.plugins {
            match CodeModule::load_with(&plugin.runtime, &plugin.id, host) {
                Ok(module) => {
                    world.insert(plugin.id.clone(), module, &plugin.blocks);
                    world.allow_gpu(
                        &plugin.id,
                        plugin
                            .runtime
                            .capabilities()
                            .contains(&Capability::GpuCompute),
                    );
                    world.set_nodes(&plugin.id, plugin.nodes.clone());
                    hooks.extend(plugin.hooks.iter().map(|h| (plugin.id.as_str(), h)));
                }
                Err(message) => world.error(&plugin.id, format!("could not load: {message}")),
            }
        }
        match order_hooks(hooks) {
            Ok(order) => world.order = order,
            Err(message) => world.error("", format!("hook order: {message}")),
        }
        world
    }

    /// Hosts modules already open, each with the hooks and blocks it
    /// registered.
    pub fn with_modules(modules: Vec<Preloaded>) -> Result<WorldPlugins, String> {
        let mut world = WorldPlugins::default();
        let hooks = modules
            .iter()
            .flat_map(|m| m.hooks.iter().map(|h| (m.id.as_str(), h)));
        world.order = order_hooks(hooks)?;
        for m in modules {
            world.insert(m.id, m.module, &m.blocks);
        }
        Ok(world)
    }

    fn insert(&mut self, id: String, module: CodeModule, blocks: &[LoadoutBlock]) {
        self.modules.insert(
            id,
            Hosted {
                module,
                blocks: blocks
                    .iter()
                    .map(|b| (b.type_id.clone(), b.clone()))
                    .collect(),
                missing_hooks: BTreeSet::new(),
                reads: BTreeMap::new(),
                gpu: false,
            },
        );
    }

    /// Lets (or stops) a plugin's GPU effects being honoured.
    pub fn allow_gpu(&mut self, plugin: &str, allowed: bool) {
        if let Some(hosted) = self.modules.get_mut(plugin) {
            hosted.gpu = allowed;
        }
    }

    /// Hands a finished GPU read to the plugin's `gpu.result` op. A module
    /// with no such op ignores it.
    pub fn deliver_gpu(
        &mut self,
        plugin: &str,
        tag: &str,
        buffer: &str,
        offset: u32,
        values: Value,
    ) -> Vec<Outcome> {
        let input = json!({"tag": tag, "buffer": buffer, "offset": offset, "values": values});
        self.call(plugin, RESULT_OP, &input, true)
    }

    fn error(&mut self, plugin: &str, message: String) {
        self.pending.push(Outcome::Error {
            plugin: plugin.to_string(),
            message,
        });
    }

    /// Tells the world which graph nodes `plugin`'s module computes.
    pub fn set_nodes(&mut self, plugin: &str, nodes: Vec<NodeSchema>) {
        if !nodes.is_empty() {
            self.nodes.insert(plugin.to_string(), nodes);
        }
    }

    /// The node cache, to size or clear it.
    pub fn graph_cache(&mut self) -> &mut GraphCache {
        &mut self.graph_cache
    }

    /// One node of a graph job: the progress to report, or the whole result.
    fn graph_slice(&mut self, job: &crate::jobs::Job) -> (Vec<Outcome>, Result<Value, String>) {
        let mut run = match self.graph_runs.remove(&job.id) {
            Some(run) => run,
            None => {
                let nodes = self
                    .nodes
                    .get(&job.plugin)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                match GraphRun::new(nodes, &job.args) {
                    Ok(run) => run,
                    Err(e) => return (Vec::new(), Err(e)),
                }
            }
        };
        let mut outcomes = Vec::new();
        let mut cache = std::mem::take(&mut self.graph_cache);
        let plugin = job.plugin.clone();
        let stepped = if run.is_done() {
            Ok(())
        } else {
            let mut call = |op: &str, input: &Value| -> Result<Value, String> {
                let (o, answer, missing) = self.call_answer(&plugin, op, input, false, true);
                outcomes.extend(o);
                answer.ok_or_else(|| {
                    if missing {
                        format!("the module has no op {op}")
                    } else {
                        "the module is not running".to_string()
                    }
                })
            };
            run.step(&mut cache, &mut call)
        };
        self.graph_cache = cache;
        let answer = stepped.map(|()| {
            if run.is_done() {
                json!({"done": true, "progress": 1.0, "result": run.result()})
            } else {
                let progress = run.progress();
                self.graph_runs.insert(job.id, run);
                json!({"progress": progress})
            }
        });
        (outcomes, answer)
    }

    /// What the hosted modules cost and report.
    pub fn diagnostics(&self) -> &Arc<Diagnostics> {
        &self.diagnostics
    }

    /// Gives every running job a slice, highest priority first, until
    /// `budget_ms` is spent (one slice always runs, so jobs make progress).
    /// A job that ends raises its event, and a failure is reported.
    pub fn run_jobs(&mut self, budget_ms: f64) -> Vec<Outcome> {
        let started = Instant::now();
        let mut out = self.drain();
        let notices = self.table().take_cancel_notices();
        for job in notices {
            if job.name == GRAPH_JOB {
                self.graph_runs.remove(&job.id);
                continue;
            }
            let input =
                json!({"job": job.id, "cancel": true, "args": job.args, "state": job.state});
            out.extend(self.call(&job.plugin, &format!("job.{}", job.name), &input, true));
        }
        let mut sliced = Vec::new();
        loop {
            let Some(job) = self.table().next_runnable(&sliced) else {
                break;
            };
            sliced.push(job.id);
            let left = (budget_ms - started.elapsed().as_secs_f64() * 1000.0).max(0.0);
            let input = json!({
                "job": job.id,
                "name": job.name,
                "args": job.args,
                "state": job.state,
                "slice": job.slice,
                "budget_ms": left,
            });
            if job.name == GRAPH_JOB {
                let (outcomes, answer) = self.graph_slice(&job);
                out.extend(outcomes);
                match answer {
                    Ok(answer) => self.table().answered(job.id, &answer),
                    Err(e) => self.table().failed(job.id, e),
                }
                if started.elapsed().as_secs_f64() * 1000.0 >= budget_ms {
                    break;
                }
                continue;
            }
            let op = format!("job.{}", job.name);
            let (outcomes, answer, missing) =
                self.call_answer(&job.plugin, &op, &input, false, true);
            out.extend(outcomes);
            match answer {
                Some(answer) => self.table().answered(job.id, &answer),
                None => self.table().failed(
                    job.id,
                    if missing {
                        format!("the module has no op {op}")
                    } else {
                        "the module is not running".to_string()
                    },
                ),
            }
            if started.elapsed().as_secs_f64() * 1000.0 >= budget_ms {
                break;
            }
        }
        let ended = {
            let mut table = self.table();
            table.prune();
            table.drain_ended()
        };
        for e in ended {
            self.graph_runs.remove(&e.id);
            if let Some(event) = e.event {
                out.push(Outcome::Effect {
                    plugin: e.plugin.clone(),
                    effect: Effect::Event {
                        name: event,
                        actor: None,
                        args: vec![json!(e.id), json!(e.status.name())],
                    },
                });
            }
            if let Some(error) = e.error {
                out.push(Outcome::Error {
                    plugin: e.plugin,
                    message: format!("job {}: {error}", e.id),
                });
            }
        }
        out
    }

    /// The table plugins start jobs in, shared with their `jobs.*` services.
    pub fn job_table(&self) -> Arc<Mutex<JobTable>> {
        self.jobs.clone()
    }

    /// Cancels what `plugin` is running, as a reload or removal does.
    pub fn cancel_jobs(&mut self, plugin: &str) -> usize {
        self.table().cancel_plugin(plugin)
    }

    /// How many jobs are running.
    pub fn running_jobs(&self) -> usize {
        self.table().running()
    }

    fn table(&self) -> std::sync::MutexGuard<'_, JobTable> {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The ids of the plugins still hosted.
    pub fn plugins(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// What has built up since the last call: load failures and the like.
    pub fn drain(&mut self) -> Vec<Outcome> {
        std::mem::take(&mut self.pending)
    }

    /// The run begins. `records` gives each plugin the part of the project
    /// that is its own, as `{"records": [...], "resources": [...]}`.
    pub fn start(&mut self, records: &dyn Fn(&str) -> Value) -> Vec<Outcome> {
        let ids: Vec<String> = self.modules.keys().cloned().collect();
        let mut out = self.drain();
        for id in ids {
            let mut input = records(&id);
            if let Some(object) = input.as_object_mut() {
                object.insert("plugin".to_string(), json!(id));
            }
            out.extend(self.call(&id, ops::START, &input, true));
        }
        out
    }

    /// Brings the hosted modules in line with a loadout that changed during a
    /// run. A plugin that is new is opened and started; one that is gone is
    /// stopped; a portable one whose code changed is replaced - `world.save`
    /// first, then `world.start` and `world.restore` on the new module, its
    /// jobs cancelled. A native library cannot be swapped under a running
    /// game, so a change to one is reported and the old code runs on.
    /// `records` is what `start` takes.
    pub fn reload(
        &mut self,
        old: &Loadout,
        new: &Loadout,
        records: &dyn Fn(&str) -> Value,
    ) -> (Vec<Outcome>, ReloadReport) {
        let host = self
            .services
            .clone()
            .unwrap_or_else(|| HostServices::new(""));
        let mut out = self.drain();
        let mut report = ReloadReport::default();
        let old_by: BTreeMap<&str, &LoadoutPlugin> =
            old.plugins.iter().map(|p| (p.id.as_str(), p)).collect();
        let new_ids: BTreeSet<&str> = new.plugins.iter().map(|p| p.id.as_str()).collect();
        for id in old_by.keys() {
            if !new_ids.contains(id) && self.modules.contains_key(*id) {
                out.extend(self.call(id, ops::STOP, &json!({}), true));
                self.cancel_jobs(id);
                self.modules.remove(*id);
                self.nodes.remove(*id);
                report.removed.push(id.to_string());
            }
        }
        let is_native = |p: &LoadoutPlugin| matches!(p.runtime, CodeRuntime::Native(_));
        let mut effective: Vec<&LoadoutPlugin> = Vec::new();
        for plugin in &new.plugins {
            let id = plugin.id.as_str();
            let hosted = self.modules.contains_key(id);
            let prev = old_by.get(id).copied();
            if prev == Some(plugin) {
                effective.push(plugin);
                continue;
            }
            if let Some(prev) = prev
                && hosted
                && (is_native(prev) || is_native(plugin))
            {
                let reason = if is_native(prev) {
                    "a native library stays loaded for the whole run"
                } else {
                    "a native library is not loaded into a run in progress"
                };
                report
                    .restart_needed
                    .push((id.to_string(), format!("{reason}; restart the run")));
                effective.push(prev);
                continue;
            }
            let mut saved = None;
            if hosted {
                let (o, answer, _) = self.call_answer(id, ops::SAVE, &json!({}), true, true);
                out.extend(o);
                saved = answer
                    .and_then(|a| a.get("state").cloned())
                    .filter(|state| !state.is_null());
                out.extend(self.call(id, ops::STOP, &json!({}), true));
                self.cancel_jobs(id);
                self.modules.remove(id);
            }
            match CodeModule::load_with(&plugin.runtime, id, &host) {
                Ok(module) => {
                    self.insert(id.to_string(), module, &plugin.blocks);
                    self.allow_gpu(
                        id,
                        plugin
                            .runtime
                            .capabilities()
                            .contains(&Capability::GpuCompute),
                    );
                    self.nodes.remove(id);
                    self.set_nodes(id, plugin.nodes.clone());
                    let mut input = records(id);
                    if let Some(object) = input.as_object_mut() {
                        object.insert("plugin".to_string(), json!(id));
                    }
                    out.extend(self.call(id, ops::START, &input, true));
                    if let Some(state) = saved {
                        let (o, _, missing) = self.call_answer(
                            id,
                            ops::RESTORE,
                            &json!({"state": state}),
                            true,
                            true,
                        );
                        out.extend(o);
                        if !missing {
                            report.kept_state.push(id.to_string());
                        }
                    }
                    if hosted {
                        report.reloaded.push(id.to_string());
                    } else {
                        report.added.push(id.to_string());
                    }
                    effective.push(plugin);
                }
                Err(message) => self.error(id, format!("could not reload: {message}")),
            }
        }
        let hooks = effective
            .iter()
            .filter(|p| self.modules.contains_key(&p.id))
            .flat_map(|p| p.hooks.iter().map(move |h| (p.id.as_str(), h)));
        match order_hooks(hooks) {
            Ok(order) => self.order = order,
            Err(message) => self.error("", format!("hook order: {message}")),
        }
        out.extend(self.drain());
        (out, report)
    }

    /// The run ended. Modules stay loaded; dropping `self` unloads them.
    pub fn stop(&mut self) -> Vec<Outcome> {
        let ids: Vec<String> = self.modules.keys().cloned().collect();
        let mut out = self.drain();
        for id in ids {
            out.extend(self.call(&id, ops::STOP, &json!({}), true));
        }
        out
    }

    /// Runs every hook registered for `stage`, in order.
    pub fn run_stage(&mut self, stage: Stage, tick: u64, dt: f64) -> Vec<Outcome> {
        let hooks: Vec<HookRef> = self
            .order
            .iter()
            .filter(|h| h.stage == stage)
            .cloned()
            .collect();
        let mut out = self.drain();
        for hook in hooks {
            let key = hook.qualified();
            let Some(hosted) = self.modules.get(&hook.plugin) else {
                continue;
            };
            if hosted.missing_hooks.contains(&key) {
                continue;
            }
            let input = json!({
                "stage": stage,
                "hook": hook.name,
                "tick": tick,
                "dt": dt,
            });
            let op = format!("{}{}", ops::HOOK_PREFIX, hook.name);
            let (outcomes, missing) = self.call_op(&hook.plugin, &op, &input, false);
            out.extend(outcomes);
            // A hook with no op is a plugin bug: say so once, then stop asking.
            if missing && let Some(hosted) = self.modules.get_mut(&hook.plugin) {
                hosted.missing_hooks.insert(key);
            }
        }
        out
    }

    /// Whether `plugin`'s block `block` runs in the world.
    pub fn has_block(&self, plugin: &str, block: &str) -> bool {
        self.modules
            .get(plugin)
            .is_some_and(|h| h.blocks.contains_key(block))
    }

    /// The slots of a block this world runs, in an instruction's arg order.
    pub fn block_slots(&self, plugin: &str, block: &str) -> Option<&[FieldSchema]> {
        self.modules
            .get(plugin)?
            .blocks
            .get(block)
            .map(|b| b.slots.as_slice())
    }

    /// What a reporter of this world answers, when it is one.
    pub fn returns(&self, plugin: &str, block: &str) -> Option<&FieldType> {
        self.modules
            .get(plugin)?
            .blocks
            .get(block)?
            .returns
            .as_ref()
    }

    /// Asks a reporter's op with `args` in slot order and the asking
    /// `actor`; answers the `value` it returns.
    pub fn read(
        &mut self,
        plugin: &str,
        block: &str,
        args: &[Value],
        actor: &str,
    ) -> Result<Value, String> {
        let Some(spec) = self
            .modules
            .get(plugin)
            .and_then(|h| h.blocks.get(block))
            .cloned()
        else {
            return Err(format!("{plugin}/{block} doesn't answer in this run"));
        };
        if spec.slots.len() != args.len() {
            return Err(format!(
                "{plugin}/{block}: the block has {} slots, the project's has {}",
                spec.slots.len(),
                args.len()
            ));
        }
        let input = block_input(&spec, args, actor);
        let key = format!("{block}\u{1f}{input}");
        if let Some(hosted) = self.modules.get(plugin)
            && let Some(known) = hosted.reads.get(&key)
        {
            return Ok(known.clone());
        }
        let (outcomes, answer, missing) = self.call_answer(plugin, &spec.op, &input, false, false);
        for outcome in outcomes {
            match outcome {
                Outcome::Effect { .. } => self.error(
                    plugin,
                    format!("{}: a reporter can only answer, not act", spec.op),
                ),
                other => self.pending.push(other),
            }
        }
        if missing {
            return Err(format!(
                "{plugin}/{block}: the module has no op {}",
                spec.op
            ));
        }
        let Some(answer) = answer else {
            return Err(format!("{plugin}/{block} could not answer"));
        };
        let Some(value) = answer.get("value") else {
            return Err(format!("{plugin}/{block}: the answer has no value"));
        };
        if let Some(hosted) = self.modules.get_mut(plugin) {
            hosted.reads.insert(key, value.clone());
        }
        Ok(value.clone())
    }

    /// Asks a module op directly and answers what it said, for the editor's
    /// own questions (a tool's cast). Nothing it does is applied: this is a
    /// read, so effects in the answer are an error.
    pub fn query(&mut self, plugin: &str, op: &str, input: &Value) -> Result<Value, String> {
        if !self.modules.contains_key(plugin) {
            return Err(format!("{plugin} is not hosted here"));
        }
        let (outcomes, answer, missing) = self.call_answer(plugin, op, input, false, false);
        for outcome in outcomes {
            match outcome {
                Outcome::Effect { .. } => {
                    self.error(plugin, format!("{op}: a query can only answer, not act"))
                }
                other => self.pending.push(other),
            }
        }
        if missing {
            return Err(format!("{plugin}: the module has no op {op}"));
        }
        answer.ok_or_else(|| format!("{plugin}/{op} could not answer"))
    }

    /// Drops every remembered read. The world calls this at the start of
    /// each frame, since what a module reports may move with it.
    pub fn forget_reads(&mut self) {
        for hosted in self.modules.values_mut() {
            hosted.reads.clear();
        }
    }

    /// Runs a block's op with `args` in slot order and the running `actor`.
    pub fn run_block(
        &mut self,
        plugin: &str,
        block: &str,
        args: &[Value],
        actor: &str,
    ) -> Vec<Outcome> {
        let Some(spec) = self
            .modules
            .get(plugin)
            .and_then(|h| h.blocks.get(block))
            .cloned()
        else {
            let mut out = self.drain();
            out.push(Outcome::Error {
                plugin: plugin.to_string(),
                message: format!("{block}: no block of that name runs here"),
            });
            return out;
        };
        let mut out = self.drain();
        if spec.slots.len() != args.len() {
            out.push(Outcome::Error {
                plugin: plugin.to_string(),
                message: format!(
                    "{block}: the block has {} slots, the project's has {}",
                    spec.slots.len(),
                    args.len()
                ),
            });
            return out;
        }
        let input = block_input(&spec, args, actor);
        out.extend(self.call(plugin, &spec.op, &input, false));
        out
    }

    /// One call: the module's logs, then its answer's effects, or the error.
    /// `quiet_unsupported` skips an op the module does not have.
    fn call(
        &mut self,
        plugin: &str,
        op: &str,
        input: &Value,
        quiet_unsupported: bool,
    ) -> Vec<Outcome> {
        self.call_op(plugin, op, input, quiet_unsupported).0
    }

    /// [`WorldPlugins::call`], also saying whether the module had no such op.
    fn call_op(
        &mut self,
        plugin: &str,
        op: &str,
        input: &Value,
        quiet_unsupported: bool,
    ) -> (Vec<Outcome>, bool) {
        let (out, _, missing) = self.call_answer(plugin, op, input, quiet_unsupported, true);
        (out, missing)
    }

    /// One call, handing back the module's answer too. A call that is not a
    /// read (`acts`) may change what the module reports, so it forgets reads.
    fn call_answer(
        &mut self,
        plugin: &str,
        op: &str,
        input: &Value,
        quiet_unsupported: bool,
        acts: bool,
    ) -> (Vec<Outcome>, Option<Value>, bool) {
        let Some(hosted) = self.modules.get_mut(plugin) else {
            return (Vec::new(), None, false);
        };
        if acts {
            hosted.reads.clear();
        }
        let mut missing = false;
        let started = Instant::now();
        let answer = hosted.module.call_json(op, input);
        self.diagnostics.record_call(plugin, op, started.elapsed());
        let mut out: Vec<Outcome> = hosted
            .module
            .take_logs()
            .into_iter()
            .map(|(level, text)| Outcome::Log {
                plugin: plugin.to_string(),
                level,
                text,
            })
            .collect();
        let mut value = None;
        match answer {
            Ok(answer) => {
                out.extend(effects_of(plugin, op, &answer, hosted.gpu));
                value = Some(answer);
            }
            Err(error) if is_unsupported(&error) => {
                missing = true;
                if !quiet_unsupported {
                    out.push(Outcome::Error {
                        plugin: plugin.to_string(),
                        message: format!("{op}: the module has no such op"),
                    });
                }
            }
            Err(error) => {
                out.push(Outcome::Error {
                    plugin: plugin.to_string(),
                    message: error,
                });
                // A module that trapped or ran out of budget has lost its
                // state; keeping it would run the game on half of one.
                if hosted.module.is_stopped() {
                    self.modules.remove(plugin);
                    out.push(Outcome::Error {
                        plugin: plugin.to_string(),
                        message: "was stopped and is off for the rest of this run".to_string(),
                    });
                }
            }
        }
        (out, value, missing)
    }
}

/// The JSON a block's op is called with: its slots by name, plus the actor
/// when the command asks for one.
fn block_input(spec: &LoadoutBlock, args: &[Value], actor: &str) -> Value {
    let mut object = Map::new();
    for (slot, value) in spec.slots.iter().zip(args) {
        object.insert(slot.name.clone(), value.clone());
    }
    if spec.wants_actor && !object.contains_key("actor") {
        object.insert("actor".to_string(), json!(actor));
    }
    Value::Object(object)
}

fn effects_of(plugin: &str, op: &str, answer: &Value, gpu_allowed: bool) -> Vec<Outcome> {
    let Some(list) = answer.get("effects") else {
        return Vec::new();
    };
    let Some(list) = list.as_array() else {
        return vec![Outcome::Error {
            plugin: plugin.to_string(),
            message: format!("{op}: effects must be a list"),
        }];
    };
    list.iter()
        .map(
            |item| match serde_json::from_value::<Effect>(item.clone()) {
                Ok(Effect::Mesh(mesh)) if mesh.check().is_err() => Outcome::Error {
                    plugin: plugin.to_string(),
                    message: format!("{op}: {}", mesh.check().unwrap_err()),
                },
                Ok(Effect::Instances(set)) if set.check().is_err() => Outcome::Error {
                    plugin: plugin.to_string(),
                    message: format!("{op}: {}", set.check().unwrap_err()),
                },
                Ok(effect) if effect.is_gpu() && !gpu_allowed => Outcome::Error {
                    plugin: plugin.to_string(),
                    message: format!("{op}: GPU effects need the gpu-compute capability"),
                },
                Ok(effect) if matches!(effect.gpu_command(), Some(Err(_))) => Outcome::Error {
                    plugin: plugin.to_string(),
                    message: format!("{op}: {}", effect.gpu_command().unwrap().unwrap_err()),
                },
                Ok(effect) => Outcome::Effect {
                    plugin: plugin.to_string(),
                    effect,
                },
                Err(error) => Outcome::Error {
                    plugin: plugin.to_string(),
                    message: format!("{op}: {error}"),
                },
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::active::ActivePlugins;
    use crate::cache::Cache;
    use crate::install::{Change, Environment, install};
    use crate::lock::ProjectPlugins;
    use crate::native::{NativeModule, default_services};
    use crate::package::fixtures::portable;
    use crate::source::Source;
    use blockloom_plugin_api::Version;
    use blockloom_plugin_api::abi::{Buffer, HostApi, PluginApi, Slice};
    use std::cell::RefCell;

    thread_local! {
        // How many times the scripted plugin has been asked to `count`.
        static ASKED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        // One host table per module started on this thread; a module's handle
        // is its index plus one.
        static HOSTS: RefCell<Vec<*const HostApi>> = const { RefCell::new(Vec::new()) };
    }

    fn answer(out: *mut Buffer, value: Value) -> i32 {
        unsafe { *out = Buffer::from_vec(serde_json::to_vec(&value).unwrap()) };
        0
    }

    /// A plugin written straight against the C layout: hooks that say what
    /// tick they ran on, a block op, and ways to go wrong.
    unsafe extern "C" fn scripted_call(
        handle: u64,
        op: Slice,
        input: Slice,
        out: *mut Buffer,
    ) -> i32 {
        let result = std::panic::catch_unwind(|| {
            let op = std::str::from_utf8(unsafe { op.as_bytes() }).unwrap();
            let input: Value = serde_json::from_slice(unsafe { input.as_bytes() }).unwrap();
            match op {
                "world.start" => {
                    let host = unsafe { &*HOSTS.with_borrow(|hosts| hosts[handle as usize - 1]) };
                    let line = format!(
                        "start {} with {}",
                        input["plugin"].as_str().unwrap_or("?"),
                        input["records"].as_array().map_or(0, Vec::len)
                    );
                    unsafe { (host.log.unwrap())(host.ctx, 3, Slice::of(line.as_bytes())) };
                    answer(out, Value::Null)
                }
                "hook.tick" => answer(
                    out,
                    json!({"effects": [{"effect": "say", "text": format!("tick {}", input["tick"])}]}),
                ),
                "hook.late" => answer(
                    out,
                    json!({"effects": [{"effect": "broadcast", "message": input["stage"]}]}),
                ),
                "hook.ghost" => 3,
                "add" => answer(
                    out,
                    json!({"effects": [{
                        "effect": "say",
                        "text": format!("{} {} {}", input["name"], input["by"], input["actor"]),
                    }]}),
                ),
                "count" => {
                    let n = ASKED.with(|asked| {
                        asked.set(asked.get() + 1);
                        asked.get()
                    });
                    answer(out, json!({"value": n}))
                }
                "bare" => answer(out, json!({})),
                // A job that counts to `args.to`, a slice at a time.
                "job.count" => {
                    if input["cancel"] == true {
                        return answer(
                            out,
                            json!({"effects": [{"effect": "say", "text": "cancelled"}]}),
                        );
                    }
                    let at = input["state"].as_u64().unwrap_or(0) + 1;
                    let to = input["args"]["to"].as_u64().unwrap_or(1);
                    if at >= to {
                        answer(out, json!({"done": true, "result": {"counted": at}}))
                    } else {
                        answer(
                            out,
                            json!({"state": at, "progress": at as f64 / to as f64,
                                   "effects": [{"effect": "say", "text": format!("at {at}")}]}),
                        )
                    }
                }
                "node.noise" => answer(
                    out,
                    json!({"outputs": {"height": input["params"]["scale"].as_f64().unwrap_or(1.0)}}),
                ),
                "node.double" => answer(
                    out,
                    json!({"outputs": {"out": input["inputs"]["in"].as_f64().unwrap_or(0.0) * 2.0}}),
                ),
                "job.fail" => answer(out, json!({"error": "no cells to make"})),
                "ping" => answer(
                    out,
                    json!({"effects": [{"effect": "event", "name": "pinged", "args": [input["by"]]}]}),
                ),
                "dance" => answer(out, json!({"effects": [{"effect": "dance"}]})),
                "list" => answer(out, json!({"effects": 3})),
                "boom" => panic!("boom"),
                _ => 3,
            }
        });
        result.unwrap_or(8)
    }

    unsafe extern "C" fn scripted_free(_handle: u64, buffer: Buffer) {
        drop(unsafe { buffer.into_vec() });
    }

    unsafe extern "C" fn scripted_shutdown(_handle: u64) {}

    unsafe extern "C" fn scripted_entry(host: *const HostApi, out: *mut PluginApi) -> i32 {
        let handle = HOSTS.with_borrow_mut(|hosts| {
            hosts.push(host);
            hosts.len() as u64
        });
        unsafe {
            *out = PluginApi {
                handle,
                call: Some(scripted_call),
                free_buffer: Some(scripted_free),
                shutdown: Some(scripted_shutdown),
                ..PluginApi::empty()
            };
        }
        0
    }

    fn scripted() -> CodeModule {
        let module = unsafe {
            NativeModule::from_entry(
                scripted_entry,
                BTreeSet::new(),
                default_services("test".to_string()),
            )
        }
        .unwrap();
        CodeModule::Native(module)
    }

    fn hook(name: &str, stage: Stage, after: &[&str]) -> HookSchema {
        HookSchema {
            name: name.to_string(),
            stage,
            before: Vec::new(),
            after: after.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn block(
        type_id: &str,
        op: &str,
        slots: &[(&str, FieldType)],
        wants_actor: bool,
    ) -> LoadoutBlock {
        LoadoutBlock {
            type_id: type_id.to_string(),
            op: op.to_string(),
            slots: slots
                .iter()
                .map(|(name, ty)| FieldSchema {
                    name: name.to_string(),
                    ty: ty.clone(),
                    default: None,
                    description: String::new(),
                    ui: None,
                })
                .collect(),
            wants_actor,
            returns: None,
        }
    }

    fn said(outcomes: &[Outcome]) -> Vec<String> {
        outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Effect {
                    plugin,
                    effect: Effect::Say { text },
                } => Some(format!("{plugin}: {text}")),
                _ => None,
            })
            .collect()
    }

    fn errors(outcomes: &[Outcome]) -> Vec<String> {
        outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Error { plugin, message } => Some(format!("{plugin}: {message}")),
                _ => None,
            })
            .collect()
    }

    fn two_plugins() -> WorldPlugins {
        let preloaded = |id: &str, hooks: Vec<HookSchema>, blocks: Vec<LoadoutBlock>| Preloaded {
            id: id.to_string(),
            module: scripted(),
            hooks,
            blocks,
        };
        WorldPlugins::with_modules(vec![
            // `b` is listed first but asks to run after `a`.
            preloaded(
                "b",
                vec![hook("tick", Stage::FixedSimulation, &["a/tick"])],
                vec![],
            ),
            preloaded(
                "a",
                vec![
                    hook("tick", Stage::FixedSimulation, &[]),
                    hook("late", Stage::PostPhysics, &[]),
                    hook("ghost", Stage::FixedSimulation, &[]),
                ],
                vec![block(
                    "add",
                    "add",
                    &[
                        ("name", FieldType::Text { max_len: None }),
                        (
                            "by",
                            FieldType::Int {
                                min: None,
                                max: None,
                            },
                        ),
                    ],
                    true,
                )],
            ),
        ])
        .unwrap()
    }

    #[test]
    fn hooks_run_by_stage_in_their_declared_order() {
        let mut world = two_plugins();
        let outcomes = world.run_stage(Stage::FixedSimulation, 7, 0.02);
        assert_eq!(said(&outcomes), ["a: tick 7", "b: tick 7"]);
        // Another stage runs only its own hooks.
        let late = world.run_stage(Stage::PostPhysics, 7, 0.02);
        assert_eq!(
            late,
            [Outcome::Effect {
                plugin: "a".to_string(),
                effect: Effect::Broadcast {
                    message: "post_physics".to_string()
                }
            }]
        );
        assert!(world.run_stage(Stage::Input, 7, 0.02).is_empty());
    }

    #[test]
    fn a_hook_without_an_op_is_reported_once() {
        let mut world = two_plugins();
        let first = world.run_stage(Stage::FixedSimulation, 1, 0.02);
        assert_eq!(errors(&first), ["a: hook.ghost: the module has no such op"]);
        let second = world.run_stage(Stage::FixedSimulation, 2, 0.02);
        assert!(errors(&second).is_empty(), "{second:?}");
        assert_eq!(said(&second), ["a: tick 2", "b: tick 2"]);
    }

    #[test]
    fn a_cycle_between_hooks_refuses_the_set() {
        let looped = |id: &str, other: &str| Preloaded {
            id: id.to_string(),
            module: scripted(),
            hooks: vec![hook(
                "tick",
                Stage::FixedSimulation,
                &[&format!("{other}/tick")],
            )],
            blocks: vec![],
        };
        let error = WorldPlugins::with_modules(vec![looped("a", "b"), looped("b", "a")])
            .err()
            .unwrap();
        assert!(error.contains("cycle"), "{error}");
    }

    #[test]
    fn start_hands_each_plugin_its_records_and_a_missing_stop_is_quiet() {
        let mut world = two_plugins();
        let outcomes = world.start(&|id| json!({"records": vec![id; id.len() + 1]}));
        let logs: Vec<_> = outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Log { plugin, text, .. } => Some(format!("{plugin}: {text}")),
                _ => None,
            })
            .collect();
        assert_eq!(logs, ["a: start a with 2", "b: start b with 2"]);
        assert!(errors(&outcomes).is_empty());
        // The scripted plugin has no `world.stop`; that is not a fault.
        assert!(world.stop().is_empty());
    }

    #[test]
    fn a_block_runs_its_op_with_its_slots_and_the_running_actor() {
        let mut world = two_plugins();
        assert!(world.has_block("a", "add") && !world.has_block("b", "add"));
        assert_eq!(world.block_slots("a", "add").unwrap().len(), 2);
        let outcomes = world.run_block("a", "add", &[json!("coins"), json!(3)], "actor-9");
        assert_eq!(said(&outcomes), [r#"a: "coins" 3 "actor-9""#]);
        let few = world.run_block("a", "add", &[json!("coins")], "actor-9");
        assert_eq!(
            errors(&few),
            ["a: add: the block has 2 slots, the project's has 1"]
        );
        let unknown = world.run_block("a", "nope", &[], "actor-9");
        assert_eq!(
            errors(&unknown),
            ["a: nope: no block of that name runs here"]
        );
    }

    #[test]
    fn bad_answers_and_panics_are_reported_and_the_plugin_stays() {
        let mut world = WorldPlugins::with_modules(vec![Preloaded {
            id: "a".to_string(),
            module: scripted(),
            hooks: vec![],
            blocks: vec![
                block("dance", "dance", &[], false),
                block("list", "list", &[], false),
                block("boom", "boom", &[], false),
                block("fine", "hook.tick", &[], false),
            ],
        }])
        .unwrap();
        let dance = errors(&world.run_block("a", "dance", &[], ""));
        assert!(
            dance[0].starts_with("a: dance: unknown variant `dance`"),
            "{dance:?}"
        );
        let list = errors(&world.run_block("a", "list", &[], ""));
        assert_eq!(list, ["a: list: effects must be a list"]);
        let boom = errors(&world.run_block("a", "boom", &[], ""));
        assert_eq!(boom, ["a: boom: Panicked"]);
        // A native panic is contained; the module still answers.
        assert_eq!(
            said(&world.run_block("a", "fine", &[], "")).len(),
            1,
            "still hosted"
        );
    }

    fn reading_world() -> WorldPlugins {
        let mut counted = block(
            "count",
            "count",
            &[("by", FieldType::Text { max_len: None })],
            false,
        );
        counted.returns = Some(FieldType::Int {
            min: None,
            max: None,
        });
        WorldPlugins::with_modules(vec![Preloaded {
            id: "a".to_string(),
            module: scripted(),
            hooks: vec![hook("tick", Stage::FixedSimulation, &[])],
            blocks: vec![
                counted,
                block("bare", "bare", &[], false),
                block(
                    "ping",
                    "ping",
                    &[("by", FieldType::Text { max_len: None })],
                    false,
                ),
            ],
        }])
        .unwrap()
    }

    #[test]
    fn a_reporter_is_asked_once_until_something_else_runs() {
        ASKED.with(|asked| asked.set(0));
        let mut world = reading_world();
        assert!(world.returns("a", "count").is_some());
        let first = world.read("a", "count", &[json!("x")], "me").unwrap();
        assert_eq!(first, json!(1));
        // The same question is remembered, a different one is asked.
        assert_eq!(
            world.read("a", "count", &[json!("x")], "me").unwrap(),
            json!(1)
        );
        assert_eq!(
            world.read("a", "count", &[json!("y")], "me").unwrap(),
            json!(2)
        );
        // A hook runs, so the answers may have moved.
        world.run_stage(Stage::FixedSimulation, 1, 0.02);
        assert_eq!(
            world.read("a", "count", &[json!("x")], "me").unwrap(),
            json!(3)
        );
        world.forget_reads();
        assert_eq!(
            world.read("a", "count", &[json!("x")], "me").unwrap(),
            json!(4)
        );
    }

    #[test]
    fn a_read_that_cannot_answer_says_why() {
        let mut world = reading_world();
        let wrong = world.read("a", "count", &[], "me").unwrap_err();
        assert!(wrong.contains("1 slots"), "{wrong}");
        let none = world.read("a", "bare", &[], "me").unwrap_err();
        assert!(none.contains("no value"), "{none}");
        let ghost = world.read("a", "ghost", &[], "me").unwrap_err();
        assert!(ghost.contains("doesn't answer"), "{ghost}");
        let missing = world.read("zzz", "count", &[], "me").unwrap_err();
        assert!(missing.contains("doesn't answer"), "{missing}");
    }

    #[test]
    fn a_block_can_fire_the_plugins_own_event() {
        let mut world = reading_world();
        let outcomes = world.run_block("a", "ping", &[json!("coins")], "me");
        assert_eq!(
            outcomes,
            [Outcome::Effect {
                plugin: "a".to_string(),
                effect: Effect::Event {
                    name: "pinged".to_string(),
                    actor: None,
                    args: vec![json!("coins")],
                }
            }]
        );
    }

    #[test]
    fn mesh_effects_are_read_and_checked() {
        let triangle = json!({
            "effect": "mesh", "name": "t",
            "positions": [0, 0, 0, 1, 0, 0, 0, 1, 0],
            "normals": [0, 0, 1, 0, 0, 1, 0, 0, 1],
            "colors": [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            "indices": [0, 1, 2], "collider": true,
        });
        let mut bad = triangle.clone();
        bad["indices"] = json!([0, 1, 9]);
        let outcomes = effects_of(
            "p",
            "build",
            &json!({"effects": [triangle, bad, {"effect": "remove_mesh", "name": "t"}]}),
            false,
        );
        assert!(matches!(
            &outcomes[0],
            Outcome::Effect { effect: Effect::Mesh(mesh), .. } if mesh.collider
        ));
        assert!(
            errors(&outcomes[1..2])[0].contains("past 3"),
            "{outcomes:?}"
        );
        assert_eq!(
            outcomes[2],
            Outcome::Effect {
                plugin: "p".to_string(),
                effect: Effect::RemoveMesh {
                    name: "t".to_string()
                }
            }
        );
    }

    #[test]
    fn gpu_effects_need_the_capability_and_become_commands() {
        let effects = json!({"effects": [
            {"effect": "gpu_buffer", "name": "a", "words": 8},
            {"effect": "gpu_write", "buffer": "a", "offset": 2, "f32": [1.5, 2.0]},
            {"effect": "gpu_dispatch", "kernel": "k", "bindings": {"input": "a"}, "groups": [4]},
            {"effect": "gpu_read", "buffer": "a", "words": 8, "tag": "t", "as": "f32"},
            {"effect": "gpu_free", "buffer": "a"},
            {"effect": "gpu_write", "buffer": "a", "f32": [1.0], "u32": [1]},
            {"effect": "gpu_dispatch", "kernel": "k", "bindings": {}, "groups": [1, 2, 3, 4]},
        ]});
        let refused = effects_of("p", "go", &effects, false);
        assert_eq!(refused.len(), 7);
        for outcome in &refused[..5] {
            assert!(
                errors(std::slice::from_ref(outcome))[0].contains("gpu-compute"),
                "{outcome:?}"
            );
        }
        let allowed = effects_of("p", "go", &effects, true);
        let commands: Vec<GpuCommand> = allowed[..5]
            .iter()
            .map(|o| match o {
                Outcome::Effect { effect, .. } => effect.gpu_command().unwrap().unwrap(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            commands[1],
            GpuCommand::Write {
                buffer: "a".to_string(),
                offset: 2,
                data: vec![1.5f32.to_bits(), 2.0f32.to_bits()],
            }
        );
        assert_eq!(
            commands[2],
            GpuCommand::Dispatch {
                kernel: "k".to_string(),
                bindings: vec![("input".to_string(), "a".to_string())],
                groups: [4, 1, 1],
            }
        );
        assert!(matches!(
            commands[3],
            GpuCommand::Read {
                as_type: ReadAs::F32,
                ..
            }
        ));
        // Mixed value types and too many axes are refused with a reason.
        assert!(errors(&allowed[5..6])[0].contains("exactly one"));
        assert!(errors(&allowed[6..7])[0].contains("1 to 3"));
    }

    struct Rig {
        dir: tempfile::TempDir,
        loadout: blockloom_plugin_api::loadout::Loadout,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectPlugins::new(dir.path().join("project"));
        std::fs::create_dir_all(&project.dir).unwrap();
        let env = Environment {
            cache: Cache::new(dir.path().join("cache")),
            engine: Version::new(0, 0, 1),
            target: "x86_64-unknown-linux-gnu".to_string(),
            offline: false,
            registries: BTreeMap::new(),
        };
        let pkg = portable(&dir.path().join("src"), "com.example.code", "1.0.0");
        install(
            &env,
            &project,
            Change::Add {
                id: "com.example.code".into(),
                req: "*".parse().unwrap(),
                source: Some(Source::Path(pkg)),
                features: vec![],
            },
        )
        .unwrap();
        let loadout = ActivePlugins::load(&project, &env.cache, &env.target, &env.engine).loadout();
        Rig { dir, loadout }
    }

    #[test]
    fn a_loadout_opens_a_portable_module_and_runs_its_blocks() {
        let rig = rig();
        let mut world = WorldPlugins::load(&rig.loadout, "0.0.1");
        assert_eq!(world.plugins().collect::<Vec<_>>(), ["com.example.code"]);
        // The test module has no `hook.tick`, which a declared hook must have.
        let tick = world.run_stage(Stage::FixedSimulation, 1, 0.02);
        assert_eq!(
            errors(&tick),
            ["com.example.code: hook.tick: the module has no such op"]
        );
        // `echo` answers its arguments, which carry no effects.
        let echoed = world.run_block("com.example.code", "echo_block", &[json!("hi")], "a1");
        assert!(echoed.is_empty(), "{echoed:?}");
        let _ = &rig.dir;
    }

    #[test]
    fn a_changed_portable_plugin_is_replaced_and_a_native_one_is_not() {
        let rig = rig();
        let records = |_: &str| json!({"records": [], "resources": []});
        let mut world = WorldPlugins::load(&rig.loadout, "0.0.1");

        // The same loadout changes nothing.
        let (out, report) = world.reload(&rig.loadout, &rig.loadout, &records);
        assert!(out.is_empty() && report.is_empty(), "{out:?} {report:?}");

        // A new build of a portable module replaces it.
        let mut newer = rig.loadout.clone();
        if let CodeRuntime::Portable(library) = &mut newer.plugins[0].runtime {
            library.hash = "newer".to_string();
        }
        let (_, report) = world.reload(&rig.loadout, &newer, &records);
        assert_eq!(report.reloaded, ["com.example.code"]);
        assert!(report.kept_state.is_empty());
        assert_eq!(world.plugins().collect::<Vec<_>>(), ["com.example.code"]);

        // A native library is never swapped in a running game.
        let mut native = newer.clone();
        native.plugins[0].runtime =
            CodeRuntime::Native(blockloom_plugin_api::loadout::NativeLibrary {
                path: rig.dir.path().join("lib.so"),
                hash: "n".to_string(),
                capabilities: BTreeSet::new(),
            });
        let (_, report) = world.reload(&newer, &native, &records);
        assert!(report.reloaded.is_empty());
        assert_eq!(report.restart_needed.len(), 1);
        assert!(report.restart_needed[0].1.contains("restart the run"));
        assert_eq!(world.plugins().count(), 1);

        // A plugin that goes is stopped and dropped.
        let (_, report) = world.reload(&newer, &Loadout::default(), &records);
        assert_eq!(report.removed, ["com.example.code"]);
        assert!(world.is_empty());

        // And one that comes is opened and started.
        let (_, report) = world.reload(&Loadout::default(), &rig.loadout, &records);
        assert_eq!(report.added, ["com.example.code"]);
        assert_eq!(world.plugins().count(), 1);
    }

    #[test]
    fn a_module_that_runs_out_of_budget_is_off_for_the_rest_of_the_run() {
        let rig = rig();
        let mut world = WorldPlugins::load(&rig.loadout, "0.0.1");
        let spun = world.run_block("com.example.code", "spin_block", &[], "a1");
        let errs = errors(&spun);
        assert!(errs[0].contains("5 ms of work"), "{errs:?}");
        assert!(errs[1].contains("off for the rest of this run"), "{errs:?}");
        assert!(world.is_empty());
        assert!(world.run_stage(Stage::FixedSimulation, 2, 0.02).is_empty());
    }

    #[test]
    fn a_plugin_that_cannot_load_is_reported_and_the_rest_still_run() {
        let mut rig = rig();
        let mut broken = rig.loadout.plugins[0].clone();
        broken.id = "com.example.broken".to_string();
        if let blockloom_plugin_api::loadout::CodeRuntime::Portable(library) = &mut broken.runtime {
            library.path = rig.dir.path().join("missing.wasm");
        }
        rig.loadout.plugins.push(broken);
        let mut world = WorldPlugins::load(&rig.loadout, "0.0.1");
        assert_eq!(world.plugins().collect::<Vec<_>>(), ["com.example.code"]);
        let pending = errors(&world.drain());
        assert_eq!(pending.len(), 1);
        assert!(
            pending[0].starts_with("com.example.broken: could not load"),
            "{pending:?}"
        );
        assert!(world.drain().is_empty());
    }

    fn counting_world() -> WorldPlugins {
        WorldPlugins::with_modules(vec![Preloaded {
            id: "a".to_string(),
            module: scripted(),
            hooks: vec![],
            blocks: vec![],
        }])
        .unwrap()
    }

    #[test]
    fn a_job_runs_a_slice_per_call_and_raises_its_event_when_done() {
        let mut world = counting_world();
        let table = world.job_table();
        let id = table
            .lock()
            .unwrap()
            .start("a", "count", json!({"to": 3}), Some("counted".into()), 0)
            .unwrap();
        let first = world.run_jobs(50.0);
        assert_eq!(said(&first), ["a: at 1"]);
        assert_eq!(world.running_jobs(), 1);
        assert_eq!(said(&world.run_jobs(50.0)), ["a: at 2"]);
        let last = world.run_jobs(50.0);
        assert_eq!(world.running_jobs(), 0);
        let raised: Vec<_> = last
            .iter()
            .filter_map(|o| match o {
                Outcome::Effect {
                    effect: Effect::Event { name, args, .. },
                    ..
                } => Some((name.clone(), args.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            raised,
            [("counted".to_string(), vec![json!(id), json!("done")])]
        );
        let result = table.lock().unwrap().take("a", id).unwrap().result.unwrap();
        assert_eq!(result["counted"], 3);
    }

    #[test]
    fn a_cancelled_job_is_told_once_and_a_failing_one_is_reported() {
        let mut world = counting_world();
        let table = world.job_table();
        let slow = table
            .lock()
            .unwrap()
            .start("a", "count", json!({"to": 100}), None, 0)
            .unwrap();
        world.run_jobs(50.0);
        assert_eq!(world.cancel_jobs("a"), 1);
        assert_eq!(said(&world.run_jobs(50.0)), ["a: cancelled"]);
        assert!(world.run_jobs(50.0).is_empty());
        assert_eq!(world.running_jobs(), 0);
        let _ = slow;
        table
            .lock()
            .unwrap()
            .start("a", "fail", Value::Null, None, 0)
            .unwrap();
        let failed = errors(&world.run_jobs(50.0));
        assert_eq!(failed, ["a: job 2: no cells to make"]);
        // An op the module lacks fails the job, not the run.
        table
            .lock()
            .unwrap()
            .start("a", "ghost", Value::Null, None, 0)
            .unwrap();
        let missing = errors(&world.run_jobs(50.0));
        assert!(missing.iter().any(|e| e.contains("job 3")), "{missing:?}");
    }

    #[test]
    fn a_zero_budget_still_gives_one_slice_so_jobs_make_progress() {
        let mut world = counting_world();
        let table = world.job_table();
        table
            .lock()
            .unwrap()
            .start("a", "count", json!({"to": 9}), None, 0)
            .unwrap();
        table
            .lock()
            .unwrap()
            .start("a", "count", json!({"to": 9}), None, 0)
            .unwrap();
        assert_eq!(said(&world.run_jobs(0.0)).len(), 1);
        assert_eq!(said(&world.run_jobs(0.0)).len(), 1);
        // Generous budget: every job gets exactly one slice.
        assert_eq!(said(&world.run_jobs(1000.0)).len(), 2);
    }

    #[test]
    fn a_graph_job_evaluates_a_node_per_slice_and_reuses_the_cache() {
        let nodes: Vec<NodeSchema> = serde_json::from_value(json!([
            {"name": "noise", "title": "Noise", "outputs": [{"name": "height", "type": "number"}],
             "params": [{"name": "scale", "type": "number", "default": 1.0}]},
            {"name": "double", "title": "Double", "inputs": [{"name": "in", "type": "number"}],
             "outputs": [{"name": "out", "type": "number"}]}
        ]))
        .unwrap();
        let mut world = counting_world();
        world.set_nodes("a", nodes);
        let table = world.job_table();
        let args = json!({
            "graph": {
                "nodes": [{"id": "n", "node": "noise", "params": {"scale": 4.0}}, {"id": "d", "node": "double"}],
                "edges": [{"from": "n.height", "to": "d.in"}],
                "outputs": ["d.out"]
            },
            "tile": {"origin": [0, 0, 0], "size": [4, 4, 4]}
        });
        let first = table
            .lock()
            .unwrap()
            .start("a", GRAPH_JOB, args.clone(), Some("tile".into()), 0)
            .unwrap();
        world.run_jobs(50.0);
        assert_eq!(world.running_jobs(), 1, "one node is not the whole graph");
        let ended = world.run_jobs(50.0);
        assert_eq!(world.running_jobs(), 0);
        assert!(ended.iter().any(|o| matches!(o, Outcome::Effect { effect: Effect::Event { name, .. }, .. } if name == "tile")));
        let result = table
            .lock()
            .unwrap()
            .take("a", first)
            .unwrap()
            .result
            .unwrap();
        assert_eq!(result["outputs"]["d.out"], 8.0);
        assert_eq!(
            (result["evaluated"].as_u64(), result["cached"].as_u64()),
            (Some(2), Some(0))
        );
        // The same graph again comes straight from the cache.
        let second = table
            .lock()
            .unwrap()
            .start("a", GRAPH_JOB, args, None, 0)
            .unwrap();
        world.run_jobs(50.0);
        world.run_jobs(50.0);
        let again = table
            .lock()
            .unwrap()
            .take("a", second)
            .unwrap()
            .result
            .unwrap();
        assert_eq!(
            (again["evaluated"].as_u64(), again["cached"].as_u64()),
            (Some(0), Some(2))
        );
        // A graph the plugin has no nodes for fails the job, not the world.
        let bad = table
            .lock()
            .unwrap()
            .start("a", GRAPH_JOB, json!({"graph": {"nodes": [{"id": "x", "node": "nope"}]}, "tile": {"origin": [0,0,0], "size": [1,1,1]}}), None, 0)
            .unwrap();
        let errs = errors(&world.run_jobs(50.0));
        assert!(
            errs[0].contains(&format!("job {bad}")) && errs[0].contains("no node nope"),
            "{errs:?}"
        );
    }
}
