//! HDR images: Radiance `.hdr` and OpenEXR `.exr`. Import reads only the
//! headers, enough to size the BC6H plan and spot an equirectangular sky;
//! [`decode_hdr`] and [`HdrCube`] are what a sky is actually built from.

use serde::{Deserialize, Serialize};

/// What an HDR file holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HdrInfo {
    /// `hdr` or `exr`.
    pub format: String,
    pub width: u32,
    pub height: u32,
    /// Channel names as the file lists them (`R`, `G`, `B`, `A`, ...).
    pub channels: Vec<String>,
    /// Bits per channel: 32 for RGBE's shared exponent, 16 or 32 for EXR.
    pub bits: u8,
    pub bytes: u64,
}

impl HdrInfo {
    /// 2:1, the shape a sky or IBL source comes in.
    pub fn is_equirect(&self) -> bool {
        self.width == self.height * 2
    }
}

/// Reads the header of either format, by magic rather than extension.
pub fn inspect_hdr(name: &str, bytes: &[u8]) -> Result<HdrInfo, String> {
    if bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        return inspect_exr(name, bytes);
    }
    if bytes.starts_with(b"#?") {
        return inspect_radiance(name, bytes);
    }
    Err(format!(
        "{name} isn't an HDR image Blockloom reads (.hdr, .exr)"
    ))
}

fn inspect_radiance(name: &str, bytes: &[u8]) -> Result<HdrInfo, String> {
    let head = &bytes[..bytes.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    let mut lines = text.lines();
    let mut format_ok = true;
    // Header lines run to the first blank one; the resolution follows it.
    for line in lines.by_ref() {
        if line.trim().is_empty() {
            break;
        }
        if let Some(format) = line.strip_prefix("FORMAT=") {
            format_ok = format.trim() == "32-bit_rle_rgbe";
        }
    }
    if !format_ok {
        return Err(format!(
            "{name}: only RGBE Radiance files are read, not XYZE"
        ));
    }
    let resolution = lines
        .next()
        .ok_or_else(|| format!("{name}: no resolution line"))?;
    let parts: Vec<&str> = resolution.split_whitespace().collect();
    let [a, h, b, w] = parts[..] else {
        return Err(format!("{name}: bad resolution line \"{resolution}\""));
    };
    let number = |text: &str| {
        text.parse::<u32>()
            .map_err(|_| format!("{name}: bad resolution line \"{resolution}\""))
    };
    // `-Y h +X w` is the usual order; a rotated file swaps the axes.
    let (width, height) = if a.ends_with('Y') && b.ends_with('X') {
        (number(w)?, number(h)?)
    } else if a.ends_with('X') && b.ends_with('Y') {
        (number(h)?, number(w)?)
    } else {
        return Err(format!("{name}: bad resolution line \"{resolution}\""));
    };
    Ok(HdrInfo {
        format: "hdr".to_string(),
        width,
        height,
        channels: ["R", "G", "B"].map(String::from).to_vec(),
        bits: 32,
        bytes: bytes.len() as u64,
    })
}

fn inspect_exr(name: &str, bytes: &[u8]) -> Result<HdrInfo, String> {
    let truncated = || format!("{name}: EXR header is truncated");
    let i32_at = |at: usize| -> Result<i32, String> {
        let slice = bytes.get(at..at + 4).ok_or_else(truncated)?;
        Ok(i32::from_le_bytes(slice.try_into().unwrap()))
    };
    let flags = bytes.get(5).copied().unwrap_or(0);
    if flags & 0x10 != 0 {
        return Err(format!("{name}: multi-part EXR isn't supported"));
    }
    if flags & 0x08 != 0 {
        return Err(format!("{name}: deep EXR isn't supported"));
    }
    let cstr = |at: usize| -> Result<(String, usize), String> {
        let rest = bytes.get(at..).ok_or_else(truncated)?;
        let end = rest.iter().position(|b| *b == 0).ok_or_else(truncated)?;
        Ok((
            String::from_utf8_lossy(&rest[..end]).into_owned(),
            at + end + 1,
        ))
    };
    let mut at = 8;
    let mut window = None;
    let mut channels = Vec::new();
    let mut bits = 0u8;
    loop {
        let (attribute, next) = cstr(at)?;
        if attribute.is_empty() {
            break;
        }
        let (kind, next) = cstr(next)?;
        let size = usize::try_from(i32_at(next)?).map_err(|_| truncated())?;
        let value = next + 4;
        if bytes.len() < value + size {
            return Err(truncated());
        }
        match (attribute.as_str(), kind.as_str()) {
            ("dataWindow", "box2i") => {
                let x0 = i32_at(value)?;
                let y0 = i32_at(value + 4)?;
                let x1 = i32_at(value + 8)?;
                let y1 = i32_at(value + 12)?;
                window = Some(((x1 - x0 + 1).max(0) as u32, (y1 - y0 + 1).max(0) as u32));
            }
            ("channels", "chlist") => {
                let mut cursor = value;
                while cursor < value + size {
                    let (channel, after) = cstr(cursor)?;
                    if channel.is_empty() {
                        break;
                    }
                    // 0 uint, 1 half, 2 float; then pLinear, 3 reserved, x/y sampling.
                    bits = bits.max(match i32_at(after)? {
                        1 => 16,
                        _ => 32,
                    });
                    channels.push(channel);
                    cursor = after + 16;
                }
            }
            _ => {}
        }
        at = value + size;
    }
    let (width, height) = window.ok_or_else(|| format!("{name}: EXR has no dataWindow"))?;
    if channels.is_empty() {
        return Err(format!("{name}: EXR has no channels"));
    }
    Ok(HdrInfo {
        format: "exr".to_string(),
        width,
        height,
        channels,
        bits,
        bytes: bytes.len() as u64,
    })
}

/// BC6H bytes for one texture: 16 bytes per 4x4 block, plus a third again
/// for the mip chain when there is one.
pub fn bc6h_bytes(width: u32, height: u32, mips: bool) -> u64 {
    let blocks = width.div_ceil(4) as u64 * height.div_ceil(4) as u64;
    let base = blocks * 16;
    if mips { base * 4 / 3 } else { base }
}

/// A decoded HDR image: linear RGB, row-major from the top-left.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
}

impl HdrImage {
    fn at(&self, x: u32, y: u32) -> [f32; 3] {
        self.pixels[(y.min(self.height - 1) * self.width + x.min(self.width - 1)) as usize]
    }

    /// Cross-fades a panorama's left and right edges over `degrees` either
    /// side of its wrap seam, so a pano whose ends don't quite meet shows no
    /// line. Each side is pulled towards the average of the two edges, fully
    /// at the seam and not at all `degrees` away. Strips are left alone.
    pub fn fix_seam(&mut self, degrees: f32) {
        if !(degrees > 0.0) || SkyLayout::of(self.width, self.height) != SkyLayout::Equirect {
            return;
        }
        let band = ((degrees / 360.0 * self.width as f32).round() as u32).clamp(1, self.width / 2);
        let last = self.width - 1;
        for y in 0..self.height {
            let row = (y * self.width) as usize;
            let left = self.pixels[row];
            let right = self.pixels[row + last as usize];
            let mean: [f32; 3] = std::array::from_fn(|c| (left[c] + right[c]) * 0.5);
            for i in 0..band {
                let weight = 1.0 - i as f32 / band as f32;
                for (x, edge) in [(i, left), (last - i, right)] {
                    let texel = &mut self.pixels[row + x as usize];
                    for c in 0..3 {
                        texel[c] = (texel[c] + (mean[c] - edge[c]) * weight).max(0.0);
                    }
                }
            }
        }
    }

    /// Scales every texel by 2^`ev`: the per-texture exposure bias.
    pub fn bias(&mut self, ev: f32) {
        if ev == 0.0 || !ev.is_finite() {
            return;
        }
        let scale = ev.exp2();
        for texel in &mut self.pixels {
            *texel = texel.map(|c| c * scale);
        }
    }
}

/// Decodes either format to linear RGB. Negative, NaN and infinite texels
/// read as 0, since nothing downstream can show them.
pub fn decode_hdr(name: &str, bytes: &[u8]) -> Result<HdrImage, String> {
    let format = if bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        image::ImageFormat::OpenExr
    } else if bytes.starts_with(b"#?") {
        image::ImageFormat::Hdr
    } else {
        return Err(format!(
            "{name} isn't an HDR image Blockloom reads (.hdr, .exr)"
        ));
    };
    let decoded = image::load_from_memory_with_format(bytes, format)
        .map_err(|e| format!("{name}: {e}"))?
        .into_rgb32f();
    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 {
        return Err(format!("{name} is empty"));
    }
    let pixels = decoded
        .pixels()
        .map(|p| p.0.map(|c| if c.is_finite() { c.max(0.0) } else { 0.0 }))
        .collect();
    Ok(HdrImage {
        width,
        height,
        pixels,
    })
}

/// Reads and decodes a project file.
pub fn load_hdr(project_dir: &std::path::Path, relative: &str) -> Result<HdrImage, String> {
    let path = crate::assets::resolve(project_dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{relative}: {e}"))?;
    decode_hdr(relative, &bytes)
}

/// Six square faces in cubemap order (+X, -X, +Y, -Y, +Z, -Z), laid out the
/// way Bevy samples a cube: z negated, so the +Z face looks down world -Z.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrCube {
    pub size: u32,
    pub faces: [Vec<[f32; 3]>; 6],
}

/// The layouts a sky file comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkyLayout {
    /// A 2:1 latitude-longitude panorama, its centre straight ahead (-Z).
    Equirect,
    /// Six faces side by side, in cubemap order.
    StripH,
    /// Six faces stacked, in cubemap order.
    StripV,
}

impl SkyLayout {
    pub fn of(width: u32, height: u32) -> SkyLayout {
        if width == height * 6 {
            SkyLayout::StripH
        } else if height == width * 6 {
            SkyLayout::StripV
        } else {
            SkyLayout::Equirect
        }
    }
}

impl HdrCube {
    /// The largest face a sky gets, whatever its source.
    pub const MAX_FACE: u32 = 2048;

    /// Builds the faces from any layout. The face side is a power of two no
    /// bigger than `max_face`, which filtered image-based light requires and
    /// BC6H's 4x4 blocks divide.
    pub fn from_image(image: &HdrImage, max_face: u32) -> HdrCube {
        let layout = SkyLayout::of(image.width, image.height);
        let native = match layout {
            SkyLayout::Equirect => image.width / 4,
            SkyLayout::StripH => image.height,
            SkyLayout::StripV => image.width,
        };
        let size = face_size(native, max_face);
        let faces = std::array::from_fn(|face| {
            let mut texels = Vec::with_capacity((size * size) as usize);
            for y in 0..size {
                for x in 0..size {
                    texels.push(match layout {
                        SkyLayout::Equirect => {
                            let dir = face_direction(face, x, y, size);
                            sample_equirect(image, dir)
                        }
                        SkyLayout::StripH | SkyLayout::StripV => {
                            strip_texel(image, layout, face as u32, x, y, size)
                        }
                    });
                }
            }
            texels
        });
        HdrCube { size, faces }
    }

    /// This cube and every level below it, each half the last, down to
    /// `min` texels a side. Box-filtered, so a small bright sun spreads into
    /// its neighbours rather than flickering in and out of a coarse level.
    pub fn mip_chain(self, min: u32) -> Vec<HdrCube> {
        let min = min.max(1);
        let mut levels = vec![self];
        loop {
            let last = levels.last().unwrap();
            if last.size / 2 < min || last.size % 2 != 0 {
                break;
            }
            let size = last.size / 2;
            let faces = std::array::from_fn(|face| {
                let source = &last.faces[face];
                let at = |x: u32, y: u32| source[(y * last.size + x) as usize];
                let mut texels = Vec::with_capacity((size * size) as usize);
                for y in 0..size {
                    for x in 0..size {
                        let quad = [
                            at(2 * x, 2 * y),
                            at(2 * x + 1, 2 * y),
                            at(2 * x, 2 * y + 1),
                            at(2 * x + 1, 2 * y + 1),
                        ];
                        texels.push(std::array::from_fn(|c| {
                            quad.iter().map(|t| t[c]).sum::<f32>() * 0.25
                        }));
                    }
                }
                texels
            });
            levels.push(HdrCube { size, faces });
        }
        levels
    }

    /// The world direction a face texel looks along, undoing Bevy's z flip.
    pub fn direction(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
        face_direction(face, x, y, size)
    }
}

fn face_size(native: u32, max_face: u32) -> u32 {
    let max = max_face.clamp(4, HdrCube::MAX_FACE);
    // Round to the nearest power of two, never below one BC6H block.
    let native = native.max(4);
    let down = 1u32 << (31 - native.leading_zeros());
    let near = if native - down > down / 2 {
        down * 2
    } else {
        down
    };
    near.clamp(4, max)
}

/// The standard cube face mapping, then z negated into Bevy's world.
fn face_direction(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
    let s = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let t = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    let [lx, ly, lz] = match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    };
    let length = (lx * lx + ly * ly + lz * lz).sqrt();
    [lx / length, ly / length, -lz / length]
}

/// Bilinear, wrapping around in longitude.
fn sample_equirect(image: &HdrImage, dir: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = dir;
    let u = 0.5 + x.atan2(-z) / std::f32::consts::TAU;
    let v = y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    let fx = u * image.width as f32 - 0.5;
    let fy = (v * image.height as f32 - 0.5).clamp(0.0, (image.height - 1) as f32);
    let x0 = fx.floor();
    let y0 = fy.floor();
    let (tx, ty) = (fx - x0, fy - y0);
    let wrap = |x: f32| (x as i64).rem_euclid(image.width as i64) as u32;
    let (xa, xb) = (wrap(x0), wrap(x0 + 1.0));
    let (ya, yb) = (y0 as u32, (y0 as u32 + 1).min(image.height - 1));
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
    let top = lerp(image.at(xa, ya), image.at(xb, ya), tx);
    let bottom = lerp(image.at(xa, yb), image.at(xb, yb), tx);
    lerp(top, bottom, ty)
}

/// Box-filters one texel of a strip face down (or nearest up) to `size`.
fn strip_texel(
    image: &HdrImage,
    layout: SkyLayout,
    face: u32,
    x: u32,
    y: u32,
    size: u32,
) -> [f32; 3] {
    let side = match layout {
        SkyLayout::StripH => image.height,
        _ => image.width,
    };
    let (ox, oy) = match layout {
        SkyLayout::StripH => (face * side, 0),
        _ => (0, face * side),
    };
    let x0 = x * side / size;
    let x1 = ((x + 1) * side / size).max(x0 + 1);
    let y0 = y * side / size;
    let y1 = ((y + 1) * side / size).max(y0 + 1);
    let mut sum = [0.0f32; 3];
    for sy in y0..y1 {
        for sx in x0..x1 {
            let texel = image.at(ox + sx, oy + sy);
            for i in 0..3 {
                sum[i] += texel[i];
            }
        }
    }
    let count = ((x1 - x0) * (y1 - y0)) as f32;
    sum.map(|c| c / count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, texel: impl Fn(u32, u32) -> [f32; 3]) -> HdrImage {
        HdrImage {
            width,
            height,
            pixels: (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| texel(x, y))
                .collect(),
        }
    }

    #[test]
    fn a_panorama_centre_lands_straight_ahead() {
        // Bright at the panorama's centre column, the horizon straight ahead.
        let image = flat(64, 32, |x, _| {
            if (30..34).contains(&x) {
                [10.0; 3]
            } else {
                [0.1; 3]
            }
        });
        let cube = HdrCube::from_image(&image, 64);
        assert_eq!(cube.size, 16);
        // Bevy's +Z face looks down world -Z, the default camera's forward.
        let centre = (cube.size / 2 * cube.size + cube.size / 2) as usize;
        assert!(
            cube.faces[4][centre][0] > 5.0,
            "{:?}",
            cube.faces[4][centre]
        );
        assert!(cube.faces[5][centre][0] < 1.0);
        let [x, y, z] = HdrCube::direction(4, 8, 8, 16);
        assert!(z < -0.99 && x.abs() < 0.1 && y.abs() < 0.1);
    }

    #[test]
    fn a_strip_keeps_its_faces_in_order() {
        let image = flat(24, 4, |x, _| [(x / 4) as f32; 3]);
        let cube = HdrCube::from_image(&image, 2048);
        assert_eq!(cube.size, 4);
        for face in 0..6 {
            assert_eq!(cube.faces[face][5], [face as f32; 3]);
        }
        let tall = flat(8, 48, |_, y| [(y / 8) as f32; 3]);
        let cube = HdrCube::from_image(&tall, 4);
        assert_eq!(cube.size, 4);
        assert_eq!(cube.faces[3][0], [3.0; 3]);
    }

    #[test]
    fn a_bias_scales_by_stops() {
        let mut image = flat(2, 1, |_, _| [1.0, 2.0, 0.5]);
        image.bias(2.0);
        assert_eq!(image.pixels[0], [4.0, 8.0, 2.0]);
    }

    #[test]
    fn a_radiance_file_decodes() {
        let mut bytes = Vec::new();
        let texels = vec![image::Rgb([2.0f32, 1.0, 0.5]); 8];
        image::codecs::hdr::HdrEncoder::new(&mut bytes)
            .encode(&texels, 4, 2)
            .unwrap();
        let image = decode_hdr("sky.hdr", &bytes).unwrap();
        assert_eq!((image.width, image.height), (4, 2));
        assert!((image.pixels[3][0] - 2.0).abs() < 0.05);
        assert!(decode_hdr("sky.png", b"\x89PNG").is_err());
    }

    #[test]
    fn a_radiance_header_gives_its_size() {
        let bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 512 +X 1024\n\x02\x02";
        let info = inspect_hdr("sky.hdr", bytes).unwrap();
        assert_eq!((info.width, info.height), (1024, 512));
        assert!(info.is_equirect());
    }

    #[test]
    fn xyze_radiance_is_refused() {
        let bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_xyze\n\n-Y 4 +X 4\n";
        assert!(inspect_hdr("sky.hdr", bytes).is_err());
    }

    fn exr_header(width: i32, height: i32, pixel_type: i32) -> Vec<u8> {
        let mut bytes = vec![0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0];
        let mut attribute = |name: &str, kind: &str, value: &[u8]| {
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(kind.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&(value.len() as i32).to_le_bytes());
            bytes.extend_from_slice(value);
        };
        let mut chlist = Vec::new();
        for channel in ["B", "G", "R"] {
            chlist.extend_from_slice(channel.as_bytes());
            chlist.push(0);
            chlist.extend_from_slice(&pixel_type.to_le_bytes());
            chlist.extend_from_slice(&[0, 0, 0, 0]);
            chlist.extend_from_slice(&1i32.to_le_bytes());
            chlist.extend_from_slice(&1i32.to_le_bytes());
        }
        chlist.push(0);
        attribute("channels", "chlist", &chlist);
        let window: Vec<u8> = [0, 0, width - 1, height - 1]
            .iter()
            .flat_map(|v: &i32| v.to_le_bytes())
            .collect();
        attribute("dataWindow", "box2i", &window);
        bytes.push(0);
        bytes
    }

    #[test]
    fn an_exr_header_gives_size_channels_and_depth() {
        let info = inspect_hdr("probe.exr", &exr_header(256, 128, 1)).unwrap();
        assert_eq!((info.width, info.height), (256, 128));
        assert_eq!(info.channels, ["B", "G", "R"]);
        assert_eq!(info.bits, 16);
    }

    #[test]
    fn a_seam_fix_meets_both_edges_in_the_middle() {
        // Left edge 1, right edge 3: a visible line where the pano wraps.
        let mut image = flat(72, 36, |x, _| if x < 36 { [1.0; 3] } else { [3.0; 3] });
        image.fix_seam(10.0);
        assert_eq!(image.at(0, 5), [2.0; 3]);
        assert_eq!(image.at(71, 5), [2.0; 3]);
        // Fading back to the original two degrees in from a 10 degree band.
        assert!(image.at(1, 5)[0] > 1.0 && image.at(1, 5)[0] < 2.0);
        assert_eq!(image.at(20, 5), [1.0; 3]);
        assert_eq!(image.at(50, 5), [3.0; 3]);
        // Off, or a strip, changes nothing.
        let mut strip = flat(24, 4, |x, _| [x as f32; 3]);
        let before = strip.clone();
        strip.fix_seam(10.0);
        assert_eq!(strip, before);
    }

    #[test]
    fn a_mip_chain_halves_by_averaging() {
        let image = flat(64, 32, |x, _| [x as f32; 3]);
        let levels = HdrCube::from_image(&image, 16).mip_chain(1);
        assert_eq!(
            levels.iter().map(|l| l.size).collect::<Vec<_>>(),
            [16, 8, 4, 2, 1]
        );
        let mean = |cube: &HdrCube| {
            cube.faces.iter().flatten().map(|t| t[0]).sum::<f32>()
                / (cube.size * cube.size * 6) as f32
        };
        assert!((mean(&levels[0]) - mean(&levels[4])).abs() < 1e-3);
    }

    #[test]
    fn bc6h_is_a_byte_a_pixel() {
        assert_eq!(bc6h_bytes(1024, 512, false), 1024 * 512);
        assert_eq!(bc6h_bytes(2, 2, false), 16);
    }
}
