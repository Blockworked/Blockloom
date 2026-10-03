//! Bounded coarse mesh jobs with shared sample halos and edit dependencies.

use crate::grid::Grid;
use crate::lod::{MAX_LEVEL, Sample};
use crate::mesher::{self, Group};
use crate::palette::Palette;
use crate::{Surface, smooth};
use std::collections::{BTreeMap, VecDeque};

pub const TILE: i32 = 8;
pub const MAX_TILES: usize = 4;
pub const BASE_VISITS_PER_POLL: usize = 65536;
pub const MAX_OUTPUT_WORDS: usize = 262144;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub level: u8,
    pub tile: [i32; 3],
}

pub struct Tile {
    pub key: Key,
    pub base: [i32; 3],
    pub extent: [i32; 3],
    pub revision: u64,
    pub visits: usize,
    lo: [i32; 3],
    hi: [i32; 3],
    samples: Vec<Sample>,
    pub groups: Option<BTreeMap<Option<u8>, Group>>,
}

impl Tile {
    fn new(key: Key, grid: &Grid) -> Result<Self, String> {
        if !(1..=MAX_LEVEL).contains(&key.level) {
            return Err("coarse mesh level must be between 1 and 4".into());
        }
        let scale = 1 << key.level;
        let size = grid.size().map(|s| (s + scale - 1) / scale);
        if (0..3).any(|a| key.tile[a] < 0 || key.tile[a] >= (size[a] + TILE - 1) / TILE) {
            return Err("coarse mesh tile is outside the world".into());
        }
        let base = key.tile.map(|c| c * TILE);
        let extent = [0, 1, 2].map(|a| (size[a] - base[a]).min(TILE));
        Ok(Self {
            key,
            base,
            extent,
            revision: grid.revision(),
            visits: 0,
            lo: base.map(|c| c - 2),
            hi: [0, 1, 2].map(|a| base[a] + extent[a] + 2),
            samples: Vec::new(),
            groups: None,
        })
    }

    fn n(&self) -> [usize; 3] {
        [0, 1, 2].map(|a| (self.hi[a] - self.lo[a]) as usize)
    }

    pub fn total(&self) -> usize {
        self.n().iter().product()
    }

    pub fn progress(&self) -> usize {
        self.samples.len()
    }

    pub fn dependencies(&self) -> ([i32; 3], [i32; 3]) {
        let scale = 1 << self.key.level;
        (self.lo.map(|c| c * scale), self.hi.map(|c| c * scale - 1))
    }

    fn sample(&self, at: [i32; 3]) -> Sample {
        let c = [0, 1, 2].map(|a| (at[a] - self.lo[a]) as usize);
        let n = self.n();
        self.samples[(c[2] * n[1] + c[1]) * n[0] + c[0]]
    }

    fn advance(
        &mut self,
        grid: &mut Grid,
        palette: &Palette,
        surface: Surface,
        voxel: f32,
    ) -> Result<(), String> {
        self.visits = 0;
        if self.groups.is_some() {
            return Ok(());
        }
        let n = self.n();
        let scale = 1 << self.key.level;
        let size = grid.size().map(|s| (s + scale - 1) / scale);
        let cost = (scale * scale * scale) as usize;
        while self.samples.len() < self.total() {
            let i = self.samples.len();
            let at = [
                self.lo[0] + (i % n[0]) as i32,
                self.lo[1] + ((i / n[0]) % n[1]) as i32,
                self.lo[2] + (i / (n[0] * n[1])) as i32,
            ];
            let in_bounds = (0..3).all(|a| (0..size[a]).contains(&at[a]));
            if in_bounds && self.visits + cost > BASE_VISITS_PER_POLL {
                break;
            }
            self.samples.push(grid.lod_sample(self.key.level, at)?);
            if in_bounds {
                self.visits += cost;
            }
        }
        if self.samples.len() != self.total() {
            return Ok(());
        }
        let width = voxel * scale as f32;
        let mut groups = match surface {
            Surface::Cubes => mesher::mesh_cubes(palette, self.base, self.extent, width, |at| {
                self.sample(at).material
            }),
            Surface::Smooth => {
                let mut groups =
                    smooth::mesh_samples(palette, self.base, self.extent, width, |at| {
                        let sample = self.sample(at);
                        (sample.density, sample.density_material)
                    });
                let proxies = mesher::mesh_cubes(palette, self.base, self.extent, width, |at| {
                    let sample = self.sample(at);
                    if sample.opacity > 0 && sample.opacity < 255 {
                        sample.material
                    } else {
                        0
                    }
                });
                for (glow, proxy) in proxies {
                    append(groups.entry(glow).or_default(), proxy);
                }
                groups
            }
        };
        let lo = self.base.map(|c| -(c * scale) as f32 * voxel);
        let hi = [0, 1, 2].map(|a| (grid.size()[a] - self.base[a] * scale) as f32 * voxel);
        for group in groups.values_mut() {
            *group = clip(group, lo, hi);
        }
        groups.retain(|_, group| !group.indices.is_empty());
        let words: usize = groups.values().map(words).sum();
        if words > MAX_OUTPUT_WORDS {
            return Err("coarse mesh exceeds the tile output budget".into());
        }
        self.groups = Some(groups);
        Ok(())
    }
}

pub fn words(group: &Group) -> usize {
    group.positions.len() + group.normals.len() + group.colors.len() + group.indices.len()
}

fn append(group: &mut Group, other: Group) {
    let offset = group.positions.len() as u32 / 3;
    group.positions.extend(other.positions);
    group.normals.extend(other.normals);
    group.colors.extend(other.colors);
    group
        .indices
        .extend(other.indices.into_iter().map(|i| i + offset));
}

fn clip(group: &Group, lo: [f32; 3], hi: [f32; 3]) -> Group {
    let mut result = group.clone();
    let mut moved = vec![false; group.positions.len() / 3];
    for (i, position) in result.positions.chunks_exact_mut(3).enumerate() {
        for a in 0..3 {
            let before = position[a];
            position[a] = before.clamp(lo[a], hi[a]);
            moved[i] |= before != position[a];
        }
    }
    if moved.iter().all(|v| !v) {
        return result;
    }
    // Project partial edge cells onto the exact world box, retaining outer caps.
    let mut projected = Group::default();
    for tri in group.indices.chunks_exact(3) {
        let p = tri
            .iter()
            .map(|&i| std::array::from_fn::<_, 3, _>(|a| result.positions[i as usize * 3 + a]))
            .collect::<Vec<_>>();
        let a = std::array::from_fn::<_, 3, _>(|axis| p[1][axis] - p[0][axis]);
        let b = std::array::from_fn::<_, 3, _>(|axis| p[2][axis] - p[0][axis]);
        let normal = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length <= 1e-8 {
            continue;
        }
        let changed = tri.iter().any(|&i| moved[i as usize]);
        for (&i, point) in tri.iter().zip(p) {
            let i = i as usize;
            projected.indices.push(projected.positions.len() as u32 / 3);
            projected.positions.extend(point);
            if changed {
                projected.normals.extend(normal.map(|v| v / length));
            } else {
                projected
                    .normals
                    .extend_from_slice(&result.normals[i * 3..i * 3 + 3]);
            }
            projected
                .colors
                .extend_from_slice(&result.colors[i * 4..i * 4 + 4]);
        }
    }
    projected
}

#[derive(Default)]
pub struct Cache {
    tiles: BTreeMap<Key, Tile>,
    order: VecDeque<Key>,
}

impl Cache {
    pub fn invalidate(&mut self, bounds: Option<([i32; 3], [i32; 3])>) {
        let Some((lo, hi)) = bounds else {
            return;
        };
        self.tiles.retain(|_, tile| {
            let (a, b) = tile.dependencies();
            (0..3).any(|axis| hi[axis] < a[axis] || lo[axis] > b[axis])
        });
        self.order.retain(|key| self.tiles.contains_key(key));
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn poll(
        &mut self,
        key: Key,
        grid: &mut Grid,
        palette: &Palette,
        surface: Surface,
        voxel: f32,
    ) -> Result<&Tile, String> {
        self.invalidate(grid.take_lod_changes());
        if !self.tiles.contains_key(&key) {
            let tile = Tile::new(key, grid)?;
            if self.tiles.len() == MAX_TILES {
                self.tiles.remove(&self.order.pop_front().unwrap());
            }
            self.tiles.insert(key, tile);
            self.order.push_back(key);
        }
        if let Err(why) = self
            .tiles
            .get_mut(&key)
            .unwrap()
            .advance(grid, palette, surface, voxel)
        {
            self.tiles.remove(&key);
            self.order.retain(|other| *other != key);
            return Err(why);
        }
        Ok(&self.tiles[&key])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(
        cache: &mut Cache,
        key: Key,
        grid: &mut Grid,
        surface: Surface,
    ) -> BTreeMap<Option<u8>, Group> {
        let palette = Palette::new(&[], &[]).unwrap();
        for _ in 0..200 {
            let tile = cache.poll(key, grid, &palette, surface, 1.0).unwrap();
            assert!(tile.visits <= BASE_VISITS_PER_POLL);
            if let Some(groups) = &tile.groups {
                assert!(groups.values().map(words).sum::<usize>() <= MAX_OUTPUT_WORDS);
                return groups.clone();
            }
        }
        panic!("coarse tile never completed");
    }

    fn key(level: u8, x: i32) -> Key {
        Key {
            level,
            tile: [x, 0, 0],
        }
    }

    #[test]
    fn cube_proxies_scale_and_partial_world_edges_keep_caps() {
        for level in 1..=MAX_LEVEL {
            let mut grid = Grid::new([33, 1, 31]);
            grid.set_shaped([32, 0, 30], 7, crate::shape::Shape::Post);
            let scale = 1 << level;
            let key = Key {
                level,
                tile: [32 / scale / TILE, 0, 30 / scale / TILE],
            };
            let groups = finish(&mut Cache::default(), key, &mut grid, Surface::Cubes);
            let group = &groups[&Some(7)];
            assert_eq!(group.indices.len(), 36);
            let base = key.tile.map(|c| c * TILE * scale);
            for p in group.positions.chunks_exact(3) {
                assert!((0..3).all(|a| p[a] + base[a] as f32 <= grid.size()[a] as f32));
                assert!((0..3).all(|a| p[a] + base[a] as f32 >= 0.0));
            }
            let max_x = group
                .positions
                .chunks_exact(3)
                .map(|p| p[0])
                .fold(f32::MIN, f32::max);
            assert_eq!(max_x + base[0] as f32, 33.0);
        }
    }

    #[test]
    fn boundary_edits_invalidate_both_tiles_but_unrelated_jobs_survive() {
        let mut grid = Grid::new([128, 16, 16]);
        grid.stream("empty", 0);
        grid.set([15, 4, 4], 1);
        grid.set([16, 4, 4], 1);
        let mut cache = Cache::default();
        let a = finish(&mut cache, key(1, 0), &mut grid, Surface::Cubes);
        let b = finish(&mut cache, key(1, 1), &mut grid, Surface::Cubes);
        // Each proxy has five exterior faces; the shared face is suppressed.
        assert_eq!(a[&None].indices.len(), 30);
        assert_eq!(b[&None].indices.len(), 30);
        finish(&mut cache, key(1, 4), &mut grid, Surface::Cubes);
        grid.set([16, 4, 4], 0);
        cache.invalidate(grid.take_lod_changes());
        assert_eq!(cache.len(), 1);
        assert!(cache.tiles.contains_key(&key(1, 4)));
        let a = finish(&mut cache, key(1, 0), &mut grid, Surface::Cubes);
        assert_eq!(a[&None].indices.len(), 36);
        assert!(finish(&mut cache, key(1, 1), &mut grid, Surface::Cubes).is_empty());
        assert_eq!(grid.allocated_bytes(), 0);
    }

    #[test]
    fn delayed_job_restarts_on_edits_and_eviction_is_bounded() {
        let mut grid = Grid::new([768, 256, 256]);
        grid.stream("empty", 0);
        let palette = Palette::new(&[], &[]).unwrap();
        let mut cache = Cache::default();
        let tile = cache
            .poll(key(4, 0), &mut grid, &palette, Surface::Cubes, 1.0)
            .unwrap();
        assert!(tile.groups.is_none());
        let progress = tile.progress();
        let revision = tile.revision;
        grid.set([0, 0, 0], 1);
        let tile = cache
            .poll(key(4, 0), &mut grid, &palette, Surface::Cubes, 1.0)
            .unwrap();
        assert_eq!(tile.progress(), progress);
        assert!(tile.revision > revision);
        assert!(!finish(&mut cache, key(4, 0), &mut grid, Surface::Cubes).is_empty());
        for x in 0..5 {
            finish(&mut cache, key(1, x), &mut grid, Surface::Cubes);
        }
        assert_eq!(cache.len(), MAX_TILES);
        assert!(!cache.tiles.contains_key(&key(1, 0)));
        grid.stream("empty", 1);
        assert!(finish(&mut cache, key(1, 4), &mut grid, Surface::Cubes).is_empty());
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .poll(key(0, 0), &mut grid, &palette, Surface::Cubes, 1.0)
                .is_err()
        );
        assert!(
            cache
                .poll(key(4, i32::MAX), &mut grid, &palette, Surface::Cubes, 1.0)
                .is_err()
        );
    }

    #[test]
    fn smooth_tiles_share_positions_and_gradient_normals_at_equal_resolution() {
        let mut grid = Grid::new([64, 32, 32]);
        for z in 2..14 {
            for y in 2..14 {
                for x in 4..28 {
                    grid.set_density([x, y, z], -128, 1);
                }
            }
        }
        let mut cache = Cache::default();
        let a = finish(&mut cache, key(1, 0), &mut grid, Surface::Smooth);
        let b = finish(&mut cache, key(1, 1), &mut grid, Surface::Smooth);
        let seam = |group: &Group, offset: f32| -> BTreeMap<[i32; 3], [i32; 3]> {
            group
                .positions
                .chunks_exact(3)
                .zip(group.normals.chunks_exact(3))
                .filter(|(p, _)| (p[0] + offset - 17.0).abs() < 1e-5)
                .map(|(p, n)| {
                    (
                        [
                            ((p[0] + offset) * 10000.0).round() as i32,
                            (p[1] * 10000.0).round() as i32,
                            (p[2] * 10000.0).round() as i32,
                        ],
                        std::array::from_fn(|a| (n[a] * 10000.0).round() as i32),
                    )
                })
                .collect()
        };
        let left = seam(&a[&None], 0.0);
        assert!(!left.is_empty());
        assert_eq!(left, seam(&b[&None], 16.0));
    }
    #[test]
    fn projected_smooth_edges_remain_closed_inside_exact_world_bounds() {
        let mut grid = Grid::new([31; 3]);
        for z in 0..31 {
            for y in 0..31 {
                for x in 0..31 {
                    grid.set([x, y, z], 1);
                }
            }
        }
        let groups = finish(&mut Cache::default(), key(2, 0), &mut grid, Surface::Smooth);
        let group = &groups[&None];
        let mut edges = BTreeMap::<([i32; 3], [i32; 3]), usize>::new();
        for tri in group.indices.chunks_exact(3) {
            let p = tri
                .iter()
                .map(|&i| {
                    std::array::from_fn::<_, 3, _>(|a| {
                        let v = group.positions[i as usize * 3 + a];
                        assert!((0.0..=31.0).contains(&v));
                        (v * 10000.0).round() as i32
                    })
                })
                .collect::<Vec<_>>();
            for (a, b) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
                assert_ne!(a, b);
                *edges
                    .entry(if a < b { (a, b) } else { (b, a) })
                    .or_default() += 1;
            }
        }
        assert!(!edges.is_empty());
        assert!(
            edges.values().all(|n| *n == 2),
            "projected boundary is not closed"
        );
    }
}
