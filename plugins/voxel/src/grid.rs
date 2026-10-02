//! The authoritative cells: a finite box of chunks, each 16 cells a side.
//!
//! A chunk that holds only air is not allocated. Cell coordinates are signed
//! and start at zero in one corner; anything outside the box reads as air and
//! ignores writes.

use std::collections::{BTreeMap, BTreeSet};

pub const CHUNK: i32 = 16;
const CELLS: usize = (CHUNK * CHUNK * CHUNK) as usize;

pub type Cells = [u8; CELLS];

/// What part of its cell a solid cell fills. Anything but a cube is kept in
/// a sparse table, so a world of cubes pays nothing for the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Cube,
    /// The lower half.
    Slab,
    /// The upper half.
    TopSlab,
    /// A half-width column through the middle.
    Post,
}

impl Shape {
    pub fn from_name(name: &str) -> Result<Shape, String> {
        match name
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "cube" | "full" => Ok(Shape::Cube),
            "slab" => Ok(Shape::Slab),
            "top_slab" => Ok(Shape::TopSlab),
            "post" => Ok(Shape::Post),
            other => Err(format!(
                "no shape called {other} (cube, slab, top slab or post)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shape::Cube => "cube",
            Shape::Slab => "slab",
            Shape::TopSlab => "top slab",
            Shape::Post => "post",
        }
    }

    /// The box it fills, as fractions of the cell: low corner, high corner.
    pub fn bounds(self) -> ([f32; 3], [f32; 3]) {
        match self {
            Shape::Cube => ([0.0; 3], [1.0; 3]),
            Shape::Slab => ([0.0; 3], [1.0, 0.5, 1.0]),
            Shape::TopSlab => ([0.0, 0.5, 0.0], [1.0; 3]),
            Shape::Post => ([0.25, 0.0, 0.25], [0.75, 1.0, 0.75]),
        }
    }
}

pub struct Grid {
    /// Size in cells, a whole number of chunks on each axis.
    size: [i32; 3],
    chunks: Vec<Option<Box<Cells>>>,
    /// The cells that are not whole cubes.
    shapes: BTreeMap<[i32; 3], Shape>,
    /// Chunks whose meshes are out of date: those written to, and the
    /// neighbours that see one of their cells across a boundary.
    dirty: BTreeSet<[i32; 3]>,
}

fn rounded(cells: i32) -> i32 {
    (cells.max(1) + CHUNK - 1) / CHUNK * CHUNK
}

fn local(cell: [i32; 3]) -> usize {
    let [x, y, z] = cell.map(|c| c.rem_euclid(CHUNK) as usize);
    (z * CHUNK as usize + y) * CHUNK as usize + x
}

impl Grid {
    pub fn new(size: [i32; 3]) -> Grid {
        let size = size.map(rounded);
        let count = size.map(|s| (s / CHUNK) as usize);
        Grid {
            size,
            chunks: vec![None; count[0] * count[1] * count[2]],
            shapes: BTreeMap::new(),
            dirty: BTreeSet::new(),
        }
    }

    pub fn size(&self) -> [i32; 3] {
        self.size
    }

    /// How many chunks there are along each axis.
    pub fn chunk_counts(&self) -> [i32; 3] {
        self.size.map(|s| s / CHUNK)
    }

    pub fn contains(&self, cell: [i32; 3]) -> bool {
        (0..3).all(|a| (0..self.size[a]).contains(&cell[a]))
    }

    fn slot(&self, chunk: [i32; 3]) -> Option<usize> {
        let n = self.chunk_counts();
        (0..3)
            .all(|a| (0..n[a]).contains(&chunk[a]))
            .then(|| ((chunk[2] * n[1] + chunk[1]) * n[0] + chunk[0]) as usize)
    }

    pub fn chunk(&self, chunk: [i32; 3]) -> Option<&Cells> {
        self.chunks[self.slot(chunk)?].as_deref()
    }

    pub fn get(&self, cell: [i32; 3]) -> u8 {
        if !self.contains(cell) {
            return 0;
        }
        self.chunk(cell.map(|c| c.div_euclid(CHUNK)))
            .map_or(0, |cells| cells[local(cell)])
    }

    pub fn shape_at(&self, cell: [i32; 3]) -> Shape {
        self.shapes.get(&cell).copied().unwrap_or_default()
    }

    /// A solid cell that fills its whole cell, so it hides what it touches.
    pub fn is_full(&self, cell: [i32; 3]) -> bool {
        self.get(cell) != 0 && !self.shapes.contains_key(&cell)
    }

    /// The shaped cells inside one chunk.
    pub fn shaped_in(&self, chunk: [i32; 3]) -> Vec<([i32; 3], Shape)> {
        let lo = chunk.map(|c| c * CHUNK);
        let hi = lo.map(|c| c + CHUNK);
        self.shapes
            .range(lo..hi)
            .filter(|(cell, _)| (0..3).all(|a| (lo[a]..hi[a]).contains(&cell[a])))
            .map(|(&cell, &shape)| (cell, shape))
            .collect()
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
        let chunk = cell.map(|c| c.div_euclid(CHUNK));
        let slot = self.slot(chunk).expect("a contained cell has a chunk");
        if material == 0 && self.chunks[slot].is_none() {
            return false;
        }
        let cells = self.chunks[slot].get_or_insert_with(|| Box::new([0; CELLS]));
        let index = local(cell);
        if cells[index] == material && self.shapes.get(&cell).copied().unwrap_or_default() == shape
        {
            return false;
        }
        cells[index] = material;
        if shape == Shape::Cube {
            self.shapes.remove(&cell);
        } else {
            self.shapes.insert(cell, shape);
        }
        self.dirty.insert(chunk);
        // A cell on a chunk's face decides which faces its neighbour draws.
        for axis in 0..3 {
            let at = cell[axis].rem_euclid(CHUNK);
            for (edge, step) in [(0, -1), (CHUNK - 1, 1)] {
                if at == edge {
                    let mut next = chunk;
                    next[axis] += step;
                    if self.slot(next).is_some() {
                        self.dirty.insert(next);
                    }
                }
            }
        }
        true
    }

    /// Frees a chunk that has gone back to air.
    pub fn prune(&mut self, chunk: [i32; 3]) {
        if let Some(slot) = self.slot(chunk)
            && self.chunks[slot]
                .as_deref()
                .is_some_and(|cells| cells.iter().all(|&m| m == 0))
        {
            self.chunks[slot] = None;
        }
    }

    pub fn take_dirty(&mut self) -> Vec<[i32; 3]> {
        std::mem::take(&mut self.dirty).into_iter().collect()
    }

    pub fn mark_all_dirty(&mut self) {
        let n = self.chunk_counts();
        for z in 0..n[2] {
            for y in 0..n[1] {
                for x in 0..n[0] {
                    self.dirty.insert([x, y, z]);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.chunks.iter_mut().for_each(|c| *c = None);
        self.shapes.clear();
        self.dirty.clear();
    }

    pub fn solid_count(&self) -> u64 {
        self.chunks
            .iter()
            .flatten()
            .map(|cells| cells.iter().filter(|&&m| m != 0).count() as u64)
            .sum()
    }

    /// The highest solid cell in a column, if there is one.
    pub fn height_at(&self, x: i32, z: i32) -> Option<i32> {
        (0..self.size[1]).rev().find(|&y| self.get([x, y, z]) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_round_up_to_whole_chunks() {
        let grid = Grid::new([17, 1, 32]);
        assert_eq!(grid.size(), [32, 16, 32]);
        assert_eq!(grid.chunk_counts(), [2, 1, 2]);
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
        let mut grid = Grid::new([32, 16, 16]);
        grid.set([15, 5, 5], 1);
        assert_eq!(grid.take_dirty(), [[0, 0, 0], [1, 0, 0]]);
        grid.set([5, 5, 5], 1);
        assert_eq!(grid.take_dirty(), [[0, 0, 0]]);
        assert!(grid.take_dirty().is_empty());
    }

    #[test]
    fn shapes_are_kept_per_cell_and_dropped_with_the_cell() {
        let mut grid = Grid::new([32, 16, 16]);
        assert!(!grid.reshape([1, 1, 1], Shape::Slab), "air has no shape");
        grid.set([1, 1, 1], 1);
        assert!(grid.is_full([1, 1, 1]));
        assert!(grid.reshape([1, 1, 1], Shape::Slab));
        assert!(!grid.reshape([1, 1, 1], Shape::Slab));
        assert_eq!(grid.shape_at([1, 1, 1]), Shape::Slab);
        assert!(!grid.is_full([1, 1, 1]));
        assert_eq!(grid.get([1, 1, 1]), 1);
        grid.set_shaped([20, 2, 2], 3, Shape::Post);
        assert_eq!(grid.shaped_in([0, 0, 0]).len(), 1);
        assert_eq!(grid.shaped_in([1, 0, 0]), [([20, 2, 2], Shape::Post)]);
        // Writing a cube, or air, takes the shape away.
        grid.set([1, 1, 1], 1);
        assert_eq!(grid.shape_at([1, 1, 1]), Shape::Cube);
        grid.set([20, 2, 2], 0);
        assert!(grid.shaped_in([1, 0, 0]).is_empty());
        assert!(Shape::from_name("Top slab").is_ok());
        assert!(Shape::from_name("ramp").is_err());
        assert_eq!(Shape::from_name(Shape::TopSlab.name()), Ok(Shape::TopSlab));
    }

    #[test]
    fn an_emptied_chunk_can_be_freed() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([1, 1, 1], 1);
        grid.set([1, 1, 1], 0);
        assert!(grid.chunk([0, 0, 0]).is_some());
        grid.prune([0, 0, 0]);
        assert!(grid.chunk([0, 0, 0]).is_none());
    }
}
