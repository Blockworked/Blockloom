//! BC6H (unsigned float) for HDR textures: one 128-bit block per 4x4 texels,
//! the only block format that keeps values past 1. Encodes and decodes all 14
//! modes: the one-region modes for smooth blocks, and the two-region ones,
//! over the 32 shapes, for blocks with an edge in them. The decoder is for
//! tests and for GPUs without BC support.
//!
//! A baked sky ships as a DDS cube (DX10 header, `BC6H_UF16`), written and
//! read here so the player needs no image loader for it.

use super::hdr::HdrCube;
use half::f16;

/// Interpolation weights for 3- and 4-bit indices, in 64ths.
const WEIGHTS3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHTS4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
const MAX_HALF: u16 = 0x7BFF;

/// Which pixels of a two-region block are in region 1, bit per pixel
/// row-major, and the anchor pixel whose index drops its top bit there.
const SHAPES: [u16; 32] = [
    0xCCCC, 0x8888, 0xEEEE, 0xECC8, 0xC880, 0xFEEC, 0xFEC8, 0xEC80, 0xC800, 0xFFEC, 0xFE80, 0xE800,
    0xFFE8, 0xFF00, 0xFFF0, 0xF000, 0xF710, 0x008E, 0x7100, 0x08CE, 0x008C, 0x7310, 0x3100, 0x8CCE,
    0x088C, 0x3110, 0x6666, 0x366C, 0x17E8, 0x0FF0, 0x718E, 0x399C,
];
const ANCHORS: [u8; 32] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 2, 8, 2, 2, 8, 8, 15, 2, 8,
    2, 2, 8, 8, 2, 2,
];

// Header fields: endpoints w, x (region 0) and y, z (region 1), one per
// channel, then the shape.
const RW: u8 = 0;
const GW: u8 = 1;
const BW: u8 = 2;
const RX: u8 = 3;
const GX: u8 = 4;
const BX: u8 = 5;
const RY: u8 = 6;
const GY: u8 = 7;
const BY: u8 = 8;
const RZ: u8 = 9;
const GZ: u8 = 10;
const BZ: u8 = 11;
const D: u8 = 12;

/// One of the 14 unsigned modes: its code, endpoint precision, and where
/// each header bit lands, as (field, first bit, count) in stream order.
struct Mode {
    code: u32,
    code_bits: u32,
    two_regions: bool,
    /// Endpoints x, y and z are stored as deltas from w.
    transformed: bool,
    bits: u32,
    delta: [u32; 3],
    layout: &'static [(u8, u8, u8)],
}

/// The spec's modes 1 to 14, in order.
const MODES: [Mode; 14] = [
    Mode {
        code: 0b00,
        code_bits: 2,
        two_regions: true,
        transformed: true,
        bits: 10,
        delta: [5, 5, 5],
        layout: &[
            (GY, 4, 1),
            (BY, 4, 1),
            (BZ, 4, 1),
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 5),
            (GZ, 4, 1),
            (GY, 0, 4),
            (GX, 0, 5),
            (BZ, 0, 1),
            (GZ, 0, 4),
            (BX, 0, 5),
            (BZ, 1, 1),
            (BY, 0, 4),
            (RY, 0, 5),
            (BZ, 2, 1),
            (RZ, 0, 5),
            (BZ, 3, 1),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b01,
        code_bits: 2,
        two_regions: true,
        transformed: true,
        bits: 7,
        delta: [6, 6, 6],
        layout: &[
            (GY, 5, 1),
            (GZ, 4, 1),
            (GZ, 5, 1),
            (RW, 0, 7),
            (BZ, 0, 1),
            (BZ, 1, 1),
            (BY, 4, 1),
            (GW, 0, 7),
            (BY, 5, 1),
            (BZ, 2, 1),
            (GY, 4, 1),
            (BW, 0, 7),
            (BZ, 3, 1),
            (BZ, 5, 1),
            (BZ, 4, 1),
            (RX, 0, 6),
            (GY, 0, 4),
            (GX, 0, 6),
            (GZ, 0, 4),
            (BX, 0, 6),
            (BY, 0, 4),
            (RY, 0, 6),
            (RZ, 0, 6),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b00010,
        code_bits: 5,
        two_regions: true,
        transformed: true,
        bits: 11,
        delta: [5, 4, 4],
        layout: &[
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 5),
            (RW, 10, 1),
            (GY, 0, 4),
            (GX, 0, 4),
            (GW, 10, 1),
            (BZ, 0, 1),
            (GZ, 0, 4),
            (BX, 0, 4),
            (BW, 10, 1),
            (BZ, 1, 1),
            (BY, 0, 4),
            (RY, 0, 5),
            (BZ, 2, 1),
            (RZ, 0, 5),
            (BZ, 3, 1),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b00110,
        code_bits: 5,
        two_regions: true,
        transformed: true,
        bits: 11,
        delta: [4, 5, 4],
        layout: &[
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 4),
            (RW, 10, 1),
            (GZ, 4, 1),
            (GY, 0, 4),
            (GX, 0, 5),
            (GW, 10, 1),
            (GZ, 0, 4),
            (BX, 0, 4),
            (BW, 10, 1),
            (BZ, 1, 1),
            (BY, 0, 4),
            (RY, 0, 4),
            (BZ, 0, 1),
            (BZ, 2, 1),
            (RZ, 0, 4),
            (GY, 4, 1),
            (BZ, 3, 1),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b01010,
        code_bits: 5,
        two_regions: true,
        transformed: true,
        bits: 11,
        delta: [4, 4, 5],
        layout: &[
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 4),
            (RW, 10, 1),
            (BY, 4, 1),
            (GY, 0, 4),
            (GX, 0, 4),
            (GW, 10, 1),
            (BZ, 0, 1),
            (GZ, 0, 4),
            (BX, 0, 5),
            (BW, 10, 1),
            (BY, 0, 4),
            (RY, 0, 4),
            (BZ, 1, 1),
            (BZ, 2, 1),
            (RZ, 0, 4),
            (BZ, 4, 1),
            (BZ, 3, 1),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b01110,
        code_bits: 5,
        two_regions: true,
        transformed: true,
        bits: 9,
        delta: [5, 5, 5],
        layout: &[
            (RW, 0, 9),
            (BY, 4, 1),
            (GW, 0, 9),
            (GY, 4, 1),
            (BW, 0, 9),
            (BZ, 4, 1),
            (RX, 0, 5),
            (GZ, 4, 1),
            (GY, 0, 4),
            (GX, 0, 5),
            (BZ, 0, 1),
            (GZ, 0, 4),
            (BX, 0, 5),
            (BZ, 1, 1),
            (BY, 0, 4),
            (RY, 0, 5),
            (BZ, 2, 1),
            (RZ, 0, 5),
            (BZ, 3, 1),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b10010,
        code_bits: 5,
        two_regions: true,
        transformed: true,
        bits: 8,
        delta: [6, 5, 5],
        layout: &[
            (RW, 0, 8),
            (GZ, 4, 1),
            (BY, 4, 1),
            (GW, 0, 8),
            (BZ, 2, 1),
            (GY, 4, 1),
            (BW, 0, 8),
            (BZ, 3, 1),
            (BZ, 4, 1),
            (RX, 0, 6),
            (GY, 0, 4),
            (GX, 0, 5),
            (BZ, 0, 1),
            (GZ, 0, 4),
            (BX, 0, 5),
            (BZ, 1, 1),
            (BY, 0, 4),
            (RY, 0, 6),
            (RZ, 0, 6),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b10110,
        code_bits: 5,
        two_regions: true,
        transformed: true,
        bits: 8,
        delta: [5, 6, 5],
        layout: &[
            (RW, 0, 8),
            (BZ, 0, 1),
            (BY, 4, 1),
            (GW, 0, 8),
            (GY, 5, 1),
            (GY, 4, 1),
            (BW, 0, 8),
            (GZ, 5, 1),
            (BZ, 4, 1),
            (RX, 0, 5),
            (GZ, 4, 1),
            (GY, 0, 4),
            (GX, 0, 6),
            (GZ, 0, 4),
            (BX, 0, 5),
            (BZ, 1, 1),
            (BY, 0, 4),
            (RY, 0, 5),
            (BZ, 2, 1),
            (RZ, 0, 5),
            (BZ, 3, 1),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b11010,
        code_bits: 5,
        two_regions: true,
        transformed: true,
        bits: 8,
        delta: [5, 5, 6],
        layout: &[
            (RW, 0, 8),
            (BZ, 1, 1),
            (BY, 4, 1),
            (GW, 0, 8),
            (BY, 5, 1),
            (GY, 4, 1),
            (BW, 0, 8),
            (BZ, 5, 1),
            (BZ, 4, 1),
            (RX, 0, 5),
            (GZ, 4, 1),
            (GY, 0, 4),
            (GX, 0, 5),
            (BZ, 0, 1),
            (GZ, 0, 4),
            (BX, 0, 6),
            (BY, 0, 4),
            (RY, 0, 5),
            (BZ, 2, 1),
            (RZ, 0, 5),
            (BZ, 3, 1),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b11110,
        code_bits: 5,
        two_regions: true,
        transformed: false,
        bits: 6,
        delta: [6, 6, 6],
        layout: &[
            (RW, 0, 6),
            (GZ, 4, 1),
            (BZ, 0, 1),
            (BZ, 1, 1),
            (BY, 4, 1),
            (GW, 0, 6),
            (GY, 5, 1),
            (BY, 5, 1),
            (BZ, 2, 1),
            (GY, 4, 1),
            (BW, 0, 6),
            (GZ, 5, 1),
            (BZ, 3, 1),
            (BZ, 5, 1),
            (BZ, 4, 1),
            (RX, 0, 6),
            (GY, 0, 4),
            (GX, 0, 6),
            (GZ, 0, 4),
            (BX, 0, 6),
            (BY, 0, 4),
            (RY, 0, 6),
            (RZ, 0, 6),
            (D, 0, 5),
        ],
    },
    Mode {
        code: 0b00011,
        code_bits: 5,
        two_regions: false,
        transformed: false,
        bits: 10,
        delta: [10, 10, 10],
        layout: &[
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 10),
            (GX, 0, 10),
            (BX, 0, 10),
        ],
    },
    Mode {
        code: 0b00111,
        code_bits: 5,
        two_regions: false,
        transformed: true,
        bits: 11,
        delta: [9, 9, 9],
        layout: &[
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 9),
            (RW, 10, 1),
            (GX, 0, 9),
            (GW, 10, 1),
            (BX, 0, 9),
            (BW, 10, 1),
        ],
    },
    // The high bits of w run backwards in the last two modes.
    Mode {
        code: 0b01011,
        code_bits: 5,
        two_regions: false,
        transformed: true,
        bits: 12,
        delta: [8, 8, 8],
        layout: &[
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 8),
            (RW, 11, 1),
            (RW, 10, 1),
            (GX, 0, 8),
            (GW, 11, 1),
            (GW, 10, 1),
            (BX, 0, 8),
            (BW, 11, 1),
            (BW, 10, 1),
        ],
    },
    Mode {
        code: 0b01111,
        code_bits: 5,
        two_regions: false,
        transformed: true,
        bits: 16,
        delta: [4, 4, 4],
        layout: &[
            (RW, 0, 10),
            (GW, 0, 10),
            (BW, 0, 10),
            (RX, 0, 4),
            (RW, 15, 1),
            (RW, 14, 1),
            (RW, 13, 1),
            (RW, 12, 1),
            (RW, 11, 1),
            (RW, 10, 1),
            (GX, 0, 4),
            (GW, 15, 1),
            (GW, 14, 1),
            (GW, 13, 1),
            (GW, 12, 1),
            (GW, 11, 1),
            (GW, 10, 1),
            (BX, 0, 4),
            (BW, 15, 1),
            (BW, 14, 1),
            (BW, 13, 1),
            (BW, 12, 1),
            (BW, 11, 1),
            (BW, 10, 1),
        ],
    },
];

/// A texel in the encoder's working domain: the half-float bit pattern
/// stretched by 64/31, which undoes the decoder's final `* 31 / 64`. Half
/// bits are close to logarithmic, so errors here are relative, as HDR wants.
fn working(value: f32) -> f32 {
    let bits = f16::from_f32(value.max(0.0)).to_bits().min(MAX_HALF);
    bits as f32 * 64.0 / 31.0
}

fn unquantize(q: u32, bits: u32) -> u32 {
    let max = (1 << bits) - 1;
    match q {
        _ if bits >= 15 => q,
        0 => 0,
        q if q == max => 0xFFFF,
        q => ((q << 16) + 0x8000) >> bits,
    }
}

fn quantize(working: f32, bits: u32) -> u32 {
    let max = (1u32 << bits) - 1;
    // The inverse of `unquantize`'s midpoint mapping, then the nearest of
    // the neighbours.
    let guess = ((working * (1u32 << bits) as f32 - 32768.0) / 65536.0)
        .round()
        .clamp(0.0, max as f32) as u32;
    [guess.saturating_sub(1), guess, (guess + 1).min(max)]
        .into_iter()
        .min_by(|a, b| {
            let da = (unquantize(*a, bits) as f32 - working).abs();
            let db = (unquantize(*b, bits) as f32 - working).abs();
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

fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

/// Whether pixel `i` of `shape` is in region 1.
fn region(shape: usize, i: usize) -> usize {
    (SHAPES[shape] >> i) as usize & 1
}

/// Whether pixel `i` stores its index with one bit fewer.
fn is_anchor(two_regions: bool, shape: usize, i: usize) -> bool {
    i == 0 || (two_regions && i == ANCHORS[shape] as usize)
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

    fn take(&mut self, count: u32) -> u32 {
        let value = ((self.block >> self.at) & ((1u128 << count) - 1)) as u32;
        self.at += count;
        value
    }
}

/// A block ready to write: the mode, each region's quantized endpoints as
/// [w, x, y, z][channel], the shape and every pixel's index.
struct Packed {
    mode: usize,
    ends: [[u32; 3]; 4],
    shape: usize,
    indices: [u32; 16],
}

impl Packed {
    fn write(&self) -> [u8; 16] {
        let mode = &MODES[self.mode];
        let mut fields = [0u32; 13];
        for (end, values) in self.ends.iter().enumerate() {
            for c in 0..3 {
                let value = values[c];
                fields[end * 3 + c] = if end > 0 && mode.transformed {
                    value.wrapping_sub(self.ends[0][c])
                } else {
                    value
                };
            }
        }
        fields[D as usize] = self.shape as u32;
        let mut bits = Bits { block: 0, at: 0 };
        bits.put(mode.code, mode.code_bits);
        for &(field, first, count) in mode.layout {
            bits.put(fields[field as usize] >> first, count as u32);
        }
        let index_bits = if mode.two_regions { 3 } else { 4 };
        for (i, index) in self.indices.iter().enumerate() {
            let anchor = is_anchor(mode.two_regions, self.shape, i);
            bits.put(*index, index_bits - anchor as u32);
        }
        debug_assert_eq!(bits.at, 128);
        bits.block.to_le_bytes()
    }
}

/// Decodes any unsigned BC6H block to halfs. A reserved mode decodes to
/// black, as the hardware does.
pub fn decode_block(block: &[u8; 16]) -> Result<[[u16; 3]; 16], String> {
    let mut bits = Bits {
        block: u128::from_le_bytes(*block),
        at: 0,
    };
    let mut code = bits.take(2);
    if code > 1 {
        code |= bits.take(3) << 2;
    }
    let Some(index) = MODES.iter().position(|mode| mode.code == code) else {
        return Ok([[0; 3]; 16]);
    };
    let mode = &MODES[index];
    let mut fields = [0u32; 13];
    for &(field, first, count) in mode.layout {
        fields[field as usize] |= bits.take(count as u32) << first;
    }
    let regions = if mode.two_regions { 2 } else { 1 };
    let mask = (1u32 << mode.bits) - 1;
    let mut ends = [[0u32; 3]; 4];
    for (end, values) in ends.iter_mut().enumerate().take(regions * 2) {
        for c in 0..3 {
            let raw = fields[end * 3 + c];
            values[c] = if end > 0 && mode.transformed {
                let delta = sign_extend(raw, mode.delta[c]);
                (fields[c] as i32).wrapping_add(delta) as u32 & mask
            } else {
                raw
            };
            values[c] = unquantize(values[c], mode.bits);
        }
    }
    let shape = if mode.two_regions {
        fields[D as usize] as usize
    } else {
        0
    };
    let (weights, index_bits): (&[u32], u32) = if mode.two_regions {
        (&WEIGHTS3, 3)
    } else {
        (&WEIGHTS4, 4)
    };
    Ok(std::array::from_fn(|i| {
        let anchor = is_anchor(mode.two_regions, shape, i);
        let weight = weights[bits.take(index_bits - anchor as u32) as usize];
        let r = if mode.two_regions {
            region(shape, i)
        } else {
            0
        };
        std::array::from_fn(|c| finish(interpolate(ends[r * 2][c], ends[r * 2 + 1][c], weight)))
    }))
}

type Point = [f32; 3];
type Line = (Point, Point);

fn distance(a: &Point, b: &Point) -> f32 {
    (0..3).map(|c| (a[c] - b[c]).powi(2)).sum()
}

/// A line through some points: the ends of their spread along the main
/// axis, refit twice by least squares against the weights they pick.
fn fit_line(points: &[Point], weights: &[u32]) -> (Point, Point) {
    let n = points.len().max(1) as f32;
    let mean: Point = std::array::from_fn(|c| points.iter().map(|p| p[c]).sum::<f32>() / n);
    let mut cov = [[0.0f32; 3]; 3];
    for p in points {
        for a in 0..3 {
            for b in 0..3 {
                cov[a][b] += (p[a] - mean[a]) * (p[b] - mean[b]);
            }
        }
    }
    // Power iteration from the diagonal finds the main axis.
    let mut axis = [1.0f32, 1.0, 1.0];
    for _ in 0..8 {
        let next: Point = std::array::from_fn(|a| (0..3).map(|b| cov[a][b] * axis[b]).sum());
        let length = next.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length < 1e-6 {
            break;
        }
        axis = next.map(|v| v / length);
    }
    let along = |p: &Point| (0..3).map(|c| (p[c] - mean[c]) * axis[c]).sum::<f32>();
    let (lo, hi) = points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
        (lo.min(along(p)), hi.max(along(p)))
    });
    let clamp = |v: f32| v.clamp(0.0, 65535.0);
    let mut ends = (
        std::array::from_fn(|c| clamp(mean[c] + axis[c] * lo)),
        std::array::from_fn(|c| clamp(mean[c] + axis[c] * hi)),
    );
    for _ in 0..2 {
        let picks: Vec<u32> = points.iter().map(|p| nearest(p, ends, weights).0).collect();
        match refit(points, &picks, weights) {
            Some(refit) => ends = refit,
            None => break,
        }
    }
    ends
}

fn palette_point(ends: (Point, Point), weight: u32) -> Point {
    let w = weight as f32 / 64.0;
    std::array::from_fn(|c| ends.0[c] * (1.0 - w) + ends.1[c] * w)
}

/// The weight index nearest `point` on a continuous line, and its error.
fn nearest(point: &Point, ends: (Point, Point), weights: &[u32]) -> (u32, f32) {
    (0..weights.len() as u32)
        .map(|i| {
            (
                i,
                distance(point, &palette_point(ends, weights[i as usize])),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
}

/// Least-squares endpoints for fixed weights, per channel.
fn refit(points: &[Point], picks: &[u32], weights: &[u32]) -> Option<(Point, Point)> {
    let (mut aa, mut ab, mut bb) = (0.0f32, 0.0f32, 0.0f32);
    let mut ax = [0.0f32; 3];
    let mut bx = [0.0f32; 3];
    for (point, pick) in points.iter().zip(picks) {
        let b = weights[*pick as usize] as f32 / 64.0;
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

/// Quantizes continuous region lines into `mode`, picks every index against
/// the decoded palette, and answers the block and its error, or none when
/// the endpoints' deltas don't fit the mode.
fn pack(
    points: &[Point; 16],
    mode_index: usize,
    shape: usize,
    lines: &[(Point, Point)],
) -> Option<(Packed, f32)> {
    let mode = &MODES[mode_index];
    let weights: &[u32] = if mode.two_regions {
        &WEIGHTS3
    } else {
        &WEIGHTS4
    };
    let mut ends = [[0u32; 3]; 4];
    for (r, line) in lines.iter().enumerate() {
        ends[r * 2] = line.0.map(|v| quantize(v, mode.bits));
        ends[r * 2 + 1] = line.1.map(|v| quantize(v, mode.bits));
    }
    let palettes: Vec<Vec<Point>> = (0..lines.len())
        .map(|r| {
            weights
                .iter()
                .map(|w| {
                    std::array::from_fn(|c| {
                        let e0 = unquantize(ends[r * 2][c], mode.bits);
                        let e1 = unquantize(ends[r * 2 + 1][c], mode.bits);
                        interpolate(e0, e1, *w) as f32
                    })
                })
                .collect()
        })
        .collect();
    let mut indices = [0u32; 16];
    let mut error = 0.0;
    for (i, point) in points.iter().enumerate() {
        let r = if mode.two_regions {
            region(shape, i)
        } else {
            0
        };
        let (index, e) = palettes[r]
            .iter()
            .enumerate()
            .map(|(k, p)| (k as u32, distance(point, p)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        indices[i] = index;
        error += e;
    }
    // An anchor's index keeps one bit fewer, so its top bit must be clear:
    // swap that region's ends and mirror its indices when it isn't. The
    // weights are symmetric, so the error stays the same.
    let top = weights.len() as u32 / 2;
    for r in 0..lines.len() {
        let anchor = if r == 0 { 0 } else { ANCHORS[shape] as usize };
        if indices[anchor] >= top {
            ends.swap(r * 2, r * 2 + 1);
            for (i, index) in indices.iter_mut().enumerate() {
                let here = if mode.two_regions {
                    region(shape, i)
                } else {
                    0
                };
                if here == r {
                    *index = weights.len() as u32 - 1 - *index;
                }
            }
        }
    }
    if mode.transformed {
        let wrap = 1i32 << mode.bits;
        for end in &ends[1..lines.len() * 2] {
            for c in 0..3 {
                let mut delta = (end[c] as i32 - ends[0][c] as i32).rem_euclid(wrap);
                if delta >= wrap / 2 {
                    delta -= wrap;
                }
                let limit = 1i32 << (mode.delta[c] - 1);
                if delta < -limit || delta >= limit {
                    return None;
                }
            }
        }
    }
    Some((
        Packed {
            mode: mode_index,
            ends,
            shape,
            indices,
        },
        error,
    ))
}

/// Below this error a one-region block is kept without trying two regions:
/// about half a half-float step per channel per texel.
const GOOD_ENOUGH: f32 = 16.0 * 3.0;
/// How many of the best-fitting shapes get every two-region mode tried.
const SHAPES_TRIED: usize = 4;

/// Encodes one 4x4 block of linear RGB, row-major, in whichever of the 14
/// modes keeps it closest: one region at up to 16 bits for smooth blocks,
/// two regions over the best-fitting shapes for blocks with an edge.
pub fn encode_block(texels: &[[f32; 3]; 16]) -> [u8; 16] {
    let points: [Point; 16] = texels.map(|t| t.map(working));
    let line = fit_line(&points, &WEIGHTS4);
    let mut best: Option<(Packed, f32)> = None;
    fn consider(best: &mut Option<(Packed, f32)>, candidate: Option<(Packed, f32)>) {
        if let Some(candidate) = candidate
            && best.as_ref().is_none_or(|best| candidate.1 < best.1)
        {
            *best = Some(candidate);
        }
    }
    for mode in 10..14 {
        consider(&mut best, pack(&points, mode, 0, &[line]));
    }
    if best.as_ref().is_none_or(|best| best.1 > GOOD_ENOUGH) {
        // Rank shapes by how well two continuous lines fit, before any
        // quantization, then try every mode on the few best.
        let mut shapes: Vec<(usize, f32, [Line; 2])> = (0..32)
            .map(|shape| {
                let lines: [(Point, Point); 2] = std::array::from_fn(|r| {
                    let members: Vec<Point> = (0..16)
                        .filter(|&i| region(shape, i) == r)
                        .map(|i| points[i])
                        .collect();
                    fit_line(&members, &WEIGHTS3)
                });
                let error = (0..16)
                    .map(|i| nearest(&points[i], lines[region(shape, i)], &WEIGHTS3).1)
                    .sum();
                (shape, error, lines)
            })
            .collect();
        shapes.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (shape, _, lines) in shapes.iter().take(SHAPES_TRIED) {
            for mode in 0..10 {
                consider(&mut best, pack(&points, mode, *shape, lines));
            }
        }
    }
    // Mode 11 stores both ends outright, so it always fits.
    best.expect("mode 11 always packs").0.write()
}

/// Encodes one square face, whose side is a multiple of 4, block row-major.
/// Rows of blocks are spread over the machine's cores: searching every mode
/// is slow enough that a large sky would otherwise hold a build up.
pub fn encode_face(texels: &[[f32; 3]], size: u32) -> Vec<u8> {
    let blocks = (size / 4) as usize;
    let row_bytes = blocks * 16;
    let mut out = vec![0u8; blocks * row_bytes];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows_each = blocks.div_ceil(threads).max(1);
    let control = crate::build_control::current();
    std::thread::scope(|scope| {
        for (chunk, rows) in out.chunks_mut(rows_each * row_bytes).enumerate() {
            let control = control.clone();
            scope.spawn(move || {
                for (r, row) in rows.chunks_mut(row_bytes).enumerate() {
                    let by = chunk * rows_each + r;
                    for bx in 0..blocks {
                        if control
                            .as_ref()
                            .is_some_and(crate::build_control::BuildControl::cancelled)
                        {
                            return;
                        }
                        let block: [[f32; 3]; 16] = std::array::from_fn(|i| {
                            let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                            texels[y * size as usize + x]
                        });
                        row[bx * 16..bx * 16 + 16].copy_from_slice(&encode_block(&block));
                    }
                }
            });
        }
    });
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
    write_dds_cube_levels(std::slice::from_ref(cube))
}

/// A cube and its mip chain as one DDS file, each face followed by its
/// smaller levels, the order a cube texture is uploaded in. Every level's
/// side must be a multiple of 4, halving from the first.
pub fn write_dds_cube_levels(levels: &[HdrCube]) -> Vec<u8> {
    let cube = &levels[0];
    let face_bytes = (cube.size / 4) * (cube.size / 4) * 16;
    let mut out = Vec::with_capacity(HEADER_LEN + face_bytes as usize * 8);
    let u32s = |values: &[u32], out: &mut Vec<u8>| {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    };
    let mips = levels.len() as u32;
    let mipped = mips > 1;
    out.extend_from_slice(DDS_MAGIC);
    // size, flags (caps|height|width|pixelformat|linearsize, plus mip count
    // when there are several), height, width, linear size, depth, mip count,
    // 11 reserved.
    let flags = 0x0008_1007 | if mipped { 0x0002_0000 } else { 0 };
    u32s(
        &[124, flags, cube.size, cube.size, face_bytes, 0, mips],
        &mut out,
    );
    u32s(&[0; 11], &mut out);
    // Pixel format: a FourCC of DX10, which says the real format follows.
    u32s(&[32, 0x4], &mut out);
    out.extend_from_slice(b"DX10");
    u32s(&[0; 5], &mut out);
    // Caps: texture|complex (|mipmap), then cubemap with all six faces.
    let caps = 0x1008 | if mipped { 0x40_0000 } else { 0 };
    u32s(&[caps, 0xFE00, 0, 0, 0], &mut out);
    // DX10: format, 2D, cube flag, one cube, no alpha info.
    u32s(&[DXGI_BC6H_UF16, 3, 0x4, 1, 0], &mut out);
    for face in 0..6 {
        for level in levels {
            out.extend_from_slice(&encode_face(&level.faces[face], level.size));
        }
    }
    out
}

/// A BC6H cube read out of a DDS file.
pub struct DdsCube<'a> {
    /// The first level's face side.
    pub size: u32,
    pub mips: u32,
    /// Every face's levels, face after face.
    pub blocks: &'a [u8],
}

impl DdsCube<'_> {
    /// A level's face side.
    pub fn level_size(&self, mip: u32) -> u32 {
        (self.size >> mip).max(4)
    }

    /// Bytes of one face at one level.
    pub fn level_bytes(&self, mip: u32) -> usize {
        let blocks = self.level_size(mip) / 4;
        (blocks * blocks * 16) as usize
    }
}

/// Reads back what [`write_dds_cube_levels`] wrote.
pub fn read_dds_cube_levels(bytes: &[u8]) -> Result<DdsCube<'_>, String> {
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
    let mips = word(28)?.max(1);
    if mips > 1 && (!size.is_power_of_two() || size >> (mips - 1) < 4) {
        return Err(format!("{mips} levels don't fit a {size} texel face"));
    }
    let mut cube = DdsCube {
        size,
        mips,
        blocks: &[],
    };
    let total: usize = (0..mips).map(|mip| cube.level_bytes(mip)).sum::<usize>() * 6;
    cube.blocks = bytes
        .get(HEADER_LEN..HEADER_LEN + total)
        .ok_or_else(|| "DDS cube is truncated".to_string())?;
    Ok(cube)
}

/// Reads back what [`write_dds_cube`] wrote: the face side and the six faces'
/// blocks, face after face. A file with mips is refused.
pub fn read_dds_cube(bytes: &[u8]) -> Result<(u32, &[u8]), String> {
    let cube = read_dds_cube_levels(bytes)?;
    if cube.mips != 1 {
        return Err("expected a DDS cube without mips".to_string());
    }
    Ok((cube.size, cube.blocks))
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

    /// Modes 1, 7 and 13 decoded by an independent decoder (bcdec).
    const REFERENCE: [([u8; 16], [[u16; 3]; 16]); 3] = [
        (
            [
                120, 233, 185, 210, 151, 247, 204, 148, 71, 255, 147, 103, 254, 74, 103, 147,
            ],
            [
                [26148, 11516, 31046],
                [26087, 11547, 31085],
                [26176, 11622, 30961],
                [26176, 11622, 30961],
                [26108, 11377, 31002],
                [25897, 11641, 31207],
                [25714, 11733, 31325],
                [26086, 11299, 31015],
                [26197, 11700, 30948],
                [26087, 11547, 31085],
                [25836, 11672, 31247],
                [26176, 11622, 30961],
                [26108, 11377, 31002],
                [26108, 11377, 31002],
                [25897, 11641, 31207],
                [25897, 11641, 31207],
            ],
        ),
        (
            [
                210, 247, 209, 43, 70, 138, 171, 111, 31, 6, 75, 159, 244, 201, 18, 51,
            ],
            [
                [23901, 20134, 2631],
                [24195, 19987, 2594],
                [25377, 18910, 2080],
                [25110, 18910, 2170],
                [23761, 20204, 2648],
                [23761, 20204, 2648],
                [24335, 19917, 2576],
                [25110, 18910, 2170],
                [23761, 20204, 2648],
                [23761, 20204, 2648],
                [24040, 20064, 2613],
                [25429, 18910, 2063],
                [23761, 20204, 2648],
                [24474, 19847, 2559],
                [24195, 19987, 2594],
                [23761, 20204, 2648],
            ],
        ),
        (
            [
                43, 153, 252, 243, 212, 156, 27, 119, 241, 206, 101, 182, 171, 11, 196, 201,
            ],
            [
                [25369, 7885, 12845],
                [24579, 7606, 12706],
                [24628, 7624, 12714],
                [24739, 7663, 12734],
                [25110, 7794, 12800],
                [25048, 7772, 12788],
                [25048, 7772, 12788],
                [24789, 7680, 12743],
                [24789, 7680, 12743],
                [24838, 7698, 12752],
                [24789, 7680, 12743],
                [25369, 7885, 12845],
                [25159, 7811, 12808],
                [24739, 7663, 12734],
                [24900, 7719, 12762],
                [24739, 7663, 12734],
            ],
        ),
    ];

    #[test]
    fn two_region_delta_and_high_precision_modes_decode_as_the_reference_does() {
        for (block, expected) in REFERENCE {
            assert_eq!(decode_block(&block).unwrap(), expected);
        }
        // A reserved mode is black, as the hardware makes it.
        assert_eq!(decode_block(&[0b10011; 16]).unwrap(), [[0; 3]; 16]);
    }

    fn mode_of(block: &[u8; 16]) -> usize {
        let low = block[0] as u32 & 0b11;
        let code = if low > 1 {
            block[0] as u32 & 0b11111
        } else {
            low
        };
        MODES.iter().position(|mode| mode.code == code).unwrap()
    }

    /// How far a block decodes from its texels, in half-float steps.
    fn steps(block: &[u8; 16], texels: &[[f32; 3]; 16]) -> u32 {
        let back = decode_block(block).unwrap();
        (0..16)
            .flat_map(|i| (0..3).map(move |c| (i, c)))
            .map(|(i, c)| {
                (f16::from_f32(texels[i][c]).to_bits() as i32 - back[i][c] as i32).unsigned_abs()
            })
            .sum()
    }

    #[test]
    fn an_edge_takes_two_regions_when_one_line_cant_hold_it() {
        // A lit wall beside ground that shades from brown to green: three
        // colors no one line passes through.
        let texels: [[f32; 3]; 16] = std::array::from_fn(|i| {
            let t = (i % 4) as f32 / 3.0;
            if i < 8 {
                [1.2, 1.1, 0.9]
            } else {
                [0.6 - 0.3 * t, 0.3 + 0.4 * t, 0.2]
            }
        });
        let block = encode_block(&texels);
        assert!(MODES[mode_of(&block)].two_regions);
        let points = texels.map(|t| t.map(working));
        let line = fit_line(&points, &WEIGHTS4);
        let one_region = (10..14)
            .filter_map(|mode| pack(&points, mode, 0, &[line]))
            .map(|(packed, _)| steps(&packed.write(), &texels))
            .min()
            .unwrap();
        assert!(
            steps(&block, &texels) * 2 < one_region,
            "{} vs {one_region}",
            steps(&block, &texels)
        );
    }

    #[test]
    fn a_smooth_block_stays_in_one_region_at_high_precision() {
        let texels: [[f32; 3]; 16] = std::array::from_fn(|i| {
            let v = 3.0 + i as f32 * 0.01;
            [v, v * 0.9, v * 0.8]
        });
        let block = encode_block(&texels);
        assert!(!MODES[mode_of(&block)].two_regions);
        // Within a few half-float steps of every channel.
        assert!(
            steps(&block, &texels) <= 48 * 3,
            "{}",
            steps(&block, &texels)
        );
    }

    #[test]
    fn a_face_encodes_the_same_on_every_thread() {
        let texels: Vec<[f32; 3]> = (0..32 * 32)
            .map(|i| [(i % 32) as f32 * 0.1, (i / 32) as f32 * 0.2, 1.0])
            .collect();
        let face = encode_face(&texels, 32);
        assert_eq!(face.len(), 8 * 8 * 16);
        let last: [[f32; 3]; 16] = std::array::from_fn(|i| texels[(28 + i / 4) * 32 + 28 + i % 4]);
        assert_eq!(face[face.len() - 16..], encode_block(&last));
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

    #[test]
    fn a_mip_chain_survives_the_dds_file_face_by_face() {
        let faces = std::array::from_fn(|face| vec![[face as f32 + 0.5; 3]; 256]);
        let levels = HdrCube { size: 16, faces }.mip_chain(4);
        assert_eq!(levels.len(), 3);
        let file = write_dds_cube_levels(&levels);
        let cube = read_dds_cube_levels(&file).unwrap();
        assert_eq!((cube.size, cube.mips), (16, 3));
        // Face 2's smallest level: past two whole faces and face 2's first
        // two levels.
        let per_face: usize = (0..3).map(|mip| cube.level_bytes(mip)).sum();
        let at = per_face * 2 + cube.level_bytes(0) + cube.level_bytes(1);
        let texels = decode_face(&cube.blocks[at..at + cube.level_bytes(2)], 4).unwrap();
        let red = f16::from_le_bytes([texels[0], texels[1]]).to_f32();
        assert!(relative_error(red, 2.5) < 0.02, "{red}");
        // The single-level reader refuses it rather than misreading it.
        assert!(read_dds_cube(&file).is_err());
    }
}
