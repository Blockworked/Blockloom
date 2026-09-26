//! Where vegetation stands: grass blades and scattered trees and rocks.
//!
//! Both are deterministic from the terrain's grids and a seed, and laid out
//! on world-fixed cells, so building any region again (a streaming cell
//! coming back, a chunk rebuilt after a stroke) puts every blade and tree
//! exactly where it was.

use super::sculpt::{hash, unit, value_noise};
use super::{Heightfield, Shape};
use crate::material::hex_to_linear;
use serde::{Deserialize, Serialize};

/// A grass layer: blades growing on one paint layer (or everywhere).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrassLayer {
    pub name: String,
    /// The paint layer it grows on, or -1 for everywhere.
    pub layer: i32,
    /// Blades per square metre where it grows fully.
    pub density: f32,
    /// Shortest and tallest blade, metres.
    pub height: [f32; 2],
    /// Blade width at the root, metres.
    pub width: f32,
    pub base_color: String,
    pub tip_color: String,
    /// How far each blade's color strays, 0 to 1.
    pub variation: f32,
    /// How much a blade resists the wind, 0 (flattens) to 1 (stiff).
    pub stiffness: f32,
    /// Multiplier on the world's wind.
    pub wind: f32,
    /// Degrees from flat past which nothing grows.
    pub max_slope: f32,
    /// Metres from the camera where blades start thinning and shrinking.
    pub fade_start: f32,
    /// Metres past which no grass draws.
    pub cull_distance: f32,
    /// A density map in the terrain store; empty grows everywhere the
    /// layer does.
    pub density_map: String,
    pub seed: u32,
}

impl Default for GrassLayer {
    fn default() -> Self {
        Self {
            name: "Grass".to_string(),
            layer: 0,
            density: 16.0,
            height: [0.25, 0.6],
            width: 0.05,
            base_color: "#2E4A1B".to_string(),
            tip_color: "#93AE4F".to_string(),
            variation: 0.2,
            stiffness: 0.5,
            wind: 1.0,
            max_slope: 40.0,
            fade_start: 30.0,
            cull_distance: 60.0,
            density_map: String::new(),
            seed: 1,
        }
    }
}

impl GrassLayer {
    pub fn normalize(&mut self, layers: usize) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.layer = self.layer.clamp(-1, layers as i32 - 1);
        self.density = finite(self.density, 16.0).clamp(0.0, 400.0);
        let mut height = self.height.map(|v| finite(v, 0.4).clamp(0.01, 10.0));
        if height[0] > height[1] {
            height.swap(0, 1);
        }
        self.height = height;
        self.width = finite(self.width, 0.05).clamp(0.001, 2.0);
        self.variation = finite(self.variation, 0.2).clamp(0.0, 1.0);
        self.stiffness = finite(self.stiffness, 0.5).clamp(0.0, 1.0);
        self.wind = finite(self.wind, 1.0).clamp(0.0, 10.0);
        self.max_slope = finite(self.max_slope, 40.0).clamp(0.0, 90.0);
        self.cull_distance = finite(self.cull_distance, 60.0).clamp(1.0, 1000.0);
        self.fade_start = finite(self.fade_start, 30.0).clamp(0.0, self.cull_distance);
        self.density_map = self.density_map.trim().to_string();
    }
}

/// A procedural stand-in, drawn when a scatter layer names no model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ScatterShape {
    /// A round crown on a trunk.
    #[default]
    Tree,
    /// A cone of tiers on a trunk.
    Pine,
    Bush,
    Rock,
}

/// A scatter layer: trees, rocks or props dotted over the ground by rule
/// and by brush.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScatterLayer {
    pub name: String,
    /// A glTF/GLB file; its meshes draw at their node transforms. Empty
    /// draws `shape`.
    pub model: String,
    /// A lighter file for the middle distance. Empty keeps the model.
    pub model_lod1: String,
    pub shape: ScatterShape,
    /// Tint of a procedural shape's crown, or over a model.
    pub color: String,
    /// Instances per 100 square metres where the layer grows fully.
    pub density: f32,
    /// Metres kept between neighbours, roughly.
    pub spacing: f32,
    /// How much instances bunch into clumps, 0 (even) to 1.
    pub clumping: f32,
    /// Metres across a clump.
    pub clump_size: f32,
    /// The paint layer it grows on, or -1 for everywhere.
    pub layer: i32,
    /// Degrees from flat it grows between.
    pub slope: [f32; 2],
    /// Metres above the actor it grows between.
    pub altitude: [f32; 2],
    /// Scale range.
    pub scale: [f32; 2],
    /// How far each instance's tint strays, 0 to 1.
    pub tint: f32,
    /// 0 stands upright, 1 leans with the ground.
    pub align: f32,
    /// Metres pushed into the ground, so roots and rock bases don't float.
    pub sink: f32,
    /// Keep clear of other actors' footprints.
    pub avoid_actors: bool,
    /// Give each instance a solid trunk (or rock) collider.
    pub collide: bool,
    /// Collider radius, metres, before scale.
    pub radius: f32,
    /// Metres past which the lighter level draws.
    pub lod1_distance: f32,
    /// An image drawn on crossed quads far away. Empty skips that level.
    pub billboard: String,
    pub billboard_distance: f32,
    /// Metres past which nothing draws.
    pub cull_distance: f32,
    /// A density map in the terrain store; empty grows everywhere the
    /// rules allow.
    pub density_map: String,
    pub seed: u32,
}

impl Default for ScatterLayer {
    fn default() -> Self {
        Self {
            name: "Trees".to_string(),
            model: String::new(),
            model_lod1: String::new(),
            shape: ScatterShape::Tree,
            color: "#3F6B2A".to_string(),
            density: 0.5,
            spacing: 3.0,
            clumping: 0.4,
            clump_size: 24.0,
            layer: -1,
            slope: [0.0, 30.0],
            altitude: [-100_000.0, 100_000.0],
            scale: [0.8, 1.3],
            tint: 0.12,
            align: 0.0,
            sink: 0.1,
            avoid_actors: true,
            collide: true,
            radius: 0.35,
            lod1_distance: 60.0,
            billboard: String::new(),
            billboard_distance: 160.0,
            cull_distance: 450.0,
            density_map: String::new(),
            seed: 1,
        }
    }
}

impl ScatterLayer {
    pub fn normalize(&mut self, layers: usize) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.model = self.model.trim().to_string();
        self.model_lod1 = self.model_lod1.trim().to_string();
        self.billboard = self.billboard.trim().to_string();
        self.density = finite(self.density, 0.5).clamp(0.0, 100.0);
        self.spacing = finite(self.spacing, 3.0).clamp(0.1, 1000.0);
        self.clumping = finite(self.clumping, 0.4).clamp(0.0, 1.0);
        self.clump_size = finite(self.clump_size, 24.0).clamp(0.5, 10_000.0);
        self.layer = self.layer.clamp(-1, layers as i32 - 1);
        let ordered = |range: [f32; 2], lo: f32, hi: f32| {
            let mut r = range.map(|v| finite(v, lo).clamp(lo, hi));
            if r[0] > r[1] {
                r.swap(0, 1);
            }
            r
        };
        self.slope = ordered(self.slope, 0.0, 90.0);
        self.altitude = ordered(self.altitude, -100_000.0, 100_000.0);
        self.scale = ordered(self.scale, 0.01, 100.0);
        self.tint = finite(self.tint, 0.12).clamp(0.0, 1.0);
        self.align = finite(self.align, 0.0).clamp(0.0, 1.0);
        self.sink = finite(self.sink, 0.1).clamp(0.0, 100.0);
        self.radius = finite(self.radius, 0.35).clamp(0.01, 100.0);
        self.cull_distance = finite(self.cull_distance, 450.0).clamp(1.0, 100_000.0);
        self.lod1_distance = finite(self.lod1_distance, 60.0).clamp(0.5, self.cull_distance);
        self.billboard_distance =
            finite(self.billboard_distance, 160.0).clamp(self.lod1_distance, self.cull_distance);
        self.density_map = self.density_map.trim().to_string();
    }
}

/// What the ground offers vegetation, all at the terrain's resolution.
#[derive(Clone, Copy)]
pub struct Ground<'a> {
    pub field: &'a Heightfield,
    pub shape: &'a Shape,
    /// Baked layer weights.
    pub weights: Option<&'a [[u8; 4]]>,
    pub holes: Option<&'a [u8]>,
}

impl Ground<'_> {
    fn nearest(&self, x: f32, z: f32) -> usize {
        let [i, j] = self.shape.sample(x, z);
        let last = (self.shape.side - 1) as f32;
        let (i, j) = (
            i.round().clamp(0.0, last) as u32,
            j.round().clamp(0.0, last) as u32,
        );
        (j * self.shape.side + i) as usize
    }

    /// How much of paint layer `layer` covers local X, Z; -1 is everywhere.
    fn layer(&self, layer: i32, x: f32, z: f32) -> f32 {
        match (layer, self.weights) {
            (layer, Some(weights)) if layer >= 0 => {
                weights[self.nearest(x, z)][layer as usize & 3] as f32 / 255.0
            }
            // Without weights everything is layer 0.
            (layer, None) if layer > 0 => 0.0,
            _ => 1.0,
        }
    }

    fn hole(&self, x: f32, z: f32) -> bool {
        self.holes
            .is_some_and(|holes| holes[self.nearest(x, z)] > 127)
    }

    fn density(map: Option<&[u8]>, index: usize) -> f32 {
        map.map_or(1.0, |map| map[index] as f32 / 255.0)
    }
}

/// One grass blade, in the terrain's local frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blade {
    pub root: [f32; 3],
    pub normal: [f32; 3],
    pub height: f32,
    pub width: f32,
    /// Radians about the ground normal.
    pub facing: f32,
    /// 0-1, which blades thin out first with distance.
    pub rank: f32,
    /// A small random lean, 0-1.
    pub lean: f32,
    pub shade: f32,
}

/// Blades in a local X, Z rectangle. Cells are fixed to the terrain, so a
/// blade belongs to exactly one rectangle of any tiling. Stops at `max`.
pub fn grass_blades(
    ground: &Ground,
    grass: &GrassLayer,
    density_map: Option<&[u8]>,
    min: [f32; 2],
    max: [f32; 2],
    limit: usize,
) -> Vec<Blade> {
    let mut out = Vec::new();
    if grass.density <= 0.0 {
        return out;
    }
    let shape = ground.shape;
    let cell = 1.0 / grass.density.sqrt();
    let origin = [-shape.size[0] * 0.5, -shape.size[1] * 0.5];
    let lo = [min[0].max(origin[0]), min[1].max(origin[1])];
    let hi = [max[0].min(-origin[0]), max[1].min(-origin[1])];
    if lo[0] >= hi[0] || lo[1] >= hi[1] {
        return out;
    }
    let first = [
        ((lo[0] - origin[0]) / cell).floor() as i64,
        ((lo[1] - origin[1]) / cell).floor() as i64,
    ];
    let last = [
        ((hi[0] - origin[0]) / cell).ceil() as i64,
        ((hi[1] - origin[1]) / cell).ceil() as i64,
    ];
    let cos_max = grass.max_slope.to_radians().cos();
    for cz in first[1]..last[1] {
        for cx in first[0]..last[0] {
            let n = (cx as u32).wrapping_mul(0x9E37_79B9) ^ (cz as u32).wrapping_mul(0x85EB_CA6B);
            let x = origin[0] + (cx as f32 + unit(hash(grass.seed, n, 0))) * cell;
            let z = origin[1] + (cz as f32 + unit(hash(grass.seed, n, 1))) * cell;
            // Half-open, so neighbouring rectangles never both take a blade.
            if x < lo[0] || x >= hi[0] || z < lo[1] || z >= hi[1] {
                continue;
            }
            if ground.hole(x, z) {
                continue;
            }
            let index = ground.nearest(x, z);
            let chance = ground.layer(grass.layer, x, z) * Ground::density(density_map, index);
            if unit(hash(grass.seed, n, 2)) >= chance {
                continue;
            }
            let normal = ground.field.normal_at(shape, x, z);
            if normal[1] < cos_max {
                continue;
            }
            let Some(y) = ground.field.height_at(shape, x, z) else {
                continue;
            };
            let t = unit(hash(grass.seed, n, 3));
            out.push(Blade {
                root: [x, y, z],
                normal,
                height: grass.height[0] + (grass.height[1] - grass.height[0]) * t,
                width: grass.width * (0.7 + 0.6 * unit(hash(grass.seed, n, 4))),
                facing: unit(hash(grass.seed, n, 5)) * std::f32::consts::TAU,
                rank: unit(hash(grass.seed, n, 6)),
                lean: unit(hash(grass.seed, n, 7)),
                shade: unit(hash(grass.seed, n, 8)) * 2.0 - 1.0,
            });
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// Grass blades as one mesh: five vertices a blade, tapering to a point.
/// `uv` carries each vertex's height up its blade (0-1) and the blade's
/// height in metres, `color`'s alpha the blade's rank, for the grass shader's
/// wind and distance thinning.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GrassMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

pub fn grass_mesh(blades: &[Blade], grass: &GrassLayer, origin: [f32; 3]) -> GrassMesh {
    let base = hex_to_linear(&grass.base_color);
    let tip = hex_to_linear(&grass.tip_color);
    let mut mesh = GrassMesh::default();
    for blade in blades {
        let up = glam::Vec3::from(blade.normal)
            .lerp(glam::Vec3::Y, 0.6)
            .normalize();
        let lean = glam::Vec3::new(blade.facing.cos(), 0.0, blade.facing.sin());
        let side =
            glam::Vec3::new(-blade.facing.sin(), 0.0, blade.facing.cos()) * blade.width * 0.5;
        let root = glam::Vec3::from(blade.root) - glam::Vec3::from(origin);
        let shade = 1.0 + blade.shade * grass.variation;
        let first = mesh.positions.len() as u32;
        for (t, half) in [
            (0.0f32, 1.0f32),
            (0.0, -1.0),
            (0.55, 0.75),
            (0.55, -0.75),
            (1.0, 0.0),
        ] {
            let bend = lean * (t * t * blade.lean * 0.3 * blade.height);
            let p = root + up * (t * blade.height) + bend + side * half * (1.0 - t * 0.4);
            mesh.positions.push(p.to_array());
            mesh.normals.push(blade.normal);
            let c: [f32; 4] = std::array::from_fn(|k| {
                if k == 3 {
                    blade.rank
                } else {
                    (base[k] + (tip[k] - base[k]) * t) * shade
                }
            });
            mesh.colors.push(c);
            mesh.uvs.push([t, blade.height]);
        }
        mesh.indices.extend_from_slice(&[
            first,
            first + 1,
            first + 2,
            first + 2,
            first + 1,
            first + 3,
            first + 2,
            first + 3,
            first + 4,
        ]);
    }
    mesh
}

/// A footprint scatter keeps clear of, local X, Z.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Avoid {
    pub center: [f32; 2],
    pub radius: f32,
}

/// One scattered tree or rock, in the terrain's local frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: f32,
    /// Multiplied over the layer's color.
    pub tint: f32,
}

/// Every instance of a scatter layer, up to `limit`.
pub fn scatter_instances(
    ground: &Ground,
    layer: &ScatterLayer,
    density_map: Option<&[u8]>,
    avoid: &[Avoid],
    limit: usize,
) -> Vec<Instance> {
    let mut out = Vec::new();
    let shape = ground.shape;
    let cell = layer.spacing;
    // One candidate per spacing cell, kept with this chance where the layer
    // grows fully.
    let chance = (layer.density / 100.0 * cell * cell).min(1.0);
    if chance <= 0.0 {
        return out;
    }
    let origin = [-shape.size[0] * 0.5, -shape.size[1] * 0.5];
    let cells = [
        (shape.size[0] / cell).ceil() as i64,
        (shape.size[1] / cell).ceil() as i64,
    ];
    let seed = layer.seed.wrapping_mul(31).wrapping_add(17);
    for cz in 0..cells[1] {
        for cx in 0..cells[0] {
            let n = (cx as u32).wrapping_mul(0x9E37_79B9) ^ (cz as u32).wrapping_mul(0x85EB_CA6B);
            // Jitter inside the middle of the cell, so neighbours keep apart.
            let x = origin[0] + (cx as f32 + 0.15 + 0.7 * unit(hash(seed, n, 0))) * cell;
            let z = origin[1] + (cz as f32 + 0.15 + 0.7 * unit(hash(seed, n, 1))) * cell;
            if !shape.contains(x, z) || ground.hole(x, z) {
                continue;
            }
            let clump = if layer.clumping > 0.0 {
                let v = value_noise(x / layer.clump_size, z / layer.clump_size, seed ^ 0x5EED);
                let bunched = ((v - 0.35) / 0.3).clamp(0.0, 1.0) * 2.0;
                1.0 + (bunched - 1.0) * layer.clumping
            } else {
                1.0
            };
            let index = ground.nearest(x, z);
            let p = chance
                * clump
                * ground.layer(layer.layer, x, z)
                * Ground::density(density_map, index);
            if unit(hash(seed, n, 2)) >= p {
                continue;
            }
            let normal = ground.field.normal_at(shape, x, z);
            let slope = normal[1].clamp(-1.0, 1.0).acos().to_degrees();
            if slope < layer.slope[0] || slope > layer.slope[1] {
                continue;
            }
            let Some(y) = ground.field.height_at(shape, x, z) else {
                continue;
            };
            if y < layer.altitude[0] || y > layer.altitude[1] {
                continue;
            }
            let scale = layer.scale[0] + (layer.scale[1] - layer.scale[0]) * unit(hash(seed, n, 3));
            if avoid.iter().any(|a| {
                let (dx, dz) = (x - a.center[0], z - a.center[1]);
                let keep = a.radius + layer.radius * scale;
                dx * dx + dz * dz < keep * keep
            }) {
                continue;
            }
            let yaw = glam::Quat::from_rotation_y(unit(hash(seed, n, 4)) * std::f32::consts::TAU);
            let up = glam::Vec3::Y
                .lerp(glam::Vec3::from(normal), layer.align)
                .normalize();
            let tilt = glam::Quat::from_rotation_arc(glam::Vec3::Y, up);
            out.push(Instance {
                position: [x, y - layer.sink * scale, z],
                rotation: (tilt * yaw).to_array(),
                scale,
                tint: 1.0 + (unit(hash(seed, n, 5)) * 2.0 - 1.0) * layer.tint,
            });
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> (Heightfield, Shape) {
        (
            Heightfield::flat(129, 0.5),
            Shape {
                size: [64.0, 64.0],
                height: 10.0,
                side: 129,
            },
        )
    }

    #[test]
    fn grass_fills_to_its_density_and_tiles_without_overlap() {
        let (field, shape) = flat();
        let ground = Ground {
            field: &field,
            shape: &shape,
            weights: None,
            holes: None,
        };
        let grass = GrassLayer {
            density: 4.0,
            ..GrassLayer::default()
        };
        let all = grass_blades(&ground, &grass, None, [-32.0; 2], [32.0; 2], usize::MAX);
        // 64 x 64 m at 4 a square metre.
        assert!((all.len() as f32 - 16384.0).abs() < 200.0, "{}", all.len());
        let left = grass_blades(
            &ground,
            &grass,
            None,
            [-32.0, -32.0],
            [0.0, 32.0],
            usize::MAX,
        );
        let right = grass_blades(
            &ground,
            &grass,
            None,
            [0.0, -32.0],
            [32.0, 32.0],
            usize::MAX,
        );
        assert_eq!(left.len() + right.len(), all.len());
        assert!(all.iter().all(|b| (b.root[1] - 5.0).abs() < 1e-4));
    }

    #[test]
    fn grass_keeps_to_its_layer_and_out_of_holes() {
        let (field, shape) = flat();
        let side = shape.side as usize;
        let mut weights = vec![[255u8, 0, 0, 0]; side * side];
        for j in 0..side {
            for i in side / 2..side {
                weights[j * side + i] = [0, 255, 0, 0];
            }
        }
        let mut holes = vec![0u8; side * side];
        for j in 0..side / 4 {
            for i in 0..side {
                holes[j * side + i] = 255;
            }
        }
        let ground = Ground {
            field: &field,
            shape: &shape,
            weights: Some(&weights),
            holes: Some(&holes),
        };
        let grass = GrassLayer {
            layer: 1,
            density: 2.0,
            ..GrassLayer::default()
        };
        let blades = grass_blades(&ground, &grass, None, [-32.0; 2], [32.0; 2], usize::MAX);
        assert!(!blades.is_empty());
        assert!(blades.iter().all(|b| b.root[0] > -0.5 && b.root[2] > -16.5));
    }

    #[test]
    fn a_grass_mesh_has_five_vertices_a_blade() {
        let (field, shape) = flat();
        let ground = Ground {
            field: &field,
            shape: &shape,
            weights: None,
            holes: None,
        };
        let grass = GrassLayer::default();
        let blades = grass_blades(&ground, &grass, None, [0.0; 2], [2.0; 2], usize::MAX);
        let mesh = grass_mesh(&blades, &grass, [0.0, 5.0, 0.0]);
        assert_eq!(mesh.positions.len(), blades.len() * 5);
        assert_eq!(mesh.indices.len(), blades.len() * 9);
        let tip = mesh.positions[4];
        assert!(tip[1] > 0.2, "tips stand up: {tip:?}");
    }

    #[test]
    fn scatter_respects_spacing_slope_and_footprints() {
        let (field, shape) = flat();
        let ground = Ground {
            field: &field,
            shape: &shape,
            weights: None,
            holes: None,
        };
        let layer = ScatterLayer {
            density: 100.0,
            spacing: 2.0,
            clumping: 0.0,
            ..ScatterLayer::default()
        };
        let avoid = [Avoid {
            center: [0.0, 0.0],
            radius: 10.0,
        }];
        let trees = scatter_instances(&ground, &layer, None, &avoid, usize::MAX);
        assert!(trees.len() > 500, "{}", trees.len());
        for (k, a) in trees.iter().enumerate() {
            let r = (a.position[0].powi(2) + a.position[2].powi(2)).sqrt();
            assert!(r >= 10.0);
            for b in &trees[k + 1..] {
                let d = ((a.position[0] - b.position[0]).powi(2)
                    + (a.position[2] - b.position[2]).powi(2))
                .sqrt();
                assert!(d >= 0.59, "too close: {d}");
            }
        }
        let steep_only = ScatterLayer {
            slope: [20.0, 90.0],
            ..layer.clone()
        };
        assert!(scatter_instances(&ground, &steep_only, None, &[], usize::MAX).is_empty());
        assert_eq!(scatter_instances(&ground, &layer, None, &[], 10).len(), 10);
        // Same inputs, same forest.
        assert_eq!(
            trees,
            scatter_instances(&ground, &layer, None, &avoid, usize::MAX)
        );
    }

    #[test]
    fn a_density_map_limits_scatter_to_what_was_painted() {
        let (field, shape) = flat();
        let side = shape.side as usize;
        let mut map = vec![0u8; side * side];
        for j in 0..side {
            for i in 0..side / 2 {
                map[j * side + i] = 255;
            }
        }
        let ground = Ground {
            field: &field,
            shape: &shape,
            weights: None,
            holes: None,
        };
        let layer = ScatterLayer {
            density: 50.0,
            ..ScatterLayer::default()
        };
        let trees = scatter_instances(&ground, &layer, Some(&map), &[], usize::MAX);
        assert!(!trees.is_empty());
        assert!(trees.iter().all(|t| t.position[0] < 0.5));
    }
}
