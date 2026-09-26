//! Persisted volumetric cloud settings. Distances are metres except tiling.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudQuality {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}
impl CloudQuality {
    pub fn steps(self) -> (u32, u32) {
        match self {
            Self::Low => (16, 3),
            Self::Medium => (32, 5),
            Self::High => (48, 6),
            Self::Ultra => (64, 8),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clouds {
    pub enabled: bool,
    pub quality: CloudQuality,
    pub coverage: f32,
    pub density: f32,
    pub cloud_type: f32,
    pub bottom: f32,
    pub top: f32,
    pub tiling_km: f32,
    pub shear: [f32; 2],
    pub toe: f32,
    pub shoulder: f32,
    pub detail_scale: f32,
    pub erosion: f32,
    pub detail_speed: f32,
    pub anvil: f32,
    pub billow: f32,
    pub feather: f32,
    pub forward: f32,
    pub backward: f32,
    pub back_blend: f32,
    pub powder: f32,
    pub lightbleed: f32,
    pub ambient: f32,
    pub bottom_occlusion: f32,
    pub shadows: bool,
    pub sun_shadows: bool,
    pub moon_shadows: bool,
    pub shadow_strength: f32,
    pub shadow_range: f32,
    pub threshold: f32,
}
impl Default for Clouds {
    fn default() -> Self {
        Self {
            enabled: false,
            quality: CloudQuality::High,
            coverage: 0.5,
            density: 0.8,
            cloud_type: 0.7,
            bottom: 1500.0,
            top: 3500.0,
            tiling_km: 12.0,
            shear: [0.0; 2],
            toe: 0.2,
            shoulder: 0.8,
            detail_scale: 6.0,
            erosion: 0.35,
            detail_speed: 1.0,
            anvil: 0.3,
            billow: 0.15,
            feather: 0.2,
            forward: 0.8,
            backward: -0.2,
            back_blend: 0.2,
            powder: 1.0,
            lightbleed: 0.3,
            ambient: 1.0,
            bottom_occlusion: 0.65,
            shadows: true,
            sun_shadows: true,
            moon_shadows: true,
            shadow_strength: 0.7,
            shadow_range: 20000.0,
            threshold: 0.01,
        }
    }
}
impl Clouds {
    pub fn normalize(&mut self) {
        fn clamp(v: &mut f32, default: f32, lo: f32, hi: f32) {
            *v = if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default.clamp(lo, hi)
            };
        }
        let d = Self::default();
        macro_rules! c {
            ($f:ident,$lo:expr,$hi:expr) => {
                clamp(&mut self.$f, d.$f, $lo, $hi);
            };
        }
        c!(coverage, 0.0, 1.0);
        c!(density, 0.0, 10.0);
        c!(cloud_type, 0.0, 1.0);
        c!(bottom, -10000.0, 100000.0);
        c!(top, self.bottom + 10.0, 200000.0);
        c!(tiling_km, 0.1, 1000.0);
        c!(toe, 0.0, 0.95);
        c!(shoulder, self.toe + 0.01, 1.0);
        c!(detail_scale, 1.0, 64.0);
        c!(erosion, 0.0, 1.0);
        c!(detail_speed, 0.0, 20.0);
        c!(anvil, 0.0, 1.0);
        c!(billow, 0.01, 0.5);
        c!(feather, 0.01, 0.5);
        c!(forward, 0.0, 0.95);
        c!(backward, -0.95, 0.0);
        c!(back_blend, 0.0, 1.0);
        c!(powder, 0.0, 2.0);
        c!(lightbleed, 0.0, 1.0);
        c!(ambient, 0.0, 10.0);
        c!(bottom_occlusion, 0.0, 1.0);
        c!(shadow_strength, 0.0, 1.0);
        c!(shadow_range, 100.0, 100000.0);
        c!(threshold, 0.001, 0.2);
        for v in &mut self.shear {
            clamp(v, 0.0, -10000.0, 10000.0);
        }
    }
}
/// Which dial `set clouds _ to` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CloudProperty {
    Coverage,
    Density,
    Type,
}
impl CloudProperty {
    pub fn name(self) -> &'static str {
        match self {
            Self::Coverage => "Coverage",
            Self::Density => "Density",
            Self::Type => "Type",
        }
    }
    /// Case-insensitive, so a script's `"coverage"` works too.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "coverage" | "cover" => Some(Self::Coverage),
            "density" => Some(Self::Density),
            "type" | "cloud_type" => Some(Self::Type),
            _ => None,
        }
    }
}
/// What `set clouds` set this run, laid over the blended clouds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CloudOverrides {
    pub coverage: Option<f32>,
    pub density: Option<f32>,
    pub cloud_type: Option<f32>,
}
impl CloudOverrides {
    /// Clamped as the project's own would be. A value that isn't finite is
    /// ignored.
    pub fn set(&mut self, property: CloudProperty, value: f32) {
        if !value.is_finite() {
            return;
        }
        match property {
            CloudProperty::Coverage => self.coverage = Some(value.clamp(0.0, 1.0)),
            CloudProperty::Density => self.density = Some(value.clamp(0.0, 10.0)),
            CloudProperty::Type => self.cloud_type = Some(value.clamp(0.0, 1.0)),
        }
    }
    pub fn apply(&self, clouds: &mut Clouds) {
        if let Some(v) = self.coverage {
            clouds.coverage = v;
        }
        if let Some(v) = self.density {
            clouds.density = v;
        }
        if let Some(v) = self.cloud_type {
            clouds.cloud_type = v;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_documents_and_bad_inputs() {
        assert_eq!(
            serde_json::from_str::<Clouds>("{}").unwrap(),
            Clouds::default()
        );
        let mut c = Clouds {
            bottom: 4000.0,
            top: 1000.0,
            coverage: f32::NAN,
            shoulder: -3.0,
            ..Default::default()
        };
        c.normalize();
        assert_eq!(c.top, 4010.0);
        assert_eq!(c.coverage, 0.5);
        c.top = f32::NAN;
        c.normalize();
        assert!(c.top > c.bottom);
        assert!(c.shoulder > c.toe);
        assert_eq!(
            serde_json::from_str::<Clouds>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
    }
    #[test]
    fn quality_budgets() {
        assert_eq!(
            [
                CloudQuality::Low,
                CloudQuality::Medium,
                CloudQuality::High,
                CloudQuality::Ultra
            ]
            .map(CloudQuality::steps),
            [(16, 3), (32, 5), (48, 6), (64, 8)]
        );
    }
    #[test]
    fn run_overrides_clamp_and_apply() {
        let mut over = CloudOverrides::default();
        over.set(CloudProperty::parse("coverage").unwrap(), 3.0);
        over.set(CloudProperty::Density, f32::NAN);
        over.set(CloudProperty::parse(" Type ").unwrap(), 0.25);
        let mut c = Clouds::default();
        over.apply(&mut c);
        assert_eq!(c.coverage, 1.0);
        assert_eq!(c.density, Clouds::default().density);
        assert_eq!(c.cloud_type, 0.25);
        assert_eq!(CloudProperty::parse("rain"), None);
    }
}
