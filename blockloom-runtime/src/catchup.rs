//! The adaptive fixed-step catch-up cap. After a hitch Bevy runs up to
//! `max_delta` (250 ms) of fixed steps in one frame. When a step itself costs
//! more than its share of a frame that spirals, so the cap shrinks to as many
//! steps as fit in `FRAME_BUDGET` at the measured step cost. A cheap sim keeps
//! Bevy's 250 ms; an expensive one turns a hitch into slow motion instead.

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use std::time::Duration;

/// Wall time a frame may spend catching up on fixed steps.
pub const FRAME_BUDGET: Duration = Duration::from_millis(100);
/// Bevy's own `max_delta`, the most the cap ever allows.
pub const CEILING: Duration = Duration::from_millis(250);

/// Measured cost of one fixed step, smoothed.
#[derive(Resource, Default, Debug, Clone)]
pub struct CatchUp {
    /// Seconds per fixed step, exponentially smoothed.
    pub step_cost: f64,
    began: Option<Instant>,
    steps: u32,
}

pub fn register(app: &mut App) {
    app.init_resource::<CatchUp>()
        .add_systems(
            RunFixedMainLoop,
            (
                begin.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
                finish.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
            ),
        )
        .add_systems(FixedFirst, count_step);
}

/// The longest frame delta worth catching up on, given what a step costs.
pub fn cap(step_cost: f64, fixed_dt: Duration) -> Duration {
    if !(step_cost.is_finite() && step_cost > 0.0) || fixed_dt.is_zero() {
        return CEILING;
    }
    let steps = (FRAME_BUDGET.as_secs_f64() / step_cost).floor().max(1.0);
    fixed_dt.mul_f64(steps).max(fixed_dt).min(CEILING)
}

fn begin(
    mut catch_up: ResMut<CatchUp>,
    fixed: Res<Time<Fixed>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    strategy: Option<Res<bevy::time::TimeUpdateStrategy>>,
) {
    catch_up.began = Some(Instant::now());
    catch_up.steps = 0;
    // A test stepping time by hand decides its own deltas.
    if strategy.is_some_and(|s| !matches!(*s, bevy::time::TimeUpdateStrategy::Automatic)) {
        return;
    }
    let wanted = cap(catch_up.step_cost, fixed.timestep());
    if virtual_time.max_delta() != wanted {
        virtual_time.set_max_delta(wanted);
    }
}

fn count_step(mut catch_up: ResMut<CatchUp>) {
    catch_up.steps += 1;
}

fn finish(mut catch_up: ResMut<CatchUp>) {
    let Some(began) = catch_up.began.take() else {
        return;
    };
    if catch_up.steps == 0 {
        return;
    }
    let each = began.elapsed().as_secs_f64() / f64::from(catch_up.steps);
    catch_up.step_cost = if catch_up.step_cost > 0.0 {
        catch_up.step_cost * 0.9 + each * 0.1
    } else {
        each
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: Duration = Duration::from_micros(16_667);

    #[test]
    fn a_cheap_sim_keeps_bevys_ceiling() {
        assert_eq!(cap(0.0005, STEP), CEILING);
        assert_eq!(cap(0.0, STEP), CEILING);
    }

    #[test]
    fn an_expensive_sim_catches_up_on_fewer_steps() {
        // 30 ms a step: three fit in the budget.
        assert_eq!(cap(0.030, STEP), STEP * 3);
        // Slower than the whole budget still runs one step a frame.
        assert_eq!(cap(0.5, STEP), STEP);
    }
}
