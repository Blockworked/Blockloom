//! What the scene view draws for an actor's invisible parts: collider shapes,
//! the character controller's capsule, joint anchors and axes, light reach
//! and the camera an actor carries. Pure line lists, so tests can read them;
//! `edit::draw_overlay` hands them to Bevy's gizmos.

use bevy::prelude::*;
use blockloom_core::components::LightKind;
use blockloom_core::physics::joints::ConstraintKind;
use blockloom_core::physics::spec::{Axis, ColliderGeometry, ColliderShape};
use blockloom_core::project::Actor;
use blockloom_core::scene::Mode;
use std::f32::consts::TAU;

/// One straight piece of a gizmo, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Color,
}

pub const COLLIDER: Color = Color::srgba(0.4, 1.0, 0.5, 0.9);
pub const TRIGGER: Color = Color::srgba(0.4, 0.8, 1.0, 0.7);
pub const CONTROLLER: Color = Color::srgba(1.0, 0.85, 0.3, 0.9);
pub const JOINT: Color = Color::srgba(1.0, 0.45, 0.9, 0.95);
pub const LIGHT: Color = Color::srgba(1.0, 0.9, 0.4, 0.7);
pub const CAMERA: Color = Color::srgba(0.8, 0.85, 1.0, 0.7);

const SEGMENTS: usize = 24;

#[derive(Default)]
struct Pen(Vec<Line>);

impl Pen {
    fn line(&mut self, a: Vec3, b: Vec3, color: Color) {
        self.0.push(Line { a, b, color });
    }
    fn strip(&mut self, points: &[Vec3], color: Color) {
        for pair in points.windows(2) {
            self.line(pair[0], pair[1], color);
        }
    }
    /// A circle of `radius` round `center` in the plane spanned by `u` and `v`.
    fn circle(
        &mut self,
        at: &Transform,
        center: Vec3,
        u: Vec3,
        v: Vec3,
        radius: f32,
        color: Color,
    ) {
        let points: Vec<Vec3> = (0..=SEGMENTS)
            .map(|i| {
                let t = i as f32 / SEGMENTS as f32 * TAU;
                at.transform_point(center + (u * t.cos() + v * t.sin()) * radius)
            })
            .collect();
        self.strip(&points, color);
    }
    fn cuboid(&mut self, at: &Transform, half: Vec3, color: Color) {
        let corner = |i: usize| {
            at.transform_point(Vec3::new(
                if i & 1 == 0 { -half.x } else { half.x },
                if i & 2 == 0 { -half.y } else { half.y },
                if i & 4 == 0 { -half.z } else { half.z },
            ))
        };
        for (a, b) in [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            self.line(corner(a), corner(b), color);
        }
    }
    fn rect(&mut self, at: &Transform, half: Vec2, color: Color) {
        let p = |x: f32, y: f32| at.transform_point(Vec3::new(x, y, 0.0));
        self.strip(
            &[
                p(-half.x, -half.y),
                p(half.x, -half.y),
                p(half.x, half.y),
                p(-half.x, half.y),
                p(-half.x, -half.y),
            ],
            color,
        );
    }
    /// A capsule along local Y: `half` is the straight part's half length.
    fn capsule(&mut self, at: &Transform, radius: f32, half: f32, color: Color) {
        for y in [-half, half] {
            self.circle(at, Vec3::Y * y, Vec3::X, Vec3::Z, radius, color);
        }
        for side in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
            self.line(
                at.transform_point(side * radius + Vec3::Y * half),
                at.transform_point(side * radius - Vec3::Y * half),
                color,
            );
        }
        // The caps, as half circles in both upright planes.
        for (end, sign) in [(half, 1.0), (-half, -1.0)] {
            for u in [Vec3::X, Vec3::Z] {
                let points: Vec<Vec3> = (0..=SEGMENTS / 2)
                    .map(|i| {
                        let t = i as f32 / SEGMENTS as f32 * TAU;
                        at.transform_point(
                            Vec3::Y * end + (u * t.cos() + Vec3::Y * sign * t.sin()) * radius,
                        )
                    })
                    .collect();
                self.strip(&points, color);
            }
        }
    }
    fn polygon(&mut self, at: &Transform, points: &[[f32; 2]], closed: bool, color: Color) {
        let mut world: Vec<Vec3> = points
            .iter()
            .map(|p| at.transform_point(Vec3::new(p[0], p[1], 0.0)))
            .collect();
        if closed && let Some(first) = world.first().copied() {
            world.push(first);
        }
        self.strip(&world, color);
    }
}

fn euler(degrees: [f32; 3]) -> Quat {
    Quat::from_euler(
        EulerRot::XYZ,
        degrees[0].to_radians(),
        degrees[1].to_radians(),
        degrees[2].to_radians(),
    )
}

fn shape_lines(pen: &mut Pen, shape: &ColliderShape, at: &Transform, color: Color) {
    match shape {
        ColliderShape::Box { size } => pen.cuboid(at, Vec3::from(*size) / 2.0, color),
        ColliderShape::Sphere { radius } => {
            for (u, v) in [(Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::X, Vec3::Z)] {
                pen.circle(at, Vec3::ZERO, u, v, *radius, color);
            }
        }
        ColliderShape::Capsule {
            radius,
            height,
            axis,
        } => {
            // End to end; turned so local Y runs along its axis.
            let turn = match axis {
                Axis::X => Quat::from_rotation_z(-TAU / 4.0),
                Axis::Z => Quat::from_rotation_x(TAU / 4.0),
                Axis::Y => Quat::IDENTITY,
            };
            let along = Transform {
                rotation: at.rotation * turn,
                ..*at
            };
            pen.capsule(&along, *radius, (height / 2.0 - radius).max(0.0), color);
        }
        ColliderShape::Rect { size } => pen.rect(at, Vec2::from(*size) / 2.0, color),
        ColliderShape::Circle { radius } => {
            pen.circle(at, Vec3::ZERO, Vec3::X, Vec3::Y, *radius, color)
        }
        ColliderShape::Capsule2d { size, horizontal } => {
            // A box with the round ends' full circles on it.
            let (w, h) = (size[0] / 2.0, size[1] / 2.0);
            let (radius, reach, along) = if *horizontal {
                (h, (w - h).max(0.0), Vec3::X)
            } else {
                (w, (h - w).max(0.0), Vec3::Y)
            };
            for end in [reach, -reach] {
                pen.circle(at, along * end, Vec3::X, Vec3::Y, radius, color);
            }
            pen.rect(at, Vec2::new(w, h), color.with_alpha(0.4));
        }
        ColliderShape::Polygon { points } => pen.polygon(at, points, true, color),
        ColliderShape::Edge { a, b } => pen.polygon(at, &[*a, *b], false, color),
        ColliderShape::Chain { points, closed } => pen.polygon(at, points, *closed, color),
        // Meshes, terrain and tiles draw as themselves; a cross marks them.
        ColliderShape::ConvexHull { .. }
        | ColliderShape::TriangleMesh { .. }
        | ColliderShape::Terrain
        | ColliderShape::Tilemap => {
            let o = at.translation;
            for d in [Vec3::X, Vec3::Y, Vec3::Z] {
                pen.line(o - d * 0.2, o + d * 0.2, color);
            }
        }
    }
}

/// Every gizmo line of `actor` standing at `pose`. `pose_of` finds another
/// actor's pose by id or name, for a joint's other end.
pub fn actor_lines(
    actor: &Actor,
    pose: &Transform,
    mode: Mode,
    pose_of: &dyn Fn(&str) -> Option<Transform>,
) -> Vec<Line> {
    let mut pen = Pen::default();
    for collider in actor.components.colliders().filter(|c| c.enabled) {
        let shape = match &collider.geometry {
            ColliderGeometry::Shape { shape } => Some(shape.clone()),
            ColliderGeometry::FromLook => actor
                .visual()
                .and_then(|v| blockloom_core::physics::shape_from_look(v, mode)),
        };
        let Some(shape) = shape.filter(|s| s.fits(mode)) else {
            continue;
        };
        let local = Transform::from_translation(Vec3::from(collider.center))
            .with_rotation(euler(collider.rotation));
        let at = pose.mul_transform(local);
        let color = if collider.trigger { TRIGGER } else { COLLIDER };
        shape_lines(&mut pen, &shape, &at, color);
    }
    if let Some(controller) = actor.components.character_controller() {
        let at = pose.mul_transform(Transform::from_translation(Vec3::from(controller.center)));
        // Upright and unscaled: the controller ignores the actor's turn.
        let at = Transform::from_translation(at.translation);
        if mode == Mode::ThreeD {
            let half = (controller.height / 2.0 - controller.radius).max(0.0);
            pen.capsule(&at, controller.radius, half, CONTROLLER);
        } else {
            let half = Vec2::new(controller.radius, controller.height / 2.0);
            pen.rect(&at, half, CONTROLLER);
        }
    }
    for joint in actor.components.constraints().filter(|j| j.enabled) {
        let anchor = pose.transform_point(Vec3::from(joint.anchor));
        let other = if joint.target.trim().is_empty() {
            Some(Vec3::from(joint.connected_anchor))
        } else {
            pose_of(joint.target.trim())
                .map(|t| t.transform_point(Vec3::from(joint.connected_anchor)))
        };
        let mark = 0.08 * if mode == Mode::TwoD { 100.0 } else { 1.0 };
        for d in [Vec3::X, Vec3::Y, Vec3::Z] {
            pen.line(anchor - d * mark, anchor + d * mark, JOINT);
        }
        if let Some(other) = other
            && !joint.auto_configure
        {
            pen.line(anchor, other, JOINT.with_alpha(0.5));
        }
        if matches!(joint.kind, ConstraintKind::Hinge | ConstraintKind::Slider) {
            let axis = pose.rotation * Vec3::from(joint.axis).normalize_or(Vec3::X);
            pen.line(
                anchor - axis * mark * 4.0,
                anchor + axis * mark * 4.0,
                JOINT,
            );
        }
    }
    if mode == Mode::ThreeD
        && let Some(light) = actor.components.light()
    {
        let at = Transform {
            scale: Vec3::ONE,
            ..*pose
        };
        let reach = light.range.max(0.0);
        match light.kind {
            LightKind::Spot => {
                let half = light.outer_angle.to_radians() / 2.0;
                let radius = reach * half.tan();
                let tip = Vec3::NEG_Z * reach;
                pen.circle(&at, tip, Vec3::X, Vec3::Y, radius, LIGHT);
                for side in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y] {
                    pen.line(
                        at.translation,
                        at.transform_point(tip + side * radius),
                        LIGHT,
                    );
                }
            }
            LightKind::Rect => {
                pen.rect(&at, Vec2::new(light.width, light.height) / 2.0, LIGHT);
                pen.line(
                    at.translation,
                    at.transform_point(Vec3::NEG_Z * reach.min(1.0)),
                    LIGHT,
                );
            }
            LightKind::Disk => {
                pen.circle(&at, Vec3::ZERO, Vec3::X, Vec3::Y, light.width / 2.0, LIGHT);
                pen.line(
                    at.translation,
                    at.transform_point(Vec3::NEG_Z * reach.min(1.0)),
                    LIGHT,
                );
            }
            _ => {
                for (u, v) in [(Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::X, Vec3::Z)] {
                    pen.circle(&at, Vec3::ZERO, u, v, reach, LIGHT.with_alpha(0.35));
                }
            }
        }
    }
    if mode == Mode::ThreeD && actor.components.camera().is_some() {
        // A small frustum looking down the actor's forward axis.
        let at = Transform {
            scale: Vec3::ONE,
            ..*pose
        };
        let (w, h, d) = (0.5, 0.375, 0.8);
        let corners = [
            Vec3::new(-w, -h, -d),
            Vec3::new(w, -h, -d),
            Vec3::new(w, h, -d),
            Vec3::new(-w, h, -d),
        ]
        .map(|c| at.transform_point(c));
        for corner in corners {
            pen.line(at.translation, corner, CAMERA);
        }
        pen.strip(
            &[corners[0], corners[1], corners[2], corners[3], corners[0]],
            CAMERA,
        );
    }
    pen.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::components::ActorComponent;
    use blockloom_core::physics::joints::ConstraintSpec;
    use blockloom_core::physics::spec::ColliderSpec;
    use blockloom_core::scene::Visual;

    fn actor(mode: Mode) -> Actor {
        let visual = if mode == Mode::ThreeD {
            Visual::Cuboid {
                color: "#fff".into(),
                size: [2.0, 2.0, 2.0],
            }
        } else {
            Visual::Rect {
                color: "#fff".into(),
                size: [40.0, 20.0],
            }
        };
        Actor::new("A", visual)
    }

    fn none(_: &str) -> Option<Transform> {
        None
    }

    fn with_collider(mut actor: Actor, spec: ColliderSpec) -> Actor {
        actor
            .components
            .insert(ActorComponent::Collider { collider: spec });
        actor
    }

    #[test]
    fn a_box_from_the_look_draws_its_twelve_edges_where_the_actor_stands() {
        let a = with_collider(actor(Mode::ThreeD), ColliderSpec::from_look());
        let pose = Transform::from_xyz(5.0, 0.0, 0.0);
        let lines = actor_lines(&a, &pose, Mode::ThreeD, &none);
        assert_eq!(lines.len(), 12);
        assert!(lines.iter().all(|l| l.color == COLLIDER));
        let xs: Vec<f32> = lines.iter().flat_map(|l| [l.a.x, l.b.x]).collect();
        assert_eq!(xs.iter().cloned().fold(f32::MAX, f32::min), 4.0);
        assert_eq!(xs.iter().cloned().fold(f32::MIN, f32::max), 6.0);
    }

    #[test]
    fn a_trigger_circle_in_2d_is_one_ring_in_the_plane() {
        let mut spec = ColliderSpec::from_look();
        spec.geometry = ColliderGeometry::Shape {
            shape: ColliderShape::Circle { radius: 10.0 },
        };
        spec.trigger = true;
        let a = with_collider(actor(Mode::TwoD), spec);
        let lines = actor_lines(&a, &Transform::IDENTITY, Mode::TwoD, &none);
        assert_eq!(lines.len(), SEGMENTS);
        assert!(lines.iter().all(|l| l.color == TRIGGER && l.a.z == 0.0));
        assert!(lines.iter().all(|l| (l.a.length() - 10.0).abs() < 1e-3));
    }

    #[test]
    fn a_joint_marks_its_anchor_and_reaches_for_its_target() {
        let mut a = actor(Mode::ThreeD);
        let mut spec = ConstraintSpec::of(ConstraintKind::Hinge, Mode::ThreeD);
        spec.target = "B".into();
        spec.auto_configure = false;
        spec.anchor = [1.0, 0.0, 0.0];
        a.components
            .insert(ActorComponent::Constraint { constraint: spec });
        let b = Transform::from_xyz(0.0, 3.0, 0.0);
        let find = |name: &str| (name == "B").then_some(b);
        let lines = actor_lines(&a, &Transform::IDENTITY, Mode::ThreeD, &find);
        // A three-line cross, the reach to B, and the hinge axis.
        assert_eq!(lines.len(), 5);
        assert!(
            lines
                .iter()
                .any(|l| l.a == Vec3::X && l.b == Vec3::new(0.0, 3.0, 0.0))
        );
    }

    #[test]
    fn nothing_invisible_draws_nothing() {
        let a = actor(Mode::ThreeD);
        assert!(actor_lines(&a, &Transform::IDENTITY, Mode::ThreeD, &none).is_empty());
    }
}
