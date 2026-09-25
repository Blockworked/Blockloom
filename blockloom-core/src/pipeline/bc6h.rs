//! BC6H (unsigned float) for HDR textures: one 128-bit block per 4x4 texels,
//! the only block format that keeps values past 1. Encodes mode 11 (one
//! region, 10-bit endpoints), which is plenty for skies, and decodes the same
//! mode back for tests and for GPUs without BC support.
//!
//! A baked sky ships as a DDS cube (DX10 header, `BC6H_UF16`), written and
//! read here so the player needs no image loader for it.

use super::hdr::HdrCube;
use half::f16;

/// Interpolation weights for 4-bit indices, in 64ths.
const WEIGHTS: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
const ENDPOINT_BITS: u32 = 10;
const MAX_HALF: u16 = 0x7BFF;

/// A texel in the encoder's working domain: the half-float bit pattern
/// stretched by 64/31, which undoes the decoder's final `* 31 / 64`. Half
/// bits are close to logarithmic, so errors here are relative, as HDR wants.
fn working(value: f32) -> f32 {
    let bits = f16::from_f32(value.max(0.0)).to_bits().min(MAX_HALF);
    bits as f32 * 64.0 / 31.0
}

fn unquantize(q: u32) -> u32 {
    let max = (1 << ENDPOINT_BITS) - 1;
    match q {
        0 => 0,
        q if q == max => 0xFFFF,
        q => ((q << 16) + 0x8000) >> ENDPOINT_BITS,
    }
}

fn quantize(working: f32) -> u32 {
    let max = (1 << ENDPOINT_BITS) - 1;
    // The inverse of `unquantize`'s midpoint mapping, then the nearer of the
    // two neighbours.
    let guess = ((working - 32.0) / 64.0).round().clamp(0.0, max as f32) as u32;
    [guess.saturating_sub(1), guess, (guess + 1).min(max)]
        .into_iter()
        .min_by(|a, b| {
            let da = (unquantize(*a) as f32 - working).abs();
            let db = (unquantize(*b) as f32 - working).abs();
            da.total_cmp(&db)
        })
        .unwrap()
}

fn interpolate(e0: u32, e1: u32, weight: u32) -> u32 {
    (e0 * (64 - weight) + e1 * weight + 32) >> 6
}

fn finish(unquantized: u32) -> u16 {
    ((unquantized * 31) >> 6) as u16
}

struct Bits {
    block: u128,
    at: u32,
}

impl Bits {
    fn put(&mut self, value: u32, count: u32) {
        self.block |= ((value as u128) & ((1u128 << count) - 1)) << self.at;
        self.at += count;
    }
}

/// Encodes one 4x4 block of linear RGB, row-major.
pub fn encode_block(texels: &[[f32; 3]; 16]) -> [u8; 16] {
    let points: Vec<[f32; 3]> = texels.iter().map(|t| t.map(working)).collect();
    // Start from the bounding box's diagonal, then refit twice by least
    // squares against the indices that line picked.
    let mut lo = [f32::MAX; 3];
    let mut hi = [0.0f32; 3];
    for point in &points {
        for c in 0..3 {
            lo[c] = lo[c].min(point[c]);
            hi[c] = hi[c].max(point[c]);
        }
    }
    let mut ends = (lo.map(quantize), hi.map(quantize));
    let mut indices = pick_indices(&points, ends);
    for _ in 0..2 {
        if let Some(refit) = refit(&points, &indices) {
            let candidate = (refit.0.map(quantize), refit.1.map(quantize));
            let picked = pick_indices(&points, candidate);
            if error(&points, candidate, &picked) < error(&points, ends, &indices) {
                ends = candidate;
                indices = picked;
            }
        }
    }
    // The anchor texel's index keeps only three bits, so its top bit must be
    // clear: swap the ends and mirror every index when it isn't.
    if indices[0] >= 8 {
        ends = (ends.1, ends.0);
        for index in &mut indices {
            *index = 15 - *index;
        }
    }
    let mut bits = Bits { block: 0, at: 0 };
    bits.put(0b00011, 5);
    for c in 0..3 {
        bits.put(ends.0[c], ENDPOINT_BITS);
    }
    for c in 0..3 {
        bits.put(ends.1[c], ENDPOINT_BITS);
    }
    bits.put(indices[0], 3);
    for index in &indices[1..] {
        bits.put(*index, 4);
    }
    debug_assert_eq!(bits.at, 128);
    bits.block.to_le_bytes()
}

type Ends = ([u32; 3], [u32; 3]);

fn palette(ends: Ends) -> [[f32; 3]; 16] {
    std::array::from_fn(|i| {
        std::array::from_fn(|c| {
            interpolate(unquantize(ends.0[c]), unquantize(ends.1[c]), WEIGHTS[i]) as f32
        })
    })
}

fn distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (0..3).map(|c| (a[c] - b[c]).powi(2)).sum()
}

fn pick_indices(points: &[[f32; 3]], ends: Ends) -> [u32; 16] {
    let palette = palette(ends);
    std::array::from_fn(|i| {
        (0..16)
            .min_by(|a, b| {
                distance(&points[i], &palette[*a]).total_cmp(&distance(&points[i], &palette[*b]))
            })
            .unwrap() as u32
    })
}

fn error(points: &[[f32; 3]], ends: Ends, indices: &[u32; 16]) -> f32 {
    let palette = palette(ends);
    (0..16)
        .map(|i| distance(&points[i], &palette[indices[i] as usize]))
        .sum()
}

/// Least-squares endpoints for fixed weights, per channel.
fn refit(points: &[[f32; 3]], indices: &[u32; 16]) -> Option<([f32; 3], [f32; 3])> {
    let (mut aa, mut ab, mut bb) = (0.0f32, 0.0f32, 0.0f32);
    let mut ax = [0.0f32; 3];
    let mut bx = [0.0f32; 3];
    for (point, index) in points.iter().zip(indices) {
        let b = WEIGHTS[*index as usize] as f32 / 64.0;
        let a = 1.0 - b;
        aa += a * a;
        ab += a * b;
        bb += b * b;
        for c in 0..3 {
            ax[c] += a * point[c];
            bx[c] += b * point[c];
        }
    }
    let det = aa * bb - ab * ab;
    if det.abs() < 1e-6 {
        return None;
    }
    let clamp = |v: f32| v.clamp(0.0, 65535.0);
    Some((
        std::array::from_fn(|c| clamp((ax[c] * bb - bx[c] * ab) / det)),
        std::array::from_fn(|c| clamp((bx[c] * aa - ax[c] * ab) / det)),
    ))
}

/// Decodes a mode 11 block, which is all [`encode_block`] writes, to halfs.
pub fn decode_block(block: &[u8; 16]) -> Result<[[u16; 3]; 16], String> {
    let bits = u128::from_le_bytes(*block);
    let take = |at: u32, count: u32| ((bits >> at) & ((1u128 << count) - 1)) as u32;
    if take(0, 5) != 0b00011 {
        return Err("only BC6H mode 11 blocks are decoded".to_string());
    }
    let e0: [u32; 3] = std::array::from_fn(|c| unquantize(take(5 + c as u32 * 10, 10)));
    let e1: [u32; 3] = std::array::from_fn(|c| unquantize(take(35 + c as u32 * 10, 10)));
    Ok(std::array::from_fn(|i| {
        let index = if i == 0 {
            take(65, 3)
        } else {
            take(68 + (i as u32 - 1) * 4, 4)
        };
        std::array::from_fn(|c| finish(interpolate(e0[c], e1[c], WEIGHTS[index as usize])))
    }))
}

/// Encodes one square face, whose side is a multiple of 4, block row-major.
pub fn encode_face(texels: &[[f32; 3]], size: u32) -> Vec<u8> {
    let blocks = size / 4;
    let mut out = Vec::with_capacity((blocks * blocks * 16) as usize);
    for by in 0..blocks {
        for bx in 0..blocks {
            let block: [[f32; 3]; 16] = std::array::from_fn(|i| {
                let (x, y) = (bx * 4 + i as u32 % 4, by * 4 + i as u32 / 4);
                texels[(y * size + x) as usize]
            });
            out.extend_from_slice(&encode_block(&block));
        }
    }
    out
}

/// Decodes a face back to RGBA16F texels, alpha 1.
pub fn decode_face(blocks: &[u8], size: u32) -> Result<Vec<u8>, String> {
    let per_row = size / 4;
    if blocks.len() != (per_row * per_row * 16) as usize {
        return Err("BC6H face is the wrong length".to_string());
    }
    let one = f16::ONE.to_bits().to_le_bytes();
    let mut out = vec![0u8; (size * size * 8) as usize];
    for (n, block) in blocks.as_chunks::<16>().0.iter().enumerate() {
        let texels = decode_block(block)?;
        let (bx, by) = (n as u32 % per_row, n as u32 / per_row);
        for (i, texel) in texels.iter().enumerate() {
            let (x, y) = (bx * 4 + i as u32 % 4, by * 4 + i as u32 / 4);
            let at = ((y * size + x) * 8) as usize;
            for c in 0..3 {
                out[at + c * 2..at + c * 2 + 2].copy_from_slice(&texel[c].to_le_bytes());
            }
            out[at + 6..at + 8].copy_from_slice(&one);
        }
    }
    Ok(out)
}

const DDS_MAGIC: &[u8; 4] = b"DDS ";
const DXGI_BC6H_UF16: u32 = 95;
const HEADER_LEN: usize = 4 + 124 + 20;

/// A cube as a DDS file: six BC6H faces, one mip each.
pub fn write_dds_cube(cube: &HdrCube) -> Vec<u8> {
    let face_bytes = (cube.size / 4) * (cube.size / 4) * 16;
    let mut out = Vec::with_capacity(HEADER_LEN + face_bytes as usize * 6);
    let u32s = |values: &[u32], out: &mut Vec<u8>| {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    };
    out.extend_from_slice(DDS_MAGIC);
    // size, flags (caps|height|width|pixelformat|linearsize), height, width,
    // linear size, depth, mip count, 11 reserved.
    u32s(
        &[124, 0x0008_1007, cube.size, cube.size, face_bytes, 0, 1],
        &mut out,
    );
    u32s(&[0; 11], &mut out);
    // Pixel format: a FourCC of DX10, which says the real format follows.
    u32s(&[32, 0x4], &mut out);
    out.extend_from_slice(b"DX10");
    u32s(&[0; 5], &mut out);
    // Caps: texture|complex, then cubemap with all six faces.
    u32s(&[0x1008, 0xFE00, 0, 0, 0], &mut out);
    // DX10: format, 2D, cube flag, one cube, no alpha info.
    u32s(&[DXGI_BC6H_UF16, 3, 0x4, 1, 0], &mut out);
    for face in &cube.faces {
        out.extend_from_slice(&encode_face(face, cube.size));
    }
    out
}

/// Reads back what [`write_dds_cube`] wrote: the face side and the six faces'
/// blocks, face after face.
pub fn read_dds_cube(bytes: &[u8]) -> Result<(u32, &[u8]), String> {
    let word = |at: usize| -> Result<u32, String> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| "DDS header is truncated".to_string())
    };
    if bytes.get(..4) != Some(DDS_MAGIC) || bytes.get(84..88) != Some(b"DX10") {
        return Err("not a DX10 DDS file".to_string());
    }
    let size = word(12)?;
    if word(128)? != DXGI_BC6H_UF16 || word(136)? & 0x4 == 0 {
        return Err("not a BC6H cube".to_string());
    }
    if size == 0 || size % 4 != 0 {
        return Err(format!("a {size} texel face doesn't fit 4x4 blocks"));
    }
    let face = (size / 4) * (size / 4) * 16;
    let data = bytes
        .get(HEADER_LEN..HEADER_LEN + face as usize * 6)
        .ok_or_else(|| "DDS cube is truncated".to_string())?;
    Ok((size, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(block: &[u8; 16]) -> [[f32; 3]; 16] {
        decode_block(block)
            .unwrap()
            .map(|t| t.map(|bits| f16::from_bits(bits).to_f32()))
    }

    fn relative_error(a: f32, b: f32) -> f32 {
        (a - b).abs() / a.abs().max(b.abs()).max(1e-3)
    }

    #[test]
    fn a_flat_block_round_trips_across_the_range() {
        for value in [0.0f32, 0.02, 0.5, 1.0, 7.5, 180.0, 30_000.0] {
            let block = encode_block(&[[value, value * 0.5, value * 0.25]; 16]);
            for texel in decoded(&block) {
                assert!(
                    relative_error(texel[0], value) < 0.02,
                    "{value} came back {texel:?}"
                );
                assert!(relative_error(texel[2], value * 0.25) < 0.02, "{texel:?}");
            }
        }
    }

    #[test]
    fn a_gradient_keeps_its_order_and_its_highlights() {
        let texels: [[f32; 3]; 16] = std::array::from_fn(|i| {
            let v = 0.1 * 2f32.powf(i as f32 / 2.0);
            [v, v, v * 0.8]
        });
        let back = decoded(&encode_block(&texels));
        for i in 1..16 {
            assert!(back[i][0] >= back[i - 1][0], "{back:?}");
        }
        assert!(relative_error(back[15][0], texels[15][0]) < 0.1);
        assert!(relative_error(back[0][0], texels[0][0]) < 0.1);
    }

    #[test]
    fn a_cube_survives_the_dds_file() {
        let faces = std::array::from_fn(|face| vec![[face as f32 + 0.5; 3]; 64]);
        let cube = HdrCube { size: 8, faces };
        let file = write_dds_cube(&cube);
        let (size, data) = read_dds_cube(&file).unwrap();
        assert_eq!(size, 8);
        let face_bytes = 4 * 16;
        let third = decode_face(&data[face_bytes * 3..face_bytes * 4], 8).unwrap();
        let red = f16::from_le_bytes([third[0], third[1]]).to_f32();
        assert!(relative_error(red, 3.5) < 0.02, "{red}");
        assert!(read_dds_cube(&file[..100]).is_err());
    }
}
