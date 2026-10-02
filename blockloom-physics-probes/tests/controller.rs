//! Ledger rows: CharacterController (Move, SimpleMove, flags, skin, slopes, steps,
//! overlap recovery, crouch headroom, platform carry).
//!
//! The character is a capsule that lives outside the world and is moved with
//! Rapier's kinematic controller, one call per submitted move.

use blockloom_physics_probes::*;
use rapier3d::control::{
    CharacterAutostep, CharacterCollision, CharacterLength, EffectiveCharacterMovement,
    KinematicCharacterController,
};
use rapier3d::prelude::*;

/// Unity's capsule: total end-to-end height, not cylinder length.
fn capsule(total_height: f32, radius: f32) -> SharedShape {
    SharedShape::capsule_y((total_height / 2.0 - radius).max(0.0), radius)
}

/// What one move did, in Unity's terms.
#[derive(Default, Debug)]
struct Flags {
    sides: bool,
    above: bool,
    below: bool,
}

struct Character {
    shape: SharedShape,
    pos: Pose,
    ctl: KinematicCharacterController,
    hits: Vec<CharacterCollision>,
}

impl Character {
    fn new(total_height: f32, radius: f32, feet: Vector) -> Self {
        Self {
            shape: capsule(total_height, radius),
            // Starts 0.2 m up: a capsule inside the skin zone is not a valid start (see
            // `skin_zone_starts_need_recovery_first`).
            pos: Pose::from_translation(feet + Vector::Y * (total_height / 2.0 + 0.2)),
            ctl: unity_profile(),
            hits: Vec::new(),
        }
    }

    fn feet(&self, total_height: f32) -> Vector {
        self.pos.translation - Vector::Y * total_height / 2.0
    }

    /// One Move(displacement). Returns Rapier's result and the Unity-style flags.
    fn mv(&mut self, w: &PhysicsWorld, d: Vector) -> (EffectiveCharacterMovement, Flags) {
        self.hits.clear();
        let hits = &mut self.hits;
        let q = w.query_pipeline();
        let r = self
            .ctl
            .move_shape(DT, &q, self.shape.as_ref(), &self.pos, d, |c| hits.push(c));
        self.pos.translation += r.translation;
        let mut flags = Flags::default();
        for h in &self.hits {
            // `normal1` is the obstacle's surface normal, towards the character.
            let up = h.hit.normal1.dot(Vector::Y);
            if up > 0.7 {
                flags.below = true;
            } else if up < -0.7 {
                flags.above = true;
            } else {
                flags.sides = true;
            }
        }
        flags.below |= r.grounded;
        (r, flags)
    }
}

/// Unity's CharacterController defaults: slope limit 45, step offset 0.3, skin width
/// 0.08, no automatic sliding, no snapping.
fn unity_profile() -> KinematicCharacterController {
    KinematicCharacterController {
        up: Vector::Y,
        offset: CharacterLength::Absolute(0.08),
        slide: true,
        autostep: Some(CharacterAutostep {
            max_height: CharacterLength::Absolute(0.3),
            min_width: CharacterLength::Absolute(0.01),
            include_dynamic_bodies: false,
        }),
        max_slope_climb_angle: 45f32.to_radians(),
        // Unity does not slide a character down a slope by itself.
        min_slope_slide_angle: 89f32.to_radians(),
        snap_to_ground: None,
        normal_nudge_factor: 1.0e-4,
    }
}

fn floor(w: &mut PhysicsWorld) {
    w.colliders
        .insert(ColliderBuilder::cuboid(50.0, 0.5, 50.0).translation(Vector::new(0.0, -0.5, 0.0)));
}

fn world_with_floor() -> PhysicsWorld {
    let mut w = world3();
    floor(&mut w);
    sync(&mut w);
    w
}

#[test]
fn capsule_height_is_end_to_end() {
    let c = capsule(2.0, 0.5);
    let aabb = c.compute_aabb(&Pose::IDENTITY);
    assert!((aabb.maxs.y - aabb.mins.y - 2.0).abs() < 1e-5);
    // A capsule shorter than a sphere clamps to the sphere instead of going negative.
    let squat = capsule(0.6, 0.5);
    let aabb = squat.compute_aabb(&Pose::IDENTITY);
    assert!((aabb.maxs.y - aabb.mins.y - 1.0).abs() < 1e-5);
}

/// The rest gap off the ground and off a wall is the offset, whichever side it is
/// measured from, so Blockloom's skin width can be defined as that gap.
#[test]
fn rest_gap_is_the_offset() {
    for offset in [0.01f32, 0.08, 0.2] {
        let mut w = world3();
        floor(&mut w);
        w.colliders
            .insert(ColliderBuilder::cuboid(0.5, 5.0, 5.0).translation(Vector::new(5.0, 0.0, 0.0)));
        sync(&mut w);
        let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 3.0, 0.0));
        c.ctl.offset = CharacterLength::Absolute(offset);
        c.mv(&w, Vector::new(0.0, -10.0, 0.0));
        let ground_gap = c.feet(2.0).y;
        c.mv(&w, Vector::new(10.0, 0.0, 0.0));
        let wall_gap = (5.0 - 0.5) - (c.pos.translation.x + 0.5);
        println!("offset {offset}: ground gap {ground_gap:.4}, wall gap {wall_gap:.4}");
        assert!((ground_gap - offset).abs() < 2e-3);
        assert!((wall_gap - offset).abs() < 2e-3);
    }
}

#[test]
fn flags_and_grounding_follow_the_completed_move() {
    let w = world_with_floor();
    let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 2.0, 0.0));
    // Mid-air: a move that touches nothing sets no flags and is not grounded.
    let (r, f) = c.mv(&w, Vector::new(0.0, -0.5, 0.0));
    println!("falling: {f:?}, grounded {}", r.grounded);
    assert!(!r.grounded && !f.below && !f.sides && !f.above);
    // Landing sets Below and grounded.
    let (r, f) = c.mv(&w, Vector::new(0.0, -5.0, 0.0));
    println!("landing: {f:?}, grounded {}", r.grounded);
    assert!(r.grounded && f.below);
    // Zero move: grounded is still computed from contacts.
    let (r, _) = c.mv(&w, Vector::ZERO);
    assert!(r.grounded, "standing still on the floor");
    // Walking sideways on the floor keeps it.
    let (r, f) = c.mv(&w, Vector::new(1.0, 0.0, 0.0));
    assert!(r.grounded && !f.sides);
    // Lifting off clears it.
    let (r, f) = c.mv(&w, Vector::new(0.0, 1.0, 0.0));
    assert!(!r.grounded && !f.below);
}

#[test]
fn walls_and_ceilings_set_flags() {
    let mut w = world_with_floor();
    w.colliders
        .insert(ColliderBuilder::cuboid(0.5, 5.0, 5.0).translation(Vector::new(4.0, 0.0, 0.0)));
    w.colliders
        .insert(ColliderBuilder::cuboid(5.0, 0.5, 5.0).translation(Vector::new(-20.0, 3.0, 0.0)));
    sync(&mut w);
    let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 0.0, 0.0));
    c.mv(&w, Vector::new(0.0, -1.0, 0.0));
    let (_, f) = c.mv(&w, Vector::new(10.0, 0.0, 0.0));
    println!("wall: {f:?}");
    assert!(f.sides && !f.above);
    // Under a ceiling at y = 2.5..3.5 the capsule (top at 2) jumping up 2 m is stopped.
    let mut under = Character::new(2.0, 0.5, Vector::new(-20.0, 0.0, 0.0));
    under.mv(&w, Vector::new(0.0, -1.0, 0.0));
    let (r, f) = under.mv(&w, Vector::new(0.0, 2.0, 0.0));
    println!("ceiling: {f:?}, rose {:.3}", r.translation.y);
    assert!(f.above);
    assert!(r.translation.y < 0.5);
}

/// A 30 and a 44 degree slope are climbed, a 46 and 60 degree one are not.
#[test]
fn slope_limit() {
    let climb = |degrees: f32| {
        let mut w = world_with_floor();
        let angle = degrees.to_radians();
        // A long ramp rising towards +x, its low edge on the floor at x = 2.
        let len = 30.0;
        let center = Vector::new(
            2.0 + len / 2.0 * angle.cos(),
            len / 2.0 * angle.sin() - 0.25 / angle.cos().max(0.1),
            0.0,
        );
        w.colliders.insert(
            ColliderBuilder::cuboid(len / 2.0, 0.25, 5.0)
                .position(Pose::from_parts(center, Rotation::from_rotation_z(angle))),
        );
        sync(&mut w);
        let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 0.0, 0.0));
        c.mv(&w, Vector::new(0.0, -1.0, 0.0));
        for _ in 0..60 {
            // 6 m/s walking, plus gravity applied by the caller as a separate move.
            c.mv(&w, Vector::new(0.1, 0.0, 0.0));
            c.mv(&w, Vector::new(0.0, -0.1, 0.0));
        }
        (c.pos.translation.x, c.feet(2.0).y)
    };
    let mut rows = Vec::new();
    for deg in [30.0, 44.0, 46.0, 60.0] {
        let (x, y) = climb(deg);
        rows.push((deg, x, y));
    }
    println!("slope (deg, x, feet y): {rows:?}");
    assert!(
        rows[0].2 > 1.0 && rows[1].2 > 1.0,
        "walkable slopes are climbed"
    );
    assert!(
        rows[2].2 < 0.5 && rows[3].2 < 0.5,
        "steeper than the limit is a wall"
    );
}

/// Rapier's autostep is not Unity's step offset: how high it climbs depends on the
/// capsule radius, the skin and the rounded base rolling over the edge, and barely on
/// `max_height`. Recorded so the adapter does not map one to the other.
#[test]
fn rapier_autostep_is_not_the_step_offset() {
    let climbs = |radius: f32, max_height: f32, offset: f32, height: f32| {
        let mut w = world_with_floor();
        w.colliders.insert(
            ColliderBuilder::cuboid(10.0, height / 2.0, 5.0).translation(Vector::new(
                13.0,
                height / 2.0,
                0.0,
            )),
        );
        sync(&mut w);
        let mut c = Character::new(2.0, radius, Vector::new(0.0, 0.0, 0.0));
        c.ctl.offset = CharacterLength::Absolute(offset);
        c.ctl.autostep = Some(CharacterAutostep {
            max_height: CharacterLength::Absolute(max_height),
            min_width: CharacterLength::Absolute(0.05),
            include_dynamic_bodies: false,
        });
        c.mv(&w, Vector::new(0.0, -1.0, 0.0));
        for _ in 0..120 {
            c.mv(&w, Vector::new(0.05, 0.0, 0.0));
            c.mv(&w, Vector::new(0.0, -0.05, 0.0));
        }
        c.pos.translation.x > 5.0
    };
    let limit = |radius, max_height, offset| {
        let (mut lo, mut hi) = (0.0f32, 1.5f32);
        for _ in 0..10 {
            let mid = (lo + hi) / 2.0;
            if climbs(radius, max_height, offset, mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    let cases = [
        (0.3, 0.3, 0.01),
        (0.3, 0.3, 0.08),
        (0.5, 0.3, 0.08),
        (0.5, 0.1, 0.08),
    ];
    let limits: Vec<f32> = cases.iter().map(|c| limit(c.0, c.1, c.2)).collect();
    println!(
        "(radius, max_height, offset) -> highest step climbed: {:?}",
        cases.iter().zip(&limits).collect::<Vec<_>>()
    );
    // The configured 0.3 does not predict the limit: it moves with the skin and the radius.
    let spread = limits.iter().cloned().fold(f32::MIN, f32::max)
        - limits.iter().cloned().fold(f32::MAX, f32::min);
    assert!(spread > 0.15, "{limits:?}");
}

/// A step climb of our own, adapter stage 4 prototyped on Rapier's casts. A ray dropped
/// just past the riser finds the top surface: it must be walkable and no higher than the
/// step offset above the support, which also rejects a steep ramp (its surface is not
/// walkable). The capsule then rises by exactly that height and moves on, and the
/// caller's gravity move settles it.
#[allow(clippy::too_many_arguments)]
fn step_up(
    w: &PhysicsWorld,
    shape: &SharedShape,
    pos: &Pose,
    feet_y: f32,
    reach: f32,
    forward: Vector,
    step: f32,
    offset: f32,
    max_slope: f32,
) -> Option<Vector> {
    let q = w.query_pipeline();
    let cast = |from: &Pose, dir: Vector, dist: f32| {
        let options = rapier3d::parry::query::details::ShapeCastOptions {
            target_distance: offset,
            stop_at_penetration: false,
            max_time_of_impact: dist,
            compute_impact_geometry_on_penetration: true,
        };
        q.cast_shape(from, dir, shape.as_ref(), options)
    };
    let free = |from: &Pose, dir: Vector, dist: f32| {
        cast(from, dir, dist).map_or(dist, |(_, hit)| hit.time_of_impact)
    };
    let len = forward.length();
    let dir = forward / len;
    // The capsule rests `offset` above its support.
    let support = feet_y - offset;
    let probe_top = support + step + 0.02;
    let from = Vector::new(
        pos.translation.x + dir.x * (reach + 0.02),
        probe_top,
        pos.translation.z + dir.z * (reach + 0.02),
    );
    let ray = Ray::new(from, -Vector::Y);
    let (_, top) = q.cast_ray_and_get_normal(&ray, step + 0.02 + offset, true)?;
    if top.time_of_impact <= 0.0 || top.normal.dot(Vector::Y) < max_slope.cos() {
        return None; // higher than the step, or not a walkable top
    }
    let raise = probe_top - top.time_of_impact - support;
    if raise <= 1e-4 || raise > step + 1e-4 {
        return None;
    }
    if free(pos, Vector::Y, raise) < raise - 1e-4 {
        return None; // no headroom
    }
    let raised = Pose::from_translation(pos.translation + Vector::Y * raise);
    let ahead = free(&raised, dir, len);
    if ahead < 0.9 * len {
        return None;
    }
    Some(dir * ahead + Vector::Y * raise)
}

/// With our own stepping on top of a controller that has no autostep, the climb limit is
/// the step offset, whatever the radius and skin.
#[test]
fn own_step_logic_honours_the_step_offset() {
    let step = 0.3;
    let mut rows = Vec::new();
    for radius in [0.3f32, 0.5] {
        for offset in [0.01f32, 0.08] {
            let climbs = |height: f32| {
                let mut w = world_with_floor();
                w.colliders.insert(
                    ColliderBuilder::cuboid(10.0, height / 2.0, 5.0).translation(Vector::new(
                        13.0,
                        height / 2.0,
                        0.0,
                    )),
                );
                sync(&mut w);
                let mut c = Character::new(2.0, radius, Vector::new(0.0, 0.0, 0.0));
                c.ctl.offset = CharacterLength::Absolute(offset);
                c.ctl.autostep = None;
                c.mv(&w, Vector::new(0.0, -1.0, 0.0));
                for _ in 0..200 {
                    let forward = Vector::new(0.05, 0.0, 0.0);
                    let (r, f) = c.mv(&w, forward);
                    if f.sides && r.translation.x < 0.04 {
                        let feet_y = c.feet(2.0).y;
                        let reach = radius + offset;
                        if let Some(d) = step_up(
                            &w,
                            &c.shape,
                            &c.pos,
                            feet_y,
                            reach,
                            forward,
                            step,
                            offset,
                            45f32.to_radians(),
                        ) {
                            c.pos.translation += d;
                        }
                    }
                    c.mv(&w, Vector::new(0.0, -0.05, 0.0));
                }
                c.pos.translation.x > 5.0
            };
            rows.push((radius, offset, climbs(0.28), climbs(0.34)));
        }
    }
    println!("(radius, offset, climbs 0.28, climbs 0.34): {rows:?}");
    for (_, _, low, high) in rows {
        assert!(low, "below the step offset climbs");
        assert!(!high, "above the step offset blocks");
    }
}

/// Sliding along a wall keeps only the tangent part and never lifts the character.
#[test]
fn wall_slide_keeps_the_tangent_and_adds_no_energy() {
    let mut w = world_with_floor();
    w.colliders
        .insert(ColliderBuilder::cuboid(0.5, 5.0, 20.0).translation(Vector::new(3.0, 0.0, 0.0)));
    sync(&mut w);
    let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 0.0, 0.0));
    c.mv(&w, Vector::new(0.0, -1.0, 0.0));
    let before = c.pos.translation;
    let d = Vector::new(6.0, 0.0, 6.0);
    let (r, _) = c.mv(&w, d);
    let moved = c.pos.translation - before;
    println!("diagonal into a wall: moved {moved:?}");
    assert!(moved.y.abs() < 1e-3, "no climb");
    assert!(moved.z > 5.0, "the tangent part survives");
    assert!(moved.length() <= d.length() + 1e-3);
    assert!(r.translation.x < 3.0);
}

/// One big move cannot pass through a 1 cm wall.
#[test]
fn thin_wall_stops_a_large_move() {
    let mut w = world_with_floor();
    w.colliders
        .insert(ColliderBuilder::cuboid(0.005, 5.0, 20.0).translation(Vector::new(30.0, 0.0, 0.0)));
    sync(&mut w);
    let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 0.0, 0.0));
    c.mv(&w, Vector::new(0.0, -1.0, 0.0));
    c.mv(&w, Vector::new(100.0, 0.0, 0.0));
    println!("stopped at x = {:.3}", c.pos.translation.x);
    assert!(c.pos.translation.x < 30.0);
}

/// Overlap recovery is bounded per call: a shallow overlap is resolved in one, a deep
/// one takes several, and each call moves at most a quarter of the height. The
/// adapter owns the call count and reports a capsule that is still stuck.
#[test]
fn overlap_recovery_is_bounded_per_call() {
    let mut w = world_with_floor();
    w.colliders
        .insert(ColliderBuilder::cuboid(0.5, 5.0, 5.0).translation(Vector::new(0.0, 0.0, 0.0)));
    sync(&mut w);
    // The wall spans x in [-0.5, 0.5]; the capsule has radius 0.5 and height 2.
    let settle = |start_x: f32, calls: usize| {
        let mut c = Character::new(2.0, 0.5, Vector::new(start_x, 0.0, 0.0));
        let mut steps = Vec::new();
        for _ in 0..calls {
            let before = c.pos.translation.x;
            c.mv(&w, Vector::ZERO);
            steps.push(c.pos.translation.x - before);
        }
        (c.pos.translation.x, steps)
    };
    let (shallow, s1) = settle(-0.95, 1);
    let (deep_one, d1) = settle(-0.2, 1);
    let (deep_four, d4) = settle(-0.2, 4);
    println!(
        "shallow overlap 0.05: {shallow:.3} in one call {s1:?}; deep overlap 0.8: {deep_one:.3} \
         after one call {d1:?}, {deep_four:.3} after four {d4:?}"
    );
    assert!(shallow < -1.0, "a shallow overlap is resolved in a call");
    assert!(
        d1[0].abs() <= 0.5 + 1e-3,
        "one call moves at most a quarter of the height"
    );
    assert!(deep_one > -0.9, "a deep overlap needs more than one call");
    assert!(deep_four < -1.0, "and gets out in a few");
}

/// Is the capsule at `pose` free of solid geometry? Uses the distance query, which is
/// accurate to a tenth of a millimetre; `intersect_shape` and `contact` both misreport a
/// capsule against a box within a centimetre of touching.
fn clear_at(w: &PhysicsWorld, shape: &SharedShape, pose: Pose) -> bool {
    let q = w.query_pipeline();
    let aabb = shape.compute_aabb(&pose);
    q.intersect_aabb_conservative(aabb).all(|(_, collider)| {
        collider.is_sensor() || {
            let pos12 = pose.inv_mul(collider.position());
            q.dispatcher
                .distance(&pos12, shape.as_ref(), collider.shape())
                .is_ok_and(|d| d.distance > 1.0e-4)
        }
    })
}

/// `intersect_shape` on a capsule against a box answers inconsistently for gaps below
/// about a centimetre. Recorded so nobody builds headroom checks on it.
#[test]
fn intersection_test_flickers_near_contact() {
    let w = world_with_floor();
    let capsule = capsule(2.0, 0.5);
    let q = w.query_pipeline();
    let pattern: Vec<bool> = (0..20)
        .map(|i| {
            let pose = Pose::from_translation(Vector::new(0.0, 1.0 + i as f32 * 0.001, 0.0));
            q.intersect_shape(pose, capsule.as_ref()).next().is_some()
        })
        .collect();
    println!("intersect_shape by gap in mm: {pattern:?}");
    let flips = pattern.windows(2).filter(|p| p[0] != p[1]).count();
    assert!(flips > 1, "not monotonic: {pattern:?}");
    // The contact-distance test is monotonic over the same range.
    let contact: Vec<bool> = (0..20)
        .map(|i| {
            clear_at(
                &w,
                &capsule,
                Pose::from_translation(Vector::new(0.0, 1.0 + i as f32 * 0.001, 0.0)),
            )
        })
        .collect();
    assert!(contact.windows(2).all(|p| p[0] <= p[1]), "{contact:?}");
}

/// Standing from a crouch is a sweep of the standing capsule with the feet anchored.
#[test]
fn stand_up_is_refused_under_a_low_ceiling() {
    let can_stand = |ceiling: f32| {
        let mut w = world_with_floor();
        w.colliders.insert(
            ColliderBuilder::cuboid(5.0, 0.5, 5.0).translation(Vector::new(
                0.0,
                ceiling + 0.5,
                0.0,
            )),
        );
        sync(&mut w);
        // Standing capsule centred 1.0 above the feet, nudged up by the skin.
        clear_at(
            &w,
            &capsule(2.0, 0.5),
            Pose::from_translation(Vector::new(0.0, 1.01, 0.0)),
        )
    };
    assert!(can_stand(2.5));
    assert!(can_stand(2.02));
    assert!(!can_stand(1.9));
    assert!(!can_stand(1.5));
}

/// Rapier's controller carries a character on a kinematic support itself, but only as part
/// of a move that hit the support, and it lags: here the character falls 0.4 m behind a
/// 3 m/s platform before the lead settles. The carry is swept (a wall stops it at the skin)
/// and the character never sinks. Held finding: the Unity adapter cannot rely on this
/// carry, so Phase 4 either owns the carry (and neutralises Rapier's) or extends the
/// controller with a switch (see the ledger, row "moving platforms").
#[test]
fn builtin_platform_carry_lags_but_is_swept() {
    let mut w = world3();
    let (platform, _) = w.insert(
        RigidBodyBuilder::kinematic_velocity_based()
            .translation(Vector::new(0.0, -0.5, 0.0))
            .linvel(Vector::new(3.0, 0.0, 0.0)),
        ColliderBuilder::cuboid(3.0, 0.5, 3.0),
    );
    w.colliders
        .insert(ColliderBuilder::cuboid(0.5, 5.0, 5.0).translation(Vector::new(6.0, 0.0, 0.0)));
    sync(&mut w);
    // `sync` on a never-stepped velocity-based kinematic body drops it from the active set.
    w.bodies[platform].wake_up(true);
    let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 0.0, 0.0));
    c.mv(&w, Vector::new(0.0, -0.5, 0.0));
    let mut worst_lag = 0.0f32;
    let mut lowest = 0.0f32;
    for tick in 0..140 {
        // Ground probe longer than the skin: the only move the character makes.
        c.mv(&w, Vector::new(0.0, -0.2, 0.0));
        w.step();
        let lead = c.pos.translation.x - w.bodies[platform].translation().x;
        // Before the wall comes into play.
        if tick < 70 {
            worst_lag = worst_lag.min(lead);
        }
        lowest = lowest.min(c.feet(2.0).y);
    }
    println!(
        "character x {:.3}, platform x {:.3}, worst lead {worst_lag:.3}, lowest feet {lowest:.3}",
        c.pos.translation.x,
        w.bodies[platform].translation().x
    );
    assert!(worst_lag < -0.2, "the built-in carry lags the platform");
    assert!(lowest > -0.1, "but never sinks");
    assert!(
        (c.pos.translation.x - 4.92).abs() < 0.05,
        "the wall stops it at wall - radius - skin"
    );
}

/// Adding a second, explicit carry on top of the controller's own double counts on some
/// ticks, so the offset to a position-based platform wanders by whole ticks of travel.
#[test]
fn explicit_carry_double_counts_with_the_controllers_own() {
    let mut w = world3();
    let (platform, _) = w.insert(
        RigidBodyBuilder::kinematic_position_based().translation(Vector::new(0.0, -0.5, 0.0)),
        ColliderBuilder::cuboid(3.0, 0.5, 3.0),
    );
    sync(&mut w);
    let mut c = Character::new(2.0, 0.5, Vector::new(0.0, 0.0, 0.0));
    c.mv(&w, Vector::new(0.0, -0.5, 0.0));
    let (mut min_lead, mut max_lead) = (f32::MAX, f32::MIN);
    for tick in 0..70 {
        let carry = Vector::new(3.0 * DT, 0.0, 0.0);
        c.mv(&w, carry);
        c.mv(&w, Vector::new(0.0, -0.05, 0.0));
        let next = w.bodies[platform].translation() + carry;
        w.bodies[platform].set_next_kinematic_translation(next);
        w.step();
        if tick >= 10 {
            let lead = c.pos.translation.x - w.bodies[platform].translation().x;
            min_lead = min_lead.min(lead);
            max_lead = max_lead.max(lead);
        }
    }
    println!("explicit carry: offset to the platform {min_lead:.3}..{max_lead:.3}");
    assert!(max_lead - min_lead > 0.03);
}

/// A velocity-based kinematic body that is synced before its first step never moves: the
/// sync drops it from the active set. A waking call after the sync (or any step first)
/// brings it back, so the fixed step's sync stage must wake bodies spawned that tick.
#[test]
fn syncing_a_fresh_velocity_kinematic_body_freezes_it_until_woken() {
    let spawn = |wake: bool| {
        let mut w = world3();
        let (b, _) = w.insert(
            RigidBodyBuilder::kinematic_velocity_based().linvel(Vector::new(3.0, 0.0, 0.0)),
            ColliderBuilder::cuboid(1.0, 1.0, 1.0),
        );
        sync(&mut w);
        if wake {
            w.bodies[b].wake_up(true);
        }
        run(&mut w, 30);
        w.bodies[b].translation().x
    };
    assert!(spawn(false).abs() < 1e-4, "frozen");
    assert!(
        (spawn(true) - 1.5).abs() < 0.01,
        "moves 3 m/s for half a second"
    );
}
