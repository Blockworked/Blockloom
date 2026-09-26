//! Heightmap terrain, 3D only: chunked meshes on the LOD selector, the
//! ground collider, grass and scattered props. The data and all the maths
//! live in `blockloom_core::terrain`; this is the world half.
//!
//! A terrain actor's entity is the root: chunks, the collider, grass cells
//! and scatter instances hang off it, so the actor's `Place` moves the lot.
//! Building happens on a `CellTasks` task, which holds warm-up until it
//! lands. Chunk levels coarser than `ChunkLayout::resident_level` are built
//! with the terrain; finer ones stream in for chunks under an active cell.

pub mod material;
pub mod vegetation;

use crate::culling::LodGroup;
use crate::engine::Engine;
use crate::streaming::{Cell, CellEntered, CellLeft, CellTasks, StreamingCells};
use crate::engine::ActorId;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy_rapier3d::prelude as rp;
use blockloom_core::terrain::mesh::{
    ChunkBounds, ChunkLayout, TerrainMesh, chunk_bounds, chunk_errors, chunk_mesh,
    collider_heights, lod_thresholds, skirt_depth,
};
use blockloom_core::terrain::scatter::{Avoid, Ground, Instance, scatter_instances};
use blockloom_core::terrain::store::{self, Grid};
use blockloom_core::terrain::{Heightfield, Shape, TerrainSpec, bake_weights};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Most instances one scatter layer places.
const SCATTER_LIMIT: usize = 20_000;

pub fn register(app: &mut App) {
    material::register(app);
    app.init_resource::<TerrainCache>()
        .init_resource::<TerrainJobs>()
        .init_resource::<TerrainIndex>()
        .init_resource::<TerrainStats>()
        .add_observer(vegetation::swap_levels)
        .add_systems(
            Update,
            (
                sync_terrains,
                land_terrains,
                stream_chunks,
                vegetation::stream_grass,
                vegetation::land_grass,
                vegetation::attach_models,
                material::build_layer_arrays,
                material::follow_rotation,
                count_stats,
            )
                .chain()
                .after(crate::world::rebuild_world)
                .after(crate::streaming::update_streaming_cells),
        );
}

/// Everything about a terrain's shape: the grid, what is cut out of it, and
/// the chunk meshes that don't stream.
pub struct Geometry {
    pub field: Heightfield,
    pub shape: Shape,
    pub holes: Option<Grid>,
    pub layout: ChunkLayout,
    pub chunks: Vec<Chunk>,
    /// Per sample: normal xyz as 0-255 and cavity, for the surface map.
    pub surface: Vec<[u8; 4]>,
    pub skirt: f32,
}

pub struct Chunk {
    pub cx: u32,
    pub cz: u32,
    pub bounds: ChunkBounds,
    pub origin: [f32; 3],
    /// Screen sizes per level, for the LOD selector.
    pub thresholds: Vec<f32>,
    /// Meshes from `resident_level` down, coarsest last.
    pub resident: Vec<TerrainMesh>,
}

impl Geometry {
    fn build(dir: Option<&Path>, spec: &TerrainSpec) -> Geometry {
        let field = store::heights_for(dir, spec);
        let holes = store::grid_for(dir, &spec.holes, spec.resolution);
        Self::from_parts(field, holes, spec)
    }

    fn from_parts(field: Heightfield, holes: Option<Grid>, spec: &TerrainSpec) -> Geometry {
        let shape = spec.shape();
        let layout = ChunkLayout::for_side(shape.side);
        let cavity = field.cavity_map(&shape);
        let surface = (0..shape.side)
            .flat_map(|j| (0..shape.side).map(move |i| (i, j)))
            .map(|(i, j)| {
                let n = field.normal(&shape, i, j);
                let byte = |v: f32| ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                [
                    byte(n[0]),
                    byte(n[1]),
                    byte(n[2]),
                    cavity[field.index(i, j)],
                ]
            })
            .collect();
        let mut geometry = Geometry {
            field,
            shape,
            holes,
            layout,
            chunks: Vec::new(),
            surface,
            skirt: 0.0,
        };
        let mut worst = Vec::new();
        for cz in 0..layout.chunks {
            for cx in 0..layout.chunks {
                worst.push(chunk_errors(&geometry.field, &shape, &layout, cx, cz));
            }
        }
        geometry.skirt = worst
            .iter()
            .map(|errors| skirt_depth(errors, &shape))
            .fold(0.0, f32::max);
        for cz in 0..layout.chunks {
            for cx in 0..layout.chunks {
                let errors = &worst[(cz * layout.chunks + cx) as usize];
                let bounds = chunk_bounds(&geometry.field, &shape, &layout, cx, cz);
                let thresholds = lod_thresholds(errors, bounds.radius(), spec.pixel_error);
                let resident = (layout.resident_level()..layout.levels)
                    .map(|level| geometry.mesh(cx, cz, level).0)
                    .collect();
                geometry.chunks.push(Chunk {
                    cx,
                    cz,
                    bounds,
                    origin: bounds.center(),
                    thresholds,
                    resident,
                });
            }
        }
        geometry
    }

    pub fn mesh(&self, cx: u32, cz: u32, level: u32) -> (TerrainMesh, [f32; 3]) {
        chunk_mesh(
            &self.field,
            &self.shape,
            &self.layout,
            cx,
            cz,
            level,
            self.holes.as_ref().and_then(Grid::mask),
            self.skirt,
        )
    }

    pub fn ground<'a>(&'a self, weights: Option<&'a [[u8; 4]]>) -> Ground<'a> {
        Ground {
            field: &self.field,
            shape: &self.shape,
            weights,
            holes: self.holes.as_ref().and_then(Grid::mask),
        }
    }

    /// Where a world-space ray first meets the ground, in the root's frame,
    /// marching the heightfield and refining the crossing.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max: f32) -> Option<Vec3> {
        let spacing = self.shape.spacing();
        let step = (spacing[0].min(spacing[1]) * 0.5).max(0.05);
        let above = |p: Vec3| {
            self.field
                .height_at(&self.shape, p.x, p.z)
                .map(|h| p.y - h)
        };
        let mut t = 0.0;
        let mut last: Option<(f32, f32)> = None;
        while t <= max {
            let p = origin + direction * t;
            match above(p) {
                Some(gap) if gap <= 0.0 => {
                    let Some((t0, g0)) = last else {
                        return Some(p);
                    };
                    let f = g0 / (g0 - gap).max(1e-6);
                    return Some(origin + direction * (t0 + (t - t0) * f));
                }
                Some(gap) => last = Some((t, gap)),
                None => last = None,
            }
            // Big strides while far above the ground.
            let gap = last.map_or(step, |(_, g)| g);
            t += (gap * 0.5).clamp(step, step * 16.0);
        }
        None
    }
}

/// What a whole terrain draws from once built.
pub struct Built {
    pub geometry: Arc<Geometry>,
    /// Painted weights as stored, for the brushes to paint over.
    pub splat: Option<Grid>,
    pub weights: Arc<Vec<[u8; 4]>>,
    pub grass_maps: Vec<Option<Grid>>,
    pub scatter: Vec<Vec<Instance>>,
}

/// Finished builds by what they were built from, so a rebuild that changed
/// nothing about a terrain doesn't build it again.
#[derive(Resource, Default)]
pub struct TerrainCache {
    built: HashMap<u64, Arc<Built>>,
    geometry: HashMap<u64, Arc<Geometry>>,
}

impl TerrainCache {
    /// Keeps only what live terrains use.
    fn retain(&mut self, live: &HashSet<u64>, live_geometry: &HashSet<u64>) {
        self.built.retain(|key, _| live.contains(key));
        self.geometry.retain(|key, _| live_geometry.contains(key));
    }

    /// Files a build the scene view already drew, under the document the
    /// editor will send back, so that reload finds it here.
    pub fn predict(&mut self, dir: Option<&Path>, spec: &TerrainSpec, built: Built) {
        self.geometry
            .insert(geometry_key(dir, spec), built.geometry.clone());
        self.built.insert(full_key(dir, spec), Arc::new(built));
    }
}

fn geometry_key(dir: Option<&Path>, spec: &TerrainSpec) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut hasher);
    (
        &spec.heights,
        &spec.holes,
        spec.resolution,
        spec.pixel_error.to_bits(),
    )
        .hash(&mut hasher);
    spec.size.map(f32::to_bits).hash(&mut hasher);
    spec.height.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn full_key(dir: Option<&Path>, spec: &TerrainSpec) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut hasher);
    serde_json::to_string(spec)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

/// Terrain builds in flight, by actor.
#[derive(Resource, Default)]
pub struct TerrainJobs {
    builds: CellTasks<String, (u64, Built)>,
    fine: CellTasks<(String, u32), Vec<(u32, TerrainMesh)>>,
    grass: CellTasks<vegetation::GrassKey, Option<(Mesh, usize, [f32; 3])>>,
}

/// Lets the scene view find a terrain by actor: its data and root.
#[derive(Resource, Default)]
pub struct TerrainIndex {
    pub terrains: HashMap<String, (Entity, Arc<Built>)>,
}

/// On a terrain actor's entity: what it was built from, and what hangs off
/// it once built.
#[derive(Component)]
pub struct Terrained {
    pub spec: TerrainSpec,
    pub key: u64,
    pub built: Option<Arc<Built>>,
    pub chunks: Vec<Entity>,
    /// Chunks drawing their streamed levels, and how many active cells
    /// want them.
    pub fine: HashMap<u32, u32>,
    pub parts: Vec<Entity>,
    pub material: Handle<material::TerrainMaterial>,
    pub grass_cells: HashMap<vegetation::GrassKey, vegetation::GrassSlot>,
    pub grass_materials: Vec<Handle<material::GrassMaterial>>,
    pub triangles: usize,
    pub instances: usize,
}

/// One chunk of a terrain's ground.
#[derive(Component)]
pub struct TerrainChunk {
    pub index: u32,
}

/// The ground collider under a terrain root.
#[derive(Component)]
pub struct TerrainCollider;

/// The terrain an actor has right now, if it carries one.
pub fn wanted(engine: &Engine, actor: &str) -> Option<TerrainSpec> {
    if !engine.project.world.mode.is_3d() || !engine.has_component(actor, "Terrain") {
        return None;
    }
    engine.actor(actor)?.components.terrain().cloned()
}

/// Starts a build for every terrain actor that needs one, and takes down
/// terrains whose actor lost or changed its component.
#[allow(clippy::too_many_arguments)]
pub fn sync_terrains(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut cache: ResMut<TerrainCache>,
    mut jobs: ResMut<TerrainJobs>,
    mut cells: ResMut<StreamingCells>,
    mut index: ResMut<TerrainIndex>,
    actors: Query<(Entity, &ActorId, &Transform, Option<&Terrained>)>,
    placed: Query<(&ActorId, &Transform)>,
) {
    let dir = engine.project_dir.clone();
    for (entity, id, transform, current) in &actors {
        let spec = wanted(&engine, &id.0);
        let key = spec.as_ref().map(|spec| full_key(dir.as_deref(), spec));
        if current.map(|current| current.key) == key {
            continue;
        }
        if let Some(current) = current {
            for &part in current.chunks.iter().chain(&current.parts) {
                commands.entity(part).try_despawn();
            }
            for slot in current.grass_cells.values() {
                if let vegetation::GrassSlot::Drawn(part) = *slot {
                    commands.entity(part).try_despawn();
                }
            }
            commands.entity(entity).remove::<Terrained>();
            index.terrains.remove(&id.0);
        }
        jobs.builds.cancel(&mut cells, &id.0);
        let (Some(spec), Some(key)) = (spec, key) else {
            continue;
        };
        commands.entity(entity).insert(Terrained {
            spec: spec.clone(),
            key,
            built: None,
            chunks: Vec::new(),
            fine: HashMap::new(),
            parts: Vec::new(),
            material: Handle::default(),
            grass_cells: HashMap::new(),
            grass_materials: Vec::new(),
            triangles: 0,
            instances: 0,
        });
        if let Some(built) = cache.built.get(&key) {
            commands.queue(spawn_built(id.0.clone(), entity, built.clone()));
            continue;
        }
        let geometry = cache.geometry.get(&geometry_key(dir.as_deref(), &spec)).cloned();
        let avoid = avoid_list(&engine, &id.0, transform, &placed);
        let cell = StreamingCells::cell_at(transform.translation);
        let dir = dir.clone();
        jobs.builds.spawn(&mut cells, id.0.clone(), cell, move || {
            (key, build(dir.as_deref(), &spec, geometry, &avoid))
        });
    }
}

/// Footprints of the other actors with something to draw, in the
/// terrain's frame.
fn avoid_list(
    engine: &Engine,
    terrain: &str,
    root: &Transform,
    placed: &Query<(&ActorId, &Transform)>,
) -> Vec<Avoid> {
    let inverse = root.compute_affine().inverse();
    placed
        .iter()
        .filter(|(id, _)| id.0 != terrain)
        .filter_map(|(id, transform)| {
            let actor = engine.actor(&id.0)?;
            let visual = actor.visual()?;
            let half = crate::world::half_extents3(visual) * transform.scale.abs();
            let local = inverse.transform_point3(transform.translation);
            Some(Avoid {
                center: [local.x, local.z],
                radius: half.x.max(half.z),
            })
        })
        .collect()
}

/// A whole terrain, off the main thread.
fn build(
    dir: Option<&Path>,
    spec: &TerrainSpec,
    geometry: Option<Arc<Geometry>>,
    avoid: &[Avoid],
) -> Built {
    let geometry = geometry.unwrap_or_else(|| Arc::new(Geometry::build(dir, spec)));
    paint(dir, spec, geometry, avoid)
}

/// Everything above the geometry: weights, grass maps, scatter.
pub fn paint(
    dir: Option<&Path>,
    spec: &TerrainSpec,
    geometry: Arc<Geometry>,
    avoid: &[Avoid],
) -> Built {
    let side = spec.resolution;
    let splat = store::grid_for(dir, &spec.splat, side);
    let painted = splat.as_ref().and_then(Grid::to_weights);
    let weights = bake_weights(
        &geometry.field,
        &geometry.shape,
        &spec.layers,
        painted.as_deref(),
    );
    let grass_maps = spec
        .grass
        .iter()
        .map(|grass| store::grid_for(dir, &grass.density_map, side))
        .collect();
    let scatter = spec
        .scatter
        .iter()
        .map(|layer| {
            let map = store::grid_for(dir, &layer.density_map, side);
            let avoid = if layer.avoid_actors { avoid } else { &[] };
            scatter_instances(
                &geometry.ground(Some(&weights)),
                layer,
                map.as_ref().and_then(Grid::mask),
                avoid,
                SCATTER_LIMIT,
            )
        })
        .collect();
    Built {
        geometry,
        splat,
        weights: Arc::new(weights),
        grass_maps,
        scatter,
    }
}

/// Takes finished builds and puts them in the world.
pub fn land_terrains(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut cache: ResMut<TerrainCache>,
    mut jobs: ResMut<TerrainJobs>,
    mut cells: ResMut<StreamingCells>,
    roots: Query<&Terrained>,
) {
    let landed = jobs.builds.poll(&mut cells);
    if landed.is_empty() {
        return;
    }
    let dir = engine.project_dir.clone();
    for (id, (key, built)) in landed {
        let Some(&entity) = engine.entities.get(&id) else {
            continue;
        };
        let Ok(root) = roots.get(entity) else {
            continue;
        };
        if root.key != key {
            continue;
        }
        let built = Arc::new(built);
        cache
            .geometry
            .insert(geometry_key(dir.as_deref(), &root.spec), built.geometry.clone());
        cache.built.insert(key, built.clone());
        commands.queue(spawn_built(id, entity, built));
    }
    let live: HashSet<u64> = roots.iter().map(|root| root.key).collect();
    let live_geometry: HashSet<u64> = roots
        .iter()
        .map(|root| geometry_key(dir.as_deref(), &root.spec))
        .collect();
    cache.retain(&live, &live_geometry);
}

/// Spawns a built terrain under its root: chunks, collider, scatter.
fn spawn_built(id: String, root: Entity, built: Arc<Built>) -> impl FnOnce(&mut World) {
    move |world: &mut World| {
        let Some(terrained) = world.get::<Terrained>(root) else {
            return;
        };
        let spec = terrained.spec.clone();
        let rotation = world
            .get::<Transform>(root)
            .map_or(Quat::IDENTITY, |t| t.rotation);
        let dir = world.non_send::<Engine>().project_dir.clone();
        let (material, pending) =
            material::terrain_material(world, &spec, &built, dir.as_deref(), rotation);
        if let Some(pending) = pending {
            world.entity_mut(root).insert(pending);
        }
        let geometry = &built.geometry;
        let mut chunks = Vec::with_capacity(geometry.chunks.len());
        let mut triangles = 0;
        for (index, chunk) in geometry.chunks.iter().enumerate() {
            let (group, first, count) = resident_group(world, geometry, chunk);
            triangles += count;
            let entity = world
                .spawn((
                    TerrainChunk {
                        index: index as u32,
                    },
                    Mesh3d(first),
                    MeshMaterial3d(material.clone()),
                    group,
                    Transform::from_translation(Vec3::from(chunk.origin)),
                    ChildOf(root),
                ))
                .id();
            chunks.push(entity);
        }
        let mut parts = Vec::new();
        if spec.collision {
            parts.push(spawn_collider(world, root, geometry));
        }
        let (scatter_parts, instances) =
            vegetation::spawn_scatter(world, root, &spec, &built, dir.as_deref());
        parts.extend(scatter_parts);
        let grass_materials = vegetation::grass_materials(world, &spec);
        if let Some(mut terrained) = world.get_mut::<Terrained>(root) {
            terrained.built = Some(built.clone());
            terrained.chunks = chunks;
            terrained.parts = parts;
            terrained.material = material;
            terrained.grass_materials = grass_materials;
            terrained.triangles = triangles;
            terrained.instances = instances;
        }
        world
            .resource_mut::<TerrainIndex>()
            .terrains
            .insert(id, (root, built));
        // Chunks under cells that are already active want their fine levels.
        let active: Vec<Cell> = world.resource::<StreamingCells>().active.iter().copied().collect();
        world.write_message_batch(active.into_iter().map(CellEntered));
    }
}

/// A chunk's LOD group over its resident meshes, the mesh to start on and
/// its triangle count.
fn resident_group(world: &mut World, geometry: &Geometry, chunk: &Chunk) -> (LodGroup, Handle<Mesh>, usize) {
    let first_level = geometry.layout.resident_level();
    let mut meshes = world.resource_mut::<Assets<Mesh>>();
    let mut group = LodGroup::new(chunk.bounds.radius());
    let mut first = None;
    let mut triangles = 0;
    for (k, mesh) in chunk.resident.iter().enumerate() {
        let level = first_level as usize + k;
        let handle = meshes.add(to_bevy(mesh));
        if first.is_none() {
            first = Some(handle.clone());
            triangles = mesh.triangles();
        }
        group = group.level(chunk.thresholds[level], Some(handle));
    }
    (group, first.unwrap_or_default(), triangles)
}

pub fn to_bevy(mesh: &TerrainMesh) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions.clone())
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, mesh.normals.clone())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, mesh.uvs.clone())
    .with_inserted_indices(Indices::U32(mesh.indices.clone()))
}

/// The ground as one fixed heightfield collider, holes left open.
fn spawn_collider(world: &mut World, root: Entity, geometry: &Geometry) -> Entity {
    use bevy_rapier3d::rapier::parry::shape::{HeightField, HeightFieldCellStatus, SharedShape};
    use bevy_rapier3d::rapier::parry::utils::Array2;
    let shape = geometry.shape;
    let side = shape.side as usize;
    let heights = Array2::new(side, side, collider_heights(&geometry.field, &shape));
    let mut field = HeightField::new(heights, Vec3::new(shape.size[0], 1.0, shape.size[1]));
    if let Some(holes) = geometry.holes.as_ref().and_then(Grid::mask) {
        for j in 0..side - 1 {
            for i in 0..side - 1 {
                if holes[j * side + i] > 127 {
                    field.set_cell_status(j, i, HeightFieldCellStatus::CELL_REMOVED);
                }
            }
        }
    }
    world
        .spawn((
            TerrainCollider,
            rp::RigidBody::Fixed,
            rp::Collider::from(SharedShape::new(field)),
            Transform::default(),
            ChildOf(root),
        ))
        .id()
}

/// Streams finer chunk levels in for chunks under an active cell, and back
/// out once no active cell covers them.
#[allow(clippy::too_many_arguments)]
pub fn stream_chunks(
    mut entered: MessageReader<CellEntered>,
    mut left: MessageReader<CellLeft>,
    mut jobs: ResMut<TerrainJobs>,
    mut cells: ResMut<StreamingCells>,
    mut meshes: ResMut<Assets<Mesh>>,
    engine: NonSend<Engine>,
    mut roots: Query<(&ActorId, &GlobalTransform, &mut Terrained)>,
    mut chunks: Query<(&TerrainChunk, &mut LodGroup, &mut Mesh3d)>,
) {
    let entered: Vec<Cell> = entered.read().map(|m| m.0).collect();
    let left: Vec<Cell> = left.read().map(|m| m.0).collect();
    for (id, transform, mut terrained) in &mut roots {
        let Some(built) = terrained.built.clone() else {
            continue;
        };
        let geometry = &built.geometry;
        if geometry.layout.resident_level() == 0 {
            continue;
        }
        let affine = transform.affine();
        for (cell, delta) in entered
            .iter()
            .map(|c| (*c, 1i32))
            .chain(left.iter().map(|c| (*c, -1)))
        {
            for chunk in &geometry.chunks {
                if !covers(&affine, &chunk.bounds, cell) {
                    continue;
                }
                let index = chunk.cz * geometry.layout.chunks + chunk.cx;
                let count = terrained.fine.entry(index).or_default();
                let was = *count;
                *count = (*count as i32 + delta).max(0) as u32;
                let now = *count;
                if now == 0 {
                    terrained.fine.remove(&index);
                }
                let key = (id.0.clone(), index);
                if was == 0 && now > 0 {
                    let geometry = geometry.clone();
                    let (cx, cz) = (chunk.cx, chunk.cz);
                    let cell = StreamingCells::cell_at(affine.transform_point3(Vec3::from(chunk.origin)));
                    jobs.fine.spawn(&mut cells, key, cell, move || {
                        (0..geometry.layout.resident_level())
                            .map(|level| (level, geometry.mesh(cx, cz, level).0))
                            .collect()
                    });
                } else if was > 0 && now == 0 {
                    jobs.fine.cancel(&mut cells, &key);
                    let group = resident_only(geometry, chunk, &mut meshes, &terrained.chunks, &mut chunks);
                    if let Some(triangles) = group {
                        terrained.triangles = terrained.triangles.saturating_sub(triangles);
                    }
                }
            }
        }
    }
    for ((id, index), fine) in jobs.fine.poll(&mut cells) {
        let Some(&root) = engine.entities.get(&id) else {
            continue;
        };
        let Ok((_, _, mut terrained)) = roots.get_mut(root) else {
            continue;
        };
        if !terrained.fine.contains_key(&index) {
            continue;
        }
        let Some(built) = terrained.built.clone() else {
            continue;
        };
        let Some(&entity) = terrained.chunks.get(index as usize) else {
            continue;
        };
        let Ok((_, mut group, mut mesh)) = chunks.get_mut(entity) else {
            continue;
        };
        let chunk = &built.geometry.chunks[index as usize];
        let mut next = LodGroup::new(chunk.bounds.radius());
        let mut added = 0;
        for (level, part) in &fine {
            added = added.max(part.triangles());
            next = next.level(chunk.thresholds[*level as usize], Some(meshes.add(to_bevy(part))));
        }
        // Levels already streamed stay; only the resident ones carry over.
        let resident = (built.geometry.layout.levels - built.geometry.layout.resident_level()) as usize;
        let kept = group.levels().len().saturating_sub(resident);
        for level in &group.levels()[kept..] {
            next = next.level(level.min_screen, level.mesh.clone());
        }
        if let Some(Some(first)) = next.levels().first().map(|level| level.mesh.clone()) {
            mesh.0 = first;
        }
        *group = next;
        terrained.triangles += added;
    }
}

/// Puts a chunk back on its resident levels. Answers the finest streamed
/// level's triangles, for the stats.
fn resident_only(
    geometry: &Geometry,
    chunk: &Chunk,
    meshes: &mut Assets<Mesh>,
    entities: &[Entity],
    chunks: &mut Query<(&TerrainChunk, &mut LodGroup, &mut Mesh3d)>,
) -> Option<usize> {
    let index = (chunk.cz * geometry.layout.chunks + chunk.cx) as usize;
    let (_, mut group, mut mesh) = chunks.get_mut(*entities.get(index)?).ok()?;
    let resident = geometry.layout.levels - geometry.layout.resident_level();
    let levels = group.levels().to_vec();
    if levels.len() <= resident as usize {
        return None;
    }
    let dropped = levels.len() - resident as usize;
    let finest = levels
        .first()
        .and_then(|level| level.mesh.as_ref())
        .and_then(|handle| meshes.get(handle))
        .map_or(0, |m| m.indices().map_or(0, |i| i.len() / 3));
    for level in &levels[..dropped] {
        if let Some(handle) = &level.mesh {
            meshes.remove(handle);
        }
    }
    let mut next = LodGroup::new(chunk.bounds.radius());
    for level in &levels[dropped..] {
        next = next.level(level.min_screen, level.mesh.clone());
    }
    if let Some(Some(first)) = next.levels().first().map(|level| level.mesh.clone()) {
        mesh.0 = first;
    }
    *group = next;
    Some(finest)
}

/// Whether a chunk's box, placed by `affine`, reaches into `cell`.
fn covers(affine: &bevy::math::Affine3A, bounds: &ChunkBounds, cell: Cell) -> bool {
    let (lo, hi) = world_rect(affine, bounds.min, bounds.max);
    let size = StreamingCells::SIZE;
    let (x0, z0) = (cell.0 as f32 * size, cell.1 as f32 * size);
    lo.x < x0 + size && hi.x > x0 && lo.y < z0 + size && hi.y > z0
}

/// The world XZ rectangle a local box covers.
pub fn world_rect(affine: &bevy::math::Affine3A, min: [f32; 3], max: [f32; 3]) -> (Vec2, Vec2) {
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    for k in 0..8 {
        let corner = Vec3::new(
            if k & 1 == 0 { min[0] } else { max[0] },
            if k & 2 == 0 { min[1] } else { max[1] },
            if k & 4 == 0 { min[2] } else { max[2] },
        );
        let p = affine.transform_point3(corner);
        lo = lo.min(Vec2::new(p.x, p.z));
        hi = hi.max(Vec2::new(p.x, p.z));
    }
    (lo, hi)
}

/// Terrain triangles and vegetation instances, for the profiler.
#[derive(Resource, Default, Clone, Copy)]
pub struct TerrainStats {
    pub terrains: usize,
    pub chunks: usize,
    pub triangles: usize,
    pub grass_cells: usize,
    pub grass_blades: usize,
    pub instances: usize,
}

impl TerrainStats {
    pub fn metrics(&self) -> [(&'static str, usize); 6] {
        [
            ("terrain/terrains", self.terrains),
            ("terrain/chunks", self.chunks),
            ("terrain/triangles", self.triangles),
            ("terrain/grass_cells", self.grass_cells),
            ("terrain/grass_blades", self.grass_blades),
            ("terrain/instances", self.instances),
        ]
    }
}

fn count_stats(
    mut stats: ResMut<TerrainStats>,
    roots: Query<&Terrained>,
    grass: Query<&vegetation::GrassCell>,
) {
    let mut next = TerrainStats::default();
    for root in &roots {
        next.terrains += 1;
        next.chunks += root.chunks.len();
        next.triangles += root.triangles;
        next.instances += root.instances;
    }
    for cell in &grass {
        next.grass_cells += 1;
        next.grass_blades += cell.blades;
    }
    *stats = next;
}

/// Where a project's terrain store lives, for the brushes.
pub fn project_dir(engine: &Engine) -> Option<PathBuf> {
    engine.project_dir.clone()
}
