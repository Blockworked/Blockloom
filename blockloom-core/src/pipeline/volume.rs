//! 3D volumes: colour grading LUTs (`.cube`) and volumes laid out as a strip
//! of slices in an ordinary image (Unity's 1024x32 grading strip, noise
//! volumes). Both decode to the same [`Volume`], which a pass uploads as a 3D
//! texture.

use serde::{Deserialize, Serialize};

/// Where a volume came from and how big it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeInfo {
    /// `cube` or `strip`.
    pub source: String,
    /// Texels along x, y and z.
    pub size: [u32; 3],
    /// A `.cube` file's `TITLE`, empty otherwise.
    #[serde(default)]
    pub title: String,
}

impl VolumeInfo {
    pub fn texels(&self) -> u64 {
        self.size.iter().map(|side| *side as u64).product()
    }
}

/// A decoded volume: RGBA texels, x fastest, then y, then z.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    pub info: VolumeInfo,
    /// The input range a LUT maps from; `[0, 1]` per channel for a strip.
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub texels: Vec<[f32; 4]>,
}

impl Volume {
    /// The texel at `(x, y, z)`.
    pub fn at(&self, x: u32, y: u32, z: u32) -> [f32; 4] {
        let [w, h, _] = self.info.size;
        self.texels[(x + y * w + z * w * h) as usize]
    }
}

/// One page in a [`VolumeAtlasLayout`]: where its texels sit in the packed
/// staging buffer and where they would sit in a depth-stacked GPU atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeAtlasPage {
    /// Texels along x, y and z.
    pub size: [u32; 3],
    /// Origin in the depth-stacked image (XY shared, pages stacked in Z).
    pub origin: [u32; 3],
    /// Offset in the packed staging buffer, in texels.
    pub offset_texels: u64,
}

/// Several volumes sharing one allocation: LUT pages in the grading atlas,
/// noise volumes in the cloud staging buffer. Staging packs pages back to
/// back with no padding, so an existing upload reads its page straight from
/// a slice; `origin`/`atlas` describe the depth-stacked GPU image the
/// single-bind pass samples instead.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeAtlasLayout {
    /// XY is the largest page's, Z the sum of every page's.
    pub atlas: [u32; 3],
    pub pages: Vec<VolumeAtlasPage>,
    /// Texels in the packed staging buffer.
    pub total_texels: u64,
}

impl VolumeAtlasLayout {
    /// Where page `index` starts in the staging buffer, in texels.
    pub fn page_offset(&self, index: usize) -> u64 {
        self.pages[index].offset_texels
    }

    /// Maps a page-local z in 0-1 onto the stacked image's z, for a shader
    /// sampling the whole atlas as one 3D texture.
    pub fn page_z(&self, index: usize, z: f32) -> f32 {
        let page = &self.pages[index];
        (page.origin[2] as f32 + z.clamp(0.0, 1.0) * page.size[2] as f32)
            / self.atlas[2].max(1) as f32
    }

    /// How much of the stacked image's XY a page covers (1 for same-size
    /// pages, less for a small page padded into a larger atlas).
    pub fn page_xy_scale(&self, index: usize) -> [f32; 2] {
        let page = &self.pages[index];
        [
            page.size[0] as f32 / self.atlas[0].max(1) as f32,
            page.size[1] as f32 / self.atlas[1].max(1) as f32,
        ]
    }
}

/// Lays `sizes` out as one atlas. Errors on an empty list or a zero-sided
/// page; overflow-safe, since stacked noise can pass 4B texels.
pub fn pack_volume_atlas(sizes: &[[u32; 3]]) -> Result<VolumeAtlasLayout, String> {
    if sizes.is_empty() {
        return Err("a volume atlas needs at least one page".into());
    }
    let mut atlas = [0u32, 0u32, 0u32];
    let mut pages = Vec::with_capacity(sizes.len());
    let mut total: u64 = 0;
    for size in sizes {
        if size.contains(&0) {
            return Err(format!(
                "a volume atlas page can't be empty ({})",
                describe(*size)
            ));
        }
        let texels: u64 = size.iter().map(|side| *side as u64).product();
        total = total
            .checked_add(texels)
            .ok_or_else(|| "a volume atlas can't hold that many texels".to_string())?;
        atlas[0] = atlas[0].max(size[0]);
        atlas[1] = atlas[1].max(size[1]);
        let origin = [0, 0, atlas[2]];
        atlas[2] = atlas[2]
            .checked_add(size[2])
            .ok_or_else(|| "a volume atlas can't stack that deep".to_string())?;
        pages.push(VolumeAtlasPage {
            size: *size,
            origin,
            offset_texels: total - texels,
        });
    }
    Ok(VolumeAtlasLayout {
        atlas,
        pages,
        total_texels: total,
    })
}

fn describe(size: [u32; 3]) -> String {
    format!("{}x{}x{}", size[0], size[1], size[2])
}

/// Parses an Adobe/Resolve `.cube` 3D LUT. Red runs fastest, as the format
/// says, which is the texel order a 3D texture wants with red on x.
pub fn parse_cube(name: &str, text: &str) -> Result<Volume, String> {
    let mut title = String::new();
    let mut size = 0u32;
    let mut domain_min = [0.0f32; 3];
    let mut domain_max = [1.0f32; 3];
    let mut texels = Vec::new();
    let triple = |rest: &str, line: usize| -> Result<[f32; 3], String> {
        let values: Vec<f32> = rest
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| format!("{name}:{line}: expected three numbers"))?;
        values
            .try_into()
            .map_err(|_| format!("{name}:{line}: expected three numbers"))
    };
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match keyword {
            "TITLE" => title = rest.trim().trim_matches('"').to_string(),
            "LUT_3D_SIZE" => {
                size = rest
                    .trim()
                    .parse()
                    .map_err(|_| format!("{name}:{number}: bad LUT_3D_SIZE"))?;
                if !(2..=256).contains(&size) {
                    return Err(format!("{name}: LUT_3D_SIZE {size} is outside 2-256"));
                }
            }
            "LUT_1D_SIZE" => {
                return Err(format!(
                    "{name} is a 1D LUT; grading takes a 3D one (LUT_3D_SIZE)"
                ));
            }
            "DOMAIN_MIN" => domain_min = triple(rest, number)?,
            "DOMAIN_MAX" => domain_max = triple(rest, number)?,
            // Resolve writes this for shaper LUTs; the grid itself is the same.
            "LUT_3D_INPUT_RANGE" => {}
            _ if keyword.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') => {
                let [r, g, b] = triple(line, number)?;
                texels.push([r, g, b, 1.0]);
            }
            _ => return Err(format!("{name}:{number}: unknown keyword {keyword}")),
        }
    }
    if size == 0 {
        return Err(format!("{name} has no LUT_3D_SIZE"));
    }
    let expected = (size as usize).pow(3);
    if texels.len() != expected {
        return Err(format!(
            "{name} has {} entries; a {size}³ LUT needs {expected}",
            texels.len()
        ));
    }
    if (0..3).any(|i| domain_max[i] <= domain_min[i]) {
        return Err(format!("{name}: DOMAIN_MAX must be above DOMAIN_MIN"));
    }
    Ok(Volume {
        info: VolumeInfo {
            source: "cube".to_string(),
            size: [size; 3],
            title,
        },
        domain_min,
        domain_max,
        texels,
    })
}

/// The volume an image strip lays out: `n` slices of `n`x`n` side by side
/// (width = height²) or stacked (height = width²). `None` for any other shape.
pub fn strip_size(width: u32, height: u32) -> Option<[u32; 3]> {
    if height >= 2 && width == height * height {
        Some([height, height, height])
    } else if width >= 2 && height == width * width {
        Some([width, width, width])
    } else {
        None
    }
}

/// Unpacks an image strip into a volume. Slices go along z in reading order.
pub fn volume_from_strip(name: &str, image: &image::RgbaImage) -> Result<Volume, String> {
    let (width, height) = image.dimensions();
    let size = strip_size(width, height).ok_or_else(|| {
        format!("{name} ({width}x{height}) isn't a strip of square slices (e.g. 1024x32)")
    })?;
    let side = size[0];
    let across = width > height;
    let mut texels = Vec::with_capacity((side as usize).pow(3));
    for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                let (px, py) = if across {
                    (z * side + x, y)
                } else {
                    (x, z * side + y)
                };
                let [r, g, b, a] = image.get_pixel(px, py).0;
                texels.push([r, g, b, a].map(|c| c as f32 / 255.0));
            }
        }
    }
    Ok(Volume {
        info: VolumeInfo {
            source: "strip".to_string(),
            size,
            title: String::new(),
        },
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        texels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_cube(size: u32) -> String {
        let mut text = format!("TITLE \"identity\"\nLUT_3D_SIZE {size}\n");
        let step = (size - 1) as f32;
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    text += &format!(
                        "{} {} {}\n",
                        r as f32 / step,
                        g as f32 / step,
                        b as f32 / step
                    );
                }
            }
        }
        text
    }

    #[test]
    fn a_cube_lut_lands_red_fastest() {
        let volume = parse_cube("id.cube", &identity_cube(3)).unwrap();
        assert_eq!(volume.info.size, [3, 3, 3]);
        assert_eq!(volume.info.title, "identity");
        assert_eq!(volume.at(2, 0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(volume.at(0, 0, 2), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_short_or_one_dimensional_cube_is_refused() {
        let mut short = identity_cube(3);
        short.truncate(short.trim_end().rfind('\n').unwrap());
        assert!(
            parse_cube("short.cube", &short)
                .unwrap_err()
                .contains("needs 27")
        );
        assert!(parse_cube("flat.cube", "LUT_1D_SIZE 4\n").is_err());
    }

    #[test]
    fn a_strip_unpacks_slice_by_slice() {
        assert_eq!(strip_size(1024, 32), Some([32; 3]));
        assert_eq!(strip_size(4, 16), Some([4; 3]));
        assert_eq!(strip_size(100, 30), None);

        let mut strip = image::RgbaImage::new(4, 2);
        // Slice z = 1 starts at x = 2.
        strip.put_pixel(3, 1, image::Rgba([255, 0, 0, 255]));
        let volume = volume_from_strip("grade.png", &strip).unwrap();
        assert_eq!(volume.at(1, 1, 1), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(volume.at(1, 1, 0), [0.0; 4]);
    }

    #[test]
    fn an_atlas_stacks_pages_with_staging_offsets() {
        // Two same-size LUT pages: one page per depth slab.
        let layout = pack_volume_atlas(&[[16; 3], [16; 3]]).unwrap();
        assert_eq!(layout.atlas, [16, 16, 32]);
        assert_eq!(layout.pages[0].origin, [0, 0, 0]);
        assert_eq!(layout.pages[1].origin, [0, 0, 16]);
        assert_eq!(layout.page_offset(0), 0);
        assert_eq!(layout.page_offset(1), 16u64.pow(3));
        assert_eq!(layout.total_texels, 2 * 16u64.pow(3));
        // A page's z maps onto its own slab of the stacked image.
        assert_eq!(layout.page_z(0, 0.0), 0.0);
        assert_eq!(layout.page_z(0, 1.0), 0.5);
        assert_eq!(layout.page_z(1, 0.0), 0.5);
        assert_eq!(layout.page_z(1, 1.0), 1.0);
        assert_eq!(layout.page_xy_scale(0), [1.0, 1.0]);
        // Mixed sizes share XY, stack in Z: a small page covers a corner.
        let mixed = pack_volume_atlas(&[[128, 128, 128], [32, 32, 32]]).unwrap();
        assert_eq!(mixed.atlas, [128, 128, 160]);
        assert_eq!(mixed.pages[1].origin, [0, 0, 128]);
        assert_eq!(mixed.page_xy_scale(1), [0.25, 0.25]);
        assert_eq!(mixed.page_z(1, 1.0), 1.0);
        // One page is the identity, and bad lists are refused.
        let single = pack_volume_atlas(&[[32; 3]]).unwrap();
        assert_eq!(single.atlas, [32; 3]);
        assert_eq!(single.page_z(0, 0.25), 0.25);
        assert!(pack_volume_atlas(&[]).is_err());
        assert!(pack_volume_atlas(&[[16, 16, 0]]).is_err());
    }
}
