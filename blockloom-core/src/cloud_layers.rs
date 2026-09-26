//! Planar cloud layers: flat cover drawn above and below the volumetric
//! clouds, and the whole sky's clouds when those are off. Each layer's
//! coverage is a grayscale texture, either an image asset or tileable FBM
//! baked from its seed, remapped by coverage and contrast when drawn.
use serde::{Deserialize, Serialize};

pub const MAX_LAYERS: usize = 4;
/// The side every layer's coverage is baked, resampled and painted at.
pub const COVERAGE_SIZE: u32 = 512;
/// The side flow maps are resampled to.
pub const FLOW_SIZE: u32 = 128;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudLayer {
    pub enabled: bool,
    pub name: String,
    /// A grayscale image asset (luma is coverage), or empty for FBM baked
    /// from `seed`, `scale` and `octaves`.
    pub coverage_texture: String,
    pub seed: u32,
    /// Noise cells across one tile at the first octave.
    pub scale: u32,
    pub octaves: u32,
    /// 0 clear, 0.5 the noise as is, 1 overcast.
    pub coverage: f32,
    /// Sharpness of the cloud edge, 1 soft.
    pub contrast: f32,
    /// How wide one tile of coverage is.
    pub tiling_km: f32,
    pub opacity: f32,
    /// Metres above the world's origin.
    pub altitude: f32,
    /// Apparent thickness in metres: how far thick parts shift against the
    /// view (parallax) and the sun tap is taken (self shadow).
    pub parallax: f32,
    pub tint: String,
    /// Tint of the sunlight scattered through the layer.
    pub sun_tint: String,
    /// Powder tint on thin, sunlit edges.
    pub edge_tint: String,
    /// Degrees above the horizon the layer starts to fade, then is gone.
    pub horizon_fade: [f32; 2],
    /// Tints by day, at sunset and by night, ramped on the sun's height.
    pub day_tint: String,
    pub sunset_tint: String,
    pub night_tint: String,
    /// Scroll of the layer's own, metres per second across x and z.
    pub scroll: [f32; 2],
    /// Multiplier on the wind's layer scroll (`CloudOffsets::layers`).
    pub wind: f32,
    /// An image whose red and green (0.5 still) push coverage around, or
    /// empty for none.
    pub flow_map: String,
    /// Metres the flow map moves coverage per phase.
    pub flow_strength: f32,
    /// Seconds per flow phase.
    pub flow_period: f32,
    /// Degrees per second the layer turns round `pivot`, for storm spin.
    pub spin: f32,
    /// World x and z the layer turns round.
    pub pivot: [f32; 2],
    /// How much aerial haze tints the layer with distance, 0-1.
    pub aerial: f32,
    /// Bumped by painting, so the runtime rereads a file whose path stayed.
    pub revision: u32,
}

impl Default for CloudLayer {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            coverage_texture: String::new(),
            seed: 1,
            scale: 4,
            octaves: 5,
            coverage: 0.5,
            contrast: 2.0,
            tiling_km: 20.0,
            opacity: 1.0,
            altitude: 8000.0,
            parallax: 200.0,
            tint: "#FFFFFF".into(),
            sun_tint: "#FFF4E6".into(),
            edge_tint: "#FFE0C0".into(),
            horizon_fade: [12.0, 1.0],
            day_tint: "#FFFFFF".into(),
            sunset_tint: "#FFB08A".into(),
            night_tint: "#8090B0".into(),
            scroll: [0.0; 2],
            wind: 1.0,
            flow_map: String::new(),
            flow_strength: 0.0,
            flow_period: 60.0,
            spin: 0.0,
            pivot: [0.0; 2],
            aerial: 1.0,
            revision: 0,
        }
    }
}

fn clamp(v: &mut f32, default: f32, lo: f32, hi: f32) {
    *v = if v.is_finite() { *v } else { default }.clamp(lo, hi);
}

impl CloudLayer {
    pub fn normalize(&mut self) {
        let d = Self::default();
        self.scale = self.scale.clamp(1, 64);
        self.octaves = self.octaves.clamp(1, 8);
        clamp(&mut self.coverage, d.coverage, 0.0, 1.0);
        clamp(&mut self.contrast, d.contrast, 1.0, 16.0);
        clamp(&mut self.tiling_km, d.tiling_km, 0.1, 1000.0);
        clamp(&mut self.opacity, d.opacity, 0.0, 1.0);
        clamp(&mut self.altitude, d.altitude, -10_000.0, 100_000.0);
        clamp(&mut self.parallax, d.parallax, 0.0, 10_000.0);
        clamp(&mut self.horizon_fade[0], d.horizon_fade[0], 0.0, 90.0);
        let start = self.horizon_fade[0];
        clamp(&mut self.horizon_fade[1], d.horizon_fade[1], 0.0, start);
        for v in &mut self.scroll {
            clamp(v, 0.0, -10_000.0, 10_000.0);
        }
        clamp(&mut self.wind, d.wind, 0.0, 100.0);
        clamp(&mut self.flow_strength, 0.0, 0.0, 100_000.0);
        clamp(&mut self.flow_period, d.flow_period, 0.1, 100_000.0);
        clamp(&mut self.spin, 0.0, -360.0, 360.0);
        for v in &mut self.pivot {
            clamp(v, 0.0, -1.0e7, 1.0e7);
        }
        clamp(&mut self.aerial, d.aerial, 0.0, 1.0);
    }

    /// How much of the sky this layer covers, 0-1: its remapped coverage
    /// over its own baked noise, times its opacity.
    pub fn cover(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        // The mean of the remap over uniform noise is close enough.
        let n = 17;
        let mean = (0..n)
            .map(|i| remap((i as f32 + 0.5) / n as f32, self.coverage, self.contrast))
            .sum::<f32>()
            / n as f32;
        mean * self.opacity
    }

    /// What the runtime keys this layer's coverage texture on.
    pub fn texture_key(&self) -> (String, u32, u32, u32, u32) {
        (
            self.coverage_texture.clone(),
            self.seed,
            self.scale,
            self.octaves,
            self.revision,
        )
    }

    /// Where painting writes this layer's coverage when it has none yet.
    pub fn paint_path(index: usize) -> String {
        format!("assets/clouds/layer-{}.png", index + 1)
    }
}

/// Keeps at most `MAX_LAYERS`, each normalized.
pub fn normalize(layers: &mut Vec<CloudLayer>) {
    layers.truncate(MAX_LAYERS);
    layers.iter_mut().for_each(CloudLayer::normalize);
}

/// Coverage noise to cloud: 0 at coverage 0, everywhere 1 at coverage 1,
/// the noise itself at 0.5 and contrast 1. Mirrors `cloud_layers.wesl`.
pub fn remap(noise: f32, coverage: f32, contrast: f32) -> f32 {
    let x = noise + coverage * 2.0 - 1.0;
    let t = ((x - 0.5) * contrast.max(1.0) + 0.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash2(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32)
        .wrapping_mul(0x8da6_b343)
        .wrapping_add((y as u32).wrapping_mul(0xd816_3841))
        .wrapping_add(seed.wrapping_mul(0xcb1a_b31f));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

/// Tileable 2D gradient noise in 0-1, `period` cells across 0..1.
fn gradient(p: [f32; 2], period: i32, seed: u32) -> f32 {
    let [x, y] = p.map(|v| v * period as f32);
    let (cx, cy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - cx as f32, y - cy as f32);
    let fade = |f: f32| f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let corner = |ox: i32, oy: i32| {
        let h = hash2(
            (cx + ox).rem_euclid(period),
            (cy + oy).rem_euclid(period),
            seed,
        );
        let angle = (h >> 8) as f32 * (std::f32::consts::TAU / 16_777_216.0);
        angle.cos() * (fx - ox as f32) + angle.sin() * (fy - oy as f32)
    };
    let (u, v) = (fade(fx), fade(fy));
    let a = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let b = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    ((a + (b - a) * v) * std::f32::consts::FRAC_1_SQRT_2 + 0.5).clamp(0.0, 1.0)
}

/// A layer's procedural coverage, `size` squared, rows of x. Tiles at its
/// edges, and one seed always bakes the same.
pub fn bake_coverage(layer: &CloudLayer, size: u32) -> Vec<u8> {
    let side = size.max(1) as usize;
    let (scale, octaves, seed) = (
        layer.scale.clamp(1, 64) as i32,
        layer.octaves.clamp(1, 8),
        layer.seed,
    );
    let mut out = vec![0u8; side * side];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = side.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (chunk, band) in out.chunks_mut(rows * side).enumerate() {
            scope.spawn(move || {
                for (i, texel) in band.iter_mut().enumerate() {
                    let (x, y) = (i % side, chunk * rows + i / side);
                    let p = [x, y].map(|v| (v as f32 + 0.5) / side as f32);
                    let (mut sum, mut amp, mut total) = (0.0, 1.0, 0.0);
                    for o in 0..octaves {
                        sum += gradient(p, scale << o, seed.wrapping_add(o * 101)) * amp;
                        total += amp;
                        amp *= 0.5;
                    }
                    // FBM bunches round 0.5; stretch it back towards 0-1.
                    let n = ((sum / total - 0.5) * 2.2 + 0.5).clamp(0.0, 1.0);
                    *texel = (n * 255.0).round() as u8;
                }
            });
        }
    });
    out
}

/// What a paint stroke does where it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaintTool {
    /// Adds cloud.
    Cloud,
    /// Takes cloud away.
    Eraser,
    /// Softens towards the neighbourhood.
    Blur,
    /// Smears coverage along the stroke, as wind would.
    Advect,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Brush {
    pub tool: PaintTool,
    /// Radius as a fraction of the tile.
    pub radius: f32,
    /// 0-1 per dab.
    pub strength: f32,
    /// 0 a hard disc, 1 soft all the way to the middle.
    pub falloff: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            tool: PaintTool::Cloud,
            radius: 0.05,
            strength: 0.5,
            falloff: 0.7,
        }
    }
}

/// Paints a stroke through `points` (0-1 across the tile, wrapping) into a
/// square grayscale coverage buffer.
pub fn paint(coverage: &mut [u8], size: u32, brush: &Brush, points: &[[f32; 2]]) {
    let side = size as i32;
    if side <= 0 || coverage.len() != (side * side) as usize || points.is_empty() {
        return;
    }
    let radius = (brush.radius.clamp(0.001, 0.5) * side as f32).max(0.5);
    let strength = brush.strength.clamp(0.0, 1.0);
    let hard = 1.0 - brush.falloff.clamp(0.0, 1.0);
    let at = |buf: &[u8], x: i32, y: i32| {
        buf[(y.rem_euclid(side) * side + x.rem_euclid(side)) as usize] as f32 / 255.0
    };
    // Dabs half a radius apart, so a fast stroke stays continuous.
    let mut dabs: Vec<([f32; 2], [f32; 2])> = Vec::new();
    let mut previous: Option<[f32; 2]> = None;
    for p in points.iter().map(|p| p.map(|v| v * side as f32)) {
        let Some(from) = previous else {
            dabs.push((p, [0.0; 2]));
            previous = Some(p);
            continue;
        };
        let d = [p[0] - from[0], p[1] - from[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let n = (len / (radius * 0.5)).ceil().max(1.0) as usize;
        let dir = if len > 0.0 {
            [d[0] / len, d[1] / len]
        } else {
            [0.0; 2]
        };
        for i in 1..=n {
            let t = i as f32 / n as f32;
            dabs.push(([from[0] + d[0] * t, from[1] + d[1] * t], dir));
        }
        previous = Some(p);
    }
    let reach = radius.ceil() as i32;
    for (centre, dir) in dabs {
        let source = coverage.to_vec();
        let (cx, cy) = (centre[0].floor() as i32, centre[1].floor() as i32);
        for y in cy - reach..=cy + reach {
            for x in cx - reach..=cx + reach {
                let (dx, dy) = (x as f32 + 0.5 - centre[0], y as f32 + 0.5 - centre[1]);
                let r = (dx * dx + dy * dy).sqrt() / radius;
                if r >= 1.0 {
                    continue;
                }
                let soft = if r <= hard {
                    1.0
                } else {
                    let t = (r - hard) / (1.0 - hard).max(1e-4);
                    1.0 - t * t * (3.0 - 2.0 * t)
                };
                let w = soft * strength;
                let old = at(&source, x, y);
                let target = match brush.tool {
                    PaintTool::Cloud => 1.0,
                    PaintTool::Eraser => 0.0,
                    PaintTool::Blur => {
                        let mut sum = 0.0;
                        for oy in -2..=2 {
                            for ox in -2..=2 {
                                sum += at(&source, x + ox, y + oy);
                            }
                        }
                        sum / 25.0
                    }
                    PaintTool::Advect => {
                        // Pull from upstream, so what was there moves along.
                        let back = radius * 0.35;
                        let (sx, sy) = (x as f32 - dir[0] * back, y as f32 - dir[1] * back);
                        let (x0, y0) = (sx.floor() as i32, sy.floor() as i32);
                        let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
                        let a = at(&source, x0, y0) * (1.0 - fx) + at(&source, x0 + 1, y0) * fx;
                        let b =
                            at(&source, x0, y0 + 1) * (1.0 - fx) + at(&source, x0 + 1, y0 + 1) * fx;
                        a * (1.0 - fy) + b * fy
                    }
                };
                let value = old + (target - old) * w;
                coverage[(y.rem_euclid(side) * side + x.rem_euclid(side)) as usize] =
                    (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
}

/// An image as square coverage: luma, resampled to `size`.
pub fn coverage_from_image(image: &image::DynamicImage, size: u32) -> Vec<u8> {
    let luma = image.to_luma8();
    if luma.width() == size && luma.height() == size {
        return luma.into_raw();
    }
    image::imageops::resize(&luma, size, size, image::imageops::FilterType::Triangle).into_raw()
}

/// Paints a stroke into layer `index`'s coverage file under the project
/// folder `dir`, and returns its asset path. A layer without its own painted
/// file gets one, started from what it drew before (its image or its baked
/// noise), so an imported image is never written over.
pub fn paint_file(
    dir: &std::path::Path,
    layer: &CloudLayer,
    index: usize,
    brush: &Brush,
    points: &[[f32; 2]],
) -> Result<String, String> {
    let path = CloudLayer::paint_path(index);
    let full = crate::assets::resolve(dir, &path)
        .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
    let mut coverage = if layer.coverage_texture.trim().is_empty() {
        bake_coverage(layer, COVERAGE_SIZE)
    } else {
        let source = crate::assets::resolve(dir, &layer.coverage_texture).ok_or_else(|| {
            format!(
                "\"{}\" isn't a path in this project",
                layer.coverage_texture
            )
        })?;
        let image = image::open(&source).map_err(|e| format!("{}: {e}", source.display()))?;
        coverage_from_image(&image, COVERAGE_SIZE)
    };
    paint(&mut coverage, COVERAGE_SIZE, brush, points);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    image::GrayImage::from_raw(COVERAGE_SIZE, COVERAGE_SIZE, coverage)
        .ok_or("The painted coverage came out the wrong size")?
        .save(&full)
        .map_err(|e| format!("{}: {e}", full.display()))?;
    Ok(path)
}

/// A layer's coverage at `COVERAGE_SIZE`: its image under `dir`, or its
/// baked noise when it names none.
pub fn load_coverage(dir: Option<&std::path::Path>, layer: &CloudLayer) -> Result<Vec<u8>, String> {
    let path = layer.coverage_texture.trim();
    if path.is_empty() {
        return Ok(bake_coverage(layer, COVERAGE_SIZE));
    }
    let image = open_asset(dir, path)?;
    Ok(coverage_from_image(&image, COVERAGE_SIZE))
}

/// A layer's flow map at `FLOW_SIZE`, or `None` when it has none.
pub fn load_flow(
    dir: Option<&std::path::Path>,
    layer: &CloudLayer,
) -> Result<Option<Vec<u8>>, String> {
    let path = layer.flow_map.trim();
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(flow_from_image(&open_asset(dir, path)?, FLOW_SIZE)))
}

fn open_asset(dir: Option<&std::path::Path>, path: &str) -> Result<image::DynamicImage, String> {
    let dir = dir.ok_or("This world has no project folder to read assets from")?;
    let full = crate::assets::resolve(dir, path)
        .ok_or_else(|| format!("\"{path}\" isn't a path in this project"))?;
    image::open(&full).map_err(|e| format!("{path}: {e}"))
}

/// A flow map as RGBA8 at `size`: red and green are the push, 0.5 none.
pub fn flow_from_image(image: &image::DynamicImage, size: u32) -> Vec<u8> {
    let rgba = image.to_rgba8();
    let rgba = if rgba.width() == size && rgba.height() == size {
        rgba
    } else {
        image::imageops::resize(&rgba, size, size, image::imageops::FilterType::Triangle)
    };
    rgba.into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_documents_and_bad_inputs() {
        assert_eq!(
            serde_json::from_str::<CloudLayer>("{}").unwrap(),
            CloudLayer::default()
        );
        let mut layers = vec![
            CloudLayer {
                coverage: f32::NAN,
                contrast: 0.0,
                horizon_fade: [5.0, 30.0],
                octaves: 0,
                ..Default::default()
            };
            6
        ];
        normalize(&mut layers);
        assert_eq!(layers.len(), MAX_LAYERS);
        let l = &layers[0];
        assert_eq!(l.coverage, 0.5);
        assert_eq!(l.contrast, 1.0);
        assert_eq!(l.horizon_fade, [5.0, 5.0]);
        assert_eq!(l.octaves, 1);
    }

    #[test]
    fn remap_spans_clear_to_overcast() {
        for n in [0.0, 0.3, 0.7, 1.0] {
            assert_eq!(remap(n, 0.0, 1.0), 0.0);
            assert_eq!(remap(n, 1.0, 1.0), 1.0);
            assert_eq!(remap(n, 0.0, 8.0), 0.0);
        }
        assert!((remap(0.5, 0.5, 1.0) - 0.5).abs() < 1e-6);
        assert!(remap(0.6, 0.5, 8.0) > remap(0.6, 0.5, 1.0));
        let mut layer = CloudLayer::default();
        let half = layer.cover();
        layer.coverage = 0.9;
        assert!(layer.cover() > half);
        layer.enabled = false;
        assert_eq!(layer.cover(), 0.0);
    }

    #[test]
    fn baked_coverage_tiles_and_follows_the_seed() {
        let layer = CloudLayer::default();
        let a = bake_coverage(&layer, 64);
        assert_eq!(a, bake_coverage(&layer, 64));
        assert_ne!(
            a,
            bake_coverage(
                &CloudLayer {
                    seed: 2,
                    ..layer.clone()
                },
                64
            )
        );
        let spread = a.iter().max().unwrap() - a.iter().min().unwrap();
        assert!(spread > 100, "flat noise: {spread}");
        // The noise wraps, so the first and last columns meet.
        let edge = gradient([0.0, 0.37], 4, 9);
        assert!((edge - gradient([1.0, 0.37], 4, 9)).abs() < 1e-5);
    }

    #[test]
    fn brushes_add_erase_blur_and_advect() {
        let size = 32;
        let mut buf = vec![0u8; 32 * 32];
        let brush = Brush {
            radius: 0.1,
            strength: 1.0,
            falloff: 0.0,
            tool: PaintTool::Cloud,
        };
        paint(&mut buf, size, &brush, &[[0.5, 0.5]]);
        assert_eq!(buf[16 * 32 + 16], 255);
        assert_eq!(buf[0], 0);
        // Wraps across the tile's edge.
        paint(&mut buf, size, &brush, &[[0.0, 0.0]]);
        assert_eq!(buf[31 * 32 + 31], 255);
        let erase = Brush {
            tool: PaintTool::Eraser,
            ..brush
        };
        paint(&mut buf, size, &erase, &[[0.5, 0.5]]);
        assert_eq!(buf[16 * 32 + 16], 0);
        let mut spot = vec![0u8; 32 * 32];
        spot[16 * 32 + 16] = 255;
        let blur = Brush {
            tool: PaintTool::Blur,
            ..brush
        };
        paint(&mut spot, size, &blur, &[[0.5, 0.5]]);
        assert!(spot[16 * 32 + 16] < 255 && spot[16 * 32 + 17] > 0);
        // Advect drags a blob the way the stroke goes.
        let mut blob = vec![0u8; 32 * 32];
        paint(&mut blob, size, &brush, &[[0.3, 0.5]]);
        let advect = Brush {
            tool: PaintTool::Advect,
            radius: 0.25,
            ..brush
        };
        let right = |b: &[u8]| {
            (0..32)
                .map(|x| b[16 * 32 + x] as u32 * x as u32)
                .sum::<u32>()
        };
        let before = right(&blob);
        paint(&mut blob, size, &advect, &[[0.2, 0.5], [0.45, 0.5]]);
        assert!(right(&blob) > before);
    }

    #[test]
    fn images_become_coverage_at_the_bake_size() {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            4,
            image::Rgb([255, 255, 255]),
        ));
        let cov = coverage_from_image(&img, 16);
        assert_eq!(cov.len(), 256);
        assert!(cov.iter().all(|&v| v == 255));
        assert_eq!(flow_from_image(&img, 4).len(), 64);
    }
}
