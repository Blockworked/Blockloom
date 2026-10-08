//! 2D post: the pixel-art and retro looks laid over the finished frame -
//! pixelation, dither, colour quantize or a fixed palette, an edge outline
//! and a CRT preset. The CPU functions are the reference the shader copies.

use serde::{Deserialize, Serialize};

/// Colours a palette may hold.
pub const MAX_PALETTE: usize = 32;
/// Largest pixel block pixelation takes.
pub const MAX_PIXEL_SIZE: u32 = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Outline {
    /// 0-1, how dark (or bright) the edge is drawn; 0 is off.
    pub strength: f32,
    pub color: String,
    /// Luma step between neighbouring pixels that counts as an edge.
    pub threshold: f32,
}

impl Default for Outline {
    fn default() -> Self {
        Self {
            strength: 0.0,
            color: "#000000".to_string(),
            threshold: 0.2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crt {
    /// 0-1, how dark the gaps between scanlines are.
    pub scanlines: f32,
    /// 0-1, how far the screen bows out.
    pub curvature: f32,
    /// 0-1, darkened corners.
    pub vignette: f32,
    /// 0-1, an RGB phosphor mask.
    pub mask: f32,
}

impl Default for Crt {
    fn default() -> Self {
        Self {
            scanlines: 0.0,
            curvature: 0.0,
            vignette: 0.0,
            mask: 0.0,
        }
    }
}

impl Crt {
    pub fn is_on(&self) -> bool {
        self.scanlines > 0.0 || self.curvature > 0.0 || self.vignette > 0.0 || self.mask > 0.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Post2d {
    /// Pixels in one block of the picture; 0 and 1 are off.
    pub pixelation: u32,
    /// Levels per colour channel (2-32), 0 for off. Ignored with a palette.
    pub levels: u32,
    /// Hex colours the picture snaps to, empty for none.
    pub palette: Vec<String>,
    /// 0-1, an ordered dither across the quantize step.
    pub dither: f32,
    pub outline: Outline,
    pub crt: Crt,
    /// 0-1, only the left of this fraction of the frame gets the look; 0 and
    /// 1 are the whole frame. A debug aid for comparing against the plain
    /// picture.
    pub split: f32,
}

impl Default for Post2d {
    fn default() -> Self {
        Self {
            pixelation: 0,
            levels: 0,
            palette: Vec::new(),
            dither: 0.0,
            outline: Outline::default(),
            crt: Crt::default(),
            split: 0.0,
        }
    }
}

fn unit(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

impl Post2d {
    pub fn normalize(&mut self) {
        self.pixelation = self.pixelation.min(MAX_PIXEL_SIZE);
        if self.levels != 0 {
            self.levels = self.levels.clamp(2, 32);
        }
        self.palette.retain(|c| parse_srgb(c).is_some());
        self.palette.truncate(MAX_PALETTE);
        self.dither = unit(self.dither);
        self.outline.strength = unit(self.outline.strength);
        self.outline.threshold = if self.outline.threshold.is_finite() {
            self.outline.threshold.clamp(0.01, 1.0)
        } else {
            0.2
        };
        if parse_srgb(&self.outline.color).is_none() {
            self.outline.color = "#000000".to_string();
        }
        self.crt.scanlines = unit(self.crt.scanlines);
        self.crt.curvature = unit(self.crt.curvature);
        self.crt.vignette = unit(self.crt.vignette);
        self.crt.mask = unit(self.crt.mask);
        self.split = unit(self.split);
    }

    /// Whether any effect is on (the pass is skipped otherwise).
    pub fn is_on(&self) -> bool {
        self.pixelation > 1
            || self.levels > 0
            || !self.palette.is_empty()
            || self.outline.strength > 0.0
            || self.crt.is_on()
    }

    /// One number by the name `set [look] to` uses; a value that doesn't
    /// read is ignored.
    pub fn set(&mut self, dial: crate::blocks::Look2dDial, value: &str) {
        use crate::blocks::Look2dDial as D;
        let Ok(v) = value.trim().parse::<f32>() else {
            return;
        };
        if !v.is_finite() {
            return;
        }
        match dial {
            D::Pixelation => self.pixelation = v.max(0.0).round() as u32,
            D::Levels => self.levels = v.max(0.0).round() as u32,
            D::Dither => self.dither = v,
            D::Outline => self.outline.strength = v,
            D::Scanlines => self.crt.scanlines = v,
            D::Curvature => self.crt.curvature = v,
            D::Vignette => self.crt.vignette = v,
            D::AmbientLight | D::AmbientColor => {}
        }
        self.normalize();
    }
}

/// `#RRGGBB` as display-referred 0-1 channels.
pub fn parse_srgb(hex: &str) -> Option<[f32; 3]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([
        byte(0)? as f32 / 255.0,
        byte(2)? as f32 / 255.0,
        byte(4)? as f32 / 255.0,
    ])
}

/// The 4x4 ordered-dither threshold at a pixel, in (-0.5, 0.5).
pub fn bayer4(x: u32, y: u32) -> f32 {
    const M: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (M[(y % 4) as usize][(x % 4) as usize] as f32 + 0.5) / 16.0 - 0.5
}

/// Snaps each channel to `levels` steps, nudged by `dither` first.
pub fn quantize(color: [f32; 3], levels: u32, dither: f32, x: u32, y: u32) -> [f32; 3] {
    let steps = (levels.max(2) - 1) as f32;
    let nudge = bayer4(x, y) * dither / steps;
    color.map(|c| ((c + nudge).clamp(0.0, 1.0) * steps).round() / steps)
}

/// The palette entry nearest the colour, after the same dither nudge.
pub fn nearest(color: [f32; 3], palette: &[[f32; 3]], dither: f32, x: u32, y: u32) -> [f32; 3] {
    let nudge = bayer4(x, y) * dither * 0.25;
    let c = color.map(|v| v + nudge);
    let mut best = color;
    let mut best_d = f32::MAX;
    for p in palette {
        let d = (0..3).map(|i| (p[i] - c[i]).powi(2)).sum::<f32>();
        if d < best_d {
            best_d = d;
            best = *p;
        }
    }
    best
}

/// Pulls a screen position (0-1) in towards the middle the way a bowed CRT
/// glass bends it; outside the glass comes back as `None`.
pub fn bow(uv: [f32; 2], curvature: f32) -> Option<[f32; 2]> {
    if curvature <= 0.0 {
        return Some(uv);
    }
    let c = [uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0];
    let k = curvature * 0.25;
    let w = [
        c[0] * (1.0 + k * c[1] * c[1]),
        c[1] * (1.0 + k * c[0] * c[0]),
    ];
    if w[0].abs() > 1.0 || w[1].abs() > 1.0 {
        return None;
    }
    Some([w[0] * 0.5 + 0.5, w[1] * 0.5 + 0.5])
}

/// Brightness multiplier for a scanline at pixel row `y`.
pub fn scanline(y: u32, strength: f32) -> f32 {
    if y.is_multiple_of(2) {
        1.0
    } else {
        1.0 - strength * 0.6
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Look2dDial;

    #[test]
    fn bayer_thresholds_are_balanced() {
        let sum: f32 = (0..16).map(|i| bayer4(i % 4, i / 4)).sum();
        assert!(sum.abs() < 1e-4);
        assert!((0..16).all(|i| bayer4(i % 4, i / 4).abs() < 0.5));
    }

    #[test]
    fn quantize_snaps_to_steps() {
        assert_eq!(quantize([0.1, 0.5, 0.9], 2, 0.0, 0, 0), [0.0, 1.0, 1.0]);
        let q = quantize([0.3, 0.3, 0.3], 4, 0.0, 0, 0);
        assert!((q[0] - 1.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn dither_changes_a_midtone_by_pixel() {
        let a = quantize([0.5; 3], 2, 1.0, 0, 0)[0];
        let b = quantize([0.5; 3], 2, 1.0, 3, 0)[0];
        assert_ne!(a, b);
    }

    #[test]
    fn the_palette_picks_the_nearest_colour() {
        let pal = [[0.0; 3], [1.0, 0.0, 0.0], [1.0; 3]];
        assert_eq!(nearest([0.9, 0.1, 0.1], &pal, 0.0, 0, 0), [1.0, 0.0, 0.0]);
        assert_eq!(nearest([0.1; 3], &pal, 0.0, 0, 0), [0.0; 3]);
    }

    #[test]
    fn the_glass_bows_towards_the_corners_and_clips() {
        assert_eq!(bow([0.3, 0.6], 0.0), Some([0.3, 0.6]));
        let mid = bow([0.5, 0.5], 1.0).unwrap();
        assert!((mid[0] - 0.5).abs() < 1e-6);
        assert_eq!(bow([0.999, 0.999], 1.0), None);
    }

    #[test]
    fn dials_clamp_and_ignore_nonsense() {
        let mut p = Post2d::default();
        p.set(Look2dDial::Pixelation, "4");
        p.set(Look2dDial::Pixelation, "wide");
        assert_eq!(p.pixelation, 4);
        p.set(Look2dDial::Pixelation, "9999");
        assert_eq!(p.pixelation, MAX_PIXEL_SIZE);
        p.set(Look2dDial::Levels, "1");
        assert_eq!(p.levels, 2);
        p.set(Look2dDial::Scanlines, "7");
        assert_eq!(p.crt.scanlines, 1.0);
        assert!(p.is_on());
    }

    #[test]
    fn normalize_drops_bad_colours_and_caps_the_palette() {
        let mut p = Post2d::default();
        p.palette = vec!["#ff0000".into(), "nope".into()];
        p.palette.extend((0..40).map(|_| "#00ff00".to_string()));
        p.normalize();
        assert_eq!(p.palette.len(), MAX_PALETTE);
        assert!(!Post2d::default().is_on());
    }
}
