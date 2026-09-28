//! Grass and scattered props on a terrain.
//!
//! Grass streams in fixed 32 m cells of the terrain, only near the camera
//! and only under active streaming cells, built on `CellTasks`. Scatter is
//! placed once per build: each instance is an entity with a `LodGroup`
//! whose levels show its LOD0, LOD1 or billboard parts, so a model made of
//! several meshes swaps as one.

use super::material::{self, GrassMaterial};
use super::{Built, TerrainJobs, Terrained};
use crate::batching::{InstanceRecord, attach_instanced};
use crate::culling::{LodChanged, LodGroup};
use crate::engine::Engine;
use crate::streaming::StreamingCells;
use crate::world::{WorldCamera, parse_color};
use bevy::asset::RenderAssetUsages;
use bevy::gltf::{Gltf, GltfMesh, GltfNode};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_core::material::hex_to_linear;
use blockloom_core::terrain::scatter::{grass_blades, grass_mesh};
use blockloom_core::terrain::store::Grid;
use blockloom_core::terrain::{ScatterLayer, ScatterShape, TerrainSpec};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

/// Metres a side of one grass cell.
pub const GRASS_CELL: f32 = 32.0;
/// Most blades one cell builds.
const BLADES_PER_CELL: usize = 60_000;
/// The field of view scatter distances are measured against.
const REFERENCE_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// Actor, grass layer, cell X and Z.
pub type GrassKey = (String, u32, i32, i32);

pub enum GrassSlot {
    Loading,
    Empty,
    Drawn(Entity),
}

#[derive(Component)]
pub struct GrassCell {
    pub blades: usize,
}

pub fn grass_materials(world: &mut World, spec: &TerrainSpec) -> Vec<Handle<GrassMaterial>> {
    let globals = world
        .resource::<crate::materials::SurfaceGlobals>()
        .buffer
        .clone();
    let mut materials = world.resource_mut::<Assets<GrassMaterial>>();
    spec.grass
        .iter()
        .map(|grass| material::grass_material(&mut materials, globals.clone(), grass))
        .collect()
}

/// Wants the grass cells within each layer's cull distance whose streaming
/// cell is active, and drops the rest.
pub fn stream_grass(
    scaling: Option<Res<crate::quality::Scaling>>,
    mut last_density: Local<Option<f32>>,
    mut commands: Commands,
    mut jobs: ResMut<TerrainJobs>,
    mut cells: ResMut<StreamingCells>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    mut roots: Query<(&crate::engine::ActorId, &Transform, &mut Terrained)>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let density = scaling
        .as_ref()
        .map_or(1.0, |s| s.budget().density * s.geometry.factors[1].powi(2));
    let density_changed = last_density.is_some_and(|old| old != density);
    *last_density = Some(density);
    let eye = camera.translation();
    for (id, transform, mut terrained) in &mut roots {
        let Some(built) = terrained.built.clone() else {
            continue;
        };
        if terrained.spec.grass.is_empty() {
            continue;
        }
        let affine = transform.compute_affine();
        let local_eye = affine.inverse().transform_point3(eye);
        let shape = built.geometry.shape;
        let half = [shape.size[0] * 0.5, shape.size[1] * 0.5];
        let mut wanted = HashSet::new();
        for (layer, grass) in terrained.spec.grass.iter().enumerate() {
            let distance = grass.cull_distance
                * scaling
                    .as_ref()
                    .map_or(1.0, |s| s.budget().distance * s.geometry.factors[1]);
            let reach = distance + GRASS_CELL * 0.75;
            let cell_of = |v: f32, h: f32| ((v + h) / GRASS_CELL).floor() as i32;
            let (x0, x1) = (
                cell_of(local_eye.x - reach, half[0]),
                cell_of(local_eye.x + reach, half[0]),
            );
            let (z0, z1) = (
                cell_of(local_eye.z - reach, half[1]),
                cell_of(local_eye.z + reach, half[1]),
            );
            let max_x = (shape.size[0] / GRASS_CELL).ceil() as i32;
            let max_z = (shape.size[1] / GRASS_CELL).ceil() as i32;
            for gz in z0.max(0)..=z1.min(max_z - 1) {
                for gx in x0.max(0)..=x1.min(max_x - 1) {
                    let min = [
                        -half[0] + gx as f32 * GRASS_CELL,
                        -half[1] + gz as f32 * GRASS_CELL,
                    ];
                    let centre = Vec3::new(
                        min[0] + GRASS_CELL * 0.5,
                        local_eye.y,
                        min[1] + GRASS_CELL * 0.5,
                    );
                    let near = Vec2::new(
                        local_eye.x.clamp(min[0], min[0] + GRASS_CELL),
                        local_eye.z.clamp(min[1], min[1] + GRASS_CELL),
                    );
                    if near.distance(Vec2::new(local_eye.x, local_eye.z)) > distance {
                        continue;
                    }
                    let world_cell = StreamingCells::cell_at(affine.transform_point3(centre));
                    if !cells.active.contains(&world_cell) {
                        continue;
                    }
                    wanted.insert((id.0.clone(), layer as u32, gx, gz));
                }
            }
        }
        let stale: Vec<GrassKey> = terrained
            .grass_cells
            .keys()
            .filter(|key| density_changed || !wanted.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            match terrained.grass_cells.remove(&key) {
                Some(GrassSlot::Drawn(entity)) => commands.entity(entity).try_despawn(),
                Some(GrassSlot::Loading) => jobs.grass.cancel(&mut cells, &key),
                _ => {}
            }
        }
        for key in wanted {
            if terrained.grass_cells.contains_key(&key) {
                continue;
            }
            let (_, layer, gx, gz) = key.clone();
            let mut grass = terrained.spec.grass[layer as usize].clone();
            grass.density *= density;
            let built = built.clone();
            let min = [
                -half[0] + gx as f32 * GRASS_CELL,
                -half[1] + gz as f32 * GRASS_CELL,
            ];
            let max = [min[0] + GRASS_CELL, min[1] + GRASS_CELL];
            let centre = Vec3::new(min[0] + GRASS_CELL * 0.5, 0.0, min[1] + GRASS_CELL * 0.5);
            let world_cell = StreamingCells::cell_at(affine.transform_point3(centre));
            terrained
                .grass_cells
                .insert(key.clone(), GrassSlot::Loading);
            jobs.grass.spawn(&mut cells, key, world_cell, move || {
                grass_cell(&built, &grass, layer as usize, min, max)
            });
        }
    }
}

/// One cell's blades as a mesh, centred on the cell.
fn grass_cell(
    built: &Built,
    grass: &blockloom_core::terrain::GrassLayer,
    layer: usize,
    min: [f32; 2],
    max: [f32; 2],
) -> Option<(Mesh, usize, [f32; 3])> {
    let ground = built.geometry.ground(Some(&built.weights));
    let map = built
        .grass_maps
        .get(layer)
        .and_then(Option::as_ref)
        .and_then(Grid::mask);
    let blades = grass_blades(&ground, grass, map, min, max, BLADES_PER_CELL);
    if blades.is_empty() {
        return None;
    }
    let height = blades.iter().map(|b| b.root[1]).sum::<f32>() / blades.len() as f32;
    let origin = [(min[0] + max[0]) * 0.5, height, (min[1] + max[1]) * 0.5];
    let data = grass_mesh(&blades, grass, origin);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, data.positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, data.uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, data.colors)
    .with_inserted_indices(Indices::U32(data.indices));
    Some((mesh, blades.len(), origin))
}

pub fn land_grass(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut jobs: ResMut<TerrainJobs>,
    mut cells: ResMut<StreamingCells>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut roots: Query<&mut Terrained>,
) {
    for (key, result) in jobs.grass.poll(&mut cells) {
        let Some(&root) = engine.entities.get(&key.0) else {
            continue;
        };
        let Ok(mut terrained) = roots.get_mut(root) else {
            continue;
        };
        if !matches!(terrained.grass_cells.get(&key), Some(GrassSlot::Loading)) {
            continue;
        }
        let slot = match (result, terrained.grass_materials.get(key.1 as usize)) {
            (Some((mesh, blades, origin)), Some(material)) => GrassSlot::Drawn(
                commands
                    .spawn((
                        GrassCell { blades },
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(Vec3::from(origin)),
                        bevy::light::NotShadowCaster,
                        ChildOf(root),
                    ))
                    .id(),
            ),
            _ => GrassSlot::Empty,
        };
        terrained.grass_cells.insert(key, slot);
    }
}

// ─── Scatter ───────────────────────────────────────────────────────────────

/// What each level of a scatter instance shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Near,
    Far,
    Billboard,
}

/// A scattered tree or rock: its parts by level.
#[derive(Component)]
pub struct ScatterInstance {
    pub levels: Vec<Part>,
    pub parts: Vec<(Part, Entity)>,
}

/// A part holder waiting on a model file.
#[derive(Component)]
pub struct PendingModel {
    gltf: Handle<Gltf>,
    key: String,
    tint: f32,
}

/// Places every scatter layer's instances under the root. Answers what it
/// spawned and how many instances.
pub fn spawn_scatter(
    world: &mut World,
    root: Entity,
    spec: &TerrainSpec,
    built: &Arc<Built>,
    dir: Option<&Path>,
) -> (Vec<Entity>, usize) {
    let mut spawned = Vec::new();
    let mut count = 0;
    for (index, layer) in spec.scatter.iter().enumerate() {
        let Some(instances) = built.scatter.get(index) else {
            continue;
        };
        if instances.is_empty() {
            continue;
        }
        let height = shape_height(layer.shape);
        let radius = height.max(1.0) * 0.6;
        let near = level_parts(world, layer, dir, Part::Near);
        let far = level_parts(world, layer, dir, Part::Far);
        let billboard = billboard_part(world, layer, dir, height);
        let screen = |distance: f32| radius / (distance.max(0.01) * (REFERENCE_FOV * 0.5).tan());
        let mut levels = vec![(screen(layer.lod1_distance), Part::Near)];
        match &billboard {
            Some(_) => {
                levels.push((screen(layer.billboard_distance), Part::Far));
                levels.push((screen(layer.cull_distance), Part::Billboard));
            }
            None => levels.push((screen(layer.cull_distance), Part::Far)),
        }
        levels.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut group = LodGroup::new(radius);
        for (min_screen, _) in &levels {
            group = group.level(*min_screen, None);
        }
        let order: Vec<Part> = levels.iter().map(|(_, part)| *part).collect();
        for instance in instances {
            let transform = Transform::from_translation(Vec3::from(instance.position))
                .with_rotation(Quat::from_array(instance.rotation))
                .with_scale(Vec3::splat(instance.scale));
            let holder = world
                .spawn((
                    transform,
                    Visibility::default(),
                    group.clone(),
                    ChildOf(root),
                ))
                .id();
            let record = InstanceRecord {
                tint: Vec4::new(instance.tint, instance.tint, instance.tint, 1.0),
                ..InstanceRecord::IDENTITY
            };
            let mut parts = Vec::new();
            for (part, source) in [(Part::Near, &near), (Part::Far, &far)] {
                let visibility = if order.first() == Some(&part) {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                let entity = world
                    .spawn((Transform::default(), visibility, ChildOf(holder)))
                    .id();
                match source {
                    Source::Mesh { mesh, key, base } => {
                        world.entity_mut(entity).insert(Mesh3d(mesh.clone()));
                        attach_instanced(
                            world,
                            entity,
                            key.clone(),
                            Some((**base).clone()),
                            record,
                        );
                    }
                    Source::Model { gltf, key } => {
                        world.entity_mut(entity).insert(PendingModel {
                            gltf: gltf.clone(),
                            key: key.clone(),
                            tint: instance.tint,
                        });
                    }
                }
                parts.push((part, entity));
            }
            if let Some((mesh, key, base)) = &billboard {
                let entity = world
                    .spawn((
                        Mesh3d(mesh.clone()),
                        Transform::default(),
                        Visibility::Hidden,
                        ChildOf(holder),
                    ))
                    .id();
                attach_instanced(world, entity, key.clone(), Some(base.clone()), record);
                parts.push((Part::Billboard, entity));
            }
            if layer.collide {
                let r = layer.radius.max(0.05);
                let collider = if layer.shape == ScatterShape::Rock && layer.model.is_empty() {
                    rp::Collider::ball(r)
                } else {
                    rp::Collider::cylinder(height * 0.5, r)
                };
                let lift = if layer.shape == ScatterShape::Rock {
                    r * 0.5
                } else {
                    height * 0.5
                };
                world.spawn((
                    rp::RigidBody::Fixed,
                    collider,
                    Transform::from_xyz(0.0, lift, 0.0),
                    ChildOf(holder),
                ));
            }
            world.entity_mut(holder).insert(ScatterInstance {
                levels: order.clone(),
                parts,
            });
            spawned.push(holder);
            count += 1;
        }
    }
    (spawned, count)
}

enum Source {
    Mesh {
        mesh: Handle<Mesh>,
        key: String,
        base: Box<StandardMaterial>,
    },
    Model {
        gltf: Handle<Gltf>,
        key: String,
    },
}

/// A level's parts: the layer's model (or its lighter one far away), or a
/// procedural shape.
fn level_parts(world: &mut World, layer: &ScatterLayer, dir: Option<&Path>, part: Part) -> Source {
    let file = match part {
        Part::Far if !layer.model_lod1.is_empty() => &layer.model_lod1,
        _ => &layer.model,
    };
    if !file.is_empty() {
        let path = crate::world::asset_path(dir, file);
        let gltf = world.resource::<AssetServer>().load::<Gltf>(path.clone());
        return Source::Model {
            gltf,
            key: format!("scatter-model:{}", path.display()),
        };
    }
    let detail = if part == Part::Near { 1.0 } else { 0.45 };
    let mesh =
        world
            .resource_mut::<Assets<Mesh>>()
            .add(shape_mesh(layer.shape, &layer.color, detail));
    Source::Mesh {
        mesh,
        key: "scatter-shape".to_string(),
        base: Box::new(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.85,
            ..default()
        }),
    }
}

/// Crossed quads showing the layer's billboard image.
fn billboard_part(
    world: &mut World,
    layer: &ScatterLayer,
    dir: Option<&Path>,
    height: f32,
) -> Option<(Handle<Mesh>, String, StandardMaterial)> {
    if layer.billboard.is_empty() {
        return None;
    }
    let path = crate::world::asset_path(dir, &layer.billboard);
    let texture = world.resource::<AssetServer>().load::<Image>(path.clone());
    let tint = parse_color(&layer.color);
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(crossed_quads(height * 0.75, height));
    Some((
        mesh,
        format!("scatter-billboard:{}:{}", path.display(), layer.color),
        StandardMaterial {
            base_color: tint,
            base_color_texture: Some(texture),
            alpha_mode: AlphaMode::Mask(0.5),
            double_sided: true,
            cull_mode: None,
            perceptual_roughness: 0.9,
            ..default()
        },
    ))
}

/// Shows the parts of whichever level an instance moved to.
pub fn swap_levels(
    event: On<LodChanged>,
    instances: Query<&ScatterInstance>,
    mut visibility: Query<&mut Visibility>,
) {
    let Ok(instance) = instances.get(event.entity) else {
        return;
    };
    let showing = event
        .level
        .and_then(|level| instance.levels.get(level))
        .copied();
    for (part, entity) in &instance.parts {
        if let Ok(mut v) = visibility.get_mut(*entity) {
            let wanted = if Some(*part) == showing {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *v != wanted {
                *v = wanted;
            }
        }
    }
}

/// Fills part holders with a model's meshes once the file has loaded.
/// Nodes draw at their own transforms, so a nested hierarchy flattens.
pub fn attach_models(
    mut commands: Commands,
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    materials: Res<Assets<bevy::gltf::GltfMaterial>>,
    pending: Query<(Entity, &PendingModel)>,
) {
    for (entity, model) in &pending {
        if assets.load_state(&model.gltf).is_failed() {
            commands.entity(entity).remove::<PendingModel>();
            continue;
        }
        if !assets.is_loaded_with_dependencies(&model.gltf) {
            continue;
        }
        let Some(gltf) = gltfs.get(&model.gltf) else {
            continue;
        };
        let record = InstanceRecord {
            tint: Vec4::new(model.tint, model.tint, model.tint, 1.0),
            ..InstanceRecord::IDENTITY
        };
        for (n, node) in gltf.nodes.iter().filter_map(|h| nodes.get(h)).enumerate() {
            let Some(mesh) = node.mesh.as_ref().and_then(|h| gltf_meshes.get(h)) else {
                continue;
            };
            for (p, primitive) in mesh.primitives.iter().enumerate() {
                let base = primitive
                    .material
                    .as_ref()
                    .and_then(|h| materials.get(h))
                    .map(bevy::pbr::gltf::standard_material_from_gltf_material)
                    .unwrap_or_default();
                let key = format!("{}:{n}:{p}", model.key);
                let child = commands
                    .spawn((
                        Mesh3d(primitive.mesh.clone()),
                        node.transform,
                        ChildOf(entity),
                    ))
                    .id();
                commands.queue(move |world: &mut World| {
                    attach_instanced(world, child, key, Some(base), record);
                });
            }
        }
        commands.entity(entity).remove::<PendingModel>();
    }
}

fn shape_height(shape: ScatterShape) -> f32 {
    match shape {
        ScatterShape::Tree => 5.0,
        ScatterShape::Pine => 5.3,
        ScatterShape::Bush => 1.1,
        ScatterShape::Rock => 0.85,
    }
}

/// Vertex-colored geometry for a procedural shape; `detail` below 1 is the
/// lighter level.
pub fn shape_mesh(shape: ScatterShape, color: &str, detail: f32) -> Mesh {
    let crown = hex_to_linear(color);
    let crown = [crown[0], crown[1], crown[2], 1.0];
    let bark = [0.16, 0.10, 0.06, 1.0];
    let segments = ((12.0 * detail).round() as u32).max(5);
    let rings = ((8.0 * detail).round() as u32).max(3);
    let mut builder = Builder::default();
    match shape {
        ScatterShape::Tree => {
            builder.cone(Vec3::ZERO, 0.2, 0.14, 2.4, segments.min(8), bark);
            builder.blob(
                Vec3::new(0.0, 3.4, 0.0),
                Vec3::new(1.6, 1.5, 1.6),
                segments,
                rings,
                crown,
                0.12,
            );
        }
        ScatterShape::Pine => {
            builder.cone(Vec3::ZERO, 0.16, 0.1, 1.4, segments.min(8), bark);
            for (y, r, h) in [(1.0, 1.5, 2.2), (2.2, 1.15, 1.9), (3.3, 0.8, 2.0)] {
                builder.cone(Vec3::new(0.0, y, 0.0), r, 0.0, h, segments, crown);
            }
        }
        ScatterShape::Bush => {
            builder.blob(
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(0.9, 0.6, 0.9),
                segments,
                rings,
                crown,
                0.15,
            );
        }
        ScatterShape::Rock => {
            builder.blob(
                Vec3::new(0.0, 0.3, 0.0),
                Vec3::new(0.8, 0.55, 0.7),
                segments,
                rings,
                crown,
                0.25,
            );
        }
    }
    builder.finish()
}

#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Builder {
    /// A capped cone (or cylinder) standing on `base`.
    fn cone(
        &mut self,
        base: Vec3,
        bottom: f32,
        top: f32,
        height: f32,
        segments: u32,
        color: [f32; 4],
    ) {
        let first = self.positions.len() as u32;
        for ring in 0..2 {
            let (y, r) = if ring == 0 {
                (0.0, bottom)
            } else {
                (height, top)
            };
            for s in 0..segments {
                let a = s as f32 / segments as f32 * std::f32::consts::TAU;
                self.positions
                    .push((base + Vec3::new(a.cos() * r, y, a.sin() * r)).to_array());
                self.colors.push(color);
            }
        }
        for s in 0..segments {
            let (a, b) = (first + s, first + (s + 1) % segments);
            let (c, d) = (a + segments, b + segments);
            self.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
        // Bottom cap.
        let centre = self.positions.len() as u32;
        self.positions.push(base.to_array());
        self.colors.push(color);
        for s in 0..segments {
            self.indices
                .extend_from_slice(&[centre, first + s, first + (s + 1) % segments]);
        }
    }

    /// A lumpy ellipsoid.
    fn blob(
        &mut self,
        centre: Vec3,
        radii: Vec3,
        segments: u32,
        rings: u32,
        color: [f32; 4],
        lumps: f32,
    ) {
        let first = self.positions.len() as u32;
        for ring in 0..=rings {
            let v = ring as f32 / rings as f32 * std::f32::consts::PI;
            for s in 0..segments {
                let u = s as f32 / segments as f32 * std::f32::consts::TAU;
                let dir = Vec3::new(v.sin() * u.cos(), v.cos(), v.sin() * u.sin());
                let bump = 1.0
                    + lumps
                        * ((u * 3.0).sin() * (v * 2.0 + 1.3).cos()
                            + (u * 5.0 + v * 3.0).sin() * 0.5);
                self.positions
                    .push((centre + dir * radii * bump).to_array());
                self.colors.push(color);
            }
        }
        for ring in 0..rings {
            for s in 0..segments {
                let a = first + ring * segments + s;
                let b = first + ring * segments + (s + 1) % segments;
                let c = a + segments;
                let d = b + segments;
                self.indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
    }

    fn finish(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(self.indices));
        mesh.compute_smooth_normals();
        mesh
    }
}

/// Two quads crossing on the vertical axis, standing on the origin.
fn crossed_quads(width: f32, height: f32) -> Mesh {
    let w = width * 0.5;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (axis, normal) in [(Vec3::X, Vec3::Z), (Vec3::Z, Vec3::X)] {
        let first = positions.len() as u32;
        for (u, v) in [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)] {
            let p = axis * (u * 2.0 - 1.0) * w + Vec3::Y * (1.0 - v) * height;
            positions.push(p.to_array());
            normals.push(normal.to_array());
            uvs.push([u, v]);
        }
        indices.extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_shapes_have_normals_and_a_lighter_level() {
        for shape in [
            ScatterShape::Tree,
            ScatterShape::Pine,
            ScatterShape::Bush,
            ScatterShape::Rock,
        ] {
            let near = shape_mesh(shape, "#4A7A30", 1.0);
            let far = shape_mesh(shape, "#4A7A30", 0.45);
            assert!(near.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
            assert!(far.count_vertices() < near.count_vertices(), "{shape:?}");
        }
    }
}
