//! Heightmap terrain: the `Terrain` component, the height grid it is built
//! from, and everything worked out from that grid on the CPU.
//!
//! - [`TerrainSpec`] is what the document saves: size, resolution, painted
//!   layers with their slope/height/curvature rules, grass and scatter
//!   layers, and the names of the grids in the project's terrain store.
//! - [`Heightfield`] is the grid itself, 0-1 per sample, and [`Shape`] turns
//!   sample indices into metres centred on the actor.
//! - [`store`] keeps grids as content-addressed tiles, so an undo step is just
//!   an older manifest name and a stroke only writes the tiles it touched.
//! - [`sculpt`] is the brushes and erosion, [`mesh`] the chunked LOD meshes,
//!   [`scatter`] where grass blades and trees stand.
//!
//! Samples run along X by column and Z by row, row 0 at the north (-Z) edge,
//! which is how a heightmap image reads with north up.

pub mod mesh;
pub mod scatter;
pub mod sculpt;
pub mod store;

use crate::material::SurfaceDetail;
use serde::{Deserialize, Serialize};

pub use scatter::{GrassLayer, ScatterLayer, ScatterShape};

/// Paint layers per terrain: one RGBA weight texel holds them all.
pub const MAX_LAYERS: usize = 4;
pub const MIN_RESOLUTION: u32 = 129;
pub const MAX_RESOLUTION: u32 = 4097;

/// A terrain actor's ground. Centred on the actor: X and Z run
/// `-size/2..size/2`, and a full-scale sample stands `height` metres up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerrainSpec {
    /// Metres along X and Z.
    pub size: [f32; 2],
    /// Metres a sample of 1 stands above the actor.
    pub height: f32,
    /// Samples per side: 2ⁿ+1, 129 to 4097.
    pub resolution: u32,
    /// The height grid's manifest in the terrain store. Empty is flat.
    pub heights: String,
    /// Painted layer weights (RGBA8, one channel per layer). Empty leaves
    /// the rules alone.
    pub splat: String,
    /// Holes (8-bit, above half cuts the cell out) for caves and tunnels.
    pub holes: String,
    pub layers: Vec<TerrainLayer>,
    /// Screen pixels a chunk's simplified shape may stray from the full one
    /// before the next finer level is drawn.
    pub pixel_error: f32,
    /// Whether bodies collide with the ground.
    pub collision: bool,
    /// Anti-tiling, macro/detail and masks, laid over every layer.
    pub texturing: SurfaceDetail,
    pub grass: Vec<GrassLayer>,
    pub scatter: Vec<ScatterLayer>,
}

impl Default for TerrainSpec {
    fn default() -> Self {
        Self {
            size: [256.0, 256.0],
            height: 40.0,
            resolution: 257,
            heights: String::new(),
            splat: String::new(),
            holes: String::new(),
            layers: vec![TerrainLayer::default()],
            pixel_error: 4.0,
            collision: true,
            texturing: SurfaceDetail::default(),
            grass: Vec::new(),
            scatter: Vec::new(),
        }
    }
}

impl TerrainSpec {
    /// Clamp every dial into its live range, in place.
    pub fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.size = self.size.map(|v| finite(v, 256.0).clamp(1.0, 100_000.0));
        self.height = finite(self.height, 40.0).clamp(0.0, 20_000.0);
        self.resolution = snap_resolution(self.resolution);
        self.pixel_error = finite(self.pixel_error, 4.0).clamp(0.25, 64.0);
        self.heights = self.heights.trim().to_string();
        self.splat = self.splat.trim().to_string();
        self.holes = self.holes.trim().to_string();
        if self.layers.is_empty() {
            self.layers.push(TerrainLayer::default());
        }
        self.layers.truncate(MAX_LAYERS);
        for layer in &mut self.layers {
            layer.normalize();
        }
        self.texturing.normalize();
        for grass in &mut self.grass {
            grass.normalize(self.layers.len());
        }
        for scatter in &mut self.scatter {
            scatter.normalize(self.layers.len());
        }
    }

    pub fn shape(&self) -> Shape {
        Shape {
            size: self.size,
            height: self.height,
            side: self.resolution,
        }
    }
}

/// The nearest valid resolution: a power of two plus one, in range.
pub fn snap_resolution(resolution: u32) -> u32 {
    let quads = resolution.saturating_sub(1).max(1);
    let lower = 1u32 << (31 - quads.leading_zeros());
    let upper = lower.saturating_mul(2);
    let nearest = if quads - lower <= upper - quads {
        lower
    } else {
        upper
    };
    (nearest + 1).clamp(MIN_RESOLUTION, MAX_RESOLUTION)
}

/// One paint layer: what the ground looks like wherever it wins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerrainLayer {
    pub name: String,
    /// Tint, and the whole look when there is no albedo map.
    pub color: String,
    pub albedo_texture: String,
    pub normal_texture: String,
    pub roughness_texture: String,
    pub roughness: f32,
    /// Texture repeats per metre.
    pub texel_density: f32,
    pub rules: LayerRules,
}

impl Default for TerrainLayer {
    fn default() -> Self {
        Self {
            name: "Grass".to_string(),
            color: "#5E7D3A".to_string(),
            albedo_texture: String::new(),
            normal_texture: String::new(),
            roughness_texture: String::new(),
            roughness: 0.85,
            texel_density: 0.25,
            rules: LayerRules::default(),
        }
    }
}

impl TerrainLayer {
    fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        self.albedo_texture = self.albedo_texture.trim().to_string();
        self.normal_texture = self.normal_texture.trim().to_string();
        self.roughness_texture = self.roughness_texture.trim().to_string();
        self.roughness = finite(self.roughness, 0.85).clamp(0.0, 1.0);
        self.texel_density = finite(self.texel_density, 0.25).clamp(0.001, 1024.0);
        self.rules.normalize();
    }
}

/// Where a layer covers the ground by itself, before any paint: between two
/// slopes, between two heights, and on ridges or in hollows. Layers stack in
/// order, so a later layer's rule covers an earlier one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerRules {
    pub enabled: bool,
    /// Degrees from flat.
    pub slope: [f32; 2],
    /// Metres above the actor.
    pub height: [f32; 2],
    /// -1 wants hollows, 1 wants ridges, 0 doesn't mind.
    pub curvature: f32,
    /// How soft the rule's edges are, 0 (hard) to 1.
    pub softness: f32,
}

impl Default for LayerRules {
    fn default() -> Self {
        Self {
            enabled: false,
            slope: [0.0, 90.0],
            height: [-100_000.0, 100_000.0],
            curvature: 0.0,
            softness: 0.3,
        }
    }
}

impl LayerRules {
    fn normalize(&mut self) {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let mut slope = self.slope.map(|v| finite(v, 0.0).clamp(0.0, 90.0));
        if slope[0] > slope[1] {
            slope.swap(0, 1);
        }
        self.slope = slope;
        let mut height = self
            .height
            .map(|v| finite(v, 0.0).clamp(-100_000.0, 100_000.0));
        if height[0] > height[1] {
            height.swap(0, 1);
        }
        self.height = height;
        self.curvature = finite(self.curvature, 0.0).clamp(-1.0, 1.0);
        self.softness = finite(self.softness, 0.3).clamp(0.0, 1.0);
    }

    /// Coverage 0-1 at a sample `slope` degrees from flat, `height` metres
    /// up, with `curvature` -1 (ridge) to 1 (hollow).
    pub fn weight(&self, slope: f32, height: f32, curvature: f32) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let soft_slope = 1.0 + self.softness * 15.0;
        let soft_height = 0.01 + self.softness * 10.0;
        let band = |v: f32, [lo, hi]: [f32; 2], soft: f32| {
            let rise = ((v - lo) / soft + 0.5).clamp(0.0, 1.0);
            let fall = ((hi - v) / soft + 0.5).clamp(0.0, 1.0);
            rise.min(fall)
        };
        let mut weight =
            band(slope, self.slope, soft_slope) * band(height, self.height, soft_height);
        if self.curvature != 0.0 {
            // A positive `curvature` dial wants ridges, which read negative.
            let want = (-curvature * self.curvature.signum() * 4.0 + 0.5).clamp(0.0, 1.0);
            weight *= 1.0 - self.curvature.abs() + self.curvature.abs() * want;
        }
        weight
    }
}

/// Sample indices to metres, for one terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub size: [f32; 2],
    pub height: f32,
    pub side: u32,
}

impl Shape {
    /// Metres between neighbouring samples along X and Z.
    pub fn spacing(&self) -> [f32; 2] {
        let quads = (self.side - 1).max(1) as f32;
        [self.size[0] / quads, self.size[1] / quads]
    }

    /// Where sample `(i, j)` stands, X and Z, relative to the actor.
    pub fn local(&self, i: f32, j: f32) -> [f32; 2] {
        let spacing = self.spacing();
        [
            -self.size[0] * 0.5 + i * spacing[0],
            -self.size[1] * 0.5 + j * spacing[1],
        ]
    }

    /// The fractional sample under a local X, Z.
    pub fn sample(&self, x: f32, z: f32) -> [f32; 2] {
        let spacing = self.spacing();
        [
            (x + self.size[0] * 0.5) / spacing[0],
            (z + self.size[1] * 0.5) / spacing[1],
        ]
    }

    pub fn contains(&self, x: f32, z: f32) -> bool {
        x.abs() <= self.size[0] * 0.5 && z.abs() <= self.size[1] * 0.5
    }
}

/// A square grid of heights, 0-1, row-major from the north-west corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Heightfield {
    pub side: u32,
    pub samples: Vec<f32>,
}

impl Heightfield {
    pub fn flat(side: u32, level: f32) -> Self {
        Self {
            side,
            samples: vec![level; (side * side) as usize],
        }
    }

    #[inline]
    pub fn index(&self, i: u32, j: u32) -> usize {
        (j * self.side + i) as usize
    }

    #[inline]
    pub fn at(&self, i: u32, j: u32) -> f32 {
        self.samples[self.index(i, j)]
    }

    /// The sample at `(i, j)`, clamped onto the grid.
    #[inline]
    pub fn clamped(&self, i: i64, j: i64) -> f32 {
        let last = self.side as i64 - 1;
        self.at(i.clamp(0, last) as u32, j.clamp(0, last) as u32)
    }

    /// Bilinear height at a fractional sample, clamped onto the grid.
    pub fn bilinear(&self, i: f32, j: f32) -> f32 {
        let last = (self.side - 1) as f32;
        let i = i.clamp(0.0, last);
        let j = j.clamp(0.0, last);
        let (i0, j0) = (i.floor(), j.floor());
        let (fi, fj) = (i - i0, j - j0);
        let (i0, j0) = (i0 as i64, j0 as i64);
        let a = self.clamped(i0, j0);
        let b = self.clamped(i0 + 1, j0);
        let c = self.clamped(i0, j0 + 1);
        let d = self.clamped(i0 + 1, j0 + 1);
        let top = a + (b - a) * fi;
        let bottom = c + (d - c) * fi;
        top + (bottom - top) * fj
    }

    /// The same ground at another resolution.
    pub fn resample(&self, side: u32) -> Heightfield {
        if side == self.side {
            return self.clone();
        }
        let scale = (self.side - 1) as f32 / (side - 1).max(1) as f32;
        let mut samples = Vec::with_capacity((side * side) as usize);
        for j in 0..side {
            for i in 0..side {
                samples.push(self.bilinear(i as f32 * scale, j as f32 * scale));
            }
        }
        Heightfield { side, samples }
    }

    /// A decoded heightmap stretched onto a `side` grid. A non-square map
    /// is stretched on each axis.
    pub fn from_heightmap(map: &crate::pipeline::height::Heightmap, side: u32) -> Heightfield {
        let (w, h) = (map.info.width.max(1), map.info.height.max(1));
        let at = |x: i64, y: i64| {
            let x = x.clamp(0, w as i64 - 1) as u32;
            let y = y.clamp(0, h as i64 - 1) as u32;
            map.samples[(y * w + x) as usize]
        };
        let sx = (w - 1) as f32 / (side - 1).max(1) as f32;
        let sy = (h - 1) as f32 / (side - 1).max(1) as f32;
        let mut samples = Vec::with_capacity((side * side) as usize);
        for j in 0..side {
            for i in 0..side {
                let (x, y) = (i as f32 * sx, j as f32 * sy);
                let (x0, y0) = (x.floor(), y.floor());
                let (fx, fy) = (x - x0, y - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * fx;
                let bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * fx;
                samples.push((top + (bottom - top) * fy).clamp(0.0, 1.0));
            }
        }
        Heightfield { side, samples }
    }

    /// Metres above the actor at a local X, Z, or `None` off the edge.
    pub fn height_at(&self, shape: &Shape, x: f32, z: f32) -> Option<f32> {
        if !shape.contains(x, z) {
            return None;
        }
        let [i, j] = shape.sample(x, z);
        Some(self.bilinear(i, j) * shape.height)
    }

    /// The surface normal at sample `(i, j)`, from central differences.
    pub fn normal(&self, shape: &Shape, i: u32, j: u32) -> [f32; 3] {
        let (i, j) = (i as i64, j as i64);
        let spacing = shape.spacing();
        let dx =
            (self.clamped(i + 1, j) - self.clamped(i - 1, j)) * shape.height / (2.0 * spacing[0]);
        let dz =
            (self.clamped(i, j + 1) - self.clamped(i, j - 1)) * shape.height / (2.0 * spacing[1]);
        let n = glam::Vec3::new(-dx, 1.0, -dz).normalize();
        n.to_array()
    }

    /// The normal at a local X, Z, interpolated between samples.
    pub fn normal_at(&self, shape: &Shape, x: f32, z: f32) -> [f32; 3] {
        let [i, j] = shape.sample(x, z);
        let spacing = shape.spacing();
        let h = |di: f32, dj: f32| self.bilinear(i + di, j + dj) * shape.height;
        let dx = (h(1.0, 0.0) - h(-1.0, 0.0)) / (2.0 * spacing[0]);
        let dz = (h(0.0, 1.0) - h(0.0, -1.0)) / (2.0 * spacing[1]);
        glam::Vec3::new(-dx, 1.0, -dz).normalize().to_array()
    }

    /// Degrees from flat at sample `(i, j)`.
    pub fn slope(&self, shape: &Shape, i: u32, j: u32) -> f32 {
        self.normal(shape, i, j)[1]
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    /// How much sample `(i, j)` sits in a hollow (towards 1) or on a ridge
    /// (towards -1), measured over a few metres whatever the resolution.
    pub fn curvature(&self, shape: &Shape, i: u32, j: u32) -> f32 {
        let spacing = shape.spacing()[0].min(shape.spacing()[1]).max(1e-4);
        let reach = ((2.0 / spacing).round() as i64).clamp(1, 8);
        let (i, j) = (i as i64, j as i64);
        let around = (self.clamped(i + reach, j)
            + self.clamped(i - reach, j)
            + self.clamped(i, j + reach)
            + self.clamped(i, j - reach))
            * 0.25;
        let rise = (around - self.clamped(i, j)) * shape.height;
        (rise / (reach as f32 * spacing) * 2.0).clamp(-1.0, 1.0)
    }

    /// Every sample's curvature, packed 0-255 around 128 for a texture.
    pub fn cavity_map(&self, shape: &Shape) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.samples.len());
        for j in 0..self.side {
            for i in 0..self.side {
                out.push(self.cavity(shape, i, j));
            }
        }
        out
    }

    fn cavity(&self, shape: &Shape, i: u32, j: u32) -> u8 {
        let c = self.curvature(shape, i, j);
        ((c * 0.5 + 0.5) * 255.0).round() as u8
    }

    /// A sample's texel in the terrain's surface map: its normal as 0-255
    /// and its cavity.
    pub fn surface_texel(&self, shape: &Shape, i: u32, j: u32) -> [u8; 4] {
        let n = self.normal(shape, i, j);
        let byte = |v: f32| ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
        [byte(n[0]), byte(n[1]), byte(n[2]), self.cavity(shape, i, j)]
    }

    /// The whole surface map, row by row.
    pub fn surface_map(&self, shape: &Shape) -> Vec<[u8; 4]> {
        (0..self.side)
            .flat_map(|j| (0..self.side).map(move |i| (i, j)))
            .map(|(i, j)| self.surface_texel(shape, i, j))
            .collect()
    }

    /// The lowest and highest sample.
    pub fn range(&self) -> (f32, f32) {
        self.samples
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)))
    }
}

/// Final layer weights per sample, RGBA8: the rule stack everywhere paint
/// hasn't reached, the paint where it has. Layer 0 fills what nothing else
/// covers.
///
/// `splat` holds painted weights whose sum is how much paint a sample has
/// taken (255 is fully painted).
pub fn bake_weights(
    field: &Heightfield,
    shape: &Shape,
    layers: &[TerrainLayer],
    splat: Option<&[[u8; 4]]>,
) -> Vec<[u8; 4]> {
    let mut out = Vec::with_capacity(field.samples.len());
    for j in 0..field.side {
        for i in 0..field.side {
            out.push(bake_weight(field, shape, layers, splat, i, j));
        }
    }
    out
}

/// [`bake_weights`] for one sample, so a brush can re-bake just what it
/// touched.
pub fn bake_weight(
    field: &Heightfield,
    shape: &Shape,
    layers: &[TerrainLayer],
    splat: Option<&[[u8; 4]]>,
    i: u32,
    j: u32,
) -> [u8; 4] {
    let count = layers.len().clamp(1, MAX_LAYERS);
    let mut w = [1.0f32, 0.0, 0.0, 0.0];
    if layers.iter().skip(1).any(|layer| layer.rules.enabled) {
        let slope = field.slope(shape, i, j);
        let height = field.at(i, j) * shape.height;
        let curvature = field.curvature(shape, i, j);
        for (k, layer) in layers.iter().enumerate().take(count).skip(1) {
            let a = layer.rules.weight(slope, height, curvature);
            if a > 0.0 {
                for weight in w.iter_mut().take(k) {
                    *weight *= 1.0 - a;
                }
                w[k] = a;
            }
        }
    }
    if let Some(splat) = splat {
        let painted = splat[field.index(i, j)];
        let sum: u32 = painted.iter().map(|&v| v as u32).sum();
        if sum > 0 {
            let coverage = (sum as f32 / 255.0).min(1.0);
            for (k, weight) in w.iter_mut().enumerate() {
                let paint = if k < count {
                    painted[k] as f32 / sum as f32
                } else {
                    0.0
                };
                *weight = *weight * (1.0 - coverage) + paint * coverage;
            }
        }
    }
    pack_weights(w)
}

/// Four weights summing to 1 as bytes summing to 255.
pub fn pack_weights(w: [f32; 4]) -> [u8; 4] {
    let sum: f32 = w.iter().sum::<f32>().max(1e-6);
    let mut bytes = w.map(|v| ((v / sum) * 255.0).round().clamp(0.0, 255.0) as u8);
    // Rounding can leave the total a step off; the biggest takes the slack.
    let total: i32 = bytes.iter().map(|&b| b as i32).sum();
    let biggest = (0..4).max_by_key(|&k| bytes[k]).unwrap_or(0);
    bytes[biggest] = (bytes[biggest] as i32 + 255 - total).clamp(0, 255) as u8;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(side: u32) -> Heightfield {
        let mut field = Heightfield::flat(side, 0.0);
        for j in 0..side {
            for i in 0..side {
                let index = field.index(i, j);
                field.samples[index] = i as f32 / (side - 1) as f32;
            }
        }
        field
    }

    #[test]
    fn resolutions_snap_to_a_power_of_two_plus_one() {
        assert_eq!(snap_resolution(257), 257);
        assert_eq!(snap_resolution(300), 257);
        assert_eq!(snap_resolution(400), 513);
        assert_eq!(snap_resolution(10), MIN_RESOLUTION);
        assert_eq!(snap_resolution(100_000), MAX_RESOLUTION);
        for r in [129, 257, 513, 1025, 2049, 4097] {
            assert!(crate::pipeline::height::is_terrain_side(r));
        }
    }

    #[test]
    fn samples_map_onto_metres_centred_on_the_actor() {
        let shape = Shape {
            size: [100.0, 50.0],
            height: 10.0,
            side: 129,
        };
        assert_eq!(shape.local(0.0, 0.0), [-50.0, -25.0]);
        assert_eq!(shape.local(128.0, 128.0), [50.0, 25.0]);
        let [i, j] = shape.sample(0.0, 0.0);
        assert!((i - 64.0).abs() < 1e-4 && (j - 64.0).abs() < 1e-4);
    }

    #[test]
    fn a_ramp_reads_its_height_and_slope() {
        let field = ramp(129);
        let shape = Shape {
            size: [128.0, 128.0],
            height: 128.0,
            side: 129,
        };
        // One metre up per metre across: 45 degrees.
        assert!((field.slope(&shape, 64, 64) - 45.0).abs() < 0.1);
        let h = field.height_at(&shape, 0.0, 10.0).unwrap();
        assert!((h - 64.0).abs() < 1e-3, "{h}");
        assert!(field.height_at(&shape, 65.0, 0.0).is_none());
        let n = field.normal_at(&shape, 0.0, 0.0);
        assert!(n[0] < -0.7 && n[1] > 0.7);
    }

    #[test]
    fn resampling_keeps_the_shape() {
        let field = ramp(257);
        let small = field.resample(129);
        assert_eq!(small.side, 129);
        assert!((small.at(64, 10) - 0.5).abs() < 1e-4);
        assert!((small.at(128, 0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn hollows_read_positive_and_ridges_negative() {
        let side = 129;
        let mut field = Heightfield::flat(side, 0.5);
        let pit = field.index(64, 64);
        field.samples[pit] = 0.4;
        let peak = field.index(20, 20);
        field.samples[peak] = 0.6;
        let shape = Shape {
            size: [128.0, 128.0],
            height: 20.0,
            side,
        };
        assert!(field.curvature(&shape, 64, 64) > 0.5);
        assert!(field.curvature(&shape, 20, 20) < -0.5);
        assert!(field.curvature(&shape, 100, 100).abs() < 1e-4);
    }

    #[test]
    fn rules_stack_and_paint_overrides_them() {
        let field = ramp(129);
        let shape = Shape {
            size: [128.0, 128.0],
            height: 128.0,
            side: 129,
        };
        let mut rock = TerrainLayer {
            name: "Rock".into(),
            ..TerrainLayer::default()
        };
        rock.rules = LayerRules {
            enabled: true,
            slope: [30.0, 90.0],
            ..LayerRules::default()
        };
        let layers = vec![TerrainLayer::default(), rock];
        let weights = bake_weights(&field, &shape, &layers, None);
        // The whole ramp is 45 degrees: rock wins everywhere.
        assert_eq!(weights[field.index(64, 64)], [0, 255, 0, 0]);
        let mut splat = vec![[0u8; 4]; weights.len()];
        splat[field.index(64, 64)] = [255, 0, 0, 0];
        splat[field.index(10, 10)] = [64, 0, 0, 0];
        let painted = bake_weights(&field, &shape, &layers, Some(&splat));
        assert_eq!(painted[field.index(64, 64)], [255, 0, 0, 0]);
        let partial = painted[field.index(10, 10)];
        assert!(
            partial[0] > 50 && partial[0] < 80 && partial[1] > 170,
            "{partial:?}"
        );
        for w in painted {
            assert_eq!(w.iter().map(|&v| v as u32).sum::<u32>(), 255);
        }
    }

    #[test]
    fn an_old_document_gets_a_default_terrain() {
        let spec: TerrainSpec = serde_json::from_str("{}").unwrap();
        assert_eq!(spec, TerrainSpec::default());
        let mut odd = TerrainSpec {
            resolution: 1000,
            layers: vec![TerrainLayer::default(); 7],
            size: [f32::NAN, 10.0],
            ..TerrainSpec::default()
        };
        odd.normalize();
        assert_eq!(odd.resolution, 1025);
        assert_eq!(odd.layers.len(), MAX_LAYERS);
        assert_eq!(odd.size, [256.0, 10.0]);
    }
}
