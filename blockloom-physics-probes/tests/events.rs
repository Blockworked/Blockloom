//! Ledger rows: the pair event matrix, contact lifecycle, touching reference
//! counts, payloads and removal.

use std::collections::HashMap;

use blockloom_physics_probes::*;
use rapier3d::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Static,
    Kinematic,
    Dynamic,
}

const KINDS: [Kind; 3] = [Kind::Static, Kind::Kinematic, Kind::Dynamic];

fn body(kind: Kind) -> RigidBodyBuilder {
    match kind {
        Kind::Static => RigidBodyBuilder::fixed(),
        Kind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        Kind::Dynamic => RigidBodyBuilder::dynamic().gravity_scale(0.0),
    }
}

/// Two overlapping shapes of the given kinds: did the solver or the trigger lifecycle
/// see them? `all_types` is the adapter's choice to enable every body-type pair, on
/// the trigger alone when there is one.
fn sees(a: Kind, b: Kind, trigger: bool, all_types: bool) -> bool {
    let types = if all_types {
        ActiveCollisionTypes::all()
    } else {
        ActiveCollisionTypes::default()
    };
    sees_with(a, b, trigger, types)
}

fn sees_with(a: Kind, b: Kind, trigger: bool, types: ActiveCollisionTypes) -> bool {
    let mut w = world3();
    let events = Recorder::default();
    let make = |w: &mut PhysicsWorld, kind: Kind, x: f32, sensor: bool| {
        let types = if trigger && !sensor {
            ActiveCollisionTypes::default()
        } else {
            types
        };
        w.insert(
            body(kind).translation(Vector::new(x, 0.0, 0.0)),
            ColliderBuilder::cuboid(0.5, 0.5, 0.5)
                .sensor(sensor)
                .active_events(ActiveEvents::COLLISION_EVENTS)
                .active_collision_types(types),
        )
    };
    make(&mut w, a, 0.0, trigger);
    make(&mut w, b, 0.6, false);
    for _ in 0..5 {
        w.step_with_events(&(), &events);
    }
    events.drain().iter().any(|e| e.started())
}

/// Unity's collision/trigger matrix against Rapier's defaults and with every pair
/// enabled. The adapter must enable the pairs Unity reports and mask the one it
/// does not (trigger static/static).
#[test]
fn pair_matrix_against_unity() {
    let unity_solid = |a: Kind, b: Kind| matches!((a, b), (Kind::Dynamic, _) | (_, Kind::Dynamic));
    let unity_trigger = |a: Kind, b: Kind| !(a == Kind::Static && b == Kind::Static);
    let mut rows = Vec::new();
    let (mut solid_default_gaps, mut trigger_default_gaps) = (0, 0);
    for a in KINDS {
        for b in KINDS {
            let solid = sees(a, b, false, false);
            let solid_all = sees(a, b, false, true);
            let trigger = sees(a, b, true, false);
            let trigger_all = sees(a, b, true, true);
            rows.push(format!(
                "{a:?}/{b:?}: solid default {solid} all {solid_all}; trigger default {trigger} all {trigger_all}"
            ));
            // Solid contacts exist exactly where Unity reports a collision.
            assert_eq!(solid, unity_solid(a, b), "solid {a:?}/{b:?} by default");
            // Enabling every pair makes the engine see more than Unity reports.
            if unity_trigger(a, b) {
                assert!(trigger_all, "trigger {a:?}/{b:?} reachable with all types");
            }
            if trigger != unity_trigger(a, b) {
                trigger_default_gaps += 1;
            }
            if solid != unity_solid(a, b) {
                solid_default_gaps += 1;
            }
        }
    }
    println!("{}", rows.join("\n"));
    println!("default gaps: solid {solid_default_gaps}, trigger {trigger_default_gaps}");
}

/// The trigger-only collision types that reproduce Unity exactly: every pair except
/// static/static. Solids keep Rapier's defaults, so a kinematic body never reports a
/// solid collision with scenery.
#[test]
fn trigger_types_without_static_pairs_match_unity() {
    let types = ActiveCollisionTypes::all() - ActiveCollisionTypes::FIXED_FIXED;
    for a in KINDS {
        for b in KINDS {
            let expected = !(a == Kind::Static && b == Kind::Static);
            assert_eq!(
                sees_with(a, b, true, types),
                expected,
                "trigger {a:?}/{b:?}"
            );
            assert_eq!(
                sees_with(a, b, false, ActiveCollisionTypes::default()),
                a == Kind::Dynamic || b == Kind::Dynamic,
                "solid {a:?}/{b:?}"
            );
        }
    }
}

/// What a touching pair looks like at one fixed tick: start, then one touch per tick, then stop.
fn bounce_timeline(steps: usize) -> Vec<(usize, &'static str)> {
    let mut w = world3();
    w.colliders.insert(
        ColliderBuilder::cuboid(20.0, 0.5, 20.0)
            .translation(Vector::new(0.0, -0.5, 0.0))
            .restitution(0.6),
    );
    let (_, ball) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 2.0, 0.0)),
        ColliderBuilder::ball(0.5)
            .restitution(0.6)
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let events = Recorder::default();
    let mut out = Vec::new();
    for tick in 0..steps {
        w.step_with_events(&(), &events);
        for e in events.drain() {
            out.push((tick, if e.started() { "enter" } else { "exit" }));
        }
        let touching = w
            .narrow_phase
            .contact_pairs_with(ball)
            .any(|p| p.has_any_active_contact());
        if touching {
            out.push((tick, "stay"));
        }
    }
    out
}

/// Enter and exit alternate and every transition lands on its own fixed tick, however
/// the ticks are grouped into rendered frames: events are drained after each step.
#[test]
fn enter_stay_exit_alternate_per_fixed_tick() {
    let timeline = bounce_timeline(240);
    let transitions: Vec<_> = timeline.iter().filter(|(_, k)| *k != "stay").collect();
    println!("transitions: {transitions:?}");
    assert!(transitions.len() >= 4, "the ball bounces at least twice");
    for pair in transitions.windows(2) {
        assert_ne!(pair[0].1, pair[1].1, "enter and exit alternate");
        assert!(pair[0].0 <= pair[1].0, "ticks never go backwards");
    }
    let mut state = false;
    for (_, kind) in &timeline {
        match *kind {
            "enter" => {
                assert!(!state);
                state = true;
            }
            "exit" => {
                assert!(state);
                state = false;
            }
            _ => assert!(state, "stay is only reported while touching"),
        }
    }
}

/// Reference counting over collider pairs: an actor keeps touching until its final
/// shape leaves, and the aggregate exit is emitted exactly once.
#[test]
fn touching_is_counted_over_collider_pairs() {
    let mut w = world3();
    let floor = w.colliders.insert(
        ColliderBuilder::cuboid(20.0, 0.5, 20.0)
            .translation(Vector::new(0.0, -0.5, 0.0))
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let (b, left) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 0.5, 0.0)),
        ColliderBuilder::cuboid(0.5, 0.5, 0.5)
            .translation(Vector::new(-1.0, 0.0, 0.0))
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let right = w.colliders.insert_with_parent(
        ColliderBuilder::cuboid(0.5, 0.5, 0.5)
            .translation(Vector::new(1.0, 0.0, 0.0))
            .active_events(ActiveEvents::COLLISION_EVENTS),
        b,
        &mut w.bodies,
    );
    let events = Recorder::default();
    let owner: HashMap<ColliderHandle, &str> =
        [(floor, "floor"), (left, "crate"), (right, "crate")].into();
    // actor pair -> number of active collider pairs
    let mut touching: HashMap<(&str, &str), i32> = HashMap::new();
    let mut aggregate = Vec::new();
    let mut apply = |touching: &mut HashMap<(&str, &str), i32>, evs: Vec<CollisionEvent>| {
        for e in evs {
            let (a, b) = (owner[&e.collider1()], owner[&e.collider2()]);
            let key = if a < b { (a, b) } else { (b, a) };
            let n = touching.entry(key).or_insert(0);
            let before = *n;
            *n += if e.started() { 1 } else { -1 };
            if before == 0 && *n == 1 {
                aggregate.push("enter");
            }
            if before == 1 && *n == 0 {
                aggregate.push("exit");
            }
        }
    };
    for _ in 0..30 {
        w.step_with_events(&(), &events);
        apply(&mut touching, events.drain());
    }
    assert_eq!(
        touching[&("crate", "floor")],
        2,
        "both shapes touch the floor"
    );
    // Remove one shape: touching stays true, nothing is emitted.
    w.colliders.remove(
        left,
        &mut w.islands,
        &mut w.bodies,
        &mut w.soft_bodies,
        true,
    );
    w.bodies[b].recompute_mass_properties_from_colliders(&w.colliders);
    w.step_with_events(&(), &events);
    let evs = events.drain();
    println!("events after removing one shape: {evs:?}");
    assert!(
        evs.iter().all(|e| e.removed()),
        "removal is flagged so the reason is known"
    );
    apply(&mut touching, evs);
    assert_eq!(touching[&("crate", "floor")], 1);
    // The last shape leaves: one aggregate exit.
    w.colliders.remove(
        right,
        &mut w.islands,
        &mut w.bodies,
        &mut w.soft_bodies,
        true,
    );
    w.step_with_events(&(), &events);
    apply(&mut touching, events.drain());
    assert_eq!(touching[&("crate", "floor")], 0);
    println!("aggregate: {aggregate:?}");
    assert_eq!(aggregate, ["enter", "exit"]);
}

/// A disabled collider ends its pairs the same way, so disable needs no special case.
#[test]
fn disable_ends_pairs_with_an_event() {
    let mut w = world3();
    let floor = w.colliders.insert(
        ColliderBuilder::cuboid(20.0, 0.5, 20.0)
            .translation(Vector::new(0.0, -0.5, 0.0))
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let (_, ball) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 0.5, 0.0)),
        ColliderBuilder::ball(0.5).active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let events = Recorder::default();
    for _ in 0..20 {
        w.step_with_events(&(), &events);
    }
    events.drain();
    w.colliders[floor].set_enabled(false);
    w.step_with_events(&(), &events);
    let evs = events.drain();
    println!("events after disable: {evs:?}");
    assert!(
        evs.iter()
            .any(|e| e.stopped() && e.collider2() == ball || e.collider1() == ball)
    );
}

/// The payload Unity's Collision carries is recoverable: normal, points, impulse from the pair,
/// relative velocity from the bodies sampled before the step.
#[test]
fn collision_payload_is_recoverable() {
    let mut w = world3();
    w.colliders.insert(
        ColliderBuilder::cuboid(20.0, 0.5, 20.0)
            .translation(Vector::new(0.0, -0.5, 0.0))
            .restitution(0.0)
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let (b, ball) = w.insert(
        RigidBodyBuilder::dynamic()
            .translation(Vector::new(0.0, 0.8, 0.0))
            .linvel(Vector::new(0.0, -5.0, 0.0)),
        ColliderBuilder::ball(0.5)
            .restitution(0.0)
            .density(1.0)
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let events = Recorder::default();
    let mass = w.bodies[b].mass();
    for _ in 0..30 {
        let velocity_before = w.bodies[b].linvel();
        w.step_with_events(&(), &events);
        if events.drain().iter().any(|e| e.started()) {
            let pair = w
                .narrow_phase
                .contact_pairs_with(ball)
                .next()
                .expect("pair");
            let manifold = &pair.manifolds()[0];
            let impulse = pair.total_impulse().length();
            println!(
                "enter: normal {:?}, points {}, impulse {impulse:.3} (m*v = {:.3}), v before {velocity_before:?}",
                manifold.data.normal,
                manifold.points.len(),
                mass * 5.0
            );
            assert!(manifold.data.normal.y.abs() > 0.99);
            assert!(!manifold.points.is_empty());
            assert!(impulse > 0.0, "impulse is readable after the step");
            return;
        }
    }
    panic!("no enter event");
}

/// A resting body that falls asleep keeps its contact pair, so Stay and `touching?`
/// can stay true while it sleeps.
#[test]
fn sleeping_body_keeps_its_pair() {
    let mut w = world3();
    w.colliders
        .insert(ColliderBuilder::cuboid(20.0, 0.5, 20.0).translation(Vector::new(0.0, -0.5, 0.0)));
    let (b, ball) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 0.5, 0.0)),
        ColliderBuilder::cuboid(0.5, 0.5, 0.5),
    );
    run(&mut w, 600);
    let asleep = w.bodies[b].is_sleeping();
    let touching = w
        .narrow_phase
        .contact_pairs_with(ball)
        .any(|p| p.has_any_active_contact());
    println!("asleep {asleep}, still touching {touching}");
    assert!(asleep);
    assert!(touching);
}

/// Fixed-step result delivery: a frame that runs several fixed steps must see every
/// transition, so events are kept per step and handed over in order. Polling the contact
/// state once per frame misses touches that begin and end inside it.
#[test]
fn polling_per_frame_misses_touches_that_events_keep() {
    let mut w = world3();
    w.colliders.insert(
        ColliderBuilder::cuboid(20.0, 0.5, 20.0)
            .translation(Vector::new(0.0, -0.5, 0.0))
            .restitution(0.9),
    );
    let (_, ball) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 2.0, 0.0)),
        ColliderBuilder::ball(0.5)
            .restitution(0.9)
            .active_events(ActiveEvents::COLLISION_EVENTS),
    );
    let events = Recorder::default();
    let (mut seen_by_events, mut seen_by_polling) = (0, 0);
    let mut per_step_order = Vec::new();
    // Eight fixed steps per rendered frame, longer than a contact lasts.
    for _frame in 0..40 {
        for _ in 0..8 {
            w.step_with_events(&(), &events);
            for e in events.drain() {
                per_step_order.push(e.started());
            }
        }
        if w.narrow_phase
            .contact_pairs_with(ball)
            .any(|p| p.has_any_active_contact())
        {
            seen_by_polling += 1;
        }
    }
    seen_by_events += per_step_order.iter().filter(|s| **s).count();
    println!("touches seen by events {seen_by_events}, by frame polling {seen_by_polling}");
    assert!(seen_by_events >= 3, "the ball bounces several times");
    assert!(
        seen_by_polling < seen_by_events,
        "polling misses short touches"
    );
    assert!(
        per_step_order.windows(2).all(|p| p[0] != p[1]),
        "events alternate enter and exit however the steps are grouped"
    );
}
