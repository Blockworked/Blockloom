//! The mesh submission service: meshes plugins ask the world to draw.
//!
//! A plugin's `mesh` effect names a mesh; this turns it into one entity
//! (`Mesh3d`, a shared standard material keyed by roughness and emission, and
//! a fixed trimesh collider when it is solid), and a later mesh of the same
//! name replaces it. All of a plugin's meshes go when the run ends. 3D only.

use crate::engine::Engine;
use crate::plugins::MeshOp;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_plugin_api::mesh::MeshData;
use std::collections::HashMap;

/// Marks the entity a plugin's mesh became.
#[derive(Component)]
pub struct PluginMesh;

/// Which entity holds each (plugin, name), and the materials in use.
#[derive(Resource, Default)]
pub struct PluginMeshes {
    live: HashMap<(String, String), Entity>,
    materials: HashMap<[u32; 4], Handle<StandardMaterial>>,
}

#[cfg(test)]
impl PluginMeshes {
    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

fn bevy_mesh(data: &MeshData) -> Mesh {
    Mesh::new(
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
    .with_inserted_indices(Indices::U32(data.indices.clone()))
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
    rp::Collider::trimesh(vertices, triangles).ok()
}

/// Applies the queued mesh operations in the order the plugins made them.
pub fn sync(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    mut state: ResMut<PluginMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if engine.plugins.meshes.is_empty() {
        return;
    }
    for op in std::mem::take(&mut engine.plugins.meshes) {
        match op {
            MeshOp::Put { plugin, mesh } => {
                if let Some(old) = state.live.remove(&(plugin.clone(), mesh.name.clone())) {
                    commands.entity(old).despawn();
                }
                let emission = mesh.emission.unwrap_or([0.0; 3]);
                let key = [
                    mesh.roughness.to_bits(),
                    emission[0].to_bits(),
                    emission[1].to_bits(),
                    emission[2].to_bits(),
                ];
                let material = state
                    .materials
                    .entry(key)
                    .or_insert_with(|| {
                        materials.add(StandardMaterial {
                            base_color: Color::WHITE,
                            perceptual_roughness: mesh.roughness.clamp(0.05, 1.0),
                            emissive: LinearRgba::rgb(emission[0], emission[1], emission[2]),
                            ..default()
                        })
                    })
                    .clone();
                let mut entity = commands.spawn((
                    PluginMesh,
                    Mesh3d(meshes.add(bevy_mesh(&mesh))),
                    MeshMaterial3d(material),
                    Transform::from_translation(Vec3::from(mesh.origin)),
                ));
                if mesh.collider
                    && let Some(shape) = collider(&mesh)
                {
                    entity.insert((rp::RigidBody::Fixed, shape));
                }
                state.live.insert((plugin, mesh.name), entity.id());
            }
            MeshOp::Remove { plugin, name } => {
                if let Some(old) = state.live.remove(&(plugin, name)) {
                    commands.entity(old).despawn();
                }
            }
            MeshOp::Clear => {
                for (_, old) in state.live.drain() {
                    commands.entity(old).despawn();
                }
                state.materials.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::scene::Mode;

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
            collider,
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
