//! Ragdolls for 3D models: `go limp` hands the bones the Animation
//! component's `ragdoll` names to physics (one dynamic body per bone,
//! jointed to the nearest named bone above it) and `get up` hands them back,
//! blending into the animation. The bodies and the handoff are simulation,
//! on the tick; laying the bones over the bodies is presentation, after Bevy
//! animates. The actor's own body and colliders are switched off meanwhile,
//! and the actor follows the hips across the ground so cameras and senses
//! keep up.

use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_core::animation::RagdollSpec;
use blockloom_protocol::RuntimeMessage;

use crate::bridge;
use crate::engine::{Engine, PendingEffects};
use crate::model::ModelChild;
use crate::verbs::calls;

/// One bone handed to physics.
#[derive(Debug, Clone)]
pub struct Part {
    pub bone: Entity,
    pub body: Entity,
    /// The bone in its body's frame.
    pub offset: Transform,
}

/// On a limp actor: its parts, hips first, and what it switched off.
#[derive(Component, Debug, Clone)]
pub struct Ragdoll {
    pub parts: Vec<Part>,
    /// The actor's height above the ground when it went limp.
    gap: f32,
    disabled_body: bool,
    disabled_colliders: Vec<Entity>,
}

/// A ragdoll body, so nothing else mistakes it for an actor's.
#[derive(Component)]
pub struct RagdollPart;

/// On an actor getting up: the pose it lay in, blending out.
#[derive(Component, Debug, Clone)]
pub struct GettingUp {
    from: Vec<(Entity, Transform)>,
    left: f32,
    total: f32,
}

fn report(actor: &str, message: String) {
    bridge::send(&RuntimeMessage::Error {
        actor: actor.to_string(),
        message,
    });
}

/// The ground under `at`, ignoring `own`'s colliders and every ragdoll part.
fn ground_under(
    rapier: &rp::RapierContext,
    at: Vec3,
    own: Entity,
    parents: &Query<&ChildOf>,
    parts: &Query<(), With<RagdollPart>>,
) -> Option<f32> {
    let skip = |collider: Entity, _: &bevy_rapier3d::rapier::geometry::Collider| {
        if parts.contains(collider) {
            return false;
        }
        let mut at = collider;
        loop {
            if at == own {
                return false;
            }
            match parents.get(at) {
                Ok(parent) => at = parent.parent(),
                Err(_) => return true,
            }
        }
    };
    let filter = rp::QueryFilter::default()
        .exclude_sensors()
        .predicate(&skip);
    let origin = at + Vec3::Y * 0.5;
    rapier.with_query_pipeline(filter, |pipeline| {
        pipeline
            .cast_ray(origin, Vec3::NEG_Y, 50.0, true)
            .map(|(_, toi)| origin.y - toi)
    })
}

/// The named bones on the scene, each with the nearest named bone above it
/// and its first child bone (where its capsule ends).
fn find_parts(
    scene: Entity,
    spec: &RagdollSpec,
    children: &Query<&Children>,
    names: &Query<&Name>,
    parents: &Query<&ChildOf>,
) -> Result<Vec<(Entity, Option<usize>, Option<Entity>)>, String> {
    let mut found = Vec::new();
    for wanted in &spec.bones {
        let bone = children
            .iter_descendants(scene)
            .find(|&e| {
                names
                    .get(e)
                    .is_ok_and(|n| n.as_str().eq_ignore_ascii_case(wanted))
            })
            .ok_or_else(|| format!("the model has no bone called \"{wanted}\" for its ragdoll"))?;
        found.push(bone);
    }
    Ok(found
        .iter()
        .map(|&bone| {
            let mut above = None;
            let mut at = bone;
            while let Ok(parent) = parents.get(at) {
                at = parent.parent();
                if let Some(i) = found.iter().position(|&b| b == at) {
                    above = Some(i);
                    break;
                }
            }
            let tip = children
                .get(bone)
                .ok()
                .and_then(|kids| kids.iter().find(|kid| names.contains(*kid)));
            (bone, above, tip)
        })
        .collect())
}

/// `go limp` and `get up`, on the tick.
#[allow(clippy::too_many_arguments)]
pub fn apply_ragdoll_verbs(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
    actors: Query<(
        &ModelChild,
        Option<&Ragdoll>,
        Option<&rp::Velocity>,
        &Transform,
    )>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    locals: Query<&Transform>,
    colliders: Query<(), With<rp::Collider>>,
    disabled: Query<(Has<rp::RigidBodyDisabled>, Has<rp::ColliderDisabled>)>,
    parts: Query<(), With<RagdollPart>>,
    context: rp::ReadRapierContext,
) {
    if !engine.running || engine.paused {
        return;
    }
    let rapier = context.single().ok();
    for verb in ["go_limp", "get_up"] {
        for (actor, _) in calls(&effects, verb) {
            let Some(&entity) = engine.entities.get(actor) else {
                continue;
            };
            let Ok((model, limp, velocity, at)) = actors.get(entity) else {
                report(actor, "only a model actor can ragdoll".to_string());
                continue;
            };
            if verb == "get_up" {
                let Some(limp) = limp else {
                    continue;
                };
                let from = limp
                    .parts
                    .iter()
                    .flat_map(|part| {
                        let mut bones = vec![part.bone];
                        bones.extend(children.iter_descendants(part.bone));
                        bones
                    })
                    .filter_map(|bone| locals.get(bone).ok().map(|t| (bone, *t)))
                    .collect();
                let total = engine
                    .actor(actor)
                    .and_then(|a| a.components.animation())
                    .and_then(|spec| spec.ragdoll.as_ref())
                    .map_or(0.4, |spec| spec.get_up);
                for part in &limp.parts {
                    commands.entity(part.body).despawn();
                }
                let mut back = commands.entity(entity);
                back.remove::<Ragdoll>().insert(GettingUp {
                    from,
                    left: total,
                    total,
                });
                if limp.disabled_body {
                    back.remove::<rp::RigidBodyDisabled>();
                }
                for &collider in &limp.disabled_colliders {
                    commands.entity(collider).remove::<rp::ColliderDisabled>();
                }
                continue;
            }
            if limp.is_some() {
                continue;
            }
            let Some(spec) = engine
                .actor(actor)
                .and_then(|a| a.components.animation())
                .filter(|_| engine.has_component(actor, "Animation"))
                .and_then(|spec| spec.ragdoll.clone())
                .filter(|spec| !spec.bones.is_empty())
            else {
                report(
                    actor,
                    "my Animation component names no ragdoll bones".to_string(),
                );
                continue;
            };
            let found = match find_parts(model.0, &spec, &children, &names, &parents) {
                Ok(found) => found,
                Err(message) => {
                    report(actor, message);
                    continue;
                }
            };
            let speed = velocity.cloned().unwrap_or_default();
            let mass = spec.mass / found.len() as f32;
            let swing = spec.swing.to_radians();
            let mut made: Vec<(Entity, Transform)> = Vec::new();
            let mut ragdoll = Vec::new();
            for &(bone, above, tip) in &found {
                let world = globals
                    .get(bone)
                    .copied()
                    .unwrap_or_default()
                    .compute_transform();
                let frame =
                    Transform::from_translation(world.translation).with_rotation(world.rotation);
                let shape = match tip.and_then(|tip| globals.get(tip).ok()) {
                    Some(end) => {
                        let local =
                            world.rotation.inverse() * (end.translation() - world.translation);
                        if local.length() > spec.radius * 0.5 {
                            rp::Collider::capsule(Vec3::ZERO, local, spec.radius)
                        } else {
                            rp::Collider::ball(spec.radius)
                        }
                    }
                    None => rp::Collider::ball(spec.radius),
                };
                let body = commands
                    .spawn((
                        RagdollPart,
                        rp::RigidBody::Dynamic,
                        shape,
                        rp::ColliderMassProperties::Mass(mass),
                        rp::Damping {
                            linear_damping: 0.1,
                            angular_damping: 1.0,
                        },
                        speed,
                        frame,
                    ))
                    .id();
                if let Some(i) = above {
                    let (parent_body, parent_frame) = made[i];
                    let anchor = parent_frame.rotation.inverse()
                        * (frame.translation - parent_frame.translation);
                    let mut joint = rp::SphericalJointBuilder::new()
                        .local_anchor1(anchor)
                        .local_anchor2(Vec3::ZERO)
                        .contacts_enabled(false);
                    for axis in [
                        rp::JointAxis::AngX,
                        rp::JointAxis::AngY,
                        rp::JointAxis::AngZ,
                    ] {
                        joint = joint.limits(axis, [-swing, swing]);
                    }
                    commands
                        .entity(body)
                        .insert(rp::ImpulseJoint::new(parent_body, joint));
                }
                made.push((body, frame));
                let offset = Transform::from_scale(world.scale);
                ragdoll.push(Part { bone, body, offset });
            }
            // The actor's own body steps aside while the ragdoll lies there.
            let (body_off, _) = disabled.get(entity).unwrap_or_default();
            let mut off = Vec::new();
            for collider in std::iter::once(entity).chain(children.iter_descendants(entity)) {
                if colliders.contains(collider) && !disabled.get(collider).is_ok_and(|d| d.1) {
                    commands.entity(collider).insert(rp::ColliderDisabled);
                    off.push(collider);
                }
            }
            if !body_off {
                commands.entity(entity).insert(rp::RigidBodyDisabled);
            }
            let gap = rapier
                .as_ref()
                .and_then(|r| ground_under(r, at.translation, entity, &parents, &parts))
                .map_or(0.0, |ground| at.translation.y - ground);
            commands.entity(entity).insert(Ragdoll {
                parts: ragdoll,
                gap,
                disabled_body: !body_off,
                disabled_colliders: off,
            });
        }
    }
}

/// A limp actor follows its hips: over them, standing as high above the
/// ground under them as it stood when it fell.
pub fn follow_ragdolls(
    engine: NonSend<Engine>,
    mut actors: Query<(Entity, &Ragdoll, &mut Transform), Without<RagdollPart>>,
    bodies: Query<&Transform, With<RagdollPart>>,
    parents: Query<&ChildOf>,
    parts: Query<(), With<RagdollPart>>,
    context: rp::ReadRapierContext,
) {
    if !engine.running || engine.paused {
        return;
    }
    let rapier = context.single().ok();
    for (entity, ragdoll, mut at) in &mut actors {
        let Some(hips) = ragdoll.parts.first().and_then(|p| bodies.get(p.body).ok()) else {
            continue;
        };
        let ground = rapier
            .as_ref()
            .and_then(|r| ground_under(r, hips.translation, entity, &parents, &parts));
        at.translation.x = hips.translation.x;
        at.translation.z = hips.translation.z;
        if let Some(ground) = ground {
            at.translation.y = ground + ragdoll.gap;
        }
    }
}

/// Lays each ragdoll bone over its body, and blends a getting-up actor from
/// where it lay into what its animation now plays.
pub fn pose_ragdolls(
    mut commands: Commands,
    time: Res<Time>,
    ragdolls: Query<&Ragdoll>,
    mut rising: Query<(Entity, &mut GettingUp)>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform, Without<RagdollPart>>,
    bodies: Query<&Transform, With<RagdollPart>>,
) {
    for ragdoll in &ragdolls {
        for part in &ragdoll.parts {
            // A body is a root, so its local transform is its world one.
            let Ok(&body) = bodies.get(part.body) else {
                continue;
            };
            let world = body.mul_transform(part.offset);
            let parent = parents
                .get(part.bone)
                .map(|p| global_of(p.parent(), &transforms, &parents))
                .unwrap_or_default();
            if let Ok(mut local) = transforms.get_mut(part.bone) {
                let inverse = parent.compute_affine().inverse();
                *local = Transform::from_matrix((inverse * world.compute_affine()).into());
            }
        }
    }
    let dt = time.delta_secs();
    for (entity, mut rising) in &mut rising {
        rising.left -= dt;
        let share = if rising.total > 0.0 {
            (rising.left / rising.total).clamp(0.0, 1.0)
        } else {
            0.0
        };
        for (bone, from) in &rising.from {
            if let Ok(mut local) = transforms.get_mut(*bone) {
                local.translation = local.translation.lerp(from.translation, share);
                local.rotation = local.rotation.slerp(from.rotation, share);
            }
        }
        if rising.left <= 0.0 {
            commands.entity(entity).remove::<GettingUp>();
        }
    }
}

fn global_of(
    entity: Entity,
    transforms: &Query<&mut Transform, Without<RagdollPart>>,
    parents: &Query<&ChildOf>,
) -> Transform {
    let mut out = transforms.get(entity).copied().unwrap_or_default();
    let mut at = entity;
    while let Ok(parent) = parents.get(at) {
        at = parent.parent();
        out = transforms
            .get(at)
            .copied()
            .unwrap_or_default()
            .mul_transform(out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{ActorId, InsertEngine};
    use crate::model::ModelRig;
    use blockloom_core::animation::AnimationSpec;
    use blockloom_core::components::ActorComponent;
    use blockloom_core::project::Actor;
    use blockloom_core::scene::{Mode, Visual};
    use blockloom_core::vm::Effect;

    #[test]
    fn a_limp_body_falls_jointed_and_gets_back_up() {
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        engine.running = true;
        let mut actor = Actor::new(
            "Hero",
            Visual::Cuboid {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0, 1.0],
            },
        );
        let spec: AnimationSpec =
            serde_json::from_str(r#"{"ragdoll":{"bones":["Hips","Thigh","Shin"],"get_up":0.2}}"#)
                .unwrap();
        actor
            .components
            .insert(ActorComponent::Animation { animation: spec });
        let id = actor.id.clone();
        engine
            .attached
            .insert(id.clone(), ["Animation".to_string()].into_iter().collect());
        engine.project.actors.push(actor);
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            bevy::time::TimePlugin,
            bevy::transform::TransformPlugin,
            bevy::asset::AssetPlugin::default(),
        ));
        app.init_asset::<Mesh>();
        app.add_plugins(rp::RapierPhysicsPlugin::<()>::default());
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
        app.init_resource::<PendingEffects>();
        app.world_mut().spawn((
            rp::Collider::cuboid(5.0, 0.5, 5.0),
            Transform::from_xyz(0.0, -0.5, 0.0),
        ));
        let rest = [
            ("Hips", Vec3::new(0.0, 0.0, 0.0)),
            ("Thigh", Vec3::new(0.1, -0.05, 0.0)),
            ("Shin", Vec3::new(0.0, -0.4, 0.0)),
            ("Foot", Vec3::new(0.0, -0.4, 0.0)),
        ];
        let bones: Vec<Entity> = rest
            .iter()
            .map(|(name, at)| {
                app.world_mut()
                    .spawn((Name::new(*name), Transform::from_translation(*at)))
                    .id()
            })
            .collect();
        for pair in bones.windows(2) {
            app.world_mut().entity_mut(pair[0]).add_child(pair[1]);
        }
        let scene = app
            .world_mut()
            .spawn((ModelRig::empty(), Transform::default()))
            .id();
        app.world_mut().entity_mut(scene).add_child(bones[0]);
        let hero = app
            .world_mut()
            .spawn((
                ActorId(id.clone()),
                ModelChild(scene),
                Transform::from_xyz(0.0, 1.5, 0.0),
                rp::RigidBody::KinematicPositionBased,
                rp::Collider::capsule_y(0.5, 0.3),
            ))
            .id();
        app.world_mut().entity_mut(hero).add_child(scene);
        engine.entities.insert(id.clone(), hero);
        app.insert_engine(engine);
        app.add_systems(
            FixedUpdate,
            (
                apply_ragdoll_verbs,
                follow_ragdolls,
                crate::world::clear_effects,
            )
                .chain(),
        );
        app.add_systems(
            PostUpdate,
            pose_ragdolls.before(bevy::transform::TransformSystems::Propagate),
        );
        for _ in 0..3 {
            app.update();
        }
        let limp = Effect::Verb {
            actor: id.clone(),
            verb: "go_limp".to_string(),
            args: Vec::new(),
        };
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![limp];
        app.update();
        let ragdoll = app.world().get::<Ragdoll>(hero).cloned().expect("limp");
        assert_eq!(ragdoll.parts.len(), 3);
        assert!(app.world().get::<rp::RigidBodyDisabled>(hero).is_some());
        assert!(app.world().get::<rp::ColliderDisabled>(hero).is_some());
        let jointed = ragdoll
            .parts
            .iter()
            .filter(|p| app.world().get::<rp::ImpulseJoint>(p.body).is_some())
            .count();
        assert_eq!(jointed, 2, "every part but the hips hangs off one above");
        assert!((ragdoll.gap - 1.5).abs() < 0.05, "stood {} up", ragdoll.gap);
        for _ in 0..90 {
            app.update();
        }
        // It fell from 1.5 m onto the ground (the leg may prop it up), the
        // bones riding their bodies.
        let hips = app
            .world()
            .get::<Transform>(ragdoll.parts[0].body)
            .unwrap()
            .translation;
        assert!(hips.y < 1.0 && hips.y > 0.0, "hips at {hips}");
        let shin = ragdoll.parts[2].clone();
        let body = *app.world().get::<GlobalTransform>(shin.body).unwrap();
        let bone = *app.world().get::<GlobalTransform>(shin.bone).unwrap();
        assert!(body.translation().distance(bone.translation()) < 0.02);
        let at = app.world().get::<Transform>(hero).unwrap().translation;
        assert!(
            (at.x - hips.x).abs() < 0.05 && (at.y - 1.5).abs() < 0.1,
            "actor at {at}"
        );

        let up = Effect::Verb {
            actor: id,
            verb: "get_up".to_string(),
            args: Vec::new(),
        };
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![up];
        app.update();
        assert!(app.world().get::<Ragdoll>(hero).is_none());
        assert!(app.world().get::<GettingUp>(hero).is_some());
        assert!(app.world().get::<rp::RigidBodyDisabled>(hero).is_none());
        assert!(app.world().get::<rp::ColliderDisabled>(hero).is_none());
        assert!(app.world().get_entity(ragdoll.parts[0].body).is_err());
        for _ in 0..20 {
            app.update();
        }
        assert!(app.world().get::<GettingUp>(hero).is_none());
    }
}
