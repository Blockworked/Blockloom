//! Geometric physics queries over the sensor snapshot.
//!
//! The sensing operators in [`crate::value`] are plain functions with no
//! physics world handle, so raycasts and overlap checks run here against the
//! shapes `publish_sensors` stored on each [`ActorSense`]: an axis-aligned
//! box or a ball, in world units. Rotation is ignored, which is what a
//! platformer's ground check wants.

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

/// Where a segment first enters any body's shape, and the outward normal
/// there, skipping `skip`. A segment that starts inside a shape doesn't hit
/// it, so what was born inside a body can leave. The CPU particle pool's
/// collisions.
pub fn segment_contact(
    sensors: &Sensors,
    from: [f32; 3],
    to: [f32; 3],
    skip: Option<&str>,
) -> Option<([f32; 3], [f32; 3])> {
    let mut best: Option<(f32, [f32; 3])> = None;
    for (id, actor) in &sensors.actors {
        if !actor.has_body || skip.is_some_and(|skip| skip == id) {
            continue;
        }
        let hit = match actor.shape {
            ColliderShape::None => None,
            ColliderShape::Box { half } => enter_box(from, to, actor.position, half),
            ColliderShape::Ball { radius } => enter_ball(from, to, actor.position, radius),
        };
        if let Some((t, normal)) = hit
            && best.is_none_or(|(held, _)| t < held)
        {
            best = Some((t, normal));
        }
    }
    best.map(|(t, normal)| {
        (
            std::array::from_fn(|i| from[i] + (to[i] - from[i]) * t),
            normal,
        )
    })
}

/// The segment fraction where it enters a box from outside, and that face's
/// normal.
fn enter_box(
    from: [f32; 3],
    to: [f32; 3],
    center: [f32; 3],
    half: [f32; 3],
) -> Option<(f32, [f32; 3])> {
    let mut tmin: f32 = 0.0;
    let mut tmax: f32 = 1.0;
    let mut normal = [0.0; 3];
    let mut outside = false;
    for axis in 0..3 {
        let origin = from[axis] - center[axis];
        let dir = to[axis] - from[axis];
        if origin.abs() > half[axis] {
            outside = true;
        }
        if dir.abs() < 1e-9 {
            if origin.abs() > half[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir;
        let mut t0 = (-half[axis] - origin) * inv;
        let mut t1 = (half[axis] - origin) * inv;
        let mut sign = -1.0;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
            sign = 1.0;
        }
        if t0 > tmin {
            tmin = t0;
            normal = [0.0; 3];
            normal[axis] = sign;
        }
        tmax = tmax.min(t1);
        if tmin > tmax {
            return None;
        }
    }
    (outside && normal != [0.0; 3]).then_some((tmin, normal))
}

fn enter_ball(
    from: [f32; 3],
    to: [f32; 3],
    center: [f32; 3],
    radius: f32,
) -> Option<(f32, [f32; 3])> {
    let dir: [f32; 3] = std::array::from_fn(|i| to[i] - from[i]);
    let oc: [f32; 3] = std::array::from_fn(|i| from[i] - center[i]);
    let a = dir.iter().map(|d| d * d).sum::<f32>();
    let c = oc.iter().map(|d| d * d).sum::<f32>() - radius * radius;
    if a <= 1e-12 || c <= 0.0 {
        return None;
    }
    let b = 2.0 * (0..3).map(|i| oc[i] * dir[i]).sum::<f32>();
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / (2.0 * a);
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let at: [f32; 3] = std::array::from_fn(|i| oc[i] + dir[i] * t);
    let len = at.iter().map(|d| d * d).sum::<f32>().sqrt().max(1e-9);
    Some((t, at.map(|v| v / len)))
}

fn segment_hit(from: [f32; 3], to: [f32; 3], actor: &ActorSense) -> Option<f32> {
    match actor.shape {
        ColliderShape::None => None,
        ColliderShape::Box { half } => segment_box(from, to, actor.position, half),
        ColliderShape::Ball { radius } => segment_ball(from, to, actor.position, radius),
    }
}

fn contains(actor: &ActorSense, point: [f32; 3]) -> bool {
    match actor.shape {
        ColliderShape::None => false,
        ColliderShape::Box { half } => {
            (point[0] - actor.position[0]).abs() <= half[0]
                && (point[1] - actor.position[1]).abs() <= half[1]
                && (point[2] - actor.position[2]).abs() <= half[2]
        }
        ColliderShape::Ball { radius } => {
            let dx = point[0] - actor.position[0];
            let dy = point[1] - actor.position[1];
            let dz = point[2] - actor.position[2];
            dx * dx + dy * dy + dz * dz <= radius * radius
        }
    }
}

fn touches_ball(actor: &ActorSense, center: [f32; 3], radius: f32) -> bool {
    match actor.shape {
        ColliderShape::None => false,
        ColliderShape::Box { half } => {
            let dx = (center[0] - actor.position[0]).abs().max(0.0) - half[0];
            let dy = (center[1] - actor.position[1]).abs().max(0.0) - half[1];
            let dz = (center[2] - actor.position[2]).abs().max(0.0) - half[2];
            let outside = [dx.max(0.0), dy.max(0.0), dz.max(0.0)];
            let inside = dx.min(dy.min(dz)).min(0.0);
            outside[0] * outside[0]
                + outside[1] * outside[1]
                + outside[2] * outside[2]
                + inside * inside
                <= radius * radius
        }
        ColliderShape::Ball { radius: other } => {
            let dx = center[0] - actor.position[0];
            let dy = center[1] - actor.position[1];
            let dz = center[2] - actor.position[2];
            let sum = radius + other;
            dx * dx + dy * dy + dz * dz <= sum * sum
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

    #[test]
    fn a_segment_into_a_box_reports_the_face_it_crossed() {
        let (point, normal) = enter_box([0.0, 5.0, 0.0], [0.0, -5.0, 0.0], [0.0; 3], [1.0; 3])
            .map(|(t, n)| ([0.0, 5.0 - 10.0 * t, 0.0], n))
            .unwrap();
        assert!((point[1] - 1.0).abs() < 1e-5);
        assert_eq!(normal, [0.0, 1.0, 0.0]);
        // Leaving from inside isn't a hit.
        assert!(enter_box([0.0; 3], [0.0, 5.0, 0.0], [0.0; 3], [1.0; 3]).is_none());
        let (_, normal) = enter_ball([5.0, 0.0, 0.0], [0.0; 3], [0.0; 3], 1.0).unwrap();
        assert!((normal[0] - 1.0).abs() < 1e-5);
    }
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
}
