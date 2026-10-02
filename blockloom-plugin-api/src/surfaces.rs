//! Where a plugin appears in the editor beyond its inspector and panels:
//! menu items, keyboard shortcuts and scene-view overlays.
//!
//! A menu item or shortcut runs one of the package's commands (with fixed
//! arguments), so everything it does is the command's validated, undoable
//! action. An overlay is a toggle in the scene view: while on, the plugin's
//! hosted preview module is asked for `overlay.<name>` and what it answers
//! ([`parse_shapes`]) is drawn over the world. Overlays only look; they never
//! change the project.

use crate::id::validate_type_id;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The most shapes one overlay answer may hold.
pub const MAX_SHAPES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuSchema {
    pub name: String,
    pub title: String,
    /// A heading inside the plugin's menu; items with the same group sit together.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    pub command: String,
    #[serde(default)]
    pub args: BTreeMap<String, Value>,
    /// Keys that also run it, such as `Ctrl+Shift+V`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortcutSchema {
    pub name: String,
    pub title: String,
    pub keys: String,
    pub command: String,
    #[serde(default)]
    pub args: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlaySchema {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Shown as soon as the scene view is, until the user turns it off.
    #[serde(default)]
    pub default_on: bool,
    /// How often the module is asked again while the overlay is on.
    #[serde(default = "default_interval")]
    pub interval_ms: u32,
}

fn default_interval() -> u32 {
    250
}

impl OverlaySchema {
    /// The module op the overlay is asked through.
    pub fn op(&self) -> String {
        format!("overlay.{}", self.name)
    }

    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        if self.title.trim().is_empty() {
            return Err(format!("overlay {}: a title is needed", self.name));
        }
        if !(50..=10_000).contains(&self.interval_ms) {
            return Err(format!(
                "overlay {}: interval_ms is between 50 and 10000",
                self.name
            ));
        }
        Ok(())
    }
}

impl MenuSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        if self.title.trim().is_empty() {
            return Err(format!("menu item {}: a title is needed", self.name));
        }
        if let Some(keys) = &self.shortcut {
            normalize_shortcut(keys).map_err(|e| format!("menu item {}: {e}", self.name))?;
        }
        Ok(())
    }

    /// The shortcut in its one spelling, when it has one.
    pub fn keys(&self) -> Option<String> {
        self.shortcut
            .as_deref()
            .and_then(|k| normalize_shortcut(k).ok())
    }
}

impl ShortcutSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        if self.title.trim().is_empty() {
            return Err(format!("shortcut {}: a title is needed", self.name));
        }
        normalize_shortcut(&self.keys).map_err(|e| format!("shortcut {}: {e}", self.name))?;
        Ok(())
    }

    pub fn normalized(&self) -> Option<String> {
        normalize_shortcut(&self.keys).ok()
    }
}

const NAMED_KEYS: &[&str] = &[
    "Space",
    "Enter",
    "Escape",
    "Tab",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Up",
    "Down",
    "Left",
    "Right",
    "Plus",
    "Minus",
    "Comma",
    "Period",
    "Slash",
    "Backslash",
    "Semicolon",
    "Quote",
    "BracketLeft",
    "BracketRight",
    "Backquote",
];

/// `ctrl+shift+v` as `Ctrl+Shift+V`: modifiers in a fixed order, one key. A
/// plain key with no modifier is refused (it would steal typing) unless it is
/// an F-key.
pub fn normalize_shortcut(keys: &str) -> Result<String, String> {
    let mut modifiers = BTreeSet::new();
    let mut key: Option<String> = None;
    for part in keys.split('+') {
        let part = part.trim();
        let modifier = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(0),
            "alt" | "option" => Some(1),
            "shift" => Some(2),
            "meta" | "cmd" | "command" | "super" => Some(3),
            _ => None,
        };
        if let Some(rank) = modifier {
            if !modifiers.insert(rank) {
                return Err(format!("\"{keys}\" repeats a modifier"));
            }
            continue;
        }
        if key.is_some() {
            return Err(format!("\"{keys}\" names more than one key"));
        }
        let upper = part.to_ascii_uppercase();
        let named = NAMED_KEYS.iter().find(|n| n.eq_ignore_ascii_case(part));
        let function = upper
            .strip_prefix('F')
            .and_then(|n| n.parse::<u8>().ok())
            .filter(|n| (1..=12).contains(n));
        key = Some(if let Some(n) = named {
            (*n).to_string()
        } else if function.is_some()
            || (part.len() == 1 && part.chars().all(|c| c.is_ascii_alphanumeric()))
        {
            upper
        } else {
            return Err(format!("\"{part}\" is not a key"));
        });
    }
    let key = key.ok_or_else(|| format!("\"{keys}\" names no key"))?;
    let is_function = key.starts_with('F') && key[1..].parse::<u8>().is_ok();
    if modifiers.is_empty() && !is_function {
        return Err(format!(
            "\"{keys}\" has no modifier, and would take over typing"
        ));
    }
    let names = ["Ctrl", "Alt", "Shift", "Meta"];
    let mut out: Vec<String> = modifiers.iter().map(|m| names[*m].to_string()).collect();
    out.push(key);
    Ok(out.join("+"))
}

/// One thing an overlay draws, in world units.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Line {
        from: [f32; 3],
        to: [f32; 3],
        color: [f32; 4],
    },
    Box {
        min: [f32; 3],
        max: [f32; 3],
        color: [f32; 4],
    },
    Point {
        at: [f32; 3],
        size: f32,
        color: [f32; 4],
    },
}

fn vec3(v: &Value, what: &str) -> Result<[f32; 3], String> {
    let list = v
        .as_array()
        .filter(|l| l.len() == 3)
        .ok_or_else(|| format!("{what} is [x, y, z]"))?;
    let mut out = [0.0f32; 3];
    for (slot, n) in out.iter_mut().zip(list) {
        let n = n
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{what} holds finite numbers"))?;
        *slot = n as f32;
    }
    Ok(out)
}

/// `#RRGGBB`, `#RRGGBBAA` or `[r, g, b, a]` in 0 to 1; white when absent.
pub fn parse_color(v: Option<&Value>) -> Result<[f32; 4], String> {
    match v {
        None | Some(Value::Null) => Ok([1.0; 4]),
        Some(Value::String(s)) => {
            let hex = s.strip_prefix('#').unwrap_or(s);
            if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("\"{s}\" is not #RRGGBB or #RRGGBBAA"));
            }
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(255);
            let a = if hex.len() == 8 { byte(6) } else { 255 };
            Ok([byte(0), byte(2), byte(4), a].map(|b| f32::from(b) / 255.0))
        }
        Some(Value::Array(list)) if list.len() == 4 => {
            let mut out = [0.0f32; 4];
            for (slot, n) in out.iter_mut().zip(list) {
                *slot = n
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or("a color holds finite numbers")? as f32;
            }
            Ok(out.map(|c| c.clamp(0.0, 1.0)))
        }
        Some(_) => Err("a color is a hex string or [r, g, b, a]".to_string()),
    }
}

/// An overlay op's answer: `{"lines": [{from, to, color?}], "boxes": [{min,
/// max, color?}], "points": [{at, size?, color?}]}`, any of them absent.
pub fn parse_shapes(answer: &Value) -> Result<Vec<Shape>, String> {
    type Make<'a> = Box<dyn FnMut(&Value) -> Result<Shape, String> + 'a>;
    let mut shapes = Vec::new();
    let mut each = |key: &str, mut make: Make| {
        let Some(list) = answer.get(key) else {
            return Ok(());
        };
        let list = list
            .as_array()
            .ok_or_else(|| format!("`{key}` is a list"))?;
        for item in list {
            if shapes.len() >= MAX_SHAPES {
                return Err(format!("an overlay may draw {MAX_SHAPES} shapes at most"));
            }
            shapes.push(make(item).map_err(|e| format!("{key}: {e}"))?);
        }
        Ok(())
    };
    each(
        "lines",
        Box::new(|v| {
            Ok(Shape::Line {
                from: vec3(v.get("from").unwrap_or(&Value::Null), "from")?,
                to: vec3(v.get("to").unwrap_or(&Value::Null), "to")?,
                color: parse_color(v.get("color"))?,
            })
        }),
    )?;
    each(
        "boxes",
        Box::new(|v| {
            Ok(Shape::Box {
                min: vec3(v.get("min").unwrap_or(&Value::Null), "min")?,
                max: vec3(v.get("max").unwrap_or(&Value::Null), "max")?,
                color: parse_color(v.get("color"))?,
            })
        }),
    )?;
    each(
        "points",
        Box::new(|v| {
            Ok(Shape::Point {
                at: vec3(v.get("at").unwrap_or(&Value::Null), "at")?,
                size: v.get("size").and_then(Value::as_f64).unwrap_or(0.25) as f32,
                color: parse_color(v.get("color"))?,
            })
        }),
    )?;
    Ok(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shortcuts_are_normalized_and_plain_keys_are_refused() {
        assert_eq!(normalize_shortcut("ctrl+shift+v").unwrap(), "Ctrl+Shift+V");
        assert_eq!(normalize_shortcut("Shift+Ctrl+V").unwrap(), "Ctrl+Shift+V");
        assert_eq!(normalize_shortcut("cmd+pageup").unwrap(), "Meta+PageUp");
        assert_eq!(normalize_shortcut("f5").unwrap(), "F5");
        assert_eq!(normalize_shortcut("Alt+F12").unwrap(), "Alt+F12");
        for bad in [
            "v",
            "ctrl",
            "ctrl+ctrl+v",
            "ctrl+a+b",
            "ctrl+F13",
            "ctrl+é",
            "ctrl+",
            "",
            "ctrl+ab",
        ] {
            assert!(normalize_shortcut(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn menu_items_shortcuts_and_overlays_are_checked() {
        let item: MenuSchema = serde_json::from_value(
            json!({"name": "regen", "title": "Regenerate", "command": "regen", "shortcut": "ctrl+r"}),
        )
        .unwrap();
        item.check_definition().unwrap();
        assert_eq!(item.keys().as_deref(), Some("Ctrl+R"));
        let mut bad = item.clone();
        bad.shortcut = Some("r".into());
        assert!(bad.check_definition().is_err());
        bad.shortcut = None;
        bad.title = " ".into();
        assert!(bad.check_definition().is_err());
        let shortcut: ShortcutSchema = serde_json::from_value(
            json!({"name": "go", "title": "Go", "keys": "alt+g", "command": "go"}),
        )
        .unwrap();
        shortcut.check_definition().unwrap();
        assert_eq!(shortcut.normalized().as_deref(), Some("Alt+G"));
        let mut overlay: OverlaySchema =
            serde_json::from_value(json!({"name": "chunks", "title": "Chunk borders"})).unwrap();
        overlay.check_definition().unwrap();
        assert_eq!(overlay.op(), "overlay.chunks");
        overlay.interval_ms = 5;
        assert!(overlay.check_definition().is_err());
    }

    #[test]
    fn an_overlay_answer_becomes_shapes() {
        let shapes = parse_shapes(&json!({
            "lines": [{"from": [0, 0, 0], "to": [1, 2, 3], "color": "#FF0000"}],
            "boxes": [{"min": [0, 0, 0], "max": [1, 1, 1], "color": [0, 1, 0, 0.5]}],
            "points": [{"at": [4, 5, 6]}]
        }))
        .unwrap();
        assert_eq!(shapes.len(), 3);
        assert_eq!(
            shapes[0],
            Shape::Line {
                from: [0.0; 3],
                to: [1.0, 2.0, 3.0],
                color: [1.0, 0.0, 0.0, 1.0]
            }
        );
        assert!(matches!(shapes[2], Shape::Point { size, .. } if (size - 0.25).abs() < 1e-6));
        assert!(parse_shapes(&json!({})).unwrap().is_empty());
        assert!(parse_shapes(&json!({"lines": 3})).is_err());
        assert!(parse_shapes(&json!({"lines": [{"from": [0, 0], "to": [1, 1, 1]}]})).is_err());
        assert!(
            parse_shapes(&json!({"boxes": [{"min": [0,0,0], "max": [1,1,1], "color": "red"}]}))
                .is_err()
        );
        let many: Vec<Value> = (0..MAX_SHAPES + 1)
            .map(|_| json!({"at": [0, 0, 0]}))
            .collect();
        assert!(parse_shapes(&json!({"points": many})).is_err());
    }
}
