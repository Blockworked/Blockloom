//! Seeded terrain: the same seed and size always give the same cells, whatever
//! order they are made in, since every cell asks only a hash of its own
//! coordinates.

use crate::grid::Grid;
use crate::palette::{DIRT, GLOW, GRASS, LEAVES, SAND, STONE, WOOD};

pub const PRESETS: [&str; 4] = ["island", "caves", "flat", "empty"];

fn hash(seed: u32, x: i32, y: i32, z: i32) -> u32 {
    let mut h = seed
        ^ (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA77)
        ^ (z as u32).wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Value noise in 0..1, trilinear over the integer lattice.
fn noise3(seed: u32, x: f32, y: f32, z: f32) -> f32 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (tx, ty, tz) = (smooth(x - fx), smooth(y - fy), smooth(z - fz));
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let corner = |dx, dy, dz| unit(hash(seed, ix + dx, iy + dy, iz + dz));
    let lower = lerp(
        lerp(corner(0, 0, 0), corner(1, 0, 0), tx),
        lerp(corner(0, 1, 0), corner(1, 1, 0), tx),
        ty,
    );
    let upper = lerp(
        lerp(corner(0, 0, 1), corner(1, 0, 1), tx),
        lerp(corner(0, 1, 1), corner(1, 1, 1), tx),
        ty,
    );
    lerp(lower, upper, tz)
}

/// Layered noise in 0..1.
fn fbm(seed: u32, x: f32, z: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * noise3(seed.wrapping_add(o * 101), x * freq, 0.5, z * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

fn seed32(seed: i64) -> u32 {
    (seed as u64 ^ (seed as u64 >> 32)) as u32
}

fn column(size: [i32; 3], preset: &str, seed: u32, x: i32, z: i32) -> (i32, bool) {
    let [sx, sy, sz] = size;
    let (fx, fz, height) = (x as f32, z as f32, sy as f32);
    let top = match preset {
        "flat" => (height * 0.25) as i32,
        "island" => {
            let dx = (fx / sx as f32 - 0.5) * 2.0;
            let dz = (fz / sz as f32 - 0.5) * 2.0;
            let mask = (1.0 - (dx * dx + dz * dz)).clamp(0.0, 1.0);
            ((0.12 + 0.5 * mask * (0.4 + fbm(seed, fx / 28.0, fz / 28.0, 4))) * height) as i32
        }
        _ => ((0.3 + 0.35 * fbm(seed, fx / 36.0, fz / 36.0, 4)) * height) as i32,
    }
    .clamp(1, sy - 1);
    (top, preset == "island" && top <= (height * 0.22) as i32 + 1)
}

fn tree(size: [i32; 3], preset: &str, seed: u32, x: i32, z: i32) -> Option<(i32, i32)> {
    if preset == "flat"
        || x <= 3
        || z <= 3
        || x >= size[0] - 3
        || z >= size[2] - 3
        || !hash(seed ^ 0x77, x, 0, z).is_multiple_of(170)
    {
        return None;
    }
    let (top, beach) = column(size, preset, seed, x, z);
    (!beach).then(|| (top, top + 4 + (hash(seed, x, 1, z) % 2) as i32))
}

pub fn height_bound(size: [i32; 3], preset: &str, seed: i64, x: i32, z: i32) -> i32 {
    if preset == "empty" {
        return -1;
    }
    let seed = seed32(seed);
    let mut top = column(size, preset, seed, x, z).0;
    for dx in -2..=2 {
        for dz in -2..=2 {
            if let Some((_, crown)) = tree(size, preset, seed, x + dx, z + dz) {
                top = top.max(crown + 2);
            }
        }
    }
    top.min(size[1] - 1)
}

/// A canonical sample, including trees crossing page borders.
pub fn sample(size: [i32; 3], preset: &str, seed: i64, cell: [i32; 3]) -> u8 {
    if preset == "empty" || (0..3).any(|a| cell[a] < 0 || cell[a] >= size[a]) {
        return 0;
    }
    let seed = seed32(seed);
    let [x, y, z] = cell;
    let (top, beach) = column(size, preset, seed, x, z);
    if y <= top {
        if preset == "caves"
            && y > 1
            && y < top - 2
            && noise3(
                seed ^ 0x5bd1,
                x as f32 / 11.0,
                y as f32 / 7.0,
                z as f32 / 11.0,
            ) > 0.64
        {
            return 0;
        }
        let depth = top - y;
        return if depth == 0 {
            if beach { SAND } else { GRASS }
        } else if depth <= 3 {
            if beach { SAND } else { DIRT }
        } else if preset == "caves"
            && noise3(seed ^ 0x2f1, x as f32 / 4.0, y as f32 / 4.0, z as f32 / 4.0) > 0.8
        {
            GLOW
        } else {
            STONE
        };
    }
    if let Some((top, crown)) = tree(size, preset, seed, x, z)
        && y > top
        && y <= crown
    {
        return WOOD;
    }
    if preset != "flat" {
        for dx in -2..=2 {
            for dz in -2..=2 {
                if let Some((_, crown)) = tree(size, preset, seed, x + dx, z + dz) {
                    let dy = y - crown;
                    if (-1..=2).contains(&dy) && dx * dx + dz * dz + dy * dy * 2 <= 6 {
                        return LEAVES;
                    }
                }
            }
        }
    }
    0
}

/// Fills `grid` from `preset`. Unknown presets are an error; `empty` leaves
/// the world as air.
pub fn generate(grid: &mut Grid, preset: &str, seed: i64) -> Result<(), String> {
    if !PRESETS.contains(&preset) {
        return Err(format!(
            "no terrain preset called {preset} (try {})",
            PRESETS.join(", ")
        ));
    }
    grid.clear();
    if preset == "empty" {
        return Ok(());
    }
    let seed = seed32(seed);
    let [sx, sy, sz] = grid.size();
    let height = sy as f32;
    let sea = (height * 0.22) as i32;
    let mut trees = Vec::new();
    for x in 0..sx {
        for z in 0..sz {
            let (fx, fz) = (x as f32, z as f32);
            let top = match preset {
                "flat" => (height * 0.25) as i32,
                "island" => {
                    let dx = (fx / sx as f32 - 0.5) * 2.0;
                    let dz = (fz / sz as f32 - 0.5) * 2.0;
                    let mask = (1.0 - (dx * dx + dz * dz)).clamp(0.0, 1.0);
                    let n = fbm(seed, fx / 28.0, fz / 28.0, 4);
                    ((0.12 + 0.5 * mask * (0.4 + n)) * height) as i32
                }
                _ => ((0.3 + 0.35 * fbm(seed, fx / 36.0, fz / 36.0, 4)) * height) as i32,
            }
            .clamp(1, sy - 1);
            let beach = preset == "island" && top <= sea + 1;
            for y in 0..=top {
                if preset == "caves" && y > 1 && y < top - 2 {
                    let cave = noise3(seed ^ 0x5bd1, fx / 11.0, y as f32 / 7.0, fz / 11.0);
                    if cave > 0.64 {
                        continue;
                    }
                }
                let depth = top - y;
                let material = if depth == 0 {
                    if beach { SAND } else { GRASS }
                } else if depth <= 3 {
                    if beach { SAND } else { DIRT }
                } else if preset == "caves"
                    && noise3(seed ^ 0x2f1, fx / 4.0, y as f32 / 4.0, fz / 4.0) > 0.8
                {
                    GLOW
                } else {
                    STONE
                };
                grid.set([x, y, z], material);
            }
            if !beach
                && preset != "flat"
                && x > 3
                && z > 3
                && x < sx - 3
                && z < sz - 3
                && hash(seed ^ 0x77, x, 0, z).is_multiple_of(170)
            {
                trees.push([x, top, z]);
            }
        }
    }
    for [x, top, z] in trees {
        let trunk = 4 + (hash(seed, x, 1, z) % 2) as i32;
        for dy in 1..=trunk {
            grid.set([x, top + dy, z], WOOD);
        }
        let crown = top + trunk;
        for dx in -2..=2i32 {
            for dy in -1..=2i32 {
                for dz in -2..=2i32 {
                    let near = dx * dx + dz * dz + dy * dy * 2;
                    let cell = [x + dx, crown + dy, z + dz];
                    if near <= 6 && grid.get(cell) == 0 {
                        grid.set(cell, LEAVES);
                    }
                }
            }
        }
    }
    // Generation is the base state, not an edit.
    grid.take_dirty();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(grid: &Grid) -> Vec<u8> {
        let [sx, sy, sz] = grid.size();
        let mut all = Vec::new();
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    all.push(grid.get([x, y, z]));
                }
            }
        }
        all
    }

    #[test]
    fn paged_samples_match_eager_generation_including_trees() {
        for preset in PRESETS {
            let mut grid = Grid::new([48, 32, 48]);
            generate(&mut grid, preset, 42).unwrap();
            for z in 0..48 {
                for y in 0..32 {
                    for x in 0..48 {
                        assert_eq!(
                            grid.get([x, y, z]),
                            sample(grid.size(), preset, 42, [x, y, z]),
                            "{preset} at {x},{y},{z}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_seed_always_makes_the_same_world() {
        let mut a = Grid::new([48, 32, 48]);
        let mut b = Grid::new([48, 32, 48]);
        generate(&mut a, "caves", 42).unwrap();
        generate(&mut b, "caves", 42).unwrap();
        assert_eq!(cells(&a), cells(&b));
        generate(&mut b, "caves", 43).unwrap();
        assert_ne!(cells(&a), cells(&b));
        // Regenerating onto an old world forgets it.
        generate(&mut a, "caves", 43).unwrap();
        assert_eq!(cells(&a), cells(&b));
    }

    #[test]
    fn flat_is_a_layered_floor() {
        let mut grid = Grid::new([32, 32, 32]);
        generate(&mut grid, "flat", 1).unwrap();
        assert_eq!(grid.height_at(5, 5), Some(8));
        assert_eq!(grid.get([5, 8, 5]), GRASS);
        assert_eq!(grid.get([5, 6, 5]), DIRT);
        assert_eq!(grid.get([5, 2, 5]), STONE);
        assert!(grid.take_dirty().is_empty());
    }

    #[test]
    fn an_island_is_higher_in_the_middle_and_has_beaches() {
        let mut grid = Grid::new([64, 32, 64]);
        generate(&mut grid, "island", 7).unwrap();
        let middle = grid.height_at(32, 32).unwrap();
        let rim = grid.height_at(1, 1).unwrap();
        assert!(middle > rim, "{middle} vs {rim}");
        assert_eq!(grid.get([1, rim, 1]), SAND);
    }

    #[test]
    fn caves_are_hollow_and_have_glowing_ore() {
        let mut grid = Grid::new([64, 32, 64]);
        generate(&mut grid, "caves", 3).unwrap();
        let all = cells(&grid);
        assert!(all.contains(&GLOW), "no ore in the stone");
        // Somewhere below the surface is air with stone above it.
        let hollow = (0..64).any(|x| {
            (0..64).any(|z| {
                let top = grid.height_at(x, z).unwrap_or(0);
                (2..top - 3).any(|y| grid.get([x, y, z]) == 0)
            })
        });
        assert!(hollow);
    }

    #[test]
    fn empty_is_air_and_unknown_presets_are_refused() {
        let mut grid = Grid::new([16, 16, 16]);
        generate(&mut grid, "flat", 1).unwrap();
        generate(&mut grid, "empty", 1).unwrap();
        assert_eq!(grid.solid_count(), 0);
        assert!(
            generate(&mut grid, "moon", 1)
                .unwrap_err()
                .contains("island")
        );
    }

    #[test]
    fn noise_stays_in_range_and_varies() {
        let values: Vec<f32> = (0..200)
            .map(|i| fbm(9, i as f32 * 0.37, i as f32 * 0.11, 4))
            .collect();
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
        let (lo, hi) = values
            .iter()
            .fold((1.0f32, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(hi - lo > 0.2, "{lo}..{hi}");
    }
}
