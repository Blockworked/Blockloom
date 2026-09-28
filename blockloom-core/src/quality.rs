//! Whole-frame quality budgets shared by the editor and player.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Quality {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

impl Quality {
    pub fn lower(self) -> Self {
        match self {
            Self::Ultra => Self::High,
            Self::High => Self::Medium,
            _ => Self::Low,
        }
    }

    pub fn budget(self) -> Budget {
        let (density, distance, particles, decals, shards, shadow, reflection, captures) =
            match self {
                Self::Low => (0.25, 0.5, 25_000, 64, 64, 1024, 128, 1),
                Self::Medium => (0.5, 0.75, 75_000, 128, 128, 2048, 256, 1),
                Self::High => (1.0, 1.0, 200_000, 256, 256, 4096, 512, 2),
                Self::Ultra => (1.0, 1.5, 400_000, 256, 256, 8192, 1024, 4),
            };
        let multiplier = match self {
            Self::Low => 0.25,
            Self::Medium => 0.5,
            Self::High => 1.0,
            Self::Ultra => 2.0,
        };
        Budget {
            draws: [256, 256, 1024, 64, 128, 16].map(|n| (n as f32 * multiplier) as u32),
            triangles: [2_000_000, 1_000_000, 2_000_000, 400_000, 200_000, 400_000]
                .map(|n| (n as f32 * multiplier) as u64),
            density,
            distance,
            particles,
            decals,
            shards,
            shadow,
            reflection,
            captures,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub draws: [u32; 6],
    pub triangles: [u64; 6],
    pub density: f32,
    pub distance: f32,
    pub particles: u32,
    pub decals: usize,
    pub shards: usize,
    pub shadow: usize,
    pub reflection: u32,
    pub captures: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DrawCost {
    pub draws: u32,
    pub triangles: u64,
}

impl DrawCost {
    pub fn exceeds(self, budget: &Budget, system: usize) -> bool {
        self.draws > budget.draws[system] || self.triangles > budget.triangles[system]
    }
}

/// Run-only distance and density throttles. Reductions last until a setting
/// change or rebuild, so unloading content cannot cause a reload loop.
#[derive(Clone, Debug)]
pub struct GeometryController {
    pub factors: [f32; 6],
    over: [u32; 6],
}

impl Default for GeometryController {
    fn default() -> Self {
        Self {
            factors: [1.0; 6],
            over: [0; 6],
        }
    }
}

impl GeometryController {
    pub const LOCAL_SYSTEMS: [usize; 6] = [0, 1, 2, 3, 4, 5];

    pub fn sample(&mut self, settings: &Settings, budget: &Budget, costs: &[DrawCost; 6]) {
        for system in Self::LOCAL_SYSTEMS {
            let cost = costs[system];
            self.over[system] = if settings.auto_drop && cost.exceeds(budget, system) {
                self.over[system].saturating_add(1)
            } else {
                0
            };
            if self.over[system] >= settings.over_budget_frames {
                self.factors[system] = (self.factors[system] - 0.1).max(0.25);
                self.over[system] = 0;
            }
        }
    }

    /// Shared preset feedback handles systems still over budget at their local floor.
    pub fn needs_preset_drop(&self, budget: &Budget, costs: &[DrawCost; 6]) -> bool {
        costs.iter().enumerate().any(|(system, cost)| {
            cost.exceeds(budget, system)
                && (!Self::LOCAL_SYSTEMS.contains(&system) || self.factors[system] <= 0.25)
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Upscaler {
    #[default]
    Spatial,
    Taa,
    Dlss,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DlssMode {
    Dlaa,
    #[default]
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub preset: Quality,
    pub resolution_scale: f32,
    pub dynamic_resolution: bool,
    pub min_scale: f32,
    pub target_ms: f32,
    pub auto_drop: bool,
    pub over_budget_frames: u32,
    pub upscaler: Upscaler,
    pub dlss_mode: DlssMode,
    pub sharpness: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preset: Quality::High,
            resolution_scale: 1.0,
            dynamic_resolution: false,
            min_scale: 0.5,
            target_ms: 16.667,
            auto_drop: false,
            over_budget_frames: 120,
            upscaler: Upscaler::Spatial,
            dlss_mode: DlssMode::Quality,
            sharpness: 0.0,
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        fn finite(value: f32, fallback: f32, lo: f32, hi: f32) -> f32 {
            if value.is_finite() {
                value.clamp(lo, hi)
            } else {
                fallback
            }
        }
        self.resolution_scale = finite(self.resolution_scale, 1.0, 0.25, 1.0);
        self.min_scale = finite(self.min_scale, 0.5, 0.25, 1.0).min(self.resolution_scale);
        self.target_ms = finite(self.target_ms, 16.667, 4.0, 100.0);
        self.sharpness = finite(self.sharpness, 0.0, 0.0, 1.0);
        self.over_budget_frames = self.over_budget_frames.clamp(15, 3600);
    }
}

/// Hysteretic feedback: reduce pixel rate first, then density and distance.
#[derive(Clone, Debug)]
pub struct Controller {
    pub quality: Quality,
    pub scale: f32,
    pub frame_ms: f32,
    pub drops: u32,
    over: u32,
    under: u32,
    frames: u32,
}

impl Controller {
    pub fn new(settings: &Settings) -> Self {
        Self {
            quality: settings.preset,
            scale: settings.resolution_scale,
            frame_ms: 0.0,
            drops: 0,
            over: 0,
            under: 0,
            frames: 0,
        }
    }

    pub fn sample(&mut self, settings: &Settings, ms: f32) -> bool {
        self.sample_with_cost(settings, ms, false)
    }

    pub fn sample_with_cost(&mut self, settings: &Settings, ms: f32, geometry_over: bool) -> bool {
        if !ms.is_finite() || ms <= 0.0 {
            return false;
        }
        self.frame_ms = if self.frame_ms == 0.0 {
            ms
        } else {
            self.frame_ms * 0.9 + ms * 0.1
        };
        self.frames = self.frames.saturating_add(1);
        let over = self.frame_ms > settings.target_ms * 1.1 || geometry_over;
        self.over = if over { self.over.saturating_add(1) } else { 0 };
        self.under = if self.frame_ms < settings.target_ms * 0.8 {
            self.under.saturating_add(1)
        } else {
            0
        };
        if settings.dynamic_resolution && self.frames >= 30 && !geometry_over {
            if self.over >= 30 && self.scale > settings.min_scale {
                self.scale = (self.scale - 0.05).max(settings.min_scale);
                self.frames = 0;
                self.over = 0;
            } else if self.under >= 120 && self.scale < settings.resolution_scale {
                self.scale = (self.scale + 0.05).min(settings.resolution_scale);
                self.frames = 0;
                self.under = 0;
            }
        }
        if settings.auto_drop
            && self.over >= settings.over_budget_frames
            && (geometry_over || !settings.dynamic_resolution || self.scale <= settings.min_scale)
            && self.quality != Quality::Low
        {
            self.quality = self.quality.lower();
            self.over = 0;
            self.drops += 1;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_geometry_pressure_is_sustained_and_isolated() {
        let settings = Settings {
            auto_drop: true,
            ..Default::default()
        };
        let budget = settings.preset.budget();
        let mut costs = [DrawCost::default(); 6];
        costs[1].triangles = budget.triangles[1] + 1;
        let mut local = GeometryController::default();
        for _ in 0..settings.over_budget_frames - 1 {
            local.sample(&settings, &budget, &costs);
        }
        assert_eq!(local.factors, [1.0; 6]);
        assert!(!local.needs_preset_drop(&budget, &costs));
        local.sample(&settings, &budget, &costs);
        assert_eq!(local.factors, [1.0, 0.9, 1.0, 1.0, 1.0, 1.0]);
        for _ in 0..settings.over_budget_frames * 20 {
            local.sample(&settings, &budget, &costs);
        }
        assert_eq!(local.factors, [1.0, 0.25, 1.0, 1.0, 1.0, 1.0]);
        assert!(local.needs_preset_drop(&budget, &costs));
    }

    #[test]
    fn brief_pressure_and_disabled_feedback_keep_local_detail() {
        let mut settings = Settings {
            auto_drop: true,
            ..Default::default()
        };
        let budget = settings.preset.budget();
        let mut costs = [DrawCost::default(); 6];
        costs[0].draws = budget.draws[0] + 1;
        let mut local = GeometryController::default();
        for _ in 0..settings.over_budget_frames - 1 {
            local.sample(&settings, &budget, &costs);
        }
        local.sample(&settings, &budget, &[DrawCost::default(); 6]);
        local.sample(&settings, &budget, &costs);
        assert_eq!(local.factors, [1.0; 6]);
        settings.auto_drop = false;
        for _ in 0..settings.over_budget_frames * 10 {
            local.sample(&settings, &budget, &costs);
        }
        assert_eq!(local.factors, [1.0; 6]);
    }

    #[test]
    fn unloading_does_not_restore_local_density_and_restart_loading() {
        let settings = Settings {
            auto_drop: true,
            ..Default::default()
        };
        let budget = settings.preset.budget();
        let mut costs = [DrawCost::default(); 6];
        costs[0].draws = budget.draws[0] + 1;
        let mut local = GeometryController::default();
        for _ in 0..settings.over_budget_frames {
            local.sample(&settings, &budget, &costs);
        }
        for _ in 0..1000 {
            local.sample(&settings, &budget, &[DrawCost::default(); 6]);
        }
        assert_eq!(local.factors, [0.9, 1.0, 1.0, 1.0, 1.0, 1.0]);
        costs[2].draws = budget.draws[2] + 1;
        assert!(!local.needs_preset_drop(&budget, &costs));
    }

    #[test]
    fn local_pressure_throttles_only_its_system_before_the_shared_preset() {
        let settings = Settings {
            auto_drop: true,
            ..Default::default()
        };
        let budget = settings.preset.budget();
        for system in [2, 3, 4, 5] {
            let mut local = GeometryController::default();
            let mut costs = [DrawCost::default(); 6];
            costs[system].triangles = budget.triangles[system] + 1;
            for _ in 0..settings.over_budget_frames - 1 {
                local.sample(&settings, &budget, &costs);
            }
            assert_eq!(local.factors, [1.0; 6]);
            local.sample(&settings, &budget, &costs);
            let mut expected = [1.0; 6];
            expected[system] = 0.9;
            assert_eq!(local.factors, expected);
            assert!(!local.needs_preset_drop(&budget, &costs));
            for _ in 0..settings.over_budget_frames * 20 {
                local.sample(&settings, &budget, &costs);
            }
            assert_eq!(local.factors[system], 0.25);
            assert!(local.needs_preset_drop(&budget, &costs));
            for _ in 0..1000 {
                local.sample(&settings, &budget, &[DrawCost::default(); 6]);
            }
            assert_eq!(local.factors[system], 0.25);
        }
    }

    #[test]
    fn old_settings_keep_native_resolution() {
        assert_eq!(
            serde_json::from_str::<Settings>("{}").unwrap(),
            Settings::default()
        );
    }
    #[test]
    fn sanitize_nonfinite_and_inverted_limits() {
        let mut s = Settings {
            resolution_scale: 0.3,
            min_scale: f32::INFINITY,
            target_ms: f32::NAN,
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.min_scale, 0.3);
        assert_eq!(s.target_ms, 16.667);
    }
    #[test]
    fn feedback_has_hysteresis_and_a_floor() {
        let s = Settings {
            dynamic_resolution: true,
            auto_drop: true,
            ..Default::default()
        };
        let mut c = Controller::new(&s);
        for _ in 0..29 {
            assert!(!c.sample(&s, 30.0));
        }
        assert_eq!(c.scale, 1.0);
        for _ in 0..1500 {
            c.sample(&s, 30.0);
        }
        assert_eq!(c.scale, s.min_scale);
        assert_eq!(c.quality, Quality::Low);
        assert_eq!(c.drops, 2);
        for _ in 0..3000 {
            c.sample(&s, 5.0);
        }
        assert_eq!(c.scale, 1.0);
        assert_eq!(c.quality, Quality::Low);
    }
    #[test]
    fn geometry_pressure_reduces_density_without_changing_pixel_rate() {
        let s = Settings {
            dynamic_resolution: true,
            auto_drop: true,
            ..Default::default()
        };
        let mut c = Controller::new(&s);
        for _ in 0..s.over_budget_frames - 1 {
            assert!(!c.sample_with_cost(&s, 5.0, true));
        }
        assert!(c.sample_with_cost(&s, 5.0, true));
        assert_eq!(c.quality, Quality::Medium);
        assert_eq!(c.scale, 1.0);
    }
    #[test]
    fn disabled_feedback_and_bad_samples_do_not_change_quality() {
        let s = Settings::default();
        let mut c = Controller::new(&s);
        for _ in 0..1000 {
            c.sample(&s, 100.0);
        }
        c.sample(&s, f32::NAN);
        assert_eq!((c.scale, c.quality, c.drops), (1.0, Quality::High, 0));
        assert!(c.frame_ms.is_finite());
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Setting {
    #[default]
    Quality,
    ResolutionScale,
    Upscaler,
    DlssMode,
}

impl Settings {
    pub fn set(&mut self, setting: Setting, value: &str) -> Result<(), String> {
        fn named<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, String> {
            serde_json::from_value(serde_json::Value::String(value.to_owned()))
                .map_err(|_| format!("unknown rendering setting value: {value}"))
        }
        match setting {
            Setting::Quality => self.preset = named(value)?,
            Setting::ResolutionScale => {
                let n: f32 = value
                    .parse()
                    .map_err(|_| "resolution scale must be a number")?;
                if !n.is_finite() {
                    return Err("resolution scale must be finite".into());
                }
                self.resolution_scale = n;
            }
            Setting::Upscaler => self.upscaler = named(value)?,
            Setting::DlssMode => self.dlss_mode = named(value)?,
        }
        self.normalize();
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub frame_ms: f64,
    pub draw_calls: u32,
    pub quality: Quality,
    pub dlss_available: bool,
}
