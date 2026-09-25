//! The 3D half of the world: meshes, materials, `bevy_rapier3d` bodies, and
//! the effects that need a 3D physics engine attached. A 3D unit is a metre.

use crate::engine::{Engine, PendingEffects, PhysicsPose, PrevPose};
use crate::materials::GraphMaterial3d;
use bevy::ecs::system::EntityCommands;
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::prelude::*;
use bevy::render::view::Msaa;
use bevy_rapier3d::prelude as rp;
use blockloom_core::project::Actor;
use blockloom_core::scene::{BodyKind, Visual};
use blockloom_core::vm::Effect;
use std::collections::{HashMap, HashSet};
use std::path::Path;

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
        // A rig collides as its authored box; the drawn mesh swaps in.
        Visual::Model { scale, .. } => Some(rp::Collider::cuboid(
            scale[0] / 2.0,
            scale[1] / 2.0,
            scale[2] / 2.0,
        )),
        // A solid tilemap collides tile by tile, one slab-thick box per
        // merged run; a decorative one lets bodies pass through.
        Visual::Tilemap { tilemap } => {
            let parts: Vec<_> = tilemap
                .solid_rects()
                .into_iter()
                .map(|rect| {
                    (
                        Vec3::new(rect.center[0], rect.center[1], 0.0),
                        Quat::IDENTITY,
                        rp::Collider::cuboid(rect.half[0], rect.half[1], PLANE_THICKNESS / 2.0),
                    )
                })
                .collect();
            (!parts.is_empty()).then(|| rp::Collider::compound(parts))
        }
        _ => None,
    }
}

fn mesh_for(visual: &Visual) -> Option<Mesh> {
    match visual {
        Visual::Cuboid { size, .. } => Some(Cuboid::new(size[0], size[1], size[2]).into()),
        Visual::Sphere { radius, .. } => Some(Sphere::new(*radius).into()),
        Visual::Capsule { radius, height, .. } => Some(Capsule3d::new(*radius, *height).into()),
        Visual::Plane { size, .. } => Some(Cuboid::new(size[0], PLANE_THICKNESS, size[1]).into()),
        // Stands in until the glTF scene streams in (see `model`): a tinted
        // box at the authored scale, so a missing rig is visible rather than
        // invisible.
        Visual::Model { scale, .. } => Some(Cuboid::new(scale[0], scale[1], scale[2]).into()),
        // An empty tilemap draws nothing: let the unseen fallback take it.
        Visual::Tilemap { tilemap } => {
            let built = tilemap.build_mesh();
            if built.is_empty() {
                None
            } else {
                Some(crate::materials::tilemesh_to_bevy(&built))
            }
        }
        _ => None,
    }
}

/// Spawns one actor, or nothing if its visual belongs to the other dimension
/// or a tilemap is empty. A custom-shaded actor renders through the graph
/// material; a tilemap through its textured mesh; a material-carrying actor
/// through its PBR properties; everything else through flat color.
#[allow(clippy::too_many_arguments)]
pub fn spawn_actor(
    commands: &mut Commands,
    actor: &Actor,
    dir: Option<&Path>,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    graph_materials: &mut Assets<GraphMaterial3d>,
) -> Option<Entity> {
    let visual = actor.visual()?.clone();
    let mesh = meshes.add(mesh_for(&visual)?);
    let id = commands
        .spawn((crate::world::actor_bundle(actor), Mesh3d(mesh.clone())))
        .id();
    insert_look_extras(commands, id, actor, &visual, &mesh, dir, assets);
    insert_surface(
        commands,
        id,
        actor,
        &visual,
        dir,
        assets,
        materials,
        graph_materials,
    );
    insert_body(&mut commands.entity(id), actor);
    Some(id)
}

/// What a look draws beyond its mesh and surface: a model's glTF scene, or
/// a tilemap's frame clock.
fn insert_look_extras(
    commands: &mut Commands,
    id: Entity,
    actor: &Actor,
    visual: &Visual,
    mesh: &Handle<Mesh>,
    dir: Option<&Path>,
    assets: &AssetServer,
) {
    match visual {
        Visual::Model { .. } => {
            crate::model::attach(commands, id, &actor.id, visual, dir, assets);
        }
        Visual::Tilemap { tilemap } => {
            if let Some(animated) = crate::materials::AnimatedTiles::of(tilemap, mesh) {
                commands.entity(id).insert(animated);
            }
        }
        _ => {}
    }
}

/// The actor's surface: graph effect, tilemap texture, or PBR properties.
#[allow(clippy::too_many_arguments)]
fn insert_surface(
    commands: &mut Commands,
    id: Entity,
    actor: &Actor,
    visual: &Visual,
    dir: Option<&Path>,
    assets: &AssetServer,
    materials: &mut Assets<StandardMaterial>,
    graph_materials: &mut Assets<GraphMaterial3d>,
) {
    let color = visual
        .color()
        .map(crate::world::parse_color)
        .unwrap_or(Color::WHITE);
    let material = actor.components.material();
    if let Some(effect) = material.and_then(|material| material.shader.as_ref()) {
        let secondary = crate::world::parse_color(&effect.color);
        let shader = crate::materials::surface_shader(commands, &actor.id, effect, dir, true);
        commands
            .entity(id)
            .insert(MeshMaterial3d(graph_materials.add(
                crate::materials::graph_material_3d(effect, color, secondary, None, shader),
            )));
        return;
    }
    if let Visual::Tilemap { tilemap } = visual {
        let texture = if tilemap.tileset.trim().is_empty() {
            None
        } else {
            Some(assets.load(crate::world::asset_path(dir, tilemap.tileset.trim())))
        };
        commands
            .entity(id)
            .insert(MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: texture,
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                ..default()
            })));
        return;
    }
    let standard = match material {
        Some(material) => crate::materials::surface_standard(material, color, dir, assets),
        None => StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.6,
            ..default()
        },
    };
    commands
        .entity(id)
        .insert(MeshMaterial3d(materials.add(standard)));
}

/// Drop whatever surface the actor renders with: standard or graph.
fn remove_surface(commands: &mut Commands, id: Entity) {
    commands
        .entity(id)
        .remove::<MeshMaterial3d<StandardMaterial>>();
    commands
        .entity(id)
        .remove::<MeshMaterial3d<GraphMaterial3d>>();
}

/// Gives an actor the rigid body its `Body` component asks for, with the
/// collider its look implies. Nothing happens without both.
fn insert_body(entity: &mut EntityCommands, actor: &Actor) {
    let physics = actor.physics();
    let Some(collider) = actor.visual().and_then(collider_for) else {
        return;
    };
    let Some(body) = body_for(physics.body) else {
        return;
    };
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

/// The rapier filter for one actor's layer and mask, on both halves so a
/// filtered pair neither reports nor pushes.
fn groups_for(layer: u8, mask: u8) -> (rp::CollisionGroups, rp::SolverGroups) {
    let layer = layer.clamp(1, 8);
    let memberships = rp::Group::from_bits(1 << (layer - 1)).unwrap_or(rp::Group::ALL);
    let filters = rp::Group::from_bits(mask as u32).unwrap_or(rp::Group::ALL);
    (
        rp::CollisionGroups::new(memberships, filters),
        rp::SolverGroups::new(memberships, filters),
    )
}

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

pub fn set_gravity(config: &mut rp::RapierConfiguration, gravity: [f32; 3]) {
    config.gravity = Vec3::new(gravity[0], gravity[1], gravity[2]);
}

/// The effects that need 3D physics or a material - everything else is handled
/// once, dimension-agnostically, in `world::apply_common`.
pub fn apply_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    mut engine: NonSendMut<Engine>,
    time: Res<Time>,
    mut bodies: Query<(&mut rp::Velocity, &mut rp::ExternalImpulse)>,
    mut transforms: Query<&mut Transform>,
    mut config: Query<&mut rp::RapierConfiguration>,
    surfaces: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<AssetServer>,
    mut graph_materials: ResMut<Assets<GraphMaterial3d>>,
    models: Query<&crate::model::ModelChild>,
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
    let dt = time.delta_secs().max(1.0 / 1000.0);
    let dir = engine.project_dir.clone();
    // Dynamic actors a walk verb drives this tick. Whoever was driven last
    // tick but isn't now just let go: brake them below.
    let mut driven: HashSet<String> = HashSet::new();
    // Each walked actor's composed tick total, per axis, with which axes a
    // real walk named. An axis only facing dust touched stays unnamed, so it
    // keeps its falling or cruising speed; an axis walks canceled out on
    // still writes zero, stopping instead of cruising stale.
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
            // `move` below. That keeps a turn-move-turn sandwich (how
            // strafe is spelled) turned while the move reads its facing;
            // turning in the common pass would net the sandwich to zero
            // first, walking both strafe keys forward.
            Effect::Turn {
                actor,
                axis,
                degrees,
            } => {
                let Some(entity) = engine.entities.get(actor) else {
                    continue;
                };
                if !crate::world::is_dynamic(&engine, actor) {
                    continue;
                }
                if let Ok(mut transform) = transforms.get_mut(*entity) {
                    crate::world::turn_3d(&mut transform, *axis, degrees.to_radians());
                }
            }
            // A dynamic body walks by velocity, not by teleporting: that keeps
            // the solver able to resolve contacts, so it stops at walls and
            // rests on floors instead of passing through them. One tick's
            // walks compose per actor into `walks`, written once below: each
            // move writing straight through lets float dust from a turned
            // facing clobber another move's axis, and W+A would walk purely
            // sideways.
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
                    crate::world::forward_of(&transform, blockloom_core::scene::Mode::ThreeD);
                let walk = walks
                    .entry(actor.clone())
                    .or_insert((Vec3::ZERO, [false; 3]));
                for axis in 0..3 {
                    let part = forward[axis] * *steps;
                    walk.0[axis] += part;
                    // Steps units, so no frame rate can move the line: a real
                    // walk is orders above it, facing dust orders below.
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
                if let Some((mut velocity, _)) = engine
                    .entities
                    .get(actor)
                    .and_then(|entity| bodies.get_mut(*entity).ok())
                {
                    velocity.linear += Vec3::new(impulse[0], impulse[1], impulse[2]);
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
                let Some(authored) = engine.actor(actor).cloned() else {
                    continue;
                };
                match component.as_str() {
                    "Body" => {
                        insert_body(&mut commands.entity(entity), &authored);
                        let (layer, mask, trigger) = engine.filter_of(actor);
                        apply_filter(&mut commands.entity(entity), layer, mask, trigger);
                    }
                    "Look" => {
                        let Some(visual) = authored.visual().cloned() else {
                            continue;
                        };
                        let Some(mesh) = mesh_for(&visual) else {
                            continue;
                        };
                        let mesh = meshes.add(mesh);
                        commands.entity(entity).insert(Mesh3d(mesh.clone()));
                        crate::model::detach(&mut commands, entity, &models);
                        insert_look_extras(
                            &mut commands,
                            entity,
                            &authored,
                            &visual,
                            &mesh,
                            dir.as_deref(),
                            &assets,
                        );
                        insert_surface(
                            &mut commands,
                            entity,
                            &authored,
                            &visual,
                            dir.as_deref(),
                            &assets,
                            &mut materials,
                            &mut graph_materials,
                        );
                    }
                    "Material" => {
                        // Rebuild the surface from scratch: whatever it drew
                        // with before, it now draws with the authored look
                        // plus the authored material.
                        let Some(visual) = authored.visual().cloned() else {
                            continue;
                        };
                        if mesh_for(&visual).is_none() {
                            continue;
                        }
                        remove_surface(&mut commands, entity);
                        insert_surface(
                            &mut commands,
                            entity,
                            &authored,
                            &visual,
                            dir.as_deref(),
                            &assets,
                            &mut materials,
                            &mut graph_materials,
                        );
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
                        commands
                            .entity(id)
                            .remove::<(Mesh3d, crate::materials::AnimatedTiles)>();
                        remove_surface(&mut commands, id);
                        crate::model::detach(&mut commands, id, &models);
                    }
                    // Back to the plain standard surface.
                    "Material" => {
                        let Some(authored) = engine.actor(actor).cloned() else {
                            continue;
                        };
                        let Some(visual) = authored.visual().cloned() else {
                            continue;
                        };
                        remove_surface(&mut commands, id);
                        insert_surface(
                            &mut commands,
                            id,
                            &authored,
                            &visual,
                            dir.as_deref(),
                            &assets,
                            &mut materials,
                            &mut graph_materials,
                        );
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
            for (axis, named) in named.iter().enumerate() {
                if *named {
                    velocity.linear[axis] = total[axis] / dt;
                }
            }
        }
    }
    // A walk sets an absolute velocity, so the tick after the strand goes
    // quiet that speed would otherwise glide on. Brake every axis but the
    // one gravity pulls along: releasing a key stops the run, never a fall.
    // Only ever walk-driven actors are braked, so solver-driven motion - a
    // ball off a collision, an impulse, an explicit `set velocity` - keeps
    // its inertia.
    let gravity = config
        .single()
        .map(|config| config.gravity)
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
            for (axis, brake) in brakes.iter().enumerate() {
                if *brake {
                    velocity.linear[axis] = 0.0;
                }
            }
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

/// A light and a camera, so a fresh 3D project isn't a black window. The
/// light and the ambient come from the project's lighting settings; AO is a
/// component on the camera, so it is only there when the project asks for it.
/// Post-process rides the camera the same way: exposure and tonemapping
/// always, bloom and vignette only when enabled.
pub fn spawn_scenery(
    commands: &mut Commands,
    camera: &blockloom_core::scene::Camera,
    lighting: &blockloom_core::scene::Lighting,
    post: &blockloom_core::scene::PostProcess,
) {
    let mut camera_entity = commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 75.0_f32.to_radians(),
            ..default()
        }),
        Transform::from_xyz(camera.position[0], camera.position[1], camera.position[2]).looking_at(
            Vec3::new(camera.look_at[0], camera.look_at[1], camera.look_at[2]),
            Vec3::Y,
        ),
        crate::world::WorldCamera,
        // The one listener positional voices pan against. It rides the
        // camera, so what the player sees is what they hear.
        bevy::audio::SpatialListener::default(),
        bevy::camera::Exposure {
            ev100: post.exposure_ev,
        },
        crate::world::tonemapping_of(post.tonemapping),
    ));
    if lighting.ao_enabled {
        // SSAO needs multisampling off on the same camera, or `bevy_pbr`
        // logs a mismatch error and skips the effect.
        camera_entity.insert((ScreenSpaceAmbientOcclusion::default(), Msaa::Off));
    }
    if post.bloom_enabled {
        camera_entity.insert(crate::world::bloom_of(post));
    }
    if post.vignette_strength > 0.0 {
        camera_entity.insert(crate::world::vignette_of(post.vignette_strength));
    }
    // A zero direction has nowhere to point, so fall back to straight down.
    let dir = lighting.light_direction;
    let from = if dir.iter().all(|v| *v == 0.0) {
        Vec3::new(0.0, 16.0, 0.0)
    } else {
        Vec3::new(dir[0], dir[1], dir[2])
    };
    commands.spawn((
        DirectionalLight {
            color: crate::world::parse_color(&lighting.light_color),
            illuminance: lighting.illuminance.max(0.0),
            shadow_maps_enabled: true,
            shadow_depth_bias: lighting.shadow_bias,
            ..default()
        },
        Transform::from_translation(from).looking_at(Vec3::ZERO, Vec3::Y),
        crate::world::WorldLight,
    ));
    // Shadow map size is a resource, not a light field: one size for every
    // cascade. Powers of two only; anything else falls back to 2048.
    commands.insert_resource(bevy::light::DirectionalLightShadowMap {
        size: shadow_map_size(lighting.shadow_map_size),
    });
    commands.insert_resource(GlobalAmbientLight {
        color: crate::world::parse_color(&lighting.ambient_color),
        brightness: lighting.ambient_brightness.max(0.0),
        ..default()
    });
}

/// Snap a shadow map size to the powers of two Bevy accepts.
fn shadow_map_size(size: u32) -> usize {
    const SIZES: &[usize] = &[512, 1024, 2048, 4096, 8192];
    let wanted = size.max(512) as usize;
    SIZES
        .iter()
        .copied()
        .min_by_key(|candidate| candidate.abs_diff(wanted))
        .unwrap_or(2048)
}
