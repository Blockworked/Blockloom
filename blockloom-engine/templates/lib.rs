//! A starter engine plugin: a `BlockloomModule` that adds its own resource
//! and systems to the game world. It counts the fixed ticks a run advanced
//! and says the total in the run log when the run ends.

use blockloom_engine::prelude::*;
use blockloom_engine::{SimulationSet, say};

pub struct StarterModule;

/// Ticks advanced since Play.
#[derive(Resource, Default, Debug)]
pub struct Ticks {
    pub count: u64,
    running: bool,
}

impl BlockloomModule for StarterModule {
    fn name(&self) -> &'static str {
        "starter"
    }

    fn build(&self, app: &mut App) {
        app.init_resource::<Ticks>()
            .add_systems(FixedUpdate, count.in_set(SimulationSet))
            .add_systems(Update, report);
    }
}

blockloom_engine::export_module!(StarterModule);

fn count(run: Res<RunState>, mut ticks: ResMut<Ticks>) {
    if run.running && !run.paused {
        ticks.count += 1;
    }
}

/// Says the count once a run ends, and starts over.
fn report(run: Res<RunState>, mut ticks: ResMut<Ticks>) {
    if run.running == ticks.running {
        return;
    }
    if ticks.running {
        say("", &format!("{} ticks", ticks.count));
        ticks.count = 0;
    }
    ticks.running = run.running;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_counts_only_while_running() {
        let mut app = App::new();
        app.init_resource::<RunState>()
            .init_resource::<Ticks>()
            .add_systems(Update, count);
        app.update();
        app.world_mut().resource_mut::<RunState>().running = true;
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Ticks>().count, 2);
    }
}
