// Typed wrappers over the string dials scripts used to name by hand.
// This file is compiled twice: once as `script::typed` in `blockloom-core`
// (so host tests cover it), and once as text into the `blockloom` crate a
// script links against (so `use blockloom::*` finds it). It touches nothing
// host-side, which is what keeps both halves identical.
//
// The string methods stay: a script written last year still builds. These
// enums are the checked spelling - a typo fails compile instead of silently
// reading zero at runtime.

/// How a force applies to a body for one fixed step. Names match the block
/// dropdown and [`crate::physics::ForceMode`], so the host parses them back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ForceMode {
    Force,
    Acceleration,
    Impulse,
    VelocityChange,
}

impl ForceMode {
    pub fn name(self) -> &'static str {
        match self {
            ForceMode::Force => "Force",
            ForceMode::Acceleration => "Acceleration",
            ForceMode::Impulse => "Impulse",
            ForceMode::VelocityChange => "VelocityChange",
        }
    }
}

/// Which wind dial `set wind` writes. Names match `WindProperty`, so the
/// host parses them back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindDial {
    Direction,
    Speed,
    Gust,
    Storm,
}

impl WindDial {
    pub fn name(self) -> &'static str {
        match self {
            WindDial::Direction => "direction",
            WindDial::Speed => "speed",
            WindDial::Gust => "gust",
            WindDial::Storm => "storm",
        }
    }
}

/// Which water dial `set water` writes. Names match `WaterProperty`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaterDial {
    Level,
    Chop,
    Foam,
}

impl WaterDial {
    pub fn name(self) -> &'static str {
        match self {
            WaterDial::Level => "level",
            WaterDial::Chop => "chop",
            WaterDial::Foam => "foam",
        }
    }
}

/// Which volumetric-cloud dial `set clouds` writes. Names match
/// `CloudProperty`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CloudDial {
    Coverage,
    Density,
    Type,
}

impl CloudDial {
    pub fn name(self) -> &'static str {
        match self {
            CloudDial::Coverage => "coverage",
            CloudDial::Density => "density",
            CloudDial::Type => "type",
        }
    }
}

/// Which precipitation `set precipitation` writes. Names match
/// `PrecipitationKind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Precipitation {
    Rain,
    Snow,
}

impl Precipitation {
    pub fn name(self) -> &'static str {
        match self {
            Precipitation::Rain => "rain",
            Precipitation::Snow => "snow",
        }
    }
}

/// How a tween eases from 0 to 1. Names match `TweenEasing` and the tween
/// blocks' dropdown.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Bounce,
    Elastic,
}

impl Easing {
    pub fn name(self) -> &'static str {
        match self {
            Easing::Linear => "Linear",
            Easing::EaseIn => "EaseIn",
            Easing::EaseOut => "EaseOut",
            Easing::EaseInOut => "EaseInOut",
            Easing::Bounce => "Bounce",
            Easing::Elastic => "Elastic",
        }
    }
}

/// An sRGB color as three bytes. Formats as the `#RRGGBB` the tint blocks
/// and `set_color` take, so a typo fails compile instead of landing magenta
/// at runtime.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const BLACK: Color = Color::new(0, 0, 0);
    pub const WHITE: Color = Color::new(255, 255, 255);
    pub const RED: Color = Color::new(255, 0, 0);
    pub const GREEN: Color = Color::new(0, 255, 0);
    pub const BLUE: Color = Color::new(0, 0, 255);
    pub const YELLOW: Color = Color::new(255, 255, 0);
    pub const CYAN: Color = Color::new(0, 255, 255);
    pub const MAGENTA: Color = Color::new(255, 0, 255);

    /// `#RRGGBB`, the spelling the host parses.
    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Parses `#RRGGBB` (a leading `#` is optional). `None` on anything else.
    pub fn parse(hex: &str) -> Option<Self> {
        let bytes = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
        if bytes.len() != 6 {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(bytes.get(at..at + 2)?, 16).ok();
        Some(Color::new(channel(0)?, channel(2)?, channel(4)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_names_match_the_documented_spellings() {
        assert_eq!(ForceMode::Force.name(), "Force");
        assert_eq!(ForceMode::Acceleration.name(), "Acceleration");
        assert_eq!(ForceMode::Impulse.name(), "Impulse");
        assert_eq!(ForceMode::VelocityChange.name(), "VelocityChange");
        assert_eq!(WindDial::Direction.name(), "direction");
        assert_eq!(WindDial::Speed.name(), "speed");
        assert_eq!(WindDial::Gust.name(), "gust");
        assert_eq!(WindDial::Storm.name(), "storm");
        assert_eq!(WaterDial::Level.name(), "level");
        assert_eq!(WaterDial::Chop.name(), "chop");
        assert_eq!(WaterDial::Foam.name(), "foam");
        assert_eq!(CloudDial::Coverage.name(), "coverage");
        assert_eq!(CloudDial::Density.name(), "density");
        assert_eq!(CloudDial::Type.name(), "type");
        assert_eq!(Precipitation::Rain.name(), "rain");
        assert_eq!(Precipitation::Snow.name(), "snow");
        assert_eq!(Easing::Linear.name(), "Linear");
        assert_eq!(Easing::EaseIn.name(), "EaseIn");
        assert_eq!(Easing::EaseOut.name(), "EaseOut");
        assert_eq!(Easing::EaseInOut.name(), "EaseInOut");
        assert_eq!(Easing::Bounce.name(), "Bounce");
        assert_eq!(Easing::Elastic.name(), "Elastic");
    }

    #[test]
    fn color_hex_round_trips() {
        assert_eq!(Color::RED.hex(), "#FF0000");
        assert_eq!(Color::parse("#ff0000"), Some(Color::RED));
        assert_eq!(Color::parse("00FF00"), Some(Color::GREEN));
        assert_eq!(Color::parse("nope"), None);
        assert_eq!(Color::parse("#12345"), None);
    }
}
