//! 2D weather lite: rain and snow motes as a pure function of the clock.
//! The runtime draws them over the view, scaled by the weather blend's
//! precipitation and snow and slanted by the wind. Stateless, so a mote at a
//! time is the same however the frames fell.

use crate::vfx::{pcg, unit};

/// Motes drawn per kind at full intensity.
pub const MAX_MOTES: usize = 160;
/// Screen widths a second of sideways drift per metre a second of wind.
pub const DRIFT_PER_WIND: f32 = 0.04;
/// Where in a fall the mote has landed and starts to splash.
const LAND_AT: f32 = 0.85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoteKind {
    Rain,
    Snow,
    /// Tumbling leaves that come with a stiff wind.
    Leaf,
}

/// One mote for one frame. `x`/`y` are screen fractions from the top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mote {
    pub x: f32,
    pub y: f32,
    pub alpha: f32,
    /// Pixels long (rain) or across (snow).
    pub size: f32,
    /// How far through a splash ring it is, when it is one.
    pub splash: Option<f32>,
}

/// Leaves per wind speed: none under a breeze, all of them in a gale.
pub fn leaf_intensity(wind_speed: f32) -> f32 {
    if !wind_speed.is_finite() {
        return 0.0;
    }
    ((wind_speed - 4.0) / 12.0).clamp(0.0, 1.0)
}

/// How many motes a 0-1 intensity shows.
pub fn count(intensity: f32) -> usize {
    if !intensity.is_finite() || intensity <= 0.0 {
        return 0;
    }
    ((intensity.min(1.0) * MAX_MOTES as f32).ceil() as usize).min(MAX_MOTES)
}

fn hash(i: usize, salt: u32) -> f32 {
    unit(pcg((i as u32).wrapping_mul(0x9E37_79B1) ^ salt))
}

/// Mote `i` of a kind at `time` seconds, with the wind's sideways push.
pub fn mote(kind: MoteKind, i: usize, time: f32, wind_x: f32) -> Mote {
    let drift = if wind_x.is_finite() {
        (wind_x * DRIFT_PER_WIND).clamp(-0.6, 0.6)
    } else {
        0.0
    };
    let (period, size) = match kind {
        MoteKind::Rain => (0.55 + 0.35 * hash(i, 1), 10.0 + 8.0 * hash(i, 2)),
        MoteKind::Snow => (4.0 + 4.0 * hash(i, 1), 2.0 + 2.5 * hash(i, 2)),
        MoteKind::Leaf => (3.0 + 3.0 * hash(i, 1), 5.0 + 3.0 * hash(i, 2)),
    };
    let t = time.max(0.0) + period * hash(i, 3);
    let cycle = (t / period).floor();
    let p = t / period - cycle;
    let x0 = hash(i, 4 ^ (cycle as u32).wrapping_mul(0x85EB_CA6B));
    let seconds = p * period;
    match kind {
        MoteKind::Rain => {
            let land = 0.7 + 0.3 * hash(i, 5 ^ cycle as u32);
            let x = (x0 + drift * seconds).rem_euclid(1.0);
            if p < LAND_AT {
                let y = -0.05 + (land + 0.05) * (p / LAND_AT);
                Mote {
                    x,
                    y,
                    alpha: 0.55,
                    size,
                    splash: None,
                }
            } else {
                let ring = (p - LAND_AT) / (1.0 - LAND_AT);
                Mote {
                    x,
                    y: land,
                    alpha: 0.5 * (1.0 - ring),
                    size,
                    splash: Some(ring),
                }
            }
        }
        MoteKind::Leaf => {
            let sway = (seconds * 2.6 + hash(i, 6) * std::f32::consts::TAU).sin() * 0.03;
            let x = (x0 + drift * seconds * 2.0 + sway).rem_euclid(1.0);
            Mote {
                x,
                y: -0.05 + 1.1 * p,
                alpha: 0.9,
                size,
                splash: None,
            }
        }
        MoteKind::Snow => {
            let sway = (seconds * 1.7 + hash(i, 6) * std::f32::consts::TAU).sin() * 0.012;
            let x = (x0 + drift * seconds + sway).rem_euclid(1.0);
            Mote {
                x,
                y: -0.05 + 1.1 * p,
                alpha: 0.9,
                size,
                splash: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_need_a_stiff_wind() {
        assert_eq!(leaf_intensity(2.0), 0.0);
        assert_eq!(leaf_intensity(10.0), 0.5);
        assert_eq!(leaf_intensity(40.0), 1.0);
        assert_eq!(leaf_intensity(f32::NAN), 0.0);
    }

    #[test]
    fn count_scales_and_clamps() {
        assert_eq!(count(0.0), 0);
        assert_eq!(count(f32::NAN), 0);
        assert_eq!(count(1.0), MAX_MOTES);
        assert_eq!(count(9.0), MAX_MOTES);
        assert!(count(0.5) > 0 && count(0.5) < MAX_MOTES);
    }

    #[test]
    fn motes_are_deterministic_and_on_screen() {
        for kind in [MoteKind::Rain, MoteKind::Snow, MoteKind::Leaf] {
            for i in 0..MAX_MOTES {
                for step in 0..40 {
                    let t = step as f32 * 0.137;
                    let m = mote(kind, i, t, 3.0);
                    assert_eq!(m, mote(kind, i, t, 3.0));
                    assert!((0.0..1.0).contains(&m.x), "{kind:?} {i} x {}", m.x);
                    assert!((-0.06..=1.06).contains(&m.y), "{kind:?} {i} y {}", m.y);
                    assert!((0.0..=1.0).contains(&m.alpha));
                }
            }
        }
    }

    #[test]
    fn only_rain_splashes_and_wind_pushes_sideways() {
        let rain = (0..MAX_MOTES).any(|i| {
            (0..60).any(|s| {
                mote(MoteKind::Rain, i, s as f32 * 0.05, 0.0)
                    .splash
                    .is_some()
            })
        });
        let snow = (0..MAX_MOTES).any(|i| {
            (0..60).any(|s| {
                mote(MoteKind::Snow, i, s as f32 * 0.05, 0.0)
                    .splash
                    .is_some()
            })
        });
        assert!(rain && !snow);
        // Same mote early in a fall: wind moves it across.
        let calm = mote(MoteKind::Snow, 3, 1.0, 0.0);
        let windy = mote(MoteKind::Snow, 3, 1.0, 8.0);
        assert_ne!(calm.x, windy.x);
    }
}
