//! Block and sky light, 0 to 15, flood-filled around a region so its faces
//! can be shaded. Light loses one level a cell, so a margin of 16 cells is
//! all a region's faces can see: the result is exact, not an estimate.

use crate::grid::{Block, Grid, Shape};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX: u8 = 15;
/// Cells around a region that can still light the cells beside its faces.
const MARGIN: i32 = MAX as i32 + 1;

/// How each block treats light: what it gives off and whether it lets light
/// through (cutout and translucent blocks do).
#[derive(Clone, Debug, Default)]
pub struct LightRules {
    emit: BTreeMap<Block, u8>,
    clear: BTreeSet<Block>,
}

impl LightRules {
    pub fn new(registry: &crate::registry::Registry, palette: &crate::palette::Palette) -> Self {
        let mut rules = LightRules::default();
        for id in 1..=palette.len() {
            let def = registry.get(id);
            let glows = palette.get(id).is_some_and(|m| m.emission > 0.0);
            let level = def
                .and_then(|d| d.light)
                .unwrap_or(if glows { MAX } else { 0 });
            if level > 0 {
                rules.emit.insert(id, level);
            }
            if def.is_some_and(|d| d.layer != crate::registry::Layer::Opaque) {
                rules.clear.insert(id);
            }
        }
        for def in registry.ids().into_iter().filter_map(|id| registry.get(id)) {
            if let Some(level) = def.light.filter(|l| *l > 0) {
                rules.emit.insert(def.id, level);
            }
            if def.layer != crate::registry::Layer::Opaque {
                rules.clear.insert(def.id);
            }
        }
        rules
    }

    /// Whether a block lets light (and the faces behind it) be seen.
    pub fn is_clear(&self, id: Block) -> bool {
        id == 0 || self.clear.contains(&id)
    }

    fn blocks(&self, grid: &Grid, cell: [i32; 3], id: Block) -> bool {
        !self.is_clear(id) && grid.shape_at(cell) == Shape::Cube
    }
}

/// Sky and block light over a region and its margin.
#[derive(Debug)]
pub struct LightField {
    lo: [i32; 3],
    dims: [i32; 3],
    sky: Vec<u8>,
    block: Vec<u8>,
}

impl LightField {
    pub fn compute(grid: &Grid, rules: &LightRules, base: [i32; 3], extent: [i32; 3]) -> Self {
        let lo = base.map(|b| b - MARGIN);
        let dims = extent.map(|e| e + 2 * MARGIN);
        let total = (dims[0] * dims[1] * dims[2]) as usize;
        let mut field = LightField {
            lo,
            dims,
            sky: vec![0; total],
            block: vec![0; total],
        };
        let mut solid = vec![false; total];
        let mut queue = VecDeque::new();
        for z in 0..dims[2] {
            for x in 0..dims[0] {
                let (wx, wz) = (lo[0] + x, lo[2] + z);
                let top_y = lo[1] + dims[1] - 1;
                // Shade the field's top when anything opaque stands above it.
                let mut lit = true;
                if let Some(top) = grid.height_at(wx, wz) {
                    lit = !(top_y + 1..=top).any(|y| {
                        let cell = [wx, y, wz];
                        rules.blocks(grid, cell, grid.get(cell))
                    });
                }
                for y in (0..dims[1]).rev() {
                    let cell = [wx, lo[1] + y, wz];
                    let i = field.index([x, y, z]);
                    let id = grid.get(cell);
                    if rules.blocks(grid, cell, id) {
                        solid[i] = true;
                        lit = false;
                    } else if lit {
                        field.sky[i] = MAX;
                        queue.push_back(i);
                    }
                    if let Some(&level) = rules.emit.get(&id) {
                        field.block[i] = level;
                    }
                }
            }
        }
        field.spread(&solid, queue, true);
        let emitters = (0..total).filter(|&i| field.block[i] > 0).collect();
        field.spread(&solid, emitters, false);
        field
    }

    fn index(&self, at: [i32; 3]) -> usize {
        ((at[2] * self.dims[1] + at[1]) * self.dims[0] + at[0]) as usize
    }

    fn spread(&mut self, solid: &[bool], mut queue: VecDeque<usize>, sky: bool) {
        let [dx, dy, _] = self.dims;
        let (sx, sy) = (1usize, dx as usize);
        let sz = (dx * dy) as usize;
        while let Some(i) = queue.pop_front() {
            let levels = if sky { &mut self.sky } else { &mut self.block };
            let level = levels[i];
            if level <= 1 {
                continue;
            }
            let x = i % sy;
            let y = (i / sy) % dy as usize;
            let z = i / sz;
            let mut visit = |j: usize, levels: &mut Vec<u8>| {
                if !solid[j] && levels[j] < level - 1 {
                    levels[j] = level - 1;
                    queue.push_back(j);
                }
            };
            let levels = if sky { &mut self.sky } else { &mut self.block };
            if x > 0 {
                visit(i - sx, levels);
            }
            if x + 1 < dx as usize {
                visit(i + sx, levels);
            }
            if y > 0 {
                visit(i - sy, levels);
            }
            if y + 1 < dy as usize {
                visit(i + sy, levels);
            }
            if z > 0 {
                visit(i - sz, levels);
            }
            if z + 1 < self.dims[2] as usize {
                visit(i + sz, levels);
            }
        }
    }

    /// The light at a cell: its block light or its sky light dimmed by the
    /// time of day, whichever is brighter. Outside the field is open sky.
    pub fn level(&self, cell: [i32; 3], daylight: f32) -> u8 {
        let at = [0, 1, 2].map(|a| cell[a] - self.lo[a]);
        if (0..3).any(|a| at[a] < 0 || at[a] >= self.dims[a]) {
            return (f32::from(MAX) * daylight).round() as u8;
        }
        let i = self.index(at);
        let sky = (f32::from(self.sky[i]) * daylight.clamp(0.0, 1.0)).round() as u8;
        sky.max(self.block[i])
    }
}

/// What a light level does to a surface's color: full at 15, a fifth less
/// each level below, never quite black.
pub fn brightness(level: u8) -> f32 {
    if level >= MAX {
        1.0
    } else {
        0.03 + 0.97 * 0.8f32.powi(i32::from(MAX - level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_with_lamp(lamp: Block) -> LightRules {
        let mut rules = LightRules::default();
        rules.emit.insert(lamp, 14);
        rules
    }

    #[test]
    fn sky_fills_the_open_and_stops_under_a_roof() {
        let mut grid = Grid::new([32, 32, 32]);
        for x in 0..32 {
            for z in 0..32 {
                grid.set([x, 10, z], 1);
            }
        }
        let field = LightField::compute(&grid, &LightRules::default(), [0, 0, 0], [32, 32, 32]);
        assert_eq!(field.level([5, 20, 5], 1.0), 15);
        // A sealed roof over the whole world: dark below, unless the edge
        // of the world lets the sky in from the side.
        assert_eq!(field.level([16, 5, 16], 1.0), 0);
        assert!(field.level([0, 5, 0], 1.0) > 0);
        // Night dims the sky but not block light.
        assert_eq!(field.level([5, 20, 5], 0.2), 3);
    }

    #[test]
    fn a_lamp_lights_one_level_less_a_cell_and_walls_stop_it() {
        let mut grid = Grid::new([32, 32, 32]);
        // A closed box from y 1 to 9 with a lamp inside.
        for x in 0..32 {
            for z in 0..32 {
                grid.set([x, 10, z], 1);
            }
        }
        grid.set([16, 5, 16], 2);
        grid.set([20, 5, 16], 1);
        let rules = rules_with_lamp(2);
        let field = LightField::compute(&grid, &rules, [0, 0, 0], [32, 32, 32]);
        assert_eq!(field.level([16, 5, 16], 0.0), 14);
        assert_eq!(field.level([17, 5, 16], 0.0), 13);
        assert_eq!(field.level([19, 5, 16], 0.0), 11);
        // Around the stone, not through it.
        assert_eq!(field.level([21, 5, 16], 0.0), 7);
        // A see-through block lets it pass.
        let mut clear = rules.clone();
        clear.clear.insert(1);
        let field = LightField::compute(&grid, &clear, [0, 0, 0], [32, 32, 32]);
        assert_eq!(field.level([21, 5, 16], 0.0), 9);
    }

    #[test]
    fn light_from_beyond_the_region_reaches_its_faces() {
        let mut grid = Grid::new([64, 32, 32]);
        for x in 0..64 {
            for z in 0..32 {
                grid.set([x, 10, z], 1);
            }
        }
        grid.set([30, 5, 16], 2);
        let rules = rules_with_lamp(2);
        let whole = LightField::compute(&grid, &rules, [0, 0, 0], [64, 32, 32]);
        let part = LightField::compute(&grid, &rules, [32, 0, 0], [32, 32, 32]);
        for x in 31..48 {
            assert_eq!(
                part.level([x, 5, 16], 0.0),
                whole.level([x, 5, 16], 0.0),
                "{x}"
            );
        }
        assert_eq!(brightness(15), 1.0);
        assert!(brightness(0) > 0.0 && brightness(0) < brightness(1));
    }
}
