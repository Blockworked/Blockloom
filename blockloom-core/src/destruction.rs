//! Deterministic convex fracture and bounded grids for small fluid effects.

pub use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub const SHARD_CAP: usize = 256;
pub const GRID: usize = 64;
pub const SMOKE_CAP: usize = 16;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FractureSpec {
    pub cells: u32,
    pub seed: u32,
    pub interior: crate::material::SurfaceMaterial,
    pub impulse_threshold: f32,
    pub lifetime: f32,
    pub sleep_seconds: f32,
    pub pool_cap: u32,
    pub bounce_sound: String,
}

impl Default for FractureSpec {
    fn default() -> Self {
        Self {
            cells: 16,
            seed: 1,
            interior: Default::default(),
            impulse_threshold: 8.0,
            lifetime: 20.0,
            sleep_seconds: 2.0,
            pool_cap: SHARD_CAP as u32,
            bounce_sound: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Face {
    pub vertices: Vec<Vec3>,
    pub interior: bool,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub faces: Vec<Face>,
    pub center: Vec3,
}

fn random(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    (*seed >> 8) as f32 / 16777216.0
}

/// Clips a convex solid at a bisector, closing it with an interior face.
fn clip(faces: Vec<Face>, normal: Vec3, distance: f32) -> Vec<Face> {
    let mut result = Vec::new();
    let mut cap: Vec<Vec3> = Vec::new();
    for face in faces {
        let mut vertices = Vec::new();
        for (a, b) in face
            .vertices
            .iter()
            .zip(face.vertices.iter().cycle().skip(1))
            .take(face.vertices.len())
        {
            let da = normal.dot(*a) - distance;
            let db = normal.dot(*b) - distance;
            if da <= 1e-6 {
                vertices.push(*a);
            }
            if (da < 0.0) != (db < 0.0) {
                let point = a.lerp(*b, da / (da - db));
                vertices.push(point);
                if !cap.iter().any(|p| p.distance_squared(point) < 1e-10) {
                    cap.push(point);
                }
            }
        }
        if vertices.len() >= 3 {
            result.push(Face {
                vertices,
                interior: face.interior,
            });
        }
    }
    if cap.len() >= 3 {
        let center = cap.iter().copied().sum::<Vec3>() / cap.len() as f32;
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        cap.sort_by(|a, b| {
            let a = *a - center;
            let b = *b - center;
            a.dot(v)
                .atan2(a.dot(u))
                .total_cmp(&b.dot(v).atan2(b.dot(u)))
        });
        result.push(Face {
            vertices: cap,
            interior: true,
        });
    }
    result
}

/// Voronoi cells inside a box, or inside a caller's convex mesh.
pub fn fracture(hull: &[Face], size: Vec3, count: u32, seed: u32) -> Vec<Cell> {
    if !size.is_finite() || size.min_element() <= 0.0 {
        return Vec::new();
    }
    let mut seed = seed;
    let mut sites = Vec::new();
    let count = count.clamp(2, 64) as usize;
    for _ in 0..count * 1024 {
        let point = (Vec3::new(random(&mut seed), random(&mut seed), random(&mut seed))
            - Vec3::splat(0.5))
            * size;
        if hull.iter().all(|face| {
            let a = face.vertices[0];
            let normal = (face.vertices[1] - a)
                .cross(face.vertices[2] - a)
                .normalize_or_zero();
            normal.dot(point - a) <= 1e-6
        }) {
            sites.push(point);
        }
        if sites.len() == count {
            break;
        }
    }
    if sites.len() < 2 {
        return Vec::new();
    }
    sites
        .iter()
        .enumerate()
        .filter_map(|(i, site)| {
            let mut faces = hull.to_vec();
            for (j, other) in sites.iter().enumerate() {
                if i == j {
                    continue;
                }
                let n = (*other - *site).normalize_or_zero();
                faces = clip(faces, n, n.dot((*site + *other) * 0.5));
                if faces.is_empty() {
                    return None;
                }
            }
            let vertices: Vec<_> = faces
                .iter()
                .flat_map(|f| f.vertices.iter().copied())
                .collect();
            let center = vertices.iter().copied().sum::<Vec3>() / vertices.len() as f32;
            Some(Cell { faces, center })
        })
        .collect()
}

pub fn box_hull(size: Vec3) -> Vec<Face> {
    let h = size * 0.5;
    let p = [
        Vec3::new(-h.x, -h.y, -h.z),
        Vec3::new(h.x, -h.y, -h.z),
        Vec3::new(h.x, h.y, -h.z),
        Vec3::new(-h.x, h.y, -h.z),
        Vec3::new(-h.x, -h.y, h.z),
        Vec3::new(h.x, -h.y, h.z),
        Vec3::new(h.x, h.y, h.z),
        Vec3::new(-h.x, h.y, h.z),
    ];
    [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 4, 7, 3],
        [1, 2, 6, 5],
        [0, 1, 5, 4],
        [3, 7, 6, 2],
    ]
    .map(|indices| Face {
        vertices: indices.map(|i| p[i]).to_vec(),
        interior: false,
    })
    .to_vec()
}

/// One world-space surface map. Channels are scorch and wetness.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurfaceMap {
    pub origin: [f32; 2],
    pub extent: f32,
    pub cells: Vec<[f32; 2]>,
}

impl Default for SurfaceMap {
    fn default() -> Self {
        Self {
            origin: [-128.0; 2],
            extent: 256.0,
            cells: vec![[0.0; 2]; GRID * GRID],
        }
    }
}

impl SurfaceMap {
    pub fn valid(&self) -> bool {
        self.extent.is_finite()
            && self.extent > 0.0
            && self.origin.iter().all(|v| v.is_finite())
            && self.cells.len() == GRID * GRID
            && self
                .cells
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }
    pub fn paint(&mut self, at: Vec2, radius: f32, scorch: f32, wet: f32) {
        if !at.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return;
        }
        let pitch = self.extent / GRID as f32;
        for y in 0..GRID {
            for x in 0..GRID {
                let p = Vec2::from(self.origin) + Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * pitch;
                let weight = (1.0 - p.distance(at) / radius.max(pitch)).clamp(0.0, 1.0);
                let cell = &mut self.cells[y * GRID + x];
                cell[0] = cell[0].max((scorch * weight).clamp(0.0, 1.0));
                cell[1] = cell[1].max((wet * weight).clamp(0.0, 1.0));
            }
        }
    }
    pub fn step(&mut self, dt: f32, rain: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let rain = if rain.is_finite() {
            rain.clamp(0.0, 1.0)
        } else {
            0.0
        };
        for cell in &mut self.cells {
            cell[1] = (cell[1] + dt * (rain * 0.1 - 0.005)).clamp(0.0, 1.0);
        }
    }
}

/// Semi-Lagrangian density on a vertical plane, with buoyancy and decay.
pub struct SmokeGrid {
    pub at: Vec3,
    pub radius: f32,
    pub age: f32,
    pub density: Vec<f32>,
}

impl SmokeGrid {
    pub fn new(at: Vec3, radius: f32, amount: f32) -> Option<Self> {
        if !at.is_finite()
            || !radius.is_finite()
            || !amount.is_finite()
            || radius <= 0.0
            || amount <= 0.0
        {
            return None;
        }
        let mut grid = Self {
            at,
            radius: radius.clamp(0.01, 100.0),
            age: 0.0,
            density: vec![0.0; GRID * GRID],
        };
        for y in 0..GRID {
            for x in 0..GRID {
                let p = Vec2::new(x as f32 / (GRID - 1) as f32, y as f32 / (GRID - 1) as f32);
                grid.density[y * GRID + x] = (1.0 - p.distance(Vec2::new(0.5, 0.18)) * 5.0)
                    .clamp(0.0, 1.0)
                    * amount.min(1.0);
            }
        }
        Some(grid)
    }
    pub fn step(&mut self, dt: f32, air: Vec2) {
        if !dt.is_finite() || dt <= 0.0 || !air.is_finite() {
            return;
        }
        if dt > 0.1 {
            let steps = (dt / 0.1).ceil().min(100.0) as usize;
            for _ in 0..steps {
                self.step((dt / steps as f32).min(0.1), air);
            }
            return;
        }
        let mut next = vec![0.0; GRID * GRID];
        let velocity = (air + Vec2::Y * 0.5) * dt / self.radius * GRID as f32 * 0.25;
        for y in 0..GRID {
            for x in 0..GRID {
                let p = Vec2::new(x as f32, y as f32) - velocity;
                if p.x < 0.0 || p.y < 0.0 || p.x >= (GRID - 1) as f32 || p.y >= (GRID - 1) as f32 {
                    continue;
                }
                let ix = p.x as usize;
                let iy = p.y as usize;
                let f = p.fract();
                let a = self.density[iy * GRID + ix] * (1.0 - f.x)
                    + self.density[iy * GRID + ix + 1] * f.x;
                let b = self.density[(iy + 1) * GRID + ix] * (1.0 - f.x)
                    + self.density[(iy + 1) * GRID + ix + 1] * f.x;
                next[y * GRID + x] = (a * (1.0 - f.y) + b * f.y) * (-dt * 0.35).exp();
            }
        }
        self.density = next;
        self.age += dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cells_partition_the_box_and_caps_face_out() {
        let cells = fracture(&box_hull(Vec3::splat(2.0)), Vec3::splat(2.0), 16, 4);
        assert_eq!(cells.len(), 16);
        let mut volume = 0.0;
        for cell in cells {
            for face in cell.faces {
                let a = face.vertices[0];
                for pair in face.vertices[1..].windows(2) {
                    let n = (pair[0] - a).cross(pair[1] - a);
                    assert!(n.dot(a - cell.center) >= -1e-5);
                    volume += a.dot(pair[0].cross(pair[1])) / 6.0;
                }
            }
        }
        assert!((volume - 8.0).abs() < 1e-4, "{volume}");
    }
    #[test]
    fn lasting_scorch_and_drying_wetness_survive_serialization() {
        let mut map = SurfaceMap::default();
        map.paint(Vec2::ZERO, 12.0, 1.0, 1.0);
        let scorch: Vec<_> = map.cells.iter().map(|c| c[0]).collect();
        map.step(300.0, 0.0);
        assert!(map.cells.iter().all(|c| c[1] == 0.0));
        assert_eq!(scorch, map.cells.iter().map(|c| c[0]).collect::<Vec<_>>());
        let loaded: SurfaceMap =
            serde_json::from_str(&serde_json::to_string(&map).unwrap()).unwrap();
        assert!(loaded.valid());
        assert_eq!(loaded.cells, map.cells);
    }
    #[test]
    fn smoke_advects_upwards_and_dissipates() {
        let mut grid = SmokeGrid::new(Vec3::ZERO, 2.0, 1.0).unwrap();
        let center = |g: &SmokeGrid| {
            g.density
                .iter()
                .enumerate()
                .map(|(i, d)| (i / GRID) as f32 * d)
                .sum::<f32>()
                / g.density.iter().sum::<f32>()
        };
        let before = center(&grid);
        let mass: f32 = grid.density.iter().sum();
        for _ in 0..30 {
            grid.step(0.1, Vec2::ZERO);
        }
        assert!(center(&grid) > before);
        assert!(grid.density.iter().sum::<f32>() < mass);
    }
}
