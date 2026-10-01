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
//! "broadcast" | "error", ...}`; see [`Effect`].

use crate::hooks::{HookRef, order_hooks};
use crate::module::{CodeModule, is_unsupported};
use blockloom_plugin_api::loadout::{Loadout, LoadoutBlock, ops};
use blockloom_plugin_api::schema::{FieldSchema, HookSchema, Stage};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

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
}

#[derive(Default)]
pub struct WorldPlugins {
    modules: BTreeMap<String, Hosted>,
    order: Vec<HookRef>,
    pending: Vec<Outcome>,
}

impl WorldPlugins {
    /// Opens every module in `loadout`. A plugin that fails to open is left
    /// out and reported by the next call that returns outcomes
    /// ([`WorldPlugins::drain`]).
    pub fn load(loadout: &Loadout, engine: &str) -> WorldPlugins {
        let mut world = WorldPlugins::default();
        let mut hooks = Vec::new();
        for plugin in &loadout.plugins {
            match CodeModule::load(&plugin.runtime, engine) {
                Ok(module) => {
                    world.insert(plugin.id.clone(), module, &plugin.blocks);
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
            },
        );
    }

    fn error(&mut self, plugin: &str, message: String) {
        self.pending.push(Outcome::Error {
            plugin: plugin.to_string(),
            message,
        });
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
        let mut object = Map::new();
        for (slot, value) in spec.slots.iter().zip(args) {
            object.insert(slot.name.clone(), value.clone());
        }
        if spec.wants_actor && !object.contains_key("actor") {
            object.insert("actor".to_string(), json!(actor));
        }
        out.extend(self.call(plugin, &spec.op, &Value::Object(object), false));
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
        let Some(hosted) = self.modules.get_mut(plugin) else {
            return (Vec::new(), false);
        };
        let mut missing = false;
        let answer = hosted.module.call_json(op, input);
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
        match answer {
            Ok(answer) => out.extend(effects_of(plugin, op, &answer)),
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
        (out, missing)
    }
}

fn effects_of(plugin: &str, op: &str, answer: &Value) -> Vec<Outcome> {
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
    use blockloom_plugin_api::schema::FieldType;
    use std::cell::RefCell;

    thread_local! {
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
                })
                .collect(),
            wants_actor,
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
}
