//! Saved interface documents and renderer-independent layout and binding rules.
use super::*;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum UiAlign {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

/// Zero is content-sized; percentages are relative to the containing box.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum UiLength {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiStyles {
    pub normal: UiPaint,
    pub hover: UiPaint,
    pub pressed: UiPaint,
    pub disabled: UiPaint,
    pub focused: UiPaint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiBinding {
    pub property: UiProp,
    pub source: UiSource,
    #[serde(default)]
    pub converter: UiConverter,
    #[serde(default)]
    pub two_way: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiWidget {
    pub element: UiElement,
    pub layout: Option<UiLayout>,
    pub style: UiStyles,
    pub class: String,
    pub bindings: Vec<UiBinding>,
    pub items: Vec<String>,
    pub tooltip: String,
    pub world_actor: String,
    pub scroll_target: String,
    pub tab_index: Option<usize>,
    pub transition: f32,
}
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum UiScale {
    #[default]
    ConstantPixel,
    ScaleWithSize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiDocument {
    pub widgets: Vec<UiWidget>,
    pub theme: UiTheme,
    pub styles: BTreeMap<String, UiStyles>,
    pub stylesheets: Vec<String>,
    pub scale: UiScale,
    pub reference_size: [f32; 2],
    pub safe_area: [f32; 4],
    pub prefabs: BTreeMap<String, Vec<UiWidget>>,
}
impl Default for UiDocument {
    fn default() -> Self {
        Self {
            widgets: vec![],
            theme: UiTheme::default(),
            styles: BTreeMap::new(),
            stylesheets: vec![],
            scale: UiScale::default(),
            reference_size: [960., 720.],
            safe_area: [0.; 4],
            prefabs: BTreeMap::new(),
        }
    }
}
impl UiDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .reference_size
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.)
            || self.safe_area.iter().any(|v| !v.is_finite() || *v < 0.)
        {
            return Err("Invalid interface canvas dimensions".into());
        }

        if self.widgets.len() > 10000 {
            return Err("An interface supports at most 10000 widgets".into());
        }
        let mut ids = HashSet::new();
        for widget in &self.widgets {
            let id = &widget.element.id;
            if id.is_empty()
                || id.contains("::row:")
                || id.ends_with("::extent")
                || id == "__tooltip"
                || !ids.insert(id)
            {
                return Err(format!("Duplicate or empty widget id: {id}"));
            }
            if widget.bindings.iter().any(|b| {
                !matches!(
                    b.property,
                    UiProp::Text | UiProp::Value | UiProp::Visible | UiProp::SelectedIndex
                )
            }) {
                return Err(format!("Unsupported binding property on {id}"));
            }
            if let Some(layout) = &widget.layout {
                if layout.columns == 0 || !layout.row_height.is_finite() || layout.row_height <= 0.
                {
                    return Err(format!("Invalid layout on {id}"));
                }
            }
            if !widget
                .element
                .size
                .iter()
                .chain(widget.element.offset.iter())
                .all(|v| v.is_finite())
            {
                return Err(format!("Non-finite widget dimensions: {id}"));
            }
        }
        for widget in &self.widgets {
            let mut seen = HashSet::new();
            let mut current = &widget.element;
            while !current.parent.is_empty() {
                if !seen.insert(&current.id) {
                    return Err(format!("Widget parent cycle: {}", current.id));
                }
                current = &self
                    .widgets
                    .iter()
                    .find(|w| w.element.id == current.parent)
                    .ok_or_else(|| format!("Missing widget parent: {}", current.parent))?
                    .element;
            }
        }
        Ok(())
    }
    pub fn with_stylesheets(&self, dir: &std::path::Path) -> Result<Self, String> {
        let mut result = self.clone();
        let mut styles = BTreeMap::new();
        for path in &self.stylesheets {
            let full = crate::assets::resolve(dir, path)
                .ok_or_else(|| format!("Invalid stylesheet path: {path}"))?;
            let text = std::fs::read_to_string(full).map_err(|e| format!("{path}: {e}"))?;
            let sheet: BTreeMap<String, UiStyles> =
                serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
            styles.extend(sheet);
        }
        styles.extend(self.styles.clone());
        result.styles = styles;
        Ok(result)
    }
    /// Prefix every local reference so the same prefab can appear twice.
    pub fn instantiate(
        &self,
        name: &str,
        prefix: &str,
        parent: &str,
    ) -> Result<Vec<UiWidget>, String> {
        let mut widgets = self
            .prefabs
            .get(name)
            .ok_or_else(|| format!("No UI prefab named {name}"))?
            .clone();
        let ids: HashSet<String> = widgets.iter().map(|w| w.element.id.clone()).collect();
        for w in &mut widgets {
            if ids.contains(&w.scroll_target) {
                w.scroll_target = format!("{prefix}{}", w.scroll_target);
            }
            w.element.id = format!("{prefix}{}", w.element.id);
            w.element.parent = if w.element.parent.is_empty() {
                parent.into()
            } else {
                format!("{prefix}{}", w.element.parent)
            };
        }
        Ok(widgets)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cycles_and_missing_parents() {
        let mut d = UiDocument::default();
        let mut w = UiWidget::default();
        w.element.id = "a".into();
        w.element.parent = "a".into();
        d.widgets.push(w);
        assert!(d.validate().is_err());
        d.widgets[0].element.parent = "missing".into();
        assert!(d.validate().is_err());
        d.widgets[0].element.parent.clear();
        assert!(d.validate().is_ok());
    }
    #[test]
    fn prefabs_repoint_internal_scroll_targets_and_roundtrip() {
        let mut document = UiDocument::default();
        document.prefabs.insert(
            "inventory".into(),
            vec![
                UiWidget {
                    element: UiElement {
                        id: "list".into(),
                        kind: UiKind::ListView,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                UiWidget {
                    element: UiElement {
                        id: "bar".into(),
                        kind: UiKind::Scrollbar,
                        ..Default::default()
                    },
                    scroll_target: "list".into(),
                    ..Default::default()
                },
            ],
        );
        document.widgets = document.instantiate("inventory", "copy_", "").unwrap();
        assert_eq!(document.widgets[1].scroll_target, "copy_list");
        document.validate().unwrap();
        let json = serde_json::to_string(&document).unwrap();
        assert_eq!(serde_json::from_str::<UiDocument>(&json).unwrap(), document);
    }

    #[test]
    fn rich_text_preserves_nested_styles_and_unknown_tags() {
        let runs = rich_text("[b]A[b]B[/b]C[/b][i]D[/i][color=#ff0000]E[/color][unknown]");
        assert_eq!(
            runs.iter().map(|r| r.text.as_str()).collect::<String>(),
            "ABCDE[unknown]"
        );
        assert!(runs[..3].iter().all(|r| r.bold));
        assert!(runs[3].italic);
        assert_eq!(runs[4].color.as_deref(), Some("#ff0000"));
        assert!(!runs[5].bold);
    }

    #[test]
    fn virtual_rows_stay_bounded_for_large_inventories() {
        assert_eq!(visible_rows(100000, 32000., 320., 32.), 999..1011);
        assert_eq!(visible_rows(0, 0., 320., 32.), 0..0);
    }
    #[test]
    fn converters_roundtrip_and_refuse_zero_division() {
        let mut c = UiConverter {
            scale: Some(100.),
            prefix: "$".into(),
            ..Default::default()
        };
        assert_eq!(
            c.read(Evaluated::Number(0.5)),
            Evaluated::Text("$50".into())
        );
        assert_eq!(
            c.write(&Evaluated::Text("$50".into())),
            Some(Evaluated::Number(0.5))
        );
        c.scale = Some(0.);
        assert!(c.write(&Evaluated::Number(1.)).is_none());
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiTextRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub color: Option<String>,
}

/// Small, literal-safe markup: [b], [i], [color=#rrggbb] and closing tags.
pub fn rich_text(text: &str) -> Vec<UiTextRun> {
    let mut result = Vec::new();
    let mut bold = 0usize;
    let mut italic = 0usize;
    let mut colors = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with("[b]") {
            bold += 1;
            rest = &rest[3..];
            continue;
        }
        if rest.starts_with("[/b]") {
            bold = bold.saturating_sub(1);
            rest = &rest[4..];
            continue;
        }
        if rest.starts_with("[i]") {
            italic += 1;
            rest = &rest[3..];
            continue;
        }
        if rest.starts_with("[/i]") {
            italic = italic.saturating_sub(1);
            rest = &rest[4..];
            continue;
        }
        if rest.starts_with("[/color]") {
            colors.pop();
            rest = &rest[8..];
            continue;
        }
        if rest.starts_with("[color=#") {
            if let Some(end) = rest.find(']') {
                let color = &rest[7..end];
                if matches!(color.len(), 7 | 9) && color[1..].chars().all(|c| c.is_ascii_hexdigit())
                {
                    colors.push(color.to_string());
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        let len = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '[')
            .map_or(rest.len(), |(i, _)| i);
        result.push(UiTextRun {
            text: rest[..len].into(),
            bold: bold > 0,
            italic: italic > 0,
            color: colors.last().cloned(),
        });
        rest = &rest[len..];
    }
    result
}
