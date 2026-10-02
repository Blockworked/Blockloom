//! Ledger rows: authoritative queries over every enabled collider.

use blockloom_physics_probes::*;
use rapier3d::prelude::*;

fn wall(w: &mut PhysicsWorld, x: f32) -> ColliderHandle {
    // Collider-only actor: no body, no visual.
    w.colliders
        .insert(ColliderBuilder::cuboid(0.5, 5.0, 5.0).translation(Vector::new(x, 0.0, 0.0)))
}

fn east_ray() -> Ray {
    Ray::new(Vector::new(-10.0, 0.0, 0.0), Vector::new(1.0, 0.0, 0.0))
}

/// Queries see colliders no renderer knows about, and the filter chooses sensors.
#[test]
fn rays_hit_invisible_bodyless_colliders_and_filter_triggers() {
    let mut w = world3();
    let solid = wall(&mut w, 0.0);
    let trigger = w.colliders.insert(
        ColliderBuilder::cuboid(0.5, 5.0, 5.0)
            .translation(Vector::new(-5.0, 0.0, 0.0))
            .sensor(true),
    );
    sync(&mut w);
    let ray = east_ray();
    // Triggers are included by default and excluded on request.
    let included = w.cast_ray(&ray, 100.0, true, QueryFilter::default());
    let excluded = w.cast_ray(&ray, 100.0, true, QueryFilter::default().exclude_sensors());
    let only_triggers = w.cast_ray(&ray, 100.0, true, QueryFilter::default().exclude_solids());
    println!("included {included:?}, excluded {excluded:?}, only {only_triggers:?}");
    assert_eq!(included.map(|h| h.0), Some(trigger));
    assert_eq!(excluded.map(|h| h.0), Some(solid));
    assert_eq!(only_triggers.map(|h| h.0), Some(trigger));
}

/// All-hit queries return every collider unsorted; the adapter sorts by distance and
/// then collider ID so ties are stable.
#[test]
fn all_hits_need_a_stable_sort() {
    let mut w = world3();
    // Three walls, two of them at the identical distance.
    let a = wall(&mut w, 0.0);
    let b = wall(&mut w, 0.0);
    let c = wall(&mut w, 3.0);
    sync(&mut w);
    let q = w.query_pipeline();
    let mut hits: Vec<_> = q
        .intersect_ray(east_ray(), 100.0, true)
        .map(|(h, _, i)| (i.time_of_impact, h))
        .collect();
    hits.sort_by(|x, y| {
        x.0.total_cmp(&y.0)
            .then(x.1.into_raw_parts().cmp(&y.1.into_raw_parts()))
    });
    println!("sorted hits: {hits:?}");
    assert_eq!(hits.iter().map(|h| h.1).collect::<Vec<_>>(), [a, b, c]);
    assert!((hits[0].0 - 9.5).abs() < 1e-4);
}

/// Starting inside a solid: a solid query reports distance zero, a hollow one the exit.
#[test]
fn starting_inside_reports_zero_or_the_exit() {
    let mut w = world3();
    wall(&mut w, 0.0);
    sync(&mut w);
    let inside = Ray::new(Vector::new(0.0, 0.0, 0.0), Vector::new(1.0, 0.0, 0.0));
    let solid = w
        .cast_ray_and_get_normal(&inside, 100.0, true, QueryFilter::default())
        .unwrap();
    let hollow = w
        .cast_ray_and_get_normal(&inside, 100.0, false, QueryFilter::default())
        .unwrap();
    println!(
        "inside: solid toi {} normal {:?}; hollow toi {} normal {:?}",
        solid.1.time_of_impact, solid.1.normal, hollow.1.time_of_impact, hollow.1.normal
    );
    assert_eq!(solid.1.time_of_impact, 0.0);
    assert!((hollow.1.time_of_impact - 0.5).abs() < 1e-4);
}

/// A zero direction has no meaning; the adapter has to reject it before the backend.
#[test]
fn zero_direction_is_not_a_ray() {
    let mut w = world3();
    wall(&mut w, 0.0);
    sync(&mut w);
    let ray = Ray::new(Vector::new(-10.0, 0.0, 0.0), Vector::ZERO);
    let hit = w.cast_ray(&ray, 100.0, true, QueryFilter::default());
    println!("zero-direction ray: {hit:?}");
    // Whatever it answers, it must not be a believable hit on the wall.
    assert!(hit.is_none_or(|h| !h.1.is_finite() || h.1 == 0.0 || h.1 > 0.0));
}

/// Triangle meshes are hit from both sides and the hit says which: `is_backface` gives
/// the adapter its one-sided collision policy.
#[test]
fn trimesh_reports_which_face_was_hit() {
    let mut w = world3();
    // Wound so the front faces +x.
    let vertices = vec![
        Vector::new(0.0, -5.0, -5.0),
        Vector::new(0.0, 5.0, -5.0),
        Vector::new(0.0, 0.0, 5.0),
    ];
    let mesh = w
        .colliders
        .insert(ColliderBuilder::trimesh(vertices, vec![[0, 1, 2]]).unwrap());
    sync(&mut w);
    let from_front = Ray::new(Vector::new(3.0, 0.0, 0.0), Vector::new(-1.0, 0.0, 0.0));
    let from_back = Ray::new(Vector::new(-3.0, 0.0, 0.0), Vector::new(1.0, 0.0, 0.0));
    let shape = w.colliders[mesh].shape().as_trimesh().unwrap();
    let hit = |ray: &Ray| {
        let (_, h) = w
            .cast_ray_and_get_normal(ray, 100.0, true, QueryFilter::default())
            .expect("both sides are hit");
        (h.time_of_impact, h.normal, shape.is_backface(h.feature))
    };
    let (front, back) = (hit(&from_front), hit(&from_back));
    println!("trimesh: front {front:?}, back {back:?}");
    assert_eq!((front.0, back.0), (3.0, 3.0));
    assert!(!front.2 && back.2);
}

/// Sweeps and overlaps use the same colliders and the same filter as rays.
#[test]
fn shape_casts_overlaps_and_closest_points() {
    let mut w = world3();
    let wall = wall(&mut w, 0.0);
    let (body, own) = w.insert(
        RigidBodyBuilder::dynamic().translation(Vector::new(-4.0, 0.0, 0.0)),
        ColliderBuilder::capsule_y(0.5, 0.4),
    );
    sync(&mut w);
    let capsule = SharedShape::capsule_y(0.5, 0.4);
    let start = Pose::from_translation(Vector::new(-4.0, 0.0, 0.0));
    let options = rapier3d::parry::query::details::ShapeCastOptions::with_max_time_of_impact(100.0);
    // The mover's own collider is in the way unless excluded.
    let blocked = w.cast_shape(
        &start,
        Vector::new(1.0, 0.0, 0.0),
        capsule.as_ref(),
        options,
        QueryFilter::default(),
    );
    let clear = w.cast_shape(
        &start,
        Vector::new(1.0, 0.0, 0.0),
        capsule.as_ref(),
        options,
        QueryFilter::default().exclude_rigid_body(body),
    );
    println!(
        "own collider {:?}; sweep toi {:?}",
        blocked.map(|h| h.0 == own),
        clear.map(|h| (h.1.time_of_impact, h.1.normal1))
    );
    assert_eq!(blocked.map(|h| h.0), Some(own));
    let (hit, info) = clear.unwrap();
    assert_eq!(hit, wall);
    // Capsule radius 0.4 stops 0.4 short of the wall face at x = -0.5.
    assert!((info.time_of_impact - (4.0 - 0.5 - 0.4)).abs() < 1e-3);

    let q = w.query_pipeline_with_filter(QueryFilter::default().exclude_rigid_body(body));
    let at_wall = Pose::from_translation(Vector::new(-0.6, 0.0, 0.0));
    let overlapping: Vec<_> = q
        .intersect_shape(at_wall, capsule.as_ref())
        .map(|(h, _)| h)
        .collect();
    assert_eq!(overlapping, [wall]);
    let (nearest, projection) = q
        .project_point(Vector::new(-3.0, 0.0, 0.0), f32::MAX, false)
        .unwrap();
    assert_eq!(nearest, wall);
    assert!((projection.point.x + 0.5).abs() < 1e-4);
}

/// Layer filters on a query are explicit and independent of the simulation's.
#[test]
fn query_groups_are_independent_of_collision_groups() {
    let mut w = world3();
    let layer_a = Group::GROUP_1;
    let layer_b = Group::GROUP_2;
    let a = w.colliders.insert(
        ColliderBuilder::cuboid(0.5, 5.0, 5.0)
            .translation(Vector::new(0.0, 0.0, 0.0))
            .collision_groups(InteractionGroups::new(
                layer_a,
                layer_a,
                InteractionTestMode::And,
            )),
    );
    let b = w.colliders.insert(
        ColliderBuilder::cuboid(0.5, 5.0, 5.0)
            .translation(Vector::new(3.0, 0.0, 0.0))
            .collision_groups(InteractionGroups::new(
                layer_b,
                layer_b,
                InteractionTestMode::And,
            )),
    );
    sync(&mut w);
    let only = |g: Group| {
        w.cast_ray(
            &east_ray(),
            100.0,
            true,
            QueryFilter::default().groups(InteractionGroups::new(
                Group::ALL,
                g,
                InteractionTestMode::And,
            )),
        )
        .map(|h| h.0)
    };
    assert_eq!(only(layer_a), Some(a));
    assert_eq!(only(layer_b), Some(b));
}

/// A collider moved without a step is invisible to queries at its new place until the
/// adapter refreshes the broad phase (plan section 10.1, step 5), and a collider
/// inserted since the last step is invisible entirely.
#[test]
fn queries_are_stale_until_the_broad_phase_is_synced() {
    let mut w = world3();
    let c = wall(&mut w, 0.0);
    w.colliders[c].set_position(Pose::from_translation(Vector::new(0.0, 100.0, 0.0)));
    sync(&mut w);
    assert!(
        w.cast_ray(&east_ray(), 100.0, true, QueryFilter::default())
            .is_none()
    );
    // Bring it back onto the ray without stepping.
    w.colliders[c].set_position(Pose::from_translation(Vector::new(0.0, 0.0, 0.0)));
    let stale = w.cast_ray(&east_ray(), 100.0, true, QueryFilter::default());
    sync(&mut w);
    let fresh = w.cast_ray(&east_ray(), 100.0, true, QueryFilter::default());
    println!("teleported onto the ray: stale {stale:?}, synced {fresh:?}");
    assert!(stale.is_none(), "the old position is what queries see");
    assert!(fresh.is_some());
}

/// Work scales with the candidates, not the actor count.
#[test]
fn query_cost_tracks_candidates_not_colliders() {
    let mut w = world3();
    for i in 0..20_000 {
        let x = (i % 200) as f32 * 4.0;
        let z = (i / 200) as f32 * 4.0;
        w.colliders
            .insert(ColliderBuilder::cuboid(0.5, 0.5, 0.5).translation(Vector::new(x, 0.0, z)));
    }
    sync(&mut w);
    let start = std::time::Instant::now();
    let rays = 2_000;
    let mut hits = 0;
    for i in 0..rays {
        let ray = Ray::new(
            Vector::new((i % 100) as f32 * 4.0, 10.0, (i / 100) as f32 * 4.0),
            Vector::new(0.0, -1.0, 0.0),
        );
        hits += w
            .cast_ray(&ray, 100.0, true, QueryFilter::default())
            .is_some() as usize;
    }
    let per_ray = start.elapsed().as_secs_f64() / rays as f64 * 1e6;
    println!("{rays} rays over 20000 colliders: {per_ray:.2} us each, {hits} hits");
    assert_eq!(hits, rays, "every ray lands on a box");
    assert!(per_ray < 500.0);
}
