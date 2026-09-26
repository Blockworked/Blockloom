//! Heightmap terrain, 3D only: chunked meshes on the LOD selector, the
//! ground collider, grass and scattered props. The data and all the maths
//! live in `blockloom_core::terrain`; this is the world half.
//!
//! A terrain actor's entity is the root: chunks, the collider, grass cells
//! and scatter instances hang off it, so the actor's `Place` moves the lot.
//! Building happens on a `CellTasks` task, which holds warm-up until it
//! lands. Chunk levels coarser than `ChunkLayout::resident_level` are built
//! with the terrain; finer ones stream in for chunks under an active cell.

pub mod brush;
pub mod material;
pub mod vegetation;

use crate::culling::LodGroup;
use crate::engine::ActorId;
use crate::engine::Engine;
use crate::streaming::{Cell, CellEntered, CellLeft, CellTasks, StreamingCells};
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
use std::path::Path;
use std::sync::Arc;

/// Most instances one scatter layer places.
const SCATTER_LIMIT: usize = 20_000;

pub fn register(app: &mut App) {
    material::register(app);
    app.init_resource::<TerrainCache>()
        .init_resource::<TerrainJobs>()
        .init_resource::<TerrainIndex>()
        .init_resource::<TerrainStats>()
        .init_resource::<brush::LiveStroke>()
        .add_observer(vegetation::swap_levels)
        .add_systems(
            Update,
            (
                sync_terrains,
                land_terrains,
                preview_erosion,
                brush::paint,
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

    pub fn from_parts(field: Heightfield, holes: Option<Grid>, spec: &TerrainSpec) -> Geometry {
        let shape = spec.shape();
        let layout = ChunkLayout::for_side(shape.side);
        let surface = field.surface_map(&shape);
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
}

/// Where a ray (in the terrain's frame) first meets the ground, marching
/// the heightfield and refining the crossing.
pub fn raycast(
    field: &Heightfield,
    shape: &Shape,
    origin: Vec3,
    direction: Vec3,
    max: f32,
) -> Option<Vec3> {
    let spacing = shape.spacing();
    let step = (spacing[0].min(spacing[1]) * 0.5).max(0.05);
    let above = |p: Vec3| field.height_at(shape, p.x, p.z).map(|h| p.y - h);
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

/// What a whole terrain draws from once built.
pub struct Built {
    pub geometry: Arc<Geometry>,
    /// Painted weights as stored, for the brushes to paint over.
    pub splat: Option<Grid>,
    pub weights: Arc<Vec<[u8; 4]>>,
    pub grass_maps: Vec<Option<Grid>>,
    pub scatter_maps: Vec<Option<Grid>>,
    pub scatter: Vec<Vec<Instance>>,
    /// Other actors' footprints, as scatter kept clear of them.
    pub avoid: Vec<Avoid>,
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
    preview: CellTasks<String, Built>,
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
    /// Whether streaming has counted the cells active when it landed.
    pub seeded: bool,
}

/// One chunk of a terrain's ground.
#[derive(Component)]
pub struct TerrainChunk;

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
    cache: Res<TerrainCache>,
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
            seeded: false,
        });
        if let Some(built) = cache.built.get(&key) {
            commands.queue(spawn_built(id.0.clone(), entity, built.clone()));
            continue;
        }
        let geometry = cache
            .geometry
            .get(&geometry_key(dir.as_deref(), &spec))
            .cloned();
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
    let grass_maps = spec
        .grass
        .iter()
        .map(|grass| store::grid_for(dir, &grass.density_map, side))
        .collect();
    let scatter_maps = spec
        .scatter
        .iter()
        .map(|layer| store::grid_for(dir, &layer.density_map, side))
        .collect();
    paint_from(spec, geometry, splat, grass_maps, scatter_maps, avoid)
}

/// [`paint`] from grids already in hand.
pub fn paint_from(
    spec: &TerrainSpec,
    geometry: Arc<Geometry>,
    splat: Option<Grid>,
    grass_maps: Vec<Option<Grid>>,
    scatter_maps: Vec<Option<Grid>>,
    avoid: &[Avoid],
) -> Built {
    let painted = splat.as_ref().and_then(Grid::to_weights);
    let weights = bake_weights(
        &geometry.field,
        &geometry.shape,
        &spec.layers,
        painted.as_deref(),
    );
    let scatter = spec
        .scatter
        .iter()
        .enumerate()
        .map(|(k, layer)| {
            let map = scatter_maps.get(k).and_then(Option::as_ref);
            let avoid = if layer.avoid_actors { avoid } else { &[] };
            scatter_instances(
                &geometry.ground(Some(&weights)),
                layer,
                map.and_then(Grid::mask),
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
        scatter_maps,
        scatter,
        avoid: avoid.to_vec(),
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
        cache.geometry.insert(
            geometry_key(dir.as_deref(), &root.spec),
            built.geometry.clone(),
        );
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
        for chunk in &geometry.chunks {
            let (group, first, count) = resident_group(world, geometry, chunk);
            triangles += count;
            let entity = world
                .spawn((
                    TerrainChunk,
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
            terrained.seeded = false;
            terrained.fine.clear();
        }
        world
            .resource_mut::<TerrainIndex>()
            .terrains
            .insert(id, (root, built));
    }
}

/// Takes a terrain's chunks, collider, grass and scatter down and spawns
/// them again from `built`, for an erosion preview and putting it back.
pub fn respawn(
    commands: &mut Commands,
    id: String,
    root: Entity,
    terrained: &mut Terrained,
    built: Arc<Built>,
) {
    for &part in terrained.chunks.iter().chain(&terrained.parts) {
        commands.entity(part).try_despawn();
    }
    for slot in terrained.grass_cells.values() {
        if let vegetation::GrassSlot::Drawn(part) = *slot {
            commands.entity(part).try_despawn();
        }
    }
    terrained.chunks.clear();
    terrained.parts.clear();
    terrained.grass_cells.clear();
    terrained.fine.clear();
    terrained.built = None;
    commands.queue(spawn_built(id, root, built));
}

/// Runs the erosion previews the editor asked for off the main thread, and
/// swaps each terrain to its preview (or back) when ready.
pub fn preview_erosion(
    mut commands: Commands,
    mut engine: NonSendMut<Engine>,
    cache: Res<TerrainCache>,
    mut jobs: ResMut<TerrainJobs>,
    mut cells: ResMut<StreamingCells>,
    mut roots: Query<(&Transform, &mut Terrained)>,
) {
    for (id, erosion) in std::mem::take(&mut engine.terrain_previews) {
        let Some(&root) = engine.entities.get(&id) else {
            continue;
        };
        let Ok((transform, mut terrained)) = roots.get_mut(root) else {
            continue;
        };
        jobs.preview.cancel(&mut cells, &id);
        let Some(erosion) = erosion else {
            if let Some(built) = cache.built.get(&terrained.key).cloned() {
                respawn(&mut commands, id, root, &mut terrained, built);
            }
            continue;
        };
        let Some(built) = cache.built.get(&terrained.key).cloned() else {
            continue;
        };
        let spec = terrained.spec.clone();
        let cell = StreamingCells::cell_at(transform.translation);
        jobs.preview.spawn(&mut cells, id, cell, move || {
            let mut field = built.geometry.field.clone();
            erosion.apply(&mut field, &built.geometry.shape);
            let geometry = Geometry::from_parts(field, built.geometry.holes.clone(), &spec);
            paint_from(
                &spec,
                Arc::new(geometry),
                built.splat.clone(),
                built.grass_maps.clone(),
                built.scatter_maps.clone(),
                &built.avoid,
            )
        });
    }
    for (id, preview) in jobs.preview.poll(&mut cells) {
        let Some(&root) = engine.entities.get(&id) else {
            continue;
        };
        if let Ok((_, mut terrained)) = roots.get_mut(root) {
            respawn(&mut commands, id, root, &mut terrained, Arc::new(preview));
        }
    }
}

/// A chunk's LOD group over its resident meshes, the mesh to start on and
/// its triangle count.
fn resident_group(
    world: &mut World,
    geometry: &Geometry,
    chunk: &Chunk,
) -> (LodGroup, Handle<Mesh>, usize) {
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
    mut roots: Query<(&ActorId, &Transform, &mut Terrained)>,
    mut chunks: Query<(&mut LodGroup, &mut Mesh3d), With<TerrainChunk>>,
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
        // A terrain that just landed starts from every cell already active.
        let events: Vec<(Cell, i32)> = if terrained.seeded {
            entered
                .iter()
                .map(|c| (*c, 1))
                .chain(left.iter().map(|c| (*c, -1)))
                .collect()
        } else {
            terrained.seeded = true;
            cells.active.iter().map(|c| (*c, 1)).collect()
        };
        let affine = transform.compute_affine();
        for (cell, delta) in events {
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
                    let cell =
                        StreamingCells::cell_at(affine.transform_point3(Vec3::from(chunk.origin)));
                    jobs.fine.spawn(&mut cells, key, cell, move || {
                        (0..geometry.layout.resident_level())
                            .map(|level| (level, geometry.mesh(cx, cz, level).0))
                            .collect()
                    });
                } else if was > 0 && now == 0 {
                    jobs.fine.cancel(&mut cells, &key);
                    let group =
                        resident_only(geometry, chunk, &mut meshes, &terrained.chunks, &mut chunks);
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
        let Ok((mut group, mut mesh)) = chunks.get_mut(entity) else {
            continue;
        };
        let chunk = &built.geometry.chunks[index as usize];
        let mut next = LodGroup::new(chunk.bounds.radius());
        let mut added = 0;
        for (level, part) in &fine {
            added = added.max(part.triangles());
            next = next.level(
                chunk.thresholds[*level as usize],
                Some(meshes.add(to_bevy(part))),
            );
        }
        // Levels already streamed stay; only the resident ones carry over.
        let resident =
            (built.geometry.layout.levels - built.geometry.layout.resident_level()) as usize;
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
    chunks: &mut Query<(&mut LodGroup, &mut Mesh3d), With<TerrainChunk>>,
) -> Option<usize> {
    let index = (chunk.cz * geometry.layout.chunks + chunk.cx) as usize;
    let (mut group, mut mesh) = chunks.get_mut(*entities.get(index)?).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(resolution: u32) -> TerrainSpec {
        TerrainSpec {
            size: [64.0, 64.0],
            height: 10.0,
            resolution,
            ..TerrainSpec::default()
        }
    }

    /// A 0.2-high plain with a round hill in the middle.
    fn hill(side: u32) -> Heightfield {
        let mut field = Heightfield::flat(side, 0.2);
        let c = (side - 1) as f32 * 0.5;
        for j in 0..side {
            for i in 0..side {
                let d = ((i as f32 - c).powi(2) + (j as f32 - c).powi(2)).sqrt() / c;
                let k = field.index(i, j);
                field.samples[k] += (1.0 - d).max(0.0) * 0.5;
            }
        }
        field
    }

    #[test]
    fn geometry_keeps_resident_levels_and_raycasts_onto_the_ground() {
        let spec = spec(1025);
        let geometry = Geometry::from_parts(hill(1025), None, &spec);
        let layout = geometry.layout;
        assert_eq!(geometry.chunks.len() as u32, layout.chunks * layout.chunks);
        let resident = (layout.levels - layout.resident_level()) as usize;
        assert!(geometry.chunks.iter().all(|c| c.resident.len() == resident));
        assert!(
            layout.resident_level() > 0,
            "1025 streams its finest levels"
        );
        let top = raycast(
            &geometry.field,
            &geometry.shape,
            Vec3::new(0.0, 50.0, 0.0),
            Vec3::NEG_Y,
            100.0,
        )
        .unwrap();
        assert!((top.y - 7.0).abs() < 0.1, "hilltop at 7 m, got {}", top.y);
        let slanted = raycast(
            &geometry.field,
            &geometry.shape,
            Vec3::new(-40.0, 30.0, -40.0),
            Vec3::new(1.0, -1.0, 1.0).normalize(),
            200.0,
        )
        .unwrap();
        let ground = geometry
            .field
            .height_at(&geometry.shape, slanted.x, slanted.z)
            .unwrap();
        assert!((slanted.y - ground).abs() < 0.05);
    }

    fn drop_ball(holes: Option<Grid>) -> f32 {
        let spec = spec(129);
        let geometry = Geometry::from_parts(hill(129), holes, &spec);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin));
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.add_plugins(rp::RapierPhysicsPlugin::<rp::NoUserData>::default());
        let root = app.world_mut().spawn(Transform::default()).id();
        spawn_collider(app.world_mut(), root, &geometry);
        let ball = app
            .world_mut()
            .spawn((
                rp::RigidBody::Dynamic,
                rp::Collider::ball(0.5),
                Transform::from_xyz(0.0, 12.0, 0.0),
            ))
            .id();
        for _ in 0..180 {
            app.update();
        }
        app.world().get::<Transform>(ball).unwrap().translation.y
    }

    #[test]
    fn bodies_land_on_the_ground_and_fall_through_its_holes() {
        let landed = drop_ball(None);
        assert!(
            (landed - 7.5).abs() < 0.3,
            "rests on the hilltop, got {landed}"
        );
        let mut holes = Grid::new(store::GridKind::Mask8, 129);
        let mask = holes.mask_mut().unwrap();
        for j in 60..68 {
            for i in 60..68 {
                mask[j * 129 + i] = 255;
            }
        }
        let fell = drop_ball(Some(holes));
        assert!(fell < 0.0, "fell through the cave mouth, got {fell}");
    }
}
