//! Ray casts through the cells: a grid walk (Amanatides and Woo) in cell
//! units, so every cell the ray crosses is visited once, in order.

use crate::grid::Grid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// The first solid cell the ray enters.
    pub cell: [i32; 3],
    /// The face it entered through, pointing back at the ray (zero when the
    /// ray starts inside the cell).
    pub normal: [i32; 3],
    /// How far along the ray, in cell units.
    pub distance: f32,
}

impl Hit {
    /// The empty cell the ray was in just before the hit.
    pub fn before(&self) -> [i32; 3] {
        [0, 1, 2].map(|a| self.cell[a] + self.normal[a])
    }
}

/// Casts from `origin` along `dir` (both in cell units, `dir` need not be
/// unit length) for at most `reach` cells of distance.
pub fn cast(grid: &Grid, origin: [f32; 3], dir: [f32; 3], reach: f32) -> Option<Hit> {
    walk(
        origin,
        dir,
        reach,
        [0; 3],
        grid.size(),
        |cell, t, _, normal, _| {
            (grid.get(cell) != 0).then_some(Hit {
                cell,
                normal,
                distance: t,
            })
        },
    )
}

fn walk(
    origin: [f32; 3],
    dir: [f32; 3],
    reach: f32,
    lo: [i32; 3],
    hi: [i32; 3],
    mut visit: impl FnMut([i32; 3], f32, f32, [i32; 3], [f32; 3]) -> Option<Hit>,
) -> Option<Hit> {
    let len = dir.iter().map(|d| d * d).sum::<f32>().sqrt();
    if origin.iter().any(|v| !v.is_finite())
        || !len.is_finite()
        || len <= 0.0
        || !reach.is_finite()
        || reach <= 0.0
    {
        return None;
    }
    let dir = dir.map(|d| d / len);
    let size = hi.map(|s| s as f32);
    // Clip to the world's box, so a ray from outside still finds it.
    let (mut near, mut far) = (0.0f32, reach);
    for a in 0..3 {
        if dir[a].abs() < 1e-9 {
            if origin[a] < lo[a] as f32 || origin[a] >= size[a] {
                return None;
            }
            continue;
        }
        let (t0, t1) = (
            (lo[a] as f32 - origin[a]) / dir[a],
            (size[a] - origin[a]) / dir[a],
        );
        near = near.max(t0.min(t1));
        far = far.min(t0.max(t1));
    }
    if near > far {
        return None;
    }
    // Start a hair inside so the first cell is the one the ray enters.
    let start = [0, 1, 2].map(|a| origin[a] + dir[a] * (near + 1e-4));
    let max = hi.map(|s| s - 1);
    let mut cell = [0, 1, 2].map(|a| (start[a].floor() as i32).clamp(lo[a], max[a]));
    let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
    let mut next = [0.0f32; 3];
    let mut delta = [f32::INFINITY; 3];
    for a in 0..3 {
        if dir[a].abs() >= 1e-9 {
            let edge = if step[a] > 0 { cell[a] + 1 } else { cell[a] } as f32;
            next[a] = (edge - origin[a]) / dir[a];
            delta[a] = 1.0 / dir[a].abs();
        } else {
            next[a] = f32::INFINITY;
        }
    }
    let mut t = near;
    let mut normal = [0; 3];
    loop {
        let end = next.into_iter().fold(far, f32::min);
        if let Some(hit) = visit(cell, t, end, normal, dir) {
            return Some(hit);
        }
        let axis = (0..3)
            .min_by(|&a, &b| next[a].total_cmp(&next[b]))
            .expect("three axes");
        t = next[axis];
        if t > far {
            return None;
        }
        cell[axis] += step[axis];
        if !(0..3).all(|a| (lo[a]..hi[a]).contains(&cell[a])) {
            return None;
        }
        next[axis] += delta[axis];
        normal = [0; 3];
        normal[axis] = -step[axis];
    }
}

/// Smooth queries intersect the canonical surface and shaped-cell faces.
/// The grid walk bounds work to the lattice cubes the ray actually crosses.
pub fn cast_smooth(grid: &Grid, origin: [f32; 3], dir: [f32; 3], reach: f32) -> Option<Hit> {
    walk(
        origin.map(|v| v - 0.5),
        dir,
        reach,
        [-1; 3],
        grid.size(),
        |cell, start, end, _, dir| {
            let mut hit: Option<Hit> = None;
            let mut test = |points, normal: [f32; 3], solid| {
                let Some(distance) = crate::smooth::intersect(points, origin, dir) else {
                    return;
                };
                if distance < start - 1e-4 || distance > end + 1e-4 || distance > reach {
                    return;
                }
                if hit.is_some_and(|h| h.distance <= distance) {
                    return;
                }
                let axis = (0..3)
                    .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
                    .unwrap();
                let mut face = [0; 3];
                face[axis] = if normal[axis] > 0.0 { 1 } else { -1 };
                hit = Some(Hit {
                    cell: solid,
                    normal: face,
                    distance,
                });
            };
            crate::smooth::triangles(grid, cell, |t| test(t.points, t.normal, t.cell));
            // A shaped cell can overlap eight cell-centre lattice cubes.
            for z in 0..=1 {
                for y in 0..=1 {
                    for x in 0..=1 {
                        let c = [cell[0] + x, cell[1] + y, cell[2] + z];
                        let shape = grid.shape_at(c);
                        if shape == crate::grid::Shape::Cube {
                            continue;
                        }
                        for face in shape.faces() {
                            let p: Vec<_> = face
                                .points
                                .iter()
                                .map(|p| [0, 1, 2].map(|a| c[a] as f32 + p[a]))
                                .collect();
                            for i in 1..p.len() - 1 {
                                test([p[0], p[i], p[i + 1]], face.normal, c);
                            }
                        }
                    }
                }
            }
            hit
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::STONE;

    fn floor() -> Grid {
        let mut grid = Grid::new([32, 32, 32]);
        for x in 0..32 {
            for z in 0..32 {
                grid.set([x, 4, z], STONE);
            }
        }
        grid
    }

    #[test]
    fn a_ray_down_hits_the_floor_top() {
        let hit = cast(&floor(), [5.5, 20.0, 7.5], [0.0, -1.0, 0.0], 100.0).unwrap();
        assert_eq!(hit.cell, [5, 4, 7]);
        assert_eq!(hit.normal, [0, 1, 0]);
        assert_eq!(hit.before(), [5, 5, 7]);
        assert!((hit.distance - 15.0).abs() < 0.01, "{}", hit.distance);
    }

    #[test]
    fn reach_cuts_a_ray_short() {
        assert!(cast(&floor(), [5.5, 20.0, 7.5], [0.0, -1.0, 0.0], 10.0).is_none());
        assert!(cast(&floor(), [5.5, 20.0, 7.5], [0.0, -1.0, 0.0], 15.5).is_some());
    }

    #[test]
    fn a_ray_from_below_enters_through_the_underside() {
        let hit = cast(&floor(), [5.5, 0.5, 7.5], [0.0, 1.0, 0.0], 100.0).unwrap();
        assert_eq!(hit.cell, [5, 4, 7]);
        assert_eq!(hit.normal, [0, -1, 0]);
    }

    #[test]
    fn a_slanted_ray_and_a_ray_from_outside_the_world() {
        let hit = cast(&floor(), [-10.0, 20.0, 8.5], [1.0, -1.0, 0.0], 100.0).unwrap();
        // Down 16 from y 20 to the floor's top at y 5: x has moved to 6.
        assert_eq!(hit.cell[1], 4);
        assert_eq!(hit.cell[0], 5);
        assert_eq!(hit.cell[2], 8);
        // Pointing away from the world, nothing.
        assert!(cast(&floor(), [-10.0, 20.0, 8.5], [-1.0, 0.0, 0.0], 100.0).is_none());
    }

    #[test]
    fn a_wall_gives_a_sideways_normal() {
        let mut grid = Grid::new([16, 16, 16]);
        grid.set([10, 3, 3], STONE);
        let hit = cast(&grid, [1.5, 3.5, 3.5], [1.0, 0.0, 0.0], 50.0).unwrap();
        assert_eq!(hit.cell, [10, 3, 3]);
        assert_eq!(hit.normal, [-1, 0, 0]);
        assert_eq!(hit.before(), [9, 3, 3]);
        let back = cast(&grid, [15.5, 3.5, 3.5], [-1.0, 0.0, 0.0], 50.0).unwrap();
        assert_eq!(back.normal, [1, 0, 0]);
    }

    #[test]
    fn starting_inside_a_cell_hits_it_at_once() {
        let hit = cast(&floor(), [5.5, 4.5, 7.5], [0.3, -0.2, 0.9], 10.0).unwrap();
        assert_eq!(hit.cell, [5, 4, 7]);
        assert_eq!(hit.normal, [0, 0, 0]);
        assert_eq!(hit.distance, 0.0);
    }

    #[test]
    fn smooth_rays_follow_fractional_surfaces_and_real_shape_faces() {
        let mut grid = Grid::new([16; 3]);
        for z in 0..16 {
            for x in 0..16 {
                grid.set_density([x, 4, z], -64, STONE);
            }
        }
        let hit = cast_smooth(&grid, [5.5, 10.0, 5.5], [0.0, -2.0, 0.0], 10.0).unwrap();
        assert!((hit.distance - 5.3).abs() < 1e-4, "{hit:?}");
        assert!(cast_smooth(&grid, [5.5, 10.0, 5.5], [0.0, -1.0, 0.0], 5.2).is_none());
        grid.clear();
        grid.set_shaped([5, 4, 5], STONE, crate::grid::Shape::Slab);
        let hit = cast_smooth(&grid, [5.5, 10.0, 5.5], [0.0, -1.0, 0.0], 10.0).unwrap();
        assert_eq!(hit.cell, [5, 4, 5]);
        assert_eq!(hit.distance, 5.5);
        assert!(cast_smooth(&grid, [0.0, 4.75, 5.5], [1.0, 0.0, 0.0], 15.0).is_none());
    }

    #[test]
    fn empty_worlds_and_bad_directions_miss() {
        let grid = Grid::new([16, 16, 16]);
        assert!(cast(&grid, [1.0, 1.0, 1.0], [1.0, 0.2, 0.1], 100.0).is_none());
        assert!(cast(&floor(), [1.0, 9.0, 1.0], [0.0, 0.0, 0.0], 100.0).is_none());
        assert!(cast(&floor(), [1.0, 9.0, 1.0], [0.0, -1.0, 0.0], 0.0).is_none());
        // Parallel to the floor and above it, never meets it.
        assert!(cast(&floor(), [1.0, 9.0, 1.0], [1.0, 0.0, 0.0], 100.0).is_none());
    }
}
