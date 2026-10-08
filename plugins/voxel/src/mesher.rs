//! The cube mesher: visible faces only, merged into rectangles.
//!
//! A face is drawn where a solid cell borders air (or the edge of the world),
//! and neighbouring faces of one material on one plane become a single quad,
//! so a flat field is a handful of triangles. Faces cross chunk boundaries by
//! asking the grid, so a chunk meshes the same whatever its neighbours hold.
//! Glowing materials go in a group of their own each, since the world gives
//! emission to a whole mesh. Positions are in the chunk's own frame.

use crate::grid::{Grid, SECTION};
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
    /// One uv per vertex when the group draws a texture, else empty.
    pub uvs: Vec<f32>,
    /// Project asset path of that texture, if any.
    pub texture: Option<String>,
}

/// A rectangle of one material on one plane, in cells.
pub(crate) struct Quad {
    pub(crate) axis: usize,
    pub(crate) sign: i32,
    pub(crate) plane: i32,
    pub(crate) at: [i32; 2],
    pub(crate) size: [i32; 2],
    pub(crate) color: [f32; 3],
}

impl Group {
    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    fn push(&mut self, quad: &Quad, voxel: f32) {
        let [w, h] = quad.size;
        let (a, s) = (quad.at, quad.size);
        self.push_rect(
            quad.axis,
            quad.sign,
            quad.plane as f32,
            [a[0] as f32, a[1] as f32],
            [(a[0] + w) as f32, (a[1] + s[1].min(h)) as f32],
            quad.color,
            voxel,
        );
    }

    /// A flat convex polygon of three or more points, wound counter-clockwise
    /// seen from the side `normal` points to. Positions are already scaled.
    pub(crate) fn push_poly(&mut self, points: &[[f32; 3]], normal: [f32; 3], color: [f32; 3]) {
        let base = self.positions.len() as u32 / 3;
        for p in points {
            self.positions.extend(p);
            self.normals.extend(normal);
            let [r, g, b] = color;
            self.colors.extend([r, g, b, 1.0]);
        }
        if self.texture.is_some() {
            // Planar uvs across the polygon, projected along the face
            // normal so box faces read the texture square.
            let ax = normal[0].abs();
            let ay = normal[1].abs();
            for p in points {
                let (u, v) = if ay >= ax && ay >= normal[2].abs() {
                    (p[0], p[2])
                } else if ax >= normal[2].abs() {
                    (p[2], p[1])
                } else {
                    (p[0], p[1])
                };
                self.uvs.extend([u - u.floor(), v - v.floor()]);
            }
        }
        let (a, b, c) = (points[0], points[1], points[2]);
        let (e1, e2) = (
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        );
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let facing = cross[0] * normal[0] + cross[1] * normal[1] + cross[2] * normal[2];
        for i in 1..points.len() as u32 - 1 {
            let tri = if facing >= 0.0 {
                [0, i, i + 1]
            } else {
                [0, i + 1, i]
            };
            self.indices.extend(tri.map(|t| base + t));
        }
    }

    /// A rectangle on the plane `plane` across `axis` (all in cells from the
    /// chunk's corner), spanning `lo` to `hi` on the next two axes.
    #[allow(clippy::too_many_arguments)]
    fn push_rect(
        &mut self,
        axis: usize,
        sign: i32,
        plane: f32,
        lo: [f32; 2],
        hi: [f32; 2],
        color: [f32; 3],
        voxel: f32,
    ) {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let base = self.positions.len() as u32 / 3;
        let mut normal = [0.0; 3];
        normal[axis] = sign as f32;
        for (pu, pv) in [
            (lo[0], lo[1]),
            (hi[0], lo[1]),
            (hi[0], hi[1]),
            (lo[0], hi[1]),
        ] {
            let mut p = [0.0; 3];
            p[axis] = plane * voxel;
            p[u] = pu * voxel;
            p[v] = pv * voxel;
            self.positions.extend(p);
            self.normals.extend(normal);
            let [r, g, b] = color;
            self.colors.extend([r, g, b, 1.0]);
        }
        // One uv per corner when the group is textured; merged quads span
        // 0..1 per quad so a tiled texture repeats per face.
        if self.texture.is_some() {
            self.uvs.extend([0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
        }
        // u x v = axis, so this order is counter-clockwise seen from +axis.
        let order: [u32; 6] = if sign > 0 {
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
    mesh_region(
        grid,
        palette,
        chunk.map(|c| c * SECTION),
        [SECTION; 3],
        voxel,
    )
}

pub(crate) fn mesh_region(
    grid: &Grid,
    palette: &Palette,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
) -> BTreeMap<Option<u8>, Group> {
    // Without a registry no face is textured, so the texture key is empty.
    mesh_region_with(grid, palette, None, base, extent, voxel)
        .into_iter()
        .map(|((glow, _), group)| (glow, group))
        .collect()
}

/// Same as `mesh_region`, with per-face block textures from the registry
/// when given. Groups are keyed by glowing material and texture path, so
/// faces with different textures never merge into one mesh.
pub(crate) fn mesh_region_with(
    grid: &Grid,
    palette: &Palette,
    registry: Option<&crate::registry::Registry>,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
) -> BTreeMap<(Option<u8>, Option<String>), Group> {
    // Sample the tile and its face halo once, instead of looking up each
    // voxel repeatedly for all six face directions.
    let width = extent.map(|n| (n + 2) as usize);
    let mut cells = Vec::with_capacity(width.iter().product());
    for z in 0..width[2] {
        for y in 0..width[1] {
            for x in 0..width[0] {
                let cell = [
                    base[0] + x as i32 - 1,
                    base[1] + y as i32 - 1,
                    base[2] + z as i32 - 1,
                ];
                let material = grid.get(cell);
                cells.push(
                    if material != 0 && grid.shape_at(cell) == crate::shape::Shape::Cube {
                        material
                    } else {
                        0
                    },
                );
            }
        }
    }
    let mut groups = mesh_cubes_with(palette, registry, base, extent, voxel, |cell| {
        let at = [0, 1, 2].map(|a| (cell[a] - base[a] + 1) as usize);
        cells[(at[2] * width[1] + at[1]) * width[0] + at[0]]
    });
    mesh_shapes_textured(
        grid,
        palette,
        registry,
        base,
        extent,
        voxel,
        false,
        &mut groups,
    );
    // Custom mesh models draw over their cell's cube faces.
    if let Some(reg) = registry {
        emit_custom_models(grid, palette, reg, base, extent, voxel, &mut groups);
    }
    groups
}

/// The face a quad points to: top, bottom or side.
pub(crate) fn face_name(axis: usize, sign: i32) -> &'static str {
    if axis == 1 {
        if sign > 0 { "top" } else { "bottom" }
    } else {
        "side"
    }
}

pub(crate) fn mesh_cubes(
    palette: &Palette,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
    material_at: impl Fn([i32; 3]) -> u8,
) -> BTreeMap<Option<u8>, Group> {
    let mut groups: BTreeMap<Option<u8>, Group> = BTreeMap::new();
    cube_quads(palette, base, extent, material_at, |quad, material| {
        let look = palette.get(material).unwrap();
        let key = (look.emission > 0.0).then_some(material);
        groups.entry(key).or_default().push(&quad, voxel);
    });
    groups
}

/// Same as `mesh_cubes`, with one group per (material, face texture).
pub(crate) fn mesh_cubes_with(
    palette: &Palette,
    registry: Option<&crate::registry::Registry>,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
    material_at: impl Fn([i32; 3]) -> u8,
) -> BTreeMap<(Option<u8>, Option<String>), Group> {
    let mut groups: BTreeMap<(Option<u8>, Option<String>), Group> = BTreeMap::new();
    cube_quads(palette, base, extent, material_at, |quad, material| {
        let look = palette.get(material).unwrap();
        let key = (look.emission > 0.0).then_some(material);
        let texture = registry
            .and_then(|reg| reg.get(material))
            .and_then(|def| def.texture_face(face_name(quad.axis, quad.sign)))
            .map(str::to_string);
        let group = groups.entry((key, texture.clone())).or_default();
        if group.texture.is_none() {
            group.texture = texture;
        }
        group.push(&quad, voxel);
    });
    groups
}

pub(crate) fn cube_quads(
    palette: &Palette,
    base: [i32; 3],
    extent: [i32; 3],
    material_at: impl Fn([i32; 3]) -> u8,
    mut emit: impl FnMut(Quad, u8),
) {
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for sign in [1, -1] {
            let n = extent[u] as usize;
            let rows = extent[v] as usize;
            let mut mask = vec![0u8; n * rows];
            for slice in 0..extent[axis] {
                mask.fill(0);
                for j in 0..extent[v] {
                    for i in 0..extent[u] {
                        let mut at = [0; 3];
                        at[axis] = slice;
                        at[u] = i;
                        at[v] = j;
                        let cell = [base[0] + at[0], base[1] + at[1], base[2] + at[2]];
                        let material = material_at(cell);
                        if material == 0 {
                            continue;
                        }
                        let mut next = cell;
                        next[axis] += sign;
                        if material_at(next) == 0 {
                            mask[j as usize * n + i as usize] = material;
                        }
                    }
                }
                for j in 0..rows {
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
                        while j + h < rows && (i..i + w).all(|k| mask[(j + h) * n + k] == material)
                        {
                            h += 1;
                        }
                        for row in j..j + h {
                            mask[row * n + i..row * n + i + w].fill(0);
                        }
                        if let Some(look) = palette.get(material) {
                            emit(
                                Quad {
                                    axis,
                                    sign,
                                    plane: slice + i32::from(sign > 0),
                                    at: [i as i32, j as i32],
                                    size: [w as i32, h as i32],
                                    color: look.color,
                                },
                                material,
                            );
                        }
                        i += w;
                    }
                }
            }
        }
    }
}

pub(crate) fn mesh_shapes(
    grid: &Grid,
    palette: &Palette,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
    smooth: bool,
    groups: &mut BTreeMap<Option<u8>, Group>,
) {
    mesh_shapes_inner(
        grid,
        palette,
        None,
        base,
        extent,
        voxel,
        smooth,
        &mut TupleGroups::Plain(groups),
    );
}

/// The face a normal points to: top, bottom or side.
pub(crate) fn face_for_normal(normal: [f32; 3]) -> &'static str {
    let ax = normal[0].abs();
    let ay = normal[1].abs();
    let az = normal[2].abs();
    if ay >= ax && ay >= az {
        if normal[1] >= 0.0 { "top" } else { "bottom" }
    } else {
        "side"
    }
}

enum TupleGroups<'a> {
    Plain(&'a mut BTreeMap<Option<u8>, Group>),
    Textured(&'a mut BTreeMap<(Option<u8>, Option<String>), Group>),
}

#[allow(clippy::too_many_arguments)]
fn mesh_shapes_inner(
    grid: &Grid,
    palette: &Palette,
    registry: Option<&crate::registry::Registry>,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
    smooth: bool,
    groups: &mut TupleGroups,
) {
    for (cell, shape) in grid.shaped_in(base.map(|c| c.div_euclid(SECTION))) {
        if !(0..3).all(|a| (base[a]..base[a] + extent[a]).contains(&cell[a])) {
            continue;
        }
        let id = grid.get(cell);
        let Some(look) = palette.get(id) else {
            continue;
        };
        let glow = (look.emission > 0.0).then_some(id);
        let at = [0, 1, 2].map(|a| (cell[a] - base[a]) as f32);
        for face in shape.faces() {
            if let Some(e) = face.edge
                && !smooth
                && grid.is_full([cell[0] + e[0], cell[1] + e[1], cell[2] + e[2]])
            {
                continue;
            }
            let points: Vec<[f32; 3]> = face
                .points
                .iter()
                .map(|p| [0, 1, 2].map(|a| (at[a] + p[a]) * voxel))
                .collect();
            match &mut *groups {
                TupleGroups::Plain(groups) => {
                    groups
                        .entry(glow)
                        .or_default()
                        .push_poly(&points, face.normal, look.color);
                }
                TupleGroups::Textured(groups) => {
                    let texture = registry
                        .and_then(|reg| reg.get(id))
                        .and_then(|def| def.texture_face(face_for_normal(face.normal)))
                        .map(str::to_string);
                    let group = groups.entry((glow, texture.clone())).or_default();
                    if group.texture.is_none() {
                        group.texture = texture;
                    }
                    group.push_poly(&points, face.normal, look.color);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn mesh_shapes_textured(
    grid: &Grid,
    palette: &Palette,
    registry: Option<&crate::registry::Registry>,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
    smooth: bool,
    groups: &mut BTreeMap<(Option<u8>, Option<String>), Group>,
) {
    mesh_shapes_inner(
        grid,
        palette,
        registry,
        base,
        extent,
        voxel,
        smooth,
        &mut TupleGroups::Textured(groups),
    );
}

/// Custom mesh models from the registry draw over their cell's cube faces.
/// Shaped cells keep their shape path above; only whole cubes with a
/// `mesh` model land here.
pub(crate) fn emit_custom_models(
    grid: &Grid,
    palette: &Palette,
    registry: &crate::registry::Registry,
    base: [i32; 3],
    extent: [i32; 3],
    voxel: f32,
    groups: &mut BTreeMap<(Option<u8>, Option<String>), Group>,
) {
    use crate::registry::BlockModel;
    for z in 0..extent[2] {
        for y in 0..extent[1] {
            for x in 0..extent[0] {
                let cell = [base[0] + x, base[1] + y, base[2] + z];
                let id = grid.get(cell);
                if id == 0 || grid.shape_at(cell) != crate::grid::Shape::Cube {
                    continue;
                }
                let (model, color, emission, texture) = match registry.get(id) {
                    Some(def) => match &def.model {
                        BlockModel::Mesh {
                            positions,
                            normals,
                            indices,
                            uvs,
                        } => (
                            Some((
                                positions.clone(),
                                normals.clone(),
                                indices.clone(),
                                uvs.clone(),
                            )),
                            palette.get(id).map(|m| m.color).unwrap_or([1.0; 3]),
                            palette.get(id).map(|m| m.emission).unwrap_or(0.0),
                            def.texture_face("all")
                                .or_else(|| def.texture_face("side"))
                                .map(str::to_string),
                        ),
                        _ => continue,
                    },
                    None => continue,
                };
                let Some((positions, normals, indices, uvs)) = model else {
                    continue;
                };
                // The cube faces stay underneath (they hide inside the custom
                // mesh when it fills the cell); custom tris draw over them.
                let key = ((emission > 0.0).then_some(id), texture.clone());
                let at = [x as f32, y as f32, z as f32];
                let group = groups.entry(key).or_default();
                if group.texture.is_none() {
                    group.texture = texture;
                }
                let textured = group.texture.is_some();
                let base_index = group.positions.len() as u32 / 3;
                for (vi, (p, n)) in positions.iter().zip(normals.iter()).enumerate() {
                    group.positions.extend([
                        (at[0] + p[0]) * voxel,
                        (at[1] + p[1]) * voxel,
                        (at[2] + p[2]) * voxel,
                    ]);
                    group.normals.extend(*n);
                    group.colors.extend([color[0], color[1], color[2], 1.0]);
                    if textured {
                        if let Some(uv) = uvs.get(vi) {
                            group.uvs.extend(*uv);
                        } else {
                            group.uvs.extend([p[0], p[1]]);
                        }
                    }
                }
                group.indices.extend(indices.iter().map(|i| base_index + i));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Shape;
    use crate::palette::{DIRT, GLOW, STONE};

    fn palette() -> Palette {
        Palette::new(&[], &[]).unwrap()
    }

    fn lit(groups: &BTreeMap<Option<u8>, Group>) -> usize {
        groups.get(&None).map_or(0, Group::triangles)
    }

    #[test]
    fn cached_halo_preserves_canonical_faces_and_shapes() {
        let mut grid = Grid::new([33, 25, 31]);
        grid.stream("hills", 42);
        grid.set_shaped([16, 12, 16], STONE, Shape::Slab);
        grid.set([15, 12, 16], GLOW);
        let palette = palette();
        for base in [[0; 3], [8; 3], [16; 3]] {
            let extent = [0, 1, 2].map(|a| (grid.size()[a] - base[a]).min(16));
            let mut reference = mesh_cubes_with(&palette, None, base, extent, 0.7, |cell| {
                if grid.is_full(cell) {
                    grid.get(cell)
                } else {
                    0
                }
            });
            mesh_shapes_textured(
                &grid,
                &palette,
                None,
                base,
                extent,
                0.7,
                false,
                &mut reference,
            );
            assert_eq!(
                mesh_region_with(&grid, &palette, None, base, extent, 0.7),
                reference
            );
        }
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
    fn a_slab_is_a_half_height_box_that_stays_visible_over_a_cube() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set_shaped([4, 4, 4], STONE, Shape::Slab);
        let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0);
        assert_eq!(lit(&groups), 12);
        let ys: Vec<f32> = groups[&None].positions.chunks(3).map(|p| p[1]).collect();
        let (lo, hi) = (
            ys.iter().cloned().fold(f32::MAX, f32::min),
            ys.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert_eq!((lo, hi), (4.0, 4.5));
        // The cube beside it draws every face (a slab is no wall), and the
        // slab's side against the cube is hidden: 6 + 5 faces.
        grid.set([5, 4, 4], STONE);
        let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0);
        assert_eq!(lit(&groups), 22);
    }

    #[test]
    fn a_post_has_its_sides_inset() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set_shaped([2, 2, 2], STONE, Shape::Post);
        let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 2.0);
        assert_eq!(lit(&groups), 12);
        let xs: Vec<f32> = groups[&None].positions.chunks(3).map(|p| p[0]).collect();
        assert!(xs.iter().all(|&x| x == 4.5 || x == 5.5), "{xs:?}");
        // A cube on top hides the post's top, but its own underside still
        // shows around the post: 5 + 6 faces.
        grid.set([2, 3, 2], STONE);
        let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0);
        assert_eq!(lit(&groups), 22);
    }

    #[test]
    fn stairs_and_ramps_mesh_with_normals_that_match_their_winding() {
        use crate::shape::Facing;
        for (shape, tris) in [
            (Shape::Stair(Facing::West), 22),
            (Shape::Ramp(Facing::North), 8),
        ] {
            let mut grid = Grid::new([16, 16, 16]);
            grid.set_shaped([3, 3, 3], STONE, shape);
            let groups = mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0);
            let g = &groups[&None];
            assert_eq!(g.triangles(), tris, "{shape:?}");
            for t in g.indices.chunks(3) {
                let p = |i: u32| {
                    let i = i as usize * 3;
                    [g.positions[i], g.positions[i + 1], g.positions[i + 2]]
                };
                let n = &g.normals[t[0] as usize * 3..t[0] as usize * 3 + 3];
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let (u, v) = (
                    [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                    [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                );
                let cross = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                assert!(
                    cross[0] * n[0] + cross[1] * n[1] + cross[2] * n[2] > 0.0,
                    "{shape:?}"
                );
            }
        }
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
        let mut grid = Grid::new([64, 16, 16]);
        grid.set([31, 2, 2], STONE);
        assert_eq!(lit(&mesh_chunk(&grid, &palette(), [0, 0, 0], 1.0)), 12);
        grid.set([32, 2, 2], STONE);
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

    fn textured_registry() -> crate::registry::Registry {
        crate::registry::Registry::parse(&[
            r##"{"name":"sod","id":8,"textures":{"top":"top.png","side":"side.png","bottom":"bottom.png"}}"##
                .to_string(),
        ])
        .unwrap()
    }

    fn textured_palette() -> Palette {
        Palette::new(
            &[
                "#7d7f85", "#7a5434", "#4f9a3a", "#d9c88c", "#6b4a2b", "#2f7a30", "#ffb14a",
                "#6a7a5a", "#ffffff",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
            &[],
        )
        .unwrap()
    }

    #[test]
    fn faces_with_different_textures_never_merge() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([4, 4, 4], 8);
        let reg = textured_registry();
        let groups = mesh_region_with(
            &grid,
            &textured_palette(),
            Some(&reg),
            [0, 0, 0],
            [16, 16, 16],
            1.0,
        );
        // Top, bottom and four sides: three textures, six quads.
        assert_eq!(groups.len(), 3);
        let mut quads = 0;
        for ((glow, texture), group) in &groups {
            assert_eq!(*glow, None);
            let texture = texture.as_deref().unwrap();
            assert!(["top.png", "side.png", "bottom.png"].contains(&texture));
            let verts = group.positions.len() / 3;
            assert_eq!(group.uvs.len(), verts * 2, "{texture}");
            assert_eq!(group.texture.as_deref(), Some(texture));
            assert!(group.uvs.iter().all(|v| (0.0..=1.0).contains(v)));
            quads += group.indices.len() / 6;
        }
        assert_eq!(quads, 6);
    }

    #[test]
    fn an_untextured_block_stays_in_the_plain_group() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([4, 4, 4], STONE);
        grid.set([8, 4, 4], 8);
        let reg = textured_registry();
        let groups = mesh_region_with(
            &grid,
            &textured_palette(),
            Some(&reg),
            [0, 0, 0],
            [16, 16, 16],
            1.0,
        );
        assert!(groups.contains_key(&(None, None)));
        assert!(groups[&(None, None)].uvs.is_empty());
        assert_eq!(groups.len(), 4);
    }

    #[test]
    fn custom_models_draw_their_own_uvs() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([4, 4, 4], 9);
        let reg = crate::registry::Registry::parse(&[
            r##"{"name":"frame","id":9,"textures":{"all":"frame.png"},"model":{"kind":"mesh","positions":[0,0,0, 1,0,0, 0,1,0],"uvs":[0,0, 1,0, 0,1]}}"##
                .to_string(),
        ])
        .unwrap();
        let groups = mesh_region_with(
            &grid,
            &textured_palette(),
            Some(&reg),
            [0, 0, 0],
            [16, 16, 16],
            1.0,
        );
        let key = (None, Some("frame.png".to_string()));
        let group = &groups[&key];
        assert_eq!(group.indices.len(), 3 + 36);
        assert_eq!(group.uvs.len(), group.positions.len() / 3 * 2);
        assert_eq!(
            &group.uvs[group.uvs.len() - 6..],
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]
        );
    }
}
