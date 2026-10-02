//! Legacy `Body` to Rigidbody + Collider.
//!
//! Old documents keep one `Body` per actor with the shape derived from its Look.
//! Opening one changes nothing: it loads, runs and saves byte for byte as before.
//! The functions here describe and, on request, perform the conversion:
//!
//! | Legacy | Becomes |
//! | --- | --- |
//! | No Body / `None` | no Rigidbody, no Collider |
//! | `Static` | a Collider that follows the Look |
//! | `Dynamic` / `Kinematic` | a Rigidbody plus that Collider |
//! | friction, restitution | a stored material with exactly those values |
//! | mass or density | the matching mass source |
//! | rotation lock | frozen rotation axes |
//! | layer and mask | the layer slot and a legacy mask |
//! | one-way (2D) | the collider's one-way surface |
//! | character-control switch | `legacy_character_control` on the Rigidbody |
//!
//! Conversion is idempotent (the `Body` is gone afterwards, so a second pass finds
//! nothing) and uses ids derived from the actor's, so repeating it from the same
//! document gives the same result.

use serde::Serialize;

use super::ids::{ColliderId, ComponentId};
use super::material::{
    MaterialBody, MaterialLibrary, MaterialRef, PhysicsMaterial, PhysicsMaterial2D,
};
use super::spec::{
    Axis, BodyType, ColliderShape, ColliderSpec, Constraints, MassSource, RigidbodySpec,
};
use crate::components::ActorComponent;
use crate::project::{Actor, Scene};
use crate::scene::{BodyKind, Mode, Physics, Visual};

/// How thick the legacy runtime drew a `Plane` look's collider (metres).
pub const LEGACY_PLANE_THICKNESS: f32 = 0.2;

/// The saved shape a Look implies, as the legacy runtime built it, or `None` when
/// that Look has no collider in a world of `mode`.
pub fn shape_from_look(visual: &Visual, mode: Mode) -> Option<ColliderShape> {
    Some(match (visual, mode) {
        (Visual::Cuboid { size, .. }, Mode::ThreeD) => ColliderShape::Box { size: *size },
        (Visual::Sphere { radius, .. }, Mode::ThreeD) => ColliderShape::Sphere { radius: *radius },
        // The old capsule's `height` was the straight part; the new one is end to end.
        (Visual::Capsule { radius, height, .. }, Mode::ThreeD) => ColliderShape::Capsule {
            radius: *radius,
            height: height + 2.0 * radius,
            axis: Axis::Y,
        },
        (Visual::Plane { size, .. }, Mode::ThreeD) => ColliderShape::Box {
            size: [size[0], LEGACY_PLANE_THICKNESS, size[1]],
        },
        (Visual::Model { scale, .. }, Mode::ThreeD) => ColliderShape::Box { size: *scale },
        (Visual::Rect { size, .. } | Visual::Image { size, .. }, Mode::TwoD) => {
            ColliderShape::Rect { size: *size }
        }
        (Visual::Circle { radius, .. }, Mode::TwoD) => ColliderShape::Circle { radius: *radius },
        (Visual::Tilemap { .. }, _) => ColliderShape::Tilemap,
        _ => return None,
    })
}

/// Volume (3D) or area (2D) of a shape, when it has a closed form.
fn measure(shape: &ColliderShape) -> Option<f32> {
    use std::f32::consts::PI;
    Some(match shape {
        ColliderShape::Box { size } => size[0] * size[1] * size[2],
        ColliderShape::Sphere { radius } => 4.0 / 3.0 * PI * radius.powi(3),
        ColliderShape::Capsule { radius, height, .. } => {
            PI * radius * radius * (height - 2.0 * radius) + 4.0 / 3.0 * PI * radius.powi(3)
        }
        ColliderShape::Rect { size } => size[0] * size[1],
        ColliderShape::Circle { radius } => PI * radius * radius,
        _ => return None,
    })
}

/// What one legacy `Body` converts to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Converted {
    pub rigidbody: Option<RigidbodySpec>,
    pub collider: Option<ColliderSpec>,
    /// The material the collider references, to be stored in the library.
    #[serde(skip)]
    pub material: Option<(String, MaterialBody)>,
    /// Behavior that does not carry over exactly.
    pub notes: Vec<String>,
}

/// Converts one `Body`. Pure: nothing is stored until [`Scene::migrate_physics`].
pub fn convert_body(actor: &Actor, physics: &Physics, mode: Mode) -> Converted {
    let mut notes = Vec::new();
    if physics.body == BodyKind::None {
        return Converted {
            rigidbody: None,
            collider: None,
            material: None,
            notes,
        };
    }

    let look_shape = actor
        .components
        .visual()
        .and_then(|visual| shape_from_look(visual, mode));

    let mut collider = ColliderSpec::from_look();
    collider.id = ColliderId(format!("{}-collider", actor.id));
    collider.name = "Legacy shape".to_string();
    collider.trigger = physics.trigger;
    collider.layer = physics.collision_layer.clamp(1, 8);
    if physics.collision_mask != 0xFF {
        collider.legacy_mask = Some(u32::from(physics.collision_mask));
    }
    if physics.one_way {
        if mode == Mode::TwoD && physics.body == BodyKind::Static && !physics.trigger {
            collider.one_way = true;
        } else {
            notes.push("one-way only applied to 2D static solid bodies, so it is dropped".into());
        }
    }

    let (name, body) = match mode {
        Mode::ThreeD => (
            format!(
                "Legacy f{:.2} b{:.2}",
                physics.friction, physics.restitution
            ),
            MaterialBody::Three {
                material: PhysicsMaterial {
                    static_friction: physics.friction,
                    dynamic_friction: physics.friction,
                    bounciness: physics.restitution,
                    ..PhysicsMaterial::default()
                },
            },
        ),
        Mode::TwoD => {
            notes.push(
                "2D contacts now use the geometric mean of the two frictions and the larger bounciness; the old rule averaged them, so a pair of different materials can behave a little differently"
                    .into(),
            );
            (
                format!(
                    "Legacy f{:.2} b{:.2} 2D",
                    physics.friction, physics.restitution
                ),
                MaterialBody::Two {
                    material: PhysicsMaterial2D {
                        friction: physics.friction,
                        bounciness: physics.restitution,
                    },
                },
            )
        }
    };

    let rigidbody = match physics.body {
        BodyKind::Static | BodyKind::None => None,
        BodyKind::Dynamic | BodyKind::Kinematic => {
            let mut spec = RigidbodySpec {
                id: ComponentId(format!("{}-rigidbody", actor.id)),
                body_type: if physics.body == BodyKind::Dynamic {
                    BodyType::Dynamic
                } else {
                    BodyType::Kinematic
                },
                gravity_scale: physics.gravity_scale,
                // The old runtime had no separate Use Gravity switch.
                use_gravity: true,
                // Old bodies were always drawn between their steps.
                interpolation: crate::physics::Interpolation::Interpolate,
                ..RigidbodySpec::default()
            };
            if physics.lock_rotation {
                spec.constraints = Constraints {
                    freeze_position: [false; 3],
                    freeze_rotation: match mode {
                        Mode::ThreeD => [true; 3],
                        Mode::TwoD => [false, false, true],
                    },
                };
            }
            spec.mass = match physics.mass {
                Some(mass) => MassSource::Explicit { mass },
                None if physics.trigger => match look_shape.as_ref().and_then(measure) {
                    // Triggers add no mass in the new model, so keep the weight the
                    // old shape gave this body by saying it outright.
                    Some(size) => {
                        notes.push(
                            "a trigger body's old density mass is now an explicit mass".into(),
                        );
                        MassSource::Explicit {
                            mass: (physics.density * size).max(f32::MIN_POSITIVE),
                        }
                    }
                    None => MassSource::Density {
                        density: physics.density,
                    },
                },
                None => MassSource::Density {
                    density: physics.density,
                },
            };
            spec.legacy_character_control = physics.character_controller;
            Some(spec)
        }
    };
    if physics.character_controller && physics.body != BodyKind::Kinematic {
        notes.push("the character-control switch only ever worked on kinematic bodies".into());
    }

    Converted {
        rigidbody,
        collider: Some(collider),
        material: Some((name, body)),
        notes,
    }
}

/// What converting one actor did or would do.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActorMigration {
    pub actor: String,
    pub name: String,
    pub body: String,
    pub rigidbody: Option<RigidbodySpec>,
    pub collider: Option<ColliderSpec>,
    pub notes: Vec<String>,
}

impl Scene {
    /// The conversion of every legacy `Body` in this scene, without applying it.
    /// `library` is the project's, so the preview names the same materials the real
    /// conversion would store.
    pub fn physics_migration_preview(&self, library: &MaterialLibrary) -> Vec<ActorMigration> {
        self.clone().migrate_physics(&mut library.clone())
    }

    /// Replaces every legacy `Body` with its Rigidbody and Collider and stores the
    /// materials those need in `library`. Returns what was converted; a second
    /// call converts nothing.
    pub fn migrate_physics(&mut self, library: &mut MaterialLibrary) -> Vec<ActorMigration> {
        let mode = self.world.mode;
        let mut done = Vec::new();
        for actor in &mut self.actors {
            let Some(physics) = actor.components.get("Body").and_then(|c| match c {
                ActorComponent::Body { physics } => Some(*physics),
                _ => None,
            }) else {
                continue;
            };
            let mut converted = convert_body(actor, &physics, mode);
            if let (Some(collider), Some((name, body))) =
                (converted.collider.as_mut(), converted.material.take())
            {
                collider.material = MaterialRef::Asset {
                    id: library.find_or_add(&name, body),
                };
            }
            let at = actor
                .components
                .0
                .iter()
                .position(|c| c.name() == "Body")
                .expect("just found it");
            actor.components.0.remove(at);
            let mut insert_at = at;
            if let Some(rigidbody) = converted.rigidbody.clone() {
                actor
                    .components
                    .0
                    .insert(insert_at, ActorComponent::Rigidbody { rigidbody });
                insert_at += 1;
            }
            if let Some(collider) = converted.collider.clone() {
                actor
                    .components
                    .0
                    .insert(insert_at, ActorComponent::Collider { collider });
            }
            done.push(ActorMigration {
                actor: actor.id.clone(),
                name: actor.name.clone(),
                body: format!("{:?}", physics.body),
                rigidbody: converted.rigidbody,
                collider: converted.collider,
                notes: converted.notes,
            });
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::super::spec::ColliderGeometry;
    use super::*;

    fn cuboid_actor(id: &str, physics: Physics) -> Actor {
        let mut actor = Actor::new(
            id,
            Visual::Cuboid {
                color: "#fff".into(),
                size: [2.0, 1.0, 4.0],
            },
        );
        actor.id = id.into();
        actor.components.set_physics(physics);
        actor
    }

    fn scene(mode: Mode, actors: Vec<Actor>) -> Scene {
        let mut scene = Scene::new("Test", mode);
        scene.actors = actors;
        scene
    }

    #[test]
    fn no_body_stays_decoration() {
        let none = cuboid_actor("a", Physics::default());
        let mut scene = scene(Mode::ThreeD, vec![none]);
        let mut library = MaterialLibrary::default();
        let done = scene.migrate_physics(&mut library);
        assert_eq!(done.len(), 1);
        assert!(done[0].rigidbody.is_none() && done[0].collider.is_none());
        assert!(scene.actors[0].components.colliders().next().is_none());
        assert!(!scene.actors[0].components.contains("Body"));
        assert!(library.materials.is_empty());
    }

    #[test]
    fn static_body_becomes_a_collider_only_actor() {
        let physics = Physics {
            body: BodyKind::Static,
            friction: 0.9,
            restitution: 0.2,
            collision_layer: 3,
            collision_mask: 0b0000_0101,
            ..Physics::default()
        };
        let mut scene = scene(Mode::ThreeD, vec![cuboid_actor("wall", physics)]);
        let mut library = MaterialLibrary::default();
        scene.migrate_physics(&mut library);
        let actor = &scene.actors[0];
        assert!(actor.components.rigidbody().is_none());
        let collider = actor.components.colliders().next().unwrap();
        assert_eq!(collider.geometry, ColliderGeometry::FromLook);
        assert_eq!(collider.layer, 3);
        assert_eq!(collider.legacy_mask, Some(0b101));
        assert_eq!(collider.id.as_str(), "wall-collider");
        let MaterialRef::Asset { id } = &collider.material else {
            panic!("a stored material");
        };
        let MaterialBody::Three { material } = library.get(id).unwrap().body else {
            panic!("3D material");
        };
        assert_eq!(
            (material.static_friction, material.dynamic_friction),
            (0.9, 0.9)
        );
        assert_eq!(material.bounciness, 0.2);
    }

    #[test]
    fn dynamic_and_kinematic_bodies_get_a_rigidbody() {
        let dynamic = Physics {
            body: BodyKind::Dynamic,
            gravity_scale: 2.0,
            lock_rotation: true,
            mass: Some(7.0),
            ..Physics::default()
        };
        let kinematic = Physics {
            body: BodyKind::Kinematic,
            character_controller: true,
            ..Physics::default()
        };
        let mut scene = scene(
            Mode::ThreeD,
            vec![
                cuboid_actor("crate", dynamic),
                cuboid_actor("lift", kinematic),
            ],
        );
        scene.migrate_physics(&mut MaterialLibrary::default());
        let body = scene.actors[0].components.rigidbody().unwrap();
        assert_eq!(body.body_type, BodyType::Dynamic);
        assert_eq!(body.gravity_scale, 2.0);
        assert_eq!(body.mass, MassSource::Explicit { mass: 7.0 });
        assert_eq!(body.constraints.freeze_rotation, [true; 3]);
        let lift = scene.actors[1].components.rigidbody().unwrap();
        assert_eq!(lift.body_type, BodyType::Kinematic);
        assert!(lift.legacy_character_control);
    }

    #[test]
    fn density_mass_is_kept_and_a_trigger_body_keeps_its_weight() {
        let heavy = Physics {
            body: BodyKind::Dynamic,
            density: 3.0,
            ..Physics::default()
        };
        let sensor = Physics {
            body: BodyKind::Dynamic,
            density: 3.0,
            trigger: true,
            ..Physics::default()
        };
        let mut scene = scene(
            Mode::ThreeD,
            vec![cuboid_actor("a", heavy), cuboid_actor("b", sensor)],
        );
        let done = scene.migrate_physics(&mut MaterialLibrary::default());
        assert_eq!(
            scene.actors[0].components.rigidbody().unwrap().mass,
            MassSource::Density { density: 3.0 }
        );
        // 2 x 1 x 4 metres at density 3 weighs 24 kg.
        assert_eq!(
            scene.actors[1].components.rigidbody().unwrap().mass,
            MassSource::Explicit { mass: 24.0 }
        );
        assert!(done[1].notes.iter().any(|n| n.contains("explicit mass")));
    }

    #[test]
    fn two_d_lock_one_way_and_combine_note() {
        let mut platform = Actor::new(
            "platform",
            Visual::Rect {
                color: "#fff".into(),
                size: [200.0, 20.0],
            },
        );
        platform.id = "platform".into();
        platform.components.set_physics(Physics {
            body: BodyKind::Static,
            one_way: true,
            ..Physics::default()
        });
        let mut hero = Actor::new(
            "hero",
            Visual::Rect {
                color: "#fff".into(),
                size: [30.0, 60.0],
            },
        );
        hero.id = "hero".into();
        hero.components.set_physics(Physics {
            body: BodyKind::Dynamic,
            lock_rotation: true,
            ..Physics::default()
        });
        let mut scene = scene(Mode::TwoD, vec![platform, hero]);
        let done = scene.migrate_physics(&mut MaterialLibrary::default());
        assert!(
            scene.actors[0]
                .components
                .colliders()
                .next()
                .unwrap()
                .one_way
        );
        assert_eq!(
            scene.actors[1]
                .components
                .rigidbody()
                .unwrap()
                .constraints
                .freeze_rotation,
            [false, false, true]
        );
        assert!(done[0].notes.iter().any(|n| n.contains("geometric mean")));
    }

    #[test]
    fn the_old_capsule_height_becomes_end_to_end() {
        let capsule = Visual::Capsule {
            color: "#fff".into(),
            radius: 0.5,
            height: 1.0,
        };
        assert_eq!(
            shape_from_look(&capsule, Mode::ThreeD),
            Some(ColliderShape::Capsule {
                radius: 0.5,
                height: 2.0,
                axis: Axis::Y
            })
        );
        assert_eq!(shape_from_look(&capsule, Mode::TwoD), None);
        let plane = Visual::Plane {
            color: "#fff".into(),
            size: [10.0, 8.0],
        };
        assert_eq!(
            shape_from_look(&plane, Mode::ThreeD),
            Some(ColliderShape::Box {
                size: [10.0, 0.2, 8.0]
            })
        );
    }

    #[test]
    fn migration_is_idempotent_and_deterministic() {
        let physics = Physics {
            body: BodyKind::Dynamic,
            ..Physics::default()
        };
        let build = || scene(Mode::ThreeD, vec![cuboid_actor("a", physics)]);
        let mut one = build();
        let mut lib_one = MaterialLibrary::default();
        one.migrate_physics(&mut lib_one);
        let snapshot = one.clone();
        assert!(
            one.migrate_physics(&mut lib_one).is_empty(),
            "second pass finds nothing"
        );
        assert_eq!(one, snapshot);
        let mut two = build();
        let mut lib_two = MaterialLibrary::default();
        two.migrate_physics(&mut lib_two);
        assert_eq!(one.actors, two.actors, "same document, same ids");
        assert_eq!(lib_one, lib_two);
    }

    #[test]
    fn preview_changes_nothing_and_matches_the_result() {
        let physics = Physics {
            body: BodyKind::Dynamic,
            ..Physics::default()
        };
        let mut scene = scene(Mode::ThreeD, vec![cuboid_actor("a", physics)]);
        let before = scene.clone();
        let preview = scene.physics_migration_preview(&MaterialLibrary::default());
        assert_eq!(scene, before);
        let done = scene.migrate_physics(&mut MaterialLibrary::default());
        assert_eq!(preview, done);
    }
}
