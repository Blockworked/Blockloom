//! Geometric physics queries over the sensor snapshot.
//!
//! The sensing operators in [`crate::value`] are plain functions with no
//! physics world handle, so raycasts and overlap checks run here against the
//! shapes `publish_sensors` stored on each [`ActorSense`]: an axis-aligned
//! box, a ball or a set of boxes, in world units and the actor's own frame,
//! so a turned actor is hit where it is turned to.

use crate::sense::{ActorSense, ColliderShape, Sensors};

/// The nearest body a segment hits, excluding `skip` (usually the querier
/// itself). Only actors with a body whose layer `mask` names take part, so a
/// ray inherits whoever fired it's collision filter.
pub fn ray_hit(
    sensors: &Sensors,
    from: [f32; 3],
    to: [f32; 3],
    skip: Option<&str>,
    mask: u8,
) -> Option<(String, f32)> {
    let mut best: Option<(String, f32)> = None;
    for (id, actor) in &sensors.actors {
        if !actor.has_body {
            continue;
        }
        if skip.is_some_and(|skip| skip == id) {
            continue;
        }
        if mask & (1 << (actor.layer.clamp(1, 8) - 1)) == 0 {
            continue;
        }
        let Some(distance) = segment_hit(from, to, actor) else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, held)| distance < *held) {
            best = Some((id.clone(), distance));
        }
    }
    best
}

/// The actors whose shape contains `point`, nearest first. Bodies only, so a
/// decoration with no `Body` never answers.
pub fn overlap_point(sensors: &Sensors, point: [f32; 3], mask: u8) -> Vec<String> {
    let mut hits: Vec<(String, f32)> = Vec::new();
    for (id, actor) in &sensors.actors {
        if !actor.has_body {
            continue;
        }
        if mask & (1 << (actor.layer.clamp(1, 8) - 1)) == 0 {
            continue;
        }
        if contains(actor, point) {
            let dx = actor.position[0] - point[0];
            let dy = actor.position[1] - point[1];
            let dz = actor.position[2] - point[2];
            hits.push((id.clone(), dx * dx + dy * dy + dz * dz));
        }
    }
    hits.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    hits.into_iter().map(|(id, _)| id).collect()
}

/// The actors a ball at `center` with `radius` touches. A zero or negative
/// radius reads as a point.
pub fn overlap_circle(
    sensors: &Sensors,
    center: [f32; 3],
    radius: f32,
    skip: Option<&str>,
    mask: u8,
) -> Vec<String> {
    let mut hits: Vec<(String, f32)> = Vec::new();
    for (id, actor) in &sensors.actors {
        if !actor.has_body {
            continue;
        }
        if skip.is_some_and(|skip| skip == id) {
            continue;
        }
        if mask & (1 << (actor.layer.clamp(1, 8) - 1)) == 0 {
            continue;
        }
        if touches_ball(actor, center, radius.max(0.0)) {
            let dx = actor.position[0] - center[0];
            let dy = actor.position[1] - center[1];
            let dz = actor.position[2] - center[2];
            hits.push((id.clone(), dx * dx + dy * dy + dz * dz));
        }
    }
    hits.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    hits.into_iter().map(|(id, _)| id).collect()
}

/// The mask a query from `actor` fires with: its authored filter, or every
/// layer outside a run.
pub fn query_mask(sensors: &Sensors, actor: Option<&str>) -> u8 {
    let Some(id) = actor else {
        return 0xFF;
    };
    sensors.actors.get(id).map(|me| me.mask).unwrap_or(0xFF)
}

impl ActorSense {
    /// The mask a query this actor fires uses.
    pub fn query_mask(&self) -> u8 {
        self.mask
    }
}

/// A world point in the actor's frame: moved to its position, turned back
/// by its rotation. The shape's own extents already carry its scale.
fn to_local(actor: &ActorSense, point: [f32; 3]) -> [f32; 3] {
    let [rx, ry, rz] = actor.rotation.map(f32::to_radians);
    let turn = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
    let offset = glam::Vec3::from(point) - glam::Vec3::from(actor.position);
    (turn.inverse() * offset).to_array()
}

fn segment_hit(from: [f32; 3], to: [f32; 3], actor: &ActorSense) -> Option<f32> {
    // Turning preserves length, so distances in the actor's frame are world ones.
    let (from, to) = (to_local(actor, from), to_local(actor, to));
    match &actor.shape {
        ColliderShape::None => None,
        ColliderShape::Box { half } => segment_box(from, to, [0.0; 3], *half),
        ColliderShape::Ball { radius } => segment_ball(from, to, [0.0; 3], *radius),
        ColliderShape::Parts(parts) => parts
            .iter()
            .filter_map(|part| segment_box(from, to, part.offset, part.half))
            .min_by(f32::total_cmp),
    }
}

fn in_box(point: [f32; 3], center: [f32; 3], half: [f32; 3]) -> bool {
    (point[0] - center[0]).abs() <= half[0]
        && (point[1] - center[1]).abs() <= half[1]
        && (point[2] - center[2]).abs() <= half[2]
}

fn contains(actor: &ActorSense, point: [f32; 3]) -> bool {
    let point = to_local(actor, point);
    match &actor.shape {
        ColliderShape::None => false,
        ColliderShape::Box { half } => in_box(point, [0.0; 3], *half),
        ColliderShape::Parts(parts) => parts
            .iter()
            .any(|part| in_box(point, part.offset, part.half)),
        ColliderShape::Ball { radius } => {
            point[0] * point[0] + point[1] * point[1] + point[2] * point[2] <= radius * radius
        }
    }
}

fn box_touches_ball(at: [f32; 3], half: [f32; 3], center: [f32; 3], radius: f32) -> bool {
    // Distance from the ball's centre to the box, zero inside it.
    let out = |axis: usize| ((center[axis] - at[axis]).abs() - half[axis]).max(0.0);
    let (dx, dy, dz) = (out(0), out(1), out(2));
    dx * dx + dy * dy + dz * dz <= radius * radius
}

fn touches_ball(actor: &ActorSense, center: [f32; 3], radius: f32) -> bool {
    let center = to_local(actor, center);
    match &actor.shape {
        ColliderShape::None => false,
        ColliderShape::Box { half } => box_touches_ball([0.0; 3], *half, center, radius),
        ColliderShape::Parts(parts) => parts
            .iter()
            .any(|part| box_touches_ball(part.offset, part.half, center, radius)),
        ColliderShape::Ball { radius: other } => {
            let sum = radius + other;
            center[0] * center[0] + center[1] * center[1] + center[2] * center[2] <= sum * sum
        }
    }
}

// A slab test: the segment enters the box at `tmin` and leaves at `tmax`,
// and a hit is an entry in front of `from`. Distance is in segment units,
// so the caller scales by the segment's own length.
fn segment_box(from: [f32; 3], to: [f32; 3], center: [f32; 3], half: [f32; 3]) -> Option<f32> {
    let dir = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len <= 1e-9 {
        return contains(
            &ActorSense {
                position: center,
                shape: ColliderShape::Box { half },
                ..Default::default()
            },
            from,
        )
        .then_some(0.0);
    }
    let mut tmin: f32 = 0.0;
    let mut tmax: f32 = 1.0;
    for axis in 0..3 {
        let origin = from[axis] - center[axis];
        if dir[axis].abs() < 1e-9 {
            if origin.abs() > half[axis] {
                return None;
            }
        } else {
            let inv = 1.0 / dir[axis];
            let mut t0 = (-half[axis] - origin) * inv;
            let mut t1 = (half[axis] - origin) * inv;
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
            }
            tmin = tmin.max(t0);
            tmax = tmax.min(t1);
            if tmin > tmax {
                return None;
            }
        }
    }
    if !(0.0..=1.0).contains(&tmin) {
        return None;
    }
    Some(tmin * len)
}

fn segment_ball(from: [f32; 3], to: [f32; 3], center: [f32; 3], radius: f32) -> Option<f32> {
    let dir = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len2 = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    if len2 <= 1e-12 {
        return contains(
            &ActorSense {
                position: center,
                shape: ColliderShape::Ball { radius },
                ..Default::default()
            },
            from,
        )
        .then_some(0.0);
    }
    let oc = [
        from[0] - center[0],
        from[1] - center[1],
        from[2] - center[2],
    ];
    // |from + t*dir - center|^2 = r^2, solved for the nearest t in [0, 1].
    let a = len2;
    let b = 2.0 * (oc[0] * dir[0] + oc[1] * dir[1] + oc[2] * dir[2]);
    let c = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / (2.0 * a);
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(t * len2.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sense::ShapePart;
    use std::collections::HashSet;

    fn boxed(id: &str, at: [f32; 3], half: [f32; 3], layer: u8) -> (String, ActorSense) {
        (
            id.to_string(),
            ActorSense {
                name: id.to_string(),
                position: at,
                has_body: true,
                layer,
                shape: ColliderShape::Box { half },
                attached: HashSet::from(["Body".to_string()]),
                ..Default::default()
            },
        )
    }

    fn balled(id: &str, at: [f32; 3], radius: f32) -> (String, ActorSense) {
        (
            id.to_string(),
            ActorSense {
                name: id.to_string(),
                position: at,
                has_body: true,
                shape: ColliderShape::Ball { radius },
                ..Default::default()
            },
        )
    }

    fn world(pairs: Vec<(String, ActorSense)>) -> Sensors {
        Sensors {
            actors: pairs.into_iter().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_ray_hits_the_nearest_box_and_skips_the_querier() {
        let sensors = world(vec![
            boxed("me", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1),
            boxed("wall", [5.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1),
            boxed("far", [9.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1),
        ]);
        let hit = ray_hit(
            &sensors,
            [0.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            Some("me"),
            0xFF,
        );
        assert_eq!(
            hit.as_ref().map(|(id, _)| id).map(String::as_str),
            Some("wall")
        );
        let (id, distance) = hit.unwrap();
        assert_eq!(id, "wall");
        assert!((distance - 4.0).abs() < 1e-4);
    }

    #[test]
    fn a_ray_missing_everything_hits_nothing() {
        let sensors = world(vec![boxed("wall", [5.0, 5.0, 0.0], [1.0, 1.0, 1.0], 1)]);
        assert!(ray_hit(&sensors, [0.0, 0.0, 0.0], [20.0, 0.0, 0.0], None, 0xFF).is_none());
    }

    #[test]
    fn a_ray_respects_the_layer_mask() {
        let sensors = world(vec![boxed("ghost", [5.0, 0.0, 0.0], [1.0, 1.0, 1.0], 2)]);
        assert!(
            ray_hit(
                &sensors,
                [0.0, 0.0, 0.0],
                [20.0, 0.0, 0.0],
                None,
                0b0000_0001
            )
            .is_none()
        );
        assert!(
            ray_hit(
                &sensors,
                [0.0, 0.0, 0.0],
                [20.0, 0.0, 0.0],
                None,
                0b0000_0010
            )
            .is_some()
        );
    }

    #[test]
    fn a_ray_hits_balls_and_bodies_only() {
        let sensors = world(vec![
            balled("ball", [5.0, 0.0, 0.0], 1.0),
            boxed("deco", [3.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1),
        ]);
        let mut sensors = sensors;
        sensors.actors.get_mut("deco").unwrap().has_body = false;
        let hit = ray_hit(&sensors, [0.0, 0.0, 0.0], [20.0, 0.0, 0.0], None, 0xFF);
        assert_eq!(
            hit.as_ref().map(|(id, _)| id).map(String::as_str),
            Some("ball")
        );
    }

    #[test]
    fn an_overlap_circle_finds_boxes_and_balls_nearest_first() {
        let sensors = world(vec![
            boxed("near", [1.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1),
            balled("far", [1.5, 0.0, 0.0], 0.5),
        ]);
        let hits = overlap_circle(&sensors, [0.0, 0.0, 0.0], 2.0, None, 0xFF);
        assert_eq!(hits, vec!["near".to_string(), "far".to_string()]);
        assert!(overlap_circle(&sensors, [50.0, 50.0, 0.0], 1.0, None, 0xFF).is_empty());
    }

    #[test]
    fn a_tilemap_is_queried_cell_by_cell() {
        let parts: std::sync::Arc<[ShapePart]> = vec![
            ShapePart {
                offset: [-40.0, 0.0, 0.0],
                half: [10.0, 10.0, 0.0],
            },
            ShapePart {
                offset: [40.0, 0.0, 0.0],
                half: [10.0, 10.0, 0.0],
            },
        ]
        .into();
        let sensors = world(vec![(
            "map".to_string(),
            ActorSense {
                name: "map".into(),
                has_body: true,
                shape: ColliderShape::Parts(parts),
                ..Default::default()
            },
        )]);
        // The gap between the two solid runs is empty.
        assert!(overlap_point(&sensors, [0.0, 0.0, 0.0], 0xFF).is_empty());
        assert_eq!(overlap_point(&sensors, [42.0, 3.0, 0.0], 0xFF), vec!["map"]);
        // A ray down the gap misses; one across hits the near run's face.
        assert!(ray_hit(&sensors, [0.0, 50.0, 0.0], [0.0, -50.0, 0.0], None, 0xFF).is_none());
        let hit = ray_hit(&sensors, [0.0, 0.0, 0.0], [100.0, 0.0, 0.0], None, 0xFF);
        assert!(hit.is_some_and(|(_, d)| (d - 30.0).abs() < 1e-3));
        assert!(overlap_circle(&sensors, [0.0, 0.0, 0.0], 5.0, None, 0xFF).is_empty());
        assert_eq!(
            overlap_circle(&sensors, [0.0, 0.0, 0.0], 35.0, None, 0xFF).len(),
            1
        );
    }

    #[test]
    fn a_small_ball_inside_a_big_box_touches_it() {
        let sensors = world(vec![boxed("hall", [0.0; 3], [100.0, 100.0, 100.0], 1)]);
        assert_eq!(
            overlap_circle(&sensors, [10.0, 0.0, 0.0], 1.0, None, 0xFF).len(),
            1
        );
    }

    #[test]
    fn a_turned_box_is_hit_where_it_is_turned_to() {
        let (id, mut plank) = boxed("plank", [0.0; 3], [10.0, 1.0, 1.0], 1);
        // A quarter turn about z stands the plank upright.
        plank.rotation = [0.0, 0.0, 90.0];
        let sensors = world(vec![(id, plank)]);
        assert!(overlap_point(&sensors, [8.0, 0.0, 0.0], 0xFF).is_empty());
        assert_eq!(
            overlap_point(&sensors, [0.0, 8.0, 0.0], 0xFF),
            vec!["plank"]
        );
        let hit = ray_hit(&sensors, [0.0, 20.0, 0.0], [0.0, -20.0, 0.0], None, 0xFF);
        assert!(hit.is_some_and(|(_, d)| (d - 10.0).abs() < 1e-3));
        assert!(overlap_circle(&sensors, [5.0, 0.0, 0.0], 3.0, None, 0xFF).is_empty());
    }
}
