//! 3D model IK, laid over the animated pose each frame: feet set on the
//! ground under them (the hips dropping so the lower one reaches) and a
//! head turned toward what `look at` names. The bones come from the
//! Animation component's `ik`; the look target is set on the tick and the
//! pose fix is presentation, after Bevy animates and before transforms
//! propagate.

use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_core::animation::ModelIk;
use blockloom_protocol::RuntimeMessage;
use serde_json::Value;

use crate::bridge;
use crate::engine::{ActorId, Engine, PendingEffects};
use crate::model::{ModelChild, ModelRig};
use crate::verbs::calls;

/// What an actor's head looks at, set by `look at`.
#[derive(Component, Debug, Clone, PartialEq)]
pub enum LookTarget {
    Actor(String),
    Point(Vec3),
}

/// `look_at`, `look_at_point` and `stop_looking`, on the tick.
pub fn apply_look_verbs(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let number =
        |args: &[Value], at: usize| args.get(at).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    for verb in ["look_at", "look_at_point", "stop_looking"] {
        for (actor, args) in calls(&effects, verb) {
            let Some(&entity) = engine.entities.get(actor) else {
                continue;
            };
            let mut target = commands.entity(entity);
            match verb {
                "look_at" => {
                    let named = args.first().and_then(Value::as_str).unwrap_or_default();
                    target.insert(LookTarget::Actor(named.to_string()));
                }
                "look_at_point" => {
                    target.insert(LookTarget::Point(Vec3::new(
                        number(args, 0),
                        number(args, 1),
                        number(args, 2),
                    )));
                }
                _ => {
                    target.remove::<LookTarget>();
                }
            }
        }
    }
}

/// The bones `ModelIk` names, found once on the loaded scene, and how far
/// each fix is eased in.
#[derive(Component)]
pub struct IkBones {
    spec: ModelIk,
    /// Thigh, shin and foot of each leg.
    legs: Vec<[Entity; 3]>,
    hips: Option<Entity>,
    head: Option<Entity>,
    leg_weights: Vec<f32>,
    look_weight: f32,
}

/// How long a fix takes to ease in or out, in seconds.
const EASE: f32 = 0.15;

/// The world deltas that bend a two-bone chain `a`-`b`-`c` (hip, knee,
/// ankle) to put `c` on `target`, keeping the knee bending the way it does:
/// the upper bone's rotation, then the lower's after it.
pub fn two_bone(a: Vec3, b: Vec3, c: Vec3, target: Vec3) -> (Quat, Quat) {
    let (upper, lower) = (b - a, c - b);
    let (l1, l2) = (upper.length(), lower.length());
    let to = target - a;
    if l1 < 1e-5 || l2 < 1e-5 || to.length() < 1e-5 {
        return (Quat::IDENTITY, Quat::IDENTITY);
    }
    let dir = to.normalize();
    let d = to
        .length()
        .clamp((l1 - l2).abs() + 1e-4, (l1 + l2) * 0.9999);
    let cos_a = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).max(0.0).sqrt();
    let bend = upper - dir * dir.dot(upper);
    let side = bend
        .try_normalize()
        .unwrap_or_else(|| dir.any_orthonormal_vector());
    let knee = a + dir * (l1 * cos_a) + side * (l1 * sin_a);
    let first = Quat::from_rotation_arc(upper / l1, (knee - a) / l1);
    let lower_now = (first * lower).normalize();
    let wanted = (a + dir * d - knee).normalize();
    (first, Quat::from_rotation_arc(lower_now, wanted))
}

/// The world rotation that turns `forward` toward `to`, at most `limit`
/// radians of it.
pub fn look_delta(forward: Vec3, to: Vec3, limit: f32) -> Quat {
    let (Some(forward), Some(to)) = (forward.try_normalize(), to.try_normalize()) else {
        return Quat::IDENTITY;
    };
    let full = Quat::from_rotation_arc(forward, to);
    let angle = forward.angle_between(to);
    if angle <= limit || angle < 1e-5 {
        full
    } else {
        Quat::IDENTITY.slerp(full, limit / angle)
    }
}

/// `entity`'s world transform from the local ones up its parents, so a
/// pose just animated counts before transforms propagate.
fn global(
    entity: Entity,
    transforms: &Query<&mut Transform>,
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

fn parent_of(
    entity: Entity,
    transforms: &Query<&mut Transform>,
    parents: &Query<&ChildOf>,
) -> Transform {
    parents
        .get(entity)
        .map(|p| global(p.parent(), transforms, parents))
        .unwrap_or_default()
}

/// Turns `entity` by the world rotation `delta`.
fn turn(
    entity: Entity,
    delta: Quat,
    transforms: &mut Query<&mut Transform>,
    parents: &Query<&ChildOf>,
) {
    let parent = parent_of(entity, transforms, parents).rotation;
    if let Ok(mut local) = transforms.get_mut(entity) {
        local.rotation = (parent.inverse() * delta * parent * local.rotation).normalize();
    }
}

/// Sets `entity`'s world rotation.
fn face(
    entity: Entity,
    world: Quat,
    transforms: &mut Query<&mut Transform>,
    parents: &Query<&ChildOf>,
) {
    let parent = parent_of(entity, transforms, parents).rotation;
    if let Ok(mut local) = transforms.get_mut(entity) {
        local.rotation = (parent.inverse() * world).normalize();
    }
}

/// Moves `entity` by the world offset `delta`.
fn shift(
    entity: Entity,
    delta: Vec3,
    transforms: &mut Query<&mut Transform>,
    parents: &Query<&ChildOf>,
) {
    let parent = parent_of(entity, transforms, parents);
    if let Ok(mut local) = transforms.get_mut(entity) {
        local.translation +=
            (parent.rotation.inverse() * delta) / parent.scale.max(Vec3::splat(1e-5));
    }
}

fn report(actor: &str, message: String) {
    bridge::send(&RuntimeMessage::Error {
        actor: actor.to_string(),
        message,
    });
}

/// Finds the bones on a freshly loaded (or re-specced) model scene.
fn find_bones(
    actor: &str,
    scene: Entity,
    spec: &ModelIk,
    children: &Query<&Children>,
    names: &Query<&Name>,
    parents: &Query<&ChildOf>,
) -> IkBones {
    let find = |wanted: &str| {
        children.iter_descendants(scene).find(|&entity| {
            names
                .get(entity)
                .is_ok_and(|name| name.as_str().eq_ignore_ascii_case(wanted))
        })
    };
    let named = |wanted: &str| {
        if wanted.is_empty() {
            return None;
        }
        let found = find(wanted);
        if found.is_none() {
            report(
                actor,
                format!("the model has no bone called \"{wanted}\" for its IK"),
            );
        }
        found
    };
    let mut legs = Vec::new();
    for foot in &spec.feet {
        let Some(foot) = named(foot) else {
            continue;
        };
        let shin = parents.get(foot).map(ChildOf::parent).ok();
        let thigh = shin.and_then(|shin| parents.get(shin).map(ChildOf::parent).ok());
        if let (Some(shin), Some(thigh)) = (shin, thigh) {
            legs.push([thigh, shin, foot]);
        }
    }
    IkBones {
        spec: spec.clone(),
        leg_weights: vec![0.0; legs.len()],
        legs,
        hips: named(&spec.hips),
        head: named(&spec.head),
        look_weight: 0.0,
    }
}

/// Lays the IK over every IK'd model's animated pose.
#[allow(clippy::too_many_arguments)]
pub fn solve_ik(
    mut commands: Commands,
    time: Res<Time>,
    engine: NonSend<Engine>,
    actors: Query<
        (Entity, &ActorId, &ModelChild, Option<&LookTarget>),
        Without<crate::ragdoll::Ragdoll>,
    >,
    mut rigs: Query<Option<&mut IkBones>, With<ModelRig>>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform>,
    context: rp::ReadRapierContext,
) {
    let dt = time.delta_secs();
    let step = dt / EASE;
    let ease = |weight: &mut f32, on: bool| {
        *weight = (*weight + if on { step } else { -step }).clamp(0.0, 1.0);
    };
    let rapier = context.single().ok();
    for (entity, id, child, look) in &actors {
        let Some(ik) = engine
            .actor(&id.0)
            .and_then(|a| a.components.animation())
            .filter(|_| engine.has_component(&id.0, "Animation"))
            .and_then(|spec| spec.ik.as_ref())
        else {
            continue;
        };
        let Ok(bones) = rigs.get_mut(child.0) else {
            continue;
        };
        let Some(mut bones) = bones else {
            commands
                .entity(child.0)
                .insert(find_bones(&id.0, child.0, ik, &children, &names, &parents));
            continue;
        };
        if bones.spec != *ik {
            *bones = find_bones(&id.0, child.0, ik, &children, &names, &parents);
        }
        let body = global(entity, &transforms, &parents);
        let up = body.rotation * Vec3::Y;

        // Feet: the ground under each, then the hips down to the lower one.
        let mut targets = Vec::with_capacity(bones.legs.len());
        for (i, leg) in bones.legs.clone().iter().enumerate() {
            let ankle = global(leg[2], &transforms, &parents).translation;
            let reach = bones.spec.foot_reach;
            let hit = rapier.as_ref().and_then(|rapier| {
                let own = |collider: Entity| {
                    let mut at = collider;
                    loop {
                        if at == entity {
                            return true;
                        }
                        match parents.get(at) {
                            Ok(parent) => at = parent.parent(),
                            Err(_) => return false,
                        }
                    }
                };
                let predicate =
                    |collider: Entity, _: &bevy_rapier3d::rapier::geometry::Collider| {
                        !own(collider)
                    };
                let filter = rp::QueryFilter::default()
                    .exclude_sensors()
                    .predicate(&predicate);
                let origin = ankle + up * reach;
                rapier.with_query_pipeline(filter, |pipeline| {
                    pipeline
                        .cast_ray(origin, -up, reach * 2.0 + bones.spec.foot_height, true)
                        .map(|(_, toi)| origin - up * toi + up * bones.spec.foot_height)
                })
            });
            ease(&mut bones.leg_weights[i], hit.is_some());
            let target = hit.unwrap_or(ankle);
            targets.push(ankle.lerp(target, bones.leg_weights[i]));
        }
        if let Some(hips) = bones.hips {
            let drop = bones
                .legs
                .iter()
                .zip(&targets)
                .map(|(leg, target)| {
                    let ankle = global(leg[2], &transforms, &parents).translation;
                    (*target - ankle).dot(up)
                })
                .fold(0.0f32, f32::min);
            if drop < 0.0 {
                shift(hips, up * drop, &mut transforms, &parents);
            }
        }
        for (leg, target) in bones.legs.iter().zip(&targets) {
            let [thigh, shin, foot] = *leg;
            let a = global(thigh, &transforms, &parents).translation;
            let b = global(shin, &transforms, &parents).translation;
            let foot_now = global(foot, &transforms, &parents);
            let (first, second) = two_bone(a, b, foot_now.translation, *target);
            turn(thigh, first, &mut transforms, &parents);
            turn(shin, second, &mut transforms, &parents);
            // The foot keeps the angle its animation gave it.
            face(foot, foot_now.rotation, &mut transforms, &parents);
        }

        // The head, toward the target from the body's own facing.
        let aim = match look {
            Some(LookTarget::Point(point)) => Some(*point),
            Some(LookTarget::Actor(other)) => engine
                .entities
                .get(other)
                .map(|&other| global(other, &transforms, &parents).translation),
            None => None,
        };
        ease(&mut bones.look_weight, aim.is_some());
        if let Some(head) = bones.head
            && bones.look_weight > 0.0
            && let Some(aim) = aim
        {
            let from = global(head, &transforms, &parents).translation;
            let forward = body.rotation * Vec3::NEG_Z;
            let delta = look_delta(forward, aim - from, bones.spec.look_limit.to_radians());
            turn(
                head,
                Quat::IDENTITY.slerp(delta, bones.look_weight),
                &mut transforms,
                &parents,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_bones_reach_a_target_and_keep_their_bend() {
        let (a, b, c) = (
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.5, -0.05),
            Vec3::ZERO,
        );
        let target = Vec3::new(0.0, 0.2, 0.0);
        let (first, second) = two_bone(a, b, c, target);
        let knee = a + first * (b - a);
        let ankle = knee + second * (first * (c - b));
        assert!(ankle.distance(target) < 1e-3, "ankle at {ankle}");
        // Lengths hold, and the knee still bends forward (-Z).
        assert!(((knee - a).length() - (b - a).length()).abs() < 1e-4);
        assert!(knee.z < b.z, "knee at {knee}");
        // Out of reach, the leg straightens toward it.
        let (first, second) = two_bone(a, b, c, Vec3::new(0.0, -5.0, 0.0));
        let knee = a + first * (b - a);
        let ankle = knee + second * (first * (c - b));
        assert!(ankle.x.abs() < 1e-3 && ankle.y < 0.0);
    }

    #[test]
    fn a_look_turns_no_further_than_its_limit() {
        let forward = Vec3::NEG_Z;
        let side = look_delta(forward, Vec3::X, 30f32.to_radians());
        let turned = side * forward;
        assert!((turned.angle_between(forward) - 30f32.to_radians()).abs() < 1e-4);
        assert!(turned.x > 0.0);
        let near = look_delta(forward, Vec3::new(0.1, 0.0, -1.0), 30f32.to_radians());
        assert!((near * forward).angle_between(Vec3::new(0.1, 0.0, -1.0)) < 1e-4);
    }

    #[test]
    fn a_head_turns_toward_its_target_within_the_limit_and_eases_in() {
        use crate::engine::InsertEngine;
        use blockloom_core::animation::AnimationSpec;
        use blockloom_core::components::ActorComponent;
        use blockloom_core::project::Actor;
        use blockloom_core::scene::{Mode, Visual};
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        let mut actor = Actor::new(
            "Hero",
            Visual::Cuboid {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0, 1.0],
            },
        );
        let spec: AnimationSpec =
            serde_json::from_str(r#"{"ik":{"head":"Head","look_limit":45}}"#).unwrap();
        actor
            .components
            .insert(ActorComponent::Animation { animation: spec });
        let id = actor.id.clone();
        engine
            .attached
            .insert(id.clone(), ["Animation".to_string()].into_iter().collect());
        engine.project.actors.push(actor);
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), bevy::time::TimePlugin));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.05),
        ));
        let head = app
            .world_mut()
            .spawn((Name::new("head"), Transform::from_xyz(0.0, 1.6, 0.0)))
            .id();
        let scene = app
            .world_mut()
            .spawn((ModelRig::empty(), Transform::default()))
            .id();
        app.world_mut().entity_mut(scene).add_child(head);
        let hero = app
            .world_mut()
            .spawn((
                ActorId(id.clone()),
                ModelChild(scene),
                Transform::default(),
                LookTarget::Point(Vec3::new(10.0, 1.6, 0.0)),
            ))
            .id();
        app.world_mut().entity_mut(hero).add_child(scene);
        engine.entities.insert(id, hero);
        app.insert_engine(engine);
        // The animation would put the head back each frame; here it stays.
        let reset = move |mut transforms: Query<&mut Transform>| {
            transforms.get_mut(head).unwrap().rotation = Quat::IDENTITY;
        };
        app.add_systems(Update, (reset, solve_ik).chain());
        app.update();
        app.update();
        let turned = |app: &App| {
            let rotation = app.world().get::<Transform>(head).unwrap().rotation;
            (rotation * Vec3::NEG_Z)
                .angle_between(Vec3::NEG_Z)
                .to_degrees()
        };
        let easing = turned(&app);
        assert!(easing > 0.0 && easing < 45.0, "easing in: {easing}");
        for _ in 0..10 {
            app.update();
        }
        assert!(
            (turned(&app) - 45.0).abs() < 0.1,
            "held at the limit: {}",
            turned(&app)
        );
        let facing = app.world().get::<Transform>(head).unwrap().rotation * Vec3::NEG_Z;
        assert!(facing.x > 0.0, "toward +X: {facing}");
    }

    #[test]
    fn feet_land_on_the_ground_under_them_and_the_hips_drop_to_reach() {
        use crate::engine::InsertEngine;
        use blockloom_core::animation::AnimationSpec;
        use blockloom_core::components::ActorComponent;
        use blockloom_core::project::Actor;
        use blockloom_core::scene::{Mode, Visual};
        let (_sender, incoming) = std::sync::mpsc::channel();
        let mut engine = Engine::new(incoming, Mode::ThreeD);
        let mut actor = Actor::new(
            "Hero",
            Visual::Cuboid {
                color: "#FFFFFF".to_string(),
                size: [1.0, 1.0, 1.0],
            },
        );
        let spec: AnimationSpec = serde_json::from_str(
            r#"{"ik":{"feet":["Foot"],"hips":"Hips","foot_height":0.05,"foot_reach":0.5}}"#,
        )
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
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.05),
        ));
        // Ground whose top is at y = 0.
        app.world_mut().spawn((
            rp::Collider::cuboid(5.0, 0.5, 5.0),
            Transform::from_xyz(0.0, -0.5, 0.0),
        ));
        // A leg standing with its ankle 0.15 m up, knee bent forward.
        let rest = [
            ("Hips", Vec3::new(0.0, 1.0, 0.0)),
            ("Thigh", Vec3::ZERO),
            ("Shin", Vec3::new(0.0, -0.4, -0.05)),
            ("Foot", Vec3::new(0.0, -0.45, 0.05)),
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
            .spawn((ActorId(id.clone()), ModelChild(scene), Transform::default()))
            .id();
        app.world_mut().entity_mut(hero).add_child(scene);
        engine.entities.insert(id, hero);
        app.insert_engine(engine);
        let held = bones.clone();
        let reset = move |mut transforms: Query<&mut Transform>| {
            for (entity, (_, at)) in held.iter().zip(rest) {
                *transforms.get_mut(*entity).unwrap() = Transform::from_translation(at);
            }
        };
        app.add_systems(Update, (reset, solve_ik).chain());
        for _ in 0..12 {
            app.update();
        }
        let globals = |app: &mut App| {
            let mut state = app.world_mut().query::<(&Transform, Option<&ChildOf>)>();
            let world = app.world();
            let mut global = |mut entity: Entity| {
                let mut out = Transform::IDENTITY;
                loop {
                    let (local, parent) = state.get(world, entity).unwrap();
                    out = local.mul_transform(out);
                    match parent {
                        Some(parent) => entity = parent.parent(),
                        None => return out.translation,
                    }
                }
            };
            (global(bones[0]), global(bones[3]))
        };
        let (hips, ankle) = globals(&mut app);
        assert!((ankle.y - 0.05).abs() < 0.01, "ankle at {ankle}");
        assert!((hips.y - 0.9).abs() < 0.01, "hips at {hips}");
    }
}
