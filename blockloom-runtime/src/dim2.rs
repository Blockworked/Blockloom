//! The 2D half of the world: sprites, `bevy_rapier2d` bodies, and the effects
//! that only make sense with a 2D physics engine attached. A 2D unit is a
//! pixel, which is why gravity defaults to a few hundred of them.

use crate::engine::{Engine, PendingEffects};
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
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

/// A white disc on a transparent background, so a `Circle` draws round
/// through the same sprite pipeline every other 2D actor uses. The texture
/// is white so `Sprite::color` tints it exactly; the transparent corners are
/// what makes it read as a circle instead of a square.
fn circle_image(radius: f32) -> Image {
    let diameter = (radius * 2.0).ceil().max(2.0) as u32;
    let center = diameter as f32 / 2.0;
    let mut data = Vec::with_capacity((diameter * diameter * 4) as usize);
    for y in 0..diameter {
        for x in 0..diameter {
            // Half-pixel antialiased edge: fully opaque half a pixel inside
            // the radius, fading to transparent half a pixel outside it.
            let dist = Vec2::new(x as f32 + 0.5 - center, y as f32 + 0.5 - center).length();
            let alpha = (radius + 0.5 - dist).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: diameter,
            height: diameter,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

fn sprite_for(
    visual: &Visual,
    assets: &AssetServer,
    textures: &mut Assets<Image>,
) -> Option<Sprite> {
    match visual {
        Visual::Rect { color, size } => Some(Sprite {
            color: crate::world::parse_color(color),
            custom_size: Some(Vec2::new(size[0], size[1])),
            ..default()
        }),
        Visual::Circle { color, radius } => Some(Sprite {
            image: textures.add(circle_image(*radius)),
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
pub fn spawn_actor(
    commands: &mut Commands,
    actor: &Actor,
    assets: &AssetServer,
    textures: &mut Assets<Image>,
) -> Option<Entity> {
    let sprite = sprite_for(actor.visual()?, assets, textures)?;
    let mut entity = commands.spawn((crate::world::actor_bundle(actor), sprite));
    insert_body(&mut entity, actor);
    Some(entity.id())
}

fn insert_body(entity: &mut EntityCommands, actor: &Actor) {
    let physics = actor.physics();
    if let (Some(body), Some(collider)) = (
        body_for(physics.body),
        actor.visual().and_then(collider_for),
    ) {
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
    assets: Res<AssetServer>,
    mut textures: ResMut<Assets<Image>>,
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
            // A body or a look arriving or leaving mid-run needs this
            // dimension's own pipeline; `world::apply_component_effects`
            // owns everything else about the same effect.
            Effect::AttachComponent { actor, component } => {
                let Some(entity) = engine.entities.get(actor).copied() else {
                    continue;
                };
                match component.as_str() {
                    "Body" => {
                        if let Some(authored) = engine.project.actor(actor) {
                            insert_body(&mut commands.entity(entity), authored);
                        }
                    }
                    "Look" => {
                        let Some(sprite) = engine
                            .project
                            .actor(actor)
                            .and_then(|a| a.visual())
                            .and_then(|visual| sprite_for(visual, &assets, &mut textures))
                        else {
                            continue;
                        };
                        commands.entity(entity).insert(sprite);
                    }
                    _ => {}
                }
            }
            Effect::DetachComponent { actor, component } => {
                let Some(entity) = engine.entities.get(actor).copied() else {
                    continue;
                };
                let mut entity = commands.entity(entity);
                match component.as_str() {
                    "Body" => {
                        entity.remove::<rp::RigidBody>();
                        entity.remove::<rp::Collider>();
                    }
                    // Nothing to draw, but the actor is still there to be
                    // moved, sensed and given a look again.
                    "Look" => {
                        entity.remove::<Sprite>();
                    }
                    _ => {}
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
pub fn sync_pause(engine: NonSend<Engine>, mut configs: Query<&mut rp::RapierConfiguration>) {
    for mut config in &mut configs {
        config.physics_pipeline_active = engine.running && !engine.paused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_texture_is_a_disc_not_a_square() {
        let radius = 30.0;
        let image = circle_image(radius);
        let data = image.data.as_ref().expect("texture has pixel data");
        let d = (radius * 2.0).ceil() as usize;
        assert_eq!(data.len(), d * d * 4);
        let alpha_at = |x: usize, y: usize| data[(y * d + x) * 4 + 3];
        // The middle of the disc is fully opaque...
        assert_eq!(alpha_at(d / 2, d / 2), 255);
        // ...the middle of each edge is on the disc (up to antialiasing)...
        assert!(alpha_at(d / 2, 0) >= 250);
        assert!(alpha_at(d / 2, d - 1) >= 250);
        assert!(alpha_at(0, d / 2) >= 250);
        assert!(alpha_at(d - 1, d / 2) >= 250);
        // ...and the corners are fully transparent: the square is gone.
        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(d - 1, 0), 0);
        assert_eq!(alpha_at(0, d - 1), 0);
        assert_eq!(alpha_at(d - 1, d - 1), 0);
        // White throughout so `Sprite::color` tints the actor exactly.
        assert!(
            data.chunks_exact(4)
                .all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255)
        );
        assert_eq!(
            image.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb
        );
    }
}
