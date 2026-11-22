//! Interface documents as a tree (UI v2). A node says how it sits in its
//! parent (`LayoutItem`) apart from how it lays out its children (its
//! widget's container), and each widget kind carries only its own fields.
//! The runtime still draws a flat `UiDocument`: `UiScreen::lower` writes one,
//! and `UiScreen::from_document` reads an old one without moving anything.

use crate::ui::{
    UiAction, UiAlign, UiAnchor, UiAnimation, UiBinding, UiDocument, UiElement, UiExport, UiFx,
    UiKind, UiLayout, UiLength, UiScale, UiStyles, UiTheme, UiVariant, UiWidget,
};
use crate::value::Evaluated;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// The version a tree document is written at.
pub const UI_TREE_VERSION: u32 = 2;

/// One screen: what a `.blockui` file holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiScreen {
    pub version: u32,
    pub nodes: Vec<UiNode>,
    pub theme: UiTheme,
    /// Named classes a node's `class` picks from.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub classes: BTreeMap<String, UiStyles>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stylesheets: Vec<String>,
    pub scale: UiScale,
    pub reference_size: [f32; 2],
    pub safe_area: [f32; 4],
    /// Reusable widgets an instance node names through `instance_of`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub components: BTreeMap<String, UiComponent>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub animations: BTreeMap<String, UiAnimation>,
}

impl Default for UiScreen {
    fn default() -> Self {
        let doc = UiDocument::default();
        Self {
            version: UI_TREE_VERSION,
            nodes: Vec::new(),
            theme: doc.theme,
            classes: BTreeMap::new(),
            stylesheets: Vec::new(),
            scale: doc.scale,
            reference_size: doc.reference_size,
            safe_area: doc.safe_area,
            components: BTreeMap::new(),
            animations: BTreeMap::new(),
        }
    }
}

/// A reusable widget tree and what an instance may set on it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiComponent {
    pub nodes: Vec<UiNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<UiExport>,
}

/// One widget and its children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiNode {
    /// What blocks and scripts address it by; unique in its document.
    pub name: String,
    pub widget: Widget,
    /// How it sits in its parent.
    pub place: LayoutItem,
    /// Padding, gap and alignment over the kind's own.
    #[serde(skip_serializing_if = "Spacing::is_default")]
    pub spacing: Spacing,
    #[serde(skip_serializing_if = "is_default")]
    pub style: UiStyles,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub class: String,
    #[serde(skip_serializing_if = "UiFx::is_default")]
    pub fx: UiFx,
    #[serde(skip_serializing_if = "is_default")]
    pub transition: f32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tooltip: String,
    /// The actor it follows on screen, if any.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub world_actor: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<UiBinding>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<UiAction>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<UiVariant>,
    /// The tab page it shows on, under a Tabs parent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    /// On an instance's root: the component it came from.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instance_of: String,
    /// What an old document said that this model has no place for, kept so
    /// it is written back unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy: Option<Legacy>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<UiNode>,
}

impl Default for UiNode {
    fn default() -> Self {
        Self {
            name: String::new(),
            widget: Widget::Panel {
                title: String::new(),
                modal: false,
            },
            place: LayoutItem::default(),
            spacing: Spacing::default(),
            style: UiStyles::default(),
            class: String::new(),
            fx: UiFx::default(),
            transition: 0.,
            tooltip: String::new(),
            world_actor: String::new(),
            bindings: Vec::new(),
            actions: Vec::new(),
            variants: Vec::new(),
            page: None,
            instance_of: String::new(),
            legacy: None,
            children: Vec::new(),
        }
    }
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// Each kind and only the fields it uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Widget {
    Panel {
        #[serde(default)]
        title: String,
        #[serde(default)]
        modal: bool,
    },
    Label {
        #[serde(default)]
        text: String,
    },
    Button {
        #[serde(default)]
        caption: String,
    },
    Image {
        #[serde(default)]
        image: String,
    },
    Input {
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        text: String,
    },
    Slider {
        #[serde(default = "full_range")]
        range: [f32; 2],
        #[serde(default)]
        value: f64,
    },
    Toggle {
        #[serde(default)]
        caption: String,
        #[serde(default)]
        on: bool,
    },
    List,
    VerticalBox,
    HorizontalBox,
    Grid,
    Canvas,
    WrapBox,
    SizeBox,
    Spacer,
    Progress {
        #[serde(default = "full_range")]
        range: [f32; 2],
        #[serde(default)]
        value: f64,
    },
    RadialProgress {
        #[serde(default = "full_range")]
        range: [f32; 2],
        #[serde(default)]
        value: f64,
    },
    ListView {
        #[serde(default)]
        items: Vec<String>,
        #[serde(default)]
        selected: f64,
    },
    Tabs {
        #[serde(default)]
        items: Vec<String>,
        #[serde(default)]
        selected: f64,
    },
    Select {
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        items: Vec<String>,
        #[serde(default)]
        selected: f64,
    },
    Scrollbar {
        /// The scrolling widget it drives.
        #[serde(default)]
        target: String,
        #[serde(default = "full_range")]
        range: [f32; 2],
        #[serde(default)]
        value: f64,
    },
    RichText {
        #[serde(default)]
        text: String,
    },
    Tooltip {
        #[serde(default)]
        text: String,
    },
}

fn full_range() -> [f32; 2] {
    [0., 100.]
}

/// How a widget lays out its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Container {
    /// One after another down (`vertical`) or across.
    Stack {
        vertical: bool,
    },
    Grid,
    Wrap,
    /// Children place themselves by their anchors.
    Canvas,
    Tabs,
    /// A vertical stack that scrolls.
    Scroll,
}

impl Widget {
    pub fn kind(&self) -> UiKind {
        match self {
            Self::Panel { .. } => UiKind::Panel,
            Self::Label { .. } => UiKind::Label,
            Self::Button { .. } => UiKind::Button,
            Self::Image { .. } => UiKind::Image,
            Self::Input { .. } => UiKind::Input,
            Self::Slider { .. } => UiKind::Slider,
            Self::Toggle { .. } => UiKind::Toggle,
            Self::List => UiKind::List,
            Self::VerticalBox => UiKind::VerticalBox,
            Self::HorizontalBox => UiKind::HorizontalBox,
            Self::Grid => UiKind::Grid,
            Self::Canvas => UiKind::Canvas,
            Self::WrapBox => UiKind::WrapBox,
            Self::SizeBox => UiKind::SizeBox,
            Self::Spacer => UiKind::Spacer,
            Self::Progress { .. } => UiKind::Progress,
            Self::RadialProgress { .. } => UiKind::RadialProgress,
            Self::ListView { .. } => UiKind::ListView,
            Self::Tabs { .. } => UiKind::Tabs,
            Self::Select { .. } => UiKind::Select,
            Self::Scrollbar { .. } => UiKind::Scrollbar,
            Self::RichText { .. } => UiKind::RichText,
            Self::Tooltip { .. } => UiKind::Tooltip,
        }
    }

    /// A fresh widget of `kind`.
    pub fn blank(kind: UiKind) -> Self {
        let element = UiElement {
            kind,
            value: UiElement::blank(kind, false),
            ..UiElement::default()
        };
        Self::of(&element)
    }

    /// The container it is, if it holds children.
    pub fn container(&self) -> Option<Container> {
        Some(match self.kind() {
            UiKind::Panel | UiKind::VerticalBox | UiKind::SizeBox => {
                Container::Stack { vertical: true }
            }
            UiKind::HorizontalBox => Container::Stack { vertical: false },
            UiKind::Grid => Container::Grid,
            UiKind::WrapBox => Container::Wrap,
            UiKind::Canvas => Container::Canvas,
            UiKind::Tabs => Container::Tabs,
            UiKind::List | UiKind::ListView => Container::Scroll,
            _ => return None,
        })
    }

    /// The widget an old element describes. What it doesn't say for its
    /// kind (a title on a slider) is left for `Legacy`.
    fn of(element: &UiElement) -> Self {
        let content = element.content.clone();
        let number = element.value.as_number().unwrap_or(0.0);
        let range = element.range;
        match element.kind {
            UiKind::Panel => Self::Panel {
                title: content,
                modal: element.modal,
            },
            UiKind::Label => Self::Label { text: content },
            UiKind::Button => Self::Button { caption: content },
            UiKind::Image => Self::Image { image: content },
            UiKind::Input => Self::Input {
                placeholder: content,
                text: element.value.as_text(),
            },
            UiKind::Slider => Self::Slider {
                range,
                value: number,
            },
            UiKind::Toggle => Self::Toggle {
                caption: content,
                on: element.value.as_bool(),
            },
            UiKind::List => Self::List,
            UiKind::VerticalBox => Self::VerticalBox,
            UiKind::HorizontalBox => Self::HorizontalBox,
            UiKind::Grid => Self::Grid,
            UiKind::Canvas => Self::Canvas,
            UiKind::WrapBox => Self::WrapBox,
            UiKind::SizeBox => Self::SizeBox,
            UiKind::Spacer => Self::Spacer,
            UiKind::Progress => Self::Progress {
                range,
                value: number,
            },
            UiKind::RadialProgress => Self::RadialProgress {
                range,
                value: number,
            },
            UiKind::ListView => Self::ListView {
                items: Vec::new(),
                selected: number,
            },
            UiKind::Tabs => Self::Tabs {
                items: Vec::new(),
                selected: number,
            },
            UiKind::Select => Self::Select {
                placeholder: content,
                items: Vec::new(),
                selected: number,
            },
            UiKind::Scrollbar => Self::Scrollbar {
                target: String::new(),
                range,
                value: number,
            },
            UiKind::RichText => Self::RichText { text: content },
            UiKind::Tooltip => Self::Tooltip { text: content },
        }
    }

    /// The element fields this widget decides: content, modal, range,
    /// value, items and scroll target.
    fn write(&self, element: &mut UiElement, items: &mut Vec<String>, target: &mut String) {
        let kind = self.kind();
        element.kind = kind;
        element.value = UiElement::blank(kind, false);
        let number = |v: f64| Evaluated::Number(v);
        match self {
            Self::Panel { title, modal } => {
                element.content = title.clone();
                element.modal = *modal;
            }
            Self::Label { text } | Self::RichText { text } | Self::Tooltip { text } => {
                element.content = text.clone()
            }
            Self::Button { caption } => element.content = caption.clone(),
            Self::Image { image } => element.content = image.clone(),
            Self::Input { placeholder, text } => {
                element.content = placeholder.clone();
                element.value = Evaluated::Text(text.clone());
            }
            Self::Slider { range, value }
            | Self::Progress { range, value }
            | Self::RadialProgress { range, value } => {
                element.range = *range;
                element.value = number(*value);
            }
            Self::Toggle { caption, on } => {
                element.content = caption.clone();
                element.value = Evaluated::Bool(*on);
            }
            Self::ListView {
                items: list,
                selected,
            }
            | Self::Tabs {
                items: list,
                selected,
            } => {
                items.clone_from(list);
                element.value = number(*selected);
            }
            Self::Select {
                placeholder,
                items: list,
                selected,
            } => {
                element.content = placeholder.clone();
                items.clone_from(list);
                element.value = number(*selected);
            }
            Self::Scrollbar {
                target: driven,
                range,
                value,
            } => {
                target.clone_from(driven);
                element.range = *range;
                element.value = number(*value);
            }
            _ => {}
        }
    }

    fn set_items(&mut self, list: &[String]) -> bool {
        match self {
            Self::ListView { items, .. }
            | Self::Tabs { items, .. }
            | Self::Select { items, .. } => {
                items.extend_from_slice(list);
                true
            }
            _ => false,
        }
    }

    fn set_target(&mut self, driven: &str) -> bool {
        match self {
            Self::Scrollbar { target, .. } => {
                target.push_str(driven);
                true
            }
            _ => false,
        }
    }
}

/// How a node sits in its parent: flowing in its parent's stack, or placed
/// by anchors (in a Canvas, at the top of a screen, or pinned inside a
/// stack). The parent's container decides which one an editor offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutItem {
    pub mode: Placement,
    /// Fixed width and height; zero is the kind's own.
    pub size: [f32; 2],
    #[serde(skip_serializing_if = "Sizing::is_default")]
    pub sizing: Sizing,
}

impl Default for LayoutItem {
    fn default() -> Self {
        Self {
            mode: Placement::Flow,
            size: [0.; 2],
            sizing: Sizing::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Placement {
    /// Flows in the parent's stack, grid or wrap.
    #[default]
    Flow,
    /// Hangs off a point of the parent (`anchor`, as fractions of its box)
    /// by its own `pivot`, moved by `position`. A stretched anchor (min and
    /// max apart) is not drawn yet.
    Anchored {
        anchor_min: [f32; 2],
        anchor_max: [f32; 2],
        pivot: [f32; 2],
        position: [f32; 2],
    },
}

impl Placement {
    /// Placed at one of the nine points, pivot on the same point.
    pub fn at(anchor: UiAnchor, position: [f32; 2]) -> Self {
        let point = anchor.fractions();
        Self::Anchored {
            anchor_min: point,
            anchor_max: point,
            pivot: point,
            position,
        }
    }

    /// The nine-point anchor it is, if it is one.
    pub fn point(&self) -> Option<(UiAnchor, [f32; 2])> {
        let Self::Anchored {
            anchor_min,
            anchor_max,
            pivot,
            position,
        } = self
        else {
            return None;
        };
        if anchor_min != anchor_max || pivot != anchor_min {
            return None;
        }
        (0..9)
            .map(UiAnchor::from_index)
            .find(|a| a.fractions() == *anchor_min)
            .map(|a| (a, *position))
    }
}

/// Sizing rules beyond a fixed size, in the parent's layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sizing {
    pub width: UiLength,
    pub height: UiLength,
    pub min_size: [f32; 2],
    pub max_size: [f32; 2],
    /// Left, top, right, bottom.
    pub margin: [f32; 4],
    pub grow: f32,
}

impl Default for Sizing {
    fn default() -> Self {
        let layout = UiLayout::default();
        Self {
            width: layout.width,
            height: layout.height,
            min_size: layout.min_size,
            max_size: layout.max_size,
            margin: layout.margin,
            grow: layout.grow,
        }
    }
}

impl Sizing {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Spacing over the kind's own; `None` keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Spacing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<UiAlign>,
    /// A grid's columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    /// A virtual list's row height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_height: Option<f32>,
}

impl Spacing {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What the kind draws with when nothing overrides it: padding, gap and
/// alignment. The runtime's `node_for` is the other half; change the two
/// together.
pub fn kind_spacing(kind: UiKind) -> ([f32; 4], f32, UiAlign) {
    const PANEL_GAP: f32 = 8.0;
    const PANEL_PADDING: f32 = 12.0;
    const WIDGET_PADDING: f32 = 8.0;
    match kind {
        UiKind::Panel
        | UiKind::List
        | UiKind::VerticalBox
        | UiKind::HorizontalBox
        | UiKind::Grid
        | UiKind::Canvas
        | UiKind::WrapBox
        | UiKind::SizeBox
        | UiKind::ListView
        | UiKind::Tabs => ([PANEL_PADDING; 4], PANEL_GAP, UiAlign::Stretch),
        UiKind::Label | UiKind::RichText | UiKind::Tooltip => ([2.; 4], 0., UiAlign::Stretch),
        UiKind::Button | UiKind::Input | UiKind::Select => {
            ([WIDGET_PADDING; 4], 0., UiAlign::Center)
        }
        UiKind::Toggle => ([WIDGET_PADDING; 4], PANEL_GAP, UiAlign::Center),
        UiKind::Slider | UiKind::Scrollbar | UiKind::Progress | UiKind::RadialProgress => {
            ([0., 4., 0., 4.], 0., UiAlign::Center)
        }
        UiKind::Image | UiKind::Spacer => ([0.; 4], 0., UiAlign::Stretch),
    }
}

/// Old element fields the tree has no place for.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Legacy {
    /// Text on a kind that shows none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<[f32; 2]>,
    /// A starting value of a shape its kind doesn't take.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Evaluated>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scroll_target: String,
    /// The anchor a pinned or flowing element named but never used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<UiAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 2]>,
    /// Pinned by a layout block at the top or in a Canvas, where its anchor
    /// already places it.
    pub absolute: bool,
}

// ─── Reading an old document ──────────────────────────────────────────────

impl UiScreen {
    /// The tree an old document describes. Every widget lands under its
    /// parent in document order; one whose parent is missing becomes a root.
    pub fn from_document(doc: &UiDocument) -> Self {
        Self {
            version: UI_TREE_VERSION,
            nodes: tree(&doc.widgets),
            theme: doc.theme,
            classes: doc.styles.clone(),
            stylesheets: doc.stylesheets.clone(),
            scale: doc.scale,
            reference_size: doc.reference_size,
            safe_area: doc.safe_area,
            components: doc
                .prefabs
                .iter()
                .map(|(name, widgets)| {
                    let component = UiComponent {
                        nodes: tree(widgets),
                        exports: doc.exports.get(name).cloned().unwrap_or_default(),
                    };
                    (name.clone(), component)
                })
                .collect(),
            animations: doc.animations.clone(),
        }
    }

    /// The flat document the runtime draws: widgets parent first, in tree
    /// order. Fails on what the runtime can't draw yet.
    pub fn lower(&self) -> Result<UiDocument, String> {
        let mut widgets = Vec::new();
        flatten(&self.nodes, None, &mut widgets)?;
        let mut prefabs = BTreeMap::new();
        let mut exports = BTreeMap::new();
        for (name, component) in &self.components {
            let mut list = Vec::new();
            flatten(&component.nodes, None, &mut list)?;
            prefabs.insert(name.clone(), list);
            if !component.exports.is_empty() {
                exports.insert(name.clone(), component.exports.clone());
            }
        }
        Ok(UiDocument {
            version: crate::ui::UI_SCHEMA_VERSION,
            widgets,
            theme: self.theme,
            styles: self.classes.clone(),
            stylesheets: self.stylesheets.clone(),
            scale: self.scale,
            reference_size: self.reference_size,
            safe_area: self.safe_area,
            prefabs,
            exports,
            animations: self.animations.clone(),
        })
    }

    /// Every node, depth first.
    pub fn walk(&self) -> impl Iterator<Item = &UiNode> {
        let mut stack: Vec<&UiNode> = self.nodes.iter().rev().collect();
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend(node.children.iter().rev());
            Some(node)
        })
    }
}

/// Builds the tree from a flat list, children in list order.
fn tree(widgets: &[UiWidget]) -> Vec<UiNode> {
    let ids: HashMap<&str, usize> = widgets
        .iter()
        .enumerate()
        .map(|(i, w)| (w.element.id.as_str(), i))
        .collect();
    let canvas: Vec<bool> = widgets
        .iter()
        .map(|w| {
            let parent = &w.element.parent;
            parent.is_empty()
                || ids
                    .get(parent.as_str())
                    .is_none_or(|&p| widgets[p].element.kind == UiKind::Canvas)
        })
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); widgets.len()];
    let mut roots = Vec::new();
    for (i, widget) in widgets.iter().enumerate() {
        match ids.get(widget.element.parent.as_str()) {
            // A cycle never reaches a root, so it is broken at its first
            // member: that one becomes a root.
            Some(&p) if p != i && !reaches(widgets, &ids, p, i) => children[p].push(i),
            _ => roots.push(i),
        }
    }
    fn build(i: usize, widgets: &[UiWidget], canvas: &[bool], children: &[Vec<usize>]) -> UiNode {
        let mut node = node_of(&widgets[i], canvas[i]);
        node.children = children[i]
            .iter()
            .map(|&c| build(c, widgets, canvas, children))
            .collect();
        node
    }
    roots
        .into_iter()
        .map(|i| build(i, widgets, &canvas, &children))
        .collect()
}

/// Whether walking up from `from` meets `target`.
fn reaches(widgets: &[UiWidget], ids: &HashMap<&str, usize>, from: usize, target: usize) -> bool {
    let mut at = from;
    for _ in 0..widgets.len() {
        match ids.get(widgets[at].element.parent.as_str()) {
            Some(&p) if p == target => return true,
            Some(&p) => at = p,
            None => return false,
        }
    }
    true
}

/// One node from one widget. `free` is whether it places itself: a root or
/// a Canvas child.
fn node_of(widget: &UiWidget, free: bool) -> UiNode {
    let element = &widget.element;
    let mut widget_part = Widget::of(element);
    let mut legacy = Legacy::default();
    // What the widget writes back has to be what was there, or the rest
    // goes to `Legacy`.
    let mut check = UiElement::default();
    let (mut items, mut target) = (Vec::new(), String::new());
    if !widget.items.is_empty() && !widget_part.set_items(&widget.items) {
        legacy.items = widget.items.clone();
    }
    if !widget.scroll_target.is_empty() && !widget_part.set_target(&widget.scroll_target) {
        legacy.scroll_target = widget.scroll_target.clone();
    }
    widget_part.write(&mut check, &mut items, &mut target);
    if check.content != element.content {
        legacy.content = Some(element.content.clone());
    }
    if check.modal != element.modal {
        legacy.modal = Some(element.modal);
    }
    if check.range != element.range {
        legacy.range = Some(element.range);
    }
    if check.value != element.value {
        legacy.value = Some(element.value.clone());
    }
    let layout = widget.layout.clone();
    let absolute = layout.as_ref().is_some_and(|l| l.absolute);
    let mode = if absolute {
        // Pinned to the parent's top left by its offset, whatever its anchor.
        if element.anchor != UiAnchor::TopLeft {
            legacy.anchor = Some(element.anchor);
        }
        legacy.absolute = free;
        Placement::at(UiAnchor::TopLeft, element.offset)
    } else if free {
        Placement::at(element.anchor, element.offset)
    } else {
        if element.anchor != UiAnchor::default() {
            legacy.anchor = Some(element.anchor);
        }
        if element.offset != [0.; 2] {
            legacy.offset = Some(element.offset);
        }
        Placement::Flow
    };
    let (sizing, spacing) = match &layout {
        Some(l) => (
            Sizing {
                width: l.width,
                height: l.height,
                min_size: l.min_size,
                max_size: l.max_size,
                margin: l.margin,
                grow: l.grow,
            },
            Spacing {
                padding: Some(l.padding),
                gap: Some(l.gap),
                align: Some(l.align),
                columns: Some(l.columns),
                row_height: Some(l.row_height),
            },
        ),
        None => (Sizing::default(), Spacing::default()),
    };
    UiNode {
        name: element.id.clone(),
        widget: widget_part,
        place: LayoutItem {
            mode,
            size: element.size,
            sizing,
        },
        spacing,
        style: widget.style.clone(),
        class: widget.class.clone(),
        fx: widget.fx,
        transition: widget.transition,
        tooltip: widget.tooltip.clone(),
        world_actor: widget.world_actor.clone(),
        bindings: widget.bindings.clone(),
        actions: widget.actions.clone(),
        variants: widget.variants.clone(),
        page: widget.tab_index,
        instance_of: widget.instance_of.clone(),
        legacy: (legacy != Legacy::default()).then_some(legacy),
        children: Vec::new(),
    }
}

// ─── Writing the flat document ────────────────────────────────────────────

fn flatten(
    nodes: &[UiNode],
    parent: Option<&UiNode>,
    out: &mut Vec<UiWidget>,
) -> Result<(), String> {
    for node in nodes {
        out.push(widget_of(node, parent)?);
        flatten(&node.children, Some(node), out)?;
    }
    Ok(())
}

fn widget_of(node: &UiNode, parent: Option<&UiNode>) -> Result<UiWidget, String> {
    let free = parent.is_none_or(|p| p.widget.container() == Some(Container::Canvas));
    let mut element = UiElement {
        id: node.name.clone(),
        parent: parent.map(|p| p.name.clone()).unwrap_or_default(),
        size: node.place.size,
        ..UiElement::default()
    };
    let (mut items, mut scroll_target) = (Vec::new(), String::new());
    node.widget
        .write(&mut element, &mut items, &mut scroll_target);
    let kind = element.kind;
    let mut absolute = false;
    match &node.place.mode {
        Placement::Flow => {}
        placed => {
            let Some((anchor, position)) = placed.point() else {
                return Err(format!("{}: stretched anchors aren't drawn yet", node.name));
            };
            element.offset = position;
            if free {
                element.anchor = anchor;
            } else if anchor == UiAnchor::TopLeft {
                element.anchor = anchor;
                absolute = true;
            } else {
                return Err(format!(
                    "{}: inside a stack only the top-left anchor is drawn yet",
                    node.name
                ));
            }
        }
    }
    // A flowing node with nowhere to flow sits where a new element does.
    if let Some(legacy) = &node.legacy {
        if let Some(content) = &legacy.content {
            element.content.clone_from(content);
        }
        if let Some(modal) = legacy.modal {
            element.modal = modal;
        }
        if let Some(range) = legacy.range {
            element.range = range;
        }
        if let Some(value) = &legacy.value {
            element.value = value.clone();
        }
        if !legacy.items.is_empty() {
            items.clone_from(&legacy.items);
        }
        if !legacy.scroll_target.is_empty() {
            scroll_target.clone_from(&legacy.scroll_target);
        }
        if let Some(anchor) = legacy.anchor {
            element.anchor = anchor;
        }
        if let Some(offset) = legacy.offset {
            element.offset = offset;
        }
        absolute |= legacy.absolute;
    }
    let spacing = node.spacing;
    let layout_set = absolute || !node.place.sizing.is_default() || !spacing.is_default();
    let layout = layout_set.then(|| {
        let (padding, gap, align) = kind_spacing(kind);
        let defaults = UiLayout::default();
        let sizing = node.place.sizing;
        UiLayout {
            width: sizing.width,
            height: sizing.height,
            min_size: sizing.min_size,
            max_size: sizing.max_size,
            margin: sizing.margin,
            padding: spacing.padding.unwrap_or(padding),
            gap: spacing.gap.unwrap_or(gap),
            columns: spacing.columns.unwrap_or(defaults.columns),
            grow: sizing.grow,
            align: spacing.align.unwrap_or(align),
            absolute,
            row_height: spacing.row_height.unwrap_or(defaults.row_height),
        }
    });
    Ok(UiWidget {
        element,
        layout,
        style: node.style.clone(),
        class: node.class.clone(),
        bindings: node.bindings.clone(),
        actions: node.actions.clone(),
        variants: node.variants.clone(),
        items,
        tooltip: node.tooltip.clone(),
        world_actor: node.world_actor.clone(),
        scroll_target,
        tab_index: node.page,
        transition: node.transition,
        fx: node.fx,
        instance_of: node.instance_of.clone(),
    })
}

#[cfg(test)]
#[path = "ui_tree_tests.rs"]
mod tests;
