//! The 3D half of the world: meshes, materials, `bevy_rapier3d` bodies, and
//! the effects that need a 3D physics engine attached. A 3D unit is a metre.

use crate::engine::{Engine, PendingEffects};
use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_core::project::Actor;
use blockloom_core::scene::{BodyKind, Visual};
use blockloom_core::vm::Effect;

/// How thick a `plane` actor is made, since a real half-space can't be moved
/// or clicked the way every other actor can.
const PLANE_THICKNESS: f32 = 0.2;

fn collider_for(visual: &Visual) -> Option<rp::Collider> {
    match visual {
        Visual::Cuboid { size, .. } => Some(rp::Collider::cuboid(
            size[0] / 2.0,
            size[1] / 2.0,
            size[2] / 2.0,
        )),
        Visual::Sphere { radius, .. } => Some(rp::Collider::ball(*radius)),
        Visual::Capsule { radius, height, .. } => {
            Some(rp::Collider::capsule_y(height / 2.0, *radius))
        }
        Visual::Plane { size, .. } => Some(rp::Collider::cuboid(
            size[0] / 2.0,
            PLANE_THICKNESS / 2.0,
            size[1] / 2.0,
        )),
        _ => None,
    }
}

fn mesh_for(visual: &Visual) -> Option<Mesh> {
    Some(match visual {
        Visual::Cuboid { size, .. } => Cuboid::new(size[0], size[1], size[2]).into(),
        Visual::Sphere { radius, .. } => Sphere::new(*radius).into(),
        Visual::Capsule { radius, height, .. } => Capsule3d::new(*radius, *height).into(),
        Visual::Plane { size, .. } => Cuboid::new(size[0], PLANE_THICKNESS, size[1]).into(),
        _ => return None,
    })
}

/// Spawns one actor, or nothing if its visual belongs to the other dimension.
pub fn spawn_actor(
    commands: &mut Commands,
    actor: &Actor,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Entity> {
    let visual = actor.visual()?;
    let mesh = mesh_for(visual)?;
    let color = visual
        .color()
        .map(crate::world::parse_color)
        .unwrap_or(Color::WHITE);
    let physics = actor.physics();
    let mut entity = commands.spawn((
        crate::world::actor_bundle(actor),
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.6,
            ..default()
        })),
    ));
    if let (Some(body), Some(collider)) = (body_for(physics.body), collider_for(visual)) {
        entity.insert((
            body,
            collider,
            rp::ActiveEvents::COLLISION_EVENTS,
            rp::Velocity::zero(),
            rp::ExternalImpulse::default(),
            rp::GravityScale(physics.gravity_scale),
            rp::Restitution::coefficient(physics.restitution),
            rp::Friction::coefficient(physics.friction),
        ));
        if physics.lock_rotation {
            entity.insert(rp::LockedAxes::ROTATION_LOCKED);
        }
    }
    Some(entity.id())
}

fn body_for(body: BodyKind) -> Option<rp::RigidBody> {
    match body {
        BodyKind::None => None,
        BodyKind::Static => Some(rp::RigidBody::Fixed),
        BodyKind::Dynamic => Some(rp::RigidBody::Dynamic),
        BodyKind::Kinematic => Some(rp::RigidBody::KinematicPositionBased),
    }
}

pub fn set_gravity(config: &mut rp::RapierConfiguration, gravity: [f32; 3]) {
    config.gravity = Vec3::new(gravity[0], gravity[1], gravity[2]);
}

/// The effects that need 3D physics or a material - everything else is handled
/// once, dimension-agnostically, in `world::apply_common`.
pub fn apply_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
    time: Res<Time>,
    mut bodies: Query<(&mut rp::Velocity, &mut rp::ExternalImpulse)>,
    transforms: Query<&Transform>,
    mut config: Query<&mut rp::RapierConfiguration>,
    surfaces: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !engine.running || engine.paused {
        // Still apply gravity while idle so the config is correct on Play.
        for effect in &effects.0 {
            if let Effect::SetGravity { gravity } = effect
                && let Ok(mut config) = config.single_mut()
            {
                set_gravity(&mut config, *gravity);
            }
        }
        return;
    }
    let dt = time.delta_secs().max(1.0 / 240.0);
    for effect in &effects.0 {
        match effect {
            // See `dim2::apply_effects` for why a dynamic body moves this way.
            Effect::Move { actor, steps } => {
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                if !crate::world::is_dynamic(&engine, actor) {
                    continue;
                }
                let Ok(transform) = transforms.get(*entity) else {
                    continue;
                };
                let forward =
                    crate::world::forward_of(transform, blockloom_core::scene::Mode::ThreeD);
                if let Ok((mut velocity, _)) = bodies.get_mut(*entity) {
                    let step = forward * *steps / dt;
                    for axis in 0..3 {
                        if step[axis].abs() > f32::EPSILON {
                            velocity.linear[axis] = step[axis];
                        }
                    }
                }
            }
            Effect::ChangePosition { actor, axis, by } => {
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                if !crate::world::is_dynamic(&engine, actor) {
                    continue;
                }
                if let Ok((mut velocity, _)) = bodies.get_mut(*entity) {
                    velocity.linear[axis.index()] = *by / dt;
                }
            }
            Effect::SetVelocity { actor, velocity } => {
                if let Some((mut current, _)) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| bodies.get_mut(*entity).ok())
                {
                    current.linear = Vec3::new(velocity[0], velocity[1], velocity[2]);
                }
            }
            Effect::ApplyImpulse { actor, impulse } => {
                if let Some((_, mut external)) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| bodies.get_mut(*entity).ok())
                {
                    external.impulse = Vec3::new(impulse[0], impulse[1], impulse[2]);
                }
            }
            Effect::SetGravity { gravity } => {
                if let Ok(mut config) = config.single_mut() {
                    set_gravity(&mut config, *gravity);
                }
            }
            Effect::SetColor { actor, color } => {
                if let Some(mut material) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| surfaces.get(*entity).ok())
                    .and_then(|handle| materials.get_mut(&handle.0))
                {
                    material.base_color = crate::world::parse_color(color);
                }
            }
            Effect::SetBody { actor, body } => {
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                let Some(visual) = engine
                    .project
                    .actor(actor)
                    .and_then(|a| a.visual())
                    .cloned()
                else {
                    continue;
                };
                let mut entity = commands.entity(*entity);
                match (body_for(*body), collider_for(&visual)) {
                    (Some(rigid_body), Some(collider)) => {
                        entity.insert((rigid_body, collider, rp::ActiveEvents::COLLISION_EVENTS));
                    }
                    _ => {
                        entity.remove::<rp::RigidBody>();
                        entity.remove::<rp::Collider>();
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn relay_collisions(
    mut messages: MessageReader<rp::CollisionEvent>,
    mut engine: NonSendMut<Engine>,
) {
    if !engine.running || engine.paused {
        messages.clear();
        return;
    }
    for message in messages.read() {
        let (a, b, started) = match message {
            rp::CollisionEvent::Started(a, b, _) => (*a, *b, true),
            rp::CollisionEvent::Stopped(a, b, _) => (*a, *b, false),
        };
        crate::world::note_contact(&mut engine, a, b, started);
    }
}

/// Freezes the physics pipeline while paused or stopped so bodies stop falling
/// and velocities don't integrate. Runs in `Update` before rapier's own
/// `PostUpdate` step, so it takes effect the same frame.
pub fn sync_pause(engine: NonSend<Engine>, mut configs: Query<&mut rp::RapierConfiguration>) {
    for mut config in &mut configs {
        config.physics_pipeline_active = engine.running && !engine.paused;
    }
}

/// A light and a camera, so a fresh 3D project isn't a black window.
pub fn spawn_scenery(commands: &mut Commands, camera: &blockloom_core::scene::Camera) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(camera.position[0], camera.position[1], camera.position[2]).looking_at(
            Vec3::new(camera.look_at[0], camera.look_at[1], camera.look_at[2]),
            Vec3::Y,
        ),
        crate::world::WorldCamera,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(8.0, 16.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
