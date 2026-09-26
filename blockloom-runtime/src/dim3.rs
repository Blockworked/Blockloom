//! The 3D half of the world: meshes, materials, `bevy_rapier3d` bodies, and
//! the effects that need a 3D physics engine attached. A 3D unit is a metre.

use crate::engine::{Engine, PendingEffects, PhysicsPose, PrevPose};
use crate::materials::GraphMaterial3d;
use bevy::ecs::system::EntityCommands;
use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_core::components::JointKind;
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

pub(super) fn mesh_for(visual: &Visual) -> Option<Mesh> {
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

fn surface_mesh(
    actor: &Actor,
    visual: &Visual,
    cache: &mut crate::performance::RenderCache,
    meshes: &mut Assets<Mesh>,
) -> Option<Handle<Mesh>> {
    let Some(material) = actor.components.material() else {
        return cache.mesh(visual, meshes);
    };
    let revised = material.is_projected()
        || material.tiling != [1.0, 1.0]
        || material.offset != [0.0, 0.0]
        || material.rotation != 0.0
        || material.sampler != blockloom_core::material::TextureSampler::Clamp
        || !material.normal_texture.is_empty()
        || !material.roughness_texture.is_empty()
        || material.texel_density != 1.0;
    if !revised || !matches!(visual, Visual::Cuboid { .. } | Visual::Plane { .. }) {
        return cache.mesh(visual, meshes);
    }
    cache.scaled_mesh(
        visual,
        || {
            let mut mesh = mesh_for(visual)?;
            scale_primitive_uv(&mut mesh, visual, 1.0);
            Some(mesh)
        },
        meshes,
    )
}

fn scale_primitive_uv(mesh: &mut Mesh, visual: &Visual, density: f32) {
    let (
        Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)),
        Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)),
    ) = (
        mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
        mesh.attribute(Mesh::ATTRIBUTE_UV_0),
    )
    else {
        return;
    };
    let mut scaled = uvs.clone();
    let extent = match visual {
        Visual::Cuboid { size, .. } => *size,
        Visual::Plane { size, .. } => [size[0], PLANE_THICKNESS, size[1]],
        _ => unreachable!(),
    };
    for (uv, normal) in scaled.iter_mut().zip(normals) {
        let dimensions = if normal[0].abs() > 0.5 {
            [extent[2], extent[1]]
        } else if normal[1].abs() > 0.5 {
            [extent[0], extent[2]]
        } else {
            [extent[0], extent[1]]
        };
        uv[0] *= dimensions[0] * density;
        uv[1] *= dimensions[1] * density;
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, scaled);
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
    cache: &mut crate::performance::RenderCache,
) -> Option<Entity> {
    let visual = actor.visual()?.clone();
    let mesh = surface_mesh(actor, &visual, cache, meshes)?;
    let id = commands
        .spawn((crate::world::actor_bundle(actor), Mesh3d(mesh.clone())))
        .id();
    if let Some(lod) = cache.lod(&visual, &mesh, meshes) {
        commands.entity(id).insert(lod);
    }
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
        cache,
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
    let solid = match visual {
        Visual::Cuboid { size, .. } => Some(Vec3::from(*size)),
        Visual::Plane { size, .. } => Some(Vec3::new(size[0], PLANE_THICKNESS, size[1])),
        _ => None,
    };
    if let Some(size) = solid {
        let aabb = bevy::camera::primitives::Aabb::from_min_max(-size * 0.5, size * 0.5);
        commands.entity(id).insert(crate::culling::Occluder(aabb));
    }
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
    cache: &mut crate::performance::RenderCache,
) {
    let color = visual
        .color()
        .map(crate::world::parse_color)
        .unwrap_or(Color::WHITE);
    let material = actor.components.material();
    if let Some((material, effect)) =
        material.and_then(|material| material.shader.as_ref().map(|effect| (material, effect)))
    {
        let secondary = crate::world::parse_color(&effect.color);
        let shader = crate::materials::surface_shader(commands, &actor.id, effect, dir, true);
        let texture = crate::materials::load_surface_image(
            commands,
            &material.albedo_texture,
            material,
            dir,
            assets,
            true,
        );
        commands
            .entity(id)
            .insert(MeshMaterial3d(graph_materials.add(
                crate::materials::graph_material_3d(
                    material, effect, color, secondary, texture, shader,
                ),
            )));
        return;
    }
    if let Some(material) = material
        .filter(|material| material.is_projected())
    {
        let mut surface = crate::materials::box_material(commands, material, color, dir, assets);
        let key = format!(
            "projected:{:?}:{}:{}",
            dir,
            visual.color().unwrap_or(""),
            serde_json::to_string(material).unwrap_or_default()
        );
        commands.queue(move |world: &mut World| {
            if let Some(globals) = world.get_resource::<crate::materials::SurfaceGlobals>() {
                surface.extension.globals = globals.buffer.clone();
            }
            let handle =
                world.resource_scope(|world, mut cache: Mut<crate::performance::RenderCache>| {
                    let mut materials =
                        world.resource_mut::<Assets<crate::materials::BoxMaterial>>();
                    cache.box_material(key, || surface, &mut materials)
                });
            world.entity_mut(id).insert(MeshMaterial3d(handle));
        });
        return;
    }
    if let Visual::Tilemap { tilemap } = visual {
        let key = format!("tile:{}:{:?}", tilemap.tileset, dir);
        let handle = cache.material(
            key,
            || {
                let texture = if tilemap.tileset.trim().is_empty() {
                    None
                } else {
                    Some(assets.load(crate::world::asset_path(dir, tilemap.tileset.trim())))
                };
                StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: texture,
                    alpha_mode: AlphaMode::Blend,
                    cull_mode: None,
                    ..default()
                }
            },
            materials,
        );
        commands.entity(id).insert(MeshMaterial3d(handle));
        return;
    }
    // Everything else draws instanced: the shared material holds what the
    // surface is made of, the actor's slot its tint and UV transform.
    let key = crate::batching::surface_key(dir, material);
    let base = (!cache.has_instanced(&key)).then(|| {
        let mut base = match material {
            Some(material) => {
                crate::materials::surface_standard(commands, material, Color::WHITE, dir, assets)
            }
            None => StandardMaterial {
                perceptual_roughness: 0.6,
                ..default()
            },
        };
        base.uv_transform = bevy::math::Affine2::IDENTITY;
        // Forward, or deferred while ray tracing lights the G-buffer.
        base.opaque_render_method = bevy::material::OpaqueRendererMethod::Auto;
        base
    });
    let record = crate::batching::InstanceRecord::of(material, color);
    commands.queue(move |world: &mut World| {
        crate::batching::attach_instanced(world, id, key, base, record);
    });
}

/// Drop whatever surface the actor renders with: standard or graph.
fn remove_surface(commands: &mut Commands, id: Entity) {
    commands
        .entity(id)
        .remove::<MeshMaterial3d<StandardMaterial>>();
    commands
        .entity(id)
        .remove::<MeshMaterial3d<GraphMaterial3d>>();
    commands
        .entity(id)
        .remove::<MeshMaterial3d<crate::materials::BoxMaterial>>();
    commands.entity(id).remove::<(
        MeshMaterial3d<crate::batching::InstancedMaterial>,
        bevy::mesh::MeshTag,
        crate::batching::InstanceSlot,
    )>();
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
    if physics.character_controller && physics.body == BodyKind::Kinematic {
        entity.insert(rp::KinematicCharacterController::default());
    }
}

/// Connect authored bodies once both endpoints exist in the world.
pub fn sync_joints(
    mut commands: Commands,
    engine: NonSend<Engine>,
    actors: Query<(
        Entity,
        &crate::engine::ActorId,
        Option<&rp::ImpulseJoint>,
        Option<&rp::RigidBody>,
    )>,
    bodies: Query<&rp::RigidBody>,
) {
    for (entity, id, installed, body) in &actors {
        let desired = engine
            .has_component(&id.0, "Joint")
            .then(|| engine.actor(&id.0).and_then(|a| a.components.joint()))
            .flatten();
        let target = desired.and_then(|j| engine.entities.get(&j.target).copied());
        let valid = body.is_some() && target.is_some_and(|t| t != entity && bodies.get(t).is_ok());
        if !valid {
            if installed.is_some() {
                commands.entity(entity).remove::<rp::ImpulseJoint>();
            }
            continue;
        }
        let spec = desired.unwrap();
        let target = target.unwrap();
        if installed.is_some_and(|joint| joint.parent == target) {
            continue;
        }
        let anchor = Vec3::from(spec.anchor);
        let joint = match spec.kind {
            JointKind::Fixed => {
                rp::ImpulseJoint::new(target, rp::FixedJointBuilder::new().local_anchor2(anchor))
            }
            JointKind::Hinge => rp::ImpulseJoint::new(
                target,
                rp::RevoluteJointBuilder::new(Vec3::Z).local_anchor2(anchor),
            ),
            JointKind::Rope => rp::ImpulseJoint::new(
                target,
                rp::RopeJointBuilder::new(spec.length.max(0.01)).local_anchor2(anchor),
            ),
        };
        commands.entity(entity).insert(joint);
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
    mut controllers: Query<&mut rp::KinematicCharacterController>,
    mut transforms: Query<&mut Transform>,
    mut config: Query<&mut rp::RapierConfiguration>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<AssetServer>,
    mut graph_materials: ResMut<Assets<GraphMaterial3d>>,
    mut cache: ResMut<crate::performance::RenderCache>,
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
            Effect::Move { actor, .. }
            | Effect::ChangePosition { actor, .. }
            | Effect::NavigateTo { actor, .. } => actor,
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
                if let Ok(mut controller) = controllers.get_mut(*entity) {
                    if let Ok(transform) = transforms.get(*entity) {
                        let delta = crate::world::forward_of(
                            transform,
                            blockloom_core::scene::Mode::ThreeD,
                        ) * *steps;
                        controller.translation =
                            Some(controller.translation.unwrap_or(Vec3::ZERO) + delta);
                    }
                    continue;
                }
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
                if let Ok(mut controller) = controllers.get_mut(*entity) {
                    let mut delta = controller.translation.unwrap_or(Vec3::ZERO);
                    delta[axis.index()] += *by;
                    controller.translation = Some(delta);
                    continue;
                }
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
                let Some(entity) = engine.entities.get(actor).copied() else {
                    continue;
                };
                // An instanced actor recolors its own slot; anything else
                // gets a material of its own.
                let color = crate::world::parse_color(color);
                commands.queue(move |world: &mut World| {
                    if crate::batching::set_tint(world, entity, color) {
                        return;
                    }
                    let Some(handle) = world
                        .get::<MeshMaterial3d<StandardMaterial>>(entity)
                        .cloned()
                    else {
                        return;
                    };
                    let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
                    let Some(mut unique) = materials.get(&handle.0).cloned() else {
                        return;
                    };
                    unique.base_color = color;
                    let unique = materials.add(unique);
                    world.entity_mut(entity).insert(MeshMaterial3d(unique));
                });
            }
            Effect::SetEmissiveStrength { actor, strength } => {
                let Some(entity) = engine.entities.get(actor).copied() else {
                    continue;
                };
                if !strength.is_finite() {
                    continue;
                }
                let strength = strength.max(0.0);
                commands.queue(move |world: &mut World| set_glow(world, entity, strength));
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
                        if *body == BodyKind::Kinematic
                            && engine
                                .actor(actor)
                                .is_some_and(|a| a.physics().character_controller)
                        {
                            entity.insert(rp::KinematicCharacterController::default());
                        } else {
                            entity.remove::<rp::KinematicCharacterController>();
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
                        entity.remove::<rp::KinematicCharacterController>();
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
                        let Some(mesh) = surface_mesh(&authored, &visual, &mut cache, &mut meshes)
                        else {
                            continue;
                        };
                        commands.entity(entity).insert(Mesh3d(mesh.clone()));
                        commands
                            .entity(entity)
                            .remove::<(crate::culling::LodGroup, crate::culling::Occluder)>();
                        if let Some(lod) = cache.lod(&visual, &mesh, &mut meshes) {
                            commands.entity(entity).insert(lod);
                        }
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
                            &mut cache,
                        );
                    }
                    "Material" => {
                        // Rebuild the surface from scratch: whatever it drew
                        // with before, it now draws with the authored look
                        // plus the authored material.
                        let Some(visual) = authored.visual().cloned() else {
                            continue;
                        };
                        let Some(mesh) = surface_mesh(&authored, &visual, &mut cache, &mut meshes)
                        else {
                            continue;
                        };
                        remove_surface(&mut commands, entity);
                        commands.entity(entity).insert(Mesh3d(mesh));
                        insert_surface(
                            &mut commands,
                            entity,
                            &authored,
                            &visual,
                            dir.as_deref(),
                            &assets,
                            &mut materials,
                            &mut graph_materials,
                            &mut cache,
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
                        entity.remove::<rp::KinematicCharacterController>();
                    }
                    // Nothing to draw, but the actor is still there to be
                    // moved, sensed and given a look again.
                    "Look" => {
                        commands.entity(id).remove::<(
                            Mesh3d,
                            crate::materials::AnimatedTiles,
                            crate::culling::LodGroup,
                            crate::culling::Occluder,
                        )>();
                        remove_surface(&mut commands, id);
                        crate::model::detach(&mut commands, id, &models);
                    }
                    // Back to the plain standard surface.
                    "Material" => {
                        let Some(mut authored) = engine.actor(actor).cloned() else {
                            continue;
                        };
                        authored.components.remove("Material");
                        let Some(visual) = authored.visual().cloned() else {
                            continue;
                        };
                        remove_surface(&mut commands, id);
                        if let Some(mesh) =
                            surface_mesh(&authored, &visual, &mut cache, &mut meshes)
                        {
                            commands.entity(id).insert(Mesh3d(mesh));
                        }
                        insert_surface(
                            &mut commands,
                            id,
                            &authored,
                            &visual,
                            dir.as_deref(),
                            &assets,
                            &mut materials,
                            &mut graph_materials,
                            &mut cache,
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

/// A camera and a sun, so a fresh 3D project isn't a black window. How they
/// look - exposure, post, AO, the light itself - comes from the blended
/// environment (`environment::apply_environment`).
pub fn spawn_scenery(commands: &mut Commands, camera: &blockloom_core::scene::Camera) {
    commands.spawn((
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
        // Bevy's depth pyramid culls hidden meshes after the depth prepass;
        // `culling::configure_cameras` adds `OcclusionCulling` per policy.
        bevy::core_pipeline::prepass::DepthPrepass,
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        crate::world::WorldLight,
    ));
}

/// The emissive a glowing actor started from, so strengths don't compound,
/// and the material made its own, which later writes change in place.
#[derive(Component, Clone, Copy)]
struct OwnGlow(LinearRgba, bevy::asset::UntypedAssetId);

/// Glows `strength` times the surface's authored emissive, or its color when
/// it has none. The first write gives the actor a material of its own.
fn set_glow(world: &mut World, entity: Entity, strength: f32) {
    let tint = crate::batching::tint_of(world, entity);
    let _ =
        glow::<crate::batching::InstancedMaterial>(world, entity, strength, tint, |m| &mut m.base)
            || glow::<crate::materials::BoxMaterial>(world, entity, strength, None, |m| {
                &mut m.base
            })
            || glow::<StandardMaterial>(world, entity, strength, None, |m| m);
}

fn glow<M: Material + Clone>(
    world: &mut World,
    entity: Entity,
    strength: f32,
    tint: Option<LinearRgba>,
    surface: impl Fn(&mut M) -> &mut StandardMaterial,
) -> bool {
    let Some(handle) = world.get::<MeshMaterial3d<M>>(entity).map(|m| m.0.clone()) else {
        return false;
    };
    let own = world.get::<OwnGlow>(entity).copied();
    let unique = own.is_some_and(|own| own.1 == handle.id().untyped());
    let mut materials = world.resource_mut::<Assets<M>>();
    let Some(mut material) = materials.get(&handle).cloned() else {
        return true;
    };
    let base = surface(&mut material);
    let authored = own.map_or_else(
        || {
            if base.emissive == LinearRgba::BLACK {
                tint.unwrap_or_else(|| base.base_color.into())
            } else {
                base.emissive
            }
        },
        |own| own.0,
    );
    base.emissive = authored * strength;
    if unique {
        if let Some(mut current) = materials.get_mut(&handle) {
            *current = material;
        }
    } else {
        let made = materials.add(material);
        let id = made.id().untyped();
        world
            .entity_mut(entity)
            .insert((MeshMaterial3d(made), OwnGlow(authored, id)));
    }
    true
}

#[cfg(test)]
mod texture_tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    #[test]
    fn box_faces_tile_by_their_own_world_dimensions() {
        let visual = Visual::Cuboid {
            color: "#FFFFFF".into(),
            size: [4.0, 2.0, 8.0],
        };
        let mut mesh = mesh_for(&visual).unwrap();
        let VertexAttributeValues::Float32x3(normals) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap()
        else {
            panic!("normals");
        };
        let normals = normals.clone();
        let VertexAttributeValues::Float32x2(original) =
            mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap()
        else {
            panic!("uvs");
        };
        let original = original.clone();
        scale_primitive_uv(&mut mesh, &visual, 2.0);
        let VertexAttributeValues::Float32x2(scaled) =
            mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap()
        else {
            panic!("scaled uvs");
        };
        for ((normal, before), after) in normals.iter().zip(&original).zip(scaled) {
            let dimensions = if normal[0].abs() > 0.5 {
                [8.0, 2.0]
            } else if normal[1].abs() > 0.5 {
                [4.0, 8.0]
            } else {
                [4.0, 2.0]
            };
            assert_eq!(
                *after,
                [
                    before[0] * dimensions[0] * 2.0,
                    before[1] * dimensions[1] * 2.0
                ]
            );
        }
    }
}
