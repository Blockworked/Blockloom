//! Ledger rows: Rigidbody without Collider, Collider without Rigidbody, compound
//! bodies, explicit mass, same-body shapes.

use blockloom_physics_probes::*;
use rapier3d::prelude::*;

fn ground(w: &mut PhysicsWorld) -> ColliderHandle {
    // No rigid body at all: a Collider-only actor.
    w.colliders
        .insert(ColliderBuilder::cuboid(20.0, 0.5, 20.0).translation(Vector::new(0.0, -0.5, 0.0)))
}

/// An actor with only a Collider is static geometry; nothing needs a fixed body.
#[test]
fn collider_without_rigidbody_is_static_geometry() {
    let mut w = world3();
    ground(&mut w);
    let (ball, _) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 2.0, 0.0)),
        ColliderBuilder::ball(0.5),
    );
    run(&mut w, 120);
    let y = w.bodies[ball].translation().y;
    println!("ball rests at y = {y:.4} on a body-less collider");
    assert!((y - 0.5).abs() < 0.02);
}

/// A dynamic body with no collider has no mass in Rapier, so gravity does nothing
/// until the adapter gives it one. Unity's Rigidbody defaults to 1 kg.
#[test]
fn rigidbody_without_collider_needs_an_adapter_mass() {
    let fall = |mass: Option<f32>| {
        let mut w = world3();
        let mut b = RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 10.0, 0.0));
        if let Some(m) = mass {
            b = b.additional_mass(m);
        }
        let body = w.insert_body(b);
        run(&mut w, 60);
        (10.0 - w.bodies[body].translation().y, w.bodies[body].mass())
    };
    let (bare, bare_mass) = fall(None);
    let (massed, massed_mass) = fall(Some(1.0));
    println!(
        "no collider: fell {bare:.3} m (mass {bare_mass}); with adapter mass: fell {massed:.3} m \
         (mass {massed_mass})"
    );
    assert!(massed > 4.0, "an adapter mass makes the body fall");
    // Recording the bare case is the point: if this ever starts falling, the adapter is moot.
    assert!(
        bare < massed,
        "bare body does not integrate gravity like a massed one"
    );
}

/// A body with extra shapes has the sum of their masses, and a shape's local pose
/// is kept.
#[test]
fn compound_mass_is_the_sum_and_keeps_local_poses() {
    let mut w = world3();
    let (b, _) = w.insert(
        RigidBodyBuilder::dynamic(),
        ColliderBuilder::cuboid(0.5, 0.5, 0.5).density(2.0),
    );
    let c2 = w.colliders.insert_with_parent(
        ColliderBuilder::ball(0.5)
            .translation(Vector::new(2.0, 0.0, 0.0))
            .density(2.0),
        b,
        &mut w.bodies,
    );
    let box_mass = 2.0;
    let ball_mass = 2.0 * 4.0 / 3.0 * std::f32::consts::PI * 0.125;
    let mass = w.bodies[b].mass();
    let com = w.bodies[b].local_center_of_mass();
    println!("compound mass {mass:.4}, local COM {com:?}");
    assert!((mass - (box_mass + ball_mass)).abs() < 1e-3);
    let expected_x = 2.0 * ball_mass / (box_mass + ball_mass);
    assert!((com.x - expected_x).abs() < 1e-3);
    let local = w.colliders[c2].position_wrt_parent().unwrap().translation;
    assert!((local.x - 2.0).abs() < 1e-6);
}

/// Unity's mass belongs to the body. Spreading it over the shapes by volume keeps the
/// inertia tensor shaped by geometry and the total exact, at any shape count.
#[test]
fn explicit_body_mass_is_exact_and_scales_inertia() {
    let build = |total: f32| {
        let mut w = world3();
        let shapes = [
            ColliderBuilder::cuboid(0.5, 0.5, 0.5),
            ColliderBuilder::ball(0.5).translation(Vector::new(2.0, 0.0, 0.0)),
            ColliderBuilder::capsule_y(0.5, 0.25).translation(Vector::new(-2.0, 0.0, 0.0)),
        ];
        let volumes: Vec<f32> = shapes.iter().map(|s| s.clone().build().volume()).collect();
        let sum: f32 = volumes.iter().sum();
        let b = w.insert_body(RigidBodyBuilder::dynamic());
        for (s, v) in shapes.into_iter().zip(&volumes) {
            w.colliders
                .insert_with_parent(s.mass(total * v / sum), b, &mut w.bodies);
        }
        let props = w.bodies[b].mass_properties().local_mprops;
        (w.bodies[b].mass(), props.principal_inertia())
    };
    let (m1, i1) = build(5.0);
    let (m2, i2) = build(10.0);
    println!("mass {m1} -> {m2}; inertia {i1:?} -> {i2:?}");
    assert!((m1 - 5.0).abs() < 1e-3 && (m2 - 10.0).abs() < 1e-3);
    assert!((i2.x / i1.x - 2.0).abs() < 1e-3);
    assert!((i2.y / i1.y - 2.0).abs() < 1e-3);
}

/// A trigger does not add mass when its density is zero, so triggers on a body
/// leave the Rigidbody's mass alone.
#[test]
fn sensor_colliders_can_be_massless() {
    let mut w = world3();
    let (b, _) = w.insert(
        RigidBodyBuilder::dynamic(),
        ColliderBuilder::ball(0.5).density(1.0),
    );
    let before = w.bodies[b].mass();
    w.colliders.insert_with_parent(
        ColliderBuilder::cuboid(3.0, 3.0, 3.0)
            .sensor(true)
            .density(0.0),
        b,
        &mut w.bodies,
    );
    assert!((w.bodies[b].mass() - before).abs() < 1e-6);
}

/// Shapes of one body never touch each other, so a compound needs no filter.
#[test]
fn shapes_on_one_body_do_not_collide() {
    let mut w = world3();
    ground(&mut w);
    let (b, c1) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 1.0, 0.0)),
        ColliderBuilder::ball(0.5),
    );
    let c2 = w.colliders.insert_with_parent(
        ColliderBuilder::ball(0.5).translation(Vector::new(0.2, 0.0, 0.0)),
        b,
        &mut w.bodies,
    );
    run(&mut w, 120);
    let pair = w.narrow_phase.contact_pair(c1, c2);
    println!("same-body pair present: {}", pair.is_some());
    assert!(pair.is_none_or(|p| !p.has_any_active_contact()));
    let v = w.bodies[b].linvel().length();
    assert!(
        v < 0.05,
        "overlapping shapes do not push the body about: {v}"
    );
}

/// Removing a shape updates mass, but only once the body is recomputed: the adapter
/// has to do that in the same transaction (plan 4.2), or a query in between sees the
/// old mass.
#[test]
fn removing_one_shape_updates_mass_after_recompute() {
    let mut w = world3();
    let (b, c1) = w.insert(
        RigidBodyBuilder::dynamic(),
        ColliderBuilder::ball(0.5).density(1.0),
    );
    let c2 =
        w.colliders
            .insert_with_parent(ColliderBuilder::ball(0.5).density(1.0), b, &mut w.bodies);
    let both = w.bodies[b].mass();
    w.colliders
        .remove(c2, &mut w.islands, &mut w.bodies, &mut w.soft_bodies, true);
    let stale = w.bodies[b].mass();
    w.bodies[b].recompute_mass_properties_from_colliders(&w.colliders);
    let fresh = w.bodies[b].mass();
    println!("mass {both:.4} -> {stale:.4} (before recompute) -> {fresh:.4}");
    assert!((both - 2.0 * fresh).abs() < 1e-4);
    assert!(w.colliders.get(c1).is_some());
}

/// Disabling a shape removes it from simulation and queries alike.
#[test]
fn disabled_collider_leaves_contacts_and_queries() {
    let mut w = world3();
    let g = ground(&mut w);
    let (b, ball) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 0.52, 0.0)),
        ColliderBuilder::ball(0.5),
    );
    run(&mut w, 30);
    assert!(w.bodies[b].translation().y > 0.4);
    w.colliders[g].set_enabled(false);
    run(&mut w, 30);
    assert!(
        w.bodies[b].translation().y < 0.0,
        "falls through a disabled collider"
    );
    let ray = Ray::new(Vector::new(0.0, 5.0, 0.0), Vector::new(0.0, -1.0, 0.0));
    let hit = w.cast_ray(
        &ray,
        100.0,
        true,
        QueryFilter::default().exclude_collider(ball),
    );
    println!("ray over disabled ground: {hit:?}");
    assert!(hit.is_none());
}
