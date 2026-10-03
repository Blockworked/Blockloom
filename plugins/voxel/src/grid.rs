//! Authoritative cells in sparse 32-cell sections, grouped into full-height columns.
//!
//! A section that holds only air is not allocated. Cell coordinates are signed
//! and start at zero in one corner; anything outside the box reads as air and
//! ignores writes.

pub use crate::shape::Shape;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SECTION: i32 = 32;
const LEGACY_PAGE: i32 = 16;
pub type ColumnAddress = [i32; 2];
pub type SectionAddress = [i32; 3];
pub const SECTION_CELLS: usize = (SECTION * SECTION * SECTION) as usize;

pub type Cells = [u8; SECTION_CELLS];

pub struct Grid {
    /// Exact logical bounds in cells; section padding is air.
    size: [i32; 3],
    sections: BTreeMap<SectionAddress, Box<Cells>>,
    resident: BTreeSet<SectionAddress>,
    generator: Option<(String, i64)>,
    generator_size: Option<[i32; 3]>,
    edits: BTreeMap<[i32; 3], u8>,
    /// The cells that are not whole cubes.
    shapes: BTreeMap<[i32; 3], Shape>,
    /// Signed density at cell centres, quantized to 1/256 cell.
    densities: BTreeMap<[i32; 3], i16>,
    /// Sections whose meshes are out of date: those written to, and the
    /// neighbours that see one of their cells across a boundary.
    dirty: BTreeSet<[i32; 3]>,
    lod: crate::lod::Cache,
    revision: u64,
    lod_changes: Option<([i32; 3], [i32; 3])>,
}

fn decoded_cell(section: SectionAddress, index: usize, width: i32) -> [i32; 3] {
    let n = width as usize;
    [
        section[0] * width + (index % n) as i32,
        section[1] * width + ((index / n) % n) as i32,
        section[2] * width + (index / (n * n)) as i32,
    ]
}

fn local(cell: [i32; 3]) -> usize {
    let [x, y, z] = cell.map(|c| c.rem_euclid(SECTION) as usize);
    (z * SECTION as usize + y) * SECTION as usize + x
}

impl Grid {
    pub fn new(size: [i32; 3]) -> Grid {
        let size = size.map(|s| s.max(1));
        Grid {
            size,
            sections: BTreeMap::new(),
            resident: BTreeSet::new(),
            generator: None,
            generator_size: None,
            edits: BTreeMap::new(),
            shapes: BTreeMap::new(),
            densities: BTreeMap::new(),
            dirty: BTreeSet::new(),
            lod: crate::lod::Cache::default(),
            revision: 0,
            lod_changes: None,
        }
    }

    pub fn size(&self) -> [i32; 3] {
        self.size
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn take_lod_changes(&mut self) -> Option<([i32; 3], [i32; 3])> {
        self.lod_changes.take()
    }

    pub fn lod_nodes(&self) -> usize {
        self.lod.nodes()
    }

    pub fn lod_samples(&self) -> usize {
        self.lod.samples()
    }

    /// Build only requested samples; full-detail section residency is independent.
    pub fn lod_sample(&mut self, level: u8, cell: [i32; 3]) -> Result<crate::lod::Sample, String> {
        if level > crate::lod::MAX_LEVEL {
            return Err("voxel LOD level must be between 0 and 4".into());
        }
        let scale = 1 << level;
        if (0..3).any(|a| cell[a] < 0 || cell[a] >= (self.size[a] + scale - 1) / scale) {
            return Ok(crate::lod::Sample {
                density: 256 >> level,
                ..crate::lod::Sample::AIR
            });
        }
        if level == 0 {
            return Ok(crate::lod::Sample::base(self, cell));
        }
        let address = crate::lod::Address { level, cell };
        if let Some(sample) = self.lod.get(address) {
            return Ok(sample);
        }
        let mut children = [crate::lod::Sample::AIR; 8];
        for (i, child) in children.iter_mut().enumerate() {
            let at = [
                cell[0] * 2 + (i & 1) as i32,
                cell[1] * 2 + ((i >> 1) & 1) as i32,
                cell[2] * 2 + ((i >> 2) & 1) as i32,
            ];
            *child = self.lod_sample(level - 1, at)?;
        }
        let sample = crate::lod::Sample::reduce(children);
        self.lod.insert(address, sample);
        Ok(sample)
    }

    /// How many sections intersect the logical bounds on each axis.
    pub fn section_counts(&self) -> [i32; 3] {
        self.size.map(|s| (s + SECTION - 1) / SECTION)
    }

    pub fn column_counts(&self) -> ColumnAddress {
        let n = self.section_counts();
        [n[0], n[2]]
    }

    pub fn resident_columns(&self) -> usize {
        self.resident
            .iter()
            .chain(self.sections.keys())
            .map(|c| [c[0], c[2]])
            .collect::<BTreeSet<ColumnAddress>>()
            .len()
    }

    pub fn allocated_bytes(&self) -> usize {
        self.sections.len() * SECTION_CELLS
    }

    pub fn contains(&self, cell: [i32; 3]) -> bool {
        (0..3).all(|a| (0..self.size[a]).contains(&cell[a]))
    }

    fn slot(&self, chunk: [i32; 3]) -> Option<()> {
        let n = self.section_counts();
        (0..3).all(|a| (0..n[a]).contains(&chunk[a])).then_some(())
    }

    pub fn section(&self, chunk: [i32; 3]) -> Option<&Cells> {
        self.sections.get(&chunk).map(Box::as_ref)
    }

    pub fn get(&self, cell: [i32; 3]) -> u8 {
        if !self.contains(cell) {
            return 0;
        }
        self.edits.get(&cell).copied().unwrap_or_else(|| {
            self.section(cell.map(|c| c.div_euclid(SECTION)))
                .map_or_else(|| self.base(cell), |cells| cells[local(cell)])
        })
    }

    pub fn shape_at(&self, cell: [i32; 3]) -> Shape {
        self.shapes.get(&cell).copied().unwrap_or_default()
    }

    /// A solid cell that fills its whole cell, so it hides what it touches.
    pub fn is_full(&self, cell: [i32; 3]) -> bool {
        self.get(cell) != 0 && !self.shapes.contains_key(&cell)
    }

    /// The shaped cells inside one section.
    pub fn shaped_in(&self, chunk: [i32; 3]) -> Vec<([i32; 3], Shape)> {
        let lo = chunk.map(|c| c * SECTION);
        let hi = lo.map(|c| c + SECTION);
        self.shapes
            .range(lo..hi)
            .filter(|(cell, _)| (0..3).all(|a| (lo[a]..hi[a]).contains(&cell[a])))
            .map(|(&cell, &shape)| (cell, shape))
            .collect()
    }

    /// Negative is solid. Shaped cells are drawn separately from the field.
    pub fn density(&self, cell: [i32; 3]) -> i16 {
        if !self.contains(cell) || self.shape_at(cell) != Shape::Cube {
            return 256;
        }
        self.densities
            .get(&cell)
            .copied()
            .unwrap_or_else(|| if self.get(cell) == 0 { 256 } else { -256 })
    }

    pub fn set_density(&mut self, cell: [i32; 3], density: i16, material: u8) -> bool {
        if !self.contains(cell) {
            return false;
        }
        // Keep every intersection away from lattice vertices.
        let density = if density == 0 {
            1
        } else {
            density.clamp(-256, 256)
        };
        let material = if density < 0 { material } else { 0 };
        let old = self.density(cell);
        if old == density && self.get(cell) == material && self.shape_at(cell) == Shape::Cube {
            return false;
        }
        let changed = self.set_shaped(cell, material, Shape::Cube);
        if density.abs() == 256 {
            self.densities.remove(&cell);
        } else {
            self.densities.insert(cell, density);
        }
        if old != density {
            self.dirty_sample(cell);
        }
        changed || old != density
    }

    /// Include the density-gradient halo and diagonal neighbours.
    fn dirty_sample(&mut self, cell: [i32; 3]) {
        self.revision = self.revision.wrapping_add(1);
        self.lod.invalidate(cell);
        let (lo, hi) = self.lod_changes.get_or_insert((cell, cell));
        for a in 0..3 {
            lo[a] = lo[a].min(cell[a]);
            hi[a] = hi[a].max(cell[a]);
        }
        let lo = cell.map(|v| (v - 2).max(0).div_euclid(SECTION));
        let hi = cell.map(|v| (v + 1).div_euclid(SECTION));
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let chunk = [x, y, z];
                    if self.slot(chunk).is_some() {
                        self.dirty.insert(chunk);
                    }
                }
            }
        }
    }

    /// Writes a whole cube; true when that changed the cell.
    pub fn set(&mut self, cell: [i32; 3], material: u8) -> bool {
        self.set_shaped(cell, material, Shape::Cube)
    }

    /// Changes the shape of a solid cell; true when that changed it.
    pub fn reshape(&mut self, cell: [i32; 3], shape: Shape) -> bool {
        match self.get(cell) {
            0 => false,
            material => self.set_shaped(cell, material, shape),
        }
    }

    /// Writes a cell and its shape; true when that changed it.
    pub fn set_shaped(&mut self, cell: [i32; 3], material: u8, shape: Shape) -> bool {
        if !self.contains(cell) {
            return false;
        }
        let shape = if material == 0 { Shape::Cube } else { shape };
        let chunk = cell.map(|c| c.div_euclid(SECTION));
        let before = self.get(cell);
        let density_changed = self.densities.remove(&cell).is_some();
        if density_changed {
            self.dirty_sample(cell);
        }
        if material == 0 && before == 0 && !self.shapes.contains_key(&cell) {
            return density_changed;
        }
        if before == material && self.shapes.get(&cell).copied().unwrap_or_default() == shape {
            return density_changed;
        }
        if self.generator.is_some() {
            if self.base(cell) == material {
                self.edits.remove(&cell);
            } else {
                self.edits.insert(cell, material);
            }
        } else {
            self.sections
                .entry(chunk)
                .or_insert_with(|| Box::new([0; SECTION_CELLS]))[local(cell)] = material;
        }
        if material != 0 && self.resident.contains(&chunk) && !self.sections.contains_key(&chunk) {
            self.resident.remove(&chunk);
            self.load_page(chunk);
        }
        if let Some(cells) = self.sections.get_mut(&chunk) {
            cells[local(cell)] = material;
        }
        if shape == Shape::Cube {
            self.shapes.remove(&cell);
        } else {
            self.shapes.insert(cell, shape);
        }
        self.dirty_sample(cell);
        true
    }

    /// Frees a section that has gone back to air.
    pub fn prune(&mut self, chunk: [i32; 3]) {
        if self
            .sections
            .get(&chunk)
            .is_some_and(|cells| cells.iter().all(|&m| m == 0))
        {
            self.sections.remove(&chunk);
        }
    }

    fn base(&self, cell: [i32; 3]) -> u8 {
        self.generator.as_ref().map_or(0, |(preset, seed)| {
            crate::terrain::sample(
                self.generator_size.unwrap_or(self.size),
                preset,
                *seed,
                cell,
            )
        })
    }

    pub fn stream(&mut self, preset: &str, seed: i64) {
        self.clear();
        self.generator = Some((preset.to_string(), seed));
    }

    pub fn resident_pages(&self) -> usize {
        self.resident
            .union(&self.sections.keys().copied().collect())
            .count()
    }

    pub fn load_page(&mut self, chunk: [i32; 3]) {
        if self.slot(chunk).is_none() || self.resident.contains(&chunk) {
            return;
        }
        let mut cells = Box::new([0; SECTION_CELLS]);
        for z in 0..SECTION {
            for y in 0..SECTION {
                for x in 0..SECTION {
                    let cell = [
                        chunk[0] * SECTION + x,
                        chunk[1] * SECTION + y,
                        chunk[2] * SECTION + z,
                    ];
                    cells[local(cell)] = self.get(cell);
                }
            }
        }
        self.resident.insert(chunk);
        if cells.iter().any(|m| *m != 0) {
            self.sections.insert(chunk, cells);
        }
    }

    pub fn retain_pages(&mut self, pages: &BTreeSet<[i32; 3]>) {
        // Nonprocedural sections are authoritative, so residency cannot evict them.
        if self.generator.is_none() {
            return;
        }
        self.sections.retain(|c, _| pages.contains(c));
        self.resident.retain(|c| pages.contains(c));
    }

    pub fn solid_cells(&self) -> Vec<([i32; 3], u8)> {
        let mut out = Vec::new();
        for (&chunk, data) in &self.sections {
            for (i, &material) in data.iter().enumerate() {
                if material != 0 {
                    out.push((decoded_cell(chunk, i, SECTION), material));
                }
            }
        }
        out
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut pages = BTreeMap::<[i32; 3], Box<[u16; SECTION_CELLS]>>::new();
        if self.generator.is_some() {
            for (&cell, &material) in &self.edits {
                pages
                    .entry(cell.map(|c| c.div_euclid(SECTION)))
                    .or_insert_with(|| Box::new([256; SECTION_CELLS]))[local(cell)] =
                    u16::from(material);
            }
        } else {
            for (&chunk, data) in &self.sections {
                pages.insert(chunk, Box::new(data.map(u16::from)));
            }
        }
        Snapshot {
            page_size: SECTION,
            size: self.size,
            generator: self.generator.clone(),
            generator_size: self.generator_size,
            cells: Vec::new(),
            pages: pages
                .into_iter()
                .map(|(chunk, data)| {
                    let mut runs = Vec::<(u16, u16)>::new();
                    for &m in data.iter() {
                        if let Some((count, value)) = runs.last_mut()
                            && *value == m
                        {
                            *count += 1;
                        } else {
                            runs.push((1, m));
                        }
                    }
                    Page { chunk, runs }
                })
                .collect(),
            shapes: self.shapes.iter().map(|(&c, s)| (c, s.name())).collect(),
            densities: self.densities.iter().map(|(&c, &d)| (c, d)).collect(),
        }
    }

    pub fn restore(snapshot: Snapshot, palette_len: u8) -> Result<Self, String> {
        let width = snapshot.page_size;
        if ![LEGACY_PAGE, SECTION].contains(&width) {
            return Err("unsupported voxel checkpoint page size".into());
        }
        let page_cells = (width * width * width) as usize;
        if snapshot.size.iter().any(|s| *s <= 0 || *s > 1048576)
            || snapshot.cells.len() > 4194304
            || snapshot.pages.len() > 262144
            || (!snapshot.pages.is_empty() && !snapshot.cells.is_empty())
            || snapshot.densities.len() > 4194304
        {
            return Err("invalid voxel checkpoint bounds".into());
        }
        let mut grid = Grid::new(snapshot.size);
        if let Some((preset, seed)) = snapshot.generator {
            if !crate::terrain::PRESETS.contains(&preset.as_str()) {
                return Err("unknown checkpoint generator".into());
            }
            grid.stream(&preset, seed);
        }
        if let Some(size) = snapshot.generator_size {
            if grid.generator.is_none() || size.iter().any(|s| *s <= 0 || *s > 1048576) {
                return Err("invalid checkpoint generator bounds".into());
            }
            grid.generator_size = Some(size);
        }
        let mut written = 0usize;
        let mut seen = BTreeSet::new();
        for page in snapshot.pages {
            if !(0..3)
                .all(|a| page.chunk[a] >= 0 && page.chunk[a] < (grid.size[a] + width - 1) / width)
                || !seen.insert(page.chunk)
                || page.runs.len() > page_cells
            {
                return Err("invalid checkpoint page".into());
            }
            let mut offset = 0usize;
            for (count, material) in page.runs {
                let count = count as usize;
                if count == 0
                    || offset + count > page_cells
                    || (material != 256 && material > u16::from(palette_len))
                    || (material == 256 && grid.generator.is_none())
                {
                    return Err("invalid checkpoint page run".into());
                }
                if material != 256 {
                    for i in offset..offset + count {
                        let cell = decoded_cell(page.chunk, i, width);
                        if !grid.contains(cell) {
                            if material != 0 {
                                return Err("non-air checkpoint padding".into());
                            }
                            continue;
                        }
                        written += 1;
                        if written > 4194304 {
                            return Err("voxel checkpoint edit budget exceeded".into());
                        }
                        grid.set(cell, material as u8);
                    }
                }
                offset += count;
            }
            if offset != page_cells {
                return Err("incomplete checkpoint page".into());
            }
        }
        for (cell, m) in snapshot.cells {
            if !grid.contains(cell) || m > palette_len {
                return Err("invalid checkpoint cell".into());
            }
            grid.set(cell, m);
        }
        for (cell, shape) in snapshot.shapes {
            let shape = Shape::from_name(&shape)?;
            if !grid.contains(cell) || grid.get(cell) == 0 {
                return Err("invalid checkpoint shape".into());
            }
            grid.reshape(cell, shape);
        }
        for (cell, density) in snapshot.densities {
            if !grid.contains(cell)
                || density == 0
                || density.unsigned_abs() > 256
                || grid.shape_at(cell) != Shape::Cube
            {
                return Err("invalid checkpoint density".into());
            }
            if (density < 0) != (grid.get(cell) != 0) {
                return Err("checkpoint density/material disagree".into());
            }
            grid.set_density(cell, density, grid.get(cell));
        }
        if grid.generator.is_none() {
            grid.mark_all_dirty();
        }
        Ok(grid)
    }

    pub fn clip_legacy(&mut self, size: [i32; 3]) {
        self.lod = crate::lod::Cache::default();
        self.revision = self.revision.wrapping_add(1);
        self.lod_changes = Some(([0; 3], self.size.map(|s| s - 1)));
        if self.generator.is_some() {
            self.generator_size = Some(self.size);
        }
        self.size = size;
        self.edits
            .retain(|cell, _| (0..3).all(|a| cell[a] < size[a]));
        self.shapes
            .retain(|cell, _| (0..3).all(|a| cell[a] < size[a]));
        self.densities
            .retain(|cell, _| (0..3).all(|a| cell[a] < size[a]));
        self.sections.retain(|section, data| {
            for (i, m) in data.iter_mut().enumerate() {
                let cell = decoded_cell(*section, i, SECTION);
                if (0..3).any(|a| cell[a] >= size[a]) {
                    *m = 0;
                }
            }
            data.iter().any(|m| *m != 0)
        });
        self.dirty.clear();
        if self.generator.is_none() {
            self.mark_all_dirty();
        }
    }

    pub fn take_dirty(&mut self) -> Vec<[i32; 3]> {
        std::mem::take(&mut self.dirty).into_iter().collect()
    }

    pub fn mark_all_dirty(&mut self) {
        let n = self.section_counts();
        for z in 0..n[2] {
            for y in 0..n[1] {
                for x in 0..n[0] {
                    self.dirty.insert([x, y, z]);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.lod = crate::lod::Cache::default();
        self.revision = self.revision.wrapping_add(1);
        self.lod_changes = Some(([0; 3], self.size.map(|s| s - 1)));
        self.sections.clear();
        self.resident.clear();
        self.edits.clear();
        self.generator = None;
        self.generator_size = None;
        self.shapes.clear();
        self.densities.clear();
        self.dirty.clear();
    }

    pub fn solid_count(&self) -> u64 {
        self.sections
            .values()
            .map(|cells| cells.iter().filter(|&&m| m != 0).count() as u64)
            .sum()
    }

    /// The highest solid cell in a column, if there is one.
    pub fn height_at(&self, x: i32, z: i32) -> Option<i32> {
        if x < 0 || z < 0 || x >= self.size[0] || z >= self.size[2] {
            return None;
        }
        let top = if let Some((preset, seed)) = &self.generator {
            let base = crate::terrain::height_bound(
                self.generator_size.unwrap_or(self.size),
                preset,
                *seed,
                x,
                z,
            );
            self.edits
                .iter()
                .filter(|(c, m)| c[0] == x && c[2] == z && **m != 0)
                .map(|(c, _)| c[1])
                .max()
                .unwrap_or(-1)
                .max(base)
        } else {
            self.size[1] - 1
        };
        (0..=top).rev().find(|&y| self.get([x, y, z]) != 0)
    }
}

fn legacy_page_size() -> i32 {
    LEGACY_PAGE
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Snapshot {
    #[serde(default = "legacy_page_size")]
    pub page_size: i32,
    pub size: [i32; 3],
    pub generator: Option<(String, i64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_size: Option<[i32; 3]>,
    #[serde(default)]
    pub cells: Vec<([i32; 3], u8)>,
    #[serde(default)]
    pub pages: Vec<Page>,
    pub shapes: Vec<([i32; 3], String)>,
    pub densities: Vec<([i32; 3], i16)>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Page {
    pub chunk: [i32; 3],
    pub runs: Vec<(u16, u16)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_sections_keep_padding_air_and_round_trip() {
        for height in [1, 31, 33, 100] {
            let mut grid = Grid::new([33, height, 31]);
            assert_eq!(grid.column_counts(), [2, 1]);
            assert_eq!(grid.section_counts(), [2, (height + 31) / 32, 1]);
            assert!(grid.set([32, height - 1, 30], 1));
            assert!(!grid.set([32, height, 30], 1));
            assert!(!grid.set([33, height - 1, 30], 1));
            assert_eq!(grid.solid_cells(), [([32, height - 1, 30], 1)]);
            let restored = Grid::restore(grid.snapshot(), 1).unwrap();
            assert_eq!(restored.size(), [33, height, 31]);
            assert_eq!(restored.solid_cells(), grid.solid_cells());
        }
    }

    #[test]
    fn legacy_procedural_bounds_survive_clipping_and_resaving() {
        let mut grid = Grid::new([48, 112, 48]);
        grid.stream("island", 42);
        let before = grid.get([20, 30, 20]);
        grid.clip_legacy([33, 100, 33]);
        assert_eq!(grid.get([20, 30, 20]), before);
        let restored = Grid::restore(grid.snapshot(), 7).unwrap();
        assert_eq!(restored.get([20, 30, 20]), before);
        assert_eq!(restored.get([33, 30, 20]), 0);
    }

    #[test]
    fn checkpoint_padding_and_unknown_page_sizes_are_rejected() {
        let mut grid = Grid::new([1; 3]);
        grid.set([0; 3], 1);
        let mut snapshot = grid.snapshot();
        snapshot.pages[0].runs = vec![(2, 1), (32766, 0)];
        assert!(Grid::restore(snapshot, 1).is_err());
        let mut snapshot = grid.snapshot();
        snapshot.page_size = i32::MAX;
        assert!(Grid::restore(snapshot, 1).is_err());
    }

    #[test]
    fn sizes_keep_exact_bounds() {
        let grid = Grid::new([17, 1, 32]);
        assert_eq!(grid.size(), [17, 1, 32]);
        assert_eq!(grid.section_counts(), [1, 1, 1]);
    }

    #[test]
    fn cells_read_back_and_outside_is_air() {
        let mut grid = Grid::new([32, 16, 16]);
        assert!(grid.set([20, 3, 4], 5));
        assert!(!grid.set([20, 3, 4], 5));
        assert_eq!(grid.get([20, 3, 4]), 5);
        assert_eq!(grid.get([-1, 0, 0]), 0);
        assert!(!grid.set([32, 0, 0], 1));
        assert_eq!(grid.solid_count(), 1);
        assert_eq!(grid.height_at(20, 4), Some(3));
        assert_eq!(grid.height_at(0, 0), None);
    }

    #[test]
    fn a_boundary_cell_dirties_the_neighbour_too() {
        let mut grid = Grid::new([64, 16, 16]);
        grid.set([31, 5, 5], 1);
        assert_eq!(grid.take_dirty(), [[0, 0, 0], [1, 0, 0]]);
        grid.set([5, 5, 5], 1);
        assert_eq!(grid.take_dirty(), [[0, 0, 0]]);
        assert!(grid.take_dirty().is_empty());
    }

    #[test]
    fn shapes_are_kept_per_cell_and_dropped_with_the_cell() {
        let mut grid = Grid::new([64, 16, 16]);
        assert!(!grid.reshape([1, 1, 1], Shape::Slab), "air has no shape");
        grid.set([1, 1, 1], 1);
        assert!(grid.is_full([1, 1, 1]));
        assert!(grid.reshape([1, 1, 1], Shape::Slab));
        assert!(!grid.reshape([1, 1, 1], Shape::Slab));
        assert_eq!(grid.shape_at([1, 1, 1]), Shape::Slab);
        assert!(!grid.is_full([1, 1, 1]));
        assert_eq!(grid.get([1, 1, 1]), 1);
        grid.set_shaped([33, 2, 2], 3, Shape::Post);
        assert_eq!(grid.shaped_in([0, 0, 0]).len(), 1);
        assert_eq!(grid.shaped_in([1, 0, 0]), [([33, 2, 2], Shape::Post)]);
        // Writing a cube, or air, takes the shape away.
        grid.set([1, 1, 1], 1);
        assert_eq!(grid.shape_at([1, 1, 1]), Shape::Cube);
        grid.set([33, 2, 2], 0);
        assert!(grid.shaped_in([1, 0, 0]).is_empty());
        assert!(Shape::from_name("Top slab").is_ok());
        assert!(Shape::from_name("dome").is_err());
        assert_eq!(Shape::from_name(&Shape::TopSlab.name()), Ok(Shape::TopSlab));
    }

    #[test]
    fn an_emptied_chunk_can_be_freed() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([1, 1, 1], 1);
        grid.set([1, 1, 1], 0);
        assert!(grid.section([0, 0, 0]).is_some());
        grid.prune([0, 0, 0]);
        assert!(grid.section([0, 0, 0]).is_none());
    }
}
