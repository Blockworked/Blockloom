//! The screen-space UI a game builds out of blocks: what kinds of element
//! there are, where one sits, and which of its properties a block can change.
//!
//! An element is named by an id string the project invents (`"pause-menu"`,
//! `"resume-btn"`). `show` creates or updates one, `set` changes a property,
//! `hide` and `delete` take it away. Re-showing an id that already exists
//! updates it in place, so HUD code can run every frame without piling up.
//!
//! Nothing here touches a renderer: the runtime's `overlay` module turns a
//! [`UiElement`] into Bevy UI nodes, and the codegen ABI carries the same
//! fields as plain numbers and strings.

use crate::value::Evaluated;
use serde::{Deserialize, Serialize};

/// What kind of element a `show` block makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiKind {
    /// A vertical stack its children flow down, optionally modal.
    #[default]
    Panel,
    /// Text. The HUD workhorse.
    Label,
    Button,
    Image,
    /// Basic keyboard input.
    Input,
    Slider,
    Toggle,
    /// A bounded vertical stack whose children can be scrolled.
    List,
    VerticalBox,
    HorizontalBox,
    Grid,
    Canvas,
    WrapBox,
    SizeBox,
    Spacer,
    Progress,
    RadialProgress,
    ListView,
    Tabs,
    Select,
    Scrollbar,
    RichText,
    Tooltip,
}

impl UiKind {
    pub fn scrollable(self) -> bool {
        matches!(self, Self::List | Self::ListView)
    }
    pub fn selectable(self) -> bool {
        matches!(self, Self::ListView | Self::Tabs | Self::Select)
    }
    pub fn focusable(self) -> bool {
        self.is_input() || self == Self::Button
    }

    pub fn index(self) -> usize {
        match self {
            UiKind::Panel => 0,
            UiKind::Label => 1,
            UiKind::Button => 2,
            UiKind::Image => 3,
            UiKind::Input => 4,
            UiKind::Slider => 5,
            UiKind::Toggle => 6,
            UiKind::List => 7,
            UiKind::VerticalBox => 8,
            UiKind::HorizontalBox => 9,
            UiKind::Grid => 10,
            UiKind::Canvas => 11,
            UiKind::WrapBox => 12,
            UiKind::SizeBox => 13,
            UiKind::Spacer => 14,
            UiKind::Progress => 15,
            UiKind::RadialProgress => 16,
            UiKind::ListView => 17,
            UiKind::Tabs => 18,
            UiKind::Select => 19,
            UiKind::Scrollbar => 20,
            UiKind::RichText => 21,
            UiKind::Tooltip => 22,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => UiKind::Label,
            2 => UiKind::Button,
            3 => UiKind::Image,
            4 => UiKind::Input,
            5 => UiKind::Slider,
            6 => UiKind::Toggle,
            7 => UiKind::List,
            8 => UiKind::VerticalBox,
            9 => UiKind::HorizontalBox,
            10 => UiKind::Grid,
            11 => UiKind::Canvas,
            12 => UiKind::WrapBox,
            13 => UiKind::SizeBox,
            14 => UiKind::Spacer,
            15 => UiKind::Progress,
            16 => UiKind::RadialProgress,
            17 => UiKind::ListView,
            18 => UiKind::Tabs,
            19 => UiKind::Select,
            20 => UiKind::Scrollbar,
            21 => UiKind::RichText,
            22 => UiKind::Tooltip,

            _ => UiKind::Panel,
        }
    }

    /// True for the three that a person can change, and so the three that
    /// `when (id) changed` and `value of (id)` mean anything for.
    pub fn is_input(self) -> bool {
        matches!(
            self,
            UiKind::Input
                | UiKind::Slider
                | UiKind::Toggle
                | UiKind::Select
                | UiKind::Tabs
                | UiKind::ListView
                | UiKind::Scrollbar
        )
    }
}

/// A set of defaults for elements that have not had the matching property
/// written. Per-element styling always wins over the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiTheme {
    #[default]
    Dark,
    Light,
    HighContrast,
}

impl UiTheme {
    pub const ALL: &'static [UiTheme] = &[UiTheme::Dark, UiTheme::Light, UiTheme::HighContrast];

    pub fn name(self) -> &'static str {
        match self {
            UiTheme::Dark => "Dark",
            UiTheme::Light => "Light",
            UiTheme::HighContrast => "HighContrast",
        }
    }

    pub fn index(self) -> usize {
        match self {
            UiTheme::Dark => 0,
            UiTheme::Light => 1,
            UiTheme::HighContrast => 2,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => UiTheme::Light,
            2 => UiTheme::HighContrast,
            _ => UiTheme::Dark,
        }
    }
}

/// Which corner, edge or centre of the window a top-level element hangs off.
/// Its x/y offset is measured from there, so a window resize recomputes for
/// free. Ignored for an element with a parent, which flows after its
/// siblings instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiAnchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl UiAnchor {
    pub fn index(self) -> usize {
        match self {
            UiAnchor::TopLeft => 0,
            UiAnchor::Top => 1,
            UiAnchor::TopRight => 2,
            UiAnchor::Left => 3,
            UiAnchor::Center => 4,
            UiAnchor::Right => 5,
            UiAnchor::BottomLeft => 6,
            UiAnchor::Bottom => 7,
            UiAnchor::BottomRight => 8,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => UiAnchor::TopLeft,
            1 => UiAnchor::Top,
            2 => UiAnchor::TopRight,
            3 => UiAnchor::Left,
            5 => UiAnchor::Right,
            6 => UiAnchor::BottomLeft,
            7 => UiAnchor::Bottom,
            8 => UiAnchor::BottomRight,
            _ => UiAnchor::Center,
        }
    }

    /// How far along each axis the anchor sits, as a fraction of the window:
    /// `0.0` is the left or top edge, `1.0` the right or bottom.
    pub fn fractions(self) -> [f32; 2] {
        let x = match self {
            UiAnchor::TopLeft | UiAnchor::Left | UiAnchor::BottomLeft => 0.0,
            UiAnchor::Top | UiAnchor::Center | UiAnchor::Bottom => 0.5,
            UiAnchor::TopRight | UiAnchor::Right | UiAnchor::BottomRight => 1.0,
        };
        let y = match self {
            UiAnchor::TopLeft | UiAnchor::Top | UiAnchor::TopRight => 0.0,
            UiAnchor::Left | UiAnchor::Center | UiAnchor::Right => 0.5,
            UiAnchor::BottomLeft | UiAnchor::Bottom | UiAnchor::BottomRight => 1.0,
        };
        [x, y]
    }
}

/// One property `set [prop] of (id) to (v)` can write. The list is shared by
/// every kind: a property that means nothing for one (a `min` on a label) is
/// simply ignored rather than being an error, since a HUD that rebuilds
/// itself shouldn't have to know which is which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiProp {
    #[default]
    Text,
    TextColor,
    TextSize,
    Background,
    Width,
    Height,
    Visible,
    CornerRadius,
    Padding,
    Modal,
    Min,
    Max,
    Value,
    /// A slider's granularity: what its value is rounded to, measured from
    /// its `min`. Zero means it slides continuously.
    Step,
    /// Which characters a text input takes - [`UiAllow`]'s spelling.
    Allow,
    /// How many characters a text input holds. Zero means no ceiling.
    MaxLength,
    Layout,
    Style,
    Bind,
    Items,
    Scroll,
    SelectedIndex,
    Theme,
    Enabled,
    Tooltip,
    WorldActor,
    Transition,
}

impl UiProp {
    /// Every property, in the order the block's dropdown offers them.
    pub const ALL: &'static [UiProp] = &[
        UiProp::Text,
        UiProp::TextColor,
        UiProp::TextSize,
        UiProp::Background,
        UiProp::Width,
        UiProp::Height,
        UiProp::Visible,
        UiProp::CornerRadius,
        UiProp::Padding,
        UiProp::Modal,
        UiProp::Min,
        UiProp::Max,
        UiProp::Value,
        UiProp::Step,
        UiProp::Allow,
        UiProp::MaxLength,
        UiProp::Layout,
        UiProp::Style,
        UiProp::Bind,
        UiProp::Items,
        UiProp::Scroll,
        UiProp::SelectedIndex,
        UiProp::Theme,
        UiProp::Enabled,
        UiProp::Tooltip,
        UiProp::WorldActor,
        UiProp::Transition,
    ];

    /// The wire name, which is also what the ABI carries and what the
    /// frontend's dropdown stores.
    pub fn name(self) -> &'static str {
        match self {
            UiProp::Text => "Text",
            UiProp::TextColor => "TextColor",
            UiProp::TextSize => "TextSize",
            UiProp::Background => "Background",
            UiProp::Width => "Width",
            UiProp::Height => "Height",
            UiProp::Visible => "Visible",
            UiProp::CornerRadius => "CornerRadius",
            UiProp::Padding => "Padding",
            UiProp::Modal => "Modal",
            UiProp::Min => "Min",
            UiProp::Max => "Max",
            UiProp::Value => "Value",
            UiProp::Step => "Step",
            UiProp::Allow => "Allow",
            UiProp::MaxLength => "MaxLength",
            UiProp::Layout => "Layout",
            UiProp::Style => "Style",
            UiProp::Bind => "Bind",
            UiProp::Items => "Items",
            UiProp::Scroll => "Scroll",
            UiProp::SelectedIndex => "SelectedIndex",
            UiProp::Theme => "Theme",
            UiProp::Enabled => "Enabled",
            UiProp::Tooltip => "Tooltip",
            UiProp::WorldActor => "WorldActor",
            UiProp::Transition => "Transition",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        UiProp::ALL.iter().copied().find(|prop| prop.name() == name)
    }
}

/// Which characters a text input takes. Written with `set [allow] of (id)
/// to`, by the name below in any case - anything else reads as
/// [`UiAllow::Any`], since a typo that quietly deadened a field would be
/// worse than one that quietly takes everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UiAllow {
    #[default]
    Any,
    /// A decimal number: digits, one leading minus, one point.
    Numbers,
    /// Digits and nothing else - a PIN, a port, a seed.
    Digits,
    /// Letters and spaces, for a name.
    Letters,
}

impl UiAllow {
    pub const ALL: &'static [UiAllow] = &[
        UiAllow::Any,
        UiAllow::Numbers,
        UiAllow::Digits,
        UiAllow::Letters,
    ];

    pub fn name(self) -> &'static str {
        match self {
            UiAllow::Any => "any",
            UiAllow::Numbers => "numbers",
            UiAllow::Digits => "digits",
            UiAllow::Letters => "letters",
        }
    }

    /// The rule a person's typed word names. Case and surrounding space
    /// don't matter, and a plural is the same word as its singular, since
    /// this is a slot somebody fills in by hand.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().trim_end_matches('s') {
            "number" | "numeric" | "decimal" => UiAllow::Numbers,
            "digit" | "integer" | "whole" => UiAllow::Digits,
            "letter" | "alpha" | "alphabetic" => UiAllow::Letters,
            _ => UiAllow::Any,
        }
    }

    /// Whether one more character may go on the end of what is there. The
    /// text so far is part of the question: a minus only leads, and a number
    /// has one point.
    pub fn admits(self, text: &str, ch: char) -> bool {
        match self {
            UiAllow::Any => true,
            UiAllow::Digits => ch.is_ascii_digit(),
            UiAllow::Numbers => {
                ch.is_ascii_digit()
                    || (ch == '-' && text.is_empty())
                    || (ch == '.' && !text.contains('.'))
            }
            UiAllow::Letters => ch.is_alphabetic() || ch == ' ',
        }
    }
}

/// What a text input holds after one character is typed into it, or `None`
/// when its own rule refuses the character - a full field, or one the wrong
/// sort. Backspace is the caller's business; this is only the growing half.
pub fn typed(text: &str, ch: char, allow: UiAllow, max_length: usize) -> Option<String> {
    if max_length > 0 && text.chars().count() >= max_length {
        return None;
    }
    if !allow.admits(text, ch) {
        return None;
    }
    let mut next = text.to_string();
    next.push(ch);
    Some(next)
}

/// Where typing lands in a text input, and what is selected. Positions count
/// characters, not bytes, and are kept inside the text by every operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextCaret {
    pub at: usize,
    /// The other end of a selection, when one is held.
    pub anchor: Option<usize>,
}

impl TextCaret {
    /// A caret at the end of `text`, as a freshly focused input has.
    pub fn end_of(text: &str) -> Self {
        Self {
            at: text.chars().count(),
            anchor: None,
        }
    }

    /// The selected characters as `(start, end)`, if any are.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor
            .filter(|a| *a != self.at)
            .map(|a| (a.min(self.at), a.max(self.at)))
    }

    pub fn clamp(&mut self, text: &str) {
        let len = text.chars().count();
        self.at = self.at.min(len);
        self.anchor = self.anchor.map(|a| a.min(len));
    }

    fn byte(text: &str, chars: usize) -> usize {
        text.char_indices()
            .nth(chars)
            .map_or(text.len(), |(i, _)| i)
    }

    /// Removes the selection, leaving the caret where it was. True if it did.
    fn cut(&mut self, text: &mut String) -> bool {
        let Some((start, end)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        text.replace_range(Self::byte(text, start)..Self::byte(text, end), "");
        self.at = start;
        self.anchor = None;
        true
    }

    /// Types one character over the selection, if any. The input's own rules
    /// decide; a refused character leaves the text and selection as they were.
    pub fn insert(
        &mut self,
        text: &mut String,
        ch: char,
        allow: UiAllow,
        max_length: usize,
    ) -> bool {
        self.clamp(text);
        let mut next = text.clone();
        let mut caret = *self;
        caret.cut(&mut next);
        if max_length > 0 && next.chars().count() >= max_length {
            return false;
        }
        // A minus only leads, so it is judged against what precedes it.
        let seen = if ch == '-' {
            &next[..Self::byte(&next, caret.at)]
        } else {
            &next[..]
        };
        if !allow.admits(seen, ch) {
            return false;
        }
        next.insert(Self::byte(&next, caret.at), ch);
        caret.at += 1;
        *text = next;
        *self = caret;
        true
    }

    /// Rubs out the selection, or the character before the caret.
    pub fn backspace(&mut self, text: &mut String) -> bool {
        self.clamp(text);
        if self.cut(text) {
            return true;
        }
        if self.at == 0 {
            return false;
        }
        text.replace_range(Self::byte(text, self.at - 1)..Self::byte(text, self.at), "");
        self.at -= 1;
        true
    }

    /// Rubs out the selection, or the character after the caret.
    pub fn delete(&mut self, text: &mut String) -> bool {
        self.clamp(text);
        if self.cut(text) {
            return true;
        }
        if self.at >= text.chars().count() {
            return false;
        }
        text.replace_range(Self::byte(text, self.at)..Self::byte(text, self.at + 1), "");
        true
    }

    /// Moves by `delta` characters. `extend` (Shift) grows the selection;
    /// without it a held selection collapses to the edge it moves towards.
    pub fn step(&mut self, text: &str, delta: isize, extend: bool) {
        self.clamp(text);
        if !extend && let Some((start, end)) = self.selection() {
            self.at = if delta < 0 { start } else { end };
            self.anchor = None;
            return;
        }
        let len = text.chars().count() as isize;
        let moved = (self.at as isize + delta).clamp(0, len) as usize;
        self.go(moved, extend);
    }

    /// Jumps to `at` (clamped), selecting from where it was when `extend`.
    pub fn go(&mut self, at: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.at);
        } else {
            self.anchor = None;
        }
        self.at = at;
    }

    pub fn home(&mut self, extend: bool) {
        self.go(0, extend);
    }

    pub fn end(&mut self, text: &str, extend: bool) {
        self.go(text.chars().count(), extend);
    }

    pub fn select_all(&mut self, text: &str) {
        self.anchor = Some(0);
        self.at = text.chars().count();
    }

    /// The text with the caret drawn in and any selection and uncommitted IME
    /// composition marked, for an input that holds the keyboard.
    pub fn shown(&self, text: &str, preedit: &str) -> String {
        let mut caret = *self;
        caret.clamp(text);
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let selection = caret.selection();
        for (i, ch) in chars.iter().enumerate() {
            if i == caret.at {
                out.push_str(preedit);
                if selection.is_none() {
                    out.push('|');
                }
            }
            if selection.is_some_and(|(s, _)| s == i) {
                out.push('[');
            }
            out.push(*ch);
            if selection.is_some_and(|(_, e)| e == i + 1) {
                out.push(']');
            }
        }
        if caret.at >= chars.len() {
            out.push_str(preedit);
            if selection.is_none() {
                out.push('|');
            }
        }
        out
    }
}

/// Where a slider's number really lands: inside its own ends, and on a
/// multiple of its step measured from the low end - so a 0-to-10 slider
/// stepping 3 stops at 0, 3, 6 and 9, never at 7.4 and never at 10. A step
/// that doesn't divide the span leaves the top end short, which is the price
/// of every stop being a whole step from the last.
///
/// A step of zero or less slides continuously, which is what an element that
/// never had a `step` written does.
pub fn snap(range: [f32; 2], step: f32, value: f64) -> f64 {
    let [low, high] = [range[0] as f64, range[1] as f64];
    let (bottom, top) = if low <= high {
        (low, high)
    } else {
        (high, low)
    };
    let value = value.clamp(bottom, top);
    if step <= 0.0 || !step.is_finite() {
        return value;
    }
    let step = step as f64;
    (low + ((value - low) / step).round() * step).clamp(bottom, top)
}

/// An element as a `show` block asks for it: every slot already evaluated,
/// which is what the host is handed.
///
/// `width`/`height` of zero mean "as big as the content needs". `parent`
/// empty means the element hangs off the window rather than another element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiElement {
    pub id: String,
    pub kind: UiKind,
    /// A panel's title, a label's text, a button's or toggle's caption, an
    /// image's asset path, an input's placeholder. Empty means none.
    pub content: String,
    pub anchor: UiAnchor,
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub parent: String,
    /// A modal panel pauses nothing by itself, but it swallows world clicks
    /// behind it. Only meaningful on a panel.
    pub modal: bool,
    /// A slider's ends. Ignored by every other kind.
    pub range: [f32; 2],
    /// What the widget starts at: a slider's number, a toggle's on/off, an
    /// input's text. Ignored by the rest.
    pub value: Evaluated,
}

impl Default for UiElement {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: UiKind::Panel,
            content: String::new(),
            anchor: UiAnchor::Center,
            offset: [0.0; 2],
            size: [0.0; 2],
            parent: String::new(),
            modal: false,
            range: [0.0, 100.0],
            value: Evaluated::Text(String::new()),
        }
    }
}

impl UiElement {
    /// What `value` means for this kind of element, so `value of (id)`
    /// answers the same shape whoever wrote it. What a `set value` block
    /// hands over goes through here.
    pub fn initial_value(kind: UiKind, value: &Evaluated) -> Evaluated {
        match kind {
            UiKind::Toggle => Evaluated::Bool(value.as_bool()),
            UiKind::Slider
            | UiKind::Progress
            | UiKind::RadialProgress
            | UiKind::Scrollbar
            | UiKind::Tabs
            | UiKind::Select
            | UiKind::ListView => Evaluated::Number(value.as_number().unwrap_or(0.0)),
            // A button reports whether it is being held; the rest report
            // their own text, which is the only thing they have to say.
            UiKind::Button => Evaluated::Bool(false),
            _ => Evaluated::Text(value.as_text()),
        }
    }

    /// What a fresh element of a kind whose row has no value slot starts at.
    /// `flag` is the toggle's on/off, the only one of them that carries a
    /// starting state - a new text input is empty, not the word "false".
    pub fn blank(kind: UiKind, flag: bool) -> Evaluated {
        match kind {
            UiKind::Toggle => Evaluated::Bool(flag),
            UiKind::Slider
            | UiKind::Progress
            | UiKind::RadialProgress
            | UiKind::Scrollbar
            | UiKind::Tabs
            | UiKind::Select
            | UiKind::ListView => Evaluated::Number(0.0),
            UiKind::Button => Evaluated::Bool(false),
            _ => Evaluated::Text(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_anchors_round_trip_through_their_wire_index() {
        for index in 0..23 {
            assert_eq!(UiKind::from_index(index).index(), index);
        }
        for index in 0..9 {
            assert_eq!(UiAnchor::from_index(index).index(), index);
        }
    }

    #[test]
    fn themes_round_trip_through_their_wire_index() {
        for theme in UiTheme::ALL {
            assert_eq!(UiTheme::from_index(theme.index()), *theme);
        }
    }

    #[test]
    fn an_anchor_names_the_fraction_of_the_window_it_hangs_off() {
        assert_eq!(UiAnchor::TopLeft.fractions(), [0.0, 0.0]);
        assert_eq!(UiAnchor::Center.fractions(), [0.5, 0.5]);
        assert_eq!(UiAnchor::BottomRight.fractions(), [1.0, 1.0]);
    }

    #[test]
    fn every_property_round_trips_through_its_wire_name() {
        for prop in UiProp::ALL {
            assert_eq!(UiProp::from_name(prop.name()), Some(*prop));
        }
        assert_eq!(UiProp::from_name("Nonsense"), None);
    }

    #[test]
    fn a_fresh_element_with_no_value_slot_starts_blank_but_a_toggle_keeps_its_flag() {
        assert_eq!(
            UiElement::blank(UiKind::Input, false),
            Evaluated::Text(String::new())
        );
        assert_eq!(
            UiElement::blank(UiKind::Toggle, true),
            Evaluated::Bool(true)
        );
        assert_eq!(
            UiElement::blank(UiKind::Label, true),
            Evaluated::Text(String::new())
        );
    }

    #[test]
    fn a_rule_takes_the_characters_it_names_and_turns_the_rest_away() {
        assert!(UiAllow::Any.admits("", '/'));
        assert!(UiAllow::Digits.admits("4", '2'));
        assert!(!UiAllow::Digits.admits("4", '.'));
        assert!(UiAllow::Letters.admits("Ada", ' '));
        assert!(!UiAllow::Letters.admits("Ada", '7'));
        // A minus only leads, and there is one point in a number.
        assert!(UiAllow::Numbers.admits("", '-'));
        assert!(!UiAllow::Numbers.admits("4", '-'));
        assert!(UiAllow::Numbers.admits("4", '.'));
        assert!(!UiAllow::Numbers.admits("4.5", '.'));
    }

    #[test]
    fn a_typed_character_is_refused_by_the_rule_or_by_the_ceiling() {
        assert_eq!(typed("ab", 'c', UiAllow::Any, 0), Some("abc".to_string()));
        // Full: the rule would have taken it, the ceiling doesn't.
        assert_eq!(typed("ab", 'c', UiAllow::Any, 2), None);
        assert_eq!(typed("12", 'x', UiAllow::Digits, 0), None);
        assert_eq!(
            typed("12", '3', UiAllow::Digits, 3),
            Some("123".to_string())
        );
    }

    #[test]
    fn a_rule_is_named_by_the_word_a_person_would_write() {
        assert_eq!(UiAllow::from_name("Numbers"), UiAllow::Numbers);
        assert_eq!(UiAllow::from_name(" digit "), UiAllow::Digits);
        assert_eq!(UiAllow::from_name("letters"), UiAllow::Letters);
        // A word nothing answers to takes everything rather than nothing.
        assert_eq!(UiAllow::from_name("wharrgarbl"), UiAllow::Any);
        for allow in UiAllow::ALL {
            assert_eq!(UiAllow::from_name(allow.name()), *allow);
        }
    }

    #[test]
    fn a_step_rounds_a_sliders_number_from_its_low_end_and_keeps_it_inside() {
        assert_eq!(snap([0.0, 10.0], 3.0, 7.4), 6.0);
        assert_eq!(snap([0.0, 10.0], 3.0, 8.0), 9.0);
        // A step that doesn't divide the span leaves the far end short: the
        // last stop is a whole step from the one before it.
        assert_eq!(snap([0.0, 10.0], 3.0, 10.0), 9.0);
        // One that does divide it reaches the end exactly.
        assert_eq!(snap([0.0, 100.0], 25.0, 100.0), 100.0);
        // Measured from the low end, not from zero.
        assert_eq!(snap([1.0, 11.0], 5.0, 5.0), 6.0);
        // No step at all is what v1 did, ends and all.
        assert_eq!(snap([0.0, 10.0], 0.0, 7.4), 7.4);
        assert_eq!(snap([0.0, 10.0], 0.0, 40.0), 10.0);
        assert_eq!(snap([0.0, 10.0], 0.0, -40.0), 0.0);
    }

    #[test]
    fn only_the_three_a_person_can_change_count_as_inputs() {
        assert!(UiKind::Slider.is_input());
        assert!(UiKind::Toggle.is_input());
        assert!(UiKind::Input.is_input());
        assert!(!UiKind::Label.is_input());
        assert!(!UiKind::Button.is_input());
    }
    #[test]
    fn a_caret_types_deletes_and_selects_inside_the_text() {
        let mut text = String::from("held");
        let mut caret = TextCaret::end_of(&text);
        caret.step(&text, -2, false);
        assert!(caret.insert(&mut text, 'x', UiAllow::Any, 0));
        assert_eq!((text.as_str(), caret.at), ("hexld", 3));
        assert!(caret.backspace(&mut text));
        assert_eq!((text.as_str(), caret.at), ("held", 2));
        assert!(caret.delete(&mut text));
        assert_eq!(text, "hed");
        // Shift-extend, then typing replaces the selection.
        caret.home(true);
        assert_eq!(caret.selection(), Some((0, 2)));
        assert!(caret.insert(&mut text, 'J', UiAllow::Any, 0));
        assert_eq!((text.as_str(), caret.at, caret.anchor), ("Jd", 1, None));
        caret.select_all(&text);
        assert!(caret.backspace(&mut text));
        assert_eq!((text.as_str(), caret.at), ("", 0));
        assert!(!caret.backspace(&mut text) && !caret.delete(&mut text));
    }

    #[test]
    fn a_caret_keeps_the_inputs_rules_and_counts_characters() {
        let mut text = String::from("5");
        let mut caret = TextCaret::end_of(&text);
        // A minus only leads; a full field takes no more; a number has one point.
        assert!(!caret.insert(&mut text, '-', UiAllow::Numbers, 0));
        caret.home(false);
        assert!(caret.insert(&mut text, '-', UiAllow::Numbers, 0));
        caret.end(&text, false);
        assert!(caret.insert(&mut text, '.', UiAllow::Numbers, 0));
        assert!(!caret.insert(&mut text, '.', UiAllow::Numbers, 0));
        assert!(!caret.insert(&mut text, 'a', UiAllow::Digits, 0));
        let mut word = String::from("ab");
        let mut at = TextCaret::end_of(&word);
        assert!(!at.insert(&mut word, 'c', UiAllow::Any, 2));
        // Multi-byte characters move one at a time.
        let mut text = String::from("héé");
        let mut caret = TextCaret::end_of(&text);
        caret.step(&text, -1, false);
        assert!(caret.backspace(&mut text));
        assert_eq!(text, "hé");
        // A held selection collapses towards the arrow, and the display marks it.
        let mut caret = TextCaret::end_of("abc");
        assert_eq!(caret.shown("abc", ""), "abc|");
        caret.select_all("abc");
        assert_eq!(caret.shown("abc", ""), "[abc]");
        caret.step("abc", -1, false);
        assert_eq!((caret.at, caret.selection()), (0, None));
        assert_eq!(caret.shown("abc", "ni"), "ni|abc");
    }
}

#[path = "ui_framework.rs"]
mod framework;
pub use framework::*;
