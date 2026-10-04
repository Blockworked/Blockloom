//! Bounded connectivity and conservative support across page borders.
use crate::{Grid, Shape, Surface, World, grid, mesher, serde_value, smooth};
use blockloom_plugin_api::mesh::{ColliderKind, MeshBody, MeshData};
use blockloom_plugin_sdk::{Error, Value, json};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

#[derive(Serialize, Deserialize, Clone)]
pub struct Fragment {
    pub id: u64,
    pub grid: grid::Snapshot,
    pub origin: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    #[serde(default)]
    pub angular_velocity: [f32; 3],
}

pub fn name(id: u64) -> String {
    format!("fragment/{id}")
}

impl Fragment {
    pub fn mesh(&self, world: &World) -> Result<Value, Error> {
        let grid = Grid::restore(self.grid.clone(), world.palette.len()).map_err(Error::new)?;
        let mut merged = mesher::Group::default();
        let mut emission = [0.0f32; 3];
        let mut glowing = 0usize;
        let mut mass = 0.0;
        for (cell, m) in grid.solid_cells() {
            if m == 0 {
                continue;
            }
            let volume = match grid.shape_at(cell) {
                Shape::Cube => 1.0,
                Shape::Slab | Shape::TopSlab | Shape::Ramp(_) => 0.5,
                Shape::Post => 0.25,
                Shape::Stair(_) => 0.75,
            };
            let density = world
                .settings
                .palette_density
                .get(m as usize - 1)
                .copied()
                .unwrap_or(1000.0);
            mass += density as f32 * volume * world.voxel.powi(3);
        }
        let n = grid.section_counts();
        for z in 0..n[2] {
            for y in 0..n[1] {
                for x in 0..n[0] {
                    let groups = match world.surface {
                        Surface::Cubes => {
                            mesher::mesh_chunk(&grid, &world.palette, [x, y, z], world.voxel)
                        }
                        Surface::Smooth => {
                            smooth::mesh_chunk(&grid, &world.palette, [x, y, z], world.voxel)
                        }
                    };
                    for (glow, group) in groups {
                        let base = merged.positions.len() as u32 / 3;
                        for p in group.positions.as_chunks::<3>().0 {
                            merged.positions.extend([
                                p[0] + (x * grid::SECTION) as f32 * world.voxel,
                                p[1] + (y * grid::SECTION) as f32 * world.voxel,
                                p[2] + (z * grid::SECTION) as f32 * world.voxel,
                            ]);
                        }
                        if let Some(m) = glow
                            && let Some(look) = world.palette.get(m)
                        {
                            for (a, color) in look.color.iter().enumerate() {
                                emission[a] +=
                                    color * look.emission * group.normals.len() as f32 / 3.0;
                            }
                            glowing += group.normals.len() / 3;
                        }
                        merged.normals.extend(group.normals);
                        merged.colors.extend(group.colors);
                        merged
                            .indices
                            .extend(group.indices.iter().map(|i| i + base));
                    }
                }
            }
        }
        let count = merged.positions.len() / 3;
        if glowing > 0 {
            emission = emission.map(|v| v / count as f32);
        }
        let data = MeshData {
            name: name(self.id),
            positions: merged.positions,
            normals: merged.normals,
            colors: merged.colors,
            indices: merged.indices,
            origin: self.origin,
            emission: (glowing > 0).then_some(emission),
            roughness: 0.9,
            transition_ms: 0,
            collider: true,
            collider_kind: ColliderKind::ConvexHull,
            gpu: None,
            uvs: Vec::new(),
            texture: None,
            body: Some(MeshBody {
                mass: mass.max(0.001),
                velocity: self.velocity,
                angular_velocity: self.angular_velocity,
                rotation: self.rotation,
            }),
        };
        data.check().map_err(Error::new)?;
        let mut value = serde_value(&data);
        value["effect"] = json!("mesh");
        Ok(value)
    }
}

pub fn detach(world: &mut World, a: [i32; 3], b: [i32; 3]) -> Result<Value, Error> {
    let lo = [0, 1, 2].map(|i| a[i].min(b[i]).max(0));
    let hi = [0, 1, 2].map(|i| a[i].max(b[i]).min(world.grid.size()[i] - 1));
    let volume: i64 = (0..3)
        .map(|i| i64::from((hi[i] - lo[i] + 1).max(0)))
        .product();
    if volume > 262144 {
        return Err(Error::bad_argument(
            "fracture region holds at most 262144 cells",
        ));
    }
    let mut unseen = BTreeSet::new();
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                if world.grid.get([x, y, z]) != 0 {
                    unseen.insert([x, y, z]);
                }
            }
        }
    }
    let directions = [
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ];
    let mut regions = Vec::new();
    while let Some(&first) = unseen.first() {
        unseen.remove(&first);
        let mut queue = VecDeque::from([first]);
        let mut region = Vec::new();
        let mut anchored = false;
        while let Some(cell) = queue.pop_front() {
            region.push(cell);
            anchored |= cell[1] <= world.settings.anchor_y;
            for d in directions {
                let next = [0, 1, 2].map(|i| cell[i] + d[i]);
                if unseen.remove(&next) {
                    queue.push_back(next);
                } else if (0..3).any(|i| next[i] < lo[i] || next[i] > hi[i])
                    && world.grid.get(next) != 0
                {
                    anchored = true;
                }
            }
        }
        if !anchored {
            regions.push(region);
        }
    }
    let mut detached = Vec::new();
    let mut pending = 0;
    let mut cells = 0;
    let mut meshes = Vec::new();
    for region in regions {
        if world.fragments.len() + detached.len() >= world.settings.max_fragments
            || region.len() > world.settings.max_fragment_cells
        {
            pending += 1;
            continue;
        }
        let min = [0, 1, 2].map(|i| region.iter().map(|c| c[i]).min().unwrap());
        let max = [0, 1, 2].map(|i| region.iter().map(|c| c[i]).max().unwrap());
        if (0..3)
            .any(|i| max[i] - min[i] + 1 + i32::from(world.surface == Surface::Smooth) * 2 > 128)
        {
            pending += 1;
            continue;
        }
        let mut grid = Grid::new([0, 1, 2].map(|i| max[i] - min[i] + 1));
        // Preserve a smooth fragment's positive interpolation halo too.
        if world.surface == Surface::Smooth {
            // Use one-cell padding so the fragment retains surface positions.
            grid = Grid::new([0, 1, 2].map(|i| max[i] - min[i] + 3));
        }
        let offset = min.map(|v| v - i32::from(world.surface == Surface::Smooth));
        let region_set: BTreeSet<_> = region.iter().copied().collect();
        for &cell in &region {
            let local = [0, 1, 2].map(|i| cell[i] - offset[i]);
            grid.set_shaped(local, world.grid.get(cell), world.grid.shape_at(cell));
            if world.grid.shape_at(cell) == Shape::Cube {
                grid.set_density(local, world.grid.density(cell), world.grid.get(cell));
            }
            if world.surface == Surface::Smooth {
                for z in -1..=1 {
                    for y in -1..=1 {
                        for x in -1..=1 {
                            let next = [cell[0] + x, cell[1] + y, cell[2] + z];
                            if !region_set.contains(&next) && world.grid.density(next) > 0 {
                                let at = [0, 1, 2].map(|i| next[i] - offset[i]);
                                grid.set_density(at, world.grid.density(next), 0);
                            }
                        }
                    }
                }
            }
        }
        let id = world.next_fragment + detached.len() as u64;
        let fragment = Fragment {
            id,
            grid: grid.snapshot(),
            origin: [0, 1, 2].map(|i| world.origin[i] + offset[i] as f32 * world.voxel),
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        };
        meshes.push(fragment.mesh(world)?);
        detached.push((fragment, region));
    }
    for ((fragment, region), mesh) in detached.into_iter().zip(meshes) {
        cells += region.len();
        let mut pages = BTreeSet::new();
        for cell in region {
            // Wait for every old source collider, including the smooth halo.
            let lo = cell.map(|v| (v - 2).max(0) / grid::SECTION);
            let hi = cell.map(|v| (v + 1) / grid::SECTION);
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        pages.insert([x, y, z]);
                    }
                }
            }
            world.grid.set(cell, 0);
        }
        world.next_fragment = fragment.id + 1;
        world.pending_fragments.insert(fragment.id, (pages, mesh));
        world.fragments.insert(fragment.id, fragment);
    }
    let mut effects = world.flush();
    if cells > 0 {
        effects.push(json!({"effect":"nav_dirty"}));
    }
    if cells > 0 {
        effects.push(json!({"effect":"event","name":"fractured","args":[cells,world.fragments.len(),pending]}));
    }
    Ok(
        json!({"effects":effects,"detached_cells":cells,"fragments":world.fragments.len(),"pending":pending}),
    )
}

pub fn poses(world: &mut World, host: &blockloom_plugin_sdk::Host) {
    for fragment in world.fragments.values_mut() {
        if let Ok(pose) = host.call_json("world.mesh_pose", &json!({"name":name(fragment.id)})) {
            if let Ok(position) = crate::serde_json_from(pose["position"].clone()) {
                fragment.origin = position;
            }
            if let Ok(rotation) = crate::serde_json_from(pose["rotation"].clone()) {
                fragment.rotation = rotation;
            }
            if let Ok(spin) = crate::serde_json_from(pose["angular_velocity"].clone()) {
                fragment.angular_velocity = spin;
            }
            if let Ok(velocity) = crate::serde_json_from(pose["velocity"].clone()) {
                fragment.velocity = velocity;
            }
        }
    }
}

pub fn edit(
    world: &mut World,
    id: u64,
    change: impl FnOnce(&mut World) -> Result<u64, Error>,
) -> Result<Value, Error> {
    let mut fragment = world
        .fragments
        .get(&id)
        .ok_or_else(|| Error::bad_argument("unknown voxel fragment"))?
        .clone();
    let mut settings = world.settings.clone();
    settings.size = fragment.grid.size.map(f64::from);
    settings.streamed = false;
    settings.gpu_meshing = false;
    let mut local = World::new(&settings).map_err(Error::new)?;
    local.grid = Grid::restore(fragment.grid.clone(), world.palette.len()).map_err(Error::new)?;
    let changed = change(&mut local)?;
    if local.grid.solid_count() > world.settings.max_fragment_cells as u64 {
        return Err(Error::bad_argument("fragment cell budget exceeded"));
    }
    let mut effects = Vec::new();
    if changed > 0 {
        if local.grid.solid_count() == 0 {
            world.fragments.remove(&id);
            world.pending_fragments.remove(&id);
            effects.push(json!({"effect":"remove_mesh","name":name(id)}));
        } else {
            fragment.grid = local.grid.snapshot();
            let mesh = fragment.mesh(world)?;
            if let Some((_, queued)) = world.pending_fragments.get_mut(&id) {
                *queued = mesh;
            } else {
                effects.push(mesh);
            }
            world.fragments.insert(id, fragment);
        }
    }
    Ok(json!({"changed":changed,"effects":effects}))
}

fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let axis = [q[0], q[1], q[2]];
    let t = cross(axis, v).map(|v| v * 2.0);
    let c = cross(axis, t);
    [0, 1, 2].map(|i| v[i] + q[3] * t[i] + c[i])
}

pub fn cast(
    world: &World,
    id: u64,
    origin: [f64; 3],
    direction: [f64; 3],
    reach: f64,
) -> Result<Value, Error> {
    let fragment = world
        .fragments
        .get(&id)
        .ok_or_else(|| Error::bad_argument("unknown voxel fragment"))?;
    let q = fragment.rotation;
    let inverse = [-q[0], -q[1], -q[2], q[3]];
    let origin = rotate(
        inverse,
        [0, 1, 2].map(|i| origin[i] as f32 - fragment.origin[i]),
    )
    .map(|v| v / world.voxel);
    let direction = rotate(inverse, direction.map(|v| v as f32));
    let grid = Grid::restore(fragment.grid.clone(), world.palette.len()).map_err(Error::new)?;
    let hit = if world.surface == Surface::Smooth {
        crate::ray::cast_smooth(
            &grid,
            origin,
            direction,
            (reach / world.voxel as f64) as f32,
        )
    } else {
        crate::ray::cast(
            &grid,
            origin,
            direction,
            (reach / world.voxel as f64) as f32,
        )
    };
    Ok(hit.map_or(json!({"hit":false,"value":-1}),|hit|json!({"hit":true,"cell":hit.cell,"before":hit.before(),"material":grid.get(hit.cell),"normal":rotate(q,hit.normal.map(|v|v as f32)),"distance":crate::round(hit.distance*world.voxel),"value":crate::round(hit.distance*world.voxel)})))
}
