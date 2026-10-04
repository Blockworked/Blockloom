//! The mesh submission service: meshes plugins ask the world to draw.
//!
//! A plugin's `mesh` effect names a mesh; this turns it into one entity
//! (`Mesh3d`, a shared standard material keyed by roughness and emission, and
//! a fixed trimesh collider when it is solid), and a later mesh of the same
//! name replaces it. An `instances` set is many entities sharing one of those
//! meshes and its material, which Bevy draws as one batch. All of a plugin's
//! meshes and sets go when the run ends. 3D only.

use crate::bridge;
use crate::engine::Engine;
use crate::plugins::MeshOp;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_plugin_api::mesh::{ColliderKind, MeshData};
use blockloom_protocol::RuntimeMessage;
use std::collections::{HashMap, HashSet};

#[cfg(feature = "plugins")]
use crate::plugin_compute::ComputeLink;
#[cfg(not(feature = "plugins"))]
#[derive(Resource)]
pub struct ComputeLink;
#[cfg(not(feature = "plugins"))]
impl ComputeLink {
    fn bind_mesh(&self, _: AssetId<Mesh>, _: &str, _: &str, _: u32) {}
    fn unbind_mesh(&self, _: AssetId<Mesh>) {}
    fn available(&self) -> Option<bool> {
        Some(false)
    }
}

/// Marks the entity a plugin's mesh became.
#[derive(Component)]
pub struct PluginMesh;

/// Marks an entity that is one copy of an instance set.
#[derive(Component)]
pub struct PluginInstance;

#[derive(Component)]
pub struct Crossfade {
    elapsed: f32,
    duration: f32,
    retire: bool,
}

pub fn crossfade(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<PluginMeshes>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut meshes: Query<(Entity, &GlobalTransform, &mut Crossfade)>,
) {
    let mut cameras = cameras.iter().filter(|(camera, _)| camera.is_active);
    let camera = cameras.next().map(|(_, pose)| pose.translation());
    let camera = if cameras.next().is_none() {
        camera
    } else {
        None
    };
    for (entity, pose, mut fade) in &mut meshes {
        fade.elapsed += time.delta_secs();
        if fade.elapsed >= fade.duration || camera.is_none() {
            if fade.retire {
                state.retired.retain(|old| *old != entity);
                commands.entity(entity).despawn();
            } else {
                commands
                    .entity(entity)
                    .remove::<(Crossfade, bevy::camera::visibility::VisibilityRange)>();
            }
            continue;
        }
        let progress = (fade.elapsed / fade.duration).clamp(0.0, 1.0);
        let distance = pose.translation().distance(camera.unwrap());
        let range = distance - progress..distance + 1.0 - progress;
        commands
            .entity(entity)
            .insert(bevy::camera::visibility::VisibilityRange {
                start_margin: if fade.retire { 0.0..0.0 } else { range.clone() },
                end_margin: if fade.retire {
                    range
                } else {
                    f32::MAX..f32::MAX
                },
                use_aabb: false,
            });
    }
}

/// Which entity holds each (plugin, name), and the materials in use.
#[derive(Resource, Default)]
pub struct PluginMeshes {
    live: HashMap<(String, String), Entity>,
    transitions: HashMap<(String, String), u16>,
    retired: Vec<Entity>,
    hidden: HashSet<(String, String)>,
    /// What each mesh draws with, so a set of copies can share it.
    assets: HashMap<(String, String), (Handle<Mesh>, Handle<StandardMaterial>)>,
    instances: HashMap<(String, String), Vec<Entity>>,
    materials: HashMap<([u32; 4], Option<String>), Handle<StandardMaterial>>,
    textures: HashMap<String, Handle<Image>>,
    quads: HashMap<
        (String, String),
        (
            Handle<crate::plugin_quads::QuadMaterial>,
            bevy::camera::primitives::Aabb,
        ),
    >,
}

#[cfg(test)]
impl PluginMeshes {
    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty() && self.instances.is_empty()
    }
}

fn bevy_mesh(data: &MeshData) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        data.positions.as_chunks::<3>().0.to_vec(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        data.normals.as_chunks::<3>().0.to_vec(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_COLOR,
        data.colors.as_chunks::<4>().0.to_vec(),
    )
    .with_inserted_indices(Indices::U32(data.indices.clone()));
    if !data.uvs.is_empty() {
        mesh = mesh
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, data.uvs.as_chunks::<2>().0.to_vec());
    }
    mesh
}

fn raster_mesh(data: &MeshData) -> Mesh {
    let Some(gpu) = &data.gpu else {
        return bevy_mesh(data);
    };
    if gpu.quads.is_some() {
        return bevy_mesh(data);
    }
    let count = gpu.vertices as usize;
    let mut positions = vec![[0.0; 3]; count];
    let mut normals = vec![[0.0; 3]; count];
    let mut colors = vec![[0.0; 4]; count];
    let textured = !data.uvs.is_empty();
    let mut uvs = vec![[0.0; 2]; count];
    // Keep the CPU fallback visible until the compute buffer has completed.
    for (out, &index) in data.indices.iter().enumerate() {
        let i = index as usize;
        positions[out].copy_from_slice(&data.positions[i * 3..i * 3 + 3]);
        normals[out].copy_from_slice(&data.normals[i * 3..i * 3 + 3]);
        colors[out].copy_from_slice(&data.colors[i * 4..i * 4 + 4]);
        if textured {
            uvs[out].copy_from_slice(&data.uvs[i * 2..i * 2 + 2]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    if textured {
        mesh = mesh.with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
    mesh
}

fn collider(data: &MeshData) -> Option<rp::Collider> {
    let vertices: Vec<Vec3> = data
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|v| Vec3::from(*v))
        .collect();
    let triangles: Vec<[u32; 3]> = data.indices.as_chunks::<3>().0.to_vec();
    // An empty mesh has nothing to collide with.
    if triangles.is_empty() {
        return None;
    }
    match data.collider_kind {
        ColliderKind::Trimesh => rp::Collider::trimesh(vertices, triangles).ok(),
        ColliderKind::ConvexHull => rp::Collider::convex_hull(&vertices),
        ColliderKind::Aabb => {
            let (min, max) = vertices.iter().fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), v| (lo.min(*v), hi.max(*v)),
            );
            let half = (max - min) / 2.0;
            Some(rp::Collider::compound(vec![(
                (min + max) / 2.0,
                Quat::IDENTITY,
                rp::Collider::cuboid(half.x.max(1e-4), half.y.max(1e-4), half.z.max(1e-4)),
            )]))
        }
    }
}

/// Applies the queued mesh operations in the order the plugins made them.
pub fn sync(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    mut state: ResMut<PluginMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    images: Option<Res<AssetServer>>,
    link: Option<Res<ComputeLink>>,
    mut quad_materials: Option<ResMut<Assets<crate::plugin_quads::QuadMaterial>>>,
    mut buffers: Option<ResMut<Assets<bevy::render::storage::ShaderBuffer>>>,
) {
    if engine.plugins.meshes.is_empty() {
        return;
    }
    let operations = std::mem::take(&mut engine.plugins.meshes);
    let incoming: HashSet<_> = operations
        .iter()
        .filter_map(|op| match op {
            MeshOp::Put { plugin, mesh } if mesh.transition_ms > 0 => Some(plugin.clone()),
            _ => None,
        })
        .collect();
    let transitioning: HashSet<_> = operations
        .iter()
        .filter_map(|op| {
            let (plugin, name) = match op {
                MeshOp::Remove { plugin, name } => (plugin, name),
                MeshOp::Put { plugin, mesh } => (plugin, &mesh.name),
                _ => return None,
            };
            (incoming.contains(plugin)
                && state
                    .transitions
                    .get(&(plugin.clone(), name.clone()))
                    .is_some_and(|ms| *ms > 0))
            .then(|| plugin.clone())
        })
        .collect();
    if !transitioning.is_empty() {
        for entity in state.retired.drain(..) {
            commands.entity(entity).despawn();
        }
    }
    for op in operations {
        match op {
            MeshOp::Put { plugin, mesh } => {
                if let Some(old) = state.live.remove(&(plugin.clone(), mesh.name.clone())) {
                    if transitioning.contains(&plugin) && mesh.transition_ms > 0 {
                        commands.entity(old).insert(Crossfade {
                            elapsed: 0.0,
                            duration: mesh.transition_ms as f32 / 1000.0,
                            retire: true,
                        });
                        state.retired.push(old);
                    } else {
                        commands.entity(old).despawn();
                    }
                }
                let emission = mesh.emission.unwrap_or([0.0; 3]);
                let key = (
                    [
                        mesh.roughness.to_bits(),
                        emission[0].to_bits(),
                        emission[1].to_bits(),
                        emission[2].to_bits(),
                    ],
                    mesh.texture.clone(),
                );
                // A textured surface loads its project asset once and shares
                // the handle; a missing file keeps the vertex colors alone.
                let texture_handle = mesh.texture.as_ref().and_then(|path| {
                    state.textures.get(path).cloned().or_else(|| {
                        images
                            .as_ref()
                            .map(|server| server.load::<Image>(path.clone()))
                            .inspect(|handle| {
                                state.textures.insert(path.clone(), handle.clone());
                            })
                    })
                });
                let material = state
                    .materials
                    .entry(key)
                    .or_insert_with(|| {
                        materials.add(StandardMaterial {
                            base_color: Color::WHITE,
                            base_color_texture: texture_handle.clone(),
                            perceptual_roughness: mesh.roughness.clamp(0.05, 1.0),
                            emissive: LinearRgba::rgb(emission[0], emission[1], emission[2]),
                            ..default()
                        })
                    })
                    .clone();
                if let Some((old, _)) = state.assets.get(&(plugin.clone(), mesh.name.clone()))
                    && let Some(link) = &link
                {
                    link.unbind_mesh(old.id());
                }
                state.quads.remove(&(plugin.clone(), mesh.name.clone()));
                let compact = mesh
                    .gpu
                    .as_ref()
                    .and_then(|g| g.quads.as_ref())
                    .filter(|_| {
                        mesh.texture.is_none()
                            && mesh.uvs.is_empty()
                            && link
                                .as_ref()
                                .is_none_or(|link| link.available() != Some(false))
                    });
                let compact = compact.zip(quad_materials.as_mut()).zip(buffers.as_mut());
                let quad = compact.map(|((q, materials), buffers)| {
                    let base = StandardMaterial {
                        base_color: Color::WHITE,
                        perceptual_roughness: mesh.roughness.clamp(0.05, 1.0),
                        emissive: LinearRgba::rgb(emission[0], emission[1], emission[2]),
                        ..default()
                    };
                    (
                        crate::plugin_quads::mesh(q),
                        materials.add(crate::plugin_quads::QuadMaterial {
                            base,
                            extension: crate::plugin_quads::surface(q, buffers),
                        }),
                        crate::plugin_quads::bounds(&mesh),
                    )
                });
                let handle = meshes.add(
                    quad.as_ref()
                        .map_or_else(|| raster_mesh(&mesh), |q| q.0.clone()),
                );
                if let Some(gpu) = &mesh.gpu
                    && gpu.quads.is_none()
                    && let Some(link) = &link
                {
                    link.bind_mesh(handle.id(), &plugin, &gpu.buffer, gpu.vertices);
                }
                state.assets.insert(
                    (plugin.clone(), mesh.name.clone()),
                    (handle.clone(), material.clone()),
                );
                let mut entity = commands.spawn((
                    PluginMesh,
                    if state.hidden.contains(&(plugin.clone(), mesh.name.clone())) {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    },
                    Mesh3d(handle),
                    MeshMaterial3d(material),
                    Transform::from_translation(Vec3::from(mesh.origin)),
                ));
                if let Some((_, material, bounds)) = quad {
                    entity
                        .remove::<MeshMaterial3d<StandardMaterial>>()
                        .insert((MeshMaterial3d(material.clone()), bounds));
                    state
                        .quads
                        .insert((plugin.clone(), mesh.name.clone()), (material, bounds));
                }
                if let Some(body) = &mesh.body {
                    entity.insert(Transform {
                        translation: Vec3::from(mesh.origin),
                        rotation: Quat::from_array(body.rotation),
                        ..default()
                    });
                }
                if mesh.collider
                    && let Some(shape) = collider(&mesh)
                {
                    if let Some(body) = &mesh.body {
                        entity.insert((
                            rp::RigidBody::Dynamic,
                            shape,
                            rp::ColliderMassProperties::Mass(body.mass),
                            rp::Velocity {
                                linear: Vec3::from(body.velocity),
                                angular: Vec3::from(body.angular_velocity),
                            },
                        ));
                    } else {
                        entity.insert((rp::RigidBody::Fixed, shape));
                    }
                }
                if transitioning.contains(&plugin) && mesh.transition_ms > 0 {
                    entity.insert(Crossfade {
                        elapsed: 0.0,
                        duration: mesh.transition_ms as f32 / 1000.0,
                        retire: false,
                    });
                }
                state
                    .transitions
                    .insert((plugin.clone(), mesh.name.clone()), mesh.transition_ms);
                state.live.insert((plugin, mesh.name), entity.id());
            }
            MeshOp::Visibility {
                plugin,
                name,
                visible,
            } => {
                let key = (plugin, name);
                if let Some(&entity) = state.live.get(&key) {
                    if visible {
                        state.hidden.remove(&key);
                    } else {
                        state.hidden.insert(key);
                    }
                    commands.entity(entity).insert(if visible {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    });
                }
            }
            MeshOp::Remove { plugin, name } => {
                state.quads.remove(&(plugin.clone(), name.clone()));
                state.hidden.remove(&(plugin.clone(), name.clone()));
                if let Some((old, _)) = state.assets.remove(&(plugin.clone(), name.clone()))
                    && let Some(link) = &link
                {
                    link.unbind_mesh(old.id());
                }
                let key = (plugin.clone(), name);
                let duration = state.transitions.remove(&key).unwrap_or(0);
                if let Some(old) = state.live.remove(&key) {
                    if transitioning.contains(&plugin) && duration > 0 {
                        commands.entity(old).insert(Crossfade {
                            elapsed: 0.0,
                            duration: duration as f32 / 1000.0,
                            retire: true,
                        });
                        state.retired.push(old);
                    } else {
                        commands.entity(old).despawn();
                    }
                }
            }
            MeshOp::Instances { plugin, set } => {
                let key = (plugin.clone(), set.name.clone());
                for old in state.instances.remove(&key).unwrap_or_default() {
                    commands.entity(old).despawn();
                }
                let Some((mesh, material)) = state.assets.get(&(plugin.clone(), set.mesh.clone()))
                else {
                    bridge::send(&RuntimeMessage::Error {
                        actor: String::new(),
                        message: format!(
                            "{plugin}: instances {} name the mesh {}, which was never drawn",
                            set.name, set.mesh
                        ),
                    });
                    continue;
                };
                let copies: Vec<Entity> = (0..set.count())
                    .map(|i| {
                        let at = &set.positions[i * 3..i * 3 + 3];
                        let mut copy = commands.spawn((
                            PluginInstance,
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material.clone()),
                            Transform {
                                translation: Vec3::new(at[0], at[1], at[2]),
                                rotation: Quat::from_rotation_y(
                                    set.yaw.get(i).copied().unwrap_or(0.0),
                                ),
                                scale: Vec3::splat(set.scales.get(i).copied().unwrap_or(1.0)),
                            },
                        ));
                        if let Some((material, bounds)) =
                            state.quads.get(&(plugin.clone(), set.mesh.clone()))
                        {
                            copy.remove::<MeshMaterial3d<StandardMaterial>>()
                                .insert((MeshMaterial3d(material.clone()), *bounds));
                        }
                        copy.id()
                    })
                    .collect();
                state.instances.insert(key, copies);
            }
            MeshOp::RemoveInstances { plugin, name } => {
                for old in state.instances.remove(&(plugin, name)).unwrap_or_default() {
                    commands.entity(old).despawn();
                }
            }
            MeshOp::Clear => {
                for (_, old) in state.live.drain() {
                    commands.entity(old).despawn();
                }
                for (_, copies) in state.instances.drain() {
                    for old in copies {
                        commands.entity(old).despawn();
                    }
                }
                if let Some(link) = &link {
                    for (mesh, _) in state.assets.values() {
                        link.unbind_mesh(mesh.id());
                    }
                }
                for entity in state.retired.drain(..) {
                    commands.entity(entity).despawn();
                }
                state.transitions.clear();
                state.hidden.clear();
                state.assets.clear();
                state.materials.clear();
                state.quads.clear();
            }
        }
    }
}

/// Publish moving plugin mesh poses for checkpoints and continued editing.
#[cfg(feature = "plugins")]
pub fn publish_poses(state: Res<PluginMeshes>, query: Query<(&Transform, Option<&rp::Velocity>)>) {
    let poses = state.live.iter().filter_map(|(key, entity)| {
        let (pose, velocity) = query.get(*entity).ok()?;
        Some((key.clone(), serde_json::json!({"position":pose.translation.to_array(), "rotation":pose.rotation.to_array(),
            "velocity":velocity.map_or([0.0;3], |v| v.linear.to_array()),
            "angular_velocity":velocity.map_or([0.0;3], |v| v.angular.to_array())})))
    }).collect();
    crate::plugin_services::publish_mesh_poses(poses);
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Mode;
    use blockloom_plugin_api::rendering::InstanceData;

    fn cube(name: &str, origin: [f32; 3], collider: bool) -> MeshData {
        // One quad facing +y.
        MeshData {
            name: name.to_string(),
            positions: vec![0., 1., 0., 1., 1., 0., 1., 1., 1., 0., 1., 1.],
            normals: [0., 1., 0.].repeat(4),
            colors: [1.0, 0.5, 0.25, 1.0].repeat(4),
            indices: vec![0, 2, 1, 0, 3, 2],
            origin,
            emission: None,
            roughness: 0.9,
            transition_ms: 0,
            collider,
            collider_kind: ColliderKind::Trimesh,
            gpu: None,
            uvs: Vec::new(),
            texture: None,
            body: None,
        }
    }

    fn app() -> App {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        app.insert_non_send(Engine::new(rx, Mode::ThreeD))
            .init_resource::<PluginMeshes>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_systems(Update, sync);
        app
    }

    fn push(app: &mut App, ops: Vec<MeshOp>) {
        app.world_mut()
            .non_send_mut::<Engine>()
            .plugins
            .meshes
            .extend(ops);
        app.update();
    }

    fn put(mesh: MeshData) -> MeshOp {
        MeshOp::Put {
            plugin: "p".to_string(),
            mesh,
        }
    }

    fn count<F: bevy::ecs::query::QueryFilter>(app: &mut App) -> usize {
        app.world_mut()
            .query_filtered::<(), F>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn cut_crossfades_keep_one_previous_cut_and_clear_retired_entities() {
        let mut app = app();
        let mut a = cube("visual/coarse", [0.0; 3], false);
        a.transition_ms = 150;
        push(&mut app, vec![put(a)]);
        assert_eq!(count::<With<Crossfade>>(&mut app), 0);
        let mut b = cube("visual/fine", [0.0; 3], false);
        b.transition_ms = 150;
        push(
            &mut app,
            vec![
                put(b.clone()),
                MeshOp::Remove {
                    plugin: "p".into(),
                    name: "visual/coarse".into(),
                },
            ],
        );
        assert_eq!(count::<With<PluginMesh>>(&mut app), 2);
        assert_eq!(count::<With<Crossfade>>(&mut app), 2);
        b.name = "visual/next".into();
        push(
            &mut app,
            vec![
                put(b),
                MeshOp::Remove {
                    plugin: "p".into(),
                    name: "visual/fine".into(),
                },
            ],
        );
        assert_eq!(count::<With<PluginMesh>>(&mut app), 2);
        assert_eq!(app.world().resource::<PluginMeshes>().retired.len(), 1);
        push(&mut app, vec![MeshOp::Clear]);
        assert_eq!(count::<With<PluginMesh>>(&mut app), 0);
        assert!(app.world().resource::<PluginMeshes>().retired.is_empty());
    }

    #[test]
    fn crossfade_masks_are_complementary_and_retire_after_the_duration() {
        let mut app = app();
        let mut coarse = cube("coarse", [0.0; 3], false);
        coarse.transition_ms = 150;
        push(&mut app, vec![put(coarse)]);
        let mut fine = cube("fine", [0.0; 3], false);
        fine.transition_ms = 150;
        push(
            &mut app,
            vec![
                put(fine),
                MeshOp::Remove {
                    plugin: "p".into(),
                    name: "coarse".into(),
                },
            ],
        );
        app.insert_resource(Time::<()>::default())
            .add_systems(PostUpdate, crossfade);
        app.world_mut().spawn((
            Camera3d::default(),
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 5.0)),
        ));
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(75));
        app.update();
        let mut ranges = app
            .world_mut()
            .query::<(&Crossfade, &bevy::camera::visibility::VisibilityRange)>();
        let ranges: Vec<_> = ranges
            .iter(app.world())
            .map(|(fade, range)| {
                (
                    fade.retire,
                    if fade.retire {
                        range.end_margin.clone()
                    } else {
                        range.start_margin.clone()
                    },
                )
            })
            .collect();
        assert_eq!(ranges.len(), 2);
        assert!(
            ranges.iter().all(
                |(_, range)| (range.start - 4.5).abs() < 1e-5 && (range.end - 5.5).abs() < 1e-5
            )
        );
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(75));
        app.update();
        assert_eq!(count::<With<Crossfade>>(&mut app), 0);
        assert_eq!(count::<With<PluginMesh>>(&mut app), 1);
        assert!(app.world().resource::<PluginMeshes>().retired.is_empty());
    }

    #[test]
    fn compact_assets_share_records_with_instances_and_release_on_clear() {
        let mut app = app();
        app.init_resource::<Assets<crate::plugin_quads::QuadMaterial>>()
            .init_resource::<Assets<bevy::render::storage::ShaderBuffer>>();
        let mut data = cube("quad", [0.0; 3], true);
        data.gpu = Some(blockloom_plugin_api::mesh::GpuVertices {
            buffer: "unused".into(),
            vertices: 6,
            quads: Some(blockloom_plugin_api::mesh::CompactQuads {
                records: vec![[2u32 << 24 | 1, 1 | 1 << 8]],
                palette: vec![[1.0; 4]],
                voxel: 1.0,
            }),
        });
        push(
            &mut app,
            vec![
                put(data),
                MeshOp::Instances {
                    plugin: "p".into(),
                    set: InstanceData {
                        name: "copies".into(),
                        mesh: "quad".into(),
                        positions: vec![2.0, 0.0, 0.0],
                        yaw: vec![],
                        scales: vec![],
                    },
                },
            ],
        );
        assert_eq!(
            count::<With<MeshMaterial3d<crate::plugin_quads::QuadMaterial>>>(&mut app),
            2
        );
        assert_eq!(count::<With<rp::Collider>>(&mut app), 1);
        let state = app.world().resource::<PluginMeshes>();
        let mesh = &state.assets[&("p".into(), "quad".into())].0;
        let mesh = app.world().resource::<Assets<Mesh>>().get(mesh).unwrap();
        assert!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_none());
        assert_eq!(mesh.count_vertices(), 4);
        push(&mut app, vec![MeshOp::Clear]);
        assert_eq!(count::<With<Mesh3d>>(&mut app), 0);
        assert!(app.world().resource::<PluginMeshes>().quads.is_empty());
    }

    #[test]
    fn moving_meshes_have_mass_and_pose_and_gpu_fallback_has_the_fixed_capacity() {
        use blockloom_plugin_api::mesh::{GpuVertices, MeshBody};
        let mut app = app();
        let mut mesh = cube("piece", [3.0, 4.0, 5.0], true);
        mesh.positions = vec![0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.];
        mesh.indices = vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3];
        mesh.collider_kind = ColliderKind::ConvexHull;
        mesh.body = Some(MeshBody {
            mass: 2500.0,
            velocity: [2.0, 0.0, 0.0],
            angular_velocity: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
        });
        mesh.gpu = Some(GpuVertices {
            buffer: "vertices1".into(),
            vertices: 24,
            quads: None,
        });
        mesh.check().unwrap();
        let raster = raster_mesh(&mesh);
        assert_eq!(raster.count_vertices(), 24);
        assert!(raster.indices().is_none());
        let mut layouts = bevy::mesh::MeshVertexBufferLayouts::default();
        let layout = raster.get_mesh_vertex_buffer_layout(&mut layouts);
        assert_eq!(layout.0.layout().array_stride, 40);
        push(&mut app, vec![put(mesh)]);
        let (body, mass, velocity, pose) = app
            .world_mut()
            .query::<(
                &rp::RigidBody,
                &rp::ColliderMassProperties,
                &rp::Velocity,
                &Transform,
            )>()
            .single(app.world())
            .unwrap();
        assert_eq!(*body, rp::RigidBody::Dynamic);
        assert_eq!(*mass, rp::ColliderMassProperties::Mass(2500.0));
        assert_eq!(velocity.linear, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(pose.translation, Vec3::new(3.0, 4.0, 5.0));
    }

    #[test]
    fn a_mesh_becomes_one_entity_that_a_same_named_mesh_replaces() {
        let mut app = app();
        push(&mut app, vec![put(cube("a", [0.0; 3], false))]);
        assert_eq!(count::<With<PluginMesh>>(&mut app), 1);
        assert_eq!(count::<With<rp::Collider>>(&mut app), 0);
        push(&mut app, vec![put(cube("a", [4.0, 0.0, 0.0], true))]);
        assert_eq!(count::<With<PluginMesh>>(&mut app), 1);
        assert_eq!(count::<With<rp::Collider>>(&mut app), 1);
        let x = app
            .world_mut()
            .query::<(&PluginMesh, &Transform)>()
            .single(app.world())
            .unwrap()
            .1
            .translation
            .x;
        assert_eq!(x, 4.0);
        push(&mut app, vec![put(cube("b", [0.0; 3], false))]);
        assert_eq!(count::<With<PluginMesh>>(&mut app), 2);
        assert_eq!(app.world().resource::<PluginMeshes>().len(), 2);
    }

    #[test]
    fn visibility_changes_preserve_colliders_and_replacements() {
        let mut app = app();
        let visibility = |visible| MeshOp::Visibility {
            plugin: "p".into(),
            name: "a".into(),
            visible,
        };
        push(
            &mut app,
            vec![put(cube("a", [0.0; 3], true)), visibility(false)],
        );
        assert_eq!(count::<With<rp::Collider>>(&mut app), 1);
        assert_eq!(
            *app.world_mut()
                .query_filtered::<&Visibility, With<PluginMesh>>()
                .single(app.world())
                .unwrap(),
            Visibility::Hidden
        );
        push(&mut app, vec![put(cube("a", [1.0; 3], true))]);
        assert_eq!(
            *app.world_mut()
                .query_filtered::<&Visibility, With<PluginMesh>>()
                .single(app.world())
                .unwrap(),
            Visibility::Hidden
        );
        push(&mut app, vec![visibility(true)]);
        assert_eq!(count::<With<rp::Collider>>(&mut app), 1);
        assert_eq!(
            *app.world_mut()
                .query_filtered::<&Visibility, With<PluginMesh>>()
                .single(app.world())
                .unwrap(),
            Visibility::Inherited
        );
        push(
            &mut app,
            vec![
                visibility(false),
                MeshOp::Clear,
                put(cube("a", [0.0; 3], true)),
            ],
        );
        assert_eq!(
            *app.world_mut()
                .query_filtered::<&Visibility, With<PluginMesh>>()
                .single(app.world())
                .unwrap(),
            Visibility::Inherited
        );
    }

    #[test]
    fn remove_and_clear_take_meshes_out() {
        let mut app = app();
        push(
            &mut app,
            vec![
                put(cube("a", [0.0; 3], false)),
                put(cube("b", [0.0; 3], false)),
                MeshOp::Remove {
                    plugin: "p".to_string(),
                    name: "a".to_string(),
                },
            ],
        );
        assert_eq!(count::<With<PluginMesh>>(&mut app), 1);
        push(&mut app, vec![MeshOp::Clear]);
        assert_eq!(count::<With<PluginMesh>>(&mut app), 0);
        assert!(app.world().resource::<PluginMeshes>().is_empty());
    }

    #[test]
    fn instances_share_their_meshs_buffers_and_go_with_it() {
        let mut app = app();
        let set = |name: &str, n: usize| MeshOp::Instances {
            plugin: "p".to_string(),
            set: InstanceData {
                name: name.to_string(),
                mesh: "a".to_string(),
                positions: (0..n * 3).map(|i| i as f32).collect(),
                yaw: vec![0.5; n],
                scales: vec![],
            },
        };
        push(
            &mut app,
            vec![put(cube("a", [0.0; 3], false)), set("rows", 50)],
        );
        assert_eq!(count::<With<PluginInstance>>(&mut app), 50);
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 1);
        // The same name replaces the set.
        push(&mut app, vec![set("rows", 10)]);
        assert_eq!(count::<With<PluginInstance>>(&mut app), 10);
        // A set naming a mesh nobody drew makes nothing.
        push(
            &mut app,
            vec![MeshOp::Instances {
                plugin: "q".to_string(),
                set: InstanceData {
                    name: "x".to_string(),
                    mesh: "a".to_string(),
                    positions: vec![0.0; 3],
                    yaw: vec![],
                    scales: vec![],
                },
            }],
        );
        assert_eq!(count::<With<PluginInstance>>(&mut app), 10);
        push(
            &mut app,
            vec![MeshOp::RemoveInstances {
                plugin: "p".to_string(),
                name: "rows".to_string(),
            }],
        );
        assert_eq!(count::<With<PluginInstance>>(&mut app), 0);
        push(&mut app, vec![set("rows", 3), MeshOp::Clear]);
        assert_eq!(count::<With<PluginInstance>>(&mut app), 0);
        assert!(app.world().resource::<PluginMeshes>().is_empty());
    }

    #[test]
    fn meshes_with_the_same_look_share_a_material() {
        let mut app = app();
        let mut glow = cube("g", [0.0; 3], false);
        glow.emission = Some([4.0, 1.0, 0.0]);
        push(
            &mut app,
            vec![
                put(cube("a", [0.0; 3], false)),
                put(cube("b", [1.0, 0.0, 0.0], false)),
                put(glow),
            ],
        );
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 2);
    }
}
