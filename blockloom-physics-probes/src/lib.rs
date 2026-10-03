//! Phase 0 probes for the physics and character controller plan.
//!
//! Each file under `tests/` answers rows of the compatibility ledger
//! (`docs/physics-compatibility-ledger.md`) against raw Rapier. A passing
//! probe proves a mapping; one that records a gap names it in the test.
//! Measured numbers are printed, so `cargo test -p blockloom-physics-probes --
//! --nocapture` shows what the ledger quotes.

use rapier3d::prelude::*;

/// The fixed rate the probes run at unless a test is about the rate.
pub const DT: f32 = 1.0 / 60.0;

/// A 3D world stepped at `DT`.
pub fn world3() -> PhysicsWorld {
    let mut w = PhysicsWorld::default();
    w.integration_parameters.dt = DT;
    w
}

/// Steps `n` times with no hooks and no events.
pub fn run(w: &mut PhysicsWorld, n: usize) {
    for _ in 0..n {
        w.step();
    }
}

/// Collects collision events so a test can read them after each step.
#[derive(Default)]
pub struct Recorder {
    events: std::sync::Mutex<Vec<CollisionEvent>>,
}

impl Recorder {
    /// Takes what the last step produced, in the order Rapier raised it.
    pub fn drain(&self) -> Vec<CollisionEvent> {
        std::mem::take(&mut self.events.lock().unwrap())
    }
}

impl EventHandler for Recorder {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        self.events.lock().unwrap().push(event);
    }

    fn handle_contact_force_event(
        &self,
        _dt: Real,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: Real,
    ) {
    }

    fn handle_soft_body_tear_event(&self, _soft_bodies: &SoftBodySet, _event: &SoftBodyTearEvent) {}
}

/// Refreshes the broad phase so queries see colliders inserted or moved since the last
/// step. This is the "synchronize query geometry" stage of the fixed step.
pub fn sync(w: &mut PhysicsWorld) {
    w.detect_collisions(&(), &());
}
