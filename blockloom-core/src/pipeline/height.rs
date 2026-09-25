//! Heightmaps: 16-bit (or 8-bit) grayscale images, and headerless square
//! `.r16`/`.raw` (little-endian u16) and `.r32` (little-endian f32) samples,
//! the way terrain tools export them. All decode to heights in 0-1.

use serde::{Deserialize, Serialize};

/// What a heightmap holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeightInfo {
    /// `image`, `r16` or `r32`.
    pub source: String,
    pub width: u32,
    pub height: u32,
    /// Bits per sample.
    pub bits: u8,
    /// The lowest and highest sample, 0-1.
    pub min: f32,
    pub max: f32,
}

/// Decoded samples, row-major from the top-left, 0-1.
#[derive(Debug, Clone, PartialEq)]
pub struct Heightmap {
    pub info: HeightInfo,
    pub samples: Vec<f32>,
}

impl Heightmap {
    pub fn at(&self, x: u32, y: u32) -> f32 {
        self.samples[(y * self.info.width + x) as usize]
    }
}

/// Decodes a heightmap by its extension: raw samples for `.r16`/`.raw`/`.r32`,
/// anything else through the image decoder.
pub fn decode_heightmap(name: &str, bytes: &[u8]) -> Result<Heightmap, String> {
    let extension = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let (source, bits, width, height, samples) = match extension.as_str() {
        "r16" | "raw" => {
            let side = square_side(name, bytes.len(), 2)?;
            let samples = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair) as f32 / u16::MAX as f32)
                .collect();
            ("r16", 16, side, side, samples)
        }
        "r32" => {
            let side = square_side(name, bytes.len(), 4)?;
            let samples: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|quad| f32::from_le_bytes(*quad))
                .collect();
            if samples.iter().any(|s| !s.is_finite()) {
                return Err(format!("{name} holds NaN or infinite heights"));
            }
            ("r32", 32, side, side, samples)
        }
        _ => {
            let image = image::load_from_memory(bytes)
                .map_err(|e| format!("{name} doesn't decode as an image: {e}"))?;
            let bits =
                if image.color().bits_per_pixel() / image.color().channel_count() as u16 >= 16 {
                    16
                } else {
                    8
                };
            let gray = image.to_luma16();
            let samples = gray
                .pixels()
                .map(|pixel| pixel.0[0] as f32 / u16::MAX as f32)
                .collect();
            ("image", bits, gray.width(), gray.height(), samples)
        }
    };
    let (min, max) = samples
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
    Ok(Heightmap {
        info: HeightInfo {
            source: source.to_string(),
            width,
            height,
            bits,
            min: if samples.is_empty() { 0.0 } else { min },
            max: if samples.is_empty() { 0.0 } else { max },
        },
        samples,
    })
}

/// The side of a square of `len / stride` samples, or why there isn't one.
fn square_side(name: &str, len: usize, stride: usize) -> Result<u32, String> {
    let count = len / stride;
    let side = (count as f64).sqrt().round() as usize;
    if !len.is_multiple_of(stride) || side == 0 || side * side != count {
        return Err(format!(
            "{name}: {len} bytes isn't a square of {}-bit samples",
            stride * 8
        ));
    }
    Ok(side as u32)
}

/// Whether a side suits a chunked terrain: 2ⁿ+1 samples, so chunks share
/// their edge row.
pub fn is_terrain_side(side: u32) -> bool {
    side >= 3 && (side - 1).is_power_of_two()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_r16_is_a_square_of_little_endian_samples() {
        let bytes: Vec<u8> = [0u16, 65535, 32768, 0]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let map = decode_heightmap("island.r16", &bytes).unwrap();
        assert_eq!((map.info.width, map.info.height), (2, 2));
        assert_eq!(map.at(1, 0), 1.0);
        assert_eq!((map.info.min, map.info.max), (0.0, 1.0));
        assert!(decode_heightmap("odd.r16", &[0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn a_sixteen_bit_png_keeps_its_depth() {
        let image =
            image::ImageBuffer::<image::Luma<u16>, _>::from_pixel(3, 3, image::Luma([1000]));
        let mut png = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let map = decode_heightmap("hills.png", &png).unwrap();
        assert_eq!(map.info.bits, 16);
        assert!((map.at(2, 2) - 1000.0 / 65535.0).abs() < 1e-6);
    }

    #[test]
    fn terrain_sides_are_a_power_of_two_plus_one() {
        assert!(is_terrain_side(513));
        assert!(is_terrain_side(1025));
        assert!(!is_terrain_side(512));
    }
}
