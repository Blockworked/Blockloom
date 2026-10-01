//! The cube mesher: visible faces only, merged into rectangles.
//!
//! A face is drawn where a solid cell borders air (or the edge of the world),
//! and neighbouring faces of one material on one plane become a single quad,
//! so a flat field is a handful of triangles. Faces cross chunk boundaries by
//! asking the grid, so a chunk meshes the same whatever its neighbours hold.
//! Glowing materials go in a group of their own each, since the world gives
//! emission to a whole mesh. Positions are in the chunk's own frame.

use crate::grid::{CHUNK, Grid};
use crate::palette::Palette;
use std::collections::BTreeMap;

/// One mesh's worth of a chunk: the lit surfaces together, or one glowing
/// material.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Group {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub colors: Vec<f32>,
    pub indices: Vec<u32>,
}

/// A rectangle of one material on one plane, in cells.
struct Quad {
    axis: usize,
    sign: i32,
    plane: i32,
    at: [i32; 2],
    size: [i32; 2],
    color: [f32; 3],
}

impl Group {
    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    fn push(&mut self, quad: &Quad, voxel: f32) {
        let (u, v) = ((quad.axis + 1) % 3, (quad.axis + 2) % 3);
        let base = self.positions.len() as u32 / 3;
        let mut normal = [0.0; 3];
        normal[quad.axis] = quad.sign as f32;
        let [w, h] = quad.size;
        for (du, dv) in [(0, 0), (w, 0), (w, h), (0, h)] {
            let mut p = [0.0; 3];
            p[quad.axis] = quad.plane as f32 * voxel;
            p[u] = (quad.at[0] + du) as f32 * voxel;
            p[v] = (quad.at[1] + dv) as f32 * voxel;
            self.positions.extend(p);
            self.normals.extend(normal);
            let [r, g, b] = quad.color;
            self.colors.extend([r, g, b, 1.0]);
        }
        // u x v = axis, so this order is counter-clockwise seen from +axis.
        let order: [u32; 6] = if quad.sign > 0 {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 2, 1, 0, 3, 2]
        };
        self.indices.extend(order.map(|i| base + i));
    }
}

/// A chunk's meshes, keyed by the glowing material (`None` for the lit
/// group). A chunk with nothing to draw gives none.
pub fn mesh_chunk(
    grid: &Grid,
    palette: &Palette,
    chunk: [i32; 3],
    voxel: f32,
) -> BTreeMap<Option<u8>, Group> {
    let mut groups: BTreeMap<Option<u8>, Group> = BTreeMap::new();
    if grid.chunk(chunk).is_none() {
        return groups;
    }
    let base = chunk.map(|c| c * CHUNK);
    let n = CHUNK as usize;
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for sign in [1, -1] {
            for slice in 0..CHUNK {
                let mut mask = vec![0u8; n * n];
                for j in 0..CHUNK {
                    for i in 0..CHUNK {
                        let mut at = [0; 3];
                        at[axis] = slice;
                        at[u] = i;
                        at[v] = j;
                        let cell = [base[0] + at[0], base[1] + at[1], base[2] + at[2]];
                        let material = grid.get(cell);
                        if material == 0 {
                            continue;
                        }
                        let mut next = cell;
                        next[axis] += sign;
                        if grid.get(next) == 0 {
                            mask[j as usize * n + i as usize] = material;
                        }
                    }
                }
                for j in 0..n {
                    let mut i = 0;
                    while i < n {
                        let material = mask[j * n + i];
                        if material == 0 {
                            i += 1;
                            continue;
                        }
                        let mut w = 1;
                        while i + w < n && mask[j * n + i + w] == material {
                            w += 1;
                        }
                        let mut h = 1;
                        while j + h < n && (i..i + w).all(|k| mask[(j + h) * n + k] == material) {
                            h += 1;
                        }
                        for row in j..j + h {
                            mask[row * n + i..row * n + i + w].fill(0);
                        }
                        if let Some(look) = palette.get(material) {
                            let key = (look.emission > 0.0).then_some(material);
                            groups.entry(key).or_default().push(
                                &Quad {
                                    axis,
                                    sign,
                                    plane: slice + i32::from(sign > 0),
                                    at: [i as i32, j as i32],
                                    size: [w as i32, h as i32],
                                    color: look.color,
                                },
                                voxel,
                            );
                        }
                        i += w;
                    }
                }
            }
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{DIRT, GLOW, STONE};

    fn palette() -> Palette {
        Palette::new(&[], &[]).unwrap()
    }

    fn lit(groups: &BTreeMap<Option<u8>, Group>) -> usize {
        groups.get(&None).map_or(0, Group::triangles)
    }

    #[test]
    fn a_lone_cube_is_six_quads() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([4, 4, 4], STONE);
        let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0);
        assert_eq!(lit(&groups), 12);
        let group = &groups[&None];
        assert_eq!(group.positions.len() / 3, 24);
        assert_eq!(group.colors.len() / 4, 24);
    }

    #[test]
    fn touching_cubes_hide_their_shared_faces_and_merge_the_rest() {
        let mut grid = Grid::new([16, 16, 16]);
        for x in 2..6 {
            grid.set([x, 3, 3], STONE);
        }
        // A row of four is still a box: six merged quads.
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0)), 12);
    }

    #[test]
    fn a_flat_floor_is_a_few_triangles() {
        let mut grid = Grid::new([16, 16, 16]);
        for x in 0..16 {
            for z in 0..16 {
                grid.set([x, 0, z], STONE);
            }
        }
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0)), 12);
    }

    #[test]
    fn different_materials_do_not_merge() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([1, 1, 1], STONE);
        grid.set([2, 1, 1], DIRT);
        // Two cubes side by side minus the shared faces: 5 + 5 quads.
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0)), 20);
    }

    #[test]
    fn faces_across_a_chunk_boundary_follow_the_neighbour() {
        let mut grid = Grid::new([32, 16, 16]);
        grid.set([15, 2, 2], STONE);
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0)), 12);
        grid.set([16, 2, 2], STONE);
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0)), 10);
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [1, 0, 0], 1.0)), 10);
    }

    #[test]
    fn the_edge_of_the_world_shows_faces() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([0, 0, 0], STONE);
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0)), 12);
    }

    #[test]
    fn glowing_materials_get_their_own_group() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([1, 1, 1], STONE);
        grid.set([3, 1, 1], GLOW);
        let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0);
        assert_eq!(groups.keys().collect::<Vec<_>>(), [&None, &Some(GLOW)]);
        assert_eq!(groups[&Some(GLOW)].triangles(), 12);
    }

    #[test]
    fn winding_agrees_with_the_normals() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([5, 6, 7], STONE);
        let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 2.0);
        let group = &groups[&None];
        let at = |i: u32| {
            let i = i as usize * 3;
            [
                group.positions[i],
                group.positions[i + 1],
                group.positions[i + 2],
            ]
        };
        for tri in group.indices.chunks(3) {
            let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let n = tri[0] as usize * 3;
            let normal = [group.normals[n], group.normals[n + 1], group.normals[n + 2]];
            let dot: f32 = cross.iter().zip(normal).map(|(c, n)| c * n).sum();
            assert!(dot > 0.0, "triangle {tri:?} faces the wrong way");
        }
        // The cube spans (10..12, 12..14, 14..16) at voxel size 2.
        let max = group.positions.iter().copied().fold(f32::MIN, f32::max);
        assert_eq!(max, 16.0);
    }

    #[test]
    fn an_empty_chunk_has_no_meshes() {
        let grid = Grid::new([16, 16, 16]);
        assert!(mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0).is_empty());
        assert!(mesh_chunk(&grid, &palette(), [5, 5, 5], 1.0).is_empty());
    }
}
