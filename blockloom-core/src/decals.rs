//! Transient surface marks. Only the blocks that create them are saved.

use serde::{Deserialize, Serialize};

pub const CAPACITY: usize = 256;
pub const ATLAS_CELL: u32 = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DecalPreset {
    #[default]
    Blood,
    Footprint,
    FreshScorch,
}

impl DecalPreset {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Blood" => Some(Self::Blood),
            "Footprint" => Some(Self::Footprint),
            "FreshScorch" => Some(Self::FreshScorch),
            _ => None,
        }
    }

    pub fn cell(self) -> u32 {
        match self {
            Self::Blood => 0,
            Self::Footprint => 1,
            Self::FreshScorch => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spawn {
    pub preset: DecalPreset,
    pub at: [f32; 3],
    pub normal: [f32; 3],
    pub size: f32,
    pub lifetime: f32,
    pub fade: f32,
}

impl Spawn {
    pub fn normalized(mut self) -> Option<Self> {
        if !self
            .at
            .iter()
            .chain(&self.normal)
            .chain([&self.size, &self.lifetime, &self.fade])
            .all(|v| v.is_finite())
            || self.size <= 0.0
            || self.lifetime <= 0.0
        {
            return None;
        }
        let length = self
            .normal
            .iter()
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        if length < 1e-6 {
            return None;
        }
        self.normal = self.normal.map(|v| (v as f64 / length) as f32);
        self.size = self.size.clamp(0.001, 1000.0);
        self.lifetime = self.lifetime.min(3600.0);
        self.fade = self.fade.clamp(0.0, self.lifetime);
        Some(self)
    }
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub id: u64,
    pub spawn: Spawn,
    pub age: f32,
    last_used: f64,
    forced_fade: Option<(f32, f32, f32)>,
}

impl Mark {
    pub fn opacity(&self) -> f32 {
        let natural = if self.spawn.fade > 0.0 {
            ((self.spawn.lifetime - self.age) / self.spawn.fade).clamp(0.0, 1.0)
        } else {
            f32::from(self.age < self.spawn.lifetime)
        };
        self.forced_fade
            .map_or(natural, |(start, duration, opacity)| {
                natural.min(opacity * (1.0 - (self.age - start) / duration).clamp(0.0, 1.0))
            })
    }
}

#[derive(Default)]
pub struct Pool {
    pub marks: Vec<Mark>,
    next: u64,
    clock: f64,
    pub stolen: u64,
}

impl Pool {
    pub fn spawn(&mut self, spawn: Spawn) {
        let Some(spawn) = spawn.normalized() else {
            return;
        };
        if self.marks.len() == CAPACITY {
            let oldest = self
                .marks
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.last_used.total_cmp(&b.last_used).then(a.id.cmp(&b.id)))
                .map(|(i, _)| i)
                .unwrap();
            self.marks.remove(oldest);
            self.stolen += 1;
        }
        self.next += 1;
        self.marks.push(Mark {
            id: self.next,
            spawn,
            age: 0.0,
            last_used: self.clock,
            forced_fade: None,
        });
    }

    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt < 0.0 {
            return;
        }
        self.clock += dt as f64;
        for mark in &mut self.marks {
            mark.age += dt;
        }
        self.marks.retain(|m| m.opacity() > 0.0);
    }

    pub fn touch(&mut self, id: u64) {
        if let Some(mark) = self.marks.iter_mut().find(|m| m.id == id) {
            mark.last_used = self.clock;
        }
    }

    pub fn fade_in_radius(&mut self, at: [f32; 3], radius: f32, seconds: f32) {
        if !at.iter().chain([&radius, &seconds]).all(|v| v.is_finite()) || radius < 0.0 {
            return;
        }
        for mark in &mut self.marks {
            let distance = mark
                .spawn
                .at
                .iter()
                .zip(at)
                .map(|(a, b)| (*a as f64 - b as f64).powi(2))
                .sum::<f64>();
            if distance <= (radius as f64).powi(2) {
                let duration = seconds.max(0.0);
                // Repeated requests can shorten a fade, never postpone it.
                let remaining = mark.forced_fade.map_or(duration, |(start, time, _)| {
                    (start + time - mark.age).max(0.0)
                });
                let opacity = mark.opacity();
                mark.forced_fade = Some((
                    mark.age,
                    duration.min(remaining).max(f32::MIN_POSITIVE),
                    if duration == 0.0 { 0.0 } else { opacity },
                ));
            }
        }
        self.marks.retain(|m| m.opacity() > 0.0);
    }
}

/// Four matching atlas pages: sRGB albedo, linear normal, roughness and emission.
pub fn atlas_pages() -> [Vec<u8>; 4] {
    let side = ATLAS_CELL * 2;
    let mut pages = std::array::from_fn(|_| vec![0; (side * side * 4) as usize]);
    for preset in [
        DecalPreset::Blood,
        DecalPreset::Footprint,
        DecalPreset::FreshScorch,
    ] {
        for y in 0..ATLAS_CELL {
            for x in 0..ATLAS_CELL {
                let u = (x as f32 + 0.5) / ATLAS_CELL as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / ATLAS_CELL as f32 * 2.0 - 1.0;
                let r = u.hypot(v);
                let noise =
                    ((x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)) % 1024) as f32 / 1024.0;
                let (mask, color, roughness, emission) = match preset {
                    DecalPreset::Blood => {
                        let edge = 0.62 + 0.09 * (v.atan2(u) * 7.0).sin() + noise * 0.1;
                        (
                            ((edge - r) * 24.0).clamp(0.0, 1.0),
                            [100, 5, 9],
                            55,
                            [0, 0, 0],
                        )
                    }
                    DecalPreset::Footprint => {
                        let sole = (u / 0.38).powi(2) + ((v + 0.27) / 0.5).powi(2);
                        let heel = (u / 0.3).powi(2) + ((v - 0.48) / 0.23).powi(2);
                        let tread = if (v * 24.0).sin() > 0.65 { 0.25 } else { 1.0 };
                        (
                            ((1.0 - sole.min(heel)) * 12.0).clamp(0.0, 1.0) * tread,
                            [55, 49, 40],
                            220,
                            [0, 0, 0],
                        )
                    }
                    DecalPreset::FreshScorch => {
                        let mask = ((0.83 - r - noise * 0.1) * 8.0).clamp(0.0, 1.0);
                        let ember =
                            ((1.0 - (r - 0.52).abs() * 15.0).max(0.0) * noise * 255.0) as u8;
                        (mask, [24, 20, 18], 240, [ember, ember / 8, 0])
                    }
                };
                let alpha = (mask * 255.0) as u8;
                let px = x + (preset.cell() % 2) * ATLAS_CELL;
                let py = y + (preset.cell() / 2) * ATLAS_CELL;
                let i = ((py * side + px) * 4) as usize;
                pages[0][i..i + 4].copy_from_slice(&[color[0], color[1], color[2], alpha]);
                pages[1][i..i + 4].copy_from_slice(&[
                    (128.0 + u * mask * 18.0) as u8,
                    (128.0 - v * mask * 18.0) as u8,
                    254,
                    alpha,
                ]);
                pages[2][i..i + 4].copy_from_slice(&[0, roughness, 0, alpha]);
                pages[3][i..i + 4].copy_from_slice(&[emission[0], emission[1], emission[2], alpha]);
            }
        }
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mark(x: f32) -> Spawn {
        Spawn {
            preset: DecalPreset::Blood,
            at: [x, 0.0, 0.0],
            normal: [0.0, 2.0, 0.0],
            size: 1.0,
            lifetime: 10.0,
            fade: 2.0,
        }
    }
    #[test]
    fn oldest_is_stolen_and_marks_expire() {
        let mut pool = Pool::default();
        for i in 0..=CAPACITY {
            pool.spawn(mark(i as f32));
        }
        assert_eq!(pool.marks.len(), CAPACITY);
        assert_eq!(pool.marks[0].spawn.at[0], 1.0);
        assert_eq!(pool.stolen, 1);
        pool.step(9.0);
        assert_eq!(pool.marks[0].opacity(), 0.5);
        pool.step(1.0);
        assert!(pool.marks.is_empty());
    }

    #[test]
    fn visible_marks_are_kept_ahead_of_unused_marks() {
        let mut pool = Pool::default();
        for i in 0..CAPACITY {
            pool.spawn(mark(i as f32));
        }
        let first = pool.marks[0].id;
        pool.step(1.0);
        pool.touch(first);
        pool.spawn(mark(999.0));
        assert_eq!(pool.marks[0].id, first);
        assert!(!pool.marks.iter().any(|m| m.spawn.at[0] == 1.0));
    }
    #[test]
    fn radius_fades_only_nearby_and_never_restarts() {
        let mut pool = Pool::default();
        pool.spawn(mark(0.0));
        pool.spawn(mark(4.0));
        pool.fade_in_radius([0.0; 3], 1.0, 2.0);
        pool.step(1.0);
        assert_eq!(pool.marks[0].opacity(), 0.5);
        pool.fade_in_radius([0.0; 3], 1.0, 10.0);
        pool.step(1.0);
        assert_eq!(pool.marks.len(), 1);
        assert_eq!(pool.marks[0].spawn.at[0], 4.0);
        pool.fade_in_radius([4.0, 0.0, 0.0], 0.0, 0.0);
        assert!(pool.marks.is_empty());
    }
    #[test]
    fn invalid_marks_are_rejected_and_normal_is_normalized() {
        assert_eq!(mark(0.0).normalized().unwrap().normal, [0.0, 1.0, 0.0]);
        assert!(mark(f32::NAN).normalized().is_none());
        let mut bad = mark(0.0);
        bad.normal = [0.0; 3];
        assert!(bad.normalized().is_none());
    }
    #[test]
    fn atlas_has_matching_transparent_gutters() {
        let pages = atlas_pages();
        for page in &pages {
            assert_eq!(page.len(), (ATLAS_CELL * ATLAS_CELL * 16) as usize);
        }
        for cell in 0..3 {
            let corner =
                ((cell / 2 * ATLAS_CELL * ATLAS_CELL * 2 + cell % 2 * ATLAS_CELL) * 4) as usize;
            for page in &pages {
                assert_eq!(page[corner + 3], 0);
            }
        }
        assert!(pages[0].chunks_exact(4).any(|p| p[3] == 255));
    }
}
