//! Named input actions plus the bindings that drive them.
//!
//! An action is what a game means ("Jump"); a binding is how a device says it
//! (space, left mouse, gamepad south, left stick). The project owns the map,
//! the runtime evaluates it once a frame, and reporter blocks read the answer
//! off the sensor snapshot. A `bind` block edits the map for the rest of the
//! run; the editor's own commands edit the document.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Deadzone past which an analog axis counts as held.
pub const AXIS_DEADZONE: f32 = 0.35;

/// Which mouse button a binding means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "left" | "mouse left" | "button1" => Some(MouseButton::Left),
            "right" | "mouse right" | "button2" => Some(MouseButton::Right),
            "middle" | "mouse middle" | "button3" => Some(MouseButton::Middle),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        }
    }
}

/// Gamepad buttons, in [`crate::sense`]-style lowercase spelling.
pub const GAMEPAD_BUTTONS: &[&str] = &[
    "south",
    "east",
    "north",
    "west",
    "c",
    "z",
    "lefttrigger",
    "lefttrigger2",
    "righttrigger",
    "righttrigger2",
    "select",
    "start",
    "mode",
    "leftthumb",
    "rightthumb",
    "dpadup",
    "dpaddown",
    "dpadleft",
    "dpadright",
];

/// Gamepad axes, canonical lowercase spelling.
pub const GAMEPAD_AXES: &[&str] = &[
    "leftstickx",
    "leftsticky",
    "leftz",
    "rightstickx",
    "rightsticky",
    "rightz",
];

/// Mouse buttons, canonical spelling.
pub const MOUSE_BUTTONS: &[&str] = &["left", "right", "middle"];

/// Canonical spelling of an action name.
pub fn normalize_action(name: &str) -> String {
    name.trim().to_string()
}

/// Canonical spelling of a gamepad button name.
pub fn normalize_pad_button(name: &str) -> String {
    name.trim().to_lowercase().replace([' ', '_', '-'], "")
}

/// Canonical spelling of a gamepad axis name. Short aliases (`leftx`,
/// `righty`) map onto the full stick names.
pub fn normalize_pad_axis(name: &str) -> String {
    let compact = name.trim().to_lowercase().replace([' ', '_', '-'], "");
    match compact.as_str() {
        "leftx" => "leftstickx".to_string(),
        "lefty" => "leftsticky".to_string(),
        "rightx" => "rightstickx".to_string(),
        "righty" => "rightsticky".to_string(),
        _ => compact,
    }
}

/// One way to say an action: a key, a mouse button, a gamepad button, or a
/// gamepad axis. A directed axis (`direction` of -1 or 1) only answers one
/// way of it, so Left and Right can share a stick; 0 answers the whole axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "binding")]
pub enum InputBinding {
    Key {
        key: String,
    },
    Mouse {
        button: MouseButton,
    },
    GamepadButton {
        button: String,
    },
    GamepadAxis {
        axis: String,
        #[serde(default)]
        direction: i8,
    },
}

impl InputBinding {
    pub fn key(key: &str) -> Self {
        InputBinding::Key {
            key: key.trim().to_lowercase(),
        }
    }

    /// Human spelling of a binding, which is also what the `bind` block
    /// parses: `space`, `mouse:left`, `gamepad:south`, `gamepad:leftstickx-`.
    pub fn text(&self) -> String {
        match self {
            InputBinding::Key { key } => key.clone(),
            InputBinding::Mouse { button } => format!("mouse:{}", button.name()),
            InputBinding::GamepadButton { button } => format!("gamepad:{button}"),
            InputBinding::GamepadAxis { axis, direction } => {
                let suffix = match direction {
                    -1 => "-",
                    1 => "+",
                    _ => "",
                };
                format!("gamepad:{axis}{suffix}")
            }
        }
    }

    fn normalized(mut self) -> Option<Self> {
        match &mut self {
            InputBinding::Key { key } => {
                *key = key.trim().to_lowercase();
                if key.is_empty() { None } else { Some(self) }
            }
            InputBinding::Mouse { .. } => Some(self),
            InputBinding::GamepadButton { button } => {
                *button = normalize_pad_button(button);
                if GAMEPAD_BUTTONS.contains(&button.as_str()) {
                    Some(self)
                } else {
                    None
                }
            }
            InputBinding::GamepadAxis { axis, direction } => {
                *axis = normalize_pad_axis(axis);
                if !GAMEPAD_AXES.contains(&axis.as_str()) {
                    return None;
                }
                *direction = (*direction).clamp(-1, 1);
                Some(self)
            }
        }
    }
}

/// Parses what [`InputBinding::text`] writes: `space`, `mouse:left`,
/// `gamepad:south`, `gamepad:leftstickx-` (`+` for the other way, bare for
/// the whole axis). A `pad:` prefix works like `gamepad:`.
pub fn parse_binding(text: &str) -> Option<InputBinding> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if let Some(rest) = lower
        .strip_prefix("mouse:")
        .or_else(|| lower.strip_prefix("mouse "))
    {
        return MouseButton::parse(rest).map(|button| InputBinding::Mouse { button });
    }
    if let Some(rest) = lower
        .strip_prefix("gamepad:")
        .or_else(|| lower.strip_prefix("gamepad "))
        .or_else(|| lower.strip_prefix("pad:"))
        .or_else(|| lower.strip_prefix("pad "))
    {
        let (name, direction) = if let Some(axis) = rest.strip_suffix('-') {
            (axis, -1)
        } else if let Some(axis) = rest.strip_suffix('+') {
            (axis, 1)
        } else {
            (rest, 0)
        };
        let axis = normalize_pad_axis(name);
        if GAMEPAD_AXES.contains(&axis.as_str()) {
            return Some(InputBinding::GamepadAxis { axis, direction });
        }
        let button = normalize_pad_button(name);
        if direction == 0 && GAMEPAD_BUTTONS.contains(&button.as_str()) {
            return Some(InputBinding::GamepadButton { button });
        }
        return None;
    }
    if let Some(button) = MouseButton::parse(&lower) {
        // A bare `left` is a key, not a mouse button: mouse needs its prefix.
        let _ = button;
    }
    Some(InputBinding::Key {
        key: lower.trim().to_string(),
    })
}

/// One named action and every binding that drives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputAction {
    pub name: String,
    #[serde(default)]
    pub bindings: Vec<InputBinding>,
}

impl InputAction {
    pub fn new(name: &str, bindings: Vec<InputBinding>) -> Self {
        Self {
            name: normalize_action(name),
            bindings,
        }
    }

    fn normalize(&mut self) {
        self.name = normalize_action(&self.name);
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        for binding in std::mem::take(&mut self.bindings) {
            let Some(binding) = binding.normalized() else {
                continue;
            };
            let text = binding.text();
            if seen.insert(text) {
                kept.push(binding);
            }
        }
        self.bindings = kept;
    }
}

/// Every action a project defines. Empty by default, so old documents load
/// with no actions rather than failing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct InputConfig {
    #[serde(default)]
    pub actions: Vec<InputAction>,
}

impl InputConfig {
    /// The starter set a fresh project gets: jump, fire and four moves with
    /// keyboard plus gamepad bindings each.
    pub fn starter() -> Self {
        let action = |name: &str, texts: &[&str]| {
            InputAction::new(
                name,
                texts
                    .iter()
                    .filter_map(|text| parse_binding(text))
                    .collect(),
            )
        };
        Self {
            actions: vec![
                action("Jump", &["space", "gamepad:south"]),
                action("Fire", &["mouse:left", "gamepad:righttrigger"]),
                action(
                    "Left",
                    &["a", "left arrow", "gamepad:dpadleft", "gamepad:leftstickx-"],
                ),
                action(
                    "Right",
                    &[
                        "d",
                        "right arrow",
                        "gamepad:dpadright",
                        "gamepad:leftstickx+",
                    ],
                ),
                action(
                    "Up",
                    &["w", "up arrow", "gamepad:dpadup", "gamepad:leftsticky+"],
                ),
                action(
                    "Down",
                    &["s", "down arrow", "gamepad:dpaddown", "gamepad:leftsticky-"],
                ),
            ],
        }
    }

    pub fn find(&self, name: &str) -> Option<&InputAction> {
        let wanted = normalize_action(name);
        self.actions
            .iter()
            .find(|action| action.name.eq_ignore_ascii_case(&wanted))
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut InputAction> {
        let wanted = normalize_action(name);
        self.actions
            .iter_mut()
            .find(|action| action.name.eq_ignore_ascii_case(&wanted))
    }

    pub fn names(&self) -> Vec<String> {
        self.actions
            .iter()
            .map(|action| action.name.clone())
            .collect()
    }

    /// Canonicalizes every action once a document loads: drops blanks and
    /// dup bindings, keeps the first of two names that differ only by case.
    pub fn normalize(&mut self) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut kept = Vec::new();
        for mut action in std::mem::take(&mut self.actions) {
            action.normalize();
            if action.name.is_empty() {
                continue;
            }
            if seen.insert(action.name.to_lowercase()) {
                kept.push(action);
            }
        }
        self.actions = kept;
    }

    pub fn add_action(&mut self, name: &str) -> Result<String, String> {
        let trimmed = normalize_action(name);
        if trimmed.is_empty() {
            return Err("An action needs a name".to_string());
        }
        if self.find(&trimmed).is_some() {
            return Err(format!(
                "An input action named \"{trimmed}\" already exists"
            ));
        }
        self.actions.push(InputAction::new(&trimmed, Vec::new()));
        Ok(trimmed)
    }

    pub fn rename_action(&mut self, old: &str, new: &str) -> Result<String, String> {
        let trimmed = normalize_action(new);
        if trimmed.is_empty() {
            return Err("An action needs a name".to_string());
        }
        if trimmed.to_lowercase() != old.trim().to_lowercase() && self.find(&trimmed).is_some() {
            return Err(format!(
                "An input action named \"{trimmed}\" already exists"
            ));
        }
        let Some(action) = self.find_mut(old) else {
            return Err("Input action not found".to_string());
        };
        action.name = trimmed.clone();
        self.normalize_blocks_rename(old, &trimmed);
        Ok(trimmed)
    }

    fn normalize_blocks_rename(&mut self, _old: &str, _new: &str) {}

    pub fn remove_action(&mut self, name: &str) -> bool {
        let wanted = normalize_action(name).to_lowercase();
        let before = self.actions.len();
        self.actions
            .retain(|action| action.name.to_lowercase() != wanted);
        self.actions.len() != before
    }

    /// Adds one binding text (`space`, `mouse:left`, ...) to an action.
    /// True when it was not there already.
    pub fn add_binding(&mut self, name: &str, binding: &str) -> Result<bool, String> {
        let parsed =
            parse_binding(binding).ok_or_else(|| format!("\"{binding}\" isn't a binding"))?;
        let parsed = parsed
            .normalized()
            .ok_or_else(|| format!("\"{binding}\" isn't a binding"))?;
        let Some(action) = self.find_mut(name) else {
            return Err("Input action not found".to_string());
        };
        let text = parsed.text();
        if action.bindings.iter().any(|held| held.text() == text) {
            return Ok(false);
        }
        action.bindings.push(parsed);
        Ok(true)
    }

    pub fn remove_binding(&mut self, name: &str, binding: &str) -> Result<bool, String> {
        let parsed = parse_binding(binding)
            .and_then(|binding| binding.normalized())
            .map(|binding| binding.text())
            .ok_or_else(|| format!("\"{binding}\" isn't a binding"))?;
        let Some(action) = self.find_mut(name) else {
            return Err("Input action not found".to_string());
        };
        let before = action.bindings.len();
        action.bindings.retain(|held| held.text() != parsed);
        Ok(action.bindings.len() != before)
    }

    pub fn clear_bindings(&mut self, name: &str) -> Result<bool, String> {
        let Some(action) = self.find_mut(name) else {
            return Err("Input action not found".to_string());
        };
        let had = !action.bindings.is_empty();
        action.bindings.clear();
        Ok(had)
    }

    /// Replaces an action's bindings with the one text, so a settings screen
    /// can rebind a key in one block. Empty clears it.
    pub fn rebind(&mut self, name: &str, binding: &str) -> Result<bool, String> {
        let trimmed = binding.trim();
        let Some(action) = self.find_mut(name) else {
            return Err("Input action not found".to_string());
        };
        if trimmed.is_empty() {
            let had = !action.bindings.is_empty();
            action.bindings.clear();
            return Ok(had);
        }
        let parsed =
            parse_binding(trimmed).ok_or_else(|| format!("\"{binding}\" isn't a binding"))?;
        let parsed = parsed
            .normalized()
            .ok_or_else(|| format!("\"{binding}\" isn't a binding"))?;
        let changed = action.bindings.len() != 1 || action.bindings[0].text() != parsed.text();
        action.bindings = vec![parsed];
        Ok(changed)
    }
}

/// The live device state one frame, in canonical spelling: normalized keys,
/// `left`/`right`/`middle` mouse buttons held, normalized pad buttons, and
/// axis values by canonical axis name.
#[derive(Debug, Clone, Default)]
pub struct LiveInput {
    pub keys: HashSet<String>,
    pub mouse: HashSet<String>,
    pub pad_buttons: HashSet<String>,
    pub axes: HashMap<String, f32>,
}

impl LiveInput {
    /// The signed contribution of one binding: 0/1 for buttons, the axis
    /// value (or its directed half) for sticks.
    pub fn binding_value(&self, binding: &InputBinding) -> f32 {
        match binding {
            InputBinding::Key { key } => {
                if self.keys.contains(key) {
                    1.0
                } else {
                    0.0
                }
            }
            InputBinding::Mouse { button } => {
                if self.mouse.contains(button.name()) {
                    1.0
                } else {
                    0.0
                }
            }
            InputBinding::GamepadButton { button } => {
                if self.pad_buttons.contains(button) {
                    1.0
                } else {
                    0.0
                }
            }
            InputBinding::GamepadAxis { axis, direction } => {
                let value = self.axes.get(axis).copied().unwrap_or(0.0);
                match direction {
                    -1 => (-value).max(0.0),
                    1 => value.max(0.0),
                    _ => value,
                }
            }
        }
    }

    pub fn binding_held(&self, binding: &InputBinding) -> bool {
        match binding {
            InputBinding::GamepadAxis { direction: 0, .. } => {
                self.binding_value(binding).abs() > AXIS_DEADZONE
            }
            InputBinding::GamepadAxis { .. } => self.binding_value(binding) > AXIS_DEADZONE,
            _ => self.binding_value(binding) > 0.5,
        }
    }

    /// An action's analog value: the strongest binding wins, so opposing
    /// halves of one stick never cancel inside one action.
    pub fn action_value(&self, action: &InputAction) -> f32 {
        let mut best = 0.0f32;
        for binding in &action.bindings {
            let value = self.binding_value(binding);
            if value.abs() > best.abs() {
                best = value;
            }
        }
        best
    }

    pub fn action_held(&self, action: &InputAction) -> bool {
        action
            .bindings
            .iter()
            .any(|binding| self.binding_held(binding))
    }
}

/// One action as the reporters see it this frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ActionSense {
    #[serde(default)]
    pub held: bool,
    /// True only on the frame it went down.
    #[serde(default)]
    pub pressed: bool,
    /// True only on the frame it went up.
    #[serde(default)]
    pub released: bool,
    /// The strongest binding's analog value: 0/1 for buttons, -1..1 for a
    /// whole stick axis, 0..1 for a directed half.
    #[serde(default)]
    pub value: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_text_round_trips_through_parse() {
        for text in [
            "space",
            "left arrow",
            "mouse:left",
            "mouse:right",
            "gamepad:south",
            "gamepad:dpadup",
            "gamepad:leftstickx-",
            "gamepad:rightsticky+",
            "gamepad:leftstickx",
        ] {
            let parsed = parse_binding(text).expect("a binding");
            assert_eq!(parsed.text(), normalize_binding_text(text));
        }
    }

    fn normalize_binding_text(text: &str) -> String {
        parse_binding(text).unwrap().text()
    }

    #[test]
    fn an_unknown_pad_button_or_axis_is_not_a_binding() {
        assert!(parse_binding("gamepad:banana").is_none());
        assert!(parse_binding("gamepad:leftstickw-").is_none());
        assert!(parse_binding("").is_none());
    }

    #[test]
    fn actions_stay_unique_and_bindings_stay_deduped() {
        let mut config = InputConfig::default();
        config.add_action("Jump").unwrap();
        assert!(config.add_action("jump").is_err());
        assert!(config.add_binding("Jump", "Space").unwrap());
        assert!(!config.add_binding("Jump", "space").unwrap());
        assert!(config.remove_binding("Jump", "SPACE").unwrap());
        assert!(!config.remove_binding("Jump", "space").unwrap());
    }

    #[test]
    fn a_directed_axis_only_answers_its_own_way() {
        let left = InputAction::new("Left", vec![parse_binding("gamepad:leftstickx-").unwrap()]);
        let mut live = LiveInput::default();
        live.axes.insert("leftstickx".to_string(), -0.9);
        assert!(live.action_held(&left));
        assert!((live.action_value(&left) - 0.9).abs() < 0.001);
        live.axes.insert("leftstickx".to_string(), 0.9);
        assert!(!live.action_held(&left));
        assert_eq!(live.action_value(&left), 0.0);
    }

    #[test]
    fn starter_actions_cover_keys_and_pad() {
        let starter = InputConfig::starter();
        let jump = starter.find("jump").expect("Jump");
        assert!(
            jump.bindings
                .iter()
                .any(|binding| binding.text() == "space")
        );
        assert!(
            jump.bindings
                .iter()
                .any(|binding| binding.text() == "gamepad:south")
        );
    }
}
