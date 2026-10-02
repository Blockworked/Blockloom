//! Ledger rows: separate static and dynamic friction, combine modes.
//!
//! Unity's material has a static and a dynamic friction. Rapier's contact has
//! one Coulomb coefficient per manifold, so the plan's "mapping both controls
//! to one coefficient is unacceptable" needs an adapter. The probe builds the
//! adapter as a contact-modification hook and holds it against the incline
//! experiment that separates the two coefficients.

use std::collections::HashMap;

use blockloom_physics_probes::*;
use rapier3d::prelude::*;

/// tan of the slope angle. Between the dynamic (0.3) and static (0.8) friction below.
const TAN: f32 = 0.6;
const STATIC: f32 = 0.8;
const DYNAMIC: f32 = 0.3;

struct Slope {
    w: PhysicsWorld,
    block: RigidBodyHandle,
    start: Vector,
    along: Vector,
}

/// A block resting on a tilted floor. `slide` is the speed downhill it starts with.
fn slope(floor: ColliderBuilder, block: ColliderBuilder, slide: f32) -> Slope {
    let angle = TAN.atan();
    let mut w = world3();
    let tilt = Vector::new(0.0, 0.0, angle);
    let normal = Vector::new(-angle.sin(), angle.cos(), 0.0);
    let along = Vector::new(angle.cos(), angle.sin(), 0.0);
    w.insert(RigidBodyBuilder::fixed().rotation(tilt), floor);
    let start = normal * 1.001;
    let (block_body, _) = w.insert(
        RigidBodyBuilder::dynamic()
            .translation(start)
            .rotation(tilt)
            .linvel(-along * slide),
        block,
    );
    Slope {
        w,
        block: block_body,
        start,
        along,
    }
}

impl Slope {
    /// Metres slid downhill after `steps` fixed steps.
    fn slid(&mut self, steps: usize) -> f32 {
        run(&mut self.w, steps);
        -(self.w.bodies[self.block].translation() - self.start).dot(self.along)
    }
}

fn floor() -> ColliderBuilder {
    ColliderBuilder::cuboid(40.0, 0.5, 40.0)
}

fn block() -> ColliderBuilder {
    ColliderBuilder::cuboid(0.5, 0.5, 0.5)
}

/// One friction coefficient per collider pair: no value serves both halves of the
/// Unity experiment, which is why a single coefficient is not an adapter.
#[test]
fn one_coefficient_cannot_be_both_static_and_dynamic() {
    let sticky = |slide| slope(floor().friction(STATIC), block().friction(STATIC), slide).slid(180);
    let slippery =
        |slide| slope(floor().friction(DYNAMIC), block().friction(DYNAMIC), slide).slid(180);
    let (rest_sticky, push_sticky) = (sticky(0.0), sticky(3.0));
    let (rest_slippery, push_slippery) = (slippery(0.0), slippery(3.0));
    println!(
        "single coefficient: sticky rest {rest_sticky:.3} m, sticky pushed {push_sticky:.3} m, \
         slippery rest {rest_slippery:.3} m, slippery pushed {push_slippery:.3} m"
    );
    // Unity wants: rest stays put, a pushed block keeps sliding.
    assert!(rest_sticky < 0.05, "sticky holds at rest");
    assert!(
        push_sticky < 3.5,
        "sticky also stops a block that is already sliding"
    );
    assert!(rest_slippery > 5.0, "slippery slides from rest");
}

/// Picks static or dynamic friction per manifold from the contact's own sliding speed.
struct StickSlip {
    /// Static and dynamic friction per collider; the combine rule is the project's.
    materials: HashMap<ColliderHandle, (f32, f32)>,
    /// Sliding speed below which a contact counts as stuck.
    stick_speed: f32,
}

impl PhysicsHooks for StickSlip {
    fn modify_solver_contacts(&self, ctx: &mut ContactModificationContext) {
        let (Some(a), Some(b)) = (
            self.materials.get(&ctx.collider1),
            self.materials.get(&ctx.collider2),
        ) else {
            return;
        };
        let (b1, b2) = (ctx.rigid_body1, ctx.rigid_body2);
        let bodies = ctx.bodies;
        let Some(rigid) = ctx.rigid_mut() else { return };
        let Some(first) = rigid.solver_contacts.first() else {
            return;
        };
        let point = (first.anchor1 + first.anchor2) * 0.5;
        let velocity = |body: Option<RigidBodyHandle>| {
            body.map(|h| bodies[h].velocity_at_point(point))
                .unwrap_or(Vector::ZERO)
        };
        let relative = velocity(b2) - velocity(b1);
        let n = *rigid.normal;
        let sliding = (relative - n * relative.dot(n)).length();
        let rule = CoefficientCombineRule::Average;
        let (s, d) = (
            CoefficientCombineRule::combine(a.0, b.0, rule, rule),
            CoefficientCombineRule::combine(a.1, b.1, rule, rule),
        );
        *rigid.friction = if sliding < self.stick_speed { s } else { d };
    }
}

fn hooked(slide: f32) -> (Slope, StickSlip) {
    let mut s = slope(floor(), block(), slide);
    let mut materials = HashMap::new();
    let handles: Vec<_> = s.w.colliders.iter().map(|(h, _)| h).collect();
    for h in handles {
        let c = &mut s.w.colliders[h];
        c.set_active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
        materials.insert(h, (STATIC, DYNAMIC));
    }
    (
        s,
        StickSlip {
            materials,
            stick_speed: 0.02,
        },
    )
}

fn slid_with(s: &mut Slope, hooks: &StickSlip, steps: usize) -> f32 {
    for _ in 0..steps {
        s.w.step_with_events(hooks, &());
    }
    -(s.w.bodies[s.block].translation() - s.start).dot(s.along)
}

/// The adapter reproduces both halves: stick at rest below the static limit,
/// keep sliding once moving, at the dynamic coefficient.
#[test]
fn contact_hook_gives_stick_then_slip() {
    let (mut at_rest, hooks) = hooked(0.0);
    let rest = slid_with(&mut at_rest, &hooks, 180);
    let (mut pushed, hooks) = hooked(3.0);
    let push = slid_with(&mut pushed, &hooks, 180);
    let speed = pushed.w.bodies[pushed.block].linvel().length();
    // Dynamic friction 0.3 on a 0.6 slope: a = g (sin - 0.3 cos) from 3 m/s.
    let angle = TAN.atan();
    let accel = 9.81 * (angle.sin() - DYNAMIC * angle.cos());
    let expected = 3.0 + accel * 3.0;
    println!(
        "hook: rest slid {rest:.3} m, pushed slid {push:.2} m, speed {speed:.2} m/s \
         (analytic {expected:.2} m/s)"
    );
    assert!(rest < 0.05, "static friction holds the block");
    assert!(
        push > 10.0,
        "dynamic friction lets a moving block keep sliding"
    );
    assert!(
        (speed - expected).abs() < 0.5,
        "dynamic coefficient is the one applied"
    );
}

/// A block pushed past the static limit breaks away: it was held by static friction
/// only until something moved it.
#[test]
fn contact_hook_breaks_away_when_shoved() {
    let (mut s, hooks) = hooked(0.0);
    slid_with(&mut s, &hooks, 30);
    s.w.bodies[s.block].apply_impulse(-s.along * 4.0, true);
    let slid = slid_with(&mut s, &hooks, 120);
    println!("hook: shoved block slid {slid:.2} m");
    assert!(slid > 3.0);
}

/// Unity's mixed-mode priority is Maximum > Multiply > Minimum > Average, which is
/// Rapier's order with its two extra rules above and below.
#[test]
fn combine_priority_matches_unity() {
    use CoefficientCombineRule::*;
    let order = [Average, Min, Multiply, Max];
    for (i, lo) in order.iter().enumerate() {
        for hi in &order[i + 1..] {
            // 0.2 and 0.8 give a different answer under each rule.
            let mixed = CoefficientCombineRule::combine(0.2, 0.8, *lo, *hi);
            let pure = CoefficientCombineRule::combine(0.2, 0.8, *hi, *hi);
            assert_eq!(mixed, pure, "{hi:?} outranks {lo:?}");
        }
    }
    let r = |rule| CoefficientCombineRule::combine(0.2, 0.8, rule, rule);
    assert!((r(Average) - 0.5).abs() < 1e-6);
    assert!((r(Min) - 0.2).abs() < 1e-6);
    assert!((r(Multiply) - 0.16).abs() < 1e-6);
    assert!((r(Max) - 0.8).abs() < 1e-6);
}
