//! Saved interface documents and renderer-independent layout and binding rules.
use super::*;
use blockloom_schema_derive::Schema;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Schema)]
pub enum UiAlign {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

/// Zero is content-sized; percentages are relative to the containing box.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Schema)]
pub enum UiLength {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiLayout {
    pub width: UiLength,
    pub height: UiLength,
    pub min_size: [f32; 2],
    pub max_size: [f32; 2],
    /// Left, top, right, bottom.
    pub margin: [f32; 4],
    pub padding: [f32; 4],
    pub gap: f32,
    pub columns: u16,
    pub grow: f32,
    pub align: UiAlign,
    pub absolute: bool,
    pub row_height: f32,
}
impl Default for UiLayout {
    fn default() -> Self {
        Self {
            width: UiLength::Auto,
            height: UiLength::Auto,
            min_size: [0.; 2],
            max_size: [0.; 2],
            margin: [0.; 4],
            padding: [0.; 4],
            gap: 8.,
            columns: 2,
            grow: 0.,
            align: UiAlign::Stretch,
            absolute: false,
            row_height: 32.,
        }
    }
}

impl UiLayout {
    pub fn validate_edit(&self) -> Result<(), String> {
        let layout = self;
        let length_valid = |v: UiLength| match v {
            UiLength::Auto => true,
            UiLength::Px(n) | UiLength::Percent(n) => n.is_finite() && n >= 0.,
        };
        if !length_valid(layout.width)
            || !length_valid(layout.height)
            || layout.columns == 0
            || layout.columns > 256
            || !layout.row_height.is_finite()
            || layout.row_height <= 0.
            || !layout.gap.is_finite()
            || layout.gap < 0.
            || !layout.grow.is_finite()
            || layout.grow < 0.
            || layout
                .min_size
                .iter()
                .chain(layout.max_size.iter())
                .chain(layout.padding.iter())
                .any(|v| !v.is_finite() || *v < 0.)
            || layout.margin.iter().any(|v| !v.is_finite())
            || (0..2).any(|i| layout.max_size[i] > 0. && layout.max_size[i] < layout.min_size[i])
        {
            return Err("Invalid layout dimensions or spacing".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiPaint {
    pub background: Option<String>,
    pub text_color: Option<String>,
    pub text_size: Option<f32>,
    pub border_color: Option<String>,
    pub border_width: Option<f32>,
    pub radius: Option<f32>,
    pub shadow: Option<String>,
    pub fonts: Vec<String>,
    /// How an Image widget fills its box; unset keeps the image's own size.
    pub image_fit: Option<UiImageFit>,
}

/// How an image fills its widget. `Sliced` borders are `[left, right, top, bottom]`
/// in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Schema)]
pub enum UiImageFit {
    Stretch,
    Sliced { border: [f32; 4] },
    Tiled,
}

impl UiImageFit {
    pub fn validate(&self) -> Result<(), String> {
        if let UiImageFit::Sliced { border } = self
            && border.iter().any(|v| !v.is_finite() || *v < 0.)
        {
            return Err("Nine-slice borders must be zero or more".into());
        }
        Ok(())
    }
}
impl UiPaint {
    pub fn over(&self, base: &Self) -> Self {
        Self {
            background: self.background.clone().or(base.background.clone()),
            text_color: self.text_color.clone().or(base.text_color.clone()),
            text_size: self.text_size.or(base.text_size),
            border_color: self.border_color.clone().or(base.border_color.clone()),
            border_width: self.border_width.or(base.border_width),
            radius: self.radius.or(base.radius),
            shadow: self.shadow.clone().or(base.shadow.clone()),
            fonts: if self.fonts.is_empty() {
                base.fonts.clone()
            } else {
                self.fonts.clone()
            },
            image_fit: self.image_fit.or(base.image_fit),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiStyles {
    pub normal: UiPaint,
    pub hover: UiPaint,
    pub pressed: UiPaint,
    pub disabled: UiPaint,
    pub focused: UiPaint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub enum UiSource {
    Variable {
        #[serde(default)]
        actor: String,
        name: String,
    },
    Component {
        actor: String,
        component: String,
        field: String,
    },
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiConverter {
    pub prefix: String,
    pub suffix: String,
    pub scale: Option<f64>,
    pub decimals: Option<u8>,
    pub invert: bool,
}
impl UiConverter {
    pub fn read(&self, value: Evaluated) -> Evaluated {
        let value = if self.invert {
            Evaluated::Bool(!value.as_bool())
        } else {
            value
        };
        let value = if let Some(scale) = self.scale {
            Evaluated::Number(value.as_number().unwrap_or(0.) * scale)
        } else {
            value
        };
        let text = match self.decimals {
            Some(n) => format!("{:.*}", n.min(12) as usize, value.as_number().unwrap_or(0.)),
            None => value.as_text(),
        };
        if self.decimals.is_some() || !self.prefix.is_empty() || !self.suffix.is_empty() {
            Evaluated::Text(format!("{}{}{}", self.prefix, text, self.suffix))
        } else {
            value
        }
    }
    pub fn write(&self, value: &Evaluated) -> Option<Evaluated> {
        if self.invert {
            return Some(Evaluated::Bool(!value.as_bool()));
        }
        let text = value.as_text();
        let text = text
            .strip_prefix(&self.prefix)?
            .strip_suffix(&self.suffix)?;
        if let Some(scale) = self.scale {
            if scale == 0. || !scale.is_finite() {
                return None;
            }
            return text
                .parse::<f64>()
                .ok()
                .map(|n| Evaluated::Number(n / scale));
        }
        Some(if self.prefix.is_empty() && self.suffix.is_empty() {
            value.clone()
        } else {
            Evaluated::Text(text.into())
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct UiBinding {
    pub property: UiProp,
    pub source: UiSource,
    #[serde(default)]
    pub converter: UiConverter,
    #[serde(default)]
    pub two_way: bool,
}

/// A viewport size range; every bound is optional and inclusive, in canvas
/// pixels (after the safe area and scale are taken off).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiBreakpoint {
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
}

impl UiBreakpoint {
    pub fn matches(&self, width: f32, height: f32) -> bool {
        self.min_width.is_none_or(|v| width >= v)
            && self.max_width.is_none_or(|v| width <= v)
            && self.min_height.is_none_or(|v| height >= v)
            && self.max_height.is_none_or(|v| height <= v)
    }

    pub fn validate(&self) -> Result<(), String> {
        let bounds = [
            self.min_width,
            self.max_width,
            self.min_height,
            self.max_height,
        ];
        if bounds.iter().flatten().any(|v| !v.is_finite() || *v < 0.) {
            return Err("Breakpoint sizes must be finite and not negative".into());
        }
        let ordered = |lo: Option<f32>, hi: Option<f32>| lo.zip(hi).is_none_or(|(lo, hi)| lo <= hi);
        if !ordered(self.min_width, self.max_width) || !ordered(self.min_height, self.max_height) {
            return Err("A breakpoint's minimum cannot exceed its maximum".into());
        }
        Ok(())
    }
}

/// How a widget differs while the viewport is inside `when`. The first
/// matching variant wins; what it leaves unset follows the base widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct UiVariant {
    pub when: UiBreakpoint,
    #[serde(default)]
    pub layout: Option<UiLayout>,
    #[serde(default)]
    pub visible: Option<bool>,
}

impl UiWidget {
    /// Index of the first variant that fits a viewport of this size.
    pub fn variant_for(&self, width: f32, height: f32) -> Option<usize> {
        self.variants
            .iter()
            .position(|v| v.when.matches(width, height))
    }
}

/// What a widget reports to the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum UiEventKind {
    Click,
    Change,
}

/// What a widget event does. In `SetVariable`, a value of `$value` stands for
/// the widget's new value on a Change event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub enum UiDo {
    Broadcast {
        message: String,
    },
    /// Starts (or restarts) a document animation.
    PlayAnimation {
        name: String,
    },
    StopAnimation {
        name: String,
    },
    SetVariable {
        #[serde(default)]
        actor: String,
        name: String,
        value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct UiAction {
    pub on: UiEventKind,
    pub run: UiDo,
}

/// Visual state laid over an element after layout: it never changes what
/// layout measures. Offset adds to wherever the anchor and layout put the
/// element and rotation turns it about its centre, so neither fights the
/// anchor's own shift. Opacity multiplies down the tree.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiFx {
    pub opacity: f32,
    pub offset: [f32; 2],
    /// Degrees, clockwise.
    pub rotation: f32,
}

impl Default for UiFx {
    fn default() -> Self {
        Self {
            opacity: 1.,
            offset: [0., 0.],
            rotation: 0.,
        }
    }
}

impl UiFx {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.opacity.is_finite() || !(0. ..=1.).contains(&self.opacity) {
            return Err("Opacity must be between 0 and 1".into());
        }
        if !self.offset.iter().all(|v| v.is_finite()) || !self.rotation.is_finite() {
            return Err("Offset and rotation must be finite".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiWidget {
    pub element: UiElement,
    pub layout: Option<UiLayout>,
    pub style: UiStyles,
    pub class: String,
    pub bindings: Vec<UiBinding>,
    /// Event handlers authored on the widget, run by the player without blocks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<UiAction>,
    /// Layout and visibility changes for other viewport sizes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<UiVariant>,
    pub items: Vec<String>,
    pub tooltip: String,
    pub world_actor: String,
    pub scroll_target: String,
    pub tab_index: Option<usize>,
    pub transition: f32,
    /// Opacity, offset and rotation the widget starts with.
    #[serde(skip_serializing_if = "UiFx::is_default")]
    pub fx: UiFx,
    /// On the root of a prefab instance: the prefab it came from.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instance_of: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Schema)]
pub enum UiScale {
    #[default]
    ConstantPixel,
    ScaleWithSize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiDocument {
    /// Schema version; 0 is a document saved before versions existed.
    #[serde(default = "legacy_version")]
    pub version: u32,
    pub widgets: Vec<UiWidget>,
    pub theme: UiTheme,
    /// Colors laid over `theme`'s built-in ones, from a `.blockstyle`.
    #[serde(skip_serializing_if = "UiTokens::is_empty")]
    pub tokens: UiTokens,
    pub styles: BTreeMap<String, UiStyles>,
    pub stylesheets: Vec<String>,
    pub scale: UiScale,
    pub reference_size: [f32; 2],
    pub safe_area: [f32; 4],
    pub prefabs: BTreeMap<String, Vec<UiWidget>>,
    /// What each prefab lets an instance set by name: properties and slots.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub exports: BTreeMap<String, Vec<UiExport>>,
    /// Named property animations the document's actions (or blocks) can play.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub animations: BTreeMap<String, UiAnimation>,
}

/// A theme's colors, each `#RRGGBB` or `#RRGGBBAA`. Unset ones keep the
/// built-in theme's.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Schema)]
#[serde(default)]
pub struct UiTokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_focused: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Slider and bar fills, an on toggle and the caret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Slider and bar tracks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toggle_off: Option<String>,
}

impl UiTokens {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn slots(&mut self) -> [(&'static str, &mut Option<String>); 8] {
        [
            ("panel", &mut self.panel),
            ("button", &mut self.button),
            ("input", &mut self.input),
            ("input_focused", &mut self.input_focused),
            ("text", &mut self.text),
            ("accent", &mut self.accent),
            ("track", &mut self.track),
            ("toggle_off", &mut self.toggle_off),
        ]
    }

    /// Takes every color `over` sets.
    pub fn layer(&mut self, mut over: UiTokens) {
        for ((_, mine), (_, theirs)) in self.slots().into_iter().zip(over.slots()) {
            if theirs.is_some() {
                *mine = theirs.take();
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut copy = self.clone();
        for (name, value) in copy.slots() {
            if let Some(hex) = value
                && !is_hex_color(hex)
            {
                return Err(format!(
                    "The theme color {name} isn't #RRGGBB or #RRGGBBAA: {hex}"
                ));
            }
        }
        Ok(())
    }
}

fn is_hex_color(text: &str) -> bool {
    text.strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// What a `.blockstyle` file holds: a theme to start from, colors over it,
/// and classes widgets pick by name.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiStyleFile {
    pub version: u32,
    pub base: UiTheme,
    pub tokens: UiTokens,
    pub classes: BTreeMap<String, UiStyles>,
}

pub const STYLE_EXTENSION: &str = "blockstyle";
pub const STYLE_VERSION: u32 = 1;

/// A named handle on a prefab: one property of one widget inside it, or a
/// container widget other widgets can be placed into.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
pub struct UiExport {
    pub name: String,
    /// The widget's id inside the prefab.
    pub widget: String,
    /// A property path (see `property_metadata`); empty for a slot.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub slot: bool,
}
/// The schema version this build writes.
pub const UI_SCHEMA_VERSION: u32 = 1;

fn legacy_version() -> u32 {
    0
}

impl Default for UiDocument {
    fn default() -> Self {
        Self {
            version: UI_SCHEMA_VERSION,
            widgets: vec![],
            theme: UiTheme::default(),
            tokens: UiTokens::default(),
            styles: BTreeMap::new(),
            stylesheets: vec![],
            scale: UiScale::default(),
            reference_size: [960., 720.],
            safe_area: [0.; 4],
            prefabs: BTreeMap::new(),
            exports: BTreeMap::new(),
            animations: BTreeMap::new(),
        }
    }
}
fn check_variants(variants: &[UiVariant]) -> Result<(), String> {
    for variant in variants {
        variant.when.validate()?;
        if let Some(layout) = &variant.layout
            && (layout.columns == 0 || !layout.row_height.is_finite() || layout.row_height <= 0.)
        {
            return Err("A variant's layout is invalid".into());
        }
    }
    Ok(())
}

fn check_actions(actions: &[UiAction]) -> Result<(), String> {
    for action in actions {
        match &action.run {
            UiDo::Broadcast { message } if message.trim().is_empty() => {
                return Err("A broadcast needs a message".into());
            }
            UiDo::PlayAnimation { name } | UiDo::StopAnimation { name }
                if name.trim().is_empty() =>
            {
                return Err("An animation action needs a name".into());
            }
            UiDo::SetVariable { name, .. } if name.trim().is_empty() => {
                return Err("Set variable needs a variable name".into());
            }
            _ => {}
        }
    }
    Ok(())
}

fn to_json(widget: &UiWidget) -> Result<serde_json::Value, String> {
    serde_json::to_value(widget).map_err(|e| e.to_string())
}

/// Field-level three-way merge: a field the instance changed from `old` keeps
/// its value, every other field takes the prefab's `new` one.
fn merge_json(
    old: &serde_json::Value,
    new: &serde_json::Value,
    cur: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;
    if cur == old {
        return new.clone();
    }
    match (old, new, cur) {
        (Value::Object(o), Value::Object(n), Value::Object(c)) => {
            let mut out = serde_json::Map::new();
            for key in n.keys().chain(c.keys()) {
                if out.contains_key(key) {
                    continue;
                }
                let blank = Value::Null;
                let merged = match (o.get(key), n.get(key), c.get(key)) {
                    (o, n, Some(c)) => merge_json(o.unwrap_or(&blank), n.unwrap_or(&blank), c),
                    (_, Some(n), None) => n.clone(),
                    _ => continue,
                };
                out.insert(key.clone(), merged);
            }
            Value::Object(out)
        }
        _ => cur.clone(),
    }
}

fn merge_widget(old: &UiWidget, new: &UiWidget, cur: &UiWidget) -> Result<UiWidget, String> {
    let merged = merge_json(&to_json(old)?, &to_json(new)?, &to_json(cur)?);
    let mut widget: UiWidget = serde_json::from_value(merged).map_err(|e| e.to_string())?;
    // Identity and place in the tree always follow the rebuilt instance.
    widget.element.id = new.element.id.clone();
    widget.element.parent = new.element.parent.clone();
    widget.instance_of = new.instance_of.clone();
    Ok(widget)
}

fn changed_paths(a: &serde_json::Value, b: &serde_json::Value, at: &str, f: &mut dyn FnMut(&str)) {
    use serde_json::Value;
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let path = if at.is_empty() {
                    key.clone()
                } else {
                    format!("{at}.{key}")
                };
                changed_paths(
                    x.get(key).unwrap_or(&Value::Null),
                    y.get(key).unwrap_or(&Value::Null),
                    &path,
                    f,
                );
            }
        }
        _ => f(at),
    }
}

/// A bounded pool of visible rows, including one overscan row at each end.
pub fn visible_rows(
    count: usize,
    scroll: f32,
    viewport: f32,
    row_height: f32,
) -> std::ops::Range<usize> {
    let height = row_height.max(1.);
    let first = ((scroll.max(0.) / height) as usize)
        .min(count)
        .saturating_sub(1);
    let last = (((scroll.max(0.) + viewport.max(0.)) / height).ceil() as usize)
        .saturating_add(1)
        .min(count);
    first..last.max(first)
}

/// Adds the device's display cutout insets (notch, punch hole, gesture bar)
/// over the authored `safe_area` padding, edge by edge. Both are
/// `[left, top, right, bottom]` in logical pixels; negatives clamp to zero,
/// so a stale reading never pulls widgets off screen.
pub fn effective_safe_area(authored: [f32; 4], device: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = (authored[i] + device[i]).max(0.0);
    }
    out
}

#[path = "ui_framework/animation.rs"]
mod animation;
pub use animation::*;
#[path = "ui_framework/edit.rs"]
mod edit;
pub use edit::*;
#[path = "ui_framework/prefabs.rs"]
mod prefabs;
#[path = "ui_framework/rich_text.rs"]
mod rich_text;
pub use rich_text::*;
#[cfg(test)]
#[path = "ui_framework/scale_tests.rs"]
mod scale_tests;
#[cfg(test)]
#[path = "ui_framework/tests.rs"]
mod tests;
