//! HDR images: Radiance `.hdr` and OpenEXR `.exr`. Only the headers are read
//! at import - enough to size the BC6H plan and spot an equirectangular sky.

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bc6h_is_a_byte_a_pixel() {
        assert_eq!(bc6h_bytes(1024, 512, false), 1024 * 512);
        assert_eq!(bc6h_bytes(2, 2, false), 16);
    }
}
