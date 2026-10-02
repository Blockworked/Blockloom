//! Approximate convex decomposition of a concave mesh, for dynamic bodies.
//!
//! The mesh is voxelized (surface, then the inside filled when the surface is
//! closed), and the solid cells are split along axis planes, always the part that
//! wastes the most space inside its hull, until each hull is close enough to its
//! part or the hull budget is spent. Each hull is built from voxel corners, so it
//! can stand up to one voxel proud of the mesh: that is the reported error.

use super::hull::quickhull;
use super::{CookControl, CookError, MAX_HULL_VERTICES, RawMesh, fail};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

/// How hard to decompose one mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decompose {
    /// The most hulls produced.
    #[serde(default = "default_hulls")]
    pub max_hulls: u16,
    /// The most points one hull keeps.
    #[serde(default = "default_vertices")]
    pub max_hull_vertices: u16,
    /// Stop splitting a part whose hull wastes less than this share of itself.
    #[serde(default = "default_concavity")]
    pub concavity: f32,
    /// Voxels along the longest side.
    #[serde(default = "default_resolution")]
    pub resolution: u16,
}

fn default_hulls() -> u16 {
    16
}
fn default_vertices() -> u16 {
    32
}
fn default_concavity() -> f32 {
    0.05
}
fn default_resolution() -> u16 {
    32
}

impl Default for Decompose {
    fn default() -> Self {
        Self {
            max_hulls: default_hulls(),
            max_hull_vertices: default_vertices(),
            concavity: default_concavity(),
            resolution: default_resolution(),
        }
    }
}

impl Decompose {
    pub fn check(&self) -> Result<(), String> {
        if !(1..=256).contains(&self.max_hulls) {
            return Err("Decomposition makes between 1 and 256 hulls".into());
        }
        if !(4..=MAX_HULL_VERTICES as u16).contains(&self.max_hull_vertices) {
            return Err(format!(
                "A decomposed hull keeps between 4 and {MAX_HULL_VERTICES} points"
            ));
        }
        if !(0.0..=1.0).contains(&self.concavity) {
            return Err("Concavity is a share between 0 and 1".into());
        }
        if !(8..=64).contains(&self.resolution) {
            return Err("Decomposition resolution is between 8 and 64 voxels".into());
        }
        Ok(())
    }
}

/// The hulls of a decomposed mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Decomposition {
    pub hulls: Vec<Vec<[f32; 3]>>,
    /// The hulls' summed volume (overlap counted twice).
    pub volume: f32,
    /// How far a hull may stand outside the mesh: one voxel.
    pub error: f32,
}

struct Grid {
    lo: [f32; 3],
    size: f32,
    /// Cells per axis, padded by one empty cell all round.
    dims: [usize; 3],
}

impl Grid {
    fn index(&self, c: [usize; 3]) -> usize {
        (c[2] * self.dims[1] + c[1]) * self.dims[0] + c[0]
    }

    fn coord(&self, i: usize) -> [usize; 3] {
        let x = i % self.dims[0];
        let y = (i / self.dims[0]) % self.dims[1];
        [x, y, i / (self.dims[0] * self.dims[1])]
    }

    /// The model-space position of a lattice corner.
    fn corner(&self, c: [usize; 3]) -> [f32; 3] {
        [0, 1, 2].map(|a| self.lo[a] + (c[a] as f32 - 1.0) * self.size)
    }
}

const NEIGHBOURS: [[isize; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

fn voxelize(
    mesh: &RawMesh,
    settings: &Decompose,
    control: &CookControl,
) -> Result<(Grid, Vec<bool>), CookError> {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let extent = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f32::max);
    if !(extent > 0.0) {
        return fail("The mesh has no size");
    }
    let size = extent / f32::from(settings.resolution);
    let dims = [0, 1, 2].map(|a| ((hi[a] - lo[a]) / size).floor() as usize + 3);
    let grid = Grid { lo, size, dims };
    let mut surface = vec![false; dims[0] * dims[1] * dims[2]];
    let cell = |p: [f32; 3]| -> [usize; 3] {
        [0, 1, 2].map(|a| (((p[a] - lo[a]) / size).floor() as usize + 1).min(dims[a] - 2))
    };
    for (n, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if n % 4096 == 0 {
            control.check()?;
        }
        let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[tri[k] as usize]);
        let longest = [(a, b), (b, c), (c, a)]
            .iter()
            .map(|(p, q)| {
                ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
            })
            .fold(0.0, f32::max);
        let steps = ((longest / (size * 0.5)).ceil() as usize).clamp(1, 256);
        for i in 0..=steps {
            for j in 0..=(steps - i) {
                let (u, v) = (i as f32 / steps as f32, j as f32 / steps as f32);
                let p = [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * u + (c[k] - a[k]) * v);
                surface[grid.index(cell(p))] = true;
            }
        }
    }
    // What the outside can reach is outside; everything else is solid.
    let mut outside = vec![false; surface.len()];
    let mut queue = VecDeque::from([0usize]);
    outside[0] = true;
    while let Some(i) = queue.pop_front() {
        let c = grid.coord(i);
        for d in NEIGHBOURS {
            let n = [0, 1, 2].map(|a| c[a] as isize + d[a]);
            if (0..3).any(|a| n[a] < 0 || n[a] as usize >= dims[a]) {
                continue;
            }
            let ni = grid.index(n.map(|v| v as usize));
            if !outside[ni] && !surface[ni] {
                outside[ni] = true;
                queue.push_back(ni);
            }
        }
    }
    let solid = outside.iter().map(|o| !o).collect();
    Ok((grid, solid))
}

/// A set of solid cells, with the hull it would get.
struct Part {
    cells: Vec<usize>,
    hull_volume: f32,
    volume: f32,
}

impl Part {
    fn waste(&self) -> f32 {
        (self.hull_volume - self.volume).max(0.0)
    }

    fn concavity(&self) -> f32 {
        if self.hull_volume > 0.0 {
            self.waste() / self.hull_volume
        } else {
            0.0
        }
    }
}

fn corners(grid: &Grid, cells: &[usize]) -> Vec<[f32; 3]> {
    let members: HashSet<usize> = cells.iter().copied().collect();
    let mut lattice: HashSet<[usize; 3]> = HashSet::new();
    for &i in cells {
        let c = grid.coord(i);
        let edge = NEIGHBOURS.iter().any(|d| {
            let n = [0, 1, 2].map(|a| c[a] as isize + d[a]);
            (0..3).any(|a| n[a] < 0 || n[a] as usize >= grid.dims[a])
                || !members.contains(&grid.index(n.map(|v| v as usize)))
        });
        if !edge {
            continue;
        }
        for dx in 0..2 {
            for dy in 0..2 {
                for dz in 0..2 {
                    lattice.insert([c[0] + dx, c[1] + dy, c[2] + dz]);
                }
            }
        }
    }
    let mut lattice: Vec<_> = lattice.into_iter().collect();
    lattice.sort_unstable();
    lattice.into_iter().map(|c| grid.corner(c)).collect()
}

fn measure(grid: &Grid, cells: Vec<usize>, control: &CookControl) -> Result<Part, CookError> {
    let points = corners(grid, &cells);
    let hull = quickhull(&points, MAX_HULL_VERTICES, control)?;
    Ok(Part {
        volume: cells.len() as f32 * grid.size.powi(3),
        hull_volume: hull.volume(),
        cells,
    })
}

/// Splits `part` at the best axis plane, or says no plane helps.
fn split(
    grid: &Grid,
    part: &Part,
    control: &CookControl,
) -> Result<Option<(Part, Part)>, CookError> {
    let mut best: Option<(f32, usize, usize)> = None;
    for axis in 0..3 {
        let coords: Vec<usize> = part.cells.iter().map(|i| grid.coord(*i)[axis]).collect();
        let (min, max) = (*coords.iter().min().unwrap(), *coords.iter().max().unwrap());
        if min == max {
            continue;
        }
        let span = max - min;
        let tries = span.min(8);
        for t in 1..=tries {
            let plane = min + (span * t).div_ceil(tries + 1).max(1);
            let (a, b): (Vec<usize>, Vec<usize>) = part
                .cells
                .iter()
                .copied()
                .partition(|i| grid.coord(*i)[axis] < plane);
            if a.is_empty() || b.is_empty() {
                continue;
            }
            control.check()?;
            let cost = measure(grid, a, control)?.waste() + measure(grid, b, control)?.waste();
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, axis, plane));
            }
        }
    }
    let Some((cost, axis, plane)) = best else {
        return Ok(None);
    };
    if cost >= part.waste() - 1e-6 {
        return Ok(None);
    }
    let (a, b): (Vec<usize>, Vec<usize>) = part
        .cells
        .iter()
        .copied()
        .partition(|i| grid.coord(*i)[axis] < plane);
    Ok(Some((
        measure(grid, a, control)?,
        measure(grid, b, control)?,
    )))
}

pub fn decompose(
    mesh: &RawMesh,
    settings: &Decompose,
    control: &CookControl,
) -> Result<Decomposition, CookError> {
    settings.check()?;
    control.report("voxelizing", 0.1);
    let (grid, solid) = voxelize(mesh, settings, control)?;
    let cells: Vec<usize> = (0..solid.len()).filter(|i| solid[*i]).collect();
    if cells.is_empty() {
        return fail("The mesh fills no voxels");
    }
    let mut parts = vec![measure(&grid, cells, control)?];
    let mut stuck: HashSet<usize> = HashSet::new();
    while parts.len() < usize::from(settings.max_hulls) {
        control.report(
            "splitting",
            0.3 + 0.6 * parts.len() as f32 / f32::from(settings.max_hulls),
        );
        // The part wasting the most that is still worth splitting.
        let pick = parts
            .iter()
            .enumerate()
            .filter(|(i, p)| !stuck.contains(i) && p.concavity() > settings.concavity)
            .max_by(|a, b| a.1.waste().total_cmp(&b.1.waste()))
            .map(|(i, _)| i);
        let Some(index) = pick else { break };
        match split(&grid, &parts[index], control)? {
            Some((a, b)) => {
                parts[index] = a;
                parts.push(b);
                // A new shape might split where the old one would not.
                stuck.clear();
            }
            None => {
                stuck.insert(index);
            }
        }
    }
    let mut hulls = Vec::new();
    let mut volume = 0.0;
    for part in &parts {
        let hull = quickhull(
            &corners(&grid, &part.cells),
            usize::from(settings.max_hull_vertices),
            control,
        )?;
        volume += hull.volume();
        hulls.push(hull.vertices);
    }
    Ok(Decomposition {
        hulls,
        volume,
        error: grid.size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(mesh: &mut RawMesh, lo: [f32; 3], hi: [f32; 3]) {
        let base = mesh.positions.len() as u32;
        for z in [lo[2], hi[2]] {
            for y in [lo[1], hi[1]] {
                for x in [lo[0], hi[0]] {
                    mesh.positions.push([x, y, z]);
                }
            }
        }
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        for q in quads {
            let i = |k: usize| base + q[k] as u32;
            mesh.indices.extend([i(0), i(1), i(2), i(0), i(2), i(3)]);
        }
    }

    fn l_shape() -> RawMesh {
        let mut mesh = RawMesh::default();
        boxed(&mut mesh, [0.0, 0.0, 0.0], [4.0, 1.0, 1.0]);
        boxed(&mut mesh, [0.0, 1.0, 0.0], [1.0, 4.0, 1.0]);
        mesh
    }

    #[test]
    fn a_convex_mesh_stays_one_hull() {
        let mut cube = RawMesh::default();
        boxed(&mut cube, [-1.0; 3], [1.0; 3]);
        let out = decompose(&cube, &Decompose::default(), &CookControl::new()).unwrap();
        assert_eq!(out.hulls.len(), 1);
    }

    #[test]
    fn an_l_shape_splits_and_hugs_the_shape_better_than_one_hull() {
        let mesh = l_shape();
        let settings = Decompose {
            concavity: 0.02,
            ..Default::default()
        };
        let out = decompose(&mesh, &settings, &CookControl::new()).unwrap();
        assert!(out.hulls.len() >= 2, "{} hulls", out.hulls.len());
        // The L is 7 cubic units; one hull around it is about 11.
        let one = quickhull(&mesh.positions, 255, &CookControl::new())
            .unwrap()
            .volume();
        assert!(one > 10.0, "{one}");
        assert!(out.volume < one - 1.5, "{} vs {one}", out.volume);
        assert!(out.volume > 6.5, "covers the shape: {}", out.volume);
        assert!(out.error > 0.0 && out.error < 0.2);
        assert!(out.hulls.iter().all(|h| h.len() <= 32));
    }

    #[test]
    fn the_hull_budget_is_respected() {
        let settings = Decompose {
            max_hulls: 1,
            ..Default::default()
        };
        let out = decompose(&l_shape(), &settings, &CookControl::new()).unwrap();
        assert_eq!(out.hulls.len(), 1);
    }

    #[test]
    fn the_same_mesh_decomposes_identically_and_can_be_cancelled() {
        let a = decompose(&l_shape(), &Decompose::default(), &CookControl::new()).unwrap();
        let b = decompose(&l_shape(), &Decompose::default(), &CookControl::new()).unwrap();
        assert_eq!(a, b);
        let control = CookControl::new();
        control.cancel();
        assert_eq!(
            decompose(&l_shape(), &Decompose::default(), &control).unwrap_err(),
            CookError::Cancelled
        );
    }

    #[test]
    fn settings_are_checked() {
        assert!(Decompose::default().check().is_ok());
        for bad in [
            Decompose {
                max_hulls: 0,
                ..Default::default()
            },
            Decompose {
                max_hull_vertices: 2,
                ..Default::default()
            },
            Decompose {
                concavity: 2.0,
                ..Default::default()
            },
            Decompose {
                resolution: 2,
                ..Default::default()
            },
        ] {
            assert!(bad.check().is_err());
        }
    }
}
