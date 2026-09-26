//! Persisted volumetric cloud settings. Distances are metres except tiling.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudQuality {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}
impl CloudQuality {
    pub fn steps(self) -> (u32, u32) {
        match self {
            Self::Low => (16, 3),
            Self::Medium => (32, 5),
            Self::High => (48, 6),
            Self::Ultra => (64, 8),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clouds {
    pub enabled: bool,
    pub quality: CloudQuality,
    pub coverage: f32,
    pub density: f32,
    pub cloud_type: f32,
    pub bottom: f32,
    pub top: f32,
    pub tiling_km: f32,
    pub shear: [f32; 2],
    pub toe: f32,
    pub shoulder: f32,
    pub detail_scale: f32,
    pub erosion: f32,
    pub detail_speed: f32,
    pub anvil: f32,
    pub billow: f32,
    pub feather: f32,
    pub forward: f32,
    pub backward: f32,
    pub back_blend: f32,
    pub powder: f32,
    pub lightbleed: f32,
    pub ambient: f32,
    pub bottom_occlusion: f32,
    pub shadows: bool,
    pub sun_shadows: bool,
    pub moon_shadows: bool,
    pub shadow_strength: f32,
    pub shadow_range: f32,
    pub threshold: f32,
    /// Authored shape noise (a volume asset, red = Perlin-Worley), or empty
    /// to bake it from the cloud seed.
    pub shape_volume: String,
    /// Authored erosion noise (RGB = Worley octaves), or empty to bake it.
    pub detail_volume: String,
}
impl Default for Clouds {
    fn default() -> Self {
        Self {
            enabled: false,
            quality: CloudQuality::High,
            coverage: 0.5,
            density: 0.8,
            cloud_type: 0.7,
            bottom: 1500.0,
            top: 3500.0,
            tiling_km: 12.0,
            shear: [0.0; 2],
            toe: 0.2,
            shoulder: 0.8,
            detail_scale: 6.0,
            erosion: 0.35,
            detail_speed: 1.0,
            anvil: 0.3,
            billow: 0.15,
            feather: 0.2,
            forward: 0.8,
            backward: -0.2,
            back_blend: 0.2,
            powder: 1.0,
            lightbleed: 0.3,
            ambient: 1.0,
            bottom_occlusion: 0.65,
            shadows: true,
            sun_shadows: true,
            moon_shadows: true,
            shadow_strength: 0.7,
            shadow_range: 20000.0,
            threshold: 0.01,
            shape_volume: String::new(),
            detail_volume: String::new(),
        }
    }
}
impl Clouds {
    pub fn normalize(&mut self) {
        fn clamp(v: &mut f32, default: f32, lo: f32, hi: f32) {
            *v = if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default.clamp(lo, hi)
            };
        }
        let d = Self::default();
        macro_rules! c {
            ($f:ident,$lo:expr,$hi:expr) => {
                clamp(&mut self.$f, d.$f, $lo, $hi);
            };
        }
        c!(coverage, 0.0, 1.0);
        c!(density, 0.0, 10.0);
        c!(cloud_type, 0.0, 1.0);
        c!(bottom, -10000.0, 100000.0);
        c!(top, self.bottom + 10.0, 200000.0);
        c!(tiling_km, 0.1, 1000.0);
        c!(toe, 0.0, 0.95);
        c!(shoulder, self.toe + 0.01, 1.0);
        c!(detail_scale, 1.0, 64.0);
        c!(erosion, 0.0, 1.0);
        c!(detail_speed, 0.0, 20.0);
        c!(anvil, 0.0, 1.0);
        c!(billow, 0.01, 0.5);
        c!(feather, 0.01, 0.5);
        c!(forward, 0.0, 0.95);
        c!(backward, -0.95, 0.0);
        c!(back_blend, 0.0, 1.0);
        c!(powder, 0.0, 2.0);
        c!(lightbleed, 0.0, 1.0);
        c!(ambient, 0.0, 10.0);
        c!(bottom_occlusion, 0.0, 1.0);
        c!(shadow_strength, 0.0, 1.0);
        c!(shadow_range, 100.0, 100000.0);
        c!(threshold, 0.001, 0.2);
        for v in &mut self.shear {
            clamp(v, 0.0, -10000.0, 10000.0);
        }
    }
}
/// The two noise volumes the clouds sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudNoise {
    /// Perlin-Worley in red, three Worley octaves in green, blue and alpha.
    Shape,
    /// Three Worley octaves in red, green and blue, for erosion.
    Detail,
}
impl CloudNoise {
    /// The side the runtime bakes this volume at.
    pub fn size(self) -> u32 {
        match self {
            Self::Shape => 128,
            Self::Detail => 32,
        }
    }
    /// Where `bake-cloud-noise` writes this volume.
    pub fn asset_path(self) -> &'static str {
        match self {
            Self::Shape => "assets/clouds/shape.png",
            Self::Detail => "assets/clouds/detail.png",
        }
    }
}

// CPU twin of `cloud_bake.wesl` and `blockloom::hash`, so a baked asset
// matches what the GPU would have made from the same seed.
fn pcg3(v: [u32; 3]) -> [u32; 3] {
    let mut h = v.map(|x| x.wrapping_mul(1664525).wrapping_add(1013904223));
    let mix = |h: &mut [u32; 3]| {
        h[0] = h[0].wrapping_add(h[1].wrapping_mul(h[2]));
        h[1] = h[1].wrapping_add(h[2].wrapping_mul(h[0]));
        h[2] = h[2].wrapping_add(h[0].wrapping_mul(h[1]));
    };
    mix(&mut h);
    h = h.map(|x| x ^ (x >> 16));
    mix(&mut h);
    h
}
fn unit(x: u32) -> f32 {
    (x >> 8) as f32 * (1.0 / 16777216.0)
}
fn cell_hash(cell: [i32; 3], period: i32, seed: u32) -> [f32; 3] {
    let offset = (seed & 65535) as i32;
    let wrapped = cell.map(|c| ((c % period) + period) % period + offset);
    pcg3(wrapped.map(|c| c as u32)).map(unit)
}
fn worley(p: [f32; 3], period: i32, seed: u32) -> f32 {
    let cell = p.map(|v| v.floor() as i32);
    let f = [0, 1, 2].map(|i| p[i] - p[i].floor());
    let mut d = 3.0f32;
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                let o = [x, y, z];
                let h = cell_hash([0, 1, 2].map(|i| cell[i] + o[i]), period, seed);
                let delta = [0, 1, 2].map(|i| o[i] as f32 + h[i] - f[i]);
                d = d.min(delta.iter().map(|v| v * v).sum());
            }
        }
    }
    1.0 - d.sqrt().clamp(0.0, 1.0)
}
fn perlin(p: [f32; 3], period: i32, seed: u32) -> f32 {
    let cell = p.map(|v| v.floor() as i32);
    let f = [0, 1, 2].map(|i| p[i] - p[i].floor());
    let s = f.map(|f| f * f * f * (f * (f * 6.0 - 15.0) + 10.0));
    let mut n = 0.0;
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let o = [x, y, z];
                let h = cell_hash([0, 1, 2].map(|i| cell[i] + o[i]), period, seed);
                let g = h.map(|v| v * 2.0 - 1.0 + 0.0001);
                let len = g.iter().map(|v| v * v).sum::<f32>().sqrt();
                let mut w = 1.0;
                let mut dot = 0.0;
                for i in 0..3 {
                    let v = o[i] as f32;
                    w *= if o[i] == 0 { 1.0 - s[i] } else { s[i] };
                    dot += g[i] / len * (f[i] - v);
                }
                n += dot * w;
            }
        }
    }
    n * 0.5 + 0.5
}
fn noise_texel(kind: CloudNoise, uv: [f32; 3], seed: u32) -> [f32; 4] {
    let at = |scale: f32| uv.map(|v| v * scale);
    let w = [
        worley(at(4.0), 4, seed),
        worley(at(8.0), 8, seed),
        worley(at(16.0), 16, seed),
    ];
    match kind {
        CloudNoise::Detail => [w[0], w[1], w[2], 1.0],
        CloudNoise::Shape => {
            let p = perlin(at(4.0), 4, seed) * 0.625
                + perlin(at(8.0), 8, seed) * 0.25
                + perlin(at(16.0), 16, seed) * 0.125;
            let fbm = w[0] * 0.625 + w[1] * 0.25 + w[2] * 0.125;
            [(p + (1.0 - p) * fbm).clamp(0.0, 1.0), w[0], w[1], w[2]]
        }
    }
}
/// Bakes a tileable noise volume, x fastest, then y, then z.
pub fn bake_noise(kind: CloudNoise, size: u32, seed: u32) -> Vec<[u8; 4]> {
    let side = size.max(1) as usize;
    let mut texels = vec![[0u8; 4]; side * side * side];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let per = side.div_ceil(threads).max(1) * side * side;
    std::thread::scope(|scope| {
        for (chunk, out) in texels.chunks_mut(per).enumerate() {
            scope.spawn(move || {
                for (i, texel) in out.iter_mut().enumerate() {
                    let index = chunk * per + i;
                    let id = [index % side, index / side % side, index / (side * side)];
                    let uv = id.map(|v| (v as f32 + 0.5) / side as f32);
                    *texel = noise_texel(kind, uv, seed)
                        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            });
        }
    });
    texels
}
/// A baked volume as an image strip (`size` slices side by side), the layout
/// the volume import role reads back.
pub fn noise_strip(kind: CloudNoise, size: u32, seed: u32) -> image::RgbaImage {
    let side = size.max(2);
    let texels = bake_noise(kind, side, seed);
    image::RgbaImage::from_fn(side * side, side, |px, y| {
        let (z, x) = (px / side, px % side);
        image::Rgba(texels[(x + y * side + z * side * side) as usize])
    })
}

/// Which dial `set clouds _ to` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CloudProperty {
    Coverage,
    Density,
    Type,
}
impl CloudProperty {
    pub fn name(self) -> &'static str {
        match self {
            Self::Coverage => "Coverage",
            Self::Density => "Density",
            Self::Type => "Type",
        }
    }
    /// Case-insensitive, so a script's `"coverage"` works too.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "coverage" | "cover" => Some(Self::Coverage),
            "density" => Some(Self::Density),
            "type" | "cloud_type" => Some(Self::Type),
            _ => None,
        }
    }
}
/// What `set clouds` set this run, laid over the blended clouds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CloudOverrides {
    pub coverage: Option<f32>,
    pub density: Option<f32>,
    pub cloud_type: Option<f32>,
}
impl CloudOverrides {
    /// Clamped as the project's own would be. A value that isn't finite is
    /// ignored.
    pub fn set(&mut self, property: CloudProperty, value: f32) {
        if !value.is_finite() {
            return;
        }
        match property {
            CloudProperty::Coverage => self.coverage = Some(value.clamp(0.0, 1.0)),
            CloudProperty::Density => self.density = Some(value.clamp(0.0, 10.0)),
            CloudProperty::Type => self.cloud_type = Some(value.clamp(0.0, 1.0)),
        }
    }
    pub fn apply(&self, clouds: &mut Clouds) {
        if let Some(v) = self.coverage {
            clouds.coverage = v;
        }
        if let Some(v) = self.density {
            clouds.density = v;
        }
        if let Some(v) = self.cloud_type {
            clouds.cloud_type = v;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_documents_and_bad_inputs() {
        assert_eq!(
            serde_json::from_str::<Clouds>("{}").unwrap(),
            Clouds::default()
        );
        let mut c = Clouds {
            bottom: 4000.0,
            top: 1000.0,
            coverage: f32::NAN,
            shoulder: -3.0,
            ..Default::default()
        };
        c.normalize();
        assert_eq!(c.top, 4010.0);
        assert_eq!(c.coverage, 0.5);
        c.top = f32::NAN;
        c.normalize();
        assert!(c.top > c.bottom);
        assert!(c.shoulder > c.toe);
        assert_eq!(
            serde_json::from_str::<Clouds>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
    }
    #[test]
    fn quality_budgets() {
        assert_eq!(
            [
                CloudQuality::Low,
                CloudQuality::Medium,
                CloudQuality::High,
                CloudQuality::Ultra
            ]
            .map(CloudQuality::steps),
            [(16, 3), (32, 5), (48, 6), (64, 8)]
        );
    }
    #[test]
    fn run_overrides_clamp_and_apply() {
        let mut over = CloudOverrides::default();
        over.set(CloudProperty::parse("coverage").unwrap(), 3.0);
        over.set(CloudProperty::Density, f32::NAN);
        over.set(CloudProperty::parse(" Type ").unwrap(), 0.25);
        let mut c = Clouds::default();
        over.apply(&mut c);
        assert_eq!(c.coverage, 1.0);
        assert_eq!(c.density, Clouds::default().density);
        assert_eq!(c.cloud_type, 0.25);
        assert_eq!(CloudProperty::parse("rain"), None);
    }
    #[test]
    fn baked_noise_tiles_and_round_trips_through_a_strip() {
        let seed = 7;
        // Worley and Perlin wrap at their period, so the faces meet.
        let a = noise_texel(CloudNoise::Shape, [0.0, 0.3, 0.6], seed);
        let b = noise_texel(CloudNoise::Shape, [1.0, 0.3, 0.6], seed);
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-4, "{a:?} vs {b:?}");
        }
        let texels = bake_noise(CloudNoise::Detail, 8, seed);
        assert!(texels.iter().any(|t| t[0] != texels[0][0]));
        assert!(texels.iter().all(|t| t[3] == 255));
        assert_ne!(bake_noise(CloudNoise::Detail, 8, seed + 1), texels);
        let strip = noise_strip(CloudNoise::Detail, 8, seed);
        let volume = crate::pipeline::volume::volume_from_strip("detail.png", &strip).unwrap();
        assert_eq!(volume.info.size, [8, 8, 8]);
        assert_eq!(
            volume.at(3, 5, 6),
            texels[3 + 5 * 8 + 6 * 64].map(|c| c as f32 / 255.0)
        );
    }
}
