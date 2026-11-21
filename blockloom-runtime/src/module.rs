//! The contract a world feature is built against: a `BlockloomModule`
//! registers its own resources and systems, keeps its run state in its own
//! resources (reset on `WorldRebuilt`), and reads the document through
//! `SceneDocument` and the run through `RunState` rather than the
//! main-thread `Engine`. Wind is the first one; the rest are ported to it
//! one at a time.

use crate::engine::Engine;
use bevy::prelude::*;

pub trait BlockloomModule: Send + Sync + 'static {
    /// What diagnostics and the module list call it.
    fn name(&self) -> &'static str;
    /// Adds its resources and systems.
    fn build(&self, app: &mut App);
}

/// The modules a world was built with, in order.
#[derive(Resource, Default, Debug)]
pub struct Modules(pub Vec<&'static str>);

pub trait AddModule {
    fn add_module(&mut self, module: impl BlockloomModule) -> &mut Self;
}

impl AddModule for App {
    fn add_module(&mut self, module: impl BlockloomModule) -> &mut Self {
        module.build(self);
        self.world_mut()
            .get_resource_or_init::<Modules>()
            .0
            .push(module.name());
        self
    }
}

/// Whether a run is live, as of the start of this tick or frame.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct RunState {
    pub running: bool,
    pub paused: bool,
}

/// Sent when the world is rebuilt from the document: a run started or
/// ended, a scene switched or the editor loaded a new copy. Modules drop
/// their run state on it.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct WorldRebuilt;

fn sync_run_state(engine: NonSend<Engine>, mut run: ResMut<RunState>) {
    let now = RunState {
        running: engine.running,
        paused: engine.paused,
    };
    if *run != now {
        *run = now;
    }
}

/// What every module relies on: `RunState` kept current at the head of each
/// tick and frame, and the `WorldRebuilt` message.
pub fn register(app: &mut App) {
    app.init_resource::<Modules>()
        .init_resource::<RunState>()
        .add_message::<WorldRebuilt>()
        .add_systems(FixedFirst, sync_run_state)
        .add_systems(First, sync_run_state)
        .add_systems(
            Update,
            sync_run_state
                .after(crate::world::pump_editor)
                .before(crate::world::rebuild_world),
        );
}
