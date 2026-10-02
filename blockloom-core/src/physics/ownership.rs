//! Who owns which shape: the `PhysicsOwnership` table.
//!
//! A collider belongs to its own actor's Rigidbody, or the nearest ancestor's
//! (walking `Parent` links). A nested Rigidbody starts a separate body, and a
//! collider with no body above it is static. The table is derived from the
//! document in one pass and never from renderer parentage; the runtime reads it
//! instead of re-inferring ownership per system.
//!
//! The pose of a collider actor relative to its body actor comes from the
//! world placements the runtime builds from (`Place`, with a `Parent` offset
//! resolved against its parent), so both agree on where a shape stands.

use std::collections::{HashMap, HashSet};

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::Serialize;

use super::ids::{ColliderId, ComponentId};
use crate::project::Actor;
use crate::scene::Placement;

/// A pose relative to another actor: position, rotation as a quaternion
/// (x, y, z, w) and the scale carried between the two.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LocalPose {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl LocalPose {
    pub const IDENTITY: LocalPose = LocalPose {
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };

    fn of(matrix: Mat4) -> Self {
        let (scale, rotation, position) = matrix.to_scale_rotation_translation();
        Self {
            position: position.to_array(),
            rotation: rotation.to_array(),
            scale: scale.to_array(),
        }
    }
}

/// One collider and the body that carries it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColliderOwnership {
    pub collider: ColliderId,
    pub collider_actor: String,
    /// The actor whose Rigidbody carries this shape; `None` for static geometry.
    pub body_actor: Option<String>,
    pub body: Option<ComponentId>,
    /// The collider actor's pose in the body actor's frame (identity when they
    /// are the same actor or the shape is static).
    pub local_pose: LocalPose,
    pub enabled: bool,
    pub trigger: bool,
    /// True when the shape follows the actor's Look rather than a saved shape.
    pub derived: bool,
}

/// A Rigidbody and the shapes on it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BodyEntry {
    pub actor: String,
    pub body: ComponentId,
    pub colliders: Vec<ColliderId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct PhysicsOwnership {
    pub colliders: Vec<ColliderOwnership>,
    pub bodies: Vec<BodyEntry>,
}

impl PhysicsOwnership {
    /// Resolves ownership for one scene's actors.
    pub fn resolve(actors: &[Actor]) -> Self {
        let by_id: HashMap<&str, &Actor> = actors.iter().map(|a| (a.id.as_str(), a)).collect();
        let worlds = world_matrices(actors, &by_id);
        let mut table = PhysicsOwnership::default();

        for actor in actors {
            if let Some(body) = actor.components.rigidbody() {
                table.bodies.push(BodyEntry {
                    actor: actor.id.clone(),
                    body: body.id.clone(),
                    colliders: Vec::new(),
                });
            }
        }

        for actor in actors {
            for collider in actor.components.colliders() {
                let carrier = body_above(actor, &by_id);
                let local_pose = match carrier {
                    Some(body_actor) if body_actor.id != actor.id => {
                        match (worlds.get(&body_actor.id), worlds.get(&actor.id)) {
                            (Some(body), Some(mine)) => LocalPose::of(body.inverse() * *mine),
                            _ => LocalPose::IDENTITY,
                        }
                    }
                    _ => LocalPose::IDENTITY,
                };
                if let Some(body_actor) = carrier
                    && let Some(entry) = table.bodies.iter_mut().find(|b| b.actor == body_actor.id)
                {
                    entry.colliders.push(collider.id.clone());
                }
                table.colliders.push(ColliderOwnership {
                    collider: collider.id.clone(),
                    collider_actor: actor.id.clone(),
                    body_actor: carrier.map(|a| a.id.clone()),
                    body: carrier
                        .and_then(|a| a.components.rigidbody())
                        .map(|b| b.id.clone()),
                    local_pose,
                    enabled: collider.enabled,
                    trigger: collider.trigger,
                    derived: collider.shape().is_none(),
                });
            }
        }
        table
    }

    pub fn collider(&self, id: &ColliderId) -> Option<&ColliderOwnership> {
        self.colliders.iter().find(|c| &c.collider == id)
    }

    pub fn body(&self, actor: &str) -> Option<&BodyEntry> {
        self.bodies.iter().find(|b| b.actor == actor)
    }

    /// Colliders no Rigidbody carries: scenery.
    pub fn static_colliders(&self) -> impl Iterator<Item = &ColliderOwnership> {
        self.colliders.iter().filter(|c| c.body_actor.is_none())
    }

    /// Bodies with no shape at all. They still integrate gravity and forces but
    /// have no contact surface.
    pub fn bodies_without_shape(&self) -> impl Iterator<Item = &BodyEntry> {
        self.bodies.iter().filter(|b| b.colliders.is_empty())
    }
}

/// The actor whose Rigidbody carries a collider on `actor`: itself, else the
/// nearest ancestor with one. Parent loops end the walk.
pub(super) fn body_above<'a>(actor: &'a Actor, by_id: &HashMap<&str, &'a Actor>) -> Option<&'a Actor> {
    let mut at = actor;
    let mut seen = HashSet::new();
    loop {
        if at.components.rigidbody().is_some() {
            return Some(at);
        }
        if !seen.insert(at.id.as_str()) {
            return None;
        }
        at = by_id.get(at.parent()?).copied()?;
    }
}

pub(super) fn actor_worlds(actors: &[Actor]) -> HashMap<String, Mat4> {
    let by_id: HashMap<&str, &Actor> = actors.iter().map(|a| (a.id.as_str(), a)).collect();
    world_matrices(actors, &by_id)
}

fn matrix_of(placement: &Placement) -> Mat4 {
    let [rx, ry, rz] = placement.rotation;
    Mat4::from_scale_rotation_translation(
        Vec3::from(placement.scale3()),
        Quat::from_euler(
            EulerRot::XYZ,
            rx.to_radians(),
            ry.to_radians(),
            rz.to_radians(),
        ),
        Vec3::from(placement.position),
    )
}

/// Every actor's world matrix as the runtime builds it: its own `Place`, except
/// that a child with an authored `Parent` offset stands at its parent's matrix
/// times the offset (parents first, so an offset down a chain is measured
/// against a parent that has already moved).
fn world_matrices(actors: &[Actor], by_id: &HashMap<&str, &Actor>) -> HashMap<String, Mat4> {
    let mut worlds: HashMap<String, Mat4> = actors
        .iter()
        .map(|a| (a.id.clone(), matrix_of(&a.components.placement())))
        .collect();
    let depth = |id: &str| {
        let mut at = id;
        for depth in 0..actors.len() {
            match by_id.get(at).and_then(|a| a.parent()) {
                Some(parent) if by_id.contains_key(parent) => at = parent,
                _ => return depth,
            }
        }
        actors.len()
    };
    let mut order: Vec<&Actor> = actors
        .iter()
        .filter(|a| a.parent_offset().is_some())
        .collect();
    order.sort_by_key(|a| depth(&a.id));
    for child in order {
        let (Some(parent), Some(offset)) = (child.parent(), child.parent_offset()) else {
            continue;
        };
        let Some(parent_matrix) = worlds.get(parent).copied() else {
            continue;
        };
        if let Some(mine) = worlds.get_mut(&child.id) {
            let (scale, rotation, _) = mine.to_scale_rotation_translation();
            let at = parent_matrix.transform_point3(Vec3::from(offset));
            *mine = Mat4::from_scale_rotation_translation(scale, rotation, at);
        }
    }
    worlds
}

#[cfg(test)]
mod tests {
    use super::super::spec::{ColliderShape, ColliderSpec, RigidbodySpec};
    use super::*;
    use crate::components::ActorComponent;
    use crate::scene::Visual;

    fn actor(id: &str) -> Actor {
        let mut actor = Actor::new(
            id,
            Visual::Cuboid {
                color: "#fff".into(),
                size: [1.0; 3],
            },
        );
        actor.id = id.to_string();
        actor
    }

    fn with_box(mut actor: Actor, id: &str) -> Actor {
        let mut spec = ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] });
        spec.id = id.into();
        actor
            .components
            .insert(ActorComponent::Collider { collider: spec });
        actor
    }

    fn with_body(mut actor: Actor, id: &str) -> Actor {
        let spec = RigidbodySpec {
            id: id.into(),
            ..Default::default()
        };
        actor
            .components
            .insert(ActorComponent::Rigidbody { rigidbody: spec });
        actor
    }

    fn place(mut actor: Actor, position: [f32; 3]) -> Actor {
        actor.components.placement_mut().position = position;
        actor
    }

    fn child_of(mut actor: Actor, parent: &str) -> Actor {
        actor.components.set_parent(parent);
        actor
    }

    #[test]
    fn a_collider_with_no_body_is_static_and_needs_no_look() {
        let mut ghost = with_box(actor("ghost"), "c1");
        ghost.components.remove("Look");
        ghost.components.remove("Render");
        let table = PhysicsOwnership::resolve(&[ghost]);
        assert_eq!(table.static_colliders().count(), 1);
        assert_eq!(table.colliders[0].body_actor, None);
        assert!(!table.colliders[0].derived);
    }

    #[test]
    fn a_body_without_a_shape_is_reported() {
        let table = PhysicsOwnership::resolve(&[with_body(actor("ball"), "b1")]);
        assert_eq!(table.bodies_without_shape().count(), 1);
        assert!(table.colliders.is_empty());
    }

    #[test]
    fn child_shapes_join_the_nearest_ancestor_body() {
        let car = place(with_body(actor("car"), "b1"), [10.0, 0.0, 0.0]);
        let wheel = child_of(
            place(with_box(actor("wheel"), "w"), [11.0, 0.0, 2.0]),
            "car",
        );
        let hub = child_of(
            place(with_box(actor("hub"), "h"), [11.0, 1.0, 2.0]),
            "wheel",
        );
        let table = PhysicsOwnership::resolve(&[car, wheel, hub]);
        let wheel = table.collider(&"w".into()).unwrap();
        assert_eq!(wheel.body_actor.as_deref(), Some("car"));
        assert_eq!(wheel.body, Some("b1".into()));
        assert_eq!(wheel.local_pose.position, [1.0, 0.0, 2.0]);
        assert_eq!(
            table.collider(&"h".into()).unwrap().body_actor.as_deref(),
            Some("car")
        );
        assert_eq!(table.body("car").unwrap().colliders.len(), 2);
    }

    #[test]
    fn a_nested_rigidbody_starts_a_separate_body() {
        let car = with_body(actor("car"), "b1");
        let rider = child_of(with_box(with_body(actor("rider"), "b2"), "r"), "car");
        let table = PhysicsOwnership::resolve(&[car, rider]);
        assert_eq!(
            table.collider(&"r".into()).unwrap().body_actor.as_deref(),
            Some("rider")
        );
        assert_eq!(table.body("car").unwrap().colliders.len(), 0);
    }

    #[test]
    fn a_collider_under_a_parent_without_a_body_stays_static() {
        let platform = actor("platform");
        let rail = child_of(with_box(actor("rail"), "c"), "platform");
        let table = PhysicsOwnership::resolve(&[platform, rail]);
        assert_eq!(table.static_colliders().count(), 1);
    }

    #[test]
    fn parent_offsets_place_the_child_in_the_parents_frame() {
        let mut car = with_body(actor("car"), "b1");
        car.components.placement_mut().position = [5.0, 0.0, 0.0];
        car.components.placement_mut().rotation = [0.0, 90.0, 0.0];
        let mut wheel = with_box(actor("wheel"), "w");
        wheel.components.insert(ActorComponent::Parent {
            parent: "car".into(),
            offset: Some([0.0, 0.0, -1.0]),
        });
        let table = PhysicsOwnership::resolve(&[car, wheel]);
        let pose = table.collider(&"w".into()).unwrap().local_pose;
        // The offset is in the car's own frame, so the body-relative position is the offset.
        for (got, want) in pose.position.iter().zip([0.0, 0.0, -1.0]) {
            assert!((got - want).abs() < 1e-4, "{:?}", pose.position);
        }
    }

    #[test]
    fn parent_loops_do_not_hang() {
        let a = child_of(with_box(actor("a"), "ca"), "b");
        let b = child_of(with_box(actor("b"), "cb"), "a");
        let table = PhysicsOwnership::resolve(&[a, b]);
        assert_eq!(table.static_colliders().count(), 2);
    }
}
