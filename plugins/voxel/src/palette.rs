//! What a material id means: a name, a color and how much it glows.

use blockloom_plugin_sdk::Value;

pub struct Material {
    pub name: String,
    /// Scene-linear.
    pub color: [f32; 3],
    /// Scene-linear HDR multiplier of the color; 0 does not glow.
    pub emission: f32,
}

/// The names the built-in materials answer to, in id order from 1.
pub const DEFAULT_NAMES: [&str; 7] = ["stone", "dirt", "grass", "sand", "wood", "leaves", "glow"];

const DEFAULTS: [(&str, f32); 7] = [
    ("#7d7f85", 0.0),
    ("#7a5434", 0.0),
    ("#4f9a3a", 0.0),
    ("#d9c88c", 0.0),
    ("#6b4a2b", 0.0),
    ("#2f7a30", 0.0),
    ("#ffb14a", 6.0),
];

pub const STONE: u8 = 1;
pub const DIRT: u8 = 2;
pub const GRASS: u8 = 3;
pub const SAND: u8 = 4;
pub const WOOD: u8 = 5;
pub const LEAVES: u8 = 6;
pub const GLOW: u8 = 7;

pub struct Palette {
    materials: Vec<Material>,
}

/// `#RRGGBB` (sRGB) as scene-linear.
pub fn linear(hex: &str) -> Result<[f32; 3], String> {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    if digits.len() != 6 && digits.len() != 8 {
        return Err(format!("{hex} is not a #RRGGBB color"));
    }
    let channel = |at: usize| -> Result<f32, String> {
        let byte = u8::from_str_radix(&digits[at..at + 2], 16)
            .map_err(|_| format!("{hex} is not a #RRGGBB color"))?;
        let s = f32::from(byte) / 255.0;
        Ok(if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        })
    };
    Ok([channel(0)?, channel(2)?, channel(4)?])
}

impl Palette {
    /// The built-in materials, with `colors` and `emission` laid over them by
    /// position; entries past the built-ins add materials of their own.
    pub fn new(colors: &[String], emission: &[f64]) -> Result<Palette, String> {
        let count = DEFAULTS.len().max(colors.len()).max(emission.len());
        if count > 255 {
            return Err("a palette holds at most 255 materials".to_string());
        }
        let mut materials = Vec::with_capacity(count);
        for i in 0..count {
            let (hex, glow) = DEFAULTS.get(i).copied().unwrap_or(("#c0c0c0", 0.0));
            let hex = colors.get(i).filter(|c| !c.is_empty()).map_or(hex, |c| c);
            let glow = emission.get(i).map_or(glow, |e| *e as f32);
            if !glow.is_finite() || glow < 0.0 {
                return Err(format!("material {} has an emission below 0", i + 1));
            }
            materials.push(Material {
                name: DEFAULT_NAMES
                    .get(i)
                    .map_or_else(|| format!("material{}", i + 1), |n| (*n).to_string()),
                color: linear(hex)?,
                emission: glow,
            });
        }
        Ok(Palette { materials })
    }

    pub fn len(&self) -> u8 {
        self.materials.len() as u8
    }

    pub fn get(&self, id: u8) -> Option<&Material> {
        usize::from(id)
            .checked_sub(1)
            .and_then(|i| self.materials.get(i))
    }

    /// A slot's material: an id, or a name. 0 and "air" are no material.
    pub fn id_of(&self, value: &Value) -> Result<u8, String> {
        match value {
            Value::String(name) if name.eq_ignore_ascii_case("air") || name.is_empty() => Ok(0),
            Value::String(name) => self
                .materials
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(name))
                .map(|i| (i + 1) as u8)
                .or_else(|| name.parse().ok().filter(|&id| id <= self.len()))
                .ok_or_else(|| format!("no material called {name}")),
            Value::Number(n) => n
                .as_f64()
                .map(|f| f.floor())
                .filter(|&f| f >= 0.0 && f <= f64::from(self.len()))
                .map(|f| f as u8)
                .ok_or_else(|| format!("no material {n}")),
            Value::Null => Ok(0),
            other => Err(format!("{other} is not a material")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_plugin_sdk::json;

    #[test]
    fn built_ins_answer_to_name_and_id() {
        let palette = Palette::new(&[], &[]).unwrap();
        assert_eq!(palette.len(), 7);
        assert_eq!(palette.id_of(&json!("Grass")), Ok(GRASS));
        assert_eq!(palette.id_of(&json!(7)), Ok(GLOW));
        assert_eq!(palette.id_of(&json!(7.0)), Ok(GLOW));
        assert_eq!(palette.id_of(&json!("air")), Ok(0));
        assert_eq!(palette.id_of(&json!("3")), Ok(GRASS));
        assert!(palette.id_of(&json!("lava")).is_err());
        assert!(palette.id_of(&json!(8)).is_err());
        assert!(palette.id_of(&json!(-1)).is_err());
        assert!(palette.get(GLOW).unwrap().emission > 0.0);
        assert!(palette.get(0).is_none());
    }

    #[test]
    fn settings_override_and_extend_the_built_ins() {
        let colors = vec![
            "#ff0000".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "#0000ff".to_string(),
        ];
        let palette = Palette::new(&colors, &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0]).unwrap();
        assert_eq!(palette.len(), 8);
        assert_eq!(palette.get(1).unwrap().color, [1.0, 0.0, 0.0]);
        assert_eq!(palette.get(2).unwrap().color, linear("#7a5434").unwrap());
        assert_eq!(palette.get(8).unwrap().emission, 2.0);
        assert_eq!(palette.id_of(&json!("material8")), Ok(8));
    }

    #[test]
    fn bad_colors_and_emission_are_refused() {
        assert!(Palette::new(&["red".to_string()], &[]).is_err());
        assert!(Palette::new(&[], &[-1.0]).is_err());
    }

    #[test]
    fn srgb_converts_to_linear() {
        assert_eq!(linear("#ffffff").unwrap(), [1.0; 3]);
        let mid = linear("#808080").unwrap()[0];
        assert!((mid - 0.2158).abs() < 0.001, "{mid}");
    }
}
