//! CPU isosurface extraction over shared cell-centre samples. Every cube uses
//! the same six tetrahedra, so ambiguous faces agree across chunk borders.

use crate::grid::{CHUNK, Grid};
use crate::mesher::{Group, mesh_shapes};
use crate::palette::Palette;
use std::collections::BTreeMap;

const CORNERS: [[i32; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [0, 1, 0],
    [1, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [0, 1, 1],
    [1, 1, 1],
];
const TETS: [[usize; 4]; 6] = [
    [0, 1, 3, 7],
    [0, 3, 2, 7],
    [0, 2, 6, 7],
    [0, 6, 4, 7],
    [0, 4, 5, 7],
    [0, 5, 1, 7],
];

pub(crate) struct Triangle {
    pub points: [[f32; 3]; 3],
    pub normal: [f32; 3],
    pub cell: [i32; 3],
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] - b[i])
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

/// Coordinates are global cell units; density and material come from the
/// canonical grid rather than a mesh cache.
pub(crate) fn triangles(grid: &Grid, at: [i32; 3], emit: impl FnMut(Triangle)) {
    let cells = CORNERS.map(|c| [0, 1, 2].map(|a| at[a] + c[a]));
    let density = cells.map(|c| grid.density(c));
    triangles_samples(at, density, emit);
}

fn triangles_samples(at: [i32; 3], density: [i16; 8], mut emit: impl FnMut(Triangle)) {
    let cells = CORNERS.map(|c| [0, 1, 2].map(|a| at[a] + c[a]));
    if density.iter().all(|&d| d > 0) || density.iter().all(|&d| d < 0) {
        return;
    }
    let points = cells.map(|c| c.map(|v| v as f32 + 0.5));
    let edge = |mut a: usize, mut b: usize| {
        // The same edge must interpolate in the same order on either side.
        if cells[a] > cells[b] {
            std::mem::swap(&mut a, &mut b);
        }
        let t = density[a] as f32 / (density[a] as f32 - density[b] as f32);
        [0, 1, 2].map(|i| points[a][i] + t * (points[b][i] - points[a][i]))
    };
    for tet in TETS {
        let (mut inside, mut outside) = ([0; 4], [0; 4]);
        let (mut ni, mut no) = (0, 0);
        for i in tet {
            if density[i] < 0 {
                inside[ni] = i;
                ni += 1;
            } else {
                outside[no] = i;
                no += 1;
            }
        }
        let (polygon, count) = match ni {
            1 => (
                [
                    edge(inside[0], outside[0]),
                    edge(inside[0], outside[1]),
                    edge(inside[0], outside[2]),
                    [0.0; 3],
                ],
                3,
            ),
            2 => (
                [
                    edge(inside[0], outside[0]),
                    edge(inside[0], outside[1]),
                    edge(inside[1], outside[1]),
                    edge(inside[1], outside[0]),
                ],
                4,
            ),
            3 => (
                [
                    edge(inside[0], outside[0]),
                    edge(inside[1], outside[0]),
                    edge(inside[2], outside[0]),
                    [0.0; 3],
                ],
                3,
            ),
            _ => continue,
        };
        let direction = sub(points[outside[0]], points[inside[0]]);
        for i in 1..count - 1 {
            let mut p = [polygon[0], polygon[i], polygon[i + 1]];
            let mut n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            if dot(n, direction) < 0.0 {
                p.swap(1, 2);
                n = n.map(|v| -v);
            }
            let len = dot(n, n).sqrt();
            if len <= 1e-8 {
                continue;
            }
            emit(Triangle {
                points: p,
                normal: n.map(|v| v / len),
                cell: cells[inside[0]],
            });
        }
    }
}

pub fn mesh_chunk(
    grid: &Grid,
    palette: &Palette,
    chunk: [i32; 3],
    voxel: f32,
) -> BTreeMap<Option<u8>, Group> {
    let mut groups = BTreeMap::<Option<u8>, Group>::new();
    let mut vertices = BTreeMap::<Option<u8>, BTreeMap<[u32; 4], u32>>::new();
    let base = chunk.map(|c| c * CHUNK);
    let lo = base.map(|c| if c == 0 { -1 } else { c });
    let hi = base.map(|c| c + CHUNK);
    // Read the shared halo once rather than eight times per lattice cube.
    let sample_lo = lo.map(|v| v - 1);
    let sample_hi = hi.map(|v| v + 1);
    let n = [0, 1, 2].map(|a| (sample_hi[a] - sample_lo[a] + 1) as usize);
    let index = |at: [i32; 3]| {
        let c = [0, 1, 2].map(|a| (at[a] - sample_lo[a]) as usize);
        (c[2] * n[1] + c[1]) * n[0] + c[0]
    };
    let mut samples = vec![0i16; n.iter().product()];
    let mut materials = vec![0u8; samples.len()];
    for z in sample_lo[2]..=sample_hi[2] {
        for y in sample_lo[1]..=sample_hi[1] {
            for x in sample_lo[0]..=sample_hi[0] {
                let at = [x, y, z];
                samples[index(at)] = grid.density(at);
                materials[index(at)] = grid.get(at);
            }
        }
    }
    for z in lo[2]..hi[2] {
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let at = [x, y, z];
                let density = CORNERS.map(|c| samples[index([0, 1, 2].map(|a| at[a] + c[a]))]);
                triangles_samples(at, density, |t| {
                    let material = materials[index(t.cell)];
                    let Some(look) = palette.get(material) else {
                        return;
                    };
                    let p = t
                        .points
                        .map(|p| [0, 1, 2].map(|a| (p[a] - base[a] as f32) * voxel));
                    let key = (look.emission > 0.0).then_some(material);
                    let group = groups.entry(key).or_default();
                    let table = vertices.entry(key).or_default();
                    for (point, world_point) in p.into_iter().zip(t.points) {
                        let key = [
                            material as u32,
                            point[0].to_bits(),
                            point[1].to_bits(),
                            point[2].to_bits(),
                        ];
                        let index = *table.entry(key).or_insert_with(|| {
                            let vertex_index = group.positions.len() as u32 / 3;
                            group.positions.extend(point);
                            // Central gradients use the same halo on both sides of a seam.
                            let cell = world_point.map(|p| (p - 0.5).floor() as i32);
                            let fraction = [0, 1, 2].map(|a| world_point[a] - 0.5 - cell[a] as f32);
                            let mut normal = [0.0; 3];
                            for corner in CORNERS {
                                let sample = [0, 1, 2].map(|a| cell[a] + corner[a]);
                                let weight: f32 = (0..3)
                                    .map(|a| {
                                        if corner[a] == 0 {
                                            1.0 - fraction[a]
                                        } else {
                                            fraction[a]
                                        }
                                    })
                                    .product();
                                if weight == 0.0 {
                                    continue;
                                }
                                for axis in 0..3 {
                                    let mut lower = sample;
                                    lower[axis] -= 1;
                                    let mut upper = sample;
                                    upper[axis] += 1;
                                    normal[axis] += weight
                                        * (samples[index(upper)] as f32
                                            - samples[index(lower)] as f32);
                                }
                            }
                            let length = dot(normal, normal).sqrt();
                            let normal = if length > 1e-8 {
                                normal.map(|v| v / length)
                            } else {
                                t.normal
                            };
                            group.normals.extend(normal);
                            group
                                .colors
                                .extend([look.color[0], look.color[1], look.color[2], 1.0]);
                            vertex_index
                        });
                        group.indices.push(index);
                    }
                });
            }
        }
    }
    mesh_shapes(grid, palette, chunk, voxel, true, &mut groups);
    groups
}

/// Intersection with the exact triangle used by both rendering and collision.
pub(crate) fn intersect(points: [[f32; 3]; 3], origin: [f32; 3], dir: [f32; 3]) -> Option<f32> {
    let e1 = sub(points[1], points[0]);
    let e2 = sub(points[2], points[0]);
    let h = cross(dir, e2);
    let det = dot(e1, h);
    if det.abs() < 1e-8 {
        return None;
    }
    let s = sub(origin, points[0]);
    let u = dot(s, h) / det;
    let q = cross(s, e1);
    let v = dot(dir, q) / det;
    let t = dot(e2, q) / det;
    (u >= -1e-5 && v >= -1e-5 && u + v <= 1.0 + 1e-5 && t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{GLOW, STONE};
    use std::collections::BTreeSet;

    fn check(group: &Group) {
        for tri in group.indices.as_chunks::<3>().0 {
            let normal = &group.normals[tri[0] as usize * 3..tri[0] as usize * 3 + 3];
            let p = [tri[0], tri[1], tri[2]].map(|i| {
                let i = i as usize * 3;
                [
                    group.positions[i],
                    group.positions[i + 1],
                    group.positions[i + 2],
                ]
            });
            let area = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            assert!(dot(area, [normal[0], normal[1], normal[2]]) > 0.0);
            assert!(
                (dot(
                    [normal[0], normal[1], normal[2]],
                    [normal[0], normal[1], normal[2]]
                ) - 1.0)
                    .abs()
                    < 1e-5
            );
        }
    }

    #[test]
    fn surfaces_are_closed_and_outward_even_at_world_edges() {
        let mut grid = Grid::new([16; 3]);
        grid.set([0; 3], STONE);
        let groups = mesh_chunk(&grid, &Palette::new(&[], &[]).unwrap(), [0; 3], 1.0);
        let group = &groups[&None];
        check(group);
        let vertex = |i: u32| {
            let i = i as usize * 3;
            [0, 1, 2].map(|a| (group.positions[i + a] * 10000.0).round() as i32)
        };
        let mut edges = BTreeMap::new();
        for t in group.indices.as_chunks::<3>().0 {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (vertex(a), vertex(b));
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2), "{edges:?}");
        assert!(group.positions.iter().all(|&p| (0.0..=1.0).contains(&p)));
    }

    #[test]
    fn fractional_density_and_diagonal_chunks_share_identical_seams() {
        let mut grid = Grid::new([32; 3]);
        for z in 12..20 {
            for y in 12..20 {
                for x in 12..20 {
                    let distance =
                        ((x - 16i32).pow(2) + (y - 16i32).pow(2) + (z - 16i32).pow(2)) as f32;
                    grid.set_density([x, y, z], ((distance.sqrt() - 3.2) * 256.0) as i16, STONE);
                }
            }
        }
        let palette = Palette::new(&[], &[]).unwrap();
        let a = mesh_chunk(&grid, &palette, [0, 0, 0], 1.0);
        let b = mesh_chunk(&grid, &palette, [1, 0, 0], 1.0);
        let seam = |g: &Group, offset: f32| -> BTreeSet<_> {
            g.positions
                .as_chunks::<3>()
                .0
                .iter()
                .filter(|p| (p[0] + offset - 16.5).abs() < 1e-5)
                .map(|p| {
                    [
                        (p[1] * 10000.0).round() as i32,
                        (p[2] * 10000.0).round() as i32,
                    ]
                })
                .collect()
        };
        // Lattice cubes on either side share the plane through sample 16.
        assert!(!seam(&a[&None], 0.0).is_empty());
        assert_eq!(seam(&a[&None], 0.0), seam(&b[&None], 16.0));
        let normals = |g: &Group, offset: f32| -> BTreeMap<_, _> {
            g.positions
                .as_chunks::<3>()
                .0
                .iter()
                .zip(g.normals.as_chunks::<3>().0)
                .filter(|(p, _)| (p[0] + offset - 16.5).abs() < 1e-5)
                .map(|(p, n)| {
                    (
                        [
                            (p[1] * 10000.0).round() as i32,
                            (p[2] * 10000.0).round() as i32,
                        ],
                        n.map(|v| (v * 10000.0).round() as i32),
                    )
                })
                .collect()
        };
        assert_eq!(normals(&a[&None], 0.0), normals(&b[&None], 16.0));
        check(&a[&None]);
        check(&b[&None]);
        grid.take_dirty();
        grid.set_density([16; 3], -100, STONE);
        assert_eq!(grid.take_dirty().len(), 8);
        assert!(!grid.set_density([16; 3], -100, STONE));
    }

    #[test]
    fn shaped_cells_and_emission_survive_in_a_smooth_world() {
        let mut grid = Grid::new([16; 3]);
        grid.set_shaped([3; 3], STONE, crate::grid::Shape::Slab);
        grid.set([8; 3], GLOW);
        let groups = mesh_chunk(&grid, &Palette::new(&[], &[]).unwrap(), [0; 3], 1.0);
        assert_eq!(groups[&None].triangles(), 12);
        assert!(groups[&Some(GLOW)].triangles() > 12);
        check(&groups[&Some(GLOW)]);
    }
}
