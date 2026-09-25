//! The screen-space interface a game builds out of blocks.
//!
//! [`UiManager`] is the id map: one entry per element the blocks have made,
//! holding the Bevy entity that draws it, what kind it is, whose child it is
//! and what it currently reports. `show` makes one or updates it in place,
//! `set` writes a property, `hide` takes it off the screen without forgetting
//! it, and `delete` forgets it. Every one of those is idempotent, so a HUD
//! strand can run the same blocks every frame.
//!
//! The pure half - the id map, the hit test, the anchor maths - is Bevy-free
//! except for the entity handle, so the rules can be unit-tested without a
//! window. The spawning and the property writes are the systems below.

use bevy::prelude::*;
use blockloom_core::sense::UiSense;
use blockloom_core::ui::{
    UiAllow, UiAnchor, UiBinding, UiDocument, UiElement, UiKind, UiLayout, UiPaint, UiProp,
    UiStyles, UiTheme, UiWidget,
};
use blockloom_core::value::Evaluated;
use std::collections::HashMap;

/// How a fresh element looks before a `set` block says otherwise. Dark and
/// translucent, so the naive pause menu looks intentional.
const TEXT_SIZE: f32 = 16.0;
const PANEL_GAP: f32 = 8.0;
const PANEL_PADDING: f32 = 12.0;
const WIDGET_PADDING: f32 = 8.0;
const CORNER_RADIUS: f32 = 8.0;
/// How tall a slider's track and a toggle's box are, in pixels.
const TRACK_HEIGHT: f32 = 10.0;
const KNOB: f32 = 18.0;

/// Marks the one node every element hangs off, so the tree can be found and
/// cleared without touching the status corner or the speech bubbles.
#[derive(Component)]
pub struct UiRoot;

/// The text node inside an element, which is what a `text` property writes.
#[derive(Component, Debug, Clone)]
pub struct UiElementText(pub String);

/// A slider's filled part, which follows its value.
#[derive(Component, Debug, Clone)]
pub struct UiSliderFill(pub String);

/// A toggle's lamp, which follows its on/off.
#[derive(Component, Debug, Clone)]
pub struct UiToggleLamp(pub String);

/// What a `set` block has written over an element's defaults. Each is
/// `None` until somebody says otherwise, so the active theme shows through.
#[derive(Debug, Clone, Default)]
pub struct UiStyle {
    pub text: Option<String>,
    pub text_color: Option<String>,
    pub text_size: Option<f32>,
    pub background: Option<String>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub corner_radius: Option<f32>,
    pub padding: Option<f32>,
    /// A slider's granularity, and a text input's two rules. They live here
    /// rather than on the element because no `show` row spells them: a HUD
    /// strand re-showing its slider every frame would wipe them otherwise.
    pub step: Option<f32>,
    pub allow: Option<UiAllow>,
    pub max_length: Option<usize>,
}

/// One element as the manager tracks it.
#[derive(Debug, Clone)]
pub struct UiNode {
    pub entity: Entity,
    pub kind: UiKind,
    /// The id of the element this one flows inside, or empty for the window.
    pub parent: String,
    pub modal: bool,
    pub visible: bool,
    /// A slider's ends, for turning a drag into a number.
    pub range: [f32; 2],
    /// What `value of (id)` answers with.
    pub value: Evaluated,
    /// The spec the last `show` asked for, so re-showing the same thing can
    /// leave the entity alone.
    pub spec: UiElement,
    pub style: UiStyle,
    /// Something about this element changed since the drawing system last
    /// looked. Properties live here rather than on the effect because an
    /// effect is gone by the end of the fixed step that produced it.
    pub dirty: bool,
    pub layout: Option<UiLayout>,
    pub styles: UiStyles,
    pub bindings: Vec<UiBinding>,
    pub items: Vec<String>,
    pub scroll: f32,
    pub scroll_dirty: bool,
    pub enabled: bool,
    pub theme: Option<UiTheme>,
    pub tooltip: String,
    pub world_actor: String,
    pub hovered: bool,
    pub pressed: bool,
    pub transition: f32,
    pub collection_range: Option<(usize, usize)>,
    pub collection_dirty: bool,
    pub expanded: bool,
}

impl UiNode {
    /// What a slider's number is rounded to. Zero - which is what an element
    /// nobody wrote a `step` to has - slides continuously.
    pub fn step(&self) -> f32 {
        self.style.step.unwrap_or(0.0)
    }

    /// Which characters this input takes. Everything, until somebody says
    /// otherwise.
    pub fn allow(&self) -> UiAllow {
        self.style.allow.unwrap_or_default()
    }

    /// How many characters it holds; zero for no ceiling.
    pub fn max_length(&self) -> usize {
        self.style.max_length.unwrap_or(0)
    }
}

/// Every element the blocks have made, in the order they were made - which
/// is the order they are laid out in and the reverse of the order a click is
/// tested in.
#[derive(Resource, Default)]
pub struct UiManager {
    order: Vec<String>,
    nodes: HashMap<String, UiNode>,
    /// The text input holding the keyboard, if any. While one does, game
    /// strands see no keys.
    focus: Option<String>,
    /// Elements whose entity still has to be built, in the order asked for.
    pending: Vec<String>,
    /// Elements whose entity has to go.
    dropped: Vec<Entity>,
    theme: UiTheme,
    pub navigation: Option<String>,
    pub binding_writes: Vec<(UiBinding, Evaluated)>,
    pub document: UiDocument,
    pub canvas_scale: f32,
    pub canvas_origin: Vec2,
}

impl UiManager {
    pub fn ids(&self) -> &[String] {
        &self.order
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut UiNode> {
        self.nodes.get_mut(id)
    }
    pub fn load(&mut self, document: &UiDocument) {
        self.clear();
        self.document = document.clone();
        self.set_theme(document.theme);
        for widget in &document.widgets {
            self.show_widget(widget.clone());
        }
    }
    pub fn show_widget(&mut self, widget: UiWidget) {
        let id = widget.element.id.clone();
        self.show(widget.element);
        if let Some(n) = self.nodes.get_mut(&id) {
            n.layout = widget.layout;
            let base = self
                .document
                .styles
                .get(&widget.class)
                .cloned()
                .unwrap_or_default();
            n.styles = UiStyles {
                normal: widget.style.normal.over(&base.normal),
                hover: widget.style.hover.over(&base.hover),
                pressed: widget.style.pressed.over(&base.pressed),
                disabled: widget.style.disabled.over(&base.disabled),
                focused: widget.style.focused.over(&base.focused),
            };
            n.bindings = widget.bindings;
            n.items = widget.items;
            n.tooltip = widget.tooltip;
            n.world_actor = widget.world_actor;
        }
    }
    pub fn bubble(&self, id: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current = id;
        while let Some(n) = self.nodes.get(current) {
            if result.iter().any(|s| s == current) {
                break;
            }
            result.push(current.to_string());
            if n.modal {
                break;
            }
            current = &n.parent;
        }
        result
    }
    pub fn paint(&self, node: &UiNode) -> UiPaint {
        let state = if !node.enabled {
            &node.styles.disabled
        } else if node.pressed {
            &node.styles.pressed
        } else if self.navigation.as_deref() == Some(&node.spec.id)
            || self.focus() == Some(&node.spec.id)
        {
            &node.styles.focused
        } else if node.hovered {
            &node.styles.hover
        } else {
            &node.styles.normal
        };
        let mut paint = state.over(&node.styles.normal);
        paint.background = node.style.background.clone().or(paint.background);
        paint.text_color = node.style.text_color.clone().or(paint.text_color);
        paint.text_size = node.style.text_size.or(paint.text_size);
        paint.radius = node.style.corner_radius.or(paint.radius);
        paint
    }

    pub fn get(&self, id: &str) -> Option<&UiNode> {
        self.nodes.get(id)
    }

    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    pub fn theme(&self) -> UiTheme {
        self.theme
    }

    pub fn set_theme(&mut self, theme: UiTheme) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        for node in self.nodes.values_mut() {
            node.dirty = true;
        }
    }

    /// Every element as the reporter blocks see it, which is what goes into
    /// the sensing snapshot once a frame.
    pub fn senses(&self) -> HashMap<String, UiSense> {
        self.nodes
            .iter()
            .filter(|(id, _)| {
                !id.contains("::row:") && !id.ends_with("::extent") && id.as_str() != "__tooltip"
            })
            .map(|(id, node)| {
                (
                    id.clone(),
                    UiSense {
                        value: node.value.clone(),
                        text: text_of(node),
                        shown: self.shown(node),
                    },
                )
            })
            .collect()
    }

    /// Makes the element `spec` names, or updates the one already there.
    /// Answers true when the entity has to be built again - a change of kind
    /// or of parent, which no property write can carry.
    pub fn show(&mut self, spec: UiElement) -> bool {
        let id = spec.id.clone();
        if id.is_empty() {
            return false;
        }
        if self.subtree(&id).contains(&spec.parent) {
            return false;
        }
        if self
            .nodes
            .get(&id)
            .is_some_and(|n| n.kind != spec.kind || n.parent != spec.parent)
        {
            for child in self.subtree(&id).into_iter().skip(1) {
                if let Some(node) = self.nodes.get_mut(&child) {
                    node.entity = Entity::PLACEHOLDER;
                    node.dirty = true;
                    if !self.pending.contains(&child) {
                        self.pending.push(child);
                    }
                }
            }
        }
        match self.nodes.get_mut(&id) {
            Some(node) if node.kind == spec.kind && node.parent == spec.parent => {
                // The same element again: a HUD strand rebuilding itself.
                // Only its own value is left alone, since a person may have
                // moved it since.
                if node.spec == spec && node.visible {
                    return false;
                }
                node.modal = spec.modal;
                node.visible = true;
                node.range = spec.range;
                node.spec = spec;
                node.dirty = true;
                false
            }
            _ => {
                if let Some(old) = self.nodes.remove(&id) {
                    self.dropped.push(old.entity);
                } else {
                    self.order.push(id.clone());
                }
                self.nodes.insert(
                    id.clone(),
                    UiNode {
                        // A placeholder until the spawning system runs. The
                        // hit test skips a node nothing has drawn yet.
                        entity: Entity::PLACEHOLDER,
                        kind: spec.kind,
                        parent: spec.parent.clone(),
                        modal: spec.modal,
                        visible: true,
                        range: spec.range,
                        value: spec.value.clone(),
                        spec,
                        style: UiStyle::default(),
                        dirty: true,
                        layout: None,
                        styles: UiStyles::default(),
                        bindings: vec![],
                        items: vec![],
                        scroll: 0.,
                        scroll_dirty: false,
                        enabled: true,
                        theme: None,
                        tooltip: String::new(),
                        world_actor: String::new(),
                        hovered: false,
                        pressed: false,
                        transition: 0.,
                        collection_range: None,
                        collection_dirty: true,
                        expanded: false,
                    },
                );
                self.pending.push(id);
                true
            }
        }
    }

    /// Takes an element off the screen without forgetting it, children and
    /// all. An `all` hides everything and drops keyboard focus with it.
    pub fn hide(&mut self, id: &str, all: bool) {
        if all {
            for node in self.nodes.values_mut() {
                node.visible = false;
                node.dirty = true;
            }
            self.focus = None;
            return;
        }
        for id in self.subtree(id) {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.visible = false;
                node.dirty = true;
            }
            if self.focus.as_deref() == Some(id.as_str()) {
                self.focus = None;
            }
        }
    }

    /// Forgets an element entirely, children and all.
    pub fn delete(&mut self, id: &str) {
        for id in self.subtree(id) {
            if let Some(node) = self.nodes.remove(&id) {
                self.dropped.push(node.entity);
            }
            self.order.retain(|other| other != &id);
            self.pending.retain(|other| other != &id);
            if self.focus.as_deref() == Some(id.as_str()) {
                self.focus = None;
            }
        }
    }

    /// Forgets everything, as a fresh Play does.
    pub fn clear(&mut self) {
        for node in self.nodes.values() {
            self.dropped.push(node.entity);
        }
        self.nodes.clear();
        self.order.clear();
        self.pending.clear();
        self.focus = None;
        self.theme = UiTheme::default();
        self.navigation = None;
        self.binding_writes.clear();
    }

    /// An element and everything flowing inside it, parents first.
    pub fn subtree(&self, id: &str) -> Vec<String> {
        let mut found = vec![id.to_string()];
        let mut index = 0;
        while index < found.len() {
            let parent = found[index].clone();
            for candidate in &self.order {
                if self
                    .nodes
                    .get(candidate)
                    .is_some_and(|node| node.parent == parent)
                    && !found.contains(candidate)
                {
                    found.push(candidate.clone());
                }
            }
            index += 1;
        }
        found
    }

    /// Whether a click anywhere should be kept from the world: any visible
    /// modal element swallows it, so clicking Resume never fires the gun
    /// behind the menu.
    pub fn swallows_world_clicks(&self) -> bool {
        self.nodes
            .values()
            .any(|node| node.modal && self.shown(node))
    }

    /// Whether an element and every ancestor of it is visible.
    pub fn shown(&self, node: &UiNode) -> bool {
        if !node.visible {
            return false;
        }
        let mut parent = node.parent.clone();
        let mut guard = 0;
        while !parent.is_empty() && guard < self.order.len() + 1 {
            let Some(above) = self.nodes.get(&parent) else {
                return true;
            };
            if !above.visible {
                return false;
            }
            parent = above.parent.clone();
            guard += 1;
        }
        true
    }

    /// Which element a click at `point` lands on: the topmost visible one
    /// whose rectangle covers it. Later elements are on top, so the order
    /// list is walked backwards.
    ///
    /// `rect_of` hands back each element's screen rectangle, which is what
    /// the caller reads off Bevy's computed layout.
    pub fn hit(&self, point: Vec2, rect_of: impl Fn(&UiNode) -> Option<Rect>) -> Option<&UiNode> {
        let modal = self.order.iter().rev().find(|id| {
            self.nodes
                .get(*id)
                .is_some_and(|n| n.modal && self.shown(n))
        });
        self.order.iter().rev().find_map(|id| {
            if modal.is_some_and(|m| !self.bubble(id).contains(m)) {
                return None;
            }
            let node = self.nodes.get(id)?;
            if !node.enabled || !self.shown(node) || node.entity == Entity::PLACEHOLDER {
                return None;
            }
            if !rect_of(node).is_some_and(|rect| rect.contains(point)) {
                return None;
            }
            let mut parent = node.parent.as_str();
            let mut guard = 0;
            while !parent.is_empty() && guard < self.order.len() + 1 {
                let Some(above) = self.nodes.get(parent) else {
                    break;
                };
                if !above.enabled {
                    return None;
                }
                if above.kind.scrollable()
                    && !rect_of(above).is_some_and(|rect| rect.contains(point))
                {
                    return None;
                }
                parent = &above.parent;
                guard += 1;
            }
            Some(node)
        })
    }

    /// The list under an element, or the element itself when it is a list.
    pub fn scroll_owner(&self, id: &str) -> Option<&UiNode> {
        let mut node = self.nodes.get(id)?;
        let mut guard = 0;
        loop {
            if node.kind.scrollable() {
                return Some(node);
            }
            if node.parent.is_empty() || guard > self.order.len() {
                return None;
            }
            node = self.nodes.get(&node.parent)?;
            guard += 1;
        }
    }

    /// Writes one property. Answers what the sensed value became, when the
    /// property is one that changes it.
    pub fn set(&mut self, id: &str, prop: UiProp, value: &Evaluated) -> bool {
        if prop == UiProp::Visible
            && !value.as_bool()
            && self
                .focus
                .as_ref()
                .is_some_and(|focus| self.subtree(id).contains(focus))
        {
            self.focus_on(None);
        }
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        let unchanged = match prop {
            UiProp::Text => node.style.text.as_deref() == Some(&value.as_text()),
            UiProp::Value => node.value == settled(node, value),
            UiProp::Visible => node.visible == value.as_bool(),
            UiProp::Enabled => node.enabled == value.as_bool(),
            _ => false,
        };
        if unchanged {
            return true;
        }
        let number = |value: &Evaluated| value.as_number().unwrap_or(0.0) as f32;
        node.dirty = true;
        match prop {
            UiProp::Layout => {
                if let Ok(layout) = serde_json::from_str(&value.as_text()) {
                    if node.layout.as_ref() != Some(&layout) {
                        node.collection_dirty = true;
                    }
                    node.layout = Some(layout);
                }
            }
            UiProp::Style => {
                if let Ok(style) = serde_json::from_str(&value.as_text()) {
                    node.styles = style;
                }
            }
            UiProp::Bind => {
                if let Ok(bindings) = serde_json::from_str(&value.as_text()) {
                    node.bindings = bindings;
                }
            }
            UiProp::Items => {
                if let Ok(items) = serde_json::from_str::<Vec<String>>(&value.as_text()) {
                    node.items = items;
                    node.collection_dirty = true;
                    node.value = Evaluated::Number(0.);
                }
            }
            UiProp::Scroll => {
                node.scroll = number(value).max(0.);
                node.scroll_dirty = true;
            }
            UiProp::SelectedIndex => {
                node.value = Evaluated::Number(
                    (number(value).max(0.) as usize).min(node.items.len()) as f64,
                );
            }
            UiProp::Theme => {
                node.theme =
                    serde_json::from_value(serde_json::Value::String(value.as_text())).ok();
            }
            UiProp::Enabled => node.enabled = value.as_bool(),
            UiProp::Tooltip => node.tooltip = value.as_text(),
            UiProp::WorldActor => node.world_actor = value.as_text(),
            UiProp::Transition => node.transition = number(value).clamp(0., 10.),
            UiProp::Visible => {
                let visible = value.as_bool();
                node.visible = visible;
                if !visible && self.focus.as_deref() == Some(id) {
                    self.focus = None;
                }
            }
            UiProp::Modal => node.modal = value.as_bool(),
            // Moving an end, or the step, re-settles where the knob is:
            // a slider told to step by 5 shouldn't stay at 7.4.
            UiProp::Min => {
                node.range[0] = number(value);
                node.value = settled(node, &node.value.clone());
            }
            UiProp::Max => {
                node.range[1] = number(value);
                node.value = settled(node, &node.value.clone());
            }
            UiProp::Step => {
                node.style.step = Some(number(value).max(0.0));
                node.value = settled(node, &node.value.clone());
            }
            UiProp::Value => node.value = settled(node, value),
            UiProp::Allow => node.style.allow = Some(UiAllow::from_name(&value.as_text())),
            UiProp::MaxLength => node.style.max_length = Some(number(value).max(0.0) as usize),
            UiProp::Text => node.style.text = Some(value.as_text()),
            UiProp::TextColor => node.style.text_color = Some(value.as_text()),
            UiProp::TextSize => node.style.text_size = Some(number(value).max(1.0)),
            UiProp::Background => node.style.background = Some(value.as_text()),
            UiProp::Width => node.style.width = Some(number(value)),
            UiProp::Height => node.style.height = Some(number(value)),
            UiProp::CornerRadius => node.style.corner_radius = Some(number(value).max(0.0)),
            UiProp::Padding => node.style.padding = Some(number(value).max(0.0)),
        }
        true
    }

    /// Records what a person just did to an input, and answers what it is
    /// now - which is what the `changed` event carries.
    pub fn changed(&mut self, id: &str, value: Evaluated) -> Option<Evaluated> {
        let node = self.nodes.get_mut(id)?;
        if !node.kind.is_input() {
            return None;
        }
        let next = settled(node, &value);
        if next == node.value {
            return None;
        }
        for binding in &node.bindings {
            if binding.two_way
                && matches!(
                    binding.property,
                    UiProp::Value | UiProp::Text | UiProp::SelectedIndex
                )
            {
                if let Some(value) = binding.converter.write(&next) {
                    self.binding_writes.push((binding.clone(), value));
                }
            }
        }
        node.value = next.clone();
        node.dirty = true;
        Some(next)
    }

    /// Gives the keyboard to a text input, or takes it back. Anything that
    /// isn't one drops focus instead, so clicking away releases the keys.
    pub fn focus_on(&mut self, id: Option<&str>) {
        let next = match id {
            Some(id)
                if self
                    .nodes
                    .get(id)
                    .is_some_and(|n| n.kind == UiKind::Input && n.enabled && self.shown(n)) =>
            {
                Some(id.to_string())
            }
            _ => None,
        };
        if next == self.focus {
            return;
        }
        // Both ends of the move are redrawn, since a focused input looks
        // different from an idle one and nothing else would say so.
        for id in [self.focus.clone(), next.clone()].into_iter().flatten() {
            self.mark_dirty(&id);
        }
        self.focus = next;
    }

    /// Entities the manager has finished with, handed over once.
    pub fn take_dropped(&mut self) -> Vec<Entity> {
        std::mem::take(&mut self.dropped)
    }

    /// Elements still waiting to be drawn, handed over once.
    pub fn take_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    /// Marks an element's drawing out of date again - its entity wasn't in
    /// the world yet when the drawing system last looked, which is what a
    /// `show` in this fixed step leaves behind.
    pub fn mark_dirty(&mut self, id: &str) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.dirty = true;
        }
    }

    /// Puts an element back in the queue: its parent wasn't drawn yet, so
    /// there was nothing to hang it off this frame.
    pub fn defer(&mut self, id: &str) {
        if self.nodes.contains_key(id) && !self.pending.iter().any(|other| other == id) {
            self.pending.push(id.to_string());
        }
    }

    /// Ties an id to the entity that draws it, once the spawner has made it.
    pub fn attach(&mut self, id: &str, entity: Entity) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.entity = entity;
            node.dirty = true;
        }
    }

    /// Ids whose drawing is out of date, handed over once.
    pub fn take_dirty(&mut self) -> Vec<String> {
        let mut stale = Vec::new();
        for id in &self.order {
            if self.nodes.get(id).is_some_and(|node| node.dirty) {
                stale.push(id.clone());
            }
        }
        for id in &stale {
            if let Some(node) = self.nodes.get_mut(id) {
                node.dirty = false;
            }
        }
        stale
    }
}

/// Where a slider's drag puts its value: the fraction of the way across its
/// track, mapped onto its own ends, clamped there and rounded to its step.
pub fn slider_at(range: [f32; 2], step: f32, fraction: f32) -> f64 {
    let [low, high] = range;
    let raw = (low + (high - low) * fraction.clamp(0.0, 1.0)) as f64;
    blockloom_core::ui::snap(range, step, raw)
}

/// What a value really becomes on this element: whatever its kind reports,
/// and on a slider inside its own ends and on its own step.
fn settled(node: &UiNode, value: &Evaluated) -> Evaluated {
    let next = UiElement::initial_value(node.kind, value);
    if node.kind.selectable() {
        return Evaluated::Number(
            next.as_number()
                .unwrap_or(0.)
                .round()
                .clamp(0., node.items.len() as f64),
        );
    }
    if !matches!(
        node.kind,
        UiKind::Slider | UiKind::Scrollbar | UiKind::Progress | UiKind::RadialProgress
    ) {
        return next;
    }
    Evaluated::Number(blockloom_core::ui::snap(
        node.range,
        node.step(),
        next.as_number().unwrap_or(0.0),
    ))
}

/// How a top-level element hangs off the window: its anchor as a fraction
/// of the window, the same fraction of its own size taken back off, and its
/// pixel offset as a margin.
///
/// Said this way rather than in worked-out pixels because the element's own
/// size isn't known until Bevy has laid it out - an auto-sized label is as
/// wide as its words - and because a window resize then recomputes all three
/// for free.
pub fn anchoring(anchor: UiAnchor, offset: [f32; 2]) -> Anchoring {
    let [fx, fy] = anchor.fractions();
    Anchoring {
        left: Val::Percent(fx * 100.0),
        top: Val::Percent(fy * 100.0),
        margin: UiRect {
            left: Val::Px(offset[0]),
            top: Val::Px(offset[1]),
            ..default()
        },
        // Negative fractions of the element's own size: a right-anchored
        // element sits its own right edge on the window's.
        self_shift: Val2::percent(-fx * 100.0, -fy * 100.0),
    }
}

/// The three things [`anchoring`] works out, and the transform that goes
/// with them.
pub struct Anchoring {
    pub left: Val,
    pub top: Val,
    pub margin: UiRect,
    pub self_shift: Val2,
}

/// The Bevy node one element is drawn as, before any property is written.
pub fn node_for(spec: &UiElement, parented: bool) -> Node {
    let mut node = Node {
        position_type: if parented {
            PositionType::Relative
        } else {
            PositionType::Absolute
        },
        border_radius: BorderRadius::all(Val::Px(CORNER_RADIUS)),
        ..default()
    };
    if spec.size[0] > 0.0 {
        node.width = Val::Px(spec.size[0]);
    } else if parented && spec.kind != UiKind::Image {
        // Inside a vertical stack, a row that says nothing takes the whole
        // width - which is what makes Resume / Settings / Quit line up.
        node.width = Val::Percent(100.0);
    }
    if spec.size[1] > 0.0 {
        node.height = Val::Px(spec.size[1]);
    }
    if !parented {
        let at = anchoring(spec.anchor, spec.offset);
        node.left = at.left;
        node.top = at.top;
        node.margin = at.margin;
    }
    match spec.kind {
        UiKind::Panel
        | UiKind::List
        | UiKind::VerticalBox
        | UiKind::HorizontalBox
        | UiKind::Grid
        | UiKind::Canvas
        | UiKind::WrapBox
        | UiKind::SizeBox
        | UiKind::ListView
        | UiKind::Tabs => {
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(PANEL_GAP);
            node.padding = UiRect::all(Val::Px(PANEL_PADDING));
            node.align_items = AlignItems::Stretch;
            if matches!(
                spec.kind,
                UiKind::HorizontalBox | UiKind::WrapBox | UiKind::Tabs
            ) {
                node.flex_direction = FlexDirection::Row;
            }
            if spec.kind == UiKind::WrapBox {
                node.flex_wrap = FlexWrap::Wrap;
            }
            if spec.kind == UiKind::Grid {
                node.display = Display::Grid;
                node.grid_template_columns = RepeatedGridTrack::flex(2, 1.);
            }
            if spec.kind.scrollable() {
                node.overflow = Overflow::scroll_y();
                if spec.size[0] <= 0.0 {
                    node.width = Val::Px(280.0);
                }
                if spec.size[1] <= 0.0 {
                    node.height = Val::Px(240.0);
                }
            }
        }
        UiKind::Label | UiKind::RichText | UiKind::Tooltip => {
            node.padding = UiRect::all(Val::Px(2.0));
        }
        UiKind::Button | UiKind::Input | UiKind::Select => {
            node.padding = UiRect::all(Val::Px(WIDGET_PADDING));
            node.justify_content = if spec.kind == UiKind::Button {
                JustifyContent::Center
            } else {
                JustifyContent::FlexStart
            };
            node.align_items = AlignItems::Center;
        }
        UiKind::Slider | UiKind::Scrollbar | UiKind::Progress | UiKind::RadialProgress => {
            node.height = Val::Px(if spec.size[1] > 0.0 {
                spec.size[1]
            } else {
                KNOB
            });
            node.align_items = AlignItems::Center;
            node.padding = UiRect::vertical(Val::Px(4.0));
        }
        UiKind::Toggle => {
            node.column_gap = Val::Px(PANEL_GAP);
            node.align_items = AlignItems::Center;
            node.padding = UiRect::all(Val::Px(WIDGET_PADDING));
        }
        UiKind::Image | UiKind::Spacer => {}
    }
    node
}

/// A fresh element's background, which a `background` property overrides.
/// The input holding the keyboard is the one thing here that isn't decided
/// by its kind alone.
pub fn background_for(theme: UiTheme, kind: UiKind, focused: bool) -> Color {
    let (panel, button, input, focused_input) = match theme {
        UiTheme::Dark => (
            Color::srgba(0.07, 0.09, 0.13, 0.88),
            Color::srgba(0.16, 0.20, 0.28, 0.95),
            Color::srgba(0.04, 0.05, 0.08, 0.95),
            Color::srgba(0.10, 0.16, 0.26, 0.98),
        ),
        UiTheme::Light => (
            Color::srgba(0.94, 0.96, 0.99, 0.94),
            Color::srgba(0.82, 0.86, 0.92, 0.98),
            Color::srgba(1.0, 1.0, 1.0, 0.98),
            Color::srgba(0.78, 0.87, 1.0, 1.0),
        ),
        UiTheme::HighContrast => (
            Color::BLACK,
            Color::srgb(0.12, 0.12, 0.12),
            Color::BLACK,
            Color::srgb(0.10, 0.20, 0.45),
        ),
    };
    match kind {
        UiKind::Panel | UiKind::List | UiKind::ListView | UiKind::Tooltip => panel,
        UiKind::Button | UiKind::Toggle => button,
        UiKind::Input if focused => focused_input,
        UiKind::Input => input,
        _ => Color::NONE,
    }
}

pub fn text_color(theme: UiTheme) -> Color {
    match theme {
        UiTheme::Dark => Color::srgb(0.93, 0.95, 0.98),
        UiTheme::Light => Color::srgb(0.08, 0.10, 0.14),
        UiTheme::HighContrast => Color::WHITE,
    }
}

pub fn text_size() -> f32 {
    TEXT_SIZE
}

pub fn corner_radius() -> f32 {
    CORNER_RADIUS
}

pub fn track_background(theme: UiTheme) -> Color {
    match theme {
        UiTheme::Dark => Color::srgba(0.04, 0.05, 0.08, 0.95),
        UiTheme::Light => Color::srgba(0.65, 0.69, 0.76, 1.0),
        UiTheme::HighContrast => Color::BLACK,
    }
}

pub fn accent(theme: UiTheme) -> Color {
    match theme {
        UiTheme::Dark => Color::srgb(0.36, 0.60, 0.94),
        UiTheme::Light => Color::srgb(0.10, 0.38, 0.78),
        UiTheme::HighContrast => Color::srgb(1.0, 0.85, 0.0),
    }
}

pub fn toggle_color(theme: UiTheme, on: bool) -> Color {
    if on {
        accent(theme)
    } else {
        match theme {
            UiTheme::Dark => Color::srgba(0.35, 0.38, 0.45, 0.9),
            UiTheme::Light => Color::srgba(0.58, 0.62, 0.68, 1.0),
            UiTheme::HighContrast => Color::WHITE,
        }
    }
}

pub fn track_height() -> f32 {
    TRACK_HEIGHT
}

pub fn knob_size() -> f32 {
    KNOB
}

/// What an element shows as its own text: a label's own words, a button's
/// caption, an input's typed text or its placeholder when it is empty.
pub fn text_of(node: &UiNode) -> String {
    // A `set text` wins over whatever the `show` block said, so a HUD can
    // build its labels once and only write the number afterwards.
    let written = node.style.text.clone();
    match node.kind {
        UiKind::Input => {
            let typed = node.value.as_text();
            if typed.is_empty() {
                written.unwrap_or_else(|| node.spec.content.clone())
            } else {
                typed
            }
        }
        UiKind::Select => node
            .value
            .as_number()
            .ok()
            .and_then(|v| (v as usize).checked_sub(1))
            .and_then(|i| node.items.get(i))
            .cloned()
            .unwrap_or_else(|| node.spec.content.clone()),
        UiKind::Slider => String::new(),
        _ => written.unwrap_or_else(|| node.spec.content.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, kind: UiKind) -> UiElement {
        UiElement {
            id: id.to_string(),
            kind,
            ..Default::default()
        }
    }

    fn drawn(manager: &mut UiManager) {
        for id in manager.take_pending() {
            // Any entity will do: the tests only care that one is there.
            manager.attach(&id, Entity::from_raw_u32(1).expect("a valid entity"));
        }
    }

    #[test]
    fn re_showing_the_same_element_updates_it_rather_than_making_a_second() {
        let mut manager = UiManager::default();
        assert!(manager.show(spec("hud", UiKind::Label)));
        drawn(&mut manager);
        let mut again = spec("hud", UiKind::Label);
        again.content = "score: 3".to_string();
        // The same kind in the same place: nothing to rebuild.
        assert!(!manager.show(again));
        assert_eq!(manager.senses().len(), 1);
        assert_eq!(manager.get("hud").unwrap().spec.content, "score: 3");
    }

    #[test]
    fn re_showing_an_id_as_another_kind_builds_it_again() {
        let mut manager = UiManager::default();
        manager.show(spec("thing", UiKind::Label));
        drawn(&mut manager);
        assert!(manager.show(spec("thing", UiKind::Button)));
        assert_eq!(manager.senses().len(), 1);
        assert_eq!(manager.get("thing").unwrap().kind, UiKind::Button);
    }

    #[test]
    fn hiding_a_panel_hides_everything_flowing_inside_it() {
        let mut manager = UiManager::default();
        manager.show(spec("menu", UiKind::Panel));
        let mut button = spec("resume", UiKind::Button);
        button.parent = "menu".to_string();
        manager.show(button);
        drawn(&mut manager);

        manager.hide("menu", false);
        assert!(!manager.get("menu").unwrap().visible);
        // The child's own flag is down too, and it reads as hidden either way.
        assert!(!manager.get("resume").unwrap().visible);
    }

    #[test]
    fn a_child_of_a_hidden_panel_reads_as_hidden_even_with_its_own_flag_up() {
        let mut manager = UiManager::default();
        manager.show(spec("menu", UiKind::Panel));
        let mut button = spec("resume", UiKind::Button);
        button.parent = "menu".to_string();
        manager.show(button);
        drawn(&mut manager);

        manager.set("menu", UiProp::Visible, &Evaluated::Bool(false));
        let child = manager.get("resume").unwrap();
        assert!(child.visible);
        assert!(!manager.shown(child));
    }

    #[test]
    fn deleting_a_panel_forgets_its_children_too() {
        let mut manager = UiManager::default();
        manager.show(spec("menu", UiKind::Panel));
        let mut button = spec("resume", UiKind::Button);
        button.parent = "menu".to_string();
        manager.show(button);
        drawn(&mut manager);

        manager.delete("menu");
        assert!(manager.senses().is_empty());
    }

    #[test]
    fn a_click_lands_on_the_topmost_element_under_it() {
        let mut manager = UiManager::default();
        manager.show(spec("under", UiKind::Button));
        manager.show(spec("over", UiKind::Button));
        drawn(&mut manager);

        let everywhere = |_: &UiNode| Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        let hit = manager.hit(Vec2::new(50.0, 50.0), everywhere).unwrap();
        assert_eq!(hit.spec.id, "over");
        // Outside every rectangle, nothing is hit.
        assert!(manager.hit(Vec2::new(500.0, 50.0), everywhere).is_none());
    }

    #[test]
    fn a_hidden_element_is_never_hit() {
        let mut manager = UiManager::default();
        manager.show(spec("button", UiKind::Button));
        drawn(&mut manager);
        manager.hide("button", false);
        let everywhere = |_: &UiNode| Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(manager.hit(Vec2::new(10.0, 10.0), everywhere).is_none());
    }

    #[test]
    fn only_a_visible_modal_swallows_world_clicks() {
        let mut manager = UiManager::default();
        let mut menu = spec("menu", UiKind::Panel);
        menu.modal = true;
        manager.show(menu);
        drawn(&mut manager);
        assert!(manager.swallows_world_clicks());

        manager.hide("menu", false);
        assert!(!manager.swallows_world_clicks());
    }

    #[test]
    fn hiding_everything_clears_the_keyboard_focus_with_it() {
        let mut manager = UiManager::default();
        manager.show(spec("name", UiKind::Input));
        drawn(&mut manager);
        manager.focus_on(Some("name"));
        assert_eq!(manager.focus(), Some("name"));

        manager.hide("", true);
        assert_eq!(manager.focus(), None);
        assert!(!manager.get("name").unwrap().visible);
    }

    #[test]
    fn the_input_holding_the_keyboard_is_redrawn_at_both_ends_of_the_move() {
        let mut manager = UiManager::default();
        manager.show(spec("name", UiKind::Input));
        manager.show(spec("email", UiKind::Input));
        drawn(&mut manager);
        manager.take_dirty();

        manager.focus_on(Some("name"));
        assert_eq!(manager.take_dirty(), vec!["name".to_string()]);
        // The one losing it needs redrawing as much as the one taking it.
        manager.focus_on(Some("email"));
        assert_eq!(
            manager.take_dirty(),
            vec!["name".to_string(), "email".to_string()]
        );
        // Focusing what already has it is no change at all.
        manager.focus_on(Some("email"));
        assert!(manager.take_dirty().is_empty());

        assert_ne!(
            background_for(UiTheme::Dark, UiKind::Input, true),
            background_for(UiTheme::Dark, UiKind::Input, false)
        );
    }

    #[test]
    fn changing_the_theme_redraws_every_element_and_keeps_overrides() {
        let mut manager = UiManager::default();
        manager.show(spec("menu", UiKind::Panel));
        manager.show(spec("go", UiKind::Button));
        drawn(&mut manager);
        manager.take_dirty();
        manager.set("go", UiProp::Background, &Evaluated::Text("#123456".into()));
        manager.take_dirty();

        manager.set_theme(UiTheme::Light);

        assert_eq!(manager.theme(), UiTheme::Light);
        assert_eq!(
            manager.take_dirty(),
            vec!["menu".to_string(), "go".to_string()]
        );
        assert_eq!(
            manager.get("go").unwrap().style.background.as_deref(),
            Some("#123456")
        );
    }

    #[test]
    fn a_scrolled_child_is_not_hit_outside_its_lists_viewport() {
        let mut manager = UiManager::default();
        manager.show(spec("list", UiKind::List));
        let mut child = spec("item", UiKind::Button);
        child.parent = "list".to_string();
        manager.show(child);
        drawn(&mut manager);
        let rect = |node: &UiNode| match node.spec.id.as_str() {
            "list" => Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
            "item" => Some(Rect::new(0.0, 120.0, 100.0, 150.0)),
            _ => None,
        };
        assert!(manager.hit(Vec2::new(50.0, 130.0), rect).is_none());
    }

    #[test]
    fn only_a_text_input_can_hold_the_keyboard() {
        let mut manager = UiManager::default();
        manager.show(spec("go", UiKind::Button));
        manager.show(spec("name", UiKind::Input));
        drawn(&mut manager);

        manager.focus_on(Some("go"));
        assert_eq!(manager.focus(), None);
        manager.focus_on(Some("name"));
        assert_eq!(manager.focus(), Some("name"));
        manager.focus_on(None);
        assert_eq!(manager.focus(), None);
    }

    #[test]
    fn an_anchor_puts_a_box_against_the_edge_it_names() {
        let top_left = anchoring(UiAnchor::TopLeft, [10.0, 20.0]);
        assert_eq!(top_left.left, Val::Percent(0.0));
        assert_eq!(top_left.top, Val::Percent(0.0));
        assert_eq!(top_left.margin.left, Val::Px(10.0));
        assert_eq!(top_left.margin.top, Val::Px(20.0));
        // Nothing of its own size comes off in the corner it is already in.
        assert_eq!(top_left.self_shift, Val2::percent(0.0, 0.0));

        // Bottom-right sits the element's own far corner on the window's.
        let bottom_right = anchoring(UiAnchor::BottomRight, [0.0, 0.0]);
        assert_eq!(bottom_right.left, Val::Percent(100.0));
        assert_eq!(bottom_right.top, Val::Percent(100.0));
        assert_eq!(bottom_right.self_shift, Val2::percent(-100.0, -100.0));

        let centre = anchoring(UiAnchor::Center, [0.0, 0.0]);
        assert_eq!(centre.left, Val::Percent(50.0));
        assert_eq!(centre.self_shift, Val2::percent(-50.0, -50.0));
    }

    #[test]
    fn a_change_is_only_reported_when_the_value_really_moved() {
        let mut manager = UiManager::default();
        let mut slider = spec("volume", UiKind::Slider);
        slider.range = [0.0, 10.0];
        slider.value = Evaluated::Number(5.0);
        manager.show(slider);
        drawn(&mut manager);

        assert_eq!(manager.changed("volume", Evaluated::Number(5.0)), None);
        assert_eq!(
            manager.changed("volume", Evaluated::Number(7.0)),
            Some(Evaluated::Number(7.0))
        );
        // A label has nothing a person can change.
        manager.show(spec("hud", UiKind::Label));
        assert_eq!(manager.changed("hud", Evaluated::Text("x".into())), None);
    }

    #[test]
    fn a_drag_maps_onto_the_sliders_own_ends_and_stops_there() {
        let mut manager = UiManager::default();
        let mut slider = spec("volume", UiKind::Slider);
        slider.range = [20.0, 40.0];
        manager.show(slider);
        drawn(&mut manager);
        let range = manager.get("volume").unwrap().range;

        assert_eq!(slider_at(range, 0.0, 0.0), 20.0);
        assert_eq!(slider_at(range, 0.0, 0.5), 30.0);
        assert_eq!(slider_at(range, 0.0, 2.0), 40.0);
        assert_eq!(slider_at(range, 0.0, -1.0), 20.0);
    }

    #[test]
    fn a_slider_with_a_step_only_stops_where_the_step_says() {
        let mut manager = UiManager::default();
        let mut slider = spec("volume", UiKind::Slider);
        slider.range = [0.0, 10.0];
        manager.show(slider);
        drawn(&mut manager);
        manager.set("volume", UiProp::Step, &Evaluated::Number(5.0));

        let node = manager.get("volume").unwrap();
        assert_eq!(slider_at(node.range, node.step(), 0.3), 5.0);
        assert_eq!(slider_at(node.range, node.step(), 0.9), 10.0);
        // A number written straight in lands on the step too.
        manager.set("volume", UiProp::Value, &Evaluated::Number(7.4));
        assert_eq!(manager.get("volume").unwrap().value, Evaluated::Number(5.0));
        // A drag that stays inside one stop has moved nothing, so nothing
        // is reported; one that crosses into the next reports it once.
        assert_eq!(manager.changed("volume", Evaluated::Number(6.0)), None);
        assert_eq!(
            manager.changed("volume", Evaluated::Number(8.0)),
            Some(Evaluated::Number(10.0))
        );
        assert_eq!(manager.changed("volume", Evaluated::Number(8.9)), None);
    }

    #[test]
    fn writing_a_step_moves_a_knob_already_standing_between_two() {
        let mut manager = UiManager::default();
        let mut slider = spec("volume", UiKind::Slider);
        slider.range = [0.0, 10.0];
        slider.value = Evaluated::Number(7.4);
        manager.show(slider);
        drawn(&mut manager);
        assert_eq!(manager.get("volume").unwrap().value, Evaluated::Number(7.4));

        manager.set("volume", UiProp::Step, &Evaluated::Number(5.0));
        assert_eq!(manager.get("volume").unwrap().value, Evaluated::Number(5.0));
    }

    #[test]
    fn an_inputs_rules_are_written_by_name_and_survive_a_re_show() {
        let mut manager = UiManager::default();
        manager.show(spec("name", UiKind::Input));
        drawn(&mut manager);
        manager.set("name", UiProp::Allow, &Evaluated::Text("Digits".into()));
        manager.set("name", UiProp::MaxLength, &Evaluated::Number(4.0));

        // A HUD strand showing the same field again keeps them: no `show`
        // row spells either one, so a re-show has nothing to say about them.
        manager.show(spec("name", UiKind::Input));
        let node = manager.get("name").unwrap();
        assert_eq!(node.allow(), UiAllow::Digits);
        assert_eq!(node.max_length(), 4);
    }

    #[test]
    fn what_the_reporters_read_is_every_element_the_blocks_have_made() {
        let mut manager = UiManager::default();
        manager.show(spec("menu", UiKind::Panel));
        let mut label = spec("hint", UiKind::Label);
        label.parent = "menu".to_string();
        label.content = "Paused".to_string();
        manager.show(label);
        drawn(&mut manager);

        let senses = manager.senses();
        assert_eq!(senses["hint"].text, "Paused");
        assert!(senses["hint"].shown);

        // A hidden panel takes its children off the screen without
        // forgetting them: still sensed, no longer shown.
        manager.hide("menu", false);
        let senses = manager.senses();
        assert_eq!(senses.len(), 2);
        assert!(!senses["hint"].shown);
    }

    #[test]
    fn an_empty_input_shows_its_placeholder_and_a_typed_one_shows_the_typing() {
        let mut manager = UiManager::default();
        let mut input = spec("name", UiKind::Input);
        input.content = "your name".to_string();
        manager.show(input);
        drawn(&mut manager);

        assert_eq!(text_of(manager.get("name").unwrap()), "your name");
        manager.changed("name", Evaluated::Text("Ada".to_string()));
        assert_eq!(text_of(manager.get("name").unwrap()), "Ada");
    }
}

pub fn layout_node(element: &UiNode, canvas: bool) -> Node {
    use blockloom_core::ui::{UiAlign, UiLength};
    let mut node = node_for(&element.spec, !element.parent.is_empty() && !canvas);
    if element.kind == UiKind::Spacer {
        node.flex_shrink = 0.;
    }
    if let Some(layout) = &element.layout {
        let length = |v| match v {
            UiLength::Auto => Val::Auto,
            UiLength::Px(n) => Val::Px(n.max(0.)),
            UiLength::Percent(n) => Val::Percent(n.max(0.)),
        };
        if layout.width != UiLength::Auto {
            node.width = length(layout.width);
        }
        if layout.height != UiLength::Auto {
            node.height = length(layout.height);
        }
        node.min_width = Val::Px(layout.min_size[0].max(0.));
        node.min_height = Val::Px(layout.min_size[1].max(0.));
        if layout.max_size[0] > 0. {
            node.max_width = Val::Px(layout.max_size[0]);
        }
        if layout.max_size[1] > 0. {
            node.max_height = Val::Px(layout.max_size[1]);
        }
        node.padding = edges(layout.padding);
        node.margin = edges(layout.margin);
        node.row_gap = Val::Px(layout.gap);
        node.column_gap = Val::Px(layout.gap);
        node.flex_grow = layout.grow.max(0.);
        node.align_items = match layout.align {
            UiAlign::Stretch => AlignItems::Stretch,
            UiAlign::Start => AlignItems::Start,
            UiAlign::Center => AlignItems::Center,
            UiAlign::End => AlignItems::End,
        };
        if element.kind == UiKind::Grid {
            node.grid_template_columns = RepeatedGridTrack::flex(layout.columns.clamp(1, 256), 1.);
        }
        if layout.absolute {
            node.position_type = PositionType::Absolute;
            node.left = Val::Px(element.spec.offset[0]);
            node.top = Val::Px(element.spec.offset[1]);
        }
    }
    if let Some(width) = element.style.width {
        node.width = Val::Px(width);
    }
    if let Some(height) = element.style.height {
        node.height = Val::Px(height);
    }
    if let Some(padding) = element.style.padding {
        node.padding = UiRect::all(Val::Px(padding));
    }
    if !element.visible {
        node.display = Display::None;
    }
    node
}
fn edges([left, top, right, bottom]: [f32; 4]) -> UiRect {
    UiRect {
        left: Val::Px(left),
        top: Val::Px(top),
        right: Val::Px(right),
        bottom: Val::Px(bottom),
    }
}
