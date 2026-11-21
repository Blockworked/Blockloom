//! Counts the fixed ticks a run advanced and reports them when it ends: an
//! ECS resource and two systems, with no C ABI or JSON in between.

use blockloom_engine::prelude::*;
use blockloom_engine::{SimulationSet, say};

pub struct TallyModule;

/// Ticks advanced since Play.
#[derive(Resource, Default, Debug)]
pub struct Tally {
    pub ticks: u64,
    running: bool,
}

impl BlockloomModule for TallyModule {
    fn name(&self) -> &'static str {
        "com.blockworked.tally-engine"
    }

    fn build(&self, app: &mut App) {
        app.init_resource::<Tally>()
            .add_systems(FixedUpdate, count.in_set(SimulationSet))
            .add_systems(Update, report);
    }
}

blockloom_engine::export_module!(TallyModule);

fn count(run: Res<RunState>, mut tally: ResMut<Tally>) {
    if run.running && !run.paused {
        tally.ticks += 1;
    }
}

/// Says the count once a run ends, and starts over.
fn report(run: Res<RunState>, mut tally: ResMut<Tally>) {
    if run.running == tally.running {
        return;
    }
    if tally.running {
        say("", &format!("tally-engine: {} ticks", tally.ticks));
        tally.ticks = 0;
    }
    tally.running = run.running;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_counts_only_while_running() {
        let mut app = App::new();
        app.init_resource::<RunState>()
            .init_resource::<Tally>()
            .add_systems(Update, count);
        app.update();
        app.world_mut().resource_mut::<RunState>().running = true;
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Tally>().ticks, 2);
    }
}
