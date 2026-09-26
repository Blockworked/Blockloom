//! Chunked terrain meshes with levels of detail.
//!
//! The grid is cut into a fixed square of chunks, and each chunk has a mesh
//! per level, level `l` taking every `2^l`th sample. A level's geometric
//! error is the furthest any full-resolution sample strays from it, which
//! [`lod_thresholds`] turns into the screen sizes the runtime's one LOD
//! selector works with, for a pixel-error budget. Neighbouring chunks at
//! different levels leave cracks, so every chunk hangs a skirt below its
//! edges, deep enough to cover the worst one. Holes drop whole cells.

use super::{Heightfield, Shape};

/// Quads per chunk side that are always kept in memory; finer levels
/// stream in near the camera.
pub const RESIDENT_QUADS: u32 = 32;
/// The coarsest level still has this many quads per side.
const MIN_QUADS: u32 = 4;
/// The height of a reference viewport, for turning pixels into screen
/// fractions.
pub const REFERENCE_VIEWPORT: f32 = 1080.0;

/// How a grid is cut into chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkLayout {
    pub side: u32,
    /// Chunks per side.
    pub chunks: u32,
    /// Quads per chunk side at level 0.
    pub quads: u32,
    pub levels: u32,
}

impl ChunkLayout {
    /// Sixteen chunks a side, or fewer so a chunk keeps 32 quads.
    pub fn for_side(side: u32) -> Self {
        let total = side.saturating_sub(1).max(MIN_QUADS);
        let quads = (total / 16).max(RESIDENT_QUADS).min(total);
        let chunks = (total / quads).max(1);
        let levels = (quads / MIN_QUADS).max(1).ilog2() + 1;
        Self {
            side,
            chunks,
            quads,
            levels,
        }
    }

    /// The finest level kept loaded however far away the camera is.
    pub fn resident_level(&self) -> u32 {
        (0..self.levels)
            .find(|level| self.quads >> level <= RESIDENT_QUADS)
            .unwrap_or(self.levels - 1)
    }

    /// Sample range `[start, end]` a chunk covers on each axis.
    pub fn span(&self, cx: u32, cz: u32) -> ([u32; 2], [u32; 2]) {
        (
            [cx * self.quads, (cx + 1) * self.quads],
            [cz * self.quads, (cz + 1) * self.quads],
        )
    }
}

/// Mesh data ready to upload. Positions are relative to the chunk's
/// `origin`, which is also where its bounds are centred.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TerrainMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// 0-1 across the whole terrain, for its weight and cavity maps.
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl TerrainMesh {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A chunk's bounds, in metres from the actor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl ChunkBounds {
    pub fn center(&self) -> [f32; 3] {
        std::array::from_fn(|k| (self.min[k] + self.max[k]) * 0.5)
    }

    pub fn radius(&self) -> f32 {
        let half: [f32; 3] = std::array::from_fn(|k| (self.max[k] - self.min[k]) * 0.5);
        (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt()
    }
}

pub fn chunk_bounds(
    field: &Heightfield,
    shape: &Shape,
    layout: &ChunkLayout,
    cx: u32,
    cz: u32,
) -> ChunkBounds {
    let ([i0, i1], [j0, j1]) = layout.span(cx, cz);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let h = field.at(i, j);
            lo = lo.min(h);
            hi = hi.max(h);
        }
    }
    let a = shape.local(i0 as f32, j0 as f32);
    let b = shape.local(i1 as f32, j1 as f32);
    ChunkBounds {
        min: [a[0], lo * shape.height, a[1]],
        max: [b[0], hi * shape.height, b[1]],
    }
}

/// The height level `level` draws at sample `(i, j)`: the triangle it falls
/// in, split the way [`chunk_mesh`] splits every quad.
fn simplified(field: &Heightfield, i: u32, j: u32, step: u32) -> f32 {
    let (qi, qj) = (i / step * step, j / step * step);
    let last = field.side - 1;
    let (qi, qj) = (qi.min(last - step.min(last)), qj.min(last - step.min(last)));
    let s = (i - qi) as f32 / step as f32;
    let t = (j - qj) as f32 / step as f32;
    let a = field.at(qi, qj);
    let b = field.at(qi + step, qj);
    let c = field.at(qi, qj + step);
    let d = field.at(qi + step, qj + step);
    if s + t <= 1.0 {
        a + s * (b - a) + t * (c - a)
    } else {
        d + (1.0 - s) * (c - d) + (1.0 - t) * (b - d)
    }
}

/// Metres each level of a chunk strays from the full grid at worst, never
/// less than the level before it.
pub fn chunk_errors(
    field: &Heightfield,
    shape: &Shape,
    layout: &ChunkLayout,
    cx: u32,
    cz: u32,
) -> Vec<f32> {
    let ([i0, i1], [j0, j1]) = layout.span(cx, cz);
    let mut errors = Vec::with_capacity(layout.levels as usize);
    let mut worst = 0.0f32;
    for level in 0..layout.levels {
        let step = 1 << level;
        if level > 0 {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let e = (field.at(i, j) - simplified(field, i, j, step)).abs();
                    worst = worst.max(e);
                }
            }
        }
        errors.push(worst * shape.height);
    }
    errors
}

/// Screen sizes for each level (diameter over viewport height, the runtime
/// selector's measure): level `l` is drawn from its threshold up to the one
/// before it, so the coarsest level whose error stays under `pixel_error`
/// pixels always wins. The last level never culls.
pub fn lod_thresholds(errors: &[f32], radius: f32, pixel_error: f32) -> Vec<f32> {
    // Projected error = error * viewport * screen / diameter.
    let limit = |error: f32| {
        if error <= 1e-6 {
            f32::INFINITY
        } else {
            pixel_error * 2.0 * radius / (error * REFERENCE_VIEWPORT)
        }
    };
    let mut out: Vec<f32> = (0..errors.len())
        .map(|level| errors.get(level + 1).map_or(0.0, |&next| limit(next)))
        .collect();
    if let Some(last) = out.last_mut() {
        *last = 0.0;
    }
    out
}

/// Builds one chunk at one level. `holes` is one byte per sample, cutting
/// the cell whose north-west corner it sits on; `skirt` is how far the
/// edges hang down.
#[allow(clippy::too_many_arguments)]
pub fn chunk_mesh(
    field: &Heightfield,
    shape: &Shape,
    layout: &ChunkLayout,
    cx: u32,
    cz: u32,
    level: u32,
    holes: Option<&[u8]>,
    skirt: f32,
) -> (TerrainMesh, [f32; 3]) {
    let step = 1u32 << level.min(layout.levels - 1);
    let ([i0, i1], [j0, _]) = layout.span(cx, cz);
    let origin = chunk_bounds(field, shape, layout, cx, cz).center();
    let n = (i1 - i0) / step + 1;
    let last = (shape.side - 1) as f32;
    let mut mesh = TerrainMesh::default();
    let vertex = |mesh: &mut TerrainMesh, i: u32, j: u32, drop: f32| {
        let [x, z] = shape.local(i as f32, j as f32);
        let y = field.at(i, j) * shape.height - drop;
        mesh.positions
            .push([x - origin[0], y - origin[1], z - origin[2]]);
        mesh.normals.push(field.normal(shape, i, j));
        mesh.uvs.push([i as f32 / last, j as f32 / last]);
    };
    for gj in 0..n {
        for gi in 0..n {
            vertex(&mut mesh, i0 + gi * step, j0 + gj * step, 0.0);
        }
    }
    let cut = |qi: u32, qj: u32| {
        let Some(holes) = holes else {
            return false;
        };
        (qj..qj + step).any(|j| (qi..qi + step).any(|i| holes[(j * shape.side + i) as usize] > 127))
    };
    for gj in 0..n - 1 {
        for gi in 0..n - 1 {
            if cut(i0 + gi * step, j0 + gj * step) {
                continue;
            }
            let a = gj * n + gi;
            let b = a + 1;
            let c = a + n;
            let d = c + 1;
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    if skirt > 0.0 {
        // Each edge walked so the skirt faces outwards: its top edge runs
        // clockwise seen from above.
        let edges: [Vec<u32>; 4] = [
            // North east to west, west north to south, south west to east,
            // east south to north.
            (0..n).rev().collect(),
            (0..n).map(|g| g * n).collect(),
            (0..n).map(|g| (n - 1) * n + g).collect(),
            (0..n).rev().map(|g| g * n + n - 1).collect(),
        ];
        for edge in edges {
            let base = mesh.positions.len() as u32;
            for &top in &edge {
                let gi = top % n;
                let gj = top / n;
                vertex(&mut mesh, i0 + gi * step, j0 + gj * step, skirt);
            }
            for k in 0..edge.len() as u32 - 1 {
                let (t0, t1) = (edge[k as usize], edge[k as usize + 1]);
                let (b0, b1) = (base + k, base + k + 1);
                if holed_edge(&cut, i0, j0, step, n, t0, t1) {
                    continue;
                }
                mesh.indices.extend_from_slice(&[t0, b0, t1, t1, b0, b1]);
            }
        }
    }
    (mesh, origin)
}

/// Whether the cell behind an edge segment is a hole, so its skirt goes too.
fn holed_edge(
    cut: &impl Fn(u32, u32) -> bool,
    i0: u32,
    j0: u32,
    step: u32,
    n: u32,
    t0: u32,
    t1: u32,
) -> bool {
    let (gi, gj) = ((t0 % n).min(t1 % n), (t0 / n).min(t1 / n));
    let gi = gi.min(n - 2);
    let gj = gj.min(n - 2);
    cut(i0 + gi * step, j0 + gj * step)
}

/// How deep a chunk's skirt must hang to hide any crack against a
/// neighbour: the worst error of any level, plus a little.
pub fn skirt_depth(errors: &[f32], shape: &Shape) -> f32 {
    errors.iter().copied().fold(0.0, f32::max) + shape.spacing()[0].min(shape.spacing()[1]) * 0.5
}

/// Heights laid out for a physics heightfield: column-major, a column per
/// X sample, so row `j` of column `i` is sample `(i, j)`, in metres.
pub fn collider_heights(field: &Heightfield, shape: &Shape) -> Vec<f32> {
    let side = field.side;
    let mut out = Vec::with_capacity(field.samples.len());
    for i in 0..side {
        for j in 0..side {
            out.push(field.at(i, j) * shape.height);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bumpy(side: u32) -> Heightfield {
        let mut field = Heightfield::flat(side, 0.0);
        for j in 0..side {
            for i in 0..side {
                let k = field.index(i, j);
                field.samples[k] = 0.5 + 0.25 * ((i as f32 * 0.3).sin() * (j as f32 * 0.2).cos());
            }
        }
        field
    }

    fn shape(side: u32) -> Shape {
        Shape {
            size: [256.0, 256.0],
            height: 40.0,
            side,
        }
    }

    #[test]
    fn layouts_keep_chunk_counts_bounded() {
        let small = ChunkLayout::for_side(129);
        assert_eq!((small.chunks, small.quads), (4, 32));
        assert_eq!(small.resident_level(), 0);
        let big = ChunkLayout::for_side(4097);
        assert_eq!((big.chunks, big.quads), (16, 256));
        assert_eq!(big.levels, 7);
        assert_eq!(big.resident_level(), 3);
        assert_eq!(big.quads >> (big.levels - 1), MIN_QUADS);
    }

    #[test]
    fn a_flat_chunk_has_no_error_and_draws_its_coarsest_level() {
        let side = 129;
        let field = Heightfield::flat(side, 0.3);
        let layout = ChunkLayout::for_side(side);
        let errors = chunk_errors(&field, &shape(side), &layout, 0, 0);
        assert!(errors.iter().all(|e| *e == 0.0));
        let thresholds = lod_thresholds(&errors, 10.0, 4.0);
        assert!(
            thresholds[..thresholds.len() - 1]
                .iter()
                .all(|t| t.is_infinite())
        );
        assert_eq!(*thresholds.last().unwrap(), 0.0);
    }

    #[test]
    fn errors_grow_with_level_and_thresholds_fall() {
        let side = 257;
        let field = bumpy(side);
        let layout = ChunkLayout::for_side(side);
        let errors = chunk_errors(&field, &shape(side), &layout, 1, 2);
        assert_eq!(errors[0], 0.0);
        assert!(errors.windows(2).all(|w| w[0] <= w[1]));
        assert!(*errors.last().unwrap() > 0.1);
        let thresholds = lod_thresholds(&errors, 20.0, 4.0);
        assert!(
            thresholds.windows(2).all(|w| w[0] >= w[1]),
            "{thresholds:?}"
        );
    }

    #[test]
    fn a_chunk_mesh_faces_up_with_outward_skirts() {
        let side = 129;
        let field = bumpy(side);
        let s = shape(side);
        let layout = ChunkLayout::for_side(side);
        let (mesh, origin) = chunk_mesh(&field, &s, &layout, 1, 1, 0, None, 2.0);
        let n = layout.quads + 1;
        let top = (n * n) as usize;
        let surface = (layout.quads * layout.quads * 2) as usize;
        assert_eq!(mesh.triangles(), surface + 4 * 2 * layout.quads as usize);
        let at = |k: u32| glam::Vec3::from(mesh.positions[k as usize]);
        for tri in mesh.indices.chunks(3).take(surface) {
            let normal = (at(tri[1]) - at(tri[0])).cross(at(tri[2]) - at(tri[0]));
            assert!(normal.y > 0.0, "surface faces up");
        }
        let center = glam::Vec3::ZERO;
        for tri in mesh.indices.chunks(3).skip(surface) {
            let normal = (at(tri[1]) - at(tri[0])).cross(at(tri[2]) - at(tri[0]));
            let mid = (at(tri[0]) + at(tri[1]) + at(tri[2])) / 3.0;
            let out = glam::Vec3::new(mid.x, 0.0, mid.z) - center;
            assert!(normal.dot(out) > 0.0, "skirt faces out");
        }
        assert!(
            mesh.positions[top..]
                .iter()
                .all(|p| p[1] + origin[1] < 40.0)
        );
    }

    #[test]
    fn coarser_levels_have_fewer_triangles_and_holes_cut_cells() {
        let side = 129;
        let field = bumpy(side);
        let s = shape(side);
        let layout = ChunkLayout::for_side(side);
        let (fine, _) = chunk_mesh(&field, &s, &layout, 0, 0, 0, None, 0.0);
        let (coarse, _) = chunk_mesh(&field, &s, &layout, 0, 0, 2, None, 0.0);
        assert_eq!(fine.triangles(), 32 * 32 * 2);
        assert_eq!(coarse.triangles(), 8 * 8 * 2);
        let mut holes = vec![0u8; (side * side) as usize];
        holes[(5 * side + 5) as usize] = 255;
        let (cut, _) = chunk_mesh(&field, &s, &layout, 0, 0, 0, Some(&holes), 0.0);
        assert_eq!(cut.triangles(), fine.triangles() - 2);
        let (cut_coarse, _) = chunk_mesh(&field, &s, &layout, 0, 0, 2, Some(&holes), 0.0);
        assert_eq!(cut_coarse.triangles(), coarse.triangles() - 2);
    }

    #[test]
    fn simplified_heights_match_the_mesh_triangulation() {
        let side = 129;
        let field = bumpy(side);
        // On a level's own vertices, the simplification is exact.
        for step in [2, 4, 8] {
            for j in (0..side).step_by(step as usize) {
                for i in (0..side).step_by(step as usize) {
                    assert!((simplified(&field, i, j, step) - field.at(i, j)).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn collider_heights_are_column_major() {
        let side = 129;
        let field = bumpy(side);
        let s = shape(side);
        let heights = collider_heights(&field, &s);
        assert_eq!(heights[(3 * side + 7) as usize], field.at(3, 7) * s.height);
    }
}
