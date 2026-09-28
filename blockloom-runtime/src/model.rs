//! Model looks. A `Visual::Model` actor draws its glTF file's first scene as
//! a child entity, scaled by the look's `scale`, and loops one of the file's
//! animations on the rig. The authored box stands in while the file loads and
//! stays when it won't, and it is always what the actor collides as.

use crate::engine::Engine;
use crate::materials::GraphMaterial3d;
use bevy::asset::AssetId;
use bevy::gltf::{Gltf, GltfAssetLabel};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady};
use blockloom_core::scene::Visual;
use blockloom_protocol::RuntimeMessage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The child entity a model's scene spawns under, and what it came from.
#[derive(Component)]
pub struct ModelScene {
    actor: Entity,
    id: String,
    source: String,
    gltf: Handle<Gltf>,
    animation: String,
    reported: bool,
}

/// On the actor: the child its model scene hangs off.
#[derive(Component)]
pub struct ModelChild(pub Entity);

/// Keeps every model the world has drawn loaded, so the rebuild after each
/// edit reuses the file instead of reading it again with a box in between.
/// A file whose modified time moved is reloaded.
#[derive(Resource, Default)]
pub struct ModelCache(HashMap<PathBuf, (SystemTime, Handle<Gltf>, Handle<WorldAsset>)>);

/// Whether the runtime can draw `path` as a scene. OBJ and FBX have no Bevy
/// loader, so they keep the box.
pub fn is_gltf(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".gltf") || lower.ends_with(".glb")
}

/// Start drawing `visual`'s file under `entity`. Answers whether the scene is
/// already loaded, in which case the box never needs to show.
pub fn attach(
    commands: &mut Commands,
    entity: Entity,
    actor: &str,
    visual: &Visual,
    dir: Option<&Path>,
    assets: &AssetServer,
) -> bool {
    let Visual::Model {
        path,
        scale,
        animation,
        ..
    } = visual
    else {
        return false;
    };
    let source = path.trim();
    if source.is_empty() {
        return false;
    }
    if !is_gltf(source) {
        tracing::warn!("{source}: only glTF and GLB models draw in the game; it shows as its box");
        return false;
    }
    let file = crate::world::asset_path(dir, source);
    let gltf: Handle<Gltf> = assets.load(file.clone());
    let scene: Handle<WorldAsset> = assets.load(GltfAssetLabel::Scene(0).from_asset(file.clone()));
    let loaded = assets.is_loaded_with_dependencies(&scene);
    keep_loaded(commands, file, gltf.clone(), scene.clone());
    let child = commands
        .spawn((
            WorldAssetRoot(scene),
            Transform::from_scale(Vec3::from(*scale)),
            ModelScene {
                actor: entity,
                id: actor.to_string(),
                source: source.to_string(),
                gltf,
                animation: animation.trim().to_string(),
                reported: false,
            },
        ))
        .observe(on_ready)
        .id();
    commands
        .entity(entity)
        .add_child(child)
        .insert(ModelChild(child));
    if loaded {
        commands.entity(entity).remove::<Mesh3d>();
    }
    loaded
}

/// Take the model off an actor whose look is going away.
pub fn detach(commands: &mut Commands, entity: Entity, children: &Query<&ModelChild>) {
    if let Ok(child) = children.get(entity) {
        commands.entity(child.0).despawn();
        commands.entity(entity).remove::<ModelChild>();
    }
}

fn keep_loaded(
    commands: &mut Commands,
    file: PathBuf,
    gltf: Handle<Gltf>,
    scene: Handle<WorldAsset>,
) {
    commands.queue(move |world: &mut World| {
        let modified = std::fs::metadata(&file)
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let Some(server) = world.get_resource::<AssetServer>().cloned() else {
            return;
        };
        let Some(mut cache) = world.get_resource_mut::<ModelCache>() else {
            return;
        };
        let stale = cache
            .0
            .get(&file)
            .is_some_and(|(seen, ..)| *seen != modified);
        cache.0.insert(file.clone(), (modified, gltf, scene));
        if stale {
            server.reload(file);
            // The same mesh handles can name new bytes after a reload,
            // so decimated copies must be rebuilt from them.
            if let Some(mut lod) = world.get_resource_mut::<ModelLodCache>() {
                lod.0.clear();
            }
        }
    });
}

/// The scene is in the world: drop the stand-in box, drop any camera the
/// file brought (the world has one), and start the rig.
#[allow(clippy::too_many_arguments)]
fn on_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    scenes: Query<&ModelScene>,
    children: Query<&Children>,
    cameras: Query<(), With<Camera>>,
    mut players: Query<&mut AnimationPlayer>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Ok(scene) = scenes.get(ready.entity) else {
        return;
    };
    if let Ok(mut actor) = commands.get_entity(scene.actor) {
        actor.remove::<(
            Mesh3d,
            MeshMaterial3d<StandardMaterial>,
            MeshMaterial3d<GraphMaterial3d>,
            MeshMaterial3d<crate::materials::BoxMaterial>,
            MeshMaterial3d<crate::batching::InstancedMaterial>,
            bevy::mesh::MeshTag,
            crate::batching::InstanceSlot,
        )>();
    }
    for entity in children.iter_descendants(ready.entity) {
        if cameras.contains(entity) {
            commands.entity(entity).despawn();
        }
    }
    let Some(gltf) = gltfs.get(&scene.gltf) else {
        return;
    };
    let clip = if scene.animation.is_empty() {
        gltf.animations.first().cloned()
    } else {
        let found = gltf.named_animations.get(scene.animation.as_str()).cloned();
        if found.is_none() {
            let mut names: Vec<&str> = gltf.named_animations.keys().map(|name| &**name).collect();
            names.sort_unstable();
            report(
                &scene.id,
                format!(
                    "{} has no animation called \"{}\" (it has: {})",
                    scene.source,
                    scene.animation,
                    if names.is_empty() {
                        "none".to_string()
                    } else {
                        names.join(", ")
                    }
                ),
            );
        }
        found
    };
    let Some(clip) = clip else {
        return;
    };
    let (graph, index) = AnimationGraph::from_clip(clip);
    let graph = graphs.add(graph);
    for entity in children.iter_descendants(ready.entity) {
        if let Ok(mut player) = players.get_mut(entity) {
            player.play(index).repeat();
            commands
                .entity(entity)
                .insert(AnimationGraphHandle(graph.clone()));
        }
    }
}

/// Say so once when a model file won't load; the box keeps standing in.
pub fn watch_models(mut scenes: Query<&mut ModelScene>, assets: Res<AssetServer>) {
    for mut scene in &mut scenes {
        if scene.reported {
            continue;
        }
        if let bevy::asset::LoadState::Failed(error) = assets.load_state(&scene.gltf) {
            scene.reported = true;
            report(
                &scene.id,
                format!("couldn't load the model {}: {error}", scene.source),
            );
        }
    }
}

/// Rigs hold still while the game is paused and move otherwise, including in
/// the scene view, so an idle loop shows while editing.
pub fn pause_rigs(engine: NonSend<Engine>, mut players: Query<&mut AnimationPlayer>) {
    let paused = engine.running && engine.paused;
    for mut player in &mut players {
        if paused && !player.all_paused() {
            player.pause_all();
        } else if !paused && player.all_paused() {
            player.resume_all();
        }
    }
}

/// A LOD-culled placeholder hides nothing by itself: the loaded glTF scene
/// hangs off `ModelChild` as its own entities with their own visibility.
/// Level 0 shows the full scene, level 1 a decimated copy of every mesh
/// (see `simplify_mesh`), and past the last level the whole scene hides,
/// so a distant model sheds draws exactly like a culled box does.
pub fn sync_model_lod(
    actors: Query<(Entity, &crate::culling::LodGroup, &ModelChild)>,
    states: Query<&ModelSimplified>,
    descendants: Query<&Children>,
    mut meshes: Query<&mut Mesh3d>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut cache: ResMut<ModelLodCache>,
    mut visibility: Query<&mut Visibility>,
    mut commands: Commands,
) {
    for (actor, group, child) in &actors {
        let Ok(mut shown) = visibility.get_mut(child.0) else {
            continue;
        };
        let hidden = group.is_culled();
        let want = if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *shown != want {
            *shown = want;
        }
        if hidden {
            continue;
        }
        if group.current().is_some_and(|level| level > 0) {
            let known = states
                .get(actor)
                .map(|state| state.entries.clone())
                .unwrap_or_default();
            simplify_scene(
                actor,
                child.0,
                &known,
                &descendants,
                &mut meshes,
                &mut mesh_assets,
                &mut cache,
                &mut commands,
            );
        } else {
            let known = states.get(actor).ok().map(|state| state.entries.clone());
            restore_scene(actor, known, &mut meshes, &mut commands);
        }
    }
}

/// Swap every mesh under the model scene for its decimated copy,
/// remembering the originals on the actor for `restore_scene`.
fn simplify_scene(
    actor: Entity,
    scene: Entity,
    known: &[(Entity, Handle<Mesh>)],
    descendants: &Query<&Children>,
    meshes: &mut Query<&mut Mesh3d>,
    mesh_assets: &mut Assets<Mesh>,
    cache: &mut ModelLodCache,
    commands: &mut Commands,
) {
    let mut entries: Vec<(Entity, Handle<Mesh>)> = known.to_vec();
    let known_set: std::collections::HashSet<Entity> =
        entries.iter().map(|(entity, _)| *entity).collect();
    let mut changed = false;
    for entity in descendants.iter_descendants(scene) {
        if known_set.contains(&entity) {
            continue;
        }
        let Ok(mut mesh) = meshes.get_mut(entity) else {
            continue;
        };
        let original = mesh.0.clone();
        let simplified = match cache.0.get(&original.id()) {
            Some(handle) => handle.clone(),
            None => {
                let Some(source) = mesh_assets.get(&original) else {
                    continue;
                };
                let Some(thin) = simplify_mesh(source, SIMPLIFY_STRIDE) else {
                    continue;
                };
                let handle = mesh_assets.add(thin);
                cache.0.insert(original.id(), handle.clone());
                handle
            }
        };
        mesh.0 = simplified;
        entries.push((entity, original));
        changed = true;
    }
    if changed || known.is_empty() {
        commands.entity(actor).insert(ModelSimplified { entries });
    }
}

/// Put the full meshes back after `simplify_scene`.
fn restore_scene(
    actor: Entity,
    known: Option<Vec<(Entity, Handle<Mesh>)>>,
    meshes: &mut Query<&mut Mesh3d>,
    commands: &mut Commands,
) {
    let Some(state) = known else {
        return;
    };
    for (entity, original) in &state {
        if let Ok(mut mesh) = meshes.get_mut(*entity) {
            mesh.0 = original.clone();
        }
    }
    commands.entity(actor).remove::<ModelSimplified>();
}

/// Stride for the simplified model LOD: keep every Nth triangle.
pub const SIMPLIFY_STRIDE: u32 = 4;

/// Screen size where a loaded model drops to its decimated meshes.
/// Below `performance::PROP_CULL_SCREEN` it leaves the view entirely.
pub const SIMPLIFY_SCREEN: f32 = 0.05;

/// Decimate a triangle-list mesh by keeping every `stride`-th triangle
/// and remapping only the vertices it uses. Every full-length vertex
/// attribute is thinned the same way; short ones are dropped rather than
/// guessed at. Skinned and morphed meshes are refused: the decimated copy
/// would lose its joint attributes or morph targets while the entity kept
/// its `SkinnedMesh`/`MorphWeights`, and Bevy would then bind a skinned or
/// morphed bind group against an unskinned pipeline. Returns `None` for
/// non-triangle meshes or ones too small to thin.
pub fn simplify_mesh(source: &Mesh, stride: u32) -> Option<Mesh> {
    if source.primitive_topology() != PrimitiveTopology::TriangleList || stride < 2 {
        return None;
    }
    // A decimated copy drops these, so leave the full mesh in place.
    if source.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_some()
        || source.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some()
        || source.morph_targets().is_some()
    {
        return None;
    }
    let Some(VertexAttributeValues::Float32x3(positions)) =
        source.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let count = positions.len() as u32;
    if count == 0 {
        return None;
    }
    let indices: Vec<u32> = match source.indices() {
        Some(indices) => indices.iter().map(|i| i as u32).collect(),
        None => (0..count).collect(),
    };
    if !indices.len().is_multiple_of(3) || indices.iter().any(|&i| i >= count) {
        return None;
    }
    let kept: Vec<u32> = indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .filter(|(tri, _)| (*tri as u32).is_multiple_of(stride))
        .flat_map(|(_, tri)| tri.iter().copied())
        .collect();
    // Too small to thin: fewer than two strides of triangles in.
    if kept.len() < 6 || kept.len() >= indices.len() {
        return None;
    }
    let mut remap: Vec<Option<u32>> = vec![None; count as usize];
    let mut next = 0u32;
    for &i in &kept {
        if remap[i as usize].is_none() {
            remap[i as usize] = Some(next);
            next += 1;
        }
    }
    let used: Vec<u32> = {
        let mut order = vec![u32::MAX; next as usize];
        for (old, slot) in remap.iter().enumerate() {
            if let Some(new) = slot {
                order[*new as usize] = old as u32;
            }
        }
        order
    };
    let mut out = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    for (name, values) in source.attributes() {
        if values.len() != count as usize {
            continue;
        }
        if let Some(thin) = select_rows(values, &used) {
            out.insert_attribute(*name, thin);
        }
    }
    out.attribute(Mesh::ATTRIBUTE_POSITION)?;
    let remapped: Vec<u32> = kept
        .iter()
        .map(|&i| remap[i as usize].unwrap_or(0))
        .collect();
    out.insert_indices(Indices::U32(remapped));
    out.enable_raytracing = source.enable_raytracing;
    Some(out)
}

/// The rows of one vertex attribute that `used` names, in LOD order.
fn select_rows(values: &VertexAttributeValues, used: &[u32]) -> Option<VertexAttributeValues> {
    macro_rules! thin {
        ($data:expr, $variant:ident) => {{
            Some(VertexAttributeValues::$variant(
                used.iter().map(|&i| $data[i as usize]).collect(),
            ))
        }};
    }
    match values {
        VertexAttributeValues::Uint8(data) => thin!(data, Uint8),
        VertexAttributeValues::Uint8x2(data) => thin!(data, Uint8x2),
        VertexAttributeValues::Uint8x4(data) => thin!(data, Uint8x4),
        VertexAttributeValues::Sint8(data) => thin!(data, Sint8),
        VertexAttributeValues::Sint8x2(data) => thin!(data, Sint8x2),
        VertexAttributeValues::Sint8x4(data) => thin!(data, Sint8x4),
        VertexAttributeValues::Unorm8(data) => thin!(data, Unorm8),
        VertexAttributeValues::Unorm8x2(data) => thin!(data, Unorm8x2),
        VertexAttributeValues::Unorm8x4(data) => thin!(data, Unorm8x4),
        VertexAttributeValues::Snorm8(data) => thin!(data, Snorm8),
        VertexAttributeValues::Snorm8x2(data) => thin!(data, Snorm8x2),
        VertexAttributeValues::Snorm8x4(data) => thin!(data, Snorm8x4),
        VertexAttributeValues::Uint16(data) => thin!(data, Uint16),
        VertexAttributeValues::Uint16x2(data) => thin!(data, Uint16x2),
        VertexAttributeValues::Uint16x4(data) => thin!(data, Uint16x4),
        VertexAttributeValues::Sint16(data) => thin!(data, Sint16),
        VertexAttributeValues::Sint16x2(data) => thin!(data, Sint16x2),
        VertexAttributeValues::Sint16x4(data) => thin!(data, Sint16x4),
        VertexAttributeValues::Unorm16(data) => thin!(data, Unorm16),
        VertexAttributeValues::Unorm16x2(data) => thin!(data, Unorm16x2),
        VertexAttributeValues::Unorm16x4(data) => thin!(data, Unorm16x4),
        VertexAttributeValues::Snorm16(data) => thin!(data, Snorm16),
        VertexAttributeValues::Snorm16x2(data) => thin!(data, Snorm16x2),
        VertexAttributeValues::Snorm16x4(data) => thin!(data, Snorm16x4),
        VertexAttributeValues::Float16(data) => thin!(data, Float16),
        VertexAttributeValues::Float16x2(data) => thin!(data, Float16x2),
        VertexAttributeValues::Float16x4(data) => thin!(data, Float16x4),
        VertexAttributeValues::Float32(data) => thin!(data, Float32),
        VertexAttributeValues::Sint32(data) => thin!(data, Sint32),
        VertexAttributeValues::Uint32(data) => thin!(data, Uint32),
        VertexAttributeValues::Float32x2(data) => thin!(data, Float32x2),
        VertexAttributeValues::Float32x3(data) => thin!(data, Float32x3),
        VertexAttributeValues::Float32x4(data) => thin!(data, Float32x4),
        VertexAttributeValues::Sint32x2(data) => thin!(data, Sint32x2),
        VertexAttributeValues::Sint32x3(data) => thin!(data, Sint32x3),
        VertexAttributeValues::Sint32x4(data) => thin!(data, Sint32x4),
        VertexAttributeValues::Uint32x2(data) => thin!(data, Uint32x2),
        VertexAttributeValues::Uint32x3(data) => thin!(data, Uint32x3),
        VertexAttributeValues::Uint32x4(data) => thin!(data, Uint32x4),
        VertexAttributeValues::Float64(data) => thin!(data, Float64),
        VertexAttributeValues::Float64x2(data) => thin!(data, Float64x2),
        VertexAttributeValues::Float64x3(data) => thin!(data, Float64x3),
        VertexAttributeValues::Float64x4(data) => thin!(data, Float64x4),
        VertexAttributeValues::Unorm10_10_10_2(data) => thin!(data, Unorm10_10_10_2),
        VertexAttributeValues::Unorm8x4Bgra(data) => thin!(data, Unorm8x4Bgra),
    }
}

/// Decimated copies of loaded model meshes, keyed by the original.
/// Cleared when a model file reloads, since the same handle can name
/// new bytes after that.
#[derive(Resource, Default)]
pub struct ModelLodCache(HashMap<AssetId<Mesh>, Handle<Mesh>>);

/// The full meshes a simplified actor swapped out, per scene entity.
#[derive(Component, Clone, Default)]
pub(crate) struct ModelSimplified {
    entries: Vec<(Entity, Handle<Mesh>)>,
}

fn report(actor: &str, message: String) {
    crate::bridge::send(&RuntimeMessage::Error {
        actor: actor.to_string(),
        message,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective_camera(app: &mut App) {
        app.world_mut().spawn((
            crate::world::WorldCamera,
            Camera::default(),
            GlobalTransform::default(),
            Projection::Perspective(PerspectiveProjection {
                fov: std::f32::consts::FRAC_PI_2,
                ..default()
            }),
        ));
    }

    #[test]
    fn culled_placeholders_hide_the_loaded_scene() {
        let mut app = App::new();
        app.init_resource::<crate::culling::LodPolicy>()
            .init_resource::<crate::culling::Culling>()
            .init_resource::<crate::quality::Scaling>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<ModelLodCache>()
            .add_systems(Update, (crate::culling::select_lod, sync_model_lod).chain());
        perspective_camera(&mut app);
        let child = app.world_mut().spawn(Visibility::Inherited).id();
        // A 2 m box bounds a 1.73 m sphere: visible at 5 m, gone at 200 m.
        let group = crate::culling::LodGroup::new(1.732).level(0.01, None);
        let actor = app
            .world_mut()
            .spawn((
                GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0)),
                group,
                ModelChild(child),
            ))
            .id();
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -200.0));
        app.update();
        assert!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .is_culled()
        );
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Hidden
        );
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0));
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
    }

    /// A `quads x quads` grid on the XZ plane, with normals and UVs.
    fn grid_mesh(quads: u32) -> Mesh {
        let n = quads + 1;
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        for z in 0..n {
            for x in 0..n {
                positions.push([x as f32, 0.0, z as f32]);
                normals.push([0.0, 1.0, 0.0]);
                uvs.push([x as f32 / quads as f32, z as f32 / quads as f32]);
            }
        }
        let mut indices = Vec::new();
        for z in 0..quads {
            for x in 0..quads {
                let a = z * n + x;
                indices.extend([a, a + 1, a + n, a + 1, a + n + 1, a + n]);
            }
        }
        Mesh::new(
            PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
    }

    #[test]
    fn simplify_mesh_thins_big_meshes_and_keeps_attributes() {
        let source = grid_mesh(10);
        let tris_in = source.indices().unwrap().len() / 3;
        assert_eq!(tris_in, 200);
        let thin = simplify_mesh(&source, 4).unwrap();
        let indices = match thin.indices().unwrap() {
            Indices::U32(indices) => indices.clone(),
            _ => panic!("simplified mesh stays 32-bit indexed"),
        };
        assert_eq!(indices.len() / 3, tris_in / 4);
        let positions = match thin.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            VertexAttributeValues::Float32x3(positions) => positions.len(),
            _ => panic!("positions stay float3"),
        };
        let normals = match thin.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() {
            VertexAttributeValues::Float32x3(normals) => normals.len(),
            _ => panic!("normals stay float3"),
        };
        let uvs = match thin.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() {
            VertexAttributeValues::Float32x2(uvs) => uvs.len(),
            _ => panic!("uvs stay float2"),
        };
        assert_eq!(normals, positions);
        assert_eq!(uvs, positions);
        assert!(positions < 121);
        assert!(indices.iter().all(|&i| (i as usize) < positions));
        assert!(thin.attribute(Mesh::ATTRIBUTE_POSITION).is_some());
    }

    #[test]
    fn simplify_mesh_refuses_tiny_or_non_triangle_meshes() {
        assert!(simplify_mesh(&grid_mesh(1), 4).is_none());
        let lines = Mesh::new(
            PrimitiveTopology::LineList,
            bevy::asset::RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 4]);
        assert!(simplify_mesh(&lines, 4).is_none());
        assert!(simplify_mesh(&grid_mesh(10), 1).is_none());
        assert!(
            simplify_mesh(
                &Mesh::new(
                    PrimitiveTopology::TriangleList,
                    bevy::asset::RenderAssetUsages::default(),
                ),
                4
            )
            .is_none()
        );
    }

    #[test]
    fn simplify_mesh_refuses_skinned_and_morphed_meshes() {
        let count = grid_mesh(10)
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .len();
        let mut skinned = grid_mesh(10);
        skinned.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(vec![[0, 0, 0, 0]; count]),
        );
        skinned.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_WEIGHT,
            VertexAttributeValues::Float32x4(vec![[1.0, 0.0, 0.0, 0.0]; count]),
        );
        assert!(simplify_mesh(&skinned, 4).is_none());
        let morphed = grid_mesh(10).with_morph_targets(vec![Default::default(); count]);
        assert!(simplify_mesh(&morphed, 4).is_none());
    }

    #[test]
    fn mid_distance_models_draw_decimated_and_come_back_up_close() {
        let mut app = App::new();
        app.init_resource::<crate::culling::LodPolicy>()
            .init_resource::<crate::culling::Culling>()
            .init_resource::<crate::quality::Scaling>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<ModelLodCache>()
            .add_systems(Update, (crate::culling::select_lod, sync_model_lod).chain());
        perspective_camera(&mut app);
        let original = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(grid_mesh(8));
        let part = app
            .world_mut()
            .spawn((Mesh3d(original.clone()), Visibility::Inherited))
            .id();
        let mut scene = app.world_mut().spawn(Visibility::Inherited);
        scene.add_child(part);
        let child = scene.id();
        // A 2 m box bounds a 1.73 m sphere: full at 5 m, decimated at
        // 100 m, gone at 200 m.
        let group = crate::culling::LodGroup::new(1.732)
            .level(SIMPLIFY_SCREEN, None)
            .level(0.01, None);
        let actor = app
            .world_mut()
            .spawn((
                GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0)),
                group,
                ModelChild(child),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .current(),
            Some(0)
        );
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -100.0));
        app.update();
        assert_eq!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .current(),
            Some(1)
        );
        let thin = app.world().get::<Mesh3d>(part).unwrap().0.clone();
        assert_ne!(thin, original);
        let (full_tris, thin_tris) = {
            let meshes = app.world().resource::<Assets<Mesh>>();
            let count =
                |handle: &Handle<Mesh>| meshes.get(handle).unwrap().indices().unwrap().len() / 3;
            (count(&original), count(&thin))
        };
        assert_eq!((full_tris, thin_tris), (128, 32));
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -5.0));
        app.update();
        assert_eq!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .current(),
            Some(0)
        );
        assert_eq!(app.world().get::<Mesh3d>(part).unwrap().0, original);
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -200.0));
        app.update();
        assert!(
            app.world()
                .get::<crate::culling::LodGroup>(actor)
                .unwrap()
                .is_culled()
        );
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Hidden
        );
    }
}
