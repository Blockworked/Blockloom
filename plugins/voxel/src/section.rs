//! Palette-compressed section cells: a palette of block ids and bit-packed
//! indices into it, 0 bits for a uniform section up to 16 for a direct one.

use crate::grid::{Block, SECTION_CELLS};

#[derive(Clone, Debug, PartialEq)]
pub struct Packed {
    palette: Vec<Block>,
    bits: u32,
    words: Vec<u64>,
}

impl Packed {
    pub fn filled(block: Block) -> Packed {
        Packed {
            palette: vec![block],
            bits: 0,
            words: Vec::new(),
        }
    }

    pub fn from_cells(cells: &[Block]) -> Packed {
        debug_assert_eq!(cells.len(), SECTION_CELLS);
        let mut palette: Vec<Block> = cells.to_vec();
        palette.sort_unstable();
        palette.dedup();
        let mut packed = Packed {
            bits: bits_for(palette.len()),
            words: Vec::new(),
            palette,
        };
        packed.words = vec![0; words_for(packed.bits)];
        for (i, &b) in cells.iter().enumerate() {
            let index = packed.palette.binary_search(&b).unwrap() as u64;
            packed.put(i, index);
        }
        packed
    }

    pub fn get(&self, i: usize) -> Block {
        if self.bits == 0 {
            return self.palette[0];
        }
        let per = 64 / self.bits as usize;
        let shift = (i % per) as u32 * self.bits;
        let index = (self.words[i / per] >> shift) & ((1u64 << self.bits) - 1);
        self.palette[index as usize]
    }

    /// Writes one cell; true when it changed.
    pub fn set(&mut self, i: usize, block: Block) -> bool {
        if self.get(i) == block {
            return false;
        }
        let index = match self.palette.iter().position(|&p| p == block) {
            Some(index) => index,
            None => {
                self.palette.push(block);
                let need = bits_for(self.palette.len());
                if need != self.bits {
                    self.repack(need);
                }
                self.palette.len() - 1
            }
        };
        self.put(i, index as u64);
        true
    }

    pub fn is_uniform(&self, block: Block) -> bool {
        if self.palette.len() == 1 {
            return self.palette[0] == block;
        }
        (0..SECTION_CELLS).all(|i| self.get(i) == block)
    }

    pub fn unpack(&self) -> Vec<Block> {
        (0..SECTION_CELLS).map(|i| self.get(i)).collect()
    }

    /// Drops palette entries nothing uses any more.
    pub fn compact(&mut self) {
        *self = Packed::from_cells(&self.unpack());
    }

    pub fn bytes(&self) -> usize {
        self.words.len() * 8 + self.palette.len() * 2
    }

    #[cfg(test)]
    pub fn bits(&self) -> u32 {
        self.bits
    }

    fn put(&mut self, i: usize, index: u64) {
        if self.bits == 0 {
            return;
        }
        let per = 64 / self.bits as usize;
        let shift = (i % per) as u32 * self.bits;
        let mask = ((1u64 << self.bits) - 1) << shift;
        let word = &mut self.words[i / per];
        *word = (*word & !mask) | (index << shift);
    }

    fn repack(&mut self, bits: u32) {
        let old = Packed {
            palette: self.palette.clone(),
            bits: self.bits,
            words: std::mem::take(&mut self.words),
        };
        let indices: Vec<u64> = (0..SECTION_CELLS)
            .map(|i| {
                if old.bits == 0 {
                    0
                } else {
                    let per = 64 / old.bits as usize;
                    (old.words[i / per] >> ((i % per) as u32 * old.bits)) & ((1u64 << old.bits) - 1)
                }
            })
            .collect();
        self.bits = bits;
        self.words = vec![0; words_for(bits)];
        for (i, index) in indices.into_iter().enumerate() {
            self.put(i, index);
        }
    }
}

/// Bits per index for a palette of `n` entries: 0, 1, 2, 4, 8 or 16.
fn bits_for(n: usize) -> u32 {
    match n {
        0 | 1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        17..=256 => 8,
        _ => 16,
    }
}

fn words_for(bits: u32) -> usize {
    if bits == 0 {
        0
    } else {
        SECTION_CELLS.div_ceil(64 / bits as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_widen_as_the_palette_grows_and_compact_shrinks_it() {
        let mut p = Packed::filled(0);
        assert_eq!((p.bits(), p.bytes()), (0, 2));
        assert!(p.set(5, 3));
        assert_eq!(p.bits(), 1);
        for (i, b) in (0..300u16).enumerate() {
            p.set(100 + i, 1000 + b);
        }
        assert_eq!(p.bits(), 16);
        assert_eq!(p.get(5), 3);
        assert_eq!(p.get(100 + 299), 1299);
        assert_eq!(p.get(0), 0);
        for i in 0..SECTION_CELLS {
            p.set(i, 7);
        }
        p.compact();
        assert!(p.is_uniform(7));
        assert_eq!(p.bits(), 0);
        let cells: Vec<Block> = (0..SECTION_CELLS).map(|i| (i % 5) as Block).collect();
        let p = Packed::from_cells(&cells);
        assert_eq!(p.bits(), 4);
        assert_eq!(p.unpack(), cells);
        assert_eq!(p.bytes(), SECTION_CELLS / 2 + 10);
    }
}
