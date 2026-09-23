//! The 2D half of the world: sprites, `bevy_rapier2d` bodies, and the effects
//! that only make sense with a 2D physics engine attached. A 2D unit is a
//! pixel, which is why gravity defaults to a few hundred of them.

use crate::engine::{Engine, PendingEffects, PhysicsPose, PrevPose};
use crate::materials::{GraphMaterial2d, custom_quad_size};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Mesh2d;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use bevy_rapier2d::prelude as rp;
use blockloom_core::project::Actor;
use blockloom_core::scene::{BodyKind, Visual};
use blockloom_core::vm::Effect;
use std::collections::{HashMap, HashSet};
use std::path::Path;

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
        // A solid tilemap collides as its whole slab; a decorative one lets
        // bodies pass through.
        Visual::Tilemap { tilemap } if tilemap.solid => {
            let size = tilemap.size();
            Some(rp::Collider::cuboid(size[0] / 2.0, size[1] / 2.0))
        }
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
    dir: Option<&Path>,
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
            image: assets.load(crate::world::asset_path(dir, path)),
            custom_size: Some(Vec2::new(size[0], size[1])),
            ..default()
        }),
        _ => None,
    }
}

/// Spawns one actor, or nothing if its visual belongs to the other dimension
/// or a tilemap is empty. A custom-shaded actor renders on a quad through
/// the graph material; a tilemap through its own mesh; everything else
/// through the sprite pipeline.
#[allow(clippy::too_many_arguments)]
pub fn spawn_actor(
    commands: &mut Commands,
    actor: &Actor,
    dir: Option<&Path>,
    assets: &AssetServer,
    textures: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    graph_materials: &mut Assets<GraphMaterial2d>,
    tile_materials: &mut Assets<ColorMaterial>,
) -> Option<Entity> {
    let visual = actor.visual()?.clone();
    match &visual {
        Visual::Tilemap { tilemap } if tilemap.build_mesh().is_empty() => return None,
        _ if shader_of(actor).is_some() && custom_quad_size(&visual).is_none() => return None,
        _ if shader_of(actor).is_none() && sprite_for(&visual, dir, assets, textures).is_none() => {
            return None;
        }
        _ => {}
    }
    let mut entity = commands.spawn(crate::world::actor_bundle(actor));
    // The sort layer rides on z, leaving the authored depth alone. The
    // borrow ends with this block: the passes below take `commands` again.
    let id = {
        let (transform, pose, prev) = spawn_pose(actor);
        entity.insert((transform, pose, prev));
        entity.id()
    };
    match &visual {
        Visual::Tilemap { tilemap } => {
            crate::materials::spawn_tilemap_2d(
                commands,
                id,
                tilemap,
                dir,
                assets,
                meshes,
                tile_materials,
            );
        }
        _ if shader_of(actor).is_some() => {
            insert_graph(commands, id, actor, dir, assets, meshes, graph_materials);
        }
        _ => {
            let sprite = sprite_for(&visual, dir, assets, textures).expect("checked above");
            commands.entity(id).insert(sprite);
        }
    }
    insert_body(&mut commands.entity(id), actor);
    Some(id)
}

/// The spawn transform: placement plus the sort layer, so a higher layer
/// draws on top without touching the authored z.
fn spawn_pose(actor: &Actor) -> (Transform, PhysicsPose, PrevPose) {
    let mut transform = crate::world::transform_for(actor);
    transform.translation.z += actor.components.layer() as f32;
    (transform, PhysicsPose(transform), PrevPose(transform))
}

/// The actor's custom effect, if it carries a Material with a shader.
fn shader_of(actor: &Actor) -> Option<blockloom_core::material::GraphEffect> {
    actor
        .components
        .material()
        .and_then(|material| material.shader.clone())
}

/// Render a custom-shaded look on a quad through the graph material. The
/// image look keeps its texture under the effect; the circle look renders
/// its quad round.
#[allow(clippy::too_many_arguments)]
fn insert_graph(
    commands: &mut Commands,
    entity: Entity,
    actor: &Actor,
    dir: Option<&Path>,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    graph_materials: &mut Assets<GraphMaterial2d>,
) {
    let Some(effect) = shader_of(actor) else {
        return;
    };
    let visual = actor.visual().cloned();
    let size = visual
        .as_ref()
        .and_then(custom_quad_size)
        .unwrap_or(Vec2::new(64.0, 64.0));
    let tint = visual
        .as_ref()
        .and_then(|visual| visual.color())
        .map(crate::world::parse_color)
        .unwrap_or(Color::WHITE);
    let texture = match &visual {
        Some(Visual::Image { path, .. }) => Some(assets.load(crate::world::asset_path(dir, path))),
        _ => None,
    };
    let rounded = matches!(visual, Some(Visual::Circle { .. }));
    let secondary = crate::world::parse_color(&effect.color);
    let mesh = meshes.add(Mesh::from(bevy::math::primitives::Rectangle::new(
        size.x, size.y,
    )));
    commands.entity(entity).insert((
        Mesh2d(mesh),
        MeshMaterial2d(graph_materials.add(crate::materials::graph_material_2d(
            &effect, tint, secondary, texture, rounded,
        ))),
    ));
}

/// Drop whatever the look currently draws with: sprite, graph quad, tilemap
/// child, or any mix a mid-run attach sequence left behind.
fn remove_drawn(
    commands: &mut Commands,
    id: Entity,
    tiles: &Query<&crate::materials::TilemapMesh>,
) {
    commands.entity(id).remove::<Sprite>();
    commands.entity(id).remove::<Mesh2d>();
    commands
        .entity(id)
        .remove::<MeshMaterial2d<GraphMaterial2d>>();
    if let Ok(marker) = tiles.get(id) {
        commands.entity(marker.0).despawn();
        commands
            .entity(id)
            .remove::<crate::materials::TilemapMesh>();
    }
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
            mass_properties(physics),
            groups_for(physics.layer(), physics.collision_mask),
        ));
        if physics.trigger {
            entity.insert(rp::Sensor);
        }
        if physics.lock_rotation {
            entity.insert(rp::LockedAxes::ROTATION_LOCKED);
        }
    }
}

/// The rapier filter for one actor's layer and mask. Both the collision and
/// the solver halves, so a filtered pair neither reports nor pushes.
fn groups_for(layer: u8, mask: u8) -> (rp::CollisionGroups, rp::SolverGroups) {
    let layer = layer.clamp(1, 8);
    let memberships = rp::Group::from_bits(1 << (layer - 1)).unwrap_or(rp::Group::ALL);
    let filters = rp::Group::from_bits(mask as u32).unwrap_or(rp::Group::ALL);
    (
        rp::CollisionGroups::new(memberships, filters),
        rp::SolverGroups::new(memberships, filters),
    )
}

/// Applies a live filter change to an entity: the sensor marker on or off,
/// and both group halves rewritten.
fn apply_filter(entity: &mut EntityCommands, layer: u8, mask: u8, trigger: bool) {
    let (collision, solver) = groups_for(layer, mask);
    entity.insert((collision, solver));
    if trigger {
        entity.insert(rp::Sensor);
    } else {
        entity.remove::<rp::Sensor>();
    }
}

/// How heavy the collider is: an explicit mass wins over the density its
/// shape would otherwise imply.
fn mass_properties(physics: blockloom_core::scene::Physics) -> rp::ColliderMassProperties {
    match physics.mass {
        Some(mass) => rp::ColliderMassProperties::Mass(mass),
        None => rp::ColliderMassProperties::Density(physics.density),
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
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    mut bodies: Query<(&mut rp::Velocity, &mut rp::ExternalImpulse)>,
    mut transforms: Query<&mut Transform>,
    mut config: Query<&mut rp::RapierConfiguration>,
    mut sprites: Query<&mut Sprite>,
    assets: Res<AssetServer>,
    mut textures: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut graph_materials: ResMut<Assets<GraphMaterial2d>>,
    mut tile_materials: ResMut<Assets<ColorMaterial>>,
    tiles: Query<&crate::materials::TilemapMesh>,
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
    // Turning "this far this step" into a velocity needs the step's length -
    // a fixed rate keeps it constant, whatever the display does.
    let dt = time.delta_secs().max(1.0 / 1000.0);
    let dir = engine.project_dir.clone();
    // Dynamic actors a walk verb drives this tick. Whoever was driven last
    // tick but isn't now just let go: brake them below.
    let mut driven: HashSet<String> = HashSet::new();
    // Each walked actor's composed tick total; see `dim3` for why one
    // tick's walks share one velocity write.
    let mut walks: HashMap<String, (Vec3, [bool; 3])> = HashMap::new();
    for effect in &effects.0 {
        let actor = match effect {
            Effect::Move { actor, .. } | Effect::ChangePosition { actor, .. } => actor,
            _ => continue,
        };
        if crate::world::is_dynamic(&engine, actor) {
            driven.insert(actor.clone());
        }
    }
    for effect in &effects.0 {
        match effect {
            // A dynamic body's turns land here rather than in
            // `world::apply_common`, in effect order with the deferred
            // `move` below - same sandwich reasoning as `dim3`.
            Effect::Turn {
                actor,
                axis,
                degrees,
            } => {
                // Only Z is a rotation in 2D; X and Y stay ignored, exactly
                // as `world::apply_common` treats them.
                if axis.index() != 2 {
                    continue;
                }
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                if !crate::world::is_dynamic(&engine, actor) {
                    continue;
                }
                if let Ok(mut transform) = transforms.get_mut(*entity) {
                    crate::world::turn_2d(&mut transform, degrees.to_radians());
                }
            }
            // A dynamic body walks by velocity, not by teleporting: that keeps
            // the solver able to resolve contacts, so it stops at walls and
            // rests on floors instead of passing through them. One tick's
            // walks compose per actor (see `dim3`); only the axes a real
            // walk names are written, so an upright actor's `move` leaves
            // the falling to gravity.
            Effect::Move { actor, steps } => {
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                if !crate::world::is_dynamic(&engine, actor) {
                    continue;
                }
                let Ok(transform) = transforms.get_mut(*entity) else {
                    continue;
                };
                let forward =
                    crate::world::forward_of(&transform, blockloom_core::scene::Mode::TwoD);
                let walk = walks
                    .entry(actor.clone())
                    .or_insert((Vec3::ZERO, [false; 3]));
                for axis in 0..2 {
                    let part = forward.truncate()[axis] * *steps;
                    walk.0[axis] += part;
                    walk.1[axis] |= part.abs() > 1e-5;
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
                if let Some((mut velocity, _)) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| bodies.get_mut(*entity).ok())
                {
                    velocity.linear += Vec2::new(impulse[0], impulse[1]);
                }
            }
            Effect::SetGravity { gravity } => {
                if let Ok(mut config) = config.single_mut() {
                    set_gravity(&mut config, *gravity);
                }
            }
            // `bevy_rapier` watches this component: a change recomputes the
            // rigid body's mass.
            Effect::SetDensity { actor, density } => {
                if let Some(entity) = engine.entities.get(actor).copied() {
                    commands
                        .entity(entity)
                        .insert(rp::ColliderMassProperties::Density(*density));
                }
            }
            Effect::SetMass { actor, mass } => {
                if let Some(entity) = engine.entities.get(actor).copied() {
                    commands
                        .entity(entity)
                        .insert(rp::ColliderMassProperties::Mass(*mass));
                }
            }
            Effect::SetColor { actor, color } => {
                // Sprites only: a custom-shaded actor keeps its graph tint,
                // which the material owns rather than the color block.
                if let Some(mut sprite) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| sprites.get_mut(*entity).ok())
                {
                    sprite.color = crate::world::parse_color(color);
                }
            }
            Effect::SetBody { actor, body } => {
                let Some(id) = engine.entities.get(actor).copied() else {
                    continue;
                };
                let Some(visual) = engine.actor(actor).and_then(|a| a.visual()).cloned() else {
                    continue;
                };
                let (layer, mask, trigger) = engine.filter_of(actor);
                let (collision, solver) = groups_for(layer, mask);
                let mut entity = commands.entity(id);
                match (body_for(*body), collider_for(&visual)) {
                    (Some(rigid_body), Some(collider)) => {
                        entity.insert((
                            rigid_body,
                            collider,
                            rp::ActiveEvents::COLLISION_EVENTS,
                            collision,
                            solver,
                        ));
                        if trigger {
                            entity.insert(rp::Sensor);
                        } else {
                            entity.remove::<rp::Sensor>();
                        }
                        // Rebase the pose slider so a fresh body starts
                        // interpolating from where it actually is.
                        if let Ok(transform) = transforms.get(id) {
                            entity.insert((PhysicsPose(*transform), PrevPose(*transform)));
                        }
                    }
                    _ => {
                        entity.remove::<rp::RigidBody>();
                        entity.remove::<rp::Collider>();
                    }
                }
            }
            Effect::SetTrigger { actor, trigger } => {
                let Some(id) = engine.entities.get(actor).copied() else {
                    continue;
                };
                engine.set_filter(actor, None, None, Some(*trigger));
                let (layer, mask, trigger) = engine.filter_of(actor);
                apply_filter(&mut commands.entity(id), layer, mask, trigger);
            }
            Effect::SetCollisionLayer { actor, layer } => {
                let Some(id) = engine.entities.get(actor).copied() else {
                    continue;
                };
                engine.set_filter(actor, Some(*layer), None, None);
                let (layer, mask, trigger) = engine.filter_of(actor);
                apply_filter(&mut commands.entity(id), layer, mask, trigger);
            }
            Effect::SetCollisionMask { actor, mask } => {
                let Some(id) = engine.entities.get(actor).copied() else {
                    continue;
                };
                engine.set_filter(actor, None, Some(*mask), None);
                let (layer, mask, trigger) = engine.filter_of(actor);
                apply_filter(&mut commands.entity(id), layer, mask, trigger);
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
                        if engine.actor(actor).is_some() {
                            let (layer, mask, trigger) = engine.filter_of(actor);
                            // insert_body reads the authored document; the
                            // live filter is re-applied after so a mid-run
                            // trigger/layer change survives a re-attach.
                            if let Some(authored) = engine.actor(actor).cloned() {
                                insert_body(&mut commands.entity(entity), &authored);
                            }
                            apply_filter(&mut commands.entity(entity), layer, mask, trigger);
                        }
                    }
                    "Look" => {
                        let Some(authored) = engine.actor(actor).cloned() else {
                            continue;
                        };
                        remove_drawn(&mut commands, entity, &tiles);
                        match authored.visual() {
                            Some(Visual::Tilemap { tilemap }) => {
                                crate::materials::spawn_tilemap_2d(
                                    &mut commands,
                                    entity,
                                    tilemap,
                                    dir.as_deref(),
                                    &assets,
                                    &mut meshes,
                                    &mut tile_materials,
                                );
                            }
                            Some(_) if shader_of(&authored).is_some() => {
                                insert_graph(
                                    &mut commands,
                                    entity,
                                    &authored,
                                    dir.as_deref(),
                                    &assets,
                                    &mut meshes,
                                    &mut graph_materials,
                                );
                            }
                            Some(visual) => {
                                if let Some(sprite) =
                                    sprite_for(visual, dir.as_deref(), &assets, &mut textures)
                                {
                                    commands.entity(entity).insert(sprite);
                                }
                            }
                            None => {}
                        }
                    }
                    "Material" => {
                        // A custom shader swaps the sprite for a graph quad;
                        // plain PBR props need lighting, so 2D leaves the
                        // sprite exactly as it is.
                        let Some(authored) = engine.actor(actor).cloned() else {
                            continue;
                        };
                        remove_drawn(&mut commands, entity, &tiles);
                        if shader_of(&authored).is_some() {
                            insert_graph(
                                &mut commands,
                                entity,
                                &authored,
                                dir.as_deref(),
                                &assets,
                                &mut meshes,
                                &mut graph_materials,
                            );
                        } else if let Some(visual) = authored.visual() {
                            // A tilemap re-spawns its mesh; anything else its
                            // sprite. Whatever the look draws with, the
                            // material attach rebuilds it from scratch.
                            match visual {
                                Visual::Tilemap { tilemap } => {
                                    crate::materials::spawn_tilemap_2d(
                                        &mut commands,
                                        entity,
                                        tilemap,
                                        dir.as_deref(),
                                        &assets,
                                        &mut meshes,
                                        &mut tile_materials,
                                    );
                                }
                                _ => {
                                    if let Some(sprite) =
                                        sprite_for(visual, dir.as_deref(), &assets, &mut textures)
                                    {
                                        commands.entity(entity).insert(sprite);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Effect::DetachComponent { actor, component } => {
                let Some(id) = engine.entities.get(actor).copied() else {
                    continue;
                };
                match component.as_str() {
                    "Body" => {
                        let mut entity = commands.entity(id);
                        entity.remove::<rp::RigidBody>();
                        entity.remove::<rp::Collider>();
                    }
                    // Nothing to draw, but the actor is still there to be
                    // moved, sensed and given a look again.
                    "Look" => {
                        remove_drawn(&mut commands, id, &tiles);
                    }
                    // Back to the plain sprite pipeline.
                    "Material" => {
                        remove_drawn(&mut commands, id, &tiles);
                        match engine.actor(actor).and_then(|a| a.visual()) {
                            Some(Visual::Tilemap { tilemap }) => {
                                crate::materials::spawn_tilemap_2d(
                                    &mut commands,
                                    id,
                                    tilemap,
                                    dir.as_deref(),
                                    &assets,
                                    &mut meshes,
                                    &mut tile_materials,
                                );
                            }
                            Some(visual) => {
                                if let Some(sprite) =
                                    sprite_for(visual, dir.as_deref(), &assets, &mut textures)
                                {
                                    commands.entity(id).insert(sprite);
                                }
                            }
                            None => {}
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    for (actor, (total, named)) in &walks {
        let Some(entity) = engine.entities.get(actor) else {
            continue;
        };
        if let Ok((mut velocity, _)) = bodies.get_mut(*entity) {
            for (axis, named) in named.iter().enumerate().take(2) {
                if *named {
                    velocity.linear[axis] = total[axis] / dt;
                }
            }
        }
    }
    // Same release-braking as `dim3`: a walk sets an absolute velocity, so
    // brake whoever went quiet, every axis but the one gravity pulls along.
    let gravity = config
        .single()
        .map(|config| Vec3::new(config.gravity.x, config.gravity.y, 0.0))
        .unwrap_or(Vec3::NEG_Y * 9.81);
    let brakes = crate::world::stop_axes(gravity);
    let previous = std::mem::replace(&mut engine.driven, driven);
    for id in &previous {
        if engine.driven.contains(id) {
            continue;
        }
        let Some(entity) = engine.entities.get(id) else {
            continue;
        };
        if !crate::world::is_dynamic(&engine, id) {
            continue;
        }
        if let Ok((mut velocity, _)) = bodies.get_mut(*entity) {
            for (axis, brake) in brakes.iter().enumerate().take(2) {
                if *brake {
                    velocity.linear[axis] = 0.0;
                }
            }
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
/// and velocities don't integrate. Runs before the rapier systems of the same
/// `FixedUpdate` pass, so it takes effect the same step.
pub fn sync_pause(engine: NonSend<Engine>, mut configs: Query<&mut rp::RapierConfiguration>) {
    for mut config in &mut configs {
        config.physics_pipeline_active = engine.running && !engine.paused;
    }
}

/// Steps the physics pipeline at the project's own fixed rate, the same rate
/// the blocks run at, so a body and a `move` never fight over time.
pub fn sync_timestep(engine: NonSend<Engine>, mut timestep: ResMut<rp::TimestepMode>) {
    let rate = engine.project.world.fixed_rate;
    if !rate.is_finite() {
        return;
    }
    let rate = rate.clamp(1.0, 1000.0);
    *timestep = rp::TimestepMode::Fixed {
        dt: 1.0 / rate,
        substeps: 1,
    };
}

/// Shifts each actor's pose slider forward at the end of a fixed step. Runs in
/// `FixedPostUpdate`, just after rapier's own writeback, so `PhysicsPose` is
/// exactly where the actor settled - physics bodies at the pose physics wrote,
/// and everyone else at the pose the step's effects pushed it to.
pub fn record_poses(mut posed: Query<(&Transform, &mut PhysicsPose, &mut PrevPose)>) {
    for (transform, mut current, mut previous) in &mut posed {
        previous.0 = current.0;
        current.0 = *transform;
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
