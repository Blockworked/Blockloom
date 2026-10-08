//! 2D camera: axis locks, bounds with a soft edge, zoom by view height,
//! pixel snapping, rotation and trauma-based shake. The follow itself is the
//! PlayerCamera's (`player_camera.rs`); this lays its finishing rules over the
//! pose it produced. All of it is plain arithmetic the runtime applies.

use serde::{Deserialize, Serialize};

/// The world rectangle the camera's view must stay inside.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shake {
    /// Pixels the view can be thrown at full trauma.
    pub max_offset: f32,
    /// Degrees the view can be turned at full trauma.
    pub max_angle: f32,
    /// Noise steps a second.
    pub frequency: f32,
    /// Trauma lost each second.
    pub decay: f32,
}

impl Default for Shake {
    fn default() -> Self {
        Self {
            max_offset: 24.0,
            max_angle: 3.0,
            frequency: 25.0,
            decay: 1.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Camera2dSettings {
    pub lock_x: bool,
    pub lock_y: bool,
    /// The world rectangle the view stays inside, `None` for no limit.
    pub bounds: Option<Bounds>,
    /// World units over which the camera eases to a stop at a bound.
    pub soft_edge: f32,
    /// World units the view is tall; 0 leaves the zoom as it is.
    pub zoom_height: f32,
    /// Pixels per world unit set by `set camera zoom`, `None` for the
    /// project's.
    pub zoom: Option<f32>,
    /// Whole-pixel zoom and positions, for pixel art.
    pub pixel_snap: bool,
    /// Degrees the view is turned, counter-clockwise.
    pub rotation: f32,
    pub shake: Shake,
}

fn finite(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

impl Camera2dSettings {
    pub fn normalize(&mut self) {
        self.soft_edge = finite(self.soft_edge, 0.0).max(0.0);
        self.zoom_height = finite(self.zoom_height, 0.0).max(0.0);
        self.zoom = self
            .zoom
            .filter(|z| z.is_finite())
            .map(|z| z.clamp(0.05, 64.0));
        self.rotation = finite(self.rotation, 0.0).rem_euclid(360.0);
        self.shake.max_offset = finite(self.shake.max_offset, 24.0).clamp(0.0, 2000.0);
        self.shake.max_angle = finite(self.shake.max_angle, 3.0).clamp(0.0, 90.0);
        self.shake.frequency = finite(self.shake.frequency, 25.0).clamp(0.1, 200.0);
        self.shake.decay = finite(self.shake.decay, 1.5).clamp(0.0, 100.0);
        if let Some(b) = &self.bounds
            && !b.min.iter().chain(&b.max).all(|v| v.is_finite())
        {
            self.bounds = None;
        }
    }

    /// Whether anything here changes the pose or the zoom.
    pub fn is_on(&self) -> bool {
        self.lock_x
            || self.lock_y
            || self.bounds.is_some()
            || self.zoom_height > 0.0
            || self.zoom.is_some()
            || self.pixel_snap
            || self.rotation != 0.0
    }

    /// One `set [look] to` dial (the camera ones); other dials are ignored,
    /// and so is a value that doesn't read.
    pub fn set(&mut self, dial: crate::blocks::Look2dDial, value: &str) {
        use crate::blocks::Look2dDial as D;
        let Ok(v) = value.trim().parse::<f32>() else {
            return;
        };
        if !v.is_finite() {
            return;
        }
        let edge = |bounds: &mut Option<Bounds>, side: usize, axis: usize, v: f32| {
            let b = bounds.get_or_insert(Bounds {
                min: [f32::NEG_INFINITY; 2],
                max: [f32::INFINITY; 2],
            });
            if side == 0 {
                b.min[axis] = v;
            } else {
                b.max[axis] = v;
            }
        };
        match dial {
            D::Zoom => self.zoom = Some(v),
            D::ZoomHeight => self.zoom_height = v,
            D::Rotation => self.rotation = v,
            D::LockX => self.lock_x = v != 0.0,
            D::LockY => self.lock_y = v != 0.0,
            D::PixelSnap => self.pixel_snap = v != 0.0,
            D::BoundsLeft => edge(&mut self.bounds, 0, 0, v),
            D::BoundsRight => edge(&mut self.bounds, 1, 0, v),
            D::BoundsBottom => edge(&mut self.bounds, 0, 1, v),
            D::BoundsTop => edge(&mut self.bounds, 1, 1, v),
            _ => return,
        }
        // An unbounded side stays unbounded; only a bound that was never
        // given a finite edge is dropped by `normalize`.
        let open = self
            .bounds
            .is_some_and(|b| b.min.iter().chain(&b.max).all(|v| v.is_infinite()));
        if open {
            self.bounds = None;
        }
        let keep = self.bounds;
        self.normalize_keeping(keep);
    }

    fn normalize_keeping(&mut self, bounds: Option<Bounds>) {
        self.normalize();
        // `normalize` drops non-finite edges; a partly open bound is fine
        // at run time, so put it back.
        if bounds.is_some() {
            self.bounds = bounds;
        }
    }
}

/// Keeps one axis of the view's centre inside `[lo + half, hi - half]`,
/// easing to a stop over `soft`. Returns the new centre and whether it was
/// held. A view bigger than the range sits in the middle of it.
pub fn confine_axis(p: f32, lo: f32, hi: f32, half: f32, soft: f32) -> (f32, bool) {
    let (a, b) = (lo + half, hi - half);
    if a > b {
        return ((lo + hi) * 0.5, true);
    }
    if soft <= 0.0 {
        let q = p.clamp(a, b);
        return (q, q != p);
    }
    let soft = soft.min((b - a) * 0.5);
    if p > b - soft {
        let d = p - (b - soft);
        (b - soft + soft * (1.0 - (-d / soft).exp()), d > soft * 0.5)
    } else if p < a + soft {
        let d = (a + soft) - p;
        (a + soft - soft * (1.0 - (-d / soft).exp()), d > soft * 0.5)
    } else {
        (p, false)
    }
}

/// The view's centre kept inside the bounds, with whether it is being held.
pub fn confine(centre: [f32; 2], half: [f32; 2], bounds: &Bounds, soft: f32) -> ([f32; 2], bool) {
    let (x, hx) = confine_axis(centre[0], bounds.min[0], bounds.max[0], half[0], soft);
    let (y, hy) = confine_axis(centre[1], bounds.min[1], bounds.max[1], half[1], soft);
    ([x, y], hx || hy)
}

/// Orthographic scale (world units a pixel covers) that makes the view
/// `height` world units tall on a screen `pixels` tall.
pub fn scale_for_height(height: f32, pixels: f32) -> f32 {
    if height <= 0.0 || pixels <= 0.0 {
        return 1.0;
    }
    height / pixels
}

/// A scale snapped so a world unit is a whole number of pixels (zoomed in)
/// or a pixel a whole number of units (zoomed out).
pub fn snap_scale(scale: f32) -> f32 {
    let scale = scale.max(0.01);
    if scale <= 1.0 {
        1.0 / (1.0 / scale).round().max(1.0)
    } else {
        scale.round()
    }
}

/// A position snapped to the whole pixels of a view with this scale.
pub fn snap_position(p: [f32; 2], scale: f32) -> [f32; 2] {
    let unit = scale.max(0.0001);
    [(p[0] / unit).round() * unit, (p[1] / unit).round() * unit]
}

fn hash(seed: u32, i: i32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Smooth 1D value noise in -1..1.
pub fn noise(seed: u32, t: f32) -> f32 {
    let i = t.floor();
    let f = t - i;
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash(seed, i as i32);
    let b = hash(seed, i as i32 + 1);
    a + (b - a) * s
}

/// The throw `trauma` (0-1) puts on the view at `time`: an offset in pixels
/// and an angle in degrees. Trauma is squared so small hits stay subtle.
pub fn shake_at(trauma: f32, time: f32, shake: &Shake) -> ([f32; 2], f32) {
    let power = trauma.clamp(0.0, 1.0).powi(2);
    if power == 0.0 {
        return ([0.0; 2], 0.0);
    }
    let t = time * shake.frequency;
    (
        [
            shake.max_offset * power * noise(1, t),
            shake.max_offset * power * noise(2, t),
        ],
        shake.max_angle * power * noise(3, t),
    )
}

/// Trauma after `dt` seconds of decay.
pub fn decay(trauma: f32, dt: f32, rate: f32) -> f32 {
    (trauma - rate * dt.max(0.0)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Look2dDial;

    #[test]
    fn a_hard_bound_holds_the_view_inside() {
        let (x, held) = confine_axis(95.0, 0.0, 100.0, 10.0, 0.0);
        assert_eq!((x, held), (90.0, true));
        assert_eq!(confine_axis(50.0, 0.0, 100.0, 10.0, 0.0), (50.0, false));
        assert_eq!(confine_axis(-5.0, 0.0, 100.0, 10.0, 0.0), (10.0, true));
    }

    #[test]
    fn a_view_bigger_than_the_world_sits_in_the_middle() {
        assert_eq!(confine_axis(3.0, 0.0, 10.0, 8.0, 0.0), (5.0, true));
    }

    #[test]
    fn a_soft_edge_eases_but_never_passes_the_bound() {
        let mut last = 0.0;
        for p in [70.0, 80.0, 90.0, 120.0, 400.0] {
            let (x, _) = confine_axis(p, 0.0, 100.0, 0.0, 20.0);
            assert!(x <= 100.0 && x >= last, "{p}: {x}");
            last = x;
        }
        assert_eq!(confine_axis(50.0, 0.0, 100.0, 0.0, 20.0).0, 50.0);
        let (below, _) = confine_axis(-300.0, 0.0, 100.0, 0.0, 20.0);
        assert!((0.0..1.0).contains(&below));
    }

    #[test]
    fn scale_for_a_view_height() {
        assert_eq!(scale_for_height(180.0, 720.0), 0.25);
        assert_eq!(scale_for_height(0.0, 720.0), 1.0);
    }

    #[test]
    fn pixel_snap_lands_on_whole_pixels() {
        assert_eq!(snap_scale(0.26), 0.25);
        assert_eq!(snap_scale(0.45), 0.5);
        assert_eq!(snap_scale(1.0), 1.0);
        assert_eq!(snap_scale(2.4), 2.0);
        assert_eq!(snap_position([10.3, -4.6], 0.5), [10.5, -4.5]);
        assert_eq!(snap_position([10.3, -4.6], 1.0), [10.0, -5.0]);
    }

    #[test]
    fn shake_scales_with_trauma_squared_and_replays() {
        let s = Shake::default();
        assert_eq!(shake_at(0.0, 1.0, &s), ([0.0; 2], 0.0));
        let (a, ang) = shake_at(1.0, 0.37, &s);
        assert_eq!(shake_at(1.0, 0.37, &s).0, a);
        assert!(a.iter().all(|v| v.abs() <= s.max_offset) && ang.abs() <= s.max_angle);
        let (half, _) = shake_at(0.5, 0.37, &s);
        assert!((half[0] - a[0] * 0.25).abs() < 1e-4);
    }

    #[test]
    fn noise_is_smooth_and_bounded() {
        for i in 0..200 {
            let t = i as f32 * 0.05;
            let (a, b) = (noise(1, t), noise(1, t + 0.01));
            assert!(a.abs() <= 1.0 && (a - b).abs() < 0.2);
        }
    }

    #[test]
    fn trauma_decays_to_zero() {
        assert!((decay(0.5, 0.2, 1.5) - 0.2).abs() < 1e-6);
        assert_eq!(decay(0.1, 1.0, 1.5), 0.0);
    }

    #[test]
    fn dials_set_bounds_edges_one_at_a_time() {
        let mut c = Camera2dSettings::default();
        c.set(Look2dDial::BoundsLeft, "-100");
        c.set(Look2dDial::BoundsRight, "100");
        c.set(Look2dDial::BoundsBottom, "0");
        c.set(Look2dDial::BoundsTop, "200");
        let b = c.bounds.expect("bounds");
        assert_eq!((b.min, b.max), ([-100.0, 0.0], [100.0, 200.0]));
        c.set(Look2dDial::Zoom, "2");
        c.set(Look2dDial::Zoom, "wide");
        assert_eq!(c.zoom, Some(2.0));
        c.set(Look2dDial::LockX, "1");
        c.set(Look2dDial::Rotation, "450");
        assert!(c.lock_x && c.rotation == 90.0 && c.is_on());
        assert!(!Camera2dSettings::default().is_on());
    }
}
