//! The 2D half of the world: sprites, `bevy_rapier2d` bodies, and the effects
//! that only make sense with a 2D physics engine attached. A 2D unit is a
//! pixel, which is why gravity defaults to a few hundred of them.

use crate::engine::{ActorId, Engine, PendingEffects};
use bevy::prelude::*;
use bevy_rapier2d::prelude as rp;
use blockloom_core::project::Actor;
use blockloom_core::scene::{BodyKind, Visual};
use blockloom_core::vm::Effect;

/// How many pixels make a physics metre - the scale rapier reasons about
/// masses and forces in.
pub const PIXELS_PER_METER: f32 = 100.0;

/// The collider a visual implies, or `None` for a 3D visual in a 2D project.
fn collider_for(visual: &Visual) -> Option<rp::Collider> {
    match visual {
        Visual::Rect { size, .. } | Visual::Image { size, .. } => {
            Some(rp::Collider::cuboid(size[0] / 2.0, size[1] / 2.0))
        }
        Visual::Circle { radius, .. } => Some(rp::Collider::ball(*radius)),
        _ => None,
    }
}

fn sprite_for(visual: &Visual, assets: &AssetServer) -> Option<Sprite> {
    match visual {
        Visual::Rect { color, size } => Some(Sprite {
            color: crate::world::parse_color(color),
            custom_size: Some(Vec2::new(size[0], size[1])),
            ..default()
        }),
        // A circle is drawn as a square sprite for now; its collider is a
        // proper ball, so physics still behaves round.
        Visual::Circle { color, radius } => Some(Sprite {
            color: crate::world::parse_color(color),
            custom_size: Some(Vec2::splat(radius * 2.0)),
            ..default()
        }),
        Visual::Image { path, size } => Some(Sprite {
            image: assets.load(path.clone()),
            custom_size: Some(Vec2::new(size[0], size[1])),
            ..default()
        }),
        _ => None,
    }
}

/// Spawns one actor, or nothing if its visual belongs to the other dimension.
pub fn spawn_actor(commands: &mut Commands, actor: &Actor, assets: &AssetServer) -> Option<Entity> {
    let sprite = sprite_for(&actor.visual, assets)?;
    let mut entity = commands.spawn((
        Name::new(actor.name.clone()),
        ActorId(actor.id.clone()),
        sprite,
        crate::world::transform_for(actor),
        crate::world::visibility_for(actor),
    ));
    if let (Some(body), Some(collider)) =
        (body_for(actor.physics.body), collider_for(&actor.visual))
    {
        entity.insert((
            body,
            collider,
            rp::ActiveEvents::COLLISION_EVENTS,
            rp::Velocity::zero(),
            rp::ExternalImpulse::default(),
            rp::GravityScale(actor.physics.gravity_scale),
            rp::Restitution::coefficient(actor.physics.restitution),
            rp::Friction::coefficient(actor.physics.friction),
        ));
        if actor.physics.lock_rotation {
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

/// Applies the world's gravity setting to the physics pipeline.
pub fn set_gravity(config: &mut rp::RapierConfiguration, gravity: [f32; 3]) {
    config.gravity = Vec2::new(gravity[0], gravity[1]);
}

/// The effects that need 2D physics or a sprite - everything else is handled
/// once, dimension-agnostically, in `world::apply_common`.
pub fn apply_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
    time: Res<Time>,
    mut bodies: Query<(&mut rp::Velocity, &mut rp::ExternalImpulse)>,
    transforms: Query<&Transform>,
    mut config: Query<&mut rp::RapierConfiguration>,
    mut sprites: Query<&mut Sprite>,
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
    // Turning "this far this frame" into a velocity needs the frame's own
    // length; a stalled frame would otherwise read as an enormous speed.
    let dt = time.delta_secs().max(1.0 / 240.0);
    for effect in &effects.0 {
        match effect {
            // A dynamic body walks by velocity, not by teleporting: that keeps
            // the solver able to resolve contacts, so it stops at walls and
            // rests on floors instead of passing through them. Only the axes
            // the block actually names are written, so an upright actor's
            // `move` leaves the falling to gravity.
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
                    crate::world::forward_of(transform, blockloom_core::scene::Mode::TwoD);
                if let Ok((mut velocity, _)) = bodies.get_mut(*entity) {
                    let step = forward.truncate() * *steps / dt;
                    if step.x.abs() > f32::EPSILON {
                        velocity.linear.x = step.x;
                    }
                    if step.y.abs() > f32::EPSILON {
                        velocity.linear.y = step.y;
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
                let Some(index) =
                    crate::world::position_axis(blockloom_core::scene::Mode::TwoD, *axis)
                else {
                    continue;
                };
                if let Ok((mut velocity, _)) = bodies.get_mut(*entity)
                    && index < 2
                {
                    velocity.linear[index] = *by / dt;
                }
            }
            Effect::SetVelocity { actor, velocity } => {
                if let Some((mut current, _)) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| bodies.get_mut(*entity).ok())
                {
                    current.linear = Vec2::new(velocity[0], velocity[1]);
                }
            }
            Effect::ApplyImpulse { actor, impulse } => {
                if let Some((_, mut external)) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| bodies.get_mut(*entity).ok())
                {
                    external.impulse = Vec2::new(impulse[0], impulse[1]);
                }
            }
            Effect::SetGravity { gravity } => {
                if let Ok(mut config) = config.single_mut() {
                    set_gravity(&mut config, *gravity);
                }
            }
            Effect::SetColor { actor, color } => {
                if let Some(mut sprite) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| sprites.get_mut(*entity).ok())
                {
                    sprite.color = crate::world::parse_color(color);
                }
            }
            Effect::SetBody { actor, body } => {
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                let Some(visual) = engine.project.actor(actor).map(|a| a.visual.clone()) else {
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

/// Turns rapier's contact messages into `when I touch` triggers, and keeps the
/// `touching?` reporter's answer up to date. Skipped while paused or stopped
/// so a frozen world doesn't queue new collision strands.
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
pub fn sync_pause(
    engine: NonSend<Engine>,
    mut configs: Query<&mut rp::RapierConfiguration>,
) {
    for mut config in &mut configs {
        config.physics_pipeline_active = engine.running && !engine.paused;
    }
}
