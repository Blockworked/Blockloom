//! Ledger rows: Discrete / Continuous / Continuous Dynamic / Continuous Speculative.
//!
//! Each scenario fires something fast at something thin and records whether it
//! tunneled. The table the test prints is what the ledger quotes.

use blockloom_physics_probes::*;
use rapier3d::prelude::*;

/// The Rapier setting behind each Unity-style name. The pinned revision sweeps **fixed**
/// colliders for every fast dynamic body on its own; `ccd_enabled` ("bullet") widens the
/// sweep to kinematic and dynamic bodies; soft CCD is a speculative contact margin.
#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Automatic fixed sweep off world-wide, no per-body flags.
    Discrete,
    /// Rapier as shipped: nothing set on the body.
    Automatic,
    /// `ccd_enabled`.
    Bullet,
    /// `soft_ccd_prediction`, automatic sweep off.
    Speculative,
    /// Both flags.
    BulletAndSpeculative,
}

const MODES: [Mode; 5] = [
    Mode::Discrete,
    Mode::Automatic,
    Mode::Bullet,
    Mode::Speculative,
    Mode::BulletAndSpeculative,
];

fn configure(body: RigidBodyBuilder, mode: Mode, speed: f32) -> RigidBodyBuilder {
    let predict = speed * DT * 1.5;
    match mode {
        Mode::Discrete | Mode::Automatic => body,
        Mode::Bullet => body.ccd_enabled(true),
        Mode::Speculative => body.soft_ccd_prediction(predict),
        Mode::BulletAndSpeculative => body.ccd_enabled(true).soft_ccd_prediction(predict),
    }
}

fn space(mode: Mode) -> PhysicsWorld {
    let mut w = world3();
    w.gravity = Vector::ZERO;
    if matches!(mode, Mode::Discrete | Mode::Speculative) {
        w.integration_parameters.max_ccd_substeps = 0;
    }
    w
}

const SPEED: f32 = 300.0;
const WALL_X: f32 = 5.0;
/// Chosen so no fixed step lands inside the wall (steps are 5 m): the start is 4.7 m short.
const START_X: f32 = 0.3;

fn projectile(w: &mut PhysicsWorld, mode: Mode) -> RigidBodyHandle {
    w.insert(
        configure(
            RigidBodyBuilder::dynamic()
                .translation(Vector::ZERO)
                .linvel(Vector::new(SPEED, 0.0, 0.0)),
            mode,
            SPEED,
        ),
        ColliderBuilder::ball(0.1).density(1.0),
    )
    .0
}

/// True when the projectile ended up beyond the wall.
fn tunneled(w: &PhysicsWorld, b: RigidBodyHandle) -> bool {
    w.bodies[b].translation().x > WALL_X + 1.0
}

fn static_wall(mode: Mode) -> bool {
    let mut w = space(mode);
    w.insert(
        RigidBodyBuilder::fixed().translation(Vector::new(WALL_X, 0.0, 0.0)),
        ColliderBuilder::cuboid(0.02, 5.0, 5.0),
    );
    let p = projectile(&mut w, mode);
    run(&mut w, 30);
    tunneled(&w, p)
}

/// A wall of the given half thickness drives towards the projectile at 100 m/s. Tunneled
/// means the projectile ended on the far side of the wall's plane.
fn kinematic_wall(mode: Mode, half: f32) -> bool {
    let mut w = space(mode);
    let (wall, _) = w.insert(
        RigidBodyBuilder::kinematic_velocity_based()
            .translation(Vector::new(WALL_X + 20.0, 0.0, 0.0))
            .linvel(Vector::new(-100.0, 0.0, 0.0)),
        ColliderBuilder::cuboid(half, 5.0, 5.0),
    );
    let p = projectile(&mut w, mode);
    run(&mut w, 30);
    w.bodies[p].translation().x > w.bodies[wall].translation().x
}

fn dynamic_plate(mode: Mode, plate_mode: Mode) -> bool {
    let mut w = space(mode);
    w.insert(
        configure(
            RigidBodyBuilder::dynamic().translation(Vector::new(WALL_X, 0.0, 0.0)),
            plate_mode,
            0.0,
        ),
        ColliderBuilder::cuboid(0.02, 5.0, 5.0).density(1.0),
    );
    let p = projectile(&mut w, mode);
    run(&mut w, 30);
    // A hit transfers momentum; passing straight through leaves the speed untouched.
    w.bodies[p].linvel().x > 0.95 * SPEED
}

fn sensor_pass(mode: Mode) -> bool {
    let mut w = space(mode);
    let events = Recorder::default();
    w.insert(
        RigidBodyBuilder::fixed().translation(Vector::new(WALL_X, 0.0, 0.0)),
        ColliderBuilder::cuboid(0.02, 5.0, 5.0)
            .sensor(true)
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let (p, _) = w.insert(
        configure(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(START_X, 0.0, 0.0))
                .linvel(Vector::new(SPEED, 0.0, 0.0)),
            mode,
            SPEED,
        ),
        ColliderBuilder::ball(0.1).active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let mut seen = false;
    for _ in 0..30 {
        w.step_with_events(&(), &events);
        seen |= events.drain().iter().any(|e| e.started());
    }
    let _ = p;
    !seen
}

/// A bar spinning past a pinned ball it never overlaps at a step boundary: only a
/// sweep that covers the rotation can see the hit. The spin is below Rapier's cap of
/// pi/4 per step (47.1 rad/s at 60 Hz), which is itself a restriction to document.
fn spinning_bar(mode: Mode) -> bool {
    let mut w = space(mode);
    let spin = 40.0;
    let (bar, _) = w.insert(
        configure(
            RigidBodyBuilder::dynamic().angvel(Vector::new(0.0, 0.0, spin)),
            mode,
            3.0 * spin,
        ),
        ColliderBuilder::cuboid(3.0, 0.02, 0.5).density(1.0),
    );
    // On the bar's path at 1 rad, between its poses at 0.67 and 1.33 rad.
    w.insert(
        RigidBodyBuilder::dynamic()
            .translation(Vector::new(2.5 * 1.0f32.cos(), 2.5 * 1.0f32.sin(), 0.0))
            .lock_translations(),
        ColliderBuilder::ball(0.05).density(1.0),
    );
    run(&mut w, 8);
    spin - w.bodies[bar].angvel().z > 0.5
}

/// What each mode holds, as measured. No single mode holds every scenario, so the
/// Unity names cannot be one flag each (ledger: CCD).
#[test]
fn tunneling_table() {
    use Mode::*;
    // (static wall, trigger, dynamic plate, spinning bar, 20 cm kinematic wall) held?
    let held = |m: Mode| {
        (
            !static_wall(m),
            !sensor_pass(m),
            !dynamic_plate(m, Automatic),
            spinning_bar(m),
            !kinematic_wall(m, 0.1),
        )
    };
    let mut rows = Vec::new();
    for m in MODES {
        let h = held(m);
        rows.push(format!(
            "{m:?}: static wall {}, trigger {}, dynamic plate {}, spinning bar {}, kinematic wall {} (4 cm: {})",
            h.0, h.1, h.2, h.3, h.4, !kinematic_wall(m, 0.02)
        ));
    }
    println!("{}", rows.join("\n"));
    assert_eq!(held(Discrete), (false, false, false, false, false));
    assert_eq!(held(Automatic), (true, true, false, false, false));
    assert_eq!(held(Bullet), (true, true, true, true, false));
    assert_eq!(held(Speculative), (true, false, true, false, true));
    assert_eq!(held(BulletAndSpeculative), (true, true, true, true, false));
    // Bullet against a bullet is still open.
    let both = dynamic_plate(Bullet, Bullet);
    println!("bullet projectile through a bullet plate: tunneled {both}");
    assert!(both);
}

#[test]
fn discrete_needs_the_world_switch_and_the_default_never_tunnels_static() {
    assert!(
        static_wall(Mode::Discrete),
        "with the automatic sweep off, 5 m steps skip a 4 cm wall"
    );
    assert!(
        !static_wall(Mode::Automatic),
        "the default sweeps fixed colliders for fast bodies"
    );
    assert!(
        !static_wall(Mode::Speculative),
        "soft CCD alone also holds a static wall"
    );
}
