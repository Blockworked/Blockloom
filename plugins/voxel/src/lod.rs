//! Disposable mip samples. Coordinates are in the requested level's lattice.

use crate::grid::{Grid, SECTION};
use crate::shape::Shape;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX_LEVEL: u8 = 4;
pub const REDUCTION_VERSION: u32 = 1;
pub const MAX_SAMPLES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Address {
    pub level: u8,
    pub cell: [i32; 3],
}

impl Address {
    pub fn node(self) -> (u8, [i32; 3]) {
        (self.level, self.cell.map(|c| c.div_euclid(SECTION)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub material: u8,
    /// Shape occupancy proxy, used to rank representative materials.
    pub opacity: u8,
    /// Signed distance proxy in units of 1/256 of this level's sample width.
    pub density: i16,
    pub density_material: u8,
    /// Occupied octants, ordered x + 2*y + 4*z.
    pub children: u8,
}

impl Sample {
    pub const AIR: Self = Self {
        material: 0,
        opacity: 0,
        density: 256,
        density_material: 0,
        children: 0,
    };

    pub fn base(grid: &Grid, cell: [i32; 3]) -> Self {
        let material = grid.get(cell);
        let opacity = if material == 0 {
            0
        } else {
            match grid.shape_at(cell) {
                Shape::Cube => 255,
                Shape::Slab | Shape::TopSlab | Shape::Ramp(_) => 128,
                Shape::Post => 64,
                Shape::Stair(_) => 192,
            }
        };
        let density = grid.density(cell);
        Self {
            material,
            opacity,
            density,
            density_material: if density < 0 { material } else { 0 },
            children: 0,
        }
    }

    pub fn reduce(children: [Self; 8]) -> Self {
        let mut sample = Self::AIR;
        let mut sum = 0i32;
        let mut deepest = 0;
        let mut density_material = 0;
        for (i, child) in children.into_iter().enumerate() {
            if child.material != 0 {
                sample.children |= 1 << i;
                // Strict comparison keeps the first corner on equal opacity.
                if child.opacity > sample.opacity {
                    sample.material = child.material;
                    sample.opacity = child.opacity;
                }
            }
            sum += i32::from(child.density);
            if child.density < deepest {
                deepest = child.density;
                density_material = child.density_material;
            }
        }
        // Average eight distances, then express them at twice the sample width.
        sample.density = (sum / 16) as i16;
        if sample.density == 0 {
            sample.density = if sum < 0 { -1 } else { 1 };
        }
        if sample.density < 0 {
            sample.density_material = density_material;
        }
        sample
    }
}

#[derive(Default)]
pub struct Cache {
    samples: BTreeMap<Address, (Sample, u64)>,
    order: VecDeque<(Address, u64)>,
    serial: u64,
}

impl Cache {
    pub fn get(&self, address: Address) -> Option<Sample> {
        self.samples.get(&address).map(|(sample, _)| *sample)
    }

    pub fn insert(&mut self, address: Address, sample: Sample) {
        if self.samples.contains_key(&address) {
            return;
        }
        if self.order.len() == MAX_SAMPLES {
            let (old, serial) = self.order.pop_front().unwrap();
            if self
                .samples
                .get(&old)
                .is_some_and(|(_, stamp)| *stamp == serial)
            {
                self.samples.remove(&old);
            }
        }
        self.serial = self.serial.wrapping_add(1);
        self.samples.insert(address, (sample, self.serial));
        self.order.push_back((address, self.serial));
    }

    pub fn invalidate(&mut self, cell: [i32; 3]) {
        for level in 1..=MAX_LEVEL {
            self.samples.remove(&Address {
                level,
                cell: cell.map(|c| c.div_euclid(1 << level)),
            });
        }
    }

    pub fn samples(&self) -> usize {
        self.samples.len()
    }

    pub fn nodes(&self) -> usize {
        self.samples
            .keys()
            .map(|a| a.node())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(grid: &Grid, level: u8, cell: [i32; 3]) -> Sample {
        if level == 0 {
            return Sample::base(grid, cell);
        }
        Sample::reduce(std::array::from_fn(|i| {
            reference(
                grid,
                level - 1,
                [
                    cell[0] * 2 + (i & 1) as i32,
                    cell[1] * 2 + ((i >> 1) & 1) as i32,
                    cell[2] * 2 + ((i >> 2) & 1) as i32,
                ],
            )
        }))
    }

    #[test]
    fn thin_shapes_survive_and_opaque_material_wins_with_stable_ties() {
        let mut grid = Grid::new([32; 3]);
        grid.set_shaped([0, 0, 0], 5, Shape::Post);
        for level in 1..=MAX_LEVEL {
            let sample = grid.lod_sample(level, [0; 3]).unwrap();
            assert_eq!(
                (sample.material, sample.opacity, sample.children),
                (5, 64, 1)
            );
            assert!(
                sample.density > 0,
                "shape proxies stay outside the smooth field"
            );
        }
        grid.set([1, 0, 0], 2);
        grid.set([0, 1, 0], 3);
        let sample = grid.lod_sample(1, [0; 3]).unwrap();
        assert_eq!(
            (sample.material, sample.opacity, sample.children),
            (2, 255, 7)
        );
        grid.set([1, 0, 0], 0);
        assert_eq!(grid.lod_sample(4, [0; 3]).unwrap().material, 3);
    }

    #[test]
    fn smooth_distances_scale_and_zero_ties_stay_outside() {
        let mut grid = Grid::new([2; 3]);
        for i in 0..8 {
            grid.set_density([i & 1, (i >> 1) & 1, (i >> 2) & 1], -128, 7);
        }
        let sample = grid.lod_sample(1, [0; 3]).unwrap();
        assert_eq!((sample.density, sample.density_material), (-64, 7));
        for i in 0..4 {
            grid.set_density([i & 1, (i >> 1) & 1, 0], 128, 0);
        }
        let sample = grid.lod_sample(1, [0; 3]).unwrap();
        assert_eq!((sample.density, sample.density_material), (1, 0));
        assert_eq!(sample.children, 240);
    }

    #[test]
    fn canonical_generation_edits_and_partial_bounds_match_uncached_reduction() {
        for height in [1, 31, 33, 100] {
            let mut grid = Grid::new([65, height, 33]);
            grid.stream("caves", 42);
            let cell = [32, height - 1, 32];
            for level in 1..=MAX_LEVEL {
                let at = cell.map(|c| c >> level);
                assert_eq!(
                    grid.lod_sample(level, at).unwrap(),
                    reference(&grid, level, at)
                );
            }
            let revision = grid.revision();
            grid.set_shaped(cell, 7, Shape::Slab);
            assert!(grid.revision() > revision);
            for level in 1..=MAX_LEVEL {
                let at = cell.map(|c| c >> level);
                assert_eq!(
                    grid.lod_sample(level, at).unwrap(),
                    reference(&grid, level, at)
                );
            }
            grid.set_density(cell, -17, 2);
            for level in 1..=MAX_LEVEL {
                let at = cell.map(|c| c >> level);
                assert_eq!(
                    grid.lod_sample(level, at).unwrap(),
                    reference(&grid, level, at)
                );
            }
            grid.set(cell, 0);
            let expected = reference(&grid, 4, cell.map(|c| c >> 4));
            assert_eq!(grid.lod_sample(4, cell.map(|c| c >> 4)).unwrap(), expected);
            assert_eq!(grid.resident_pages(), 0);
            assert_eq!(grid.allocated_bytes(), 0);
            let mut restored = Grid::restore(grid.snapshot(), 7).unwrap();
            assert_eq!(restored.lod_nodes(), 0);
            assert_eq!(
                restored.lod_sample(4, cell.map(|c| c >> 4)).unwrap(),
                expected
            );
            grid.stream("empty", 1);
            assert_eq!(grid.lod_nodes(), 0);
            assert_eq!(grid.lod_sample(4, [0; 3]).unwrap().material, 0);
        }
    }

    #[test]
    fn eviction_and_invalidations_keep_cache_and_queue_bounded() {
        let mut cache = Cache::default();
        for i in 0..MAX_SAMPLES * 3 {
            let address = Address {
                level: 1,
                cell: [i as i32, 0, 0],
            };
            cache.insert(address, Sample::AIR);
            cache.invalidate(address.cell.map(|c| c * 2));
            cache.insert(address, Sample::AIR);
            assert!(cache.samples() <= MAX_SAMPLES);
            assert!(cache.order.len() <= MAX_SAMPLES);
        }
        assert_eq!(
            cache.get(Address {
                level: 1,
                cell: [0; 3]
            }),
            None
        );
    }

    #[test]
    fn ancestor_invalidation_preserves_unrelated_samples_and_authoritative_sections() {
        let mut grid = Grid::new([128, 32, 32]);
        grid.set([63, 0, 0], 1);
        grid.set([64, 0, 0], 2);
        for level in 1..=MAX_LEVEL {
            for x in [63, 64] {
                grid.lod_sample(level, [x >> level, 0, 0]).unwrap();
            }
        }
        let samples = grid.lod_samples();
        let revision = grid.revision();
        assert!(!grid.set([64, 0, 0], 2));
        assert_eq!(grid.revision(), revision);
        grid.set([63, 0, 0], 0);
        assert_eq!(grid.lod_samples(), samples - 4);
        grid.retain_pages(&BTreeSet::new());
        assert_eq!(grid.get([64, 0, 0]), 2);
        for level in 1..=MAX_LEVEL {
            assert_eq!(
                grid.lod_sample(level, [63 >> level, 0, 0])
                    .unwrap()
                    .material,
                0
            );
            assert_eq!(
                grid.lod_sample(level, [64 >> level, 0, 0])
                    .unwrap()
                    .material,
                2
            );
        }
        assert!(grid.lod_sample(5, [0; 3]).is_err());
        assert_eq!(grid.lod_sample(4, [i32::MAX; 3]).unwrap().material, 0);
    }

    #[test]
    fn one_cell_high_generators_match_canonical_samples() {
        for preset in crate::terrain::PRESETS {
            let mut grid = Grid::new([9, 1, 9]);
            crate::terrain::generate(&mut grid, preset, 42).unwrap();
            for z in 0..9 {
                for x in 0..9 {
                    assert_eq!(
                        grid.get([x, 0, z]),
                        crate::terrain::sample([9, 1, 9], preset, 42, [x, 0, z])
                    );
                }
            }
        }
    }
}
