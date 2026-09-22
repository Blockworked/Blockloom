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
}

impl UiKind {
    pub fn index(self) -> usize {
        match self {
            UiKind::Panel => 0,
            UiKind::Label => 1,
            UiKind::Button => 2,
            UiKind::Image => 3,
            UiKind::Input => 4,
            UiKind::Slider => 5,
            UiKind::Toggle => 6,
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
            _ => UiKind::Panel,
        }
    }

    /// True for the three that a person can change, and so the three that
    /// `when (id) changed` and `value of (id)` mean anything for.
    pub fn is_input(self) -> bool {
        matches!(self, UiKind::Input | UiKind::Slider | UiKind::Toggle)
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
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        UiProp::ALL.iter().copied().find(|prop| prop.name() == name)
    }
}

/// An element as a `show` block asks for it: every slot already evaluated,
/// which is what the host is handed.
///
/// `width`/`height` of zero mean "as big as the content needs". `parent`
/// empty means the element hangs off the window rather than another element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
            UiKind::Slider => Evaluated::Number(value.as_number().unwrap_or(0.0)),
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
            UiKind::Slider => Evaluated::Number(0.0),
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
        for index in 0..7 {
            assert_eq!(UiKind::from_index(index).index(), index);
        }
        for index in 0..9 {
            assert_eq!(UiAnchor::from_index(index).index(), index);
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
    fn only_the_three_a_person_can_change_count_as_inputs() {
        assert!(UiKind::Slider.is_input());
        assert!(UiKind::Toggle.is_input());
        assert!(UiKind::Input.is_input());
        assert!(!UiKind::Label.is_input());
        assert!(!UiKind::Button.is_input());
    }
}
