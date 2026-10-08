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
    /// How far the pointer moved this frame along `x` or `y`; a directed
    /// binding answers one way only. Used by look actions.
    MouseDelta {
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
            InputBinding::MouseDelta { axis, direction } => {
                let suffix = match direction {
                    -1 => "-",
                    1 => "+",
                    _ => "",
                };
                format!("mouse:delta{axis}{suffix}")
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
            InputBinding::MouseDelta { axis, direction } => {
                *axis = axis.trim().to_lowercase();
                if axis != "x" && axis != "y" {
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
        if let Some(delta) = rest.strip_prefix("delta") {
            let (axis, direction) = if let Some(axis) = delta.strip_suffix('-') {
                (axis, -1)
            } else if let Some(axis) = delta.strip_suffix('+') {
                (axis, 1)
            } else {
                (delta, 0)
            };
            return (axis == "x" || axis == "y").then(|| InputBinding::MouseDelta {
                axis: axis.to_string(),
                direction,
            });
        }
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

/// What an action reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ActionType {
    /// Held, pressed and released, with an analog value.
    #[default]
    Button,
    /// One signed number, -1 to 1.
    Axis,
    /// Two numbers: a stick, four keys or a pointer.
    Vector2,
}

/// The four sides of a two-axis action. Each side takes its strongest
/// binding; the axis is right minus left and up minus down.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Composite {
    #[serde(default)]
    pub left: Vec<InputBinding>,
    #[serde(default)]
    pub right: Vec<InputBinding>,
    #[serde(default)]
    pub up: Vec<InputBinding>,
    #[serde(default)]
    pub down: Vec<InputBinding>,
}

/// Shapes an action's value after the device says it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "processor")]
pub enum Processor {
    /// Below `min` reads zero and `max` reads full, with the range between
    /// rescaled. Radial for a vector.
    DeadZone {
        min: f32,
        max: f32,
    },
    /// Sensitivity per axis.
    Scale {
        x: f32,
        y: f32,
    },
    Invert {
        x: bool,
        y: bool,
    },
    /// A vector longer than one is brought back to one.
    Normalize,
}

impl Processor {
    fn apply(&self, v: [f32; 2]) -> [f32; 2] {
        match self {
            Processor::DeadZone { min, max } => {
                let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
                if length <= *min || *max <= *min {
                    return [0.0; 2];
                }
                let scaled = ((length - min) / (max - min)).min(1.0);
                [v[0] / length * scaled, v[1] / length * scaled]
            }
            Processor::Scale { x, y } => [v[0] * x, v[1] * y],
            Processor::Invert { x, y } => {
                [if *x { -v[0] } else { v[0] }, if *y { -v[1] } else { v[1] }]
            }
            Processor::Normalize => {
                let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
                if length > 1.0 {
                    [v[0] / length, v[1] / length]
                } else {
                    v
                }
            }
        }
    }
}

fn is_button(kind: &ActionType) -> bool {
    *kind == ActionType::Button
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}

/// One named action and every binding that drives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputAction {
    pub name: String,
    #[serde(default)]
    pub bindings: Vec<InputBinding>,
    #[serde(default, skip_serializing_if = "is_button")]
    pub kind: ActionType,
    /// Where a [`ActionType::Vector2`] gets its sides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composite: Option<Composite>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processors: Vec<Processor>,
    /// The action map it belongs to; empty is always on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub map: String,
    /// The local player whose devices drive it: 0 reads the keyboard, the
    /// mouse and every pad; n reads only the nth pad.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub player: u8,
}

impl InputAction {
    pub fn new(name: &str, bindings: Vec<InputBinding>) -> Self {
        Self {
            name: normalize_action(name),
            bindings,
            kind: ActionType::Button,
            composite: None,
            processors: Vec::new(),
            map: String::new(),
            player: 0,
        }
    }

    /// A two-axis action from four sets of binding texts.
    pub fn vector2(name: &str, sides: [&[&str]; 4], processors: Vec<Processor>) -> Self {
        let side = |texts: &[&str]| -> Vec<InputBinding> {
            texts.iter().filter_map(|t| parse_binding(t)).collect()
        };
        let mut action = Self::new(name, Vec::new());
        action.kind = ActionType::Vector2;
        action.composite = Some(Composite {
            left: side(sides[0]),
            right: side(sides[1]),
            up: side(sides[2]),
            down: side(sides[3]),
        });
        action.processors = processors;
        action
    }

    /// Whether this action's bindings use any of `bindings` or its sides.
    pub fn all_bindings(&self) -> impl Iterator<Item = &InputBinding> {
        let sides = self
            .composite
            .iter()
            .flat_map(|c| c.left.iter().chain(&c.right).chain(&c.up).chain(&c.down));
        self.bindings.iter().chain(sides)
    }

    fn normalize(&mut self) {
        self.name = normalize_action(&self.name);
        self.map = self.map.trim().to_string();
        let clean = |list: Vec<InputBinding>| {
            let mut seen = HashSet::new();
            let mut kept = Vec::new();
            for binding in list {
                let Some(binding) = binding.normalized() else {
                    continue;
                };
                if seen.insert(binding.text()) {
                    kept.push(binding);
                }
            }
            kept
        };
        self.bindings = clean(std::mem::take(&mut self.bindings));
        if let Some(composite) = &mut self.composite {
            composite.left = clean(std::mem::take(&mut composite.left));
            composite.right = clean(std::mem::take(&mut composite.right));
            composite.up = clean(std::mem::take(&mut composite.up));
            composite.down = clean(std::mem::take(&mut composite.down));
        }
        if self.kind != ActionType::Vector2 {
            self.composite = None;
        }
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

    /// The standard player mappings: Move, Look, Jump, Sprint, Crouch and
    /// Interact for keyboard and mouse, gamepad and nothing else. Existing
    /// actions of the same names are left alone. Returns the names it added.
    pub fn add_player_actions(&mut self) -> Vec<String> {
        let button = |name: &str, texts: &[&str]| {
            let mut action = InputAction::new(
                name,
                texts.iter().filter_map(|t| parse_binding(t)).collect(),
            );
            action.map = "Gameplay".to_string();
            action
        };
        let mut wanted = vec![
            InputAction::vector2(
                "Move",
                [
                    &["a", "left arrow", "gamepad:dpadleft", "gamepad:leftstickx-"],
                    &[
                        "d",
                        "right arrow",
                        "gamepad:dpadright",
                        "gamepad:leftstickx+",
                    ],
                    &["w", "up arrow", "gamepad:dpadup", "gamepad:leftsticky+"],
                    &["s", "down arrow", "gamepad:dpaddown", "gamepad:leftsticky-"],
                ],
                vec![
                    Processor::DeadZone {
                        min: 0.15,
                        max: 0.95,
                    },
                    Processor::Normalize,
                ],
            ),
            InputAction::vector2(
                "Look",
                [
                    &["mouse:deltax-", "gamepad:rightstickx-"],
                    &["mouse:deltax+", "gamepad:rightstickx+"],
                    &["mouse:deltay+", "gamepad:rightsticky+"],
                    &["mouse:deltay-", "gamepad:rightsticky-"],
                ],
                vec![Processor::DeadZone { min: 0.1, max: 1.0 }],
            ),
            button("Jump", &["space", "gamepad:south"]),
            button("Sprint", &["shift", "gamepad:leftthumb"]),
            button("Crouch", &["control", "c", "gamepad:east"]),
            button("Interact", &["e", "gamepad:west"]),
        ];
        let mut added = Vec::new();
        for mut action in wanted.drain(..) {
            if action.map.is_empty() {
                action.map = "Gameplay".to_string();
            }
            if self.find(&action.name).is_none() {
                added.push(action.name.clone());
                self.actions.push(action);
            }
        }
        added
    }

    /// The nth local player's copy of a player action: the same shape,
    /// named `Move P2` and fed only by pad n-1 (`player` is 1-based past the
    /// first).
    pub fn player_copy(&self, name: &str, player: u8) -> Option<InputAction> {
        let base = self.find(name)?;
        let mut copy = base.clone();
        copy.name = format!("{} P{}", base.name, player + 1);
        copy.player = player;
        let pad_only = |list: &mut Vec<InputBinding>| {
            list.retain(|b| {
                matches!(
                    b,
                    InputBinding::GamepadButton { .. } | InputBinding::GamepadAxis { .. }
                )
            })
        };
        pad_only(&mut copy.bindings);
        if let Some(c) = &mut copy.composite {
            pad_only(&mut c.left);
            pad_only(&mut c.right);
            pad_only(&mut c.up);
            pad_only(&mut c.down);
        }
        Some(copy)
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
        Ok(trimmed)
    }

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
    /// How far the pointer moved this frame, right and up positive.
    pub mouse_delta: [f32; 2],
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
            InputBinding::MouseDelta { axis, direction } => {
                let value = self.mouse_delta[usize::from(axis == "y")];
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
            InputBinding::MouseDelta { .. } => self.binding_value(binding).abs() > 0.0,
            _ => self.binding_value(binding) > 0.5,
        }
    }

    /// An action's analog value: the strongest binding wins, so opposing
    /// halves of one stick never cancel inside one action.
    pub fn action_value(&self, action: &InputAction) -> f32 {
        if action.kind == ActionType::Vector2 && action.composite.is_some() {
            let v = self.action_vector(action);
            return (v[0] * v[0] + v[1] * v[1]).sqrt();
        }
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
        if action.kind == ActionType::Vector2 {
            let v = self.action_vector(action);
            return v[0].abs() > AXIS_DEADZONE || v[1].abs() > AXIS_DEADZONE;
        }
        action
            .bindings
            .iter()
            .any(|binding| self.binding_held(binding))
    }

    /// The strongest value among `bindings`.
    fn strongest(&self, bindings: &[InputBinding]) -> f32 {
        bindings
            .iter()
            .map(|binding| self.binding_value(binding))
            .fold(
                0.0f32,
                |best, v| if v.abs() > best.abs() { v } else { best },
            )
    }

    /// A two-axis action's value after its processors. A button or axis
    /// action reads as (value, 0).
    pub fn action_vector(&self, action: &InputAction) -> [f32; 2] {
        let mut v = match (&action.kind, &action.composite) {
            (ActionType::Vector2, Some(c)) => [
                self.strongest(&c.right) - self.strongest(&c.left),
                self.strongest(&c.up) - self.strongest(&c.down),
            ],
            _ => [self.action_value(action), 0.0],
        };
        for processor in &action.processors {
            v = processor.apply(v);
        }
        v
    }
}

thread_local! {
    static DISABLED_MAPS: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
}

/// Turns an action map on or off for the rest of the run. An action whose
/// map is off reads as released.
pub fn set_map_enabled(map: &str, on: bool) {
    let key = map.trim().to_lowercase();
    DISABLED_MAPS.with(|maps| {
        let mut maps = maps.borrow_mut();
        if on {
            maps.remove(&key);
        } else {
            maps.insert(key);
        }
    });
}

/// Whether an action map is on. The empty map is always on.
pub fn map_enabled(map: &str) -> bool {
    let key = map.trim().to_lowercase();
    key.is_empty() || DISABLED_MAPS.with(|maps| !maps.borrow().contains(&key))
}

/// Puts every map back on, as a fresh run starts.
pub fn reset_maps() {
    DISABLED_MAPS.with(|maps| maps.borrow_mut().clear());
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
    /// A two-axis action's value; (value, 0) for any other.
    #[serde(default)]
    pub vector: [f32; 2],
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
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

    #[test]
    fn four_keys_make_a_unit_vector() {
        let mut config = InputConfig::default();
        config.add_player_actions();
        let move_action = config.find("move").expect("Move").clone();
        let mut live = LiveInput::default();
        live.keys.insert("w".to_string());
        live.keys.insert("d".to_string());
        let v = live.action_vector(&move_action);
        let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((length - 1.0).abs() < 1e-4, "{v:?}");
        assert!(v[0] > 0.0 && v[1] > 0.0);
        assert!(live.action_held(&move_action));
        assert!((live.action_value(&move_action) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn opposing_keys_cancel() {
        let mut config = InputConfig::default();
        config.add_player_actions();
        let move_action = config.find("move").unwrap();
        let mut live = LiveInput::default();
        live.keys.insert("a".to_string());
        live.keys.insert("d".to_string());
        assert_eq!(live.action_vector(move_action), [0.0, 0.0]);
    }

    #[test]
    fn a_stick_inside_the_dead_zone_reads_zero_and_rescales_outside_it() {
        let mut config = InputConfig::default();
        config.add_player_actions();
        let move_action = config.find("move").unwrap();
        let mut live = LiveInput::default();
        live.axes.insert("leftstickx".to_string(), 0.1);
        assert_eq!(live.action_vector(move_action), [0.0, 0.0]);
        live.axes.insert("leftstickx".to_string(), 0.55);
        let v = live.action_vector(move_action);
        assert!((v[0] - 0.5).abs() < 1e-4, "(0.55-0.15)/(0.95-0.15): {v:?}");
        live.axes.insert("leftstickx".to_string(), 1.0);
        assert!((live.action_vector(move_action)[0] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn look_reads_the_pointer_and_scales_with_sensitivity() {
        let mut config = InputConfig::default();
        config.add_player_actions();
        let mut look = config.find("look").unwrap().clone();
        let mut live = LiveInput::default();
        live.mouse_delta = [0.5, -0.25];
        let v = live.action_vector(&look);
        assert!(v[0] > 0.0 && v[1] < 0.0, "{v:?}");
        look.processors = vec![
            Processor::Scale { x: 2.0, y: 2.0 },
            Processor::Invert { x: false, y: true },
        ];
        let v = live.action_vector(&look);
        assert_eq!(v, [1.0, 0.5]);
    }

    #[test]
    fn player_actions_are_added_once_and_leave_edits_alone() {
        let mut config = InputConfig::starter();
        let added = config.add_player_actions();
        assert!(added.contains(&"Move".to_string()));
        assert!(
            !added.contains(&"Jump".to_string()),
            "starter already had Jump"
        );
        config.find_mut("Move").unwrap().processors.clear();
        assert!(config.add_player_actions().is_empty());
        assert!(config.find("Move").unwrap().processors.is_empty());
    }

    #[test]
    fn a_second_player_reads_pads_only() {
        let mut config = InputConfig::default();
        config.add_player_actions();
        let copy = config.player_copy("Move", 1).unwrap();
        assert_eq!(copy.name, "Move P2");
        assert_eq!(copy.player, 1);
        let c = copy.composite.unwrap();
        assert!(c.left.iter().all(|b| matches!(
            b,
            InputBinding::GamepadAxis { .. } | InputBinding::GamepadButton { .. }
        )));
    }

    #[test]
    fn maps_switch_off_for_the_run() {
        reset_maps();
        assert!(map_enabled("Gameplay"));
        set_map_enabled("gameplay", false);
        assert!(!map_enabled("Gameplay"));
        assert!(map_enabled(""), "no map is always on");
        reset_maps();
        assert!(map_enabled("Gameplay"));
    }

    #[test]
    fn old_documents_still_load_and_new_fields_stay_out_of_buttons() {
        let old: InputAction =
            serde_json::from_str(r#"{"name":"Jump","bindings":[{"binding":"Key","key":"space"}]}"#)
                .unwrap();
        assert_eq!(old.kind, ActionType::Button);
        let text = serde_json::to_string(&old).unwrap();
        assert!(
            !text.contains("kind") && !text.contains("composite") && !text.contains("map"),
            "{text}"
        );
    }

    #[test]
    fn pointer_bindings_round_trip() {
        for text in ["mouse:deltax", "mouse:deltay-", "mouse:deltax+"] {
            assert_eq!(parse_binding(text).unwrap().text(), text);
        }
        assert!(parse_binding("mouse:deltaz").is_none());
    }
}
