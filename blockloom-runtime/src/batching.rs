//! Instancing and mesh batching for 3D actors.
//!
//! Plain surfaces draw through [`InstancedMaterial`]: the material is keyed by
//! what the surface is made of, while each actor's tint and UV transform live
//! in one storage buffer indexed by its `MeshTag`. So recoloring or retiling an
//! actor never re-keys its batch, and a hundred crates of one size in a hundred
//! colors are one mesh, one material and one instanced draw.
//!
//! What instancing can't group merges instead. Actors that stay put merge into
//! one mesh per streaming cell and surface (static batching), built off the
//! main thread as a streaming payload, and small moving
//! meshes merge into one mesh per surface every frame (dynamic batching). A
//! batched actor keeps its entity, body and pose; only its own draw is turned
//! off, through an empty `RenderLayers`. [`BatchPolicy`] holds the numbers.

use crate::engine::ActorId;
use crate::materials::BoxMaterial;
use crate::streaming::{CellTasks, StreamingCells};
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::MeshAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::math::{Affine3A, Vec3A};
use bevy::mesh::{Indices, MeshTag, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use bevy_rapier3d::prelude as rp;
use blockloom_core::material::SurfaceMaterial;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub type InstancedMaterial = ExtendedMaterial<StandardMaterial, InstanceData>;

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/instanced_pbr.wesl");
    app.add_plugins(MaterialPlugin::<InstancedMaterial>::default())
        .init_resource::<InstanceTable>()
        .init_resource::<BatchPolicy>()
        .init_resource::<Batches>();
}

/// The per-actor half of an instanced surface: every instanced material
/// shares the one buffer.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct InstanceData {
    #[storage(100, read_only)]
    pub instances: Handle<ShaderBuffer>,
}

impl MaterialExtension for InstanceData {
    fn fragment_shader() -> ShaderRef {
        instanced_shader()
    }

    // The same file writes the G-buffer when ray tracing has surfaces go
    // deferred, so tint and UVs reach what Solari lights.
    fn deferred_fragment_shader() -> ShaderRef {
        instanced_shader()
    }
}

fn instanced_shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
            "shaders/instanced_pbr.wesl"
        ))
        .with_source("embedded"),
    )
}

/// One actor's slot in the instance buffer. The layout is fixed whatever the
/// material's texture slots hold, so new per-actor data takes a reserved lane.
/// Uploaded as raw bytes, so it stays `repr(C)` vec4s with no padding.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct InstanceRecord {
    /// Linear RGBA multiplied over the surface's base color.
    pub tint: Vec4,
    /// UV scale X/Y, offset X/Y.
    pub uv: Vec4,
    /// UV rotation in radians; the rest is reserved.
    pub options: Vec4,
}

impl InstanceRecord {
    pub const IDENTITY: Self = Self {
        tint: Vec4::ONE,
        uv: Vec4::new(1.0, 1.0, 0.0, 0.0),
        options: Vec4::ZERO,
    };

    pub fn of(material: Option<&SurfaceMaterial>, color: Color) -> Self {
        let tint = LinearRgba::from(color).to_vec4();
        match material {
            Some(material) => Self {
                tint,
                uv: crate::materials::uv_scale_offset(material),
                options: Vec4::new(material.rotation.to_radians(), 0.0, 0.0, 0.0),
            },
            None => Self {
                tint,
                ..Self::IDENTITY
            },
        }
    }

    /// The same UV transform as a standard material's, for a surface drawn
    /// without the instance buffer.
    pub fn uv_transform(&self) -> bevy::math::Affine2 {
        bevy::math::Affine2::from_translation(self.uv.zw())
            * bevy::math::Affine2::from_angle(self.options.x)
            * bevy::math::Affine2::from_scale(self.uv.xy())
    }

    /// The shader's UV transform, for baking into a merged mesh.
    fn map_uv(&self, uv: Vec2) -> Vec2 {
        Vec2::from_angle(self.options.x).rotate(uv * self.uv.xy()) + self.uv.zw()
    }
}

/// The instanced material's cache key: the surface minus everything that
/// rides per instance, so tint and UV layout never split a batch.
pub fn surface_key(dir: Option<&Path>, material: Option<&SurfaceMaterial>) -> String {
    let shared = material.map(|material| SurfaceMaterial {
        tiling: [1.0, 1.0],
        offset: [0.0, 0.0],
        rotation: 0.0,
        texel_density: 1.0,
        ..material.clone()
    });
    format!(
        "instanced:{dir:?}:{}",
        serde_json::to_string(&shared).unwrap_or_default()
    )
}

/// On an actor drawn through [`InstancedMaterial`]: its slot, which is also
/// its `MeshTag`.
#[derive(Component, Clone, Copy)]
pub struct InstanceSlot(pub u32);

/// The CPU copy of the instance buffer and who owns each slot.
#[derive(Resource)]
pub struct InstanceTable {
    records: Vec<InstanceRecord>,
    free: Vec<u32>,
    owners: HashMap<Entity, u32>,
    buffer: Option<Handle<ShaderBuffer>>,
    uploaded: usize,
    dirty: bool,
}

impl Default for InstanceTable {
    fn default() -> Self {
        Self {
            records: vec![InstanceRecord::IDENTITY],
            free: Vec::new(),
            owners: HashMap::new(),
            buffer: None,
            uploaded: 0,
            dirty: true,
        }
    }
}

impl InstanceTable {
    /// Grows by doubling, so the GPU buffer (and the bind groups naming it)
    /// is only replaced when it runs out.
    const MIN_CAPACITY: usize = 1024;

    /// The entity's slot, reused if it already has one.
    pub fn assign(&mut self, entity: Entity, record: InstanceRecord) -> u32 {
        let slot = match self.owners.get(&entity) {
            Some(slot) => *slot,
            None => {
                let slot = self.free.pop().unwrap_or_else(|| {
                    self.records.push(InstanceRecord::IDENTITY);
                    (self.records.len() - 1) as u32
                });
                self.owners.insert(entity, slot);
                slot
            }
        };
        self.set(slot, record);
        slot
    }

    /// What a slot holds right now.
    pub fn record(&self, slot: u32) -> Option<InstanceRecord> {
        self.records.get(slot as usize).copied()
    }

    pub fn set(&mut self, slot: u32, record: InstanceRecord) {
        if let Some(current) = self.records.get_mut(slot as usize)
            && *current != record
        {
            *current = record;
            self.dirty = true;
        }
    }

    pub fn get(&self, slot: u32) -> Option<InstanceRecord> {
        self.records.get(slot as usize).copied()
    }

    fn release(&mut self, entity: Entity) {
        if let Some(slot) = self.owners.remove(&entity) {
            self.records[slot as usize] = InstanceRecord::IDENTITY;
            self.free.push(slot);
            self.dirty = true;
        }
    }

    fn padded(&self) -> Vec<InstanceRecord> {
        let capacity = self
            .records
            .len()
            .next_power_of_two()
            .max(Self::MIN_CAPACITY);
        let mut data = self.records.clone();
        data.resize(capacity, InstanceRecord::IDENTITY);
        data
    }
}

fn instance_buffer(world: &mut World) -> Option<Handle<ShaderBuffer>> {
    if let Some(buffer) = world.get_resource::<InstanceTable>()?.buffer.clone() {
        return Some(buffer);
    }
    let data = world.get_resource::<InstanceTable>()?.padded();
    let uploaded = data.len();
    let buffer = world
        .get_resource_mut::<Assets<ShaderBuffer>>()?
        .add(ShaderBuffer::from(data));
    let mut table = world.resource_mut::<InstanceTable>();
    table.buffer = Some(buffer.clone());
    table.uploaded = uploaded;
    table.dirty = false;
    Some(buffer)
}

/// Draw `entity` through the instanced surface cached under `key`, building it
/// from `base` on a miss. Runs as a queued command, since it needs the stores
/// no spawner has in hand. Does nothing where instancing isn't set up.
pub fn attach_instanced(
    world: &mut World,
    entity: Entity,
    key: String,
    base: Option<StandardMaterial>,
    record: InstanceRecord,
) {
    if world.get_entity(entity).is_err() {
        return;
    }
    let Some(buffer) = instance_buffer(world) else {
        return;
    };
    let Some(handle) =
        world.resource_scope(|world, mut cache: Mut<crate::performance::RenderCache>| {
            let mut materials = world.get_resource_mut::<Assets<InstancedMaterial>>()?;
            Some(cache.instanced_material(
                key,
                || InstancedMaterial {
                    base: base.unwrap_or_default(),
                    extension: InstanceData { instances: buffer },
                },
                &mut materials,
            ))
        })
    else {
        return;
    };
    let slot = world.resource_mut::<InstanceTable>().assign(entity, record);
    world.entity_mut(entity).insert((
        MeshMaterial3d(handle),
        MeshTag::new(slot),
        InstanceSlot(slot),
    ));
}

/// Recolor an instanced actor in place. Answers false for any other surface.
pub fn set_tint(world: &mut World, entity: Entity, color: Color) -> bool {
    let Some(slot) = world.get::<InstanceSlot>(entity).copied() else {
        return false;
    };
    let mut table = world.resource_mut::<InstanceTable>();
    let Some(mut record) = table.get(slot.0) else {
        return false;
    };
    record.tint = LinearRgba::from(color).to_vec4();
    table.set(slot.0, record);
    true
}

/// An instanced actor's tint, which rides in its slot rather than the material.
pub fn tint_of(world: &World, entity: Entity) -> Option<LinearRgba> {
    let slot = world.get::<InstanceSlot>(entity)?;
    let record = world.resource::<InstanceTable>().get(slot.0)?;
    Some(LinearRgba::from_vec4(record.tint))
}

/// Free the slots of actors that lost their instanced surface and upload the
/// table when anything moved.
pub fn upload_instances(
    mut table: ResMut<InstanceTable>,
    mut removed: RemovedComponents<InstanceSlot>,
    slots: Query<(), With<InstanceSlot>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<InstancedMaterial>>,
) {
    for entity in removed.read() {
        // A surface swap removes and re-adds the slot in one flush.
        if !slots.contains(entity) {
            table.release(entity);
        }
    }
    if !table.dirty {
        return;
    }
    let Some(handle) = table.buffer.clone() else {
        return;
    };
    let data = table.padded();
    let grew = data.len() != table.uploaded;
    if let Some(mut buffer) = buffers.get_mut(&handle) {
        *buffer = ShaderBuffer::from(data.clone());
    }
    table.uploaded = data.len();
    table.dirty = false;
    if grew {
        // A bigger buffer is a new GPU buffer: rebind every material to it.
        for (_, material) in materials.iter_mut() {
            material.extension.instances = handle.clone();
        }
    }
}

/// The numbers the batcher works to. Phase 5 tunes these per content type;
/// the mechanism reads nothing else.
#[derive(Resource, Clone, Debug)]
pub struct BatchPolicy {
    pub static_batching: bool,
    /// Frames an actor must hold still before it merges.
    pub settle_frames: u32,
    /// A cell's batch is only drawn once it would replace this many draws.
    pub static_min_members: usize,
    pub dynamic_batching: bool,
    pub dynamic_max_vertices: usize,
    pub dynamic_min_members: usize,
    /// At this many actors sharing one mesh and surface, GPU instancing
    /// already draws them in one call, so they stay out of merged batches.
    pub instance_threshold: usize,
}

impl Default for BatchPolicy {
    fn default() -> Self {
        Self {
            static_batching: true,
            settle_frames: 30,
            static_min_members: 2,
            dynamic_batching: true,
            dynamic_max_vertices: 300,
            dynamic_min_members: 4,
            instance_threshold: 4,
        }
    }
}

/// The surfaces a merged batch can draw with: both opaque, both unchanged
/// by where the vertices sit, since box projection reads world position.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Surface {
    Instanced(Handle<InstancedMaterial>),
    Boxed(Handle<BoxMaterial>),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct GroupKey {
    cell: (i32, i32),
    surface: Surface,
}

#[derive(Clone, PartialEq, Debug)]
struct Snapshot {
    pose: Affine3A,
    mesh: AssetId<Mesh>,
    record: Option<InstanceRecord>,
}

impl Snapshot {
    /// Interpolation re-writes a resting pose every frame, so poses compare
    /// with a tolerance.
    fn same_pose(&self, other: &Self) -> bool {
        self.pose.abs_diff_eq(other.pose, 1e-4)
    }

    fn same(&self, other: &Self) -> bool {
        self.same_pose(other) && self.mesh == other.mesh && self.record == other.record
    }
}

/// A cell's merge. It builds in the background, and its members draw
/// themselves until the result lands.
#[derive(Default)]
struct StaticGroup {
    entity: Option<Entity>,
    dirty: bool,
    /// Bumped per rebuild, so a merge for old membership is thrown away.
    generation: u64,
}

struct DynamicGroup {
    entity: Entity,
    mesh: Handle<Mesh>,
    members: Vec<(Entity, Snapshot)>,
}

/// Draw counts the profiler shows.
#[derive(Clone, Copy, Default, Debug)]
pub struct BatchStats {
    pub instanced_draws: usize,
    pub instanced_actors: usize,
    pub static_batches: usize,
    pub static_members: usize,
    pub dynamic_batches: usize,
    pub dynamic_members: usize,
}

#[derive(Resource, Default)]
pub struct Batches {
    settling: HashMap<Entity, (Snapshot, u32)>,
    members: HashMap<Entity, (GroupKey, Snapshot)>,
    groups: HashMap<GroupKey, StaticGroup>,
    /// Members that moved after merging. They broke the promise, so they
    /// draw themselves for the rest of the run.
    moved: HashSet<Entity>,
    dynamic: HashMap<Surface, DynamicGroup>,
    hidden: HashSet<Entity>,
    merging: CellTasks<GroupKey, (u64, Option<Mesh>)>,
    pub stats: BatchStats,
}

/// A merged batch's own entity. It carries no `ActorId`, so nothing that
/// looks for actors ever finds it.
#[derive(Component)]
pub struct MergedBatch;

struct Candidate {
    entity: Entity,
    surface: Surface,
    snapshot: Snapshot,
    fixed: bool,
    vertices: usize,
}

#[allow(clippy::type_complexity)]
pub fn batch_meshes(
    mut commands: Commands,
    policy: Res<BatchPolicy>,
    table: Res<InstanceTable>,
    mut batches: ResMut<Batches>,
    mut cells: ResMut<StreamingCells>,
    mut meshes: ResMut<Assets<Mesh>>,
    instanced: Res<Assets<InstancedMaterial>>,
    boxes: Res<Assets<BoxMaterial>>,
    actors: Query<
        (
            Entity,
            &Mesh3d,
            Option<&MeshMaterial3d<InstancedMaterial>>,
            Option<&MeshMaterial3d<BoxMaterial>>,
            Option<&InstanceSlot>,
            &GlobalTransform,
            &InheritedVisibility,
            Option<&rp::RigidBody>,
            Option<&crate::culling::LodGroup>,
        ),
        (
            With<ActorId>,
            Without<crate::materials::AnimatedTiles>,
            Without<crate::materials::TilemapLook>,
            Without<crate::model::ModelChild>,
            Without<crate::light_probes::ProbeOwner>,
        ),
    >,
) {
    let batches = &mut *batches;
    let mut candidates = Vec::new();
    for (entity, mesh, inst, boxed, slot, transform, visible, body, lod) in &actors {
        // A merged mesh can't change level, so LOD'd actors stay instanced.
        if !visible.get() || lod.is_some_and(|lod| lod.swaps_meshes()) {
            continue;
        }
        let surface = match (inst, boxed) {
            (Some(handle), _) => {
                let opaque = instanced
                    .get(&handle.0)
                    .is_some_and(|m| m.base.alpha_mode == AlphaMode::Opaque);
                if !opaque {
                    continue;
                }
                Surface::Instanced(handle.0.clone())
            }
            (None, Some(handle)) => {
                let opaque = boxes
                    .get(&handle.0)
                    .is_some_and(|m| m.base.alpha_mode == AlphaMode::Opaque);
                if !opaque {
                    continue;
                }
                Surface::Boxed(handle.0.clone())
            }
            _ => continue,
        };
        let Some(asset) = meshes.get(&mesh.0) else {
            continue;
        };
        if asset.primitive_topology() != PrimitiveTopology::TriangleList {
            continue;
        }
        let record = match surface {
            Surface::Instanced(_) => slot.and_then(|slot| table.get(slot.0)),
            Surface::Boxed(_) => None,
        };
        candidates.push(Candidate {
            entity,
            surface,
            snapshot: Snapshot {
                pose: transform.affine(),
                mesh: mesh.0.id(),
                record,
            },
            fixed: body.is_none_or(|body| *body == rp::RigidBody::Fixed),
            vertices: asset.count_vertices(),
        });
    }
    let mut copies: HashMap<(AssetId<Mesh>, &Surface), usize> = HashMap::new();
    for candidate in &candidates {
        *copies
            .entry((candidate.snapshot.mesh, &candidate.surface))
            .or_default() += 1;
    }
    let repeated = |candidate: &Candidate| {
        copies[&(candidate.snapshot.mesh, &candidate.surface)] >= policy.instance_threshold
    };
    let current: HashMap<Entity, &Candidate> = candidates.iter().map(|c| (c.entity, c)).collect();

    // Static members that changed leave their group.
    let members: Vec<Entity> = batches.members.keys().copied().collect();
    for entity in members {
        let (key, merged) = &batches.members[&entity];
        let leave = match current.get(&entity) {
            None => Some(false),
            Some(c)
                if !policy.static_batching
                    || !c.fixed
                    || c.surface != key.surface
                    || repeated(c) =>
            {
                Some(false)
            }
            Some(c) if !c.snapshot.same_pose(merged) => Some(true),
            Some(c) if !c.snapshot.same(merged) => Some(false),
            _ => None,
        };
        if let Some(moved) = leave {
            let (key, _) = batches.members.remove(&entity).unwrap();
            if let Some(group) = batches.groups.get_mut(&key) {
                group.dirty = true;
            }
            if moved {
                batches.moved.insert(entity);
            }
        }
    }
    batches.moved.retain(|entity| current.contains_key(entity));
    batches
        .settling
        .retain(|entity, _| current.contains_key(entity));

    // Actors that held still long enough join their cell's group.
    if policy.static_batching {
        for candidate in &candidates {
            if !candidate.fixed
                || repeated(candidate)
                || batches.members.contains_key(&candidate.entity)
                || batches.moved.contains(&candidate.entity)
            {
                continue;
            }
            let entry = batches
                .settling
                .entry(candidate.entity)
                .or_insert_with(|| (candidate.snapshot.clone(), 0));
            if entry.0.same(&candidate.snapshot) {
                entry.1 += 1;
            } else {
                *entry = (candidate.snapshot.clone(), 1);
            }
            if entry.1 < policy.settle_frames {
                continue;
            }
            batches.settling.remove(&candidate.entity);
            let key = GroupKey {
                cell: StreamingCells::cell_at(candidate.snapshot.pose.translation.into()),
                surface: candidate.surface.clone(),
            };
            batches.groups.entry(key.clone()).or_default().dirty = true;
            batches
                .members
                .insert(candidate.entity, (key, candidate.snapshot.clone()));
        }
    }

    // Merges that landed, if their group hasn't changed since.
    for (key, (generation, mesh)) in batches.merging.poll(&mut cells) {
        let Some(group) = batches.groups.get_mut(&key) else {
            continue;
        };
        if group.generation != generation || group.dirty {
            continue;
        }
        if let Some(mesh) = mesh {
            let aabb = mesh.get_aabb();
            let entity = spawn_batch(&mut commands, &key.surface, meshes.add(mesh));
            if let Some(aabb) = aabb {
                commands.entity(entity).insert(aabb);
            }
            group.entity = Some(entity);
        }
    }

    let mut by_group: HashMap<&GroupKey, Vec<Entity>> = HashMap::new();
    for (entity, (key, _)) in &batches.members {
        by_group.entry(key).or_default().push(*entity);
    }
    let mut hidden = HashSet::new();
    let mut stats = BatchStats::default();
    let mut emptied = Vec::new();
    for (key, group) in &mut batches.groups {
        let mut entities = by_group.remove(key).unwrap_or_default();
        let drawn = entities.len() >= policy.static_min_members;
        if group.dirty {
            group.dirty = false;
            group.generation += 1;
            // The old merge no longer matches its members: they draw
            // themselves until the new one lands.
            if let Some(entity) = group.entity.take() {
                commands.entity(entity).despawn();
            }
            if drawn {
                entities.sort();
                let parts: Vec<(Mesh, Affine3A, Option<InstanceRecord>)> = entities
                    .iter()
                    .filter_map(|entity| {
                        let (_, snapshot) = &batches.members[entity];
                        Some((
                            meshes.get(snapshot.mesh)?.clone(),
                            snapshot.pose,
                            snapshot.record,
                        ))
                    })
                    .collect();
                let generation = group.generation;
                batches
                    .merging
                    .spawn(&mut cells, key.clone(), key.cell, move || {
                        let parts: Vec<Part> = parts
                            .iter()
                            .map(|(mesh, transform, record)| Part {
                                mesh,
                                transform: *transform,
                                record: *record,
                            })
                            .collect();
                        (generation, merge(&parts))
                    });
            } else {
                batches.merging.cancel(&mut cells, key);
            }
        }
        if drawn && group.entity.is_some() {
            hidden.extend(entities.iter().copied());
            stats.static_batches += 1;
            stats.static_members += entities.len();
        }
        if entities.is_empty() {
            emptied.push(key.clone());
        }
    }
    for key in emptied {
        batches.merging.cancel(&mut cells, &key);
        batches.groups.remove(&key);
    }

    // Small movers instancing can't group merge per surface, every frame.
    let mut wanted: HashMap<Surface, Vec<(Entity, Snapshot)>> = HashMap::new();
    if policy.dynamic_batching {
        for candidate in &candidates {
            if candidate.vertices <= policy.dynamic_max_vertices
                && !repeated(candidate)
                && !batches.members.contains_key(&candidate.entity)
            {
                wanted
                    .entry(candidate.surface.clone())
                    .or_default()
                    .push((candidate.entity, candidate.snapshot.clone()));
            }
        }
        wanted.retain(|_, list| list.len() >= policy.dynamic_min_members);
    }
    batches.dynamic.retain(|surface, group| {
        let keep = wanted.contains_key(surface);
        if !keep {
            commands.entity(group.entity).despawn();
        }
        keep
    });
    for (surface, mut list) in wanted {
        list.sort_by_key(|(entity, _)| *entity);
        hidden.extend(list.iter().map(|(entity, _)| *entity));
        stats.dynamic_batches += 1;
        stats.dynamic_members += list.len();
        if batches
            .dynamic
            .get(&surface)
            .is_some_and(|group| group.members == list)
        {
            continue;
        }
        let parts: Vec<Part> = list
            .iter()
            .filter_map(|(_, snapshot)| {
                Some(Part {
                    mesh: meshes.get(snapshot.mesh)?,
                    transform: snapshot.pose,
                    record: snapshot.record,
                })
            })
            .collect();
        let Some(mesh) = merge(&parts) else {
            continue;
        };
        match batches.dynamic.get_mut(&surface) {
            Some(group) => {
                if let Some(aabb) = mesh.get_aabb() {
                    commands.entity(group.entity).insert(aabb);
                }
                if let Some(mut asset) = meshes.get_mut(&group.mesh) {
                    *asset = mesh;
                }
                group.members = list;
            }
            None => {
                let aabb = mesh.get_aabb();
                let handle = meshes.add(mesh);
                let entity = spawn_batch(&mut commands, &surface, handle.clone());
                if let Some(aabb) = aabb {
                    commands.entity(entity).insert(aabb);
                }
                batches.dynamic.insert(
                    surface,
                    DynamicGroup {
                        entity,
                        mesh: handle,
                        members: list,
                    },
                );
            }
        }
    }

    // Hand each actor's own draw back or take it away, only where it changed.
    for entity in batches.hidden.difference(&hidden) {
        if let Ok(mut entity) = commands.get_entity(*entity) {
            entity.remove::<RenderLayers>();
        }
    }
    for entity in hidden.difference(&batches.hidden) {
        commands.entity(*entity).insert(RenderLayers::none());
    }
    batches.hidden = hidden;

    let mut draws = HashSet::new();
    for candidate in &candidates {
        if let Surface::Instanced(handle) = &candidate.surface
            && !batches.hidden.contains(&candidate.entity)
        {
            draws.insert((candidate.snapshot.mesh, handle.id()));
            stats.instanced_actors += 1;
        }
    }
    stats.instanced_draws = draws.len();
    batches.stats = stats;
}

fn spawn_batch(commands: &mut Commands, surface: &Surface, mesh: Handle<Mesh>) -> Entity {
    let mut entity = commands.spawn((MergedBatch, Mesh3d(mesh), Transform::IDENTITY));
    match surface {
        Surface::Instanced(handle) => entity.insert(MeshMaterial3d(handle.clone())),
        Surface::Boxed(handle) => entity.insert(MeshMaterial3d(handle.clone())),
    };
    entity.id()
}

/// One mesh placed in the world, for [`merge`].
pub struct Part<'a> {
    pub mesh: &'a Mesh,
    pub transform: Affine3A,
    /// An instanced actor's tint and UVs, baked into vertex colors and UVs.
    pub record: Option<InstanceRecord>,
}

/// Bake `parts` into one world-space triangle list. Parts without positions,
/// normals and UVs are skipped; tangents survive only if every part has them.
pub fn merge(parts: &[Part]) -> Option<Mesh> {
    let parts: Vec<&Part> = parts
        .iter()
        .filter(|part| {
            part.mesh.primitive_topology() == PrimitiveTopology::TriangleList
                && part.transform.matrix3.determinant().abs() > 1e-12
        })
        .collect();
    let tangents = parts
        .iter()
        .all(|part| part.mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
    let colored = parts.iter().any(|part| part.record.is_some());
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut tangent_out: Vec<[f32; 4]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for part in parts {
        let (
            Some(VertexAttributeValues::Float32x3(p)),
            Some(VertexAttributeValues::Float32x3(n)),
            Some(VertexAttributeValues::Float32x2(u)),
        ) = (
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION),
            part.mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
            part.mesh.attribute(Mesh::ATTRIBUTE_UV_0),
        )
        else {
            continue;
        };
        let base = positions.len() as u32;
        let linear = part.transform.matrix3;
        let normal_matrix = linear.inverse().transpose();
        let mirrored = linear.determinant() < 0.0;
        positions.extend(
            p.iter()
                .map(|v| part.transform.transform_point3(Vec3::from(*v)).to_array()),
        );
        normals.extend(n.iter().map(|v| {
            (normal_matrix * Vec3A::from(*v))
                .normalize_or_zero()
                .to_array()
        }));
        match part.record {
            Some(record) => {
                uvs.extend(u.iter().map(|uv| record.map_uv(Vec2::from(*uv)).to_array()))
            }
            None => uvs.extend(u.iter().copied()),
        }
        if colored {
            let tint = part.record.map_or(Vec4::ONE, |record| record.tint);
            colors.extend(std::iter::repeat_n(tint.to_array(), p.len()));
        }
        if tangents
            && let Some(VertexAttributeValues::Float32x4(t)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_TANGENT)
        {
            let sign = if mirrored { -1.0 } else { 1.0 };
            tangent_out.extend(t.iter().map(|t| {
                let turned = (linear * Vec3A::new(t[0], t[1], t[2])).normalize_or_zero();
                [turned.x, turned.y, turned.z, t[3] * sign]
            }));
        }
        let local: Vec<u32> = match part.mesh.indices() {
            Some(indices) => indices.iter().map(|i| i as u32).collect(),
            None => (0..p.len() as u32).collect(),
        };
        for triangle in local.as_chunks::<3>().0 {
            // A mirrored part turns its triangles inside out; turn them back.
            if mirrored {
                indices.extend([base + triangle[0], base + triangle[2], base + triangle[1]]);
            } else {
                indices.extend([base + triangle[0], base + triangle[1], base + triangle[2]]);
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices));
    if colored {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    if tangents {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangent_out);
    }
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Mesh {
        Cuboid::new(1.0, 1.0, 1.0).into()
    }

    #[test]
    fn slots_are_reused_and_slot_zero_stays_identity() {
        let mut table = InstanceTable::default();
        let a = Entity::from_raw_u32(1).unwrap();
        let b = Entity::from_raw_u32(2).unwrap();
        let red = InstanceRecord::of(None, Color::srgb(1.0, 0.0, 0.0));
        let first = table.assign(a, red);
        assert_ne!(first, 0);
        assert_eq!(table.assign(a, red), first);
        table.release(a);
        assert_eq!(table.assign(b, red), first);
        assert_eq!(table.get(0), Some(InstanceRecord::IDENTITY));
        assert_eq!(table.padded().len(), InstanceTable::MIN_CAPACITY);
    }

    #[test]
    fn color_and_tiling_do_not_split_the_surface_key() {
        let one = SurfaceMaterial {
            tiling: [2.0, 2.0],
            ..Default::default()
        };
        let mut two = SurfaceMaterial {
            rotation: 45.0,
            ..Default::default()
        };
        assert_eq!(surface_key(None, Some(&one)), surface_key(None, Some(&two)));
        two.metallic = 1.0;
        assert_ne!(surface_key(None, Some(&one)), surface_key(None, Some(&two)));
    }

    #[test]
    fn merged_parts_land_in_world_space_with_baked_tint() {
        let mesh = cube();
        let red = InstanceRecord::of(None, Color::srgb(1.0, 0.0, 0.0));
        let merged = merge(&[
            Part {
                mesh: &mesh,
                transform: Affine3A::from_translation(Vec3::new(10.0, 0.0, 0.0)),
                record: Some(red),
            },
            Part {
                mesh: &mesh,
                transform: Affine3A::from_scale(Vec3::new(2.0, 1.0, 1.0)),
                record: None,
            },
        ])
        .unwrap();
        let aabb = merged.get_aabb().unwrap();
        assert!((aabb.max().x - 10.5).abs() < 1e-5);
        assert!((aabb.min().x + 1.0).abs() < 1e-5);
        assert_eq!(merged.count_vertices(), mesh.count_vertices() * 2);
        let Some(VertexAttributeValues::Float32x4(colors)) =
            merged.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("no colors");
        };
        assert_eq!(colors[0], red.tint.to_array());
        assert_eq!(colors[mesh.count_vertices()], [1.0; 4]);
    }

    #[test]
    fn mirrored_parts_keep_their_winding() {
        let mesh = cube();
        let plain = merge(&[Part {
            mesh: &mesh,
            transform: Affine3A::IDENTITY,
            record: None,
        }])
        .unwrap();
        let mirrored = merge(&[Part {
            mesh: &mesh,
            transform: Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
            record: None,
        }])
        .unwrap();
        let first = |mesh: &Mesh| -> Vec<u32> {
            mesh.indices()
                .unwrap()
                .iter()
                .take(3)
                .map(|i| i as u32)
                .collect()
        };
        let (a, b) = (first(&plain), first(&mirrored));
        assert_eq!((a[0], a[1], a[2]), (b[0], b[2], b[1]));
    }

    #[test]
    fn instance_uv_matches_the_material_transform() {
        let material = SurfaceMaterial {
            tiling: [2.0, 3.0],
            offset: [0.25, 0.5],
            rotation: 90.0,
            ..Default::default()
        };
        let record = InstanceRecord::of(Some(&material), Color::WHITE);
        let affine = crate::materials::uv_transform(&material);
        let uv = Vec2::new(0.3, 0.7);
        assert!(
            record
                .map_uv(uv)
                .abs_diff_eq(affine.transform_point2(uv), 1e-5)
        );
    }
}
