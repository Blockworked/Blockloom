//! Lightning: a strike is a flash of light where it lands, a pulse of the
//! sky's ambient light, and thunder that arrives late by the distance. A
//! storm is a director that strikes at random inside a region at a rate.
//!
//! The director is here rather than in the runtime so its schedule is plain
//! arithmetic on fixed ticks: one seed gives one storm, however the frames
//! fall.

use serde::{Deserialize, Serialize};

/// Metres per second sound travels, for the thunder's delay.
pub const SPEED_OF_SOUND: f32 = 343.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lightning {
    /// Strike at random while the world runs.
    pub storm: bool,
    /// Strikes a minute, on average.
    pub rate: f32,
    /// Corners of the box strikes land in, world units. In 2D only x and y
    /// count.
    pub region_min: [f32; 3],
    pub region_max: [f32; 3],
    /// Lumens of the flash at its peak.
    pub intensity: f32,
    pub color: String,
    /// Seconds the flash takes to die away.
    pub decay: f32,
    /// Metres above the strike the flash hangs.
    pub flash_height: f32,
    /// Metres the flash reaches.
    pub range: f32,
    /// How much the sky's ambient light jumps at the peak, as a multiple of
    /// itself.
    pub sky_pulse: f32,
    pub thunder: bool,
    /// 0-100, on the effects bus.
    pub thunder_volume: f32,
    /// A sound asset to use instead of the built-in rumble.
    pub thunder_sound: String,
    /// Seeds the storm's dice, so one storm replays the same.
    pub seed: u32,
}

impl Default for Lightning {
    fn default() -> Self {
        Self {
            storm: false,
            rate: 6.0,
            region_min: [-200.0, 0.0, -200.0],
            region_max: [200.0, 0.0, 200.0],
            intensity: 2.0e7,
            color: "#D8E4FF".to_string(),
            decay: 0.35,
            flash_height: 60.0,
            range: 2000.0,
            sky_pulse: 6.0,
            thunder: true,
            thunder_volume: 80.0,
            thunder_sound: String::new(),
            seed: 1,
        }
    }
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

impl Lightning {
    pub fn normalize(&mut self) {
        let d = Lightning::default();
        self.rate = finite(self.rate, d.rate).clamp(0.0, 600.0);
        for i in 0..3 {
            let (a, b) = (
                finite(self.region_min[i], d.region_min[i]),
                finite(self.region_max[i], d.region_max[i]),
            );
            self.region_min[i] = a.min(b).clamp(-1.0e6, 1.0e6);
            self.region_max[i] = a.max(b).clamp(-1.0e6, 1.0e6);
        }
        self.intensity = finite(self.intensity, d.intensity).clamp(0.0, 1.0e12);
        self.decay = finite(self.decay, d.decay).clamp(0.01, 10.0);
        self.flash_height = finite(self.flash_height, d.flash_height).clamp(0.0, 10_000.0);
        self.range = finite(self.range, d.range).clamp(1.0, 1.0e6);
        self.sky_pulse = finite(self.sky_pulse, d.sky_pulse).clamp(0.0, 1000.0);
        self.thunder_volume = finite(self.thunder_volume, d.thunder_volume).clamp(0.0, 100.0);
        self.thunder_sound = self.thunder_sound.trim().replace('\\', "/");
    }

    /// 1 at the strike, falling to 0 over `decay` seconds with a couple of
    /// re-strokes on the way, the way a real flash flickers.
    pub fn flash(&self, since: f32) -> f32 {
        if !(0.0..self.decay * 1.5).contains(&since) {
            return 0.0;
        }
        let t = since / self.decay;
        let fade = (-4.0 * t).exp();
        // Return strokes at a quarter and half the decay.
        let stroke = |at: f32, width: f32| (1.0 - ((t - at) / width).abs()).max(0.0);
        (fade + 0.6 * stroke(0.25, 0.08) + 0.35 * stroke(0.5, 0.06)).min(1.0)
    }

    /// Seconds until the thunder of a strike `distance` away is heard.
    pub fn thunder_delay(distance: f32) -> f32 {
        distance.max(0.0) / SPEED_OF_SOUND
    }
}

/// The storm's schedule: when the next strike falls and where.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StormDirector {
    /// Strikes so far; each one's dice are rolled from it and the seed.
    count: u32,
    /// Seconds until the next strike, once one is scheduled.
    wait: Option<f32>,
}

impl StormDirector {
    /// Advances by `dt` seconds of a storm striking `rate` times a minute,
    /// and gives back where each strike that fell lands.
    pub fn step(&mut self, lightning: &Lightning, rate: f32, dt: f32) -> Vec<[f32; 3]> {
        let mut strikes = Vec::new();
        if rate <= 0.0 || !dt.is_finite() || dt <= 0.0 {
            self.wait = None;
            return strikes;
        }
        let mut left = dt;
        loop {
            let wait = *self
                .wait
                .get_or_insert_with(|| interval(lightning.seed, self.count, rate));
            if wait > left {
                self.wait = Some(wait - left);
                return strikes;
            }
            left -= wait;
            strikes.push(place(lightning, self.count));
            self.count += 1;
            self.wait = None;
            // Never more than a handful in one step, whatever the rate.
            if strikes.len() >= 8 {
                return strikes;
            }
        }
    }
}

/// Seconds to the strike after strike `n`: exponential, so strikes bunch
/// and lull like a real storm's.
fn interval(seed: u32, n: u32, rate: f32) -> f32 {
    let u = unit(hash(seed, n, 0)).max(1e-6);
    -u.ln() * 60.0 / rate
}

fn place(lightning: &Lightning, n: u32) -> [f32; 3] {
    std::array::from_fn(|axis| {
        let (lo, hi) = (lightning.region_min[axis], lightning.region_max[axis]);
        lo + (hi - lo) * unit(hash(lightning.seed, n, axis as u32 + 1))
    })
}

/// The built-in thunder: a crack, then a rolling rumble, as a mono 16-bit
/// WAV. Brown noise shaped by a few random swells, so no asset is needed.
pub fn thunder_wav(seed: u32) -> Vec<u8> {
    const RATE: u32 = 22_050;
    const SECONDS: f32 = 5.0;
    let count = (RATE as f32 * SECONDS) as usize;
    let mut samples = Vec::with_capacity(count);
    let (mut brown, mut crack_state) = (0.0f32, 0.0f32);
    // Swells: when each roll peaks and how loud it is.
    let swells: Vec<(f32, f32)> = (0..5)
        .map(|i| {
            let at = 0.2 + 3.2 * unit(hash(seed, i, 11));
            (at, 0.4 + 0.6 * unit(hash(seed, i, 12)))
        })
        .collect();
    for i in 0..count {
        let t = i as f32 / RATE as f32;
        let white = unit(hash(seed, i as u32, 13)) * 2.0 - 1.0;
        // Brown noise: integrated white, leaking back to zero.
        brown = (brown + white * 0.02) * 0.998;
        crack_state = crack_state * 0.6 + white * 0.4;
        let crack = crack_state * (-t * 18.0).exp();
        let roll: f32 = swells
            .iter()
            .map(|(at, gain)| gain * (-((t - at) / 0.45).powi(2)).exp())
            .sum::<f32>()
            + 0.6 * (-t * 0.9).exp();
        let fade = (1.0 - t / SECONDS).clamp(0.0, 1.0);
        samples.push((crack * 0.5 + brown * 6.0 * roll) * fade);
    }
    let peak = samples.iter().fold(1e-6f32, |m, s| m.max(s.abs()));
    let mut wav = Vec::with_capacity(44 + count * 2);
    let data = (count * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample / peak * 0.9 * i16::MAX as f32) as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}

fn hash(seed: u32, n: u32, lane: u32) -> u32 {
    // PCG, as `blockloom::hash` does it on the GPU.
    let v = seed
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(n.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(lane.wrapping_mul(0xC2B2_AE35));
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

fn unit(x: u32) -> f32 {
    (x >> 8) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_peaks_at_once_and_dies_away() {
        let l = Lightning::default();
        assert_eq!(l.flash(0.0), 1.0);
        assert!(l.flash(l.decay) < 0.2);
        assert_eq!(l.flash(l.decay * 2.0), 0.0);
        assert_eq!(l.flash(-0.1), 0.0);
        // A return stroke brightens it again on the way down.
        assert!(l.flash(l.decay * 0.25) > l.flash(l.decay * 0.18));
    }

    #[test]
    fn thunder_is_late_by_the_distance() {
        assert!((Lightning::thunder_delay(343.0) - 1.0).abs() < 1e-6);
        assert_eq!(Lightning::thunder_delay(-5.0), 0.0);
    }

    #[test]
    fn a_storm_strikes_at_about_its_rate_inside_its_region() {
        let l = Lightning::default();
        let mut director = StormDirector::default();
        let dt = 1.0 / 60.0;
        let mut strikes = Vec::new();
        for _ in 0..(60 * 600) {
            strikes.extend(director.step(&l, 6.0, dt));
        }
        // Ten minutes at six a minute.
        assert!((40..90).contains(&strikes.len()), "{}", strikes.len());
        for s in &strikes {
            for (axis, v) in s.iter().enumerate() {
                assert!(*v >= l.region_min[axis] && *v <= l.region_max[axis]);
            }
        }
    }

    #[test]
    fn one_seed_is_one_storm_however_the_steps_fall() {
        let l = Lightning::default();
        let run = |dt: f32, steps: usize| {
            let mut director = StormDirector::default();
            (0..steps)
                .flat_map(|_| director.step(&l, 30.0, dt))
                .collect::<Vec<_>>()
        };
        let fine = run(1.0 / 120.0, 120 * 60);
        let coarse = run(1.0 / 30.0, 30 * 60);
        assert!(!fine.is_empty());
        let n = fine.len().min(coarse.len());
        assert!(n + 1 >= fine.len().max(coarse.len()));
        assert_eq!(fine[..n], coarse[..n]);
        let other = Lightning {
            seed: 2,
            ..Lightning::default()
        };
        let mut director = StormDirector::default();
        let reseeded: Vec<_> = (0..120 * 60)
            .flat_map(|_| director.step(&other, 30.0, 1.0 / 120.0))
            .collect();
        assert_ne!(reseeded[..3], fine[..3]);
    }

    #[test]
    fn the_built_in_thunder_is_a_wav_that_starts_loud_and_fades() {
        let wav = thunder_wav(3);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let samples: Vec<i16> = wav[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        let loudness = |range: std::ops::Range<usize>| {
            samples[range.clone()]
                .iter()
                .map(|s| (*s as f32).abs())
                .sum::<f32>()
                / range.len() as f32
        };
        let second = 22_050;
        assert!(loudness(0..second) > loudness(4 * second..5 * second));
        assert_eq!(thunder_wav(3), wav);
    }

    #[test]
    fn no_rate_no_strikes() {
        let mut director = StormDirector::default();
        assert!(director.step(&Lightning::default(), 0.0, 10.0).is_empty());
    }

    #[test]
    fn normalize_orders_the_region() {
        let mut l = Lightning {
            region_min: [10.0, 0.0, 5.0],
            region_max: [-10.0, 0.0, -5.0],
            ..Lightning::default()
        };
        l.normalize();
        assert_eq!(l.region_min, [-10.0, 0.0, -5.0]);
        assert_eq!(l.region_max, [10.0, 0.0, 5.0]);
        let old: Lightning = serde_json::from_str("{}").unwrap();
        assert_eq!(old, Lightning::default());
    }
}
