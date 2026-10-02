//! Editing transactions.
//!
//! Every change goes through one of these: apply to the actor, validate the
//! scene's physics, and either keep the result or put the actor's components back
//! exactly as they were. A refused edit leaves no trace, so a caller can push its
//! undo step only after success.

use std::collections::HashSet;

use super::ids::{ColliderId, ComponentId};
use super::material::MaterialLibrary;
use super::migrate::shape_from_look;
use super::spec::{ColliderGeometry, ColliderSpec, RigidbodySpec};
use super::validate::{PhysicsIssue, validate_scene};
use crate::components::{ActorComponent, Components};
use crate::project::{Actor, Scene};

fn describe(issues: &[PhysicsIssue]) -> String {
    issues
        .iter()
        .map(|issue| issue.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

impl Scene {
    /// Every physics problem in this scene.
    pub fn physics_issues(&self, library: &MaterialLibrary) -> Vec<PhysicsIssue> {
        validate_scene(&self.actors, self.world.mode, library)
    }

    /// The actor that carries collider `id`.
    pub fn collider_actor(&self, id: &ColliderId) -> Option<&Actor> {
        self.actors
            .iter()
            .find(|a| a.components.collider(id).is_some())
    }

    fn collider_in_use(&self, id: &ColliderId) -> bool {
        self.collider_actor(id).is_some()
    }

    /// Runs `change` on `actor`'s components and keeps it only when the scene's
    /// physics has no new error about this actor or the ids it touched.
    fn transact(
        &mut self,
        actor_id: &str,
        touched: &[String],
        library: &MaterialLibrary,
        change: impl FnOnce(&mut Components) -> Result<(), String>,
    ) -> Result<(), String> {
        let Some(index) = self.actors.iter().position(|a| a.id == actor_id) else {
            return Err("Actor not found".to_string());
        };
        let before = self.actors[index].components.clone();
        if let Err(message) = change(&mut self.actors[index].components) {
            self.actors[index].components = before;
            return Err(message);
        }
        let errors: Vec<PhysicsIssue> = self
            .physics_issues(library)
            .into_iter()
            .filter(|issue| {
                issue.is_error()
                    && (issue.actor.as_deref() == Some(actor_id)
                        || issue
                            .component
                            .as_ref()
                            .is_some_and(|c| touched.iter().any(|t| t == c)))
            })
            .collect();
        if errors.is_empty() {
            return Ok(());
        }
        self.actors[index].components = before;
        Err(describe(&errors))
    }

    /// Adds a collider to `actor_id`. An empty id is replaced with a fresh one; an
    /// id already used in the scene is refused.
    pub fn add_collider(
        &mut self,
        actor_id: &str,
        mut spec: ColliderSpec,
        library: &MaterialLibrary,
    ) -> Result<ColliderId, String> {
        if spec.id.is_empty() {
            spec.id = ColliderId::generate();
        }
        if self.collider_in_use(&spec.id) {
            return Err(format!("Collider id \"{}\" is already in use", spec.id));
        }
        let id = spec.id.clone();
        let touched = vec![id.to_string()];
        self.transact(actor_id, &touched, library, |components| {
            components.insert(ActorComponent::Collider { collider: spec });
            Ok(())
        })?;
        Ok(id)
    }

    /// Replaces the collider whose id is `spec.id`, in place. The id cannot change.
    pub fn set_collider(
        &mut self,
        spec: ColliderSpec,
        library: &MaterialLibrary,
    ) -> Result<(), String> {
        let Some(actor) = self.collider_actor(&spec.id) else {
            return Err(format!("No collider with id \"{}\"", spec.id));
        };
        let actor_id = actor.id.clone();
        let touched = vec![spec.id.to_string()];
        self.transact(&actor_id, &touched, library, |components| {
            components.insert(ActorComponent::Collider { collider: spec });
            Ok(())
        })
    }

    /// Removes a collider; returns the actor it was on.
    pub fn remove_collider(
        &mut self,
        id: &ColliderId,
        library: &MaterialLibrary,
    ) -> Result<String, String> {
        let Some(actor) = self.collider_actor(id) else {
            return Err(format!("No collider with id \"{id}\""));
        };
        let actor_id = actor.id.clone();
        let touched = vec![id.to_string()];
        self.transact(&actor_id, &touched, library, |components| {
            components.remove_collider(id);
            Ok(())
        })?;
        Ok(actor_id)
    }

    /// Gives the actor a Rigidbody, or replaces the one it has (keeping its id).
    pub fn set_rigidbody(
        &mut self,
        actor_id: &str,
        mut spec: RigidbodySpec,
        library: &MaterialLibrary,
    ) -> Result<ComponentId, String> {
        if spec.id.is_empty() {
            spec.id = ComponentId::generate();
        }
        // Editing a body keeps its identity whatever id the caller sent.
        if let Some(existing) = self
            .actors
            .iter()
            .find(|a| a.id == actor_id)
            .and_then(|a| a.components.rigidbody())
        {
            spec.id = existing.id.clone();
        }
        let id = spec.id.clone();
        let touched = vec![id.to_string()];
        self.transact(actor_id, &touched, library, |components| {
            match components
                .0
                .iter_mut()
                .find(|c| matches!(c, ActorComponent::Rigidbody { .. }))
            {
                Some(slot) => *slot = ActorComponent::Rigidbody { rigidbody: spec },
                None => components
                    .0
                    .push(ActorComponent::Rigidbody { rigidbody: spec }),
            }
            Ok(())
        })?;
        Ok(id)
    }

    /// Takes the Rigidbody off. Its colliders stay and become static scenery (or
    /// join an ancestor's body).
    pub fn remove_rigidbody(
        &mut self,
        actor_id: &str,
        library: &MaterialLibrary,
    ) -> Result<(), String> {
        self.transact(actor_id, &[], library, |components| {
            let Some(index) = components
                .0
                .iter()
                .position(|c| matches!(c, ActorComponent::Rigidbody { .. }))
            else {
                return Err("This actor has no Rigidbody".to_string());
            };
            components.0.remove(index);
            Ok(())
        })
    }

    /// Sets a collider's saved shape from its actor's Look: the explicit
    /// "Fit to visual" and "Make independent" command. A shape that followed the
    /// Look stops following it.
    pub fn fit_collider_to_look(
        &mut self,
        id: &ColliderId,
        library: &MaterialLibrary,
    ) -> Result<(), String> {
        let mode = self.world.mode;
        let Some(actor) = self.collider_actor(id) else {
            return Err(format!("No collider with id \"{id}\""));
        };
        let shape = actor
            .components
            .visual()
            .and_then(|visual| shape_from_look(visual, mode))
            .ok_or("This actor's Look has no collision shape to fit")?;
        let mut spec = actor
            .components
            .collider(id)
            .cloned()
            .expect("found its actor");
        spec.geometry = ColliderGeometry::Shape { shape };
        self.set_collider(spec, library)
    }

    /// Gives every empty or repeated physics id a fresh one (a hand-edited file can
    /// carry either). Returns how many changed; a clean scene changes nothing.
    pub fn normalize_physics_ids(&mut self) -> usize {
        let mut seen_colliders = HashSet::new();
        let mut seen_bodies = HashSet::new();
        let mut changed = 0;
        for actor in &mut self.actors {
            for component in actor.components.iter_mut() {
                match component {
                    ActorComponent::Collider { collider } => {
                        if collider.id.is_empty() || !seen_colliders.insert(collider.id.clone()) {
                            collider.id = loop {
                                let fresh = ColliderId::generate();
                                if seen_colliders.insert(fresh.clone()) {
                                    break fresh;
                                }
                            };
                            changed += 1;
                        }
                    }
                    ActorComponent::Rigidbody { rigidbody }
                        if (rigidbody.id.is_empty()
                            || !seen_bodies.insert(rigidbody.id.clone())) =>
                    {
                        rigidbody.id = loop {
                            let fresh = ComponentId::generate();
                            if seen_bodies.insert(fresh.clone()) {
                                break fresh;
                            }
                        };
                        changed += 1;
                    }
                    _ => {}
                }
            }
        }
        changed
    }
}

impl Actor {
    /// Gives this actor's physics components fresh ids, for a copy that must not
    /// share them with the original. Everything else about the specs is kept, and
    /// references to assets (materials, meshes) stay as they are.
    pub fn refresh_physics_ids(&mut self) {
        for component in self.components.iter_mut() {
            match component {
                ActorComponent::Collider { collider } => collider.id = ColliderId::generate(),
                ActorComponent::Rigidbody { rigidbody } => rigidbody.id = ComponentId::generate(),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::spec::{ColliderShape, MassSource};
    use super::*;
    use crate::scene::{Mode, Visual};

    fn lib() -> MaterialLibrary {
        MaterialLibrary::default()
    }

    fn scene() -> Scene {
        let mut scene = Scene::new("Test", Mode::ThreeD);
        for id in ["a", "b"] {
            let mut actor = Actor::new(
                id,
                Visual::Cuboid {
                    color: "#fff".into(),
                    size: [1.0; 3],
                },
            );
            actor.id = id.into();
            scene.actors.push(actor);
        }
        scene
    }

    fn boxed() -> ColliderSpec {
        ColliderSpec::new(ColliderShape::Box { size: [1.0; 3] })
    }

    #[test]
    fn add_set_remove_by_id_keeps_order_and_identity() {
        let mut scene = scene();
        let first = scene.add_collider("a", boxed(), &lib()).unwrap();
        let second = scene.add_collider("a", boxed(), &lib()).unwrap();
        assert_ne!(first, second);
        let mut changed = scene.actors[0].components.collider(&first).unwrap().clone();
        changed.name = "Body".into();
        changed.trigger = true;
        scene.set_collider(changed, &lib()).unwrap();
        let ids: Vec<_> = scene.actors[0]
            .components
            .colliders()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(ids, vec![first.clone(), second.clone()], "edited in place");
        assert_eq!(
            scene.actors[0].components.collider(&first).unwrap().name,
            "Body"
        );
        assert_eq!(scene.remove_collider(&first, &lib()).unwrap(), "a");
        assert_eq!(scene.actors[0].components.colliders().count(), 1);
        assert!(scene.remove_collider(&first, &lib()).is_err());
    }

    #[test]
    fn a_refused_edit_leaves_the_actor_untouched() {
        let mut scene = scene();
        scene.add_collider("a", boxed(), &lib()).unwrap();
        let before = scene.clone();
        let mut bad = boxed();
        bad.geometry = ColliderGeometry::Shape {
            shape: ColliderShape::Sphere { radius: -1.0 },
        };
        let error = scene.add_collider("a", bad, &lib()).unwrap_err();
        assert!(error.contains("radius"), "{error}");
        assert_eq!(scene, before);
        // A 2D shape in a 3D scene.
        let flat = ColliderSpec::new(ColliderShape::Circle { radius: 1.0 });
        assert!(
            scene
                .add_collider("a", flat, &lib())
                .unwrap_err()
                .contains("2D")
        );
        assert_eq!(scene, before);
    }

    #[test]
    fn ids_are_unique_across_actors() {
        let mut scene = scene();
        let mut spec = boxed();
        spec.id = "shared".into();
        scene.add_collider("a", spec.clone(), &lib()).unwrap();
        assert!(
            scene
                .add_collider("b", spec, &lib())
                .unwrap_err()
                .contains("in use")
        );
    }

    #[test]
    fn an_unrelated_existing_error_does_not_block_an_edit() {
        let mut scene = scene();
        // Actor b is already invalid (hand-edited file): a negative mass.
        let body = RigidbodySpec {
            mass: MassSource::Explicit { mass: -1.0 },
            ..Default::default()
        };
        scene.actors[1]
            .components
            .insert(ActorComponent::Rigidbody { rigidbody: body });
        assert!(scene.add_collider("a", boxed(), &lib()).is_ok());
        // Fixing the broken actor is validated on its own.
        let bad = RigidbodySpec {
            mass: MassSource::Explicit { mass: 0.0 },
            ..Default::default()
        };
        assert!(scene.set_rigidbody("b", bad, &lib()).is_err());
    }

    #[test]
    fn a_dynamic_body_takes_a_triangle_mesh_only_for_the_plan_to_judge() {
        let mut scene = scene();
        scene
            .set_rigidbody("a", RigidbodySpec::default(), &lib())
            .unwrap();
        let mesh = ColliderSpec::new(ColliderShape::TriangleMesh {
            mesh: "assets/rock.glb".into(),
        });
        // An edit can't know whether the mesh will be decomposed, so the plan
        // (which has the cooking settings) is what refuses it at Play.
        scene.add_collider("a", mesh.clone(), &lib()).unwrap();
        let plan = scene.physics_plan(&Default::default());
        assert!(plan.errors().any(|e| e.message.contains("dynamic body")));
        // As a trigger it is fine, and so is scenery without a body.
        let mut trigger = mesh.clone();
        trigger.trigger = true;
        trigger.id = ColliderId::generate();
        assert!(scene.add_collider("a", trigger, &lib()).is_ok());
        let mut scenery = mesh;
        scenery.id = ColliderId::generate();
        assert!(scene.add_collider("b", scenery, &lib()).is_ok());
    }

    #[test]
    fn removing_a_rigidbody_keeps_the_collider_as_scenery() {
        let mut scene = scene();
        let id = scene.add_collider("a", boxed(), &lib()).unwrap();
        scene
            .set_rigidbody("a", RigidbodySpec::default(), &lib())
            .unwrap();
        scene.remove_rigidbody("a", &lib()).unwrap();
        assert!(scene.collider_actor(&id).is_some());
        assert!(scene.remove_rigidbody("a", &lib()).is_err());
    }

    #[test]
    fn a_legacy_body_beside_new_components_is_refused() {
        let mut scene = scene();
        scene.actors[0]
            .components
            .set_physics(crate::scene::Physics {
                body: crate::scene::BodyKind::Static,
                ..Default::default()
            });
        let error = scene.add_collider("a", boxed(), &lib()).unwrap_err();
        assert!(error.contains("legacy Body"), "{error}");
    }

    #[test]
    fn fitting_to_the_look_makes_the_shape_independent() {
        let mut scene = scene();
        let id = scene
            .add_collider("a", ColliderSpec::from_look(), &lib())
            .unwrap();
        scene.fit_collider_to_look(&id, &lib()).unwrap();
        let collider = scene.actors[0].components.collider(&id).unwrap();
        assert_eq!(
            collider.shape(),
            Some(&ColliderShape::Box { size: [1.0; 3] })
        );
        // Replacing the Look no longer moves the shape.
        scene.actors[0].components.set_visual(Visual::Cuboid {
            color: "#000".into(),
            size: [9.0; 3],
        });
        let collider = scene.actors[0].components.collider(&id).unwrap();
        assert_eq!(
            collider.shape(),
            Some(&ColliderShape::Box { size: [1.0; 3] })
        );
    }

    #[test]
    fn a_derived_collider_needs_a_look() {
        let mut scene = scene();
        scene.actors[0].components.remove("Look");
        assert!(
            scene
                .add_collider("a", ColliderSpec::from_look(), &lib())
                .is_err()
        );
        assert!(
            scene.add_collider("a", boxed(), &lib()).is_ok(),
            "invisible geometry is fine"
        );
    }

    #[test]
    fn normalizing_repairs_empty_and_repeated_ids_once() {
        let mut scene = scene();
        for actor in ["a", "b"] {
            let mut spec = boxed();
            spec.id = "same".into();
            scene
                .actors
                .iter_mut()
                .find(|a| a.id == actor)
                .unwrap()
                .components
                .insert(ActorComponent::Collider { collider: spec });
        }
        let mut empty = boxed();
        empty.id = ColliderId(String::new());
        scene.actors[0]
            .components
            .insert(ActorComponent::Collider { collider: empty });
        assert_eq!(scene.normalize_physics_ids(), 2);
        assert_eq!(scene.normalize_physics_ids(), 0);
        assert!(scene.physics_issues(&lib()).iter().all(|i| !i.is_error()));
    }

    #[test]
    fn a_copy_gets_new_ids_and_keeps_asset_references() {
        let mut scene = scene();
        let mut spec = boxed();
        spec.material = super::super::material::MaterialRef::BuiltIn { name: "Ice".into() };
        let id = scene.add_collider("a", spec, &lib()).unwrap();
        let mut copy = scene.actors[0].clone();
        copy.refresh_physics_ids();
        let copied = copy.components.colliders().next().unwrap();
        assert_ne!(copied.id, id);
        assert_eq!(
            copied.material,
            super::super::material::MaterialRef::BuiltIn { name: "Ice".into() }
        );
    }
}
