//! Plugin code in the running world.
//!
//! The editor sends a [`Loadout`] before Play. When the run begins the world
//! opens each plugin's module, gives it its records, and from then on calls
//! its hooks once per stage and runs the plugin blocks whose commands are
//! module ops. A plugin answers with effects the world applies in order; it
//! never touches the world itself. The modules go when the run ends.
//!
//! Without the `plugins` feature (the web and Android players) the loadout is
//! ignored and every call here does nothing; a project with plugin code is
//! refused for those targets at build time.

#[cfg(feature = "plugins")]
use crate::bridge;
use crate::engine::Engine;
use bevy::prelude::*;
#[cfg(feature = "plugins")]
use blockloom_core::vm::Event;
use blockloom_plugin_api::loadout::Loadout;
use blockloom_plugin_api::schema::Stage;
#[cfg(feature = "plugins")]
use blockloom_protocol::RuntimeMessage;
use serde_json::Value;
#[cfg(feature = "plugins")]
use serde_json::json;

#[cfg(feature = "plugins")]
use blockloom_plugin_host::world::{Effect, Outcome, WorldPlugins};

/// The loadout the editor sent, and the modules a run has open.
#[derive(Default)]
pub struct PluginHost {
    pub loadout: Loadout,
    #[cfg(feature = "plugins")]
    world: Option<WorldPlugins>,
    #[cfg(feature = "plugins")]
    ticks: u64,
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
        }
    }
}

/// A plugin's share of the project: its records on actors and its resources.
#[cfg(feature = "plugins")]
fn records_for(engine: &Engine, plugin: &str) -> Value {
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
    json!({"records": records, "resources": resources})
}

/// The run begins: open the modules and tell each its records.
pub fn begin(engine: &mut Engine) {
    #[cfg(feature = "plugins")]
    {
        end(engine);
        if engine.plugins.loadout.is_empty() {
            return;
        }
        let mut world = WorldPlugins::load(&engine.plugins.loadout, env!("CARGO_PKG_VERSION"));
        let started = world.start(&|plugin| records_for(engine, plugin));
        engine.plugins.world = Some(world);
        engine.plugins.ticks = 0;
        apply(engine, applied(started));
    }
    #[cfg(not(feature = "plugins"))]
    let _ = engine;
}

/// The run ends: tell the modules, then unload them.
pub fn end(engine: &mut Engine) {
    #[cfg(feature = "plugins")]
    if let Some(mut world) = engine.plugins.world.take() {
        let stopped = world.stop();
        drop(world);
        engine.plugins.ticks = 0;
        apply(engine, applied(stopped));
    }
    #[cfg(not(feature = "plugins"))]
    let _ = engine;
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
        let Some(world) = engine.plugins.world.as_mut() else {
            return false;
        };
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
        let Some(world) = engine.plugins.world.as_mut() else {
            return false;
        };
        let outcomes = world.run_block(plugin, block, &args, actor);
        apply(engine, applied(outcomes));
        true
    }
    #[cfg(not(feature = "plugins"))]
    {
        let _ = (engine, actor, plugin, block, args);
        false
    }
}

/// The system that runs one stage's hooks. Hooks wait while the game is
/// paused or not yet running, except the two stages that follow frames.
pub fn stage(stage: Stage) -> impl FnMut(NonSendMut<Engine>, Res<Time>) {
    move |mut engine: NonSendMut<Engine>, time: Res<Time>| {
        #[cfg(feature = "plugins")]
        {
            if !engine.plugins.active() || !engine.running {
                return;
            }
            let frame_stage = matches!(stage, Stage::RenderExtraction | Stage::Presentation);
            if engine.paused && !frame_stage {
                return;
            }
            if stage == Stage::FixedSimulation {
                engine.plugins.ticks += 1;
            }
            let tick = engine.plugins.ticks;
            let dt = f64::from(time.delta_secs());
            let Some(world) = engine.plugins.world.as_mut() else {
                return;
            };
            let outcomes = world.run_stage(stage, tick, dt);
            apply(&mut engine, applied(outcomes));
        }
        #[cfg(not(feature = "plugins"))]
        let _ = (&mut engine, &time, stage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Mode;

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
}
