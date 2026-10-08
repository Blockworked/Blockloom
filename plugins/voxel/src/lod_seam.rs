//! Exact tile clipping and planar solid differences at mixed-resolution faces.

use crate::{
    Surface,
    grid::Grid,
    lod_mesh::{Key, TILE},
    mesher::Group,
    palette::Palette,
    shape::Shape,
    smooth,
};
use std::collections::BTreeMap;

pub type Groups = BTreeMap<Option<u8>, Group>;
pub type Caps = [Vec<Cap>; 6];
const EPS: f64 = 1e-9;
pub const MAX_WORK: usize = 2_000_000;

#[derive(Clone, Debug)]
pub struct Cap {
    pub points: Vec<[f64; 2]>,
    pub material: u8,
}

#[derive(Clone)]
pub struct Geometry {
    pub groups: Groups,
    pub caps: Caps,
}
impl Geometry {
    pub fn words(&self) -> usize {
        self.groups
            .values()
            .map(crate::lod_mesh::words)
            .sum::<usize>()
            + cap_words(&self.caps)
    }
}
pub fn cap_words(caps: &Caps) -> usize {
    caps.iter().flatten().map(|c| c.points.len() * 4 + 1).sum()
}

pub fn bounds(key: Key, size: [i32; 3]) -> ([i32; 3], [i32; 3]) {
    let lo = key.tile.map(|c| c * (TILE << key.level));
    (
        lo,
        [0, 1, 2].map(|a| (lo[a] + (TILE << key.level)).min(size[a])),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub other: Key,
    pub face: usize,
    pub lo: [i32; 2],
    pub hi: [i32; 2],
}
pub fn joins(key: Key, keys: &[Key], size: [i32; 3]) -> Vec<Join> {
    let (lo, hi) = bounds(key, size);
    let mut result = Vec::new();
    for &other in keys {
        if key.level == other.level {
            continue;
        }
        let (a, b) = bounds(other, size);
        for axis in 0..3 {
            let sign = if hi[axis] == a[axis] {
                1
            } else if lo[axis] == b[axis] {
                0
            } else {
                continue;
            };
            let uv = [(axis + 1) % 3, (axis + 2) % 3];
            let l = uv.map(|i| lo[i].max(a[i]));
            let h = uv.map(|i| hi[i].min(b[i]));
            if (0..2).all(|i| l[i] < h[i]) {
                result.push(Join {
                    other,
                    face: axis * 2 + sign,
                    lo: l,
                    hi: h,
                });
            }
        }
    }
    result
}

fn cross(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn area(p: &[[f64; 2]]) -> f64 {
    if p.len() < 3 {
        return 0.0;
    }
    (1..p.len() - 1)
        .map(|i| cross(p[0], p[i], p[i + 1]))
        .sum::<f64>()
        * 0.5
}

fn clean(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.dedup_by(|a, b| (0..2).all(|i| (a[i] - b[i]).abs() < EPS));
    if p.len() > 1 && (0..2).all(|i| (p[0][i] - p[p.len() - 1][i]).abs() < EPS) {
        p.pop();
    }
    if p.len() < 3 || area(&p).abs() < EPS {
        return Vec::new();
    }
    if area(&p) < 0.0 {
        p.reverse();
    }
    p
}
fn clip2(p: &[[f64; 2]], a: [f64; 2], b: [f64; 2], inside: bool) -> Vec<[f64; 2]> {
    if p.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..p.len() {
        let x = p[i];
        let y = p[(i + 1) % p.len()];
        let dx = cross(a, b, x);
        let dy = cross(a, b, y);
        let ix = if inside { dx >= -EPS } else { dx <= EPS };
        let iy = if inside { dy >= -EPS } else { dy <= EPS };
        if ix {
            out.push(x);
        }
        if ix != iy {
            let t = dx / (dx - dy);
            out.push([0, 1].map(|k| x[k] + t * (y[k] - x[k])));
        }
    }
    clean(out)
}
fn rect(lo: [i32; 2], hi: [i32; 2]) -> Vec<[f64; 2]> {
    vec![
        [lo[0] as f64, lo[1] as f64],
        [hi[0] as f64, lo[1] as f64],
        [hi[0] as f64, hi[1] as f64],
        [lo[0] as f64, hi[1] as f64],
    ]
}
fn intersect(mut p: Vec<[f64; 2]>, q: &[[f64; 2]]) -> Vec<[f64; 2]> {
    for i in 0..q.len() {
        p = clip2(&p, q[i], q[(i + 1) % q.len()], true);
        if p.is_empty() {
            break;
        }
    }
    p
}
fn overlaps(p: &[[f64; 2]], q: &[[f64; 2]]) -> bool {
    (0..2).all(|a| {
        let range = |p: &[[f64; 2]]| {
            p.iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| {
                    (l.min(p[a]), h.max(p[a]))
                })
        };
        let (l, h) = range(p);
        let (a, b) = range(q);
        h > a + EPS && b > l + EPS
    })
}
fn difference(
    p: Vec<[f64; 2]>,
    q: &[[f64; 2]],
    work: &mut usize,
) -> Result<Vec<Vec<[f64; 2]>>, String> {
    *work += 1;
    if *work > MAX_WORK {
        return Err("visual LOD seam exceeds its clipping work budget".into());
    }
    if !overlaps(&p, q) {
        return Ok(vec![p]);
    }
    let mut rest = p;
    let mut out = Vec::new();
    for i in 0..q.len() {
        *work += rest.len();
        if *work > MAX_WORK {
            return Err("visual LOD seam exceeds its clipping work budget".into());
        }
        let outside = clip2(&rest, q[i], q[(i + 1) % q.len()], false);
        if !outside.is_empty() {
            out.push(outside);
        }
        rest = clip2(&rest, q[i], q[(i + 1) % q.len()], true);
        if rest.is_empty() {
            break;
        }
    }
    Ok(out)
}
fn hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup_by(|a, b| (0..2).all(|i| (a[i] - b[i]).abs() < EPS));
    if p.len() < 3 {
        return Vec::new();
    }
    let mut h = Vec::new();
    for &x in &p {
        while h.len() > 1 && cross(h[h.len() - 2], h[h.len() - 1], x) <= EPS {
            h.pop();
        }
        h.push(x);
    }
    let n = h.len();
    for &x in p[..p.len() - 1].iter().rev() {
        while h.len() > n && cross(h[h.len() - 2], h[h.len() - 1], x) <= EPS {
            h.pop();
        }
        h.push(x);
    }
    h.pop();
    clean(h)
}

pub fn caps(
    key: Key,
    grid: &Grid,
    surface: Surface,
    faces: [bool; 6],
    sample: impl Fn([i32; 3]) -> crate::lod::Sample,
) -> Result<Caps, String> {
    let mut caps: Caps = std::array::from_fn(|_| Vec::new());
    let scale = (1 << key.level) as f64;
    let (lo, hi) = bounds(key, grid.size());
    let base = key.tile.map(|c| c * TILE);
    let end = [0, 1, 2].map(|a| ((hi[a] as f64) / scale).ceil() as i32);
    let mut work = 0;
    for (face, out) in caps.iter_mut().enumerate() {
        if !faces[face] {
            continue;
        }
        let axis = face / 2;
        let uv = [(axis + 1) % 3, (axis + 2) % 3];
        let plane = if face % 2 == 0 { lo[axis] } else { hi[axis] } as f64;
        let patch = rect(uv.map(|a| lo[a]), uv.map(|a| hi[a]));
        let mut push = |p: Vec<[f64; 2]>, material: u8| -> Result<(), String> {
            let p = hull(
                p.into_iter()
                    .map(|p| [0, 1].map(|i| p[i].clamp(0.0, grid.size()[uv[i]] as f64)))
                    .collect(),
            );
            let p = intersect(p, &patch);
            if p.is_empty() {
                return Ok(());
            }
            let mut pieces = vec![p];
            // Caps are a solid union, including proxies beside smooth density.
            for old in out.iter() {
                let mut next = Vec::new();
                for p in pieces {
                    next.extend(difference(p, &old.points, &mut work)?);
                }
                pieces = next;
                if pieces.is_empty() {
                    break;
                }
            }
            out.extend(pieces.into_iter().map(|points| Cap { points, material }));
            Ok(())
        };
        if surface == Surface::Smooth {
            let normal_cell = (plane / scale - 0.5).floor() as i32;
            for v in base[uv[1]] - 1..end[uv[1]] {
                for u in base[uv[0]] - 1..end[uv[0]] {
                    let mut at = [0; 3];
                    at[axis] = normal_cell;
                    at[uv[0]] = u;
                    at[uv[1]] = v;
                    let cells = smooth::CORNERS.map(|c| [0, 1, 2].map(|a| at[a] + c[a]));
                    let values = cells.map(&sample);
                    let points = cells.map(|c| c.map(|v| (v as f64 + 0.5) * scale));
                    for tet in smooth::TETS {
                        let Some(material) = tet
                            .iter()
                            .find(|&&i| values[i].density < 0)
                            .map(|&i| values[i].density_material)
                        else {
                            continue;
                        };
                        let edges = [1, 2, 3]
                            .map(|i| [0, 1, 2].map(|a| points[tet[i]][a] - points[tet[0]][a]));
                        let cross3 = |a: [f64; 3], b: [f64; 3]| {
                            [
                                a[1] * b[2] - a[2] * b[1],
                                a[2] * b[0] - a[0] * b[2],
                                a[0] * b[1] - a[1] * b[0],
                            ]
                        };
                        let cofactors = [
                            cross3(edges[1], edges[2]),
                            cross3(edges[2], edges[0]),
                            cross3(edges[0], edges[1]),
                        ];
                        let det = (0..3).map(|a| edges[0][a] * cofactors[0][a]).sum::<f64>();
                        let gradient = (0..3)
                            .map(|i| {
                                cofactors[i][axis]
                                    * (values[tet[i + 1]].density as f64
                                        - values[tet[0]].density as f64)
                            })
                            .sum::<f64>()
                            / det;
                        let inward = if face % 2 == 0 { 1.0 } else { -1.0 };
                        let mut section = Vec::new();
                        for i in 0..4 {
                            for j in i + 1..4 {
                                let a = tet[i];
                                let b = tet[j];
                                let da = points[a][axis] - plane;
                                let db = points[b][axis] - plane;
                                if da * db > 0.0 || (da - db).abs() < EPS {
                                    continue;
                                }
                                let t = da / (da - db);
                                let p =
                                    uv.map(|k| points[a][k] + t * (points[b][k] - points[a][k]));
                                let mut d = values[a].density as f64
                                    + t * (values[b].density as f64 - values[a].density as f64);
                                // A zero face belongs to the solid on its interior side.
                                if d.abs() < EPS {
                                    d = if gradient * inward < 0.0 { -EPS } else { EPS };
                                }
                                section.push((p, d));
                            }
                        }
                        if section.len() < 3 {
                            continue;
                        }
                        let centre = [0, 1].map(|a| {
                            section.iter().map(|(p, _)| p[a]).sum::<f64>() / section.len() as f64
                        });
                        section.sort_by(|(a, _), (b, _)| {
                            (a[1] - centre[1])
                                .atan2(a[0] - centre[0])
                                .total_cmp(&(b[1] - centre[1]).atan2(b[0] - centre[0]))
                        });
                        let mut solid = Vec::new();
                        for i in 0..section.len() {
                            let (a, da) = section[i];
                            let (b, db) = section[(i + 1) % section.len()];
                            if da < 0.0 {
                                solid.push(a)
                            }
                            if (da < 0.0) != (db < 0.0) {
                                let t = da / (da - db);
                                solid.push([0, 1].map(|k| a[k] + t * (b[k] - a[k])));
                            }
                        }
                        push(solid, material)?;
                    }
                }
            }
        }
        let cell_axis = if face % 2 == 0 {
            base[axis]
        } else {
            end[axis] - 1
        };
        for v in base[uv[1]]..end[uv[1]] {
            for u in base[uv[0]]..end[uv[0]] {
                let mut at = [0; 3];
                at[axis] = cell_axis;
                at[uv[0]] = u;
                at[uv[1]] = v;
                let (material, shape) = if key.level == 0 {
                    (grid.get(at), grid.shape_at(at))
                } else {
                    let s = sample(at);
                    if surface == Surface::Smooth && (s.opacity == 0 || s.opacity == 255) {
                        continue;
                    }
                    (s.material, Shape::Cube)
                };
                if material == 0
                    || (key.level == 0 && surface == Surface::Smooth && shape == Shape::Cube)
                {
                    continue;
                }
                for part in shape.solids() {
                    let mut section = Vec::new();
                    for f in part {
                        for i in 0..f.points.len() {
                            let a =
                                [0, 1, 2].map(|k| (at[k] as f64 + f.points[i][k] as f64) * scale);
                            let b = [0, 1, 2].map(|k| {
                                (at[k] as f64 + f.points[(i + 1) % f.points.len()][k] as f64)
                                    * scale
                            });
                            let da = a[axis] - plane;
                            let db = b[axis] - plane;
                            if da.abs() < EPS {
                                section.push(uv.map(|k| a[k]));
                            }
                            if da * db < 0.0 {
                                let t = da / (da - db);
                                section.push(uv.map(|k| a[k] + t * (b[k] - a[k])));
                            }
                        }
                    }
                    push(section, material)?;
                }
            }
        }
    }
    Ok(caps)
}

#[derive(Clone, Copy)]
struct Vertex {
    p: [f64; 3],
    n: [f64; 3],
    c: [f64; 4],
}
fn clip3(p: &[Vertex], axis: usize, plane: f64, sign: f64) -> Vec<Vertex> {
    let mut out = Vec::new();
    for i in 0..p.len() {
        let a = p[i];
        let b = p[(i + 1) % p.len()];
        let da = (a.p[axis] - plane) * sign;
        let db = (b.p[axis] - plane) * sign;
        if da >= -EPS {
            out.push(a)
        }
        if (da >= -EPS) != (db >= -EPS) {
            let t = da / (da - db);
            out.push(Vertex {
                p: [0, 1, 2].map(|k| a.p[k] + t * (b.p[k] - a.p[k])),
                n: [0, 1, 2].map(|k| a.n[k] + t * (b.n[k] - a.n[k])),
                c: [0, 1, 2, 3].map(|k| a.c[k] + t * (b.c[k] - a.c[k])),
            });
        }
    }
    out
}
fn emit(out: &mut Group, p: &[Vertex]) {
    for i in 1..p.len().saturating_sub(1) {
        let tri = [p[0], p[i], p[i + 1]];
        let a = [0, 1, 2].map(|k| tri[1].p[k] - tri[0].p[k]);
        let b = [0, 1, 2].map(|k| tri[2].p[k] - tri[0].p[k]);
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        if n.iter().map(|x| x * x).sum::<f64>() < EPS * EPS {
            continue;
        }
        for v in tri {
            out.indices.push(out.positions.len() as u32 / 3);
            out.positions.extend(v.p.map(|x| x as f32));
            let len = v.n.iter().map(|x| x * x).sum::<f64>().sqrt();
            let normal = if len > EPS {
                v.n.map(|x| x / len)
            } else {
                let length = n.iter().map(|x| x * x).sum::<f64>().sqrt();
                n.map(|x| x / length)
            };
            out.normals.extend(normal.map(|x| x as f32));
            out.colors.extend(v.c.map(|x| x as f32));
        }
    }
}
fn compact(group: Group) -> Group {
    let mut out = Group::default();
    let mut vertices = BTreeMap::<[u32; 10], u32>::new();
    for index in group.indices {
        let i = index as usize;
        let p = [0, 1, 2].map(|a| group.positions[i * 3 + a]);
        let n = [0, 1, 2].map(|a| group.normals[i * 3 + a]);
        let c = [0, 1, 2, 3].map(|a| group.colors[i * 4 + a]);
        let mut key = [0; 10];
        for a in 0..3 {
            key[a] = p[a].to_bits();
            key[a + 3] = n[a].to_bits();
        }
        for a in 0..4 {
            key[a + 6] = c[a].to_bits();
        }
        let index = *vertices.entry(key).or_insert_with(|| {
            let index = out.positions.len() as u32 / 3;
            out.positions.extend(p);
            out.normals.extend(n);
            out.colors.extend(c);
            index
        });
        out.indices.push(index);
    }
    out
}

pub fn clip_groups(
    groups: Groups,
    hi: [f32; 3],
    outer_lo: [bool; 3],
    outer_hi: [bool; 3],
) -> Groups {
    groups
        .into_iter()
        .filter_map(|(m, g)| {
            let mut out = Group::default();
            for tri in g.indices.chunks_exact(3) {
                let mut p = tri
                    .iter()
                    .map(|&i| {
                        let i = i as usize;
                        Vertex {
                            p: [0, 1, 2].map(|k| g.positions[i * 3 + k] as f64),
                            n: [0, 1, 2].map(|k| g.normals[i * 3 + k] as f64),
                            c: [0, 1, 2, 3].map(|k| g.colors[i * 4 + k] as f64),
                        }
                    })
                    .collect::<Vec<_>>();
                let a = [0, 1, 2].map(|k| p[1].p[k] - p[0].p[k]);
                let b = [0, 1, 2].map(|k| p[2].p[k] - p[0].p[k]);
                let normal = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                if (0..3).any(|a| {
                    (normal[a] > 0.0 && p.iter().all(|v| v.p[a].abs() < EPS))
                        || (normal[a] < 0.0
                            && p.iter().all(|v| (v.p[a] - hi[a] as f64).abs() < EPS))
                }) {
                    continue;
                }
                for a in 0..3 {
                    if !outer_lo[a] {
                        p = clip3(&p, a, 0.0, 1.0);
                    }
                    if !outer_hi[a] {
                        p = clip3(&p, a, hi[a] as f64, -1.0);
                    }
                }
                emit(&mut out, &p);
            }
            (!out.indices.is_empty()).then_some((m, compact(out)))
        })
        .collect()
}

pub fn stitch(
    key: Key,
    mut groups: Groups,
    own: &Caps,
    neighbors: &[(Join, &Caps)],
    size: [i32; 3],
    voxel: f32,
    palette: &Palette,
) -> Result<Groups, String> {
    let (lo, hi) = bounds(key, size);
    let mut work = 0;
    for (join, other) in neighbors {
        let axis = join.face / 2;
        let uv = [(axis + 1) % 3, (axis + 2) % 3];
        let plane = if join.face % 2 == 0 {
            lo[axis]
        } else {
            hi[axis]
        } as f64;
        let patch = rect(join.lo, join.hi);
        // Remove ordinary boundary faces; only the solid difference is exposed.
        for group in groups.values_mut() {
            let mut out = Group::default();
            for tri in group.indices.chunks_exact(3) {
                let vertices = tri
                    .iter()
                    .map(|&i| {
                        let i = i as usize;
                        Vertex {
                            p: [0, 1, 2].map(|k| group.positions[i * 3 + k] as f64),
                            n: [0, 1, 2].map(|k| group.normals[i * 3 + k] as f64),
                            c: [0, 1, 2, 3].map(|k| group.colors[i * 4 + k] as f64),
                        }
                    })
                    .collect::<Vec<_>>();
                if vertices.iter().all(|v| {
                    (v.p[axis] - ((plane - lo[axis] as f64) as f32 * voxel) as f64).abs() < EPS
                }) {
                    let p = clean(
                        vertices
                            .iter()
                            .map(|v| uv.map(|k| v.p[k] / voxel as f64 + lo[k] as f64))
                            .collect(),
                    );
                    for piece in difference(p, &patch, &mut work)? {
                        let normal = vertices[0].n;
                        let color = vertices[0].c;
                        let p = piece
                            .into_iter()
                            .map(|p| {
                                let mut point = [0.0; 3];
                                point[axis] = (plane - lo[axis] as f64) * voxel as f64;
                                for k in 0..2 {
                                    point[uv[k]] = (p[k] - lo[uv[k]] as f64) * voxel as f64;
                                }
                                Vertex {
                                    p: point,
                                    n: normal,
                                    c: color,
                                }
                            })
                            .collect::<Vec<_>>();
                        // clean() winds in +axis; restore the original outward winding.
                        let mut p = p;
                        if normal[axis] < 0.0 {
                            p.reverse()
                        }
                        emit(&mut out, &p);
                    }
                } else {
                    emit(&mut out, &vertices)
                }
            }
            *group = compact(out);
        }
        for cap in &own[join.face] {
            let p = intersect(cap.points.clone(), &patch);
            if p.is_empty() {
                continue;
            }
            let mut pieces = vec![p];
            for cap in &other[join.face ^ 1] {
                let mut next = Vec::new();
                for p in pieces {
                    next.extend(difference(p, &cap.points, &mut work)?);
                }
                pieces = next;
                if pieces.is_empty() {
                    break;
                }
            }
            let Some(look) = palette.get(cap.material) else {
                continue;
            };
            let group = groups
                .entry((look.emission > 0.0).then_some(cap.material))
                .or_default();
            let mut normal = [0.0; 3];
            normal[axis] = if join.face % 2 == 0 { -1.0 } else { 1.0 };
            for mut piece in pieces {
                if normal[axis] < 0.0 {
                    piece.reverse()
                }
                let p = piece
                    .into_iter()
                    .map(|p| {
                        let mut point = [0.0; 3];
                        point[axis] = (plane - lo[axis] as f64) * voxel as f64;
                        for k in 0..2 {
                            point[uv[k]] = (p[k] - lo[uv[k]] as f64) * voxel as f64;
                        }
                        point.map(|x| x as f32)
                    })
                    .collect::<Vec<_>>();
                group.push_poly(&p, normal.map(|x| x as f32), look.color);
            }
        }
    }
    groups.retain(|_, g| !g.indices.is_empty());
    for g in groups.values_mut() {
        *g = compact(std::mem::take(g));
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lod_mesh::Cache;
    use std::collections::{BTreeMap, BTreeSet};

    fn geometry(grid: &mut Grid, key: Key, surface: Surface) -> Geometry {
        let palette = Palette::new(&[], &[]).unwrap();
        let mut cache = Cache::default();
        for _ in 0..200 {
            let tile = cache.poll(key, grid, &palette, surface, 1.0).unwrap();
            if tile.groups.is_some() {
                return tile
                    .visual(grid, &palette, surface, 1.0, [true; 6])
                    .unwrap();
            }
        }
        panic!("tile did not finish")
    }
    fn meshes(grid: &mut Grid, keys: &[Key], surface: Surface) -> Groups {
        let palette = Palette::new(&[], &[]).unwrap();
        let all: BTreeMap<_, _> = keys
            .iter()
            .map(|&key| (key, geometry(grid, key, surface)))
            .collect();
        let mut out = Groups::new();
        for &key in keys {
            let neighbors: Vec<_> = joins(key, keys, grid.size())
                .into_iter()
                .map(|j| {
                    let c = &all[&j.other].caps;
                    (j, c)
                })
                .collect();
            let groups = stitch(
                key,
                all[&key].groups.clone(),
                &all[&key].caps,
                &neighbors,
                grid.size(),
                1.0,
                &palette,
            )
            .unwrap();
            let (base, _) = bounds(key, grid.size());
            for (material, mut group) in groups {
                for p in group.positions.chunks_exact_mut(3) {
                    for a in 0..3 {
                        p[a] += base[a] as f32
                    }
                }
                let g = out.entry(material).or_default();
                let offset = g.positions.len() as u32 / 3;
                g.positions.extend(group.positions);
                g.normals.extend(group.normals);
                g.colors.extend(group.colors);
                g.indices
                    .extend(group.indices.into_iter().map(|i| i + offset));
            }
        }
        out
    }
    fn closed(groups: &Groups, size: [i32; 3]) {
        type Point = [i64; 3];
        let mut edges = BTreeMap::<(Point, Point), i32>::new();
        let mut buckets = BTreeMap::<Point, Vec<Point>>::new();
        let mut canonical = |point: Point| -> Point {
            let bucket = point.map(|v| v.div_euclid(4));
            for z in -1..=1 {
                for y in -1..=1 {
                    for x in -1..=1 {
                        let near = [bucket[0] + x, bucket[1] + y, bucket[2] + z];
                        if let Some(points) = buckets.get(&near)
                            && let Some(&p) = points
                                .iter()
                                .find(|p| (0..3).all(|a| (p[a] - point[a]).abs() <= 2))
                        {
                            return p;
                        }
                    }
                }
            }
            buckets.entry(bucket).or_default().push(point);
            point
        };
        for group in groups.values() {
            for tri in group.indices.chunks_exact(3) {
                let p = tri
                    .iter()
                    .map(|&i| {
                        canonical([0, 1, 2].map(|a| {
                            let x = group.positions[i as usize * 3 + a];
                            assert!(x >= -1e-5 && x <= size[a] as f32 + 1e-5);
                            (x as f64 * 100000.0).round() as i64
                        }))
                    })
                    .collect::<Vec<_>>();
                for (a, b) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
                    if a == b {
                        continue;
                    }
                    let (key, sign) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                    *edges.entry(key).or_default() += sign;
                }
            }
        }
        edges.retain(|_, n| *n != 0);
        let vertices: BTreeSet<_> = edges.keys().flat_map(|(a, b)| [*a, *b]).collect();
        let mut split = BTreeMap::<(Point, Point), i32>::new();
        for ((a, b), n) in edges {
            let d = [0, 1, 2].map(|k| (b[k] - a[k]) as f64);
            let length = d.iter().map(|v| v * v).sum::<f64>();
            let mut points = vec![(0.0, a), (1.0, b)];
            for &p in &vertices {
                let delta = [0, 1, 2].map(|k| (p[k] - a[k]) as f64);
                let t = (0..3).map(|k| d[k] * delta[k]).sum::<f64>() / length;
                if t > 1e-7
                    && t < 1.0 - 1e-7
                    && (0..3).map(|k| (delta[k] - t * d[k]).powi(2)).sum::<f64>() < 16.0
                {
                    points.push((t, p));
                }
            }
            points.sort_by(|a, b| a.0.total_cmp(&b.0));
            points.dedup_by_key(|p| p.1);
            for pair in points.windows(2) {
                let (a, b) = (pair[0].1, pair[1].1);
                let (key, sign) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                *split.entry(key).or_default() += n * sign;
            }
        }
        split.retain(|_, n| *n != 0);
        assert!(
            split.is_empty(),
            "{} unclosed directed edges, first {:?}",
            split.len(),
            split.first_key_value()
        );
    }
    fn cut(size: [i32; 3], level: u8) -> Vec<Key> {
        let width = TILE << level;
        let mut keys = vec![Key {
            level,
            tile: [0; 3],
        }];
        for z in 0..(size[2] + 7) / 8 {
            for y in 0..(size[1] + 7) / 8 {
                for x in width / 8..(size[0] + 7) / 8 {
                    keys.push(Key {
                        level: 0,
                        tile: [x, y, z],
                    });
                }
            }
        }
        keys
    }
    #[test]
    fn mixed_cubes_shapes_glow_and_smooth_cavities_are_closed() {
        for surface in [Surface::Cubes, Surface::Smooth] {
            let size = [31, 15, 13];
            let mut grid = Grid::new(size);
            grid.stream("empty", 0);
            for z in 2..11 {
                for y in 2..10 {
                    for x in 6..25 {
                        if !(12..20).contains(&x) || !(4..8).contains(&y) || !(4..9).contains(&z) {
                            grid.set([x, y, z], if x < 16 { 1 } else { 7 });
                        }
                    }
                }
            }
            grid.set_shaped([16, 11, 4], 7, Shape::Ramp(crate::shape::Facing::West));
            grid.set_shaped([15, 11, 8], 1, Shape::Slab);
            let keys = cut(size, 1);
            let groups = meshes(&mut grid, &keys, surface);
            assert!(groups.contains_key(&Some(7)));
            closed(&groups, size);
            assert_eq!(grid.allocated_bytes(), 0);
        }
    }
    #[test]
    fn arbitrary_level_gaps_and_partial_world_edges_are_closed() {
        for level in [2, 4] {
            for surface in [Surface::Cubes, Surface::Smooth] {
                let width = TILE << level;
                let size = [width + 7, 7, 7];
                let mut grid = Grid::new(size);
                grid.stream("empty", 0);
                for z in 0..7 {
                    for y in 0..7 {
                        for x in width - 3..width + 7 {
                            grid.set([x, y, z], 1);
                        }
                    }
                }
                if surface == Surface::Cubes {
                    grid.set_shaped([width - 1, 1, 1], 7, Shape::Post);
                }
                closed(&meshes(&mut grid, &cut(size, level), surface), size);
            }
        }
    }
    #[test]
    fn all_face_axes_and_empty_neighbors_preserve_closed_surfaces() {
        for axis in 0..3 {
            for surface in [Surface::Cubes, Surface::Smooth] {
                let rotate = |p: [i32; 3]| {
                    let mut q = [0; 3];
                    for a in 0..3 {
                        q[(a + axis) % 3] = p[a];
                    }
                    q
                };
                let size = rotate([31, 15, 13]);
                let mut grid = Grid::new(size);
                grid.stream("empty", 0);
                for z in 3..10 {
                    for y in 3..10 {
                        for x in 13..20 {
                            grid.set(rotate([x, y, z]), 7);
                        }
                    }
                }
                let keys: Vec<_> = cut([31, 15, 13], 1)
                    .into_iter()
                    .map(|k| Key {
                        level: k.level,
                        tile: rotate(k.tile),
                    })
                    .collect();
                closed(&meshes(&mut grid, &keys, surface), size);
                // A coarse proxy can cover a completely empty fine side.
                for z in 3..10 {
                    for y in 3..10 {
                        for x in 16..20 {
                            grid.set(rotate([x, y, z]), 0);
                        }
                    }
                }
                closed(&meshes(&mut grid, &keys, surface), size);
            }
        }
    }

    #[test]
    fn fractional_density_contours_match_the_clipped_tetrahedra() {
        let size = [31, 15, 13];
        let mut grid = Grid::new(size);
        grid.stream("empty", 0);
        for z in 0..13 {
            for y in 0..15 {
                for x in 8..25 {
                    let d = ((x as f64 - 15.3).powi(2)
                        + (y as f64 - 7.2).powi(2)
                        + (z as f64 - 6.4).powi(2)
                        - 31.0)
                        * 13.0;
                    grid.set_density([x, y, z], d.round().clamp(-1024.0, 1024.0) as i16, 7);
                }
            }
        }
        closed(&meshes(&mut grid, &cut(size, 1), Surface::Smooth), size);
    }

    #[test]
    fn equal_solid_caps_emit_no_internal_faces_and_budget_is_enforced() {
        let p = rect([0, 0], [8, 8]);
        let mut work = 0;
        assert!(difference(p.clone(), &p, &mut work).unwrap().is_empty());
        let mut work = MAX_WORK;
        assert!(difference(p.clone(), &p, &mut work).is_err());
        let parts = difference(p.clone(), &rect([2, 2], [6, 6]), &mut 0).unwrap();
        assert!((parts.iter().map(|p| area(p)).sum::<f64>() - 48.0).abs() < 1e-6);
    }
}
