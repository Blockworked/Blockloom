//! Bounded physical debris and smoke, plus the lasting surface-state owner.

use crate::engine::{ActorId, Dimension, Engine, PendingEffects};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_rapier3d::prelude as rp;
use blockloom_core::components::ActorComponent;
use blockloom_core::destruction::{
    self as model, GRID, SHARD_CAP, SMOKE_CAP, SmokeGrid, SurfaceMap,
};
use blockloom_core::scene::{Mode, Visual};
use blockloom_core::vm::Effect;
use std::collections::{HashMap, VecDeque};

pub fn register(app: &mut App) {
    app.init_resource::<Destruction>()
        .init_resource::<SurfaceTexture>()
        .add_systems(
            Update,
            reset
                .after(crate::world::pump_editor)
                .before(crate::world::rebuild_world),
        )
        .add_systems(
            FixedUpdate,
            simulate
                .in_set(crate::world::SimulationSet)
                .after(crate::world::apply_common)
                .after(crate::world::apply_lifetimes)
                .after(crate::dim2::apply_effects)
                .after(crate::dim3::apply_effects)
                .after(crate::water::float_bodies_2d)
                .after(crate::water::float_bodies_3d)
                .before(crate::world::clear_effects),
        )
        .add_systems(
            Update,
            (draw_smoke, upload_surface).after(crate::world::rebuild_world),
        )
        .add_systems(Last, flush_on_exit);
    if app.world().resource::<Dimension>().0 == Mode::ThreeD {
        app.add_systems(
            FixedUpdate,
            (hits, age_shards)
                .after(crate::world::step_vm)
                .in_set(crate::world::SimulationSet)
                .before(simulate)
                .before(crate::world::clear_effects),
        );
    }
}

#[derive(Resource, Default)]
pub struct Destruction {
    pub map: SurfaceMap,
    key: String,
    dirty: bool,
    save_in: f32,
    pub shards: VecDeque<Entity>,
    smoke: VecDeque<(u64, SmokeGrid)>,
    serial: u64,
    feet: HashMap<Entity, Vec3>,
    rain_tick: u32,
    pub stolen: u64,
}

#[derive(Component)]
pub(crate) struct Shard {
    age: f32,
    last_visible: f32,
    lifetime: f32,
    asleep: f32,
    sleep_seconds: f32,
    sound: String,
    bounce_in: f32,
}

#[derive(Component)]
struct Smoke(u64);

fn storage_key(engine: &Engine) -> String {
    // IDs stay data, even when a hand-written project contains path separators.
    let bytes = format!("{}:{}", engine.project.id, engine.project.active_scene);
    bytes
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn load(key: &str) -> Result<Option<SurfaceMap>, String> {
    let path = blockloom_core::project::data_dir()
        .join("surfaces")
        .join(format!("{key}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| e.to_string())
}
#[cfg(target_arch = "wasm32")]
fn load(key: &str) -> Result<Option<SurfaceMap>, String> {
    let store = web_sys::window()
        .ok_or("no window")?
        .local_storage()
        .map_err(|e| format!("{e:?}"))?
        .ok_or("no storage")?;
    store
        .get_item(&format!("blockloom:surfaces:{key}"))
        .map_err(|e| format!("{e:?}"))?
        .map(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
        .transpose()
}
#[cfg(not(target_arch = "wasm32"))]
fn store(key: &str, map: &SurfaceMap) -> Result<(), String> {
    let dir = blockloom_core::project::data_dir().join("surfaces");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{key}.json"));
    std::fs::write(path, serde_json::to_vec(map).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
#[cfg(target_arch = "wasm32")]
fn store(key: &str, map: &SurfaceMap) -> Result<(), String> {
    let store = web_sys::window()
        .ok_or("no window")?
        .local_storage()
        .map_err(|e| format!("{e:?}"))?
        .ok_or("no storage")?;
    store
        .set_item(
            &format!("blockloom:surfaces:{key}"),
            &serde_json::to_string(map).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("{e:?}"))
}

fn report(message: String) {
    crate::bridge::send(&blockloom_protocol::RuntimeMessage::Error {
        actor: "Blockloom".into(),
        message,
    });
}

fn flush(state: &mut Destruction) {
    if !state.dirty || state.key.is_empty() {
        return;
    }
    match store(&state.key, &state.map) {
        Ok(()) => state.dirty = false,
        Err(error) => report(format!("couldn't save surface state: {error}")),
    }
    state.save_in = 5.0;
}

fn reset(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut state: ResMut<Destruction>,
    transient: Query<Entity, Or<(With<Shard>, With<Smoke>)>>,
) {
    let key = storage_key(&engine);
    if key != state.key {
        flush(&mut state);
        state.map = match load(&key) {
            Ok(Some(map)) if map.valid() => map,
            Ok(None) => SurfaceMap::default(),
            Ok(Some(_)) => {
                report("invalid saved surface map".into());
                SurfaceMap::default()
            }
            Err(error) => {
                report(format!("couldn't load surface state: {error}"));
                SurfaceMap::default()
            }
        };
        state.key = key;
    }
    if engine.rebuild || !engine.running {
        flush(&mut state);
        for entity in &transient {
            commands.entity(entity).despawn();
        }
        state.shards.clear();
        state.smoke.clear();
        state.feet.clear();
    }
}

fn hits(
    mut events: MessageReader<rp::ContactForceEvent>,
    engine: NonSend<Engine>,
    time: Res<Time<Fixed>>,
    actors: Query<&ActorId>,
    mut commands: Commands,
    bodies: Query<(Entity, &ActorId), With<rp::Collider>>,
    mut effects: ResMut<PendingEffects>,
) {
    if !engine.running || engine.paused {
        events.clear();
        return;
    }
    for (entity, id) in &bodies {
        if let Some(ActorComponent::Fracture { fracture }) = engine
            .actor(&id.0)
            .and_then(|a| a.components.get("Fracture"))
            && engine.has_component(&id.0, "Fracture")
        {
            commands.entity(entity).insert((
                rp::ActiveEvents::COLLISION_EVENTS | rp::ActiveEvents::CONTACT_FORCE_EVENTS,
                rp::ContactForceEventThreshold(
                    fracture.impulse_threshold.max(0.0) / time.delta_secs().max(1e-6),
                ),
            ));
        }
    }
    for event in events.read() {
        if !event.started {
            continue;
        }
        for entity in [event.collider1, event.collider2] {
            let Ok(id) = actors.get(entity) else {
                continue;
            };
            if !engine.has_component(&id.0, "Fracture") {
                continue;
            }
            let Some(ActorComponent::Fracture { fracture }) = engine
                .actor(&id.0)
                .and_then(|a| a.components.get("Fracture"))
            else {
                continue;
            };
            if event.total_force_magnitude * time.delta_secs()
                >= fracture.impulse_threshold.max(0.0)
            {
                effects.0.push(Effect::Fracture {
                    actor: id.0.clone(),
                });
            }
        }
    }
}

fn age_shards(
    mut commands: Commands,
    engine: NonSend<Engine>,
    time: Res<Time<Fixed>>,
    mut shards: Query<(Entity, &mut Shard, Option<&rp::Sleeping>, &Transform)>,
    mut collisions: MessageReader<rp::CollisionEvent>,
    assets: Res<AssetServer>,
    mut sound: ResMut<crate::sound::SoundState>,
) {
    if !engine.running || engine.paused {
        collisions.clear();
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut shard, sleeping, _) in &mut shards {
        shard.age += dt;
        shard.bounce_in = (shard.bounce_in - dt).max(0.0);
        shard.asleep = if sleeping.is_some_and(|s| s.sleeping) {
            shard.asleep + dt
        } else {
            0.0
        };
        if shard.age >= shard.lifetime || shard.asleep >= shard.sleep_seconds {
            commands.entity(entity).despawn();
        }
    }
    for event in collisions.read() {
        let rp::CollisionEvent::Started(a, b, _) = event else {
            continue;
        };
        for entity in [*a, *b] {
            let Ok((_, mut shard, _, transform)) = shards.get_mut(entity) else {
                continue;
            };
            if !shard.sound.is_empty() && shard.bounce_in <= 0.0 {
                sound.bounce(
                    &mut commands,
                    &assets,
                    engine.project_dir.as_deref(),
                    &shard.sound,
                    transform.translation,
                );
                shard.bounce_in = 0.2;
            }
        }
    }
}

fn fracture_hull(visual: &Visual) -> Vec<model::Face> {
    let mesh = match visual {
        Visual::Sphere { radius, .. } => Sphere::new(*radius).mesh().uv(12, 8),
        Visual::Capsule { radius, height, .. } => Capsule3d::new(*radius, *height)
            .mesh()
            .longitudes(12)
            .latitudes(8)
            .build(),
        _ => {
            return model::box_hull(model::Vec3::from(
                (crate::world::half_extents3(visual) * 2.0).to_array(),
            ));
        }
    };
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Vec::new();
    };
    let indices: Vec<_> = mesh
        .indices()
        .map(|i| i.iter().collect())
        .unwrap_or_else(|| (0..positions.len()).collect());
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|tri| {
            let mut vertices: Vec<_> = tri
                .iter()
                .map(|i| model::Vec3::from(positions[*i]))
                .collect();
            let n = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
            if n.length_squared() < 1e-10 {
                return None;
            }
            if n.dot(vertices[0]) < 0.0 {
                vertices.reverse();
            }
            Some(model::Face {
                vertices,
                interior: false,
            })
        })
        .collect()
}

fn cell_mesh(cell: &model::Cell, interior: bool) -> Option<Mesh> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uv = Vec::new();
    for face in cell.faces.iter().filter(|f| f.interior == interior) {
        let a = face.vertices[0];
        for pair in face.vertices[1..].windows(2) {
            let normal = (pair[0] - a).cross(pair[1] - a).normalize_or_zero();
            for point in [a, pair[0], pair[1]] {
                positions.push((point - cell.center).to_array());
                normals.push(normal.to_array());
                uv.push([point.x, point.z]);
            }
        }
    }
    if positions.is_empty() {
        return None;
    }
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv),
    )
}

#[allow(clippy::too_many_arguments)]
fn simulate(
    scaling: Option<Res<crate::quality::Scaling>>,
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    time: Res<Time<Fixed>>,
    effects: Res<PendingEffects>,
    mut state: ResMut<Destruction>,
    actors: Query<(&Transform, Option<&rp::Velocity>), With<ActorId>>,
    mut existing: Query<(&mut Shard, Option<&ViewVisibility>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
    mut water: ResMut<crate::water::WaterState>,
    wind: Res<crate::wind::WindField>,
    atmosphere: Res<crate::atmosphere::AtmosphereSources>,
) {
    if !engine.running || engine.paused {
        return;
    }
    let dt = time.delta_secs();
    state.shards.retain(|entity| existing.contains(*entity));
    let now = time.elapsed_secs();
    for (mut shard, visible) in &mut existing {
        if visible.is_some_and(|v| v.get()) {
            shard.last_visible = now;
        }
    }
    state.shards.make_contiguous().sort_by(|a, b| {
        let stamp = |entity| existing.get(entity).map_or(0.0, |(s, _)| s.last_visible);
        stamp(*a).total_cmp(&stamp(*b)).then(a.cmp(b))
    });
    let cap = scaling
        .as_ref()
        .map_or(SHARD_CAP, |s| s.shard_budget(SHARD_CAP));
    while state.shards.len() > cap {
        if let Some(entity) = state.shards.pop_front() {
            commands.entity(entity).try_despawn();
            state.stolen += 1;
        }
    }
    let before = state.map.cells.clone();
    state.map.step(dt, atmosphere.rain);
    let pending = effects.0.clone();
    for effect in pending {
        match effect {
            Effect::Splash {
                at,
                radius,
                strength,
            } => {
                if !Vec3::from(at).is_finite()
                    || !radius.is_finite()
                    || !strength.is_finite()
                    || radius <= 0.0
                {
                    continue;
                }
                let flat = dimension.0 == Mode::TwoD;
                let plane = if flat {
                    Vec2::new(at[0], at[1])
                } else {
                    Vec2::new(at[0], at[2])
                };
                state
                    .map
                    .paint(model::Vec2::from(plane.to_array()), radius, 0.0, 1.0);
                let bodies: Vec<_> = water
                    .bodies
                    .iter()
                    .filter(|b| b.body.covers(at[0], if flat { 0.0 } else { at[2] }))
                    .map(|b| b.body.id.clone())
                    .collect();
                for body in bodies {
                    water.disturb(
                        &body,
                        [at[0], if flat { 0.0 } else { at[2] }],
                        radius,
                        strength.clamp(-10.0, 10.0),
                        false,
                    );
                }
            }
            Effect::PuffSmoke {
                at,
                radius,
                strength,
            } => {
                if let Some(grid) = SmokeGrid::new(model::Vec3::from(at), radius, strength) {
                    if state.smoke.len() >= SMOKE_CAP {
                        state.smoke.pop_front();
                        state.stolen += 1;
                    }
                    state.serial += 1;
                    let serial = state.serial;
                    state.smoke.push_back((serial, grid));
                }
            }
            Effect::StrikeLightning { at } => {
                let plane = if dimension.0 == Mode::TwoD {
                    Vec2::new(at[0], at[1])
                } else {
                    Vec2::new(at[0], at[2])
                };
                state
                    .map
                    .paint(model::Vec2::from(plane.to_array()), 4.0, 1.0, 0.0);
            }
            Effect::Fracture { actor: id } => {
                let Some(entity) = engine.entities.get(&id).copied() else {
                    continue;
                };
                if !engine.has_component(&id, "Fracture") {
                    report(format!("{id} needs an attached Fracture component"));
                    continue;
                }
                let Ok((transform, velocity)) = actors.get(entity) else {
                    continue;
                };
                let Some(actor) = engine.actor(&id).cloned() else {
                    continue;
                };
                let Some(ActorComponent::Fracture { fracture: spec }) =
                    actor.components.get("Fracture")
                else {
                    report(format!("{} needs a Fracture component", actor.name));
                    continue;
                };
                if dimension.0 != Mode::ThreeD
                    || !matches!(
                        actor.visual(),
                        Some(
                            Visual::Cuboid { .. }
                                | Visual::Plane { .. }
                                | Visual::Sphere { .. }
                                | Visual::Capsule { .. }
                        )
                    )
                {
                    report(format!(
                        "{}: fracture supports convex 3D primitives",
                        actor.name
                    ));
                    continue;
                }
                let size = crate::world::half_extents3(actor.visual().unwrap()) * 2.0;
                let hull = fracture_hull(actor.visual().unwrap());
                let cells = model::fracture(
                    &hull,
                    model::Vec3::from(size.to_array()),
                    spec.cells,
                    spec.seed,
                );
                if cells.is_empty() {
                    report(format!("{} has no fracture volume", actor.name));
                    continue;
                }
                let linear = velocity.map_or(Vec3::ZERO, |v| v.linear);
                let angular = velocity.map_or(Vec3::ZERO, |v| v.angular);
                let authored_cap = (spec.pool_cap as usize).clamp(1, SHARD_CAP);
                let cap = scaling
                    .as_ref()
                    .map_or(authored_cap, |s| s.shard_budget(authored_cap).max(1));
                let outer = actor.components.material().cloned().unwrap_or_default();
                let outside = materials.add(crate::materials::surface_standard(
                    &mut commands,
                    &outer,
                    crate::world::parse_color(actor.visual().unwrap().color().unwrap_or("#FFFFFF")),
                    engine.project_dir.as_deref(),
                    &assets,
                ));
                let inside = materials.add(crate::materials::surface_standard(
                    &mut commands,
                    &spec.interior,
                    Color::WHITE,
                    engine.project_dir.as_deref(),
                    &assets,
                ));
                let cell_count = cells.len().max(1) as f32;
                for cell in cells {
                    let points: Vec<_> = cell
                        .faces
                        .iter()
                        .flat_map(|f| {
                            f.vertices.iter().map(|p| {
                                Vec3::from((*p - cell.center).to_array()) * transform.scale
                            })
                        })
                        .collect();
                    let Some(collider) = rp::Collider::convex_hull(&points) else {
                        continue;
                    };
                    while state.shards.len() >= cap {
                        if let Some(old) = state.shards.pop_front() {
                            commands.entity(old).despawn();
                            state.stolen += 1;
                        }
                    }
                    let offset =
                        transform.rotation * (Vec3::from(cell.center.to_array()) * transform.scale);
                    let shard = commands
                        .spawn((
                            Shard {
                                age: 0.0,
                                last_visible: now,
                                lifetime: spec.lifetime.clamp(0.1, 3600.0),
                                asleep: 0.0,
                                sleep_seconds: spec.sleep_seconds.clamp(0.1, 3600.0),
                                sound: spec.bounce_sound.clone(),
                                bounce_in: 0.0,
                            },
                            Transform::from_translation(transform.translation + offset)
                                .with_rotation(transform.rotation),
                            Visibility::default(),
                            rp::RigidBody::Dynamic,
                            rp::Sleeping::default(),
                            match actor.physics().mass {
                                Some(mass) => rp::ColliderMassProperties::Mass(mass / cell_count),
                                None => {
                                    rp::ColliderMassProperties::Density(actor.physics().density)
                                }
                            },
                            rp::GravityScale(actor.physics().gravity_scale),
                            collider,
                            rp::Velocity {
                                linear: linear + angular.cross(offset),
                                angular,
                            },
                            rp::Restitution::coefficient(actor.physics().restitution),
                            rp::Friction::coefficient(actor.physics().friction),
                            rp::ActiveEvents::COLLISION_EVENTS,
                        ))
                        .id();
                    for (interior, material) in [(false, outside.clone()), (true, inside.clone())] {
                        if let Some(mesh) = cell_mesh(&cell, interior) {
                            commands.spawn((
                                Mesh3d(meshes.add(mesh)),
                                MeshMaterial3d(material),
                                Transform::from_scale(transform.scale),
                                ChildOf(shard),
                            ));
                        }
                    }
                    state.shards.push_back(shard);
                }
                if let Some(logic) = &mut engine.logic {
                    logic.deleted(&id);
                }
                engine.vm.delete_actor(&id, &id);
                engine.variables.forget_actor(&id);
                engine.lists.forget_actor(&id);
                engine.dicts.forget_actor(&id);
                crate::world::delete_actor(&mut commands, &mut engine, &id);
            }
            _ => {}
        }
    }
    state
        .feet
        .retain(|entity, _| engine.entities.values().any(|e| e == entity));
    // Moving feet stir existing water; rain uses a deterministic cell sequence.
    for (id, entity) in &engine.entities {
        let Ok((transform, _)) = actors.get(*entity) else {
            continue;
        };
        let half = engine
            .actor(id)
            .and_then(|a| a.visual())
            .map_or(0.0, |visual| {
                if dimension.0 == Mode::TwoD {
                    crate::world::half_extents(visual).y
                } else {
                    crate::world::half_extents3(visual).y
                }
            })
            * transform.scale.y.abs();
        let position = transform.translation - Vec3::Y * half;
        let old = state.feet.get(entity).copied();
        if old.is_none() {
            state.feet.insert(*entity, position);
        }
        if old.is_some_and(|p| p.distance_squared(position) > 0.0625) {
            state.feet.insert(*entity, position);
            let z = if dimension.0 == Mode::TwoD {
                0.0
            } else {
                position.z
            };
            let bodies: Vec<_> = water
                .bodies
                .iter()
                .filter(|b| {
                    b.body.covers(position.x, z)
                        && (position.y - b.body.height_at(position.x, z, water.time)).abs() < 0.5
                })
                .map(|b| b.body.id.clone())
                .collect();
            for body in bodies {
                water.disturb(&body, [position.x, z], 0.3, 0.05, false);
            }
        }
    }
    if atmosphere.rain > 0.0 {
        state.rain_tick = state.rain_tick.wrapping_add(1);
        let seed = state.rain_tick;
        let rings: Vec<_> = water
            .bodies
            .iter()
            .map(|b| {
                let field = b.body.ripples.as_ref();
                (
                    b.body.id.clone(),
                    field.map(|f| {
                        [
                            f.origin[0] + f.cell * f.cells[0] as f32 * 0.5,
                            if b.body.flat {
                                0.0
                            } else {
                                f.origin[1] + f.cell * f.cells[1] as f32 * 0.5
                            },
                        ]
                    }),
                )
            })
            .collect();
        for (id, center) in rings {
            if let Some(center) = center {
                let x = ((seed.wrapping_mul(1664525) >> 8) % 256) as f32 / 256.0 - 0.5;
                let z = ((seed.wrapping_mul(22695477) >> 8) % 256) as f32 / 256.0 - 0.5;
                water.disturb(
                    &id,
                    [center[0] + x * 8.0, center[1] + z * 8.0],
                    0.2,
                    atmosphere.rain * 0.02,
                    false,
                );
            }
        }
    }
    for (_, grid) in &mut state.smoke {
        let air = wind.at(Vec3::from(grid.at.to_array()));
        grid.step(dt, model::Vec2::new(air.x, air.y));
    }
    state
        .smoke
        .retain(|(_, grid)| grid.age < 20.0 && grid.density.iter().any(|d| *d > 0.005));
    if state.map.cells != before {
        state.dirty = true;
    }
    state.save_in -= dt;
    if state.save_in <= 0.0 {
        flush(&mut state);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_smoke(
    mut commands: Commands,
    dimension: Res<Dimension>,
    state: Res<Destruction>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(
        Entity,
        &Smoke,
        Option<&Sprite>,
        Option<&MeshMaterial3d<StandardMaterial>>,
    )>,
    cameras: Query<&Transform, With<crate::world::WorldCamera>>,
) {
    let flat = dimension.0 == Mode::TwoD;
    let facing = cameras.iter().next().map_or(Quat::IDENTITY, |t| t.rotation);
    let mut present = std::collections::HashSet::new();
    for (entity, tag, sprite, material) in &drawn {
        if let Some((_, grid)) = state.smoke.iter().find(|(id, _)| *id == tag.0) {
            let handle = sprite.map(|s| s.image.clone()).or_else(|| {
                material.and_then(|m| {
                    materials
                        .get(&m.0)
                        .and_then(|m| m.base_color_texture.clone())
                })
            });
            if let Some(handle) = handle
                && let Some(mut image) = images.get_mut(&handle)
            {
                image.data = Some(smoke_pixels(grid));
            }
            commands.entity(entity).insert(
                Transform::from_translation(Vec3::from(grid.at.to_array()) + Vec3::Y * grid.radius)
                    .with_rotation(if flat { Quat::IDENTITY } else { facing }),
            );
            present.insert(tag.0);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (id, grid) in &state.smoke {
        if present.contains(id) {
            continue;
        }
        let image = images.add(Image::new(
            Extent3d {
                width: GRID as u32,
                height: GRID as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            smoke_pixels(grid),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ));
        let transform =
            Transform::from_translation(Vec3::from(grid.at.to_array()) + Vec3::Y * grid.radius)
                .with_rotation(if flat { Quat::IDENTITY } else { facing });
        if flat {
            commands.spawn((
                Smoke(*id),
                Sprite {
                    image,
                    custom_size: Some(Vec2::splat(grid.radius * 4.0)),
                    ..default()
                },
                transform,
            ));
        } else {
            commands.spawn((
                Smoke(*id),
                Mesh3d(meshes.add(Rectangle::new(grid.radius * 4.0, grid.radius * 4.0))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color_texture: Some(image),
                    alpha_mode: AlphaMode::Blend,
                    unlit: true,
                    cull_mode: None,
                    ..default()
                })),
                transform,
            ));
        }
    }
}
fn smoke_pixels(grid: &SmokeGrid) -> Vec<u8> {
    (0..GRID)
        .rev()
        .flat_map(|y| {
            (0..GRID).flat_map(move |x| {
                [
                    100,
                    100,
                    100,
                    (grid.density[y * GRID + x].clamp(0.0, 1.0) * 180.0) as u8,
                ]
            })
        })
        .collect()
}

#[derive(Resource, Default)]
struct SurfaceTexture {
    image: Option<Handle<Image>>,
    pixels: Vec<u8>,
}

fn upload_surface(
    state: Res<Destruction>,
    mut texture: ResMut<SurfaceTexture>,
    mut images: ResMut<Assets<Image>>,
    mut boxes: ResMut<Assets<crate::materials::BoxMaterial>>,
    terrain: Option<ResMut<Assets<crate::terrain::material::TerrainMaterial>>>,
    instanced: Option<ResMut<Assets<crate::batching::InstancedMaterial>>>,
) {
    let pixels: Vec<u8> = state
        .map
        .cells
        .iter()
        .flat_map(|c| {
            [
                (c[0] * 255.0).round() as u8,
                (c[1] * 255.0).round() as u8,
                0,
                255,
            ]
        })
        .collect();
    if texture.image.is_none() {
        let mut image = Image::new(
            Extent3d {
                width: GRID as u32,
                height: GRID as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels.clone(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        image.texture_descriptor.label = Some("destruction/surface_state");
        image.texture_descriptor.usage |=
            bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT;
        texture.image = Some(images.add(image));
    } else if pixels != texture.pixels
        && let Some(handle) = texture.image.as_ref()
        && let Some(mut image) = images.get_mut(handle)
    {
        image.data = Some(pixels.clone());
    }
    texture.pixels = pixels;
    let handle = texture.image.clone();
    let missing: Vec<_> = boxes
        .iter()
        .filter(|(_, m)| m.extension.surface_state != handle)
        .map(|(id, _)| id)
        .collect();
    for id in missing {
        if let Some(mut material) = boxes.get_mut(id) {
            material.extension.surface_state = handle.clone();
        }
    }
    if let Some(mut terrain) = terrain {
        let missing: Vec<_> = terrain
            .iter()
            .filter(|(_, m)| m.extension.surface_state != handle)
            .map(|(id, _)| id)
            .collect();
        for id in missing {
            if let Some(mut material) = terrain.get_mut(id) {
                material.extension.surface_state = handle.clone();
            }
        }
    }
    if let Some(mut instanced) = instanced {
        let missing: Vec<_> = instanced
            .iter()
            .filter(|(_, m)| m.extension.surface_state != handle)
            .map(|(id, _)| id)
            .collect();
        for id in missing {
            if let Some(mut material) = instanced.get_mut(id) {
                material.extension.surface_state = handle.clone();
            }
        }
    }
}

impl Destruction {
    pub fn metrics(&self) -> [(&'static str, f64); 5] {
        [
            ("destruction/shards", self.shards.len() as f64),
            ("destruction/shard_budget", SHARD_CAP as f64),
            ("fluids/smoke_grids", self.smoke.len() as f64),
            ("fluids/smoke_budget", SMOKE_CAP as f64),
            ("destruction/stolen", self.stolen as f64),
        ]
    }
}

fn flush_on_exit(mut exit: MessageReader<AppExit>, mut state: ResMut<Destruction>) {
    if exit.read().next().is_some() {
        flush(&mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::destruction::FractureSpec;

    fn app() -> (App, String) {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut engine = Engine::new(rx, Mode::ThreeD);
        let mut actor = engine.project.actors[0].clone();
        actor.components.set_visual(Visual::Cuboid {
            size: [2.0; 3],
            color: "#FFFFFF".into(),
        });
        actor.components.insert(ActorComponent::Fracture {
            fracture: FractureSpec {
                cells: 8,
                pool_cap: 4,
                ..default()
            },
        });
        let id = actor.id.clone();
        engine.project.actors[0] = actor;
        engine.running = true;
        engine
            .attached
            .insert(id.clone(), ["Fracture".into()].into());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .insert_resource(Dimension(Mode::ThreeD))
            .init_resource::<Destruction>()
            .init_resource::<PendingEffects>()
            .init_resource::<crate::water::WaterState>()
            .init_resource::<crate::wind::WindField>()
            .init_resource::<crate::atmosphere::AtmosphereSources>()
            .init_resource::<crate::sound::SoundState>()
            .add_message::<rp::CollisionEvent>()
            .add_systems(Update, simulate);
        let entity = app
            .world_mut()
            .spawn((
                ActorId(id.clone()),
                Transform::default(),
                rp::Velocity {
                    linear: Vec3::new(1.0, 2.0, 3.0),
                    angular: Vec3::Y,
                },
            ))
            .id();
        engine.entities.insert(id.clone(), entity);
        app.insert_non_send(engine);
        (app, id)
    }

    #[test]
    fn fracture_caps_the_pool_inherits_velocity_and_leaves_the_document() {
        let (mut app, id) = app();
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![
            Effect::Fracture { actor: id.clone() },
            Effect::Fracture { actor: id.clone() },
        ];
        app.update();
        let state = app.world().resource::<Destruction>();
        assert_eq!(state.shards.len(), 4);
        assert_eq!(state.stolen, 4);
        let engine = app.world().non_send::<Engine>();
        assert!(!engine.entities.contains_key(&id));
        assert!(engine.project.actor(&id).is_some());
        for entity in &state.shards {
            let velocity = app.world().get::<rp::Velocity>(*entity).unwrap();
            assert_eq!(velocity.linear.y, 2.0);
            assert_eq!(velocity.angular, Vec3::Y);
            assert!(app.world().get::<rp::Collider>(*entity).is_some());
        }
    }

    #[test]
    fn debris_density_evicts_hidden_shards_and_counts_their_child_meshes() {
        let (mut app, id) = app();
        app.init_resource::<crate::quality::Scaling>()
            .add_systems(Last, crate::quality::measure_draws);
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::Fracture { actor: id }];
        app.update();
        let shards: Vec<_> = app
            .world()
            .resource::<Destruction>()
            .shards
            .iter()
            .copied()
            .collect();
        // Meshes draw below the physics entity that carries the shard marker.
        let meshes: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<Mesh3d>>()
            .iter(app.world())
            .collect();
        for entity in &meshes {
            app.world_mut()
                .entity_mut(*entity)
                .insert(ViewVisibility::VISIBLE);
        }
        let mut visible = bevy::camera::visibility::VisibleEntities::default();
        visible
            .get_mut(std::any::TypeId::of::<Mesh3d>())
            .extend(meshes.iter().copied());
        app.world_mut()
            .spawn((crate::world::WorldCamera, Camera::default(), visible));
        let kept = shards[0];
        app.world_mut()
            .entity_mut(kept)
            .insert(ViewVisibility::VISIBLE);
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.world_mut()
            .resource_mut::<crate::quality::Scaling>()
            .geometry
            .factors[4] = 0.5;
        app.update();
        let state = app.world().resource::<Destruction>();
        assert_eq!(state.shards.len(), 4);
        // The global cap trims existing shards, preserving the visible one.
        app.world_mut()
            .resource_mut::<crate::quality::Scaling>()
            .controller
            .quality = blockloom_core::quality::Quality::Low;
        app.world_mut()
            .resource_mut::<crate::quality::Scaling>()
            .geometry
            .factors[4] = 0.25;
        // Add old hidden entries to exercise trimming without another fracture.
        for _ in 0..20 {
            let entity = app
                .world_mut()
                .spawn(Shard {
                    age: 0.0,
                    last_visible: -1.0,
                    lifetime: 10.0,
                    asleep: 0.0,
                    sleep_seconds: 1.0,
                    sound: String::new(),
                    bounce_in: 0.0,
                })
                .id();
            app.world_mut()
                .resource_mut::<Destruction>()
                .shards
                .push_back(entity);
        }
        for _ in 0..29 {
            app.update();
        }
        let state = app.world().resource::<Destruction>();
        assert_eq!(state.shards.len(), 16);
        assert!(state.shards.contains(&kept));
        let scaling = app.world().resource::<crate::quality::Scaling>();
        assert!(scaling.costs[4].triangles > 0);
        assert_eq!(scaling.costs[2].triangles, 0);
    }

    #[test]
    fn fracture_scales_an_authored_pool_cap_before_spawning() {
        let (mut app, id) = app();
        app.init_resource::<crate::quality::Scaling>();
        app.world_mut()
            .resource_mut::<crate::quality::Scaling>()
            .geometry
            .factors[4] = 0.5;
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![Effect::Fracture { actor: id }];
        app.update();
        assert_eq!(app.world().resource::<Destruction>().shards.len(), 2);
    }

    #[test]
    fn pause_freezes_smoke_and_surface_state() {
        let (mut app, _) = app();
        app.world_mut().resource_mut::<PendingEffects>().0 = vec![
            Effect::PuffSmoke {
                at: [0.0; 3],
                radius: 2.0,
                strength: 1.0,
            },
            Effect::Splash {
                at: [0.0; 3],
                radius: 10.0,
                strength: 0.1,
            },
        ];
        app.update();
        app.world_mut().resource_mut::<PendingEffects>().0.clear();
        app.world_mut().non_send_mut::<Engine>().paused = true;
        let before = app.world().resource::<Destruction>().map.cells.clone();
        let age = app.world().resource::<Destruction>().smoke[0].1.age;
        app.update();
        assert_eq!(app.world().resource::<Destruction>().map.cells, before);
        assert_eq!(app.world().resource::<Destruction>().smoke[0].1.age, age);
    }
}
