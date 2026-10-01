//! The authoritative cells: a finite box of chunks, each 16 cells a side.
//!
//! A chunk that holds only air is not allocated. Cell coordinates are signed
//! and start at zero in one corner; anything outside the box reads as air and
//! ignores writes.

use std::collections::BTreeSet;

pub const CHUNK: i32 = 16;
const CELLS: usize = (CHUNK * CHUNK * CHUNK) as usize;

pub type Cells = [u8; CELLS];

pub struct Grid {
    /// Size in cells, a whole number of chunks on each axis.
    size: [i32; 3],
    chunks: Vec<Option<Box<Cells>>>,
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

    /// Writes a cell; true when that changed it.
    pub fn set(&mut self, cell: [i32; 3], material: u8) -> bool {
        if !self.contains(cell) {
            return false;
        }
        let chunk = cell.map(|c| c.div_euclid(CHUNK));
        let slot = self.slot(chunk).expect("a contained cell has a chunk");
        if material == 0 && self.chunks[slot].is_none() {
            return false;
        }
        let cells = self.chunks[slot].get_or_insert_with(|| Box::new([0; CELLS]));
        let index = local(cell);
        if cells[index] == material {
            return false;
        }
        cells[index] = material;
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
    fn an_emptied_chunk_can_be_freed() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([1, 1, 1], 1);
        grid.set([1, 1, 1], 0);
        assert!(grid.chunk([0, 0, 0]).is_some());
        grid.prune([0, 0, 0]);
        assert!(grid.chunk([0, 0, 0]).is_none());
    }
}
