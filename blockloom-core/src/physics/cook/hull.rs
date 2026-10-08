//! Convex hulls by incremental quickhull.
//!
//! Each step adds the point farthest outside the current hull, so stopping early
//! at a vertex limit keeps the points that matter most and the distance of what
//! was left out is the approximation error.

use super::{CookControl, CookError, fail};
use glam::DVec3;
use std::collections::HashMap;

/// A convex hull: a subset of the input points and its outward-wound faces.
#[derive(Debug, Clone, PartialEq)]
pub struct Hull {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    /// The farthest a skipped input point lies outside the hull.
    pub error: f32,
}

impl Hull {
    pub fn volume(&self) -> f32 {
        let at = |i: u32| DVec3::from_array(self.vertices[i as usize].map(f64::from));
        self.faces
            .iter()
            .map(|f| at(f[0]).dot(at(f[1]).cross(at(f[2]))) / 6.0)
            .sum::<f64>()
            .abs() as f32
    }
}

struct Face {
    v: [usize; 3],
    n: DVec3,
    d: f64,
    outside: Vec<usize>,
    alive: bool,
}

impl Face {
    /// A face over three points. With `orient`, the winding is chosen so the
    /// normal points away from that interior point; without it the winding is
    /// kept, which a horizon edge needs to stay the twin of its neighbour.
    fn new(pts: &[DVec3], a: usize, b: usize, c: usize, orient: Option<DVec3>) -> Face {
        let mut v = [a, b, c];
        let mut n = (pts[b] - pts[a]).cross(pts[c] - pts[a]);
        if let Some(inside) = orient
            && n.dot(inside - pts[a]) > 0.0
        {
            v = [a, c, b];
            n = -n;
        }
        let len = n.length();
        let n = if len > 0.0 { n / len } else { DVec3::ZERO };
        Face {
            v,
            d: n.dot(pts[v[0]]),
            n,
            outside: Vec::new(),
            alive: true,
        }
    }

    fn dist(&self, p: DVec3) -> f64 {
        self.n.dot(p) - self.d
    }

    fn edges(&self) -> [(usize, usize); 3] {
        [
            (self.v[0], self.v[1]),
            (self.v[1], self.v[2]),
            (self.v[2], self.v[0]),
        ]
    }
}

/// The hull of `points`, keeping at most `max_vertices` hull points.
pub fn quickhull(
    points: &[[f32; 3]],
    max_vertices: usize,
    control: &CookControl,
) -> Result<Hull, CookError> {
    if points.len() < 4 {
        return fail("A convex hull needs at least four points");
    }
    let pts: Vec<DVec3> = points
        .iter()
        .map(|p| DVec3::from_array(p.map(f64::from)))
        .collect();
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    for p in &pts {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let extent = (hi - lo).max_element();
    if extent.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return fail("The mesh is a single point");
    }
    let eps = extent * 1e-7;

    // The first tetrahedron: the widest pair, then the point farthest from
    // their line, then the one farthest from that plane.
    let mut extremes = Vec::new();
    for axis in 0..3 {
        let key = |i: &usize| pts[*i][axis];
        extremes.push(
            (0..pts.len())
                .min_by(|a, b| key(a).total_cmp(&key(b)))
                .unwrap(),
        );
        extremes.push(
            (0..pts.len())
                .max_by(|a, b| key(a).total_cmp(&key(b)))
                .unwrap(),
        );
    }
    let mut best = (extremes[0], extremes[1], -1.0);
    for &a in &extremes {
        for &b in &extremes {
            let d = pts[a].distance_squared(pts[b]);
            if d > best.2 {
                best = (a, b, d);
            }
        }
    }
    let (i0, i1) = (best.0, best.1);
    if best.2.sqrt() <= eps {
        return fail("The mesh is a single point");
    }
    let line = (pts[i1] - pts[i0]).normalize();
    let far = |dist: &dyn Fn(DVec3) -> f64| {
        (0..pts.len())
            .map(|i| (i, dist(pts[i])))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap()
    };
    let (i2, d2) = far(&|p| (p - pts[i0]).cross(line).length());
    if d2 <= eps {
        return fail("The mesh is a line: a convex hull needs volume");
    }
    let plane = (pts[i1] - pts[i0]).cross(pts[i2] - pts[i0]).normalize();
    let (i3, d3) = far(&|p| plane.dot(p - pts[i0]).abs());
    if d3 <= eps {
        return fail("The mesh is flat: a convex hull needs volume");
    }
    let inside = (pts[i0] + pts[i1] + pts[i2] + pts[i3]) / 4.0;

    let mut faces: Vec<Face> = Vec::new();
    let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
    let add = |faces: &mut Vec<Face>, edges: &mut HashMap<(usize, usize), usize>, f: Face| {
        let id = faces.len();
        for e in f.edges() {
            edges.insert(e, id);
        }
        faces.push(f);
        id
    };
    for (a, b, c) in [(i0, i1, i2), (i0, i1, i3), (i0, i2, i3), (i1, i2, i3)] {
        add(
            &mut faces,
            &mut edges,
            Face::new(&pts, a, b, c, Some(inside)),
        );
    }
    let simplex = [i0, i1, i2, i3];
    for (i, &point) in pts.iter().enumerate() {
        if simplex.contains(&i) {
            continue;
        }
        if let Some(f) = faces.iter_mut().find(|f| f.dist(point) > eps) {
            f.outside.push(i);
        }
    }

    let mut count = 4;
    let mut steps = 0usize;
    loop {
        if count >= max_vertices {
            break;
        }
        // The point farthest outside any face.
        let mut pick: Option<(usize, usize, f64)> = None;
        for (fi, f) in faces.iter().enumerate() {
            if !f.alive {
                continue;
            }
            for &pi in &f.outside {
                let d = f.dist(pts[pi]);
                if pick.is_none_or(|(_, _, best)| d > best) {
                    pick = Some((fi, pi, d));
                }
            }
        }
        let Some((start, p, _)) = pick else { break };
        steps += 1;
        if steps.is_multiple_of(16) {
            control.check()?;
        }

        // Every face the point can see, and the edges around them.
        let mut visible = vec![false; faces.len()];
        let mut stack = vec![start];
        visible[start] = true;
        let mut seen = vec![start];
        while let Some(fi) = stack.pop() {
            for (a, b) in faces[fi].edges() {
                let Some(&other) = edges.get(&(b, a)) else {
                    continue;
                };
                if !visible[other] && faces[other].alive && faces[other].dist(pts[p]) > eps {
                    visible[other] = true;
                    seen.push(other);
                    stack.push(other);
                }
            }
        }
        let mut horizon = Vec::new();
        for &fi in &seen {
            for (a, b) in faces[fi].edges() {
                match edges.get(&(b, a)) {
                    Some(&other) if visible[other] => {}
                    _ => horizon.push((a, b)),
                }
            }
        }
        horizon.sort_unstable();
        let mut orphans = Vec::new();
        for &fi in &seen {
            for e in faces[fi].edges() {
                edges.remove(&e);
            }
            faces[fi].alive = false;
            orphans.extend(
                std::mem::take(&mut faces[fi].outside)
                    .into_iter()
                    .filter(|o| *o != p),
            );
        }
        let first_new = faces.len();
        for (a, b) in horizon {
            add(&mut faces, &mut edges, Face::new(&pts, a, b, p, None));
        }
        for o in orphans {
            if let Some(f) = faces[first_new..].iter_mut().find(|f| f.dist(pts[o]) > eps) {
                f.outside.push(o);
            }
        }
        let mut used = std::collections::BTreeSet::new();
        for f in faces.iter().filter(|f| f.alive) {
            used.extend(f.v);
        }
        count = used.len();
    }

    let mut error = 0.0f64;
    for f in faces.iter().filter(|f| f.alive) {
        for p in &f.outside {
            error = error.max(f.dist(pts[*p]));
        }
    }
    let error = error as f32;
    let mut used: Vec<usize> = faces.iter().filter(|f| f.alive).flat_map(|f| f.v).collect();
    used.sort_unstable();
    used.dedup();
    let index: HashMap<usize, u32> = used
        .iter()
        .enumerate()
        .map(|(n, i)| (*i, n as u32))
        .collect();
    Ok(Hull {
        vertices: used.iter().map(|i| points[*i]).collect(),
        faces: faces
            .iter()
            .filter(|f| f.alive)
            .map(|f| [index[&f.v[0]], index[&f.v[1]], index[&f.v[2]]])
            .collect(),
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_points(n: usize) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        for x in 0..n {
            for y in 0..n {
                for z in 0..n {
                    out.push(
                        [x as f32, y as f32, z as f32].map(|v| v / (n - 1) as f32 * 2.0 - 1.0),
                    );
                }
            }
        }
        out
    }

    #[test]
    fn a_cloud_in_a_cube_hulls_to_the_cube() {
        let hull = quickhull(&cube_points(4), 255, &CookControl::new()).unwrap();
        assert_eq!(hull.vertices.len(), 8, "interior and face points are gone");
        assert_eq!(hull.faces.len(), 12);
        assert!((hull.volume() - 8.0).abs() < 1e-4, "{}", hull.volume());
        assert_eq!(hull.error, 0.0);
    }

    #[test]
    fn a_sphere_hull_is_capped_and_reports_what_it_left_out() {
        let mut points = Vec::new();
        let n = 24;
        for i in 0..n {
            let v = (i as f32 + 0.5) / n as f32;
            let phi = (1.0 - 2.0 * v).acos();
            for j in 0..2 * n {
                let theta = j as f32 / (2 * n) as f32 * std::f32::consts::TAU;
                points.push([phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()]);
            }
        }
        let full = quickhull(&points, 255, &CookControl::new()).unwrap();
        assert!(full.vertices.len() <= 255);
        let small = quickhull(&points, 24, &CookControl::new()).unwrap();
        assert!(small.vertices.len() <= 24);
        assert!(
            small.error > full.error,
            "{} vs {}",
            small.error,
            full.error
        );
        assert!(small.error < 0.35, "{}", small.error);
        let v = small.volume();
        assert!(v > 2.5 && v < 4.2, "unit sphere is 4.19: {v}");
    }

    #[test]
    fn degenerate_input_says_what_is_wrong() {
        let point = vec![[1.0; 3]; 6];
        assert!(
            quickhull(&point, 255, &CookControl::new())
                .unwrap_err()
                .to_string()
                .contains("single point")
        );
        let line: Vec<_> = (0..6).map(|i| [i as f32, 0.0, 0.0]).collect();
        assert!(
            quickhull(&line, 255, &CookControl::new())
                .unwrap_err()
                .to_string()
                .contains("line")
        );
        let flat: Vec<_> = (0..9)
            .map(|i| [(i % 3) as f32, (i / 3) as f32, 0.0])
            .collect();
        assert!(
            quickhull(&flat, 255, &CookControl::new())
                .unwrap_err()
                .to_string()
                .contains("flat")
        );
        assert!(quickhull(&flat[..3], 255, &CookControl::new()).is_err());
    }

    #[test]
    fn a_hull_is_convex_and_closed() {
        // A bumpy blob: every face plane must hold every vertex behind it, and
        // every edge must be shared by exactly two faces.
        let mut seed = 7u32;
        let mut rand = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let points: Vec<_> = (0..400)
            .map(|_| [rand(), rand() * 0.5, rand() * 2.0])
            .collect();
        let hull = quickhull(&points, 255, &CookControl::new()).unwrap();
        let at = |i: u32| DVec3::from_array(hull.vertices[i as usize].map(f64::from));
        for f in &hull.faces {
            let n = (at(f[1]) - at(f[0])).cross(at(f[2]) - at(f[0])).normalize();
            for p in &points {
                let d = n.dot(DVec3::from_array(p.map(f64::from)) - at(f[0]));
                assert!(d < 1e-5, "a point sits {d} outside a face");
            }
        }
        let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
        for f in &hull.faces {
            for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                *edge_count.entry((a, b)).or_default() += 1;
            }
        }
        for ((a, b), n) in &edge_count {
            assert_eq!(*n, 1);
            assert_eq!(edge_count.get(&(*b, *a)), Some(&1), "open edge {a}-{b}");
        }
    }
}
