//! Property animations: easing, keys, tracks and sampling.
use super::*;

/// How a key eases into the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UiEase {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl UiEase {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0., 1.);
        match self {
            UiEase::Linear => t,
            UiEase::EaseIn => t * t,
            UiEase::EaseOut => 1. - (1. - t) * (1. - t),
            UiEase::EaseInOut => t * t * (3. - 2. * t),
        }
    }
}

/// A keyed value: a number, or a `#rrggbb[aa]` color for the color tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UiKeyValue {
    Number(f64),
    Color(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiKey {
    pub time: f32,
    pub value: UiKeyValue,
    /// How it eases from this key to the next.
    #[serde(default)]
    pub ease: UiEase,
}

/// One property of one widget over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiTrack {
    pub widget: String,
    pub property: UiProp,
    pub keys: Vec<UiKey>,
}

/// The properties a track can drive: the sizes and colors an element is
/// restyled by, and a value.
pub const ANIMATABLE_NUMBERS: [UiProp; 10] = [
    UiProp::Width,
    UiProp::Height,
    UiProp::TextSize,
    UiProp::CornerRadius,
    UiProp::Padding,
    UiProp::Value,
    UiProp::Opacity,
    UiProp::OffsetX,
    UiProp::OffsetY,
    UiProp::Rotation,
];
pub const ANIMATABLE_COLORS: [UiProp; 2] = [UiProp::Background, UiProp::TextColor];

fn parse_hex(text: &str) -> Option<[f32; 4]> {
    let hex = text.trim().strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| {
        u8::from_str_radix(&hex[i..i + 2], 16)
            .ok()
            .map(|b| b as f32)
    };
    Some([
        byte(0)?,
        byte(2)?,
        byte(4)?,
        if hex.len() == 8 { byte(6)? } else { 255. },
    ])
}

impl UiTrack {
    /// The value at `time`, holding the first key before it and the last after.
    pub fn sample(&self, time: f32) -> Option<UiKeyValue> {
        let first = self.keys.first()?;
        let last = self.keys.last()?;
        if time <= first.time {
            return Some(first.value.clone());
        }
        if time >= last.time {
            return Some(last.value.clone());
        }
        let at = self.keys.iter().rposition(|k| k.time <= time)?;
        let (a, b) = (&self.keys[at], &self.keys[at + 1]);
        let span = (b.time - a.time).max(f32::EPSILON);
        let f = a.ease.apply((time - a.time) / span);
        match (&a.value, &b.value) {
            (UiKeyValue::Number(x), UiKeyValue::Number(y)) => {
                Some(UiKeyValue::Number(x + (y - x) * f as f64))
            }
            (UiKeyValue::Color(x), UiKeyValue::Color(y)) => {
                let (x, y) = (parse_hex(x)?, parse_hex(y)?);
                let c: Vec<u8> = (0..4)
                    .map(|i| (x[i] + (y[i] - x[i]) * f).round().clamp(0., 255.) as u8)
                    .collect();
                Some(UiKeyValue::Color(format!(
                    "#{:02x}{:02x}{:02x}{:02x}",
                    c[0], c[1], c[2], c[3]
                )))
            }
            _ => Some(a.value.clone()),
        }
    }

    pub(super) fn validate(&self, duration: f32) -> Result<(), String> {
        let color = ANIMATABLE_COLORS.contains(&self.property);
        if !color && !ANIMATABLE_NUMBERS.contains(&self.property) {
            return Err(format!("{:?} cannot be animated", self.property));
        }
        if self.keys.is_empty() {
            return Err(format!("A track on {} needs a key", self.widget));
        }
        let mut last = -1.;
        for key in &self.keys {
            if !key.time.is_finite() || key.time < 0. || key.time > duration || key.time <= last {
                return Err("Keys must be in strictly increasing time inside the animation".into());
            }
            last = key.time;
            match &key.value {
                UiKeyValue::Number(n) if !color && n.is_finite() => {}
                UiKeyValue::Color(c) if color && parse_hex(c).is_some() => {}
                _ => {
                    return Err(format!(
                        "A key on {} holds the wrong kind of value",
                        self.widget
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiAnimation {
    pub duration: f32,
    #[serde(default)]
    pub looping: bool,
    pub tracks: Vec<UiTrack>,
}

impl UiAnimation {
    pub fn validate(&self) -> Result<(), String> {
        if !self.duration.is_finite() || self.duration <= 0. || self.duration > 3600. {
            return Err("An animation's duration must be above zero".into());
        }
        self.tracks
            .iter()
            .try_for_each(|t| t.validate(self.duration))
    }
}
