//! Plugin code in the running world.
//!
//! The editor sends a [`Loadout`] before Play. When the run begins the world
//! opens each plugin's module, gives it its records, and from then on calls
//! its hooks once per stage and runs the plugin blocks whose commands are
//! module ops. A plugin answers with effects the world applies in order; it
//! never touches the world itself. The modules go when the run ends.
//!
//! A plugin reporter is answered on demand: while a run has modules open, the
//! world installs a reader on `sense` that calls the reporter's op, so a
//! value slot asks the module the moment the VM evaluates it. A plugin hat
//! starts on an `event` effect a module answers with.
//!
//! Without the `plugins` feature (the web and Android players) the loadout is
//! ignored and every call here does nothing; a project with plugin code is
//! refused for those targets at build time.

#[cfg(feature = "plugins")]
use crate::bridge;
use crate::engine::Engine;
use bevy::prelude::*;
#[cfg(feature = "plugins")]
use blockloom_core::sense;
#[cfg(feature = "plugins")]
use blockloom_core::value::{Evaluated, evaluated_from_json, json_of};
#[cfg(feature = "plugins")]
use blockloom_core::vm::Event;
use blockloom_plugin_api::loadout::Loadout;
use blockloom_plugin_api::mesh::MeshData;
use blockloom_plugin_api::rendering::InstanceData;
use blockloom_plugin_api::schema::Stage;
#[cfg(feature = "plugins")]
use blockloom_protocol::RuntimeMessage;
use serde_json::Value;
#[cfg(feature = "plugins")]
use serde_json::json;

#[cfg(feature = "plugins")]
use blockloom_plugin_api::schema::FieldType;
#[cfg(feature = "plugins")]
use blockloom_plugin_host::world::{Effect, Outcome, WorldPlugins};
#[cfg(feature = "plugins")]
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// A change to the meshes plugins have drawn, waiting for
/// `plugin_meshes::sync` to carry it out.
#[cfg_attr(not(feature = "plugins"), allow(dead_code))]
pub enum MeshOp {
    Put {
        plugin: String,
        mesh: MeshData,
    },
    Remove {
        plugin: String,
        name: String,
    },
    Visibility {
        plugin: String,
        name: String,
        visible: bool,
    },
    /// Draws copies of a mesh the plugin already submitted.
    Instances {
        plugin: String,
        set: InstanceData,
    },
    RemoveInstances {
        plugin: String,
        name: String,
    },
    /// The run ended: every plugin's meshes go.
    Clear,
}

/// The loadout the editor sent, and the modules a run has open.
#[derive(Default)]
pub struct PluginHost {
    pub loadout: Loadout,
    pub meshes: Vec<MeshOp>,
    /// How many times a preview module has been started, so what was cast
    /// against an older one can be told from what is in the scene now.
    pub previews: u64,
    /// A plugin changed the ground; the next navigation sync bakes again.
    pub nav_dirty: std::cell::Cell<bool>,
    /// Bumped when the plugins' shader modules change, so they are registered again.
    pub shaders_serial: u64,
    /// Bumped when the plugins' compute kernels change.
    #[cfg(feature = "plugins")]
    pub kernels_serial: u64,
    /// GPU commands plugins asked for, waiting for `plugin_compute::feed`.
    #[cfg(feature = "plugins")]
    pub gpu: Vec<(String, blockloom_plugin_api::compute::GpuCommand)>,
    /// The run ended: the compute engine drops the plugins' buffers.
    #[cfg(feature = "plugins")]
    pub gpu_clear: bool,
    /// Status reports since the plugin diagnostics last went to the editor.
    reports: u32,
    /// The diagnostics the editor last heard, so an unchanged set is not resent.
    reported: String,
    /// What the open modules cost and report; kept across a preview and a run.
    #[cfg(feature = "plugins")]
    pub diagnostics: Arc<blockloom_plugin_host::diagnostics::Diagnostics>,
    #[cfg(feature = "plugins")]
    world: Option<Rc<RefCell<WorldPlugins>>>,
    #[cfg(feature = "plugins")]
    ticks: u64,
    /// What the scene-view preview was started from, when the open modules
    /// are a preview and not a run.
    #[cfg(feature = "plugins")]
    previewing: Option<String>,
}

impl PluginHost {
    pub fn active(&self) -> bool {
        #[cfg(feature = "plugins")]
        {
            self.world.is_some()
        }
        #[cfg(not(feature = "plugins"))]
        {
            false
        }
    }
}

/// What the world needs from a plugin's outcome, minus the host crate's types.
#[cfg(feature = "plugins")]
enum Applied {
    Log(String),
    Error(String),
    Say(String),
    Broadcast(String),
    Event {
        plugin: String,
        name: String,
        actor: Option<String>,
        args: Vec<String>,
    },
    Mesh {
        plugin: String,
        mesh: MeshData,
    },
    RemoveMesh {
        plugin: String,
        name: String,
    },
    MeshVisibility {
        plugin: String,
        name: String,
        visible: bool,
    },
    NavDirty,
    Instances {
        plugin: String,
        set: InstanceData,
    },
    RemoveInstances {
        plugin: String,
        name: String,
    },
    Gpu {
        plugin: String,
        command: blockloom_plugin_api::compute::GpuCommand,
    },
}

#[cfg(feature = "plugins")]
fn applied(outcomes: Vec<Outcome>) -> Vec<Applied> {
    outcomes
        .into_iter()
        .map(|outcome| match outcome {
            Outcome::Log {
                plugin,
                level,
                text,
            } if level <= blockloom_plugin_api::abi::LOG_ERROR => {
                Applied::Error(format!("{plugin}: {text}"))
            }
            Outcome::Log { plugin, text, .. } => Applied::Log(format!("[{plugin}] {text}")),
            Outcome::Error { plugin, message } if plugin.is_empty() => Applied::Error(message),
            Outcome::Error { plugin, message } => Applied::Error(format!("{plugin}: {message}")),
            Outcome::Effect { plugin, effect } => match effect {
                Effect::Say { text } => Applied::Say(format!("[{plugin}] {text}")),
                Effect::Broadcast { message } => Applied::Broadcast(message),
                Effect::Error { message } => Applied::Error(format!("{plugin}: {message}")),
                Effect::Event { name, actor, args } => Applied::Event {
                    plugin,
                    name,
                    actor,
                    args: args.iter().map(event_arg).collect(),
                },
                Effect::Mesh(mesh) => Applied::Mesh { plugin, mesh },
                Effect::RemoveMesh { name } => Applied::RemoveMesh { plugin, name },
                Effect::MeshVisibility { name, visible } => Applied::MeshVisibility {
                    plugin,
                    name,
                    visible,
                },
                Effect::NavDirty { .. } => Applied::NavDirty,
                Effect::Instances(set) => Applied::Instances { plugin, set },
                Effect::RemoveInstances { name } => Applied::RemoveInstances { plugin, name },
                gpu => match gpu.gpu_command() {
                    Some(Ok(command)) => match command.check() {
                        Ok(()) => Applied::Gpu { plugin, command },
                        Err(e) => Applied::Error(format!("{plugin}: {e}")),
                    },
                    Some(Err(e)) => Applied::Error(format!("{plugin}: {e}")),
                    None => Applied::Error(format!("{plugin}: an effect the world cannot apply")),
                },
            },
        })
        .collect()
}

#[cfg(feature = "plugins")]
fn apply(engine: &mut Engine, outcomes: Vec<Applied>) {
    for outcome in outcomes {
        match outcome {
            Applied::Log(text) | Applied::Say(text) => bridge::send(&RuntimeMessage::Say {
                actor: String::new(),
                text,
            }),
            Applied::Error(message) => bridge::send(&RuntimeMessage::Error {
                actor: String::new(),
                message,
            }),
            Applied::Broadcast(message) => engine.fire(Event::Message(message)),
            Applied::Event {
                plugin,
                name,
                actor,
                args,
            } => engine.fire(Event::Plugin {
                plugin,
                event: name,
                args,
                actor,
            }),
            Applied::Mesh { .. }
            | Applied::MeshVisibility { .. }
            | Applied::RemoveMesh { .. }
            | Applied::Instances { .. }
            | Applied::RemoveInstances { .. }
                if !engine.project.active_scene().world.mode.is_3d() =>
            {
                bridge::send(&RuntimeMessage::Error {
                    actor: String::new(),
                    message: "a plugin drew a mesh, which only a 3D game can show".to_string(),
                });
            }
            Applied::Mesh { plugin, mesh } => {
                engine.plugins.meshes.push(MeshOp::Put { plugin, mesh })
            }
            Applied::MeshVisibility {
                plugin,
                name,
                visible,
            } => engine.plugins.meshes.push(MeshOp::Visibility {
                plugin,
                name,
                visible,
            }),
            Applied::RemoveMesh { plugin, name } => {
                engine.plugins.meshes.push(MeshOp::Remove { plugin, name })
            }
            Applied::Instances { plugin, set } => engine
                .plugins
                .meshes
                .push(MeshOp::Instances { plugin, set }),
            Applied::RemoveInstances { plugin, name } => engine
                .plugins
                .meshes
                .push(MeshOp::RemoveInstances { plugin, name }),
            Applied::NavDirty => engine.plugins.nav_dirty.set(true),
            Applied::Gpu { plugin, command } => engine.plugins.gpu.push((plugin, command)),
        }
    }
}

/// What a plugin's event carries for a hat's slot to match, as text.
#[cfg(feature = "plugins")]
fn event_arg(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A plugin's share of the project: its records on actors and its resources.
#[cfg(feature = "plugins")]
fn records_for(engine: &Engine, plugin: &str, preview: bool) -> Value {
    let records: Vec<Value> = engine
        .project
        .actors
        .iter()
        .flat_map(|actor| {
            actor
                .components
                .plugin_records()
                .filter(|r| r.plugin == plugin)
                .map(|r| {
                    json!({
                        "actor": actor.id,
                        "type_id": r.type_id,
                        "schema_version": r.schema_version,
                        "payload": r.payload,
                    })
                })
        })
        .collect();
    let resources: Vec<Value> = engine
        .project
        .plugin_resources
        .iter()
        .filter(|r| r.plugin == plugin)
        .map(|r| {
            json!({
                "type_id": r.type_id,
                "schema_version": r.schema_version,
                "payload": r.payload,
            })
        })
        .collect();
    json!({"records": records, "resources": resources, "preview": preview})
}

/// What a built game's player hosts, from the plugins its pack records and
/// the files the build copied beside it. Fails with every problem found; a
/// player that cannot load a plugin's code must not run the game without it.
pub fn shipped_loadout(
    game: &std::path::Path,
    plugins: &[blockloom_core::pack::PackedPlugin],
) -> Result<Loadout, Vec<String>> {
    #[cfg(feature = "plugins")]
    {
        use blockloom_plugin_host::shipped::{Shipped, shipped_loadout};
        let dirs: Vec<std::path::PathBuf> = plugins.iter().map(|p| game.join(&p.dir)).collect();
        let shipped: Vec<Shipped> = plugins
            .iter()
            .zip(&dirs)
            .map(|(p, dir)| Shipped {
                id: &p.id,
                dir,
                hash: &p.hash,
                files: &p.files,
            })
            .collect();
        #[cfg(target_arch = "wasm32")]
        let target = "wasm32-unknown-unknown";
        #[cfg(all(target_os = "android", target_arch = "aarch64"))]
        let target = "aarch64-linux-android";
        #[cfg(all(target_os = "android", target_arch = "x86_64"))]
        let target = "x86_64-linux-android";
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        let target = blockloom_core::build::host().map_or("unknown", |t| t.triple);
        shipped_loadout(&shipped, target)
    }
    #[cfg(not(feature = "plugins"))]
    {
        let _ = game;
        match plugins.iter().find(|p| p.tier != "declarative") {
            Some(p) => Err(vec![format!(
                "{} has code, and this player was built without plugin support",
                p.id
            )]),
            None => Ok(Loadout::default()),
        }
    }
}

/// Publishes the navigation mesh for plugin queries (`nav.*` services).
pub fn publish_nav(nav: &crate::world::NavMesh) {
    #[cfg(feature = "plugins")]
    crate::plugin_services::publish_nav(nav);
    #[cfg(not(feature = "plugins"))]
    let _ = nav;
}

/// The services every module of this world is opened with: its project's
/// data, the player's saves and the run's diagnostics.
#[cfg(feature = "plugins")]
fn host_services(engine: &Engine) -> blockloom_plugin_host::services::HostServices {
    use blockloom_plugin_api::data::DATA_DIR;
    use blockloom_plugin_host::storage::{BlobStore, DiskStore, PackStore};
    let mut host = blockloom_plugin_host::services::HostServices::new(env!("CARGO_PKG_VERSION"))
        .with_diagnostics(engine.plugins.diagnostics.clone())
        .with_provider(Arc::new(crate::plugin_services::WorldQueries));
    if let Some(dir) = engine.project_dir.as_deref() {
        let data = dir.join(DATA_DIR);
        // A built game's folder carries a pack; its data is shipped and fixed.
        let store: Arc<dyn BlobStore> = if blockloom_core::pack::pack_path(dir).is_file() {
            Arc::new(PackStore::open(data))
        } else {
            Arc::new(DiskStore::new(data, false))
        };
        host = host.with_project_store(store);
    }
    // The browser keeps plugin saves in localStorage beside the variable saves.
    #[cfg(target_arch = "wasm32")]
    let saves: Arc<dyn BlobStore> = Arc::new(blockloom_plugin_host::storage::KvStore::new(
        crate::web::LocalKv,
        crate::web::plugin_save_prefix(&engine.project.id),
    ));
    #[cfg(not(target_arch = "wasm32"))]
    let saves: Arc<dyn BlobStore> =
        Arc::new(DiskStore::new(save_dir_for(&engine.project.id), false));
    host.with_save_store(saves)
}

/// Where a project's plugin saves live, beside its variable saves.
#[cfg(all(feature = "plugins", not(target_arch = "wasm32")))]
fn save_dir_for(project_id: &str) -> std::path::PathBuf {
    crate::world::save_path_for(project_id).with_extension("plugins")
}

/// The run begins: open the modules and tell each its records.
pub fn begin(engine: &mut Engine) {
    #[cfg(feature = "plugins")]
    {
        end(engine);
        if engine.plugins.loadout.is_empty() {
            return;
        }
        engine.plugins.diagnostics.reset();
        let world = WorldPlugins::load_with(&engine.plugins.loadout, &host_services(engine));
        install(engine, world);
    }
    #[cfg(not(feature = "plugins"))]
    let _ = engine;
}

/// Starts the opened modules and hosts them for the run.
#[cfg(feature = "plugins")]
fn install(engine: &mut Engine, mut world: WorldPlugins) {
    let started = world.start(&|plugin| records_for(engine, plugin, false));
    let world = Rc::new(RefCell::new(world));
    let reader = Rc::clone(&world);
    sense::set_plugin_reader(Some(Box::new(move |plugin, block, args| {
        read(&reader, plugin, block, args)
    })));
    engine.plugins.world = Some(world);
    engine.plugins.ticks = 0;
    apply(engine, applied(started));
}

/// The editor sent a different loadout while a game runs: replace what
/// changed (see `WorldPlugins::reload`) and say what happened. `old` is the
/// loadout the run was started with, or last reloaded to.
pub fn reload(engine: &mut Engine, old: &Loadout) {
    #[cfg(feature = "plugins")]
    {
        let Some(world) = engine.plugins.world.clone() else {
            return;
        };
        if !engine.running || old.plugins == engine.plugins.loadout.plugins {
            return;
        }
        let records = |plugin: &str| records_for(engine, plugin, false);
        let (outcomes, report) = {
            let Ok(mut world) = world.try_borrow_mut() else {
                return;
            };
            world.reload(old, &engine.plugins.loadout, &records)
        };
        apply(engine, applied(outcomes));
        let mut lines = Vec::new();
        for id in &report.reloaded {
            let kept = if report.kept_state.contains(id) {
                ", keeping its state"
            } else {
                ""
            };
            lines.push(format!("[plugins] reloaded {id}{kept}"));
        }
        lines.extend(
            report
                .added
                .iter()
                .map(|id| format!("[plugins] started {id}")),
        );
        lines.extend(
            report
                .removed
                .iter()
                .map(|id| format!("[plugins] stopped {id}")),
        );
        for text in lines {
            bridge::send(&RuntimeMessage::Say {
                actor: String::new(),
                text,
            });
        }
        for (id, why) in report.restart_needed {
            bridge::send(&RuntimeMessage::Error {
                actor: String::new(),
                message: format!("{id} changed: {why}"),
            });
        }
    }
    #[cfg(not(feature = "plugins"))]
    let _ = (engine, old);
}

/// Hosts the plugins that draw a preview while nothing plays, so their meshes
/// show in the scene view. Called whenever the world is loaded, the loadout
/// changes or a run ends; the modules are kept while what they were started
/// from is unchanged. Hooks and blocks stay off: only the start is run.
pub fn preview(engine: &mut Engine) {
    #[cfg(feature = "plugins")]
    {
        let host = host_services(engine);
        preview_with(engine, |wanted| WorldPlugins::load_with(wanted, &host));
    }
    #[cfg(not(feature = "plugins"))]
    let _ = engine;
}

/// `preview`, with the way modules are opened handed in so a test can host
/// one without a package on disk.
#[cfg(feature = "plugins")]
fn preview_with(engine: &mut Engine, open: impl FnOnce(&Loadout) -> WorldPlugins) {
    {
        // A built game has no scene view: its run is all there is.
        if engine.running || engine.starting || engine.link.is_some() {
            return;
        }
        let plugins: Vec<_> = engine
            .plugins
            .loadout
            .plugins
            .iter()
            .filter(|p| p.preview)
            .cloned()
            .collect();
        let kernels = engine
            .plugins
            .loadout
            .kernels
            .iter()
            .filter(|k| plugins.iter().any(|p| p.id == k.plugin))
            .cloned()
            .collect();
        let wanted = Loadout {
            plugins,
            shaders: Vec::new(),
            kernels,
        };
        if wanted.is_empty() || !engine.project.active_scene().world.mode.is_3d() {
            if engine.plugins.previewing.is_some() {
                end(engine);
            }
            return;
        }
        let key = json!([
            &wanted,
            wanted
                .plugins
                .iter()
                .map(|p| records_for(engine, &p.id, true))
                .collect::<Vec<_>>()
        ])
        .to_string();
        if engine.plugins.previewing.as_deref() == Some(key.as_str()) {
            return;
        }
        end(engine);
        let mut world = open(&wanted);
        let started = world.start(&|plugin| records_for(engine, plugin, true));
        engine.plugins.world = Some(Rc::new(RefCell::new(world)));
        engine.plugins.previewing = Some(key);
        engine.plugins.previews += 1;
        // A preview draws; what it says belongs to a run's log.
        let drawn = applied(started)
            .into_iter()
            .filter(|a| !matches!(a, Applied::Say(_) | Applied::Log(_) | Applied::Broadcast(_)))
            .collect();
        apply(engine, drawn);
    }
}

/// The run ends: tell the modules, then unload them.
pub fn end(engine: &mut Engine) {
    #[cfg(feature = "plugins")]
    if let Some(world) = engine.plugins.world.take() {
        // Reporters stop answering before the modules are told to stop.
        sense::set_plugin_reader(None);
        let stopped = world.borrow_mut().stop();
        drop(world);
        engine.plugins.ticks = 0;
        engine.plugins.previewing = None;
        apply(engine, applied(stopped));
        engine.plugins.meshes.push(MeshOp::Clear);
        engine.plugins.gpu.clear();
        engine.plugins.gpu_clear = true;
    }
    #[cfg(not(feature = "plugins"))]
    let _ = engine;
}

/// Hands the compute engine's reports back: an error goes to the run log and
/// a finished read to the plugin that asked for it, as its `gpu.result` op.
#[cfg(feature = "plugins")]
pub fn gpu_reports(engine: &mut Engine, reports: Vec<blockloom_plugin_gpu::engine::Report>) {
    use blockloom_plugin_gpu::engine::Report;
    let Some(world) = engine.plugins.world.clone() else {
        return;
    };
    for report in reports {
        let outcomes = match report {
            Report::Error { plugin, message } => vec![Outcome::Error { plugin, message }],
            Report::Read {
                plugin,
                tag,
                buffer,
                offset,
                as_type,
                words,
            } => {
                world
                    .borrow_mut()
                    .deliver_gpu(&plugin, &tag, &buffer, offset, as_type.json(&words))
            }
        };
        apply(engine, applied(outcomes));
    }
}

/// Answers a plugin reporter: slot values in as JSON, the module's `value`
/// out as the type the schema says it returns.
#[cfg(feature = "plugins")]
fn read(
    world: &Rc<RefCell<WorldPlugins>>,
    plugin: &str,
    block: &str,
    args: &[Evaluated],
) -> Result<Evaluated, String> {
    let mut world = world
        .try_borrow_mut()
        .map_err(|_| format!("{plugin}/{block} was asked while its plugin was busy"))?;
    let Some(slots) = world.block_slots(plugin, block) else {
        return Err(format!("{plugin}/{block} doesn't answer in this run"));
    };
    let json: Vec<Value> = slots
        .iter()
        .zip(args)
        .map(|(slot, value)| match (&slot.ty, value) {
            (FieldType::Actor, Evaluated::Text(name)) => {
                let id = sense::read(|s| {
                    if s.actors.contains_key(name) {
                        return Some(name.clone());
                    }
                    s.actors
                        .iter()
                        .filter(|(_, a)| a.name.eq_ignore_ascii_case(name))
                        .map(|(id, _)| id.clone())
                        .min()
                });
                Value::String(id.unwrap_or_else(|| name.clone()))
            }
            (_, value) => json_of(value),
        })
        .collect();
    let returns = world.returns(plugin, block).cloned();
    let actor = sense::current_actor().unwrap_or_default();
    let answer = world.read(plugin, block, &json, &actor)?;
    let value = evaluated_from_json(&answer).map_err(|e| format!("{plugin}/{block} {e}"))?;
    match (returns, &value) {
        (Some(FieldType::Bool), Evaluated::Bool(_)) => Ok(value),
        (Some(FieldType::Int { .. } | FieldType::Number { .. }), Evaluated::Number(_)) => Ok(value),
        (Some(FieldType::Bool | FieldType::Int { .. } | FieldType::Number { .. }), _) => Err(
            format!("{plugin}/{block} answered {answer}, which isn't what it reports"),
        ),
        (_, Evaluated::Number(n)) => Ok(Evaluated::Text(Evaluated::Number(*n).as_text())),
        _ => Ok(value),
    }
}

/// Casts the pointer's ray through a plugin tool's cast op in the hosted
/// module. A hit is the module's whole answer; a miss, or no module, is
/// nothing. `report` says whether a failing cast reaches the editor's log
/// (a click does, a hover every frame would flood it).
pub fn tool_cast(
    engine: &mut Engine,
    tool: &blockloom_protocol::PluginToolView,
    ray: Ray3d,
    report: bool,
) -> Option<Value> {
    #[cfg(feature = "plugins")]
    {
        if engine.running {
            return None;
        }
        let world = engine.plugins.world.clone()?;
        let origin = ray.origin;
        let dir = *ray.direction;
        let input = json!({
            "x": origin.x, "y": origin.y, "z": origin.z,
            "dx": dir.x, "dy": dir.y, "dz": dir.z,
            "reach": tool.reach,
        });
        let hit = world.borrow_mut().query(&tool.plugin, &tool.cast, &input);
        let hit = match hit {
            Ok(hit) => hit,
            Err(message) => {
                if report {
                    bridge::send(&RuntimeMessage::Error {
                        actor: String::new(),
                        message,
                    });
                }
                return None;
            }
        };
        (hit.get("hit").and_then(Value::as_bool) == Some(true)).then_some(hit)
    }
    #[cfg(not(feature = "plugins"))]
    {
        let _ = (engine, tool, ray, report);
        None
    }
}

/// What a plugin overlay draws right now: the hosted module's answer to
/// `overlay.<name>`, which only looks at the world. No module hosted is an
/// empty overlay.
pub fn overlay_shapes(
    engine: &mut Engine,
    view: &blockloom_protocol::PluginOverlayView,
    selected: Option<&str>,
    camera: [f32; 3],
) -> Result<Vec<blockloom_plugin_api::surfaces::Shape>, String> {
    #[cfg(feature = "plugins")]
    {
        let Some(world) = engine.plugins.world.clone() else {
            return Ok(Vec::new());
        };
        let input = json!({"selected": selected, "camera": camera});
        let op = format!("overlay.{}", view.overlay);
        let answer = world.borrow_mut().query(&view.plugin, &op, &input)?;
        blockloom_plugin_api::surfaces::parse_shapes(&answer)
            .map_err(|e| format!("{}/{}: {e}", view.plugin, view.overlay))
    }
    #[cfg(not(feature = "plugins"))]
    {
        let _ = (engine, view, selected, camera);
        Ok(Vec::new())
    }
}

/// Called with each status report: the plugins' own readings for the
/// profiler, and about once a second the whole diagnostics to the editor when
/// they changed.
pub fn report(engine: &mut Engine) -> Vec<(String, f64)> {
    #[cfg(feature = "plugins")]
    {
        if !engine.plugins.active() {
            return Vec::new();
        }
        engine.plugins.reports += 1;
        if engine.plugins.reports >= 5 {
            engine.plugins.reports = 0;
            let snapshot = engine.plugins.diagnostics.snapshot();
            let text = snapshot.to_string();
            if text != engine.plugins.reported {
                engine.plugins.reported = text;
                bridge::send(&RuntimeMessage::PluginDiagnostics { snapshot });
            }
        }
        engine.plugins.diagnostics.metrics()
    }
    #[cfg(not(feature = "plugins"))]
    {
        let _ = engine;
        Vec::new()
    }
}

/// A stroke of a plugin tool as the message the editor runs it from.
pub fn tool_stroke(
    tool: &blockloom_protocol::PluginToolView,
    hits: Vec<Value>,
) -> blockloom_protocol::RuntimeMessage {
    RuntimeMessage::PluginTool {
        plugin: tool.plugin.clone(),
        tool: tool.tool.clone(),
        hits,
        options: tool.options.clone(),
    }
}

/// The box a hit asks the editor to outline, in world units.
pub fn tool_outline(tool: &blockloom_protocol::PluginToolView, hit: &Value) -> Option<[f32; 6]> {
    blockloom_plugin_api::schema::box_at(hit, &tool.outline).map(|b| b.map(|n| n as f32))
}

/// Runs a plugin block here when its command is a module op; returns false
/// for the editor to run it (a block whose command edits the project).
pub fn run_block(
    engine: &mut Engine,
    actor: &str,
    plugin: &str,
    block: &str,
    args: &[Value],
) -> bool {
    #[cfg(feature = "plugins")]
    {
        if engine.plugins.previewing.is_some() {
            return false;
        }
        let Some(world) = engine.plugins.world.clone() else {
            return false;
        };
        let mut world = world.borrow_mut();
        if !world.has_block(plugin, block) {
            return false;
        }
        // An actor slot's text names an actor; the plugin gets its id.
        let slots = world.block_slots(plugin, block).unwrap_or_default();
        let args: Vec<Value> = slots
            .iter()
            .zip(args)
            .map(|(slot, value)| match (&slot.ty, value) {
                (blockloom_plugin_api::schema::FieldType::Actor, Value::String(name)) => {
                    let id = engine
                        .project
                        .actors
                        .iter()
                        .find(|a| a.id == *name)
                        .or_else(|| engine.project.actors.iter().find(|a| a.name == *name))
                        .map_or(name.clone(), |a| a.id.clone());
                    Value::String(id)
                }
                _ => value.clone(),
            })
            .chain(args.iter().skip(slots.len()).cloned())
            .collect();
        let outcomes = world.run_block(plugin, block, &args, actor);
        drop(world);
        apply(engine, applied(outcomes));
        true
    }
    #[cfg(not(feature = "plugins"))]
    {
        let _ = (engine, actor, plugin, block, args);
        false
    }
}

/// How long the fixed tick spends on plugin jobs, at most (one slice always runs).
#[cfg(feature = "plugins")]
const JOB_BUDGET_MS: f64 = 4.0;

#[cfg(feature = "plugins")]
fn perspective_view(
    pose: &GlobalTransform,
    projection: &PerspectiveProjection,
    size: Vec2,
) -> Value {
    json!({"position":pose.translation().to_array(),"forward":pose.forward().to_array(),
        "up":pose.up().to_array(),"viewport_width":size.x,"viewport_height":size.y,
        "fov_y":projection.fov.to_degrees()})
}

/// The system that runs one stage's hooks. Hooks wait while the game is
/// paused or not yet running, except the two stages that follow frames.
pub fn stage(
    stage: Stage,
) -> impl FnMut(
    NonSendMut<Engine>,
    Res<Time>,
    crate::queries::QueryAccess,
    Query<(&GlobalTransform, &Projection, &Camera), With<crate::world::WorldCamera>>,
) {
    move |mut engine: NonSendMut<Engine>,
          time: Res<Time>,
          queries: crate::queries::QueryAccess,
          cameras: Query<
        (&GlobalTransform, &Projection, &Camera),
        With<crate::world::WorldCamera>,
    >| {
        #[cfg(feature = "plugins")]
        {
            if engine.plugins.previewing.is_some() {
                if stage == Stage::Presentation
                    && let Some(world) = engine.plugins.world.clone()
                {
                    let outcomes = world
                        .borrow_mut()
                        .preview_frame(f64::from(time.delta_secs()));
                    apply(&mut engine, applied(outcomes));
                }
                return;
            }
            if !engine.plugins.active() || !engine.running {
                return;
            }
            let frame_stage = matches!(stage, Stage::RenderExtraction | Stage::Presentation);
            if engine.paused && !frame_stage {
                return;
            }
            if stage == Stage::FixedSimulation {
                engine.plugins.ticks += 1;
                engine.plugins.diagnostics.set_tick(engine.plugins.ticks);
            }
            let tick = engine.plugins.ticks;
            let dt = f64::from(time.delta_secs());
            let Some(world) = engine.plugins.world.clone() else {
                return;
            };
            let view = if stage == Stage::Presentation {
                cameras
                    .iter()
                    .find_map(|(pose, projection, camera)| {
                        let Projection::Perspective(projection) = projection else {
                            return None;
                        };
                        let size = camera.logical_viewport_size()?;
                        if !camera.is_active || size.x <= 0.0 || size.y <= 0.0 {
                            return None;
                        }
                        Some(perspective_view(pose, projection, size))
                    })
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            let outcomes = queries.scope(engine.contact_ticks, || {
                let mut world = world.borrow_mut();
                // What a reporter answered last step may have moved.
                if matches!(stage, Stage::Input | Stage::Presentation) {
                    world.forget_reads();
                }
                let mut outcomes = world.run_stage_with_view(stage, tick, dt, view);
                // Jobs get their slices once per fixed tick, after the hooks.
                if stage == Stage::FixedSimulation {
                    outcomes.extend(world.run_jobs(JOB_BUDGET_MS));
                }
                outcomes
            });
            apply(&mut engine, applied(outcomes));
        }
        #[cfg(not(feature = "plugins"))]
        let _ = (&mut engine, &time, &queries, &cameras, stage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Mode;
    #[test]
    #[cfg(feature = "plugins")]
    fn perspective_hooks_receive_world_pose_and_logical_viewport() {
        let pose = GlobalTransform::from(Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        let projection = PerspectiveProjection {
            fov: 60.0_f32.to_radians(),
            ..default()
        };
        let view = perspective_view(&pose, &projection, Vec2::new(1280.0, 720.0));
        assert_eq!(view["position"], json!([1.0, 2.0, 3.0]));
        assert_eq!(view["forward"], json!([0.0, 0.0, -1.0]));
        assert_eq!(view["up"], json!([0.0, 1.0, 0.0]));
        assert_eq!(view["viewport_width"], 1280.0);
        assert!((view["fov_y"].as_f64().unwrap() - 60.0).abs() < 0.001);
    }

    fn engine() -> Engine {
        let (_tx, rx) = std::sync::mpsc::channel();
        Engine::new(rx, Mode::TwoD)
    }

    #[test]
    fn a_run_with_no_plugin_code_does_nothing() {
        let mut engine = engine();
        begin(&mut engine);
        assert!(!engine.plugins.active());
        assert!(!run_block(&mut engine, "a", "p", "b", &[]));
        end(&mut engine);
    }

    #[test]
    fn stage_systems_run_without_modules() {
        let mut app = App::new();
        app.insert_non_send(engine())
            .init_resource::<Time>()
            .add_systems(
                Update,
                (stage(Stage::Input), stage(Stage::Presentation)).chain(),
            );
        app.update();
    }

    #[cfg(feature = "plugins")]
    #[test]
    fn gpu_effects_become_queued_commands_and_bad_ones_do_not() {
        use blockloom_plugin_api::compute::GpuCommand;
        let mut engine = engine();
        let effect = |effect| Outcome::Effect {
            plugin: "p".to_string(),
            effect,
        };
        let outcomes = vec![
            effect(Effect::GpuBuffer {
                name: "a".to_string(),
                words: 8,
            }),
            // Too many words for one buffer: refused before it is queued.
            effect(Effect::GpuBuffer {
                name: "b".to_string(),
                words: u32::MAX,
            }),
            effect(Effect::GpuFree {
                buffer: "a".to_string(),
            }),
        ];
        let applied = applied(outcomes);
        assert!(matches!(applied[1], Applied::Error(_)));
        apply(&mut engine, applied);
        assert_eq!(
            engine.plugins.gpu,
            vec![
                (
                    "p".to_string(),
                    GpuCommand::Buffer {
                        name: "a".to_string(),
                        words: 8
                    }
                ),
                (
                    "p".to_string(),
                    GpuCommand::Free {
                        buffer: "a".to_string()
                    }
                ),
            ]
        );
    }

    #[cfg(feature = "plugins")]
    #[test]
    fn a_world_without_a_renderer_takes_the_commands_and_says_so() {
        use blockloom_plugin_api::compute::GpuCommand;
        let mut app = App::new();
        let mut host = engine();
        host.plugins.gpu.push((
            "p".to_string(),
            GpuCommand::Free {
                buffer: "a".to_string(),
            },
        ));
        app.insert_non_send(host);
        crate::plugin_compute::register(&mut app);
        app.update();
        assert!(
            app.world_mut()
                .non_send_mut::<Engine>()
                .plugins
                .gpu
                .is_empty()
        );
        let link = app.world().resource::<crate::plugin_compute::ComputeLink>();
        assert_eq!(link.available(), Some(false));
    }

    /// A real voxel module, through the effect path to meshes in the ECS.
    #[cfg(feature = "plugins")]
    #[test]
    fn a_voxel_world_becomes_solid_meshes() {
        use crate::plugin_meshes::{PluginMesh, PluginMeshes, sync};
        use bevy_rapier3d::prelude as rp;
        use blockloom_plugin_api::record::PluginRecord;
        use blockloom_plugin_host::module::CodeModule;
        use blockloom_plugin_host::native::NativeModule;
        use blockloom_plugin_host::world::Preloaded;

        const ID: &str = "com.blockworked.voxel";
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.project.plugin_resources.push(PluginRecord::new(
            ID,
            "world",
            1,
            json!({"preset": "flat", "size": [32, 16, 32]}),
        ));
        let module = unsafe {
            NativeModule::from_entry(
                blockloom_voxel::blockloom_plugin_entry_v1,
                Default::default(),
                blockloom_plugin_host::native::default_services("0.0.1".to_string()),
            )
        }
        .unwrap();
        let world = WorldPlugins::with_modules(vec![Preloaded {
            id: ID.to_string(),
            module: CodeModule::Native(module),
            hooks: vec![],
            blocks: vec![],
        }])
        .unwrap();
        install(&mut engine, world);
        assert!(engine.plugins.active());
        // A flat 32x16x32 world is four chunks with ground in each.
        assert_eq!(engine.plugins.meshes.len(), 4);

        let mut app = App::new();
        app.insert_non_send(engine)
            .init_resource::<PluginMeshes>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_systems(Update, sync);
        app.update();
        let drawn = app
            .world_mut()
            .query_filtered::<&Transform, (With<PluginMesh>, With<rp::Collider>)>()
            .iter(app.world())
            .count();
        assert_eq!(drawn, 4);

        // The run ending takes them all away again.
        let mut engine = app.world_mut().non_send_mut::<Engine>();
        end(&mut engine);
        assert!(!engine.plugins.active());
        app.update();
        assert!(app.world().resource::<PluginMeshes>().is_empty());
    }

    /// The scene view hosts a preview plugin while nothing plays, keeps it
    /// while nothing it was started from changes, and hands over to the run.
    #[cfg(feature = "plugins")]
    #[test]
    fn a_preview_plugin_draws_while_nothing_plays() {
        use blockloom_plugin_api::loadout::{CodeRuntime, LoadoutPlugin, NativeLibrary};
        use blockloom_plugin_api::record::PluginRecord;
        use blockloom_plugin_host::module::CodeModule;
        use blockloom_plugin_host::native::NativeModule;
        use blockloom_plugin_host::world::Preloaded;

        const ID: &str = "com.blockworked.voxel";
        let opened = std::cell::Cell::new(0);
        let open = |_: &Loadout| {
            opened.set(opened.get() + 1);
            let module = unsafe {
                NativeModule::from_entry(
                    blockloom_voxel::blockloom_plugin_entry_v1,
                    Default::default(),
                    blockloom_plugin_host::native::default_services("0.0.1".to_string()),
                )
            }
            .unwrap();
            WorldPlugins::with_modules(vec![Preloaded {
                id: ID.to_string(),
                module: CodeModule::Native(module),
                hooks: vec![],
                blocks: vec![],
            }])
            .unwrap()
        };
        let plugin = |preview| LoadoutPlugin {
            id: ID.to_string(),
            runtime: CodeRuntime::Native(NativeLibrary {
                path: "voxel".into(),
                hash: "h".into(),
                capabilities: Default::default(),
            }),
            hooks: vec![],
            blocks: vec![],
            preview,
            nodes: vec![],
        };
        let world = |size: u32| {
            PluginRecord::new(
                ID,
                "world",
                1,
                json!({"preset": "flat", "size": [size, 16, size]}),
            )
        };
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        engine.project.plugin_resources.push(world(32));

        // A plugin that did not ask for a preview stays unloaded.
        engine.plugins.loadout.plugins = vec![plugin(false)];
        preview_with(&mut engine, open);
        assert!(!engine.plugins.active());
        assert_eq!(opened.get(), 0);

        // One that did draws its world, without saying anything.
        engine.plugins.loadout.plugins = vec![plugin(true)];
        preview_with(&mut engine, open);
        assert!(engine.plugins.active());
        assert_eq!(engine.plugins.meshes.len(), 4);
        assert_eq!(opened.get(), 1);
        // Hooks and blocks stay off: a preview is not a run.
        assert!(!run_block(&mut engine, "a", ID, "set_voxel", &[]));

        // A plugin tool's click casts through the hosted module.
        let tool = blockloom_protocol::PluginToolView {
            plugin: ID.to_string(),
            tool: "paint".to_string(),
            cast: "cast".to_string(),
            reach: 100.0,
            outline: "before_box".to_string(),
            drag: true,
            options: json!({"material": "wood"}),
        };
        let down = Ray3d::new(Vec3::new(5.5, 15.5, 5.5), Dir3::NEG_Y);
        let Some(hit) = tool_cast(&mut engine, &tool, down, true) else {
            panic!("a ray at the floor hits it");
        };
        assert!(hit["cell"].is_array() && hit["before"].is_array());
        // The empty cell above the floor is one cell wide.
        let outline = tool_outline(&tool, &hit).expect("the cast reports a box");
        assert!((outline[3] - outline[0] - 1.0).abs() < 1e-3);
        let RuntimeMessage::PluginTool { hits, options, .. } = tool_stroke(&tool, vec![hit]) else {
            panic!("a stroke is a plugin tool message");
        };
        assert_eq!(hits.len(), 1);
        assert_eq!(options["material"], "wood");
        let up = Ray3d::new(Vec3::new(5.5, 15.5, 5.5), Dir3::Y);
        assert!(tool_cast(&mut engine, &tool, up, true).is_none());

        // The same world again keeps the module.
        engine.plugins.meshes.clear();
        preview_with(&mut engine, open);
        assert_eq!(opened.get(), 1);
        assert!(engine.plugins.meshes.is_empty());

        // Editing the resource starts it over: the old meshes go, new ones come.
        engine.project.plugin_resources.clear();
        engine.project.plugin_resources.push(world(16));
        preview_with(&mut engine, open);
        assert_eq!(opened.get(), 2);
        let clear = engine
            .plugins
            .meshes
            .iter()
            .position(|op| matches!(op, MeshOp::Clear))
            .unwrap();
        assert!(
            engine.plugins.meshes[..clear]
                .iter()
                .all(|op| matches!(op, MeshOp::Remove { .. }))
        );
        assert!(
            engine.plugins.meshes[clear + 1..]
                .iter()
                .all(|op| matches!(op, MeshOp::Put { .. }))
        );
        assert_eq!(
            engine
                .plugins
                .meshes
                .iter()
                .filter(|op| matches!(op, MeshOp::Put { .. }))
                .count(),
            1
        );

        // A run in progress is left alone, and the plugin going takes it away.
        engine.running = true;
        engine.plugins.loadout.plugins.clear();
        preview_with(&mut engine, open);
        assert!(engine.plugins.active());
        engine.running = false;
        preview_with(&mut engine, open);
        assert!(!engine.plugins.active());
    }
}
