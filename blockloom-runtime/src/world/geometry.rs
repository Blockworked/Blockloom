//! Actor extents and collider shapes for picking and queries.

use super::*;

pub(crate) fn screen_rect(
    laid_out: &Query<(&ComputedNode, &UiGlobalTransform)>,
    entity: Entity,
) -> Option<Rect> {
    let (node, transform) = laid_out.get(entity).ok()?;
    let size = node.size();
    if size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let centre = transform.translation;
    let size = Vec2::new(
        transform.matrix2.x_axis.x.abs() * size.x + transform.matrix2.y_axis.x.abs() * size.y,
        transform.matrix2.x_axis.y.abs() * size.x + transform.matrix2.y_axis.y.abs() * size.y,
    );
    Some(Rect::from_center_size(centre, size))
}

pub(crate) fn half_extents(visual: &Visual) -> Vec2 {
    match visual {
        Visual::Rect { size, .. } | Visual::Image { size, .. } => {
            Vec2::new(size[0] / 2.0, size[1] / 2.0)
        }
        Visual::Circle { radius, .. } => Vec2::splat(*radius),
        Visual::Tilemap { tilemap } => {
            let size = tilemap.size();
            Vec2::new(size[0] / 2.0, size[1] / 2.0)
        }
        _ => Vec2::ZERO,
    }
}

pub(crate) fn half_extents3(visual: &Visual) -> Vec3 {
    match visual {
        Visual::Cuboid { size, .. } => Vec3::new(size[0] / 2.0, size[1] / 2.0, size[2] / 2.0),
        Visual::Sphere { radius, .. } => Vec3::splat(*radius),
        Visual::Capsule { radius, height, .. } => {
            Vec3::new(*radius, height / 2.0 + radius, *radius)
        }
        Visual::Plane { size, .. } => Vec3::new(size[0] / 2.0, 0.1, size[1] / 2.0),
        // A model's extents are its authored scale until the glTF scene
        // loads and reports its own bounds.
        Visual::Model { scale, .. } => Vec3::new(scale[0] / 2.0, scale[1] / 2.0, scale[2] / 2.0),
        // A wall map's face: width, height, no depth.
        Visual::Tilemap { tilemap } => {
            let size = tilemap.size();
            Vec3::new(size[0] / 2.0, size[1] / 2.0, 0.1)
        }
        _ => Vec3::ZERO,
    }
}

/// What a physics query sees: the actor's collider in world units, or
/// nothing for an actor with no body. Scale is folded in; the query turns it
/// by the actor's rotation.
pub(crate) fn collider_shape(
    engine: &Engine,
    id: &str,
    mode: Mode,
    transform: &Transform,
) -> blockloom_core::sense::ColliderShape {
    use blockloom_core::sense::ColliderShape;
    if !engine.has_component(id, "Body") {
        return ColliderShape::None;
    }
    let Some(actor) = engine.actor(id) else {
        return ColliderShape::None;
    };
    let Some(visual) = actor.visual() else {
        return ColliderShape::None;
    };
    let scale = transform.scale.abs();
    match (visual, mode) {
        (Visual::Circle { radius, .. }, Mode::TwoD) => {
            let radius = radius * scale.max_element().max(0.0);
            (radius > 0.0).then_some(ColliderShape::Ball { radius })
        }
        (Visual::Sphere { radius, .. }, Mode::ThreeD) => {
            let radius = radius * scale.max_element().max(0.0);
            (radius > 0.0).then_some(ColliderShape::Ball { radius })
        }
        (Visual::Rect { size, .. } | Visual::Image { size, .. }, Mode::TwoD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                size[1] / 2.0 * scale.y.max(0.0),
                0.0,
            ];
            (half[0] > 0.0 && half[1] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Cuboid { size, .. }, Mode::ThreeD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                size[1] / 2.0 * scale.y.max(0.0),
                size[2] / 2.0 * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[1] > 0.0 && half[2] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Capsule { radius, height, .. }, Mode::ThreeD) => {
            let half = [
                radius * scale.x.max(0.0),
                (height / 2.0 + radius) * scale.y.max(0.0),
                radius * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[1] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Plane { size, .. }, Mode::ThreeD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                0.1,
                size[1] / 2.0 * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[2] > 0.0).then_some(ColliderShape::Box { half })
        }
        (Visual::Model { scale: size, .. }, Mode::ThreeD) => {
            let half = [
                size[0] / 2.0 * scale.x.max(0.0),
                size[1] / 2.0 * scale.y.max(0.0),
                size[2] / 2.0 * scale.z.max(0.0),
            ];
            (half[0] > 0.0 && half[1] > 0.0 && half[2] > 0.0).then_some(ColliderShape::Box { half })
        }
        // `tiles::publish_level` gives a solid map its cells.
        (Visual::Tilemap { .. }, _) => None,
        _ => None,
    }
    .unwrap_or(ColliderShape::None)
}

/// The world-space point a speech bubble should sit over. The bubble itself is
/// screen-space UI, but measuring the visual here keeps it attached to the top
/// of both 2D and 3D actors as they move and scale.
pub fn actor_top(actor: &Actor, transform: &Transform, mode: Mode) -> Vec3 {
    let half_height = match (actor.visual(), mode) {
        (Some(visual), Mode::TwoD) => half_extents(visual).y,
        (Some(visual), Mode::ThreeD) => half_extents3(visual).y,
        (None, _) => 0.0,
    } * transform.scale.y.abs();
    transform.translation + Vec3::Y * half_height
}

// ─── Running blocks ────────────────────────────────────────────────────────
