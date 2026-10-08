//! Normal-map authoring for 2D sprites: a height-to-normal bake and a
//! lit preview with a movable light, so a bevelled look can be tuned
//! without leaving the editor. Normals are tangent space, +X right, +Y up
//! the image, +Z out of the sprite (the OpenGL convention).

use std::path::Path;

/// Strongest bump the bake takes.
pub const MAX_STRENGTH: f32 = 32.0;

/// Heights 0-1, row-major, top row first.
pub struct Heights {
    pub width: u32,
    pub height: u32,
    pub samples: Vec<f32>,
}

impl Heights {
    /// From an image: luma, scaled to nothing where the pixel is clear.
    pub fn from_image(image: &image::DynamicImage) -> Self {
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let samples = rgba
            .pixels()
            .map(|p| {
                let luma =
                    (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
                luma * (p[3] as f32 / 255.0)
            })
            .collect();
        Self {
            width,
            height,
            samples,
        }
    }

    fn at(&self, x: i64, y: i64) -> f32 {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        self.samples[y * self.width as usize + x]
    }
}

/// The unit normal at a pixel from a 3x3 Sobel slope, flat at the image
/// edge's clamp.
pub fn normal_at(heights: &Heights, x: u32, y: u32, strength: f32) -> [f32; 3] {
    let (x, y) = (x as i64, y as i64);
    let h = |dx: i64, dy: i64| heights.at(x + dx, y + dy);
    let dx = (h(1, -1) + 2.0 * h(1, 0) + h(1, 1)) - (h(-1, -1) + 2.0 * h(-1, 0) + h(-1, 1));
    let dy_down = (h(-1, 1) + 2.0 * h(0, 1) + h(1, 1)) - (h(-1, -1) + 2.0 * h(0, -1) + h(1, -1));
    let s = strength.clamp(0.0, MAX_STRENGTH);
    // Image rows run down; +Y is up the image.
    let n = [-dx * s * 0.25, dy_down * s * 0.25, 1.0];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    [n[0] / len, n[1] / len, n[2] / len]
}

/// The bake as RGBA8 (alpha is the source's coverage by height 0).
pub fn bake(heights: &Heights, strength: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(heights.samples.len() * 4);
    for y in 0..heights.height {
        for x in 0..heights.width {
            let n = normal_at(heights, x, y, strength);
            out.extend(n.map(|v| ((v * 0.5 + 0.5) * 255.0).round() as u8));
            out.push(255);
        }
    }
    out
}

/// A normal back out of its RGB bytes.
pub fn decode(rgb: [u8; 3]) -> [f32; 3] {
    let v = rgb.map(|b| b as f32 / 255.0 * 2.0 - 1.0);
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-4 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

/// Lambert light for a normal under a light direction (towards the light),
/// with a floor of `ambient`.
pub fn lit(normal: [f32; 3], light: [f32; 3], ambient: f32) -> f32 {
    let len = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt();
    if len < 1e-4 {
        return ambient.clamp(0.0, 1.0);
    }
    let d = (normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2]) / len;
    (ambient + (1.0 - ambient) * d.max(0.0)).clamp(0.0, 1.0)
}

/// The direction to a light dot at `(u, v)` over the preview (0-1, v down),
/// raised `height` over the sprite.
pub fn light_toward(u: f32, v: f32, height: f32) -> [f32; 3] {
    [u * 2.0 - 1.0, -(v * 2.0 - 1.0), height.max(0.05)]
}

/// The normal map's pixels shaded by a light, as RGBA8 grey.
pub fn preview(normals: &image::RgbaImage, light: [f32; 3], ambient: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(normals.as_raw().len());
    for p in normals.pixels() {
        let l = (lit(decode([p[0], p[1], p[2]]), light, ambient) * 255.0).round() as u8;
        out.extend([l, l, l, 255]);
    }
    out
}

/// `art.png` -> `art_n.png`, in the same folder.
pub fn output_name(source: &str) -> String {
    let path = Path::new(source);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("normal");
    let name = format!("{stem}_n.png");
    match path.parent().and_then(|p| p.to_str()) {
        Some(dir) if !dir.is_empty() => format!("{dir}/{name}"),
        _ => name,
    }
}

/// Bakes `source` (a project asset) to its `_n.png` neighbour and returns
/// the new project-relative path.
pub fn bake_file(project_dir: &Path, source: &str, strength: f32) -> Result<String, String> {
    let relative = crate::assets::normalize(source)
        .ok_or_else(|| format!("\"{source}\" isn't a path in this project"))?;
    let full = crate::assets::resolve(project_dir, &relative)
        .ok_or_else(|| format!("\"{source}\" isn't a path in this project"))?;
    let image = image::open(&full).map_err(|e| format!("{relative}: {e}"))?;
    let heights = Heights::from_image(&image);
    let pixels = bake(&heights, strength);
    let target = output_name(&relative);
    let out = crate::assets::resolve(project_dir, &target)
        .ok_or_else(|| format!("\"{target}\" isn't a path in this project"))?;
    image::save_buffer(
        &out,
        &pixels,
        heights.width,
        heights.height,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("{target}: {e}"))?;
    Ok(target)
}

/// A lit preview of a normal map as a PNG data URL, no larger than
/// `max_side` on its long side.
pub fn preview_url(
    project_dir: &Path,
    normal_map: &str,
    light: [f32; 3],
    max_side: u32,
) -> Result<String, String> {
    use base64::Engine as _;
    use image::ImageEncoder as _;
    let relative = crate::assets::normalize(normal_map)
        .ok_or_else(|| format!("\"{normal_map}\" isn't a path in this project"))?;
    let full = crate::assets::resolve(project_dir, &relative)
        .ok_or_else(|| format!("\"{normal_map}\" isn't a path in this project"))?;
    let mut image = image::open(&full)
        .map_err(|e| format!("{relative}: {e}"))?
        .to_rgba8();
    let side = max_side.clamp(8, 512);
    if image.width().max(image.height()) > side {
        let scale = side as f32 / image.width().max(image.height()) as f32;
        let (w, h) = (
            ((image.width() as f32 * scale) as u32).max(1),
            ((image.height() as f32 * scale) as u32).max(1),
        );
        image = image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle);
    }
    let shaded = preview(&image, light, 0.15);
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &shaded,
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Heights {
        // Brighter to the right.
        Heights {
            width: 5,
            height: 5,
            samples: (0..25).map(|i| (i % 5) as f32 / 4.0).collect(),
        }
    }

    #[test]
    fn a_flat_height_bakes_a_straight_out_normal() {
        let flat = Heights {
            width: 3,
            height: 3,
            samples: vec![0.5; 9],
        };
        assert_eq!(normal_at(&flat, 1, 1, 8.0), [0.0, 0.0, 1.0]);
        assert_eq!(&bake(&flat, 8.0)[..4], &[128, 128, 255, 255]);
    }

    #[test]
    fn a_slope_up_to_the_right_tilts_the_normal_left() {
        let n = normal_at(&ramp(), 2, 2, 4.0);
        assert!(n[0] < -0.1 && n[1].abs() < 1e-5 && n[2] > 0.0);
        let stronger = normal_at(&ramp(), 2, 2, 8.0);
        assert!(stronger[0] < n[0]);
        let zero = normal_at(&ramp(), 2, 2, 0.0);
        assert_eq!(zero, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_slope_up_the_image_tilts_the_normal_down() {
        let tall = Heights {
            width: 3,
            height: 3,
            // Bright at the top row.
            samples: vec![1.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0],
        };
        let n = normal_at(&tall, 1, 1, 4.0);
        assert!(n[1] < -0.1, "{n:?}");
    }

    #[test]
    fn lambert_follows_the_light() {
        let up = [0.0, 0.0, 1.0];
        assert!((lit(up, [0.0, 0.0, 1.0], 0.0) - 1.0).abs() < 1e-6);
        assert!(lit(up, [1.0, 0.0, 0.0], 0.0) < 1e-6);
        assert_eq!(lit(up, [1.0, 0.0, 0.0], 0.2), 0.2);
        let tilted = decode([0, 128, 255]);
        assert!(lit(tilted, [-1.0, 0.0, 0.3], 0.0) > lit(tilted, [1.0, 0.0, 0.3], 0.0));
    }

    #[test]
    fn the_light_dot_maps_to_a_direction() {
        assert_eq!(light_toward(0.5, 0.5, 1.0), [0.0, 0.0, 1.0]);
        let top_right = light_toward(1.0, 0.0, 0.5);
        assert!(top_right[0] > 0.9 && top_right[1] > 0.9);
    }

    #[test]
    fn output_names_sit_beside_the_source() {
        assert_eq!(
            output_name("assets/sprites/a.png"),
            "assets/sprites/a_n.png"
        );
        assert_eq!(output_name("a.png"), "a_n.png");
    }

    #[test]
    fn bake_and_preview_round_trip_through_files() {
        let dir = std::env::temp_dir().join(format!("blockloom-nm-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let mut art = image::RgbaImage::new(8, 8);
        for (x, _, p) in art.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8 * 30, x as u8 * 30, x as u8 * 30, 255]);
        }
        art.save(dir.join("assets/a.png")).unwrap();
        let out = bake_file(&dir, "assets/a.png", 6.0).unwrap();
        assert_eq!(out, "assets/a_n.png");
        let url = preview_url(&dir, &out, light_toward(0.0, 0.5, 0.6), 64).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(bake_file(&dir, "../outside.png", 1.0).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
