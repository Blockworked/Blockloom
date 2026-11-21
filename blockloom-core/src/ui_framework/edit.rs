//! Typed property edits, the edit enum and applying it to a document.
use super::*;
use std::collections::HashSet;

/// Canonical authored property paths with their accepted value types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "path", content = "value", deny_unknown_fields)]
pub enum UiPropertyEdit {
    #[serde(rename = "element.kind")]
    Kind(UiKind),
    #[serde(rename = "element.content")]
    Content(String),
    #[serde(rename = "element.anchor")]
    Anchor(UiAnchor),
    #[serde(rename = "element.modal")]
    Modal(bool),
    #[serde(rename = "layout")]
    Layout(#[serde(deserialize_with = "deserialize_edit_layout")] Option<UiLayout>),
    #[serde(rename = "element.range")]
    Range([f32; 2]),
    #[serde(rename = "style")]
    Style(Box<UiStyles>),
    #[serde(rename = "class")]
    Class(String),
    #[serde(rename = "bindings")]
    Bindings(Vec<UiBinding>),
    #[serde(rename = "actions")]
    Actions(Vec<UiAction>),
    #[serde(rename = "variants")]
    Variants(Vec<UiVariant>),
    #[serde(rename = "items")]
    Items(Vec<String>),
    #[serde(rename = "tooltip")]
    Tooltip(String),
    #[serde(rename = "scroll_target")]
    ScrollTarget(String),
    #[serde(rename = "tab_index")]
    TabIndex(Option<usize>),
    #[serde(rename = "transition")]
    Transition(f32),
    #[serde(rename = "fx")]
    Fx(UiFx),
    #[serde(rename = "world_actor")]
    WorldActor(String),
}

/// How the inspector should draw a property path.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UiPropertyInfo {
    pub path: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    /// `choice`, `text`, `bool` or `layout`.
    pub ty: &'static str,
    /// Choices for a `choice`; empty otherwise.
    pub choices: Vec<String>,
    /// Kinds the property applies to; empty means every kind.
    pub kinds: Vec<&'static str>,
}

impl UiPropertyEdit {
    pub fn path(&self) -> &'static str {
        match self {
            UiPropertyEdit::Kind(_) => "element.kind",
            UiPropertyEdit::Content(_) => "element.content",
            UiPropertyEdit::Anchor(_) => "element.anchor",
            UiPropertyEdit::Modal(_) => "element.modal",
            UiPropertyEdit::Layout(_) => "layout",
            UiPropertyEdit::Range(_) => "element.range",
            UiPropertyEdit::Style(_) => "style",
            UiPropertyEdit::Class(_) => "class",
            UiPropertyEdit::Bindings(_) => "bindings",
            UiPropertyEdit::Actions(_) => "actions",
            UiPropertyEdit::Variants(_) => "variants",
            UiPropertyEdit::Items(_) => "items",
            UiPropertyEdit::Tooltip(_) => "tooltip",
            UiPropertyEdit::ScrollTarget(_) => "scroll_target",
            UiPropertyEdit::TabIndex(_) => "tab_index",
            UiPropertyEdit::Transition(_) => "transition",
            UiPropertyEdit::Fx(_) => "fx",
            UiPropertyEdit::WorldActor(_) => "world_actor",
        }
    }
}

/// One entry per `UiPropertyEdit` path; a test keeps them in step.
pub fn property_metadata() -> Vec<UiPropertyInfo> {
    let names = |values: serde_json::Value| -> Vec<String> {
        values
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let kinds = names(
        serde_json::to_value([
            UiKind::Panel,
            UiKind::Label,
            UiKind::Button,
            UiKind::Image,
            UiKind::Input,
            UiKind::Slider,
            UiKind::Toggle,
            UiKind::List,
            UiKind::VerticalBox,
            UiKind::HorizontalBox,
            UiKind::Grid,
            UiKind::Canvas,
            UiKind::WrapBox,
            UiKind::SizeBox,
            UiKind::Spacer,
            UiKind::Progress,
            UiKind::RadialProgress,
            UiKind::ListView,
            UiKind::Tabs,
            UiKind::Select,
            UiKind::Scrollbar,
            UiKind::RichText,
            UiKind::Tooltip,
        ])
        .unwrap_or_default(),
    );
    let anchors = names(
        serde_json::to_value([
            UiAnchor::TopLeft,
            UiAnchor::Top,
            UiAnchor::TopRight,
            UiAnchor::Left,
            UiAnchor::Center,
            UiAnchor::Right,
            UiAnchor::BottomLeft,
            UiAnchor::Bottom,
            UiAnchor::BottomRight,
        ])
        .unwrap_or_default(),
    );
    let info =
        |path, label, group, ty, choices: Vec<String>, kinds: Vec<&'static str>| UiPropertyInfo {
            path,
            label,
            group,
            ty,
            choices,
            kinds,
        };
    vec![
        info("element.kind", "Type", "Content", "choice", kinds, vec![]),
        info(
            "element.content",
            "Content",
            "Content",
            "text",
            vec![],
            vec![],
        ),
        info(
            "element.anchor",
            "Anchor",
            "Layout",
            "choice",
            anchors,
            vec![],
        ),
        info(
            "element.modal",
            "Modal",
            "Interaction",
            "bool",
            vec![],
            vec!["Panel"],
        ),
        info("layout", "Layout", "Layout", "layout", vec![], vec![]),
        info(
            "element.range",
            "Range",
            "Content",
            "range",
            vec![],
            vec!["Slider", "Progress", "RadialProgress", "Scrollbar"],
        ),
        info("style", "Style", "Appearance", "style", vec![], vec![]),
        info("class", "Style class", "Appearance", "text", vec![], vec![]),
        info(
            "bindings",
            "Bindings",
            "Bindings",
            "bindings",
            vec![],
            vec![],
        ),
        info("actions", "Events", "Bindings", "actions", vec![], vec![]),
        info(
            "variants",
            "Responsive variants",
            "Layout",
            "variants",
            vec![],
            vec![],
        ),
        info(
            "items",
            "Items",
            "Content",
            "lines",
            vec![],
            vec!["List", "ListView", "Select", "Tabs"],
        ),
        info("tooltip", "Tooltip", "Interaction", "text", vec![], vec![]),
        info(
            "world_actor",
            "World actor",
            "Interaction",
            "text",
            vec![],
            vec![],
        ),
        info(
            "scroll_target",
            "Scroll target",
            "Interaction",
            "widget",
            vec![],
            vec!["Scrollbar"],
        ),
        info(
            "tab_index",
            "Tab page",
            "Interaction",
            "int",
            vec![],
            vec![],
        ),
        info(
            "transition",
            "Transition seconds",
            "Appearance",
            "number",
            vec![],
            vec![],
        ),
        info(
            "fx",
            "Opacity, offset and rotation",
            "Appearance",
            "fx",
            vec![],
            vec![],
        ),
    ]
}

fn deserialize_edit_layout<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<UiLayout>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let fields = [
        "width",
        "height",
        "min_size",
        "max_size",
        "margin",
        "padding",
        "gap",
        "columns",
        "grow",
        "align",
        "absolute",
        "row_height",
    ];
    if let Some(object) = value.as_object()
        && let Some(key) = object.keys().find(|key| !fields.contains(&key.as_str()))
    {
        return Err(serde::de::Error::custom(format!(
            "Unknown layout property: {key}"
        )));
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Free placement uses explicit pixel dimensions; flow retains authored sizing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum UiPlacement {
    Free { offset: [f32; 2], size: [f32; 2] },
    Flow,
}

/// Absolute authored values, applied to a transaction's starting document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum UiEdit {
    Reorder {
        id: String,
        index: usize,
    },
    SetProperty {
        id: String,
        property: UiPropertyEdit,
    },
    Reparent {
        id: String,
        parent: String,
        placement: UiPlacement,
    },
    /// Applies several edits in order as one; any failure rejects them all.
    Batch {
        edits: Vec<UiEdit>,
    },
    /// Sets document-wide options; omitted fields stay as they are.
    SetDocument {
        #[serde(default)]
        theme: Option<UiTheme>,
        #[serde(default)]
        scale: Option<UiScale>,
    },
    /// Defines or (with null) removes a shared named style.
    SetClass {
        name: String,
        styles: Option<Box<UiStyles>>,
    },
    /// Appends a new widget; a Canvas parent makes it free, any other parent flow.
    Create {
        widget: Box<UiWidget>,
    },
    /// Removes a widget and its descendants.
    Delete {
        id: String,
    },
    /// Copies a subtree after the original under fresh ids (`new_id` for the
    /// root, `new_id.old` below it). `offset` moves a free-placed root.
    Duplicate {
        id: String,
        new_id: String,
        #[serde(default)]
        offset: Option<[f32; 2]>,
    },
    Move {
        id: String,
        offset: [f32; 2],
    },
    Resize {
        id: String,
        size: [f32; 2],
        offset: [f32; 2],
    },
    /// Stores the subtree at `root` as the prefab `name` (replacing one of that
    /// name). With `update_instances`, every instance is rebuilt from the new
    /// version; widgets an instance changed from the old version are kept.
    SavePrefab {
        name: String,
        root: String,
        #[serde(default)]
        update_instances: bool,
    },
    /// Forgets a prefab; its instances stay as plain widgets.
    DeletePrefab {
        name: String,
    },
    /// Adds a prefab's widgets under `parent`, every id prefixed by `prefix`.
    InstantiatePrefab {
        name: String,
        prefix: String,
        #[serde(default)]
        parent: String,
    },
    /// Turns an instance back into plain widgets.
    DetachInstance {
        id: String,
    },
    /// Throws away every change an instance made and rebuilds it from its prefab.
    ResetInstance {
        id: String,
    },
    /// Puts one changed field of a widget inside the instance rooted at
    /// `instance` back to what its prefab says. `path` is one of the paths
    /// `instance_overrides` lists, such as `element.offset`.
    ResetField {
        instance: String,
        widget: String,
        path: String,
    },
    /// Gives a widget a new id, following the parents, scroll targets and
    /// animation tracks inside this document that name it. Blocks and scripts
    /// that name the old id are not rewritten.
    Rename {
        id: String,
        new_id: String,
    },
    /// Stores (or with `None`, removes) the animation `name`.
    SetAnimation {
        name: String,
        animation: Option<UiAnimation>,
    },
    /// Lets instances of `prefab` set `path` of its widget `widget` as `name`.
    ExposeProperty {
        prefab: String,
        name: String,
        widget: String,
        path: String,
    },
    /// Lets instances of `prefab` take widgets inside its container `widget`.
    ExposeSlot {
        prefab: String,
        name: String,
        widget: String,
    },
    Unexpose {
        prefab: String,
        name: String,
    },
    /// Sets an exposed property on the instance rooted at `instance`.
    SetExposed {
        instance: String,
        name: String,
        value: serde_json::Value,
    },
    /// Moves `widget` into the instance's slot `slot`.
    FillSlot {
        instance: String,
        slot: String,
        widget: String,
    },
}

impl UiDocument {
    pub fn apply_edit(&mut self, edit: &UiEdit) -> Result<(), String> {
        *self = self.with_edit(edit)?;
        Ok(())
    }

    /// The document after `edit`, leaving this one untouched. One clone and
    /// one validation, so a drag preview pays for each once per pointer move.
    pub fn with_edit(&self, edit: &UiEdit) -> Result<Self, String> {
        let mut next = self.clone();
        next.apply_edit_inner(edit)?;
        next.validate()?;
        Ok(next)
    }

    pub(super) fn apply_edit_inner(&mut self, edit: &UiEdit) -> Result<(), String> {
        match edit {
            UiEdit::Batch { edits } => {
                if edits.len() > 1000 || edits.iter().any(|e| matches!(e, UiEdit::Batch { .. })) {
                    return Err("A batch holds at most 1000 edits and cannot nest".into());
                }
                for edit in edits {
                    self.apply_edit_inner(edit)?;
                }
                return Ok(());
            }
            UiEdit::SetDocument { theme, scale } => {
                if let Some(theme) = theme {
                    self.theme = *theme;
                }
                if let Some(scale) = scale {
                    self.scale = *scale;
                }
                return Ok(());
            }
            UiEdit::SetClass { name, styles } => {
                if name.trim().is_empty() {
                    return Err("A style class needs a name".into());
                }
                match styles {
                    Some(styles) => {
                        self.styles.insert(name.clone(), (**styles).clone());
                    }
                    None => {
                        self.styles.remove(name);
                    }
                }
                return Ok(());
            }
            UiEdit::Create { widget } => return self.create_widget(widget),
            UiEdit::Delete { id } => return self.delete_subtree(id),
            UiEdit::Duplicate { id, new_id, offset } => {
                return self.duplicate_subtree(id, new_id, *offset);
            }
            UiEdit::SavePrefab {
                name,
                root,
                update_instances,
            } => return self.save_prefab(name, root, *update_instances),
            UiEdit::DeletePrefab { name } => {
                if self.prefabs.remove(name).is_none() {
                    return Err(format!("No UI prefab named {name}"));
                }
                self.exports.remove(name);
                for w in &mut self.widgets {
                    if &w.instance_of == name {
                        w.instance_of.clear();
                    }
                }
                return Ok(());
            }
            UiEdit::InstantiatePrefab {
                name,
                prefix,
                parent,
            } => {
                for widget in self.instantiate(name, prefix, parent)? {
                    self.create_widget(&widget)?;
                }
                return Ok(());
            }
            UiEdit::DetachInstance { id } => {
                let i = self
                    .find(id)
                    .ok_or_else(|| format!("Unknown widget: {id}"))?;
                self.widgets[i].instance_of.clear();
                return Ok(());
            }
            UiEdit::ResetInstance { id } => return self.reset_instance(id),
            UiEdit::ResetField {
                instance,
                widget,
                path,
            } => return self.reset_field(instance, widget, path),
            UiEdit::Rename { id, new_id } => return self.rename_widget(id, new_id),
            UiEdit::SetAnimation { name, animation } => {
                if name.trim().is_empty() {
                    return Err("An animation needs a name".into());
                }
                match animation {
                    Some(animation) => {
                        animation.validate()?;
                        if let Some(missing) = animation
                            .tracks
                            .iter()
                            .find(|t| self.find(&t.widget).is_none())
                        {
                            return Err(format!("Unknown widget: {}", missing.widget));
                        }
                        self.animations.insert(name.clone(), animation.clone());
                    }
                    None => {
                        if self.animations.remove(name).is_none() {
                            return Err(format!("No UI animation named {name}"));
                        }
                    }
                }
                return Ok(());
            }
            UiEdit::ExposeProperty {
                prefab,
                name,
                widget,
                path,
            } => {
                if !property_metadata().iter().any(|m| m.path == path.as_str()) {
                    return Err(format!("Unknown property path: {path}"));
                }
                return self.expose(prefab, name, widget, path, false);
            }
            UiEdit::ExposeSlot {
                prefab,
                name,
                widget,
            } => return self.expose(prefab, name, widget, "", true),
            UiEdit::Unexpose { prefab, name } => {
                let list = self.exports.get_mut(prefab);
                let before = list.as_ref().map_or(0, |l| l.len());
                if let Some(list) = list {
                    list.retain(|e| &e.name != name);
                    if list.is_empty() {
                        self.exports.remove(prefab);
                    }
                }
                let after = self.exports.get(prefab).map_or(0, |l| l.len());
                return if before == after {
                    Err(format!("{prefab} exposes nothing called {name}"))
                } else {
                    Ok(())
                };
            }
            UiEdit::SetExposed {
                instance,
                name,
                value,
            } => {
                let (export, prefix) = self.exported(instance, name, false)?;
                let property = serde_json::from_value(
                    serde_json::json!({"path": export.path, "value": value}),
                )
                .map_err(|e| format!("{name}: {e}"))?;
                return self.apply_edit(&UiEdit::SetProperty {
                    id: format!("{prefix}{}", export.widget),
                    property,
                });
            }
            UiEdit::FillSlot {
                instance,
                slot,
                widget,
            } => {
                let (export, prefix) = self.exported(instance, slot, true)?;
                let parent = format!("{prefix}{}", export.widget);
                let canvas = self
                    .find(&parent)
                    .is_some_and(|i| self.widgets[i].element.kind == UiKind::Canvas);
                let placement = if canvas {
                    let size = self
                        .find(widget)
                        .map_or([100., 40.], |i| self.widgets[i].element.size);
                    UiPlacement::Free {
                        offset: [0.; 2],
                        size: [size[0].max(1.), size[1].max(1.)],
                    }
                } else {
                    UiPlacement::Flow
                };
                return self.apply_edit(&UiEdit::Reparent {
                    id: widget.clone(),
                    parent,
                    placement,
                });
            }
            _ => {}
        }
        let id = match edit {
            UiEdit::Move { id, .. }
            | UiEdit::Resize { id, .. }
            | UiEdit::SetProperty { id, .. }
            | UiEdit::Reparent { id, .. }
            | UiEdit::Reorder { id, .. } => id,
            UiEdit::Batch { .. }
            | UiEdit::SetDocument { .. }
            | UiEdit::SetClass { .. }
            | UiEdit::Create { .. }
            | UiEdit::Delete { .. }
            | UiEdit::Duplicate { .. }
            | UiEdit::SavePrefab { .. }
            | UiEdit::DeletePrefab { .. }
            | UiEdit::InstantiatePrefab { .. }
            | UiEdit::DetachInstance { .. }
            | UiEdit::ResetInstance { .. }
            | UiEdit::ResetField { .. }
            | UiEdit::Rename { .. }
            | UiEdit::SetAnimation { .. }
            | UiEdit::ExposeProperty { .. }
            | UiEdit::ExposeSlot { .. }
            | UiEdit::Unexpose { .. }
            | UiEdit::SetExposed { .. }
            | UiEdit::FillSlot { .. } => unreachable!(),
        };
        let index = self
            .widgets
            .iter()
            .position(|w| &w.element.id == id)
            .ok_or_else(|| format!("Unknown widget: {id}"))?;
        if let UiEdit::Reorder { index: target, .. } = edit {
            let parent = &self.widgets[index].element.parent;
            let slots: Vec<_> = self
                .widgets
                .iter()
                .enumerate()
                .filter(|(_, w)| &w.element.parent == parent)
                .map(|(i, _)| i)
                .collect();
            if *target >= slots.len() {
                return Err("Sibling index is out of range".into());
            }
            let current = slots.iter().position(|slot| *slot == index).unwrap();
            let mut siblings: Vec<_> = slots
                .iter()
                .map(|slot| self.widgets[*slot].clone())
                .collect();
            let widget = siblings.remove(current);
            siblings.insert(*target, widget);
            for (slot, widget) in slots.into_iter().zip(siblings) {
                self.widgets[slot] = widget;
            }
            return Ok(());
        }
        if let UiEdit::SetProperty { property, .. } = edit {
            let ids: HashSet<String> = self.widgets.iter().map(|w| w.element.id.clone()).collect();
            let w = &mut self.widgets[index];
            match property {
                UiPropertyEdit::Kind(value) => w.element.kind = *value,
                UiPropertyEdit::Content(value) => w.element.content = value.clone(),
                UiPropertyEdit::Anchor(value) => w.element.anchor = *value,
                UiPropertyEdit::Modal(value) => w.element.modal = *value,
                UiPropertyEdit::Layout(value) => {
                    if let Some(layout) = value {
                        layout.validate_edit()?;
                    }
                    w.layout = value.clone();
                }
                UiPropertyEdit::Range(value) => {
                    if value.iter().any(|n| !n.is_finite()) || value[0] >= value[1] {
                        return Err("Range must be finite with min below max".into());
                    }
                    w.element.range = *value;
                }
                UiPropertyEdit::Style(value) => {
                    let bad = [
                        &value.normal,
                        &value.hover,
                        &value.pressed,
                        &value.disabled,
                        &value.focused,
                    ]
                    .into_iter()
                    .any(|p| {
                        [p.text_size, p.border_width, p.radius]
                            .into_iter()
                            .flatten()
                            .any(|n| !n.is_finite() || n < 0.)
                    });
                    if bad {
                        return Err("Style sizes must be finite and not negative".into());
                    }
                    for p in [
                        &value.normal,
                        &value.hover,
                        &value.pressed,
                        &value.disabled,
                        &value.focused,
                    ] {
                        if let Some(fit) = &p.image_fit {
                            fit.validate()?;
                        }
                    }
                    w.style = (**value).clone();
                }
                UiPropertyEdit::Class(value) => w.class = value.clone(),
                UiPropertyEdit::Bindings(value) => w.bindings = value.clone(),
                UiPropertyEdit::Variants(value) => {
                    check_variants(value)?;
                    w.variants = value.clone();
                }
                UiPropertyEdit::Actions(value) => {
                    check_actions(value)?;
                    w.actions = value.clone();
                }
                UiPropertyEdit::Items(value) => w.items = value.clone(),
                UiPropertyEdit::Tooltip(value) => w.tooltip = value.clone(),
                UiPropertyEdit::WorldActor(value) => w.world_actor = value.trim().to_string(),
                UiPropertyEdit::ScrollTarget(value) => {
                    if !value.is_empty() && !ids.contains(value.as_str()) {
                        return Err(format!("Unknown scroll target: {value}"));
                    }
                    w.scroll_target = value.clone();
                }
                UiPropertyEdit::TabIndex(value) => w.tab_index = *value,
                UiPropertyEdit::Transition(value) => {
                    if !value.is_finite() || *value < 0. {
                        return Err("Transition must be finite and not negative".into());
                    }
                    w.transition = *value;
                }
                UiPropertyEdit::Fx(value) => {
                    value.validate()?;
                    w.fx = *value;
                }
            }
            return Ok(());
        }
        if let UiEdit::Reparent {
            parent, placement, ..
        } = edit
        {
            if !self.widgets[index].world_actor.is_empty() {
                return Err("Projected widgets cannot be reparented".into());
            }
            if !parent.is_empty() {
                let target = self
                    .widgets
                    .iter()
                    .find(|w| &w.element.id == parent)
                    .ok_or_else(|| format!("Unknown parent: {parent}"))?;
                if !target.world_actor.is_empty() {
                    return Err("Cannot reparent into a projected widget".into());
                }
                if matches!(placement, UiPlacement::Free { .. })
                    && target.element.kind != UiKind::Canvas
                {
                    return Err("Free placement requires a Canvas parent".into());
                }
                if matches!(placement, UiPlacement::Flow) && target.element.kind == UiKind::Canvas {
                    return Err("Canvas children require free placement".into());
                }
            } else if matches!(placement, UiPlacement::Flow) {
                return Err("Flow placement requires a parent".into());
            }
            let w = &mut self.widgets[index];
            w.element.parent = parent.clone();
            match placement {
                UiPlacement::Free { offset, size } => {
                    if offset.iter().any(|n| !n.is_finite())
                        || size.iter().any(|n| !n.is_finite() || *n <= 0.)
                    {
                        return Err("Invalid free placement dimensions".into());
                    }
                    w.element.offset = *offset;
                    w.element.size = *size;
                    let layout = w.layout.get_or_insert_with(UiLayout::default);
                    layout.absolute = true;
                    layout.margin = [0.; 4];
                    layout.width = UiLength::Px(size[0]);
                    layout.height = UiLength::Px(size[1]);
                }
                UiPlacement::Flow => {
                    w.element.offset = [0.; 2];
                    if let Some(layout) = &mut w.layout {
                        layout.absolute = false;
                    }
                }
            }
            return Ok(());
        }
        let offset = match edit {
            UiEdit::Move { offset, .. } | UiEdit::Resize { offset, .. } => offset,
            _ => unreachable!(),
        };
        if offset.iter().any(|v| !v.is_finite()) {
            return Err("Invalid widget offset".into());
        }
        let widget = &self.widgets[index];
        if !widget.world_actor.is_empty() {
            return Err("Projected widgets cannot be moved or resized".into());
        }
        if !widget.element.parent.is_empty()
            && widget.layout.as_ref().is_none_or(|l| !l.absolute)
            && self
                .widgets
                .iter()
                .find(|w| w.element.id == widget.element.parent)
                .is_none_or(|w| w.element.kind != UiKind::Canvas)
        {
            return Err("This widget is positioned by its parent layout".into());
        }
        if let UiEdit::Resize { size, .. } = edit
            && size.iter().any(|v| !v.is_finite() || *v <= 0.)
        {
            return Err("Widget size must be finite and positive".into());
        }
        let widget = &mut self.widgets[index];
        widget.element.offset = *offset;
        if let UiEdit::Resize { size, .. } = edit {
            widget.element.size = *size;
            if let Some(layout) = &mut widget.layout {
                layout.width = UiLength::Px(size[0]);
                layout.height = UiLength::Px(size[1]);
            }
        }
        Ok(())
    }
}

/// What changed in a document since a base the caller already holds: every
/// field but the widgets, the widget ids in order, and the widgets that differ.
/// A drag then costs what it changed, not the whole document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPatch {
    /// The document with its widget list left empty.
    pub document: UiDocument,
    /// Every widget id, in document order.
    pub order: Vec<String>,
    /// Widgets that are new or differ from the base.
    pub widgets: Vec<UiWidget>,
}

impl UiDocument {
    pub fn patch_from(&self, base: &UiDocument) -> UiPatch {
        let before: std::collections::HashMap<&str, &UiWidget> = base
            .widgets
            .iter()
            .map(|w| (w.element.id.as_str(), w))
            .collect();
        let mut document = self.clone();
        document.widgets = Vec::new();
        UiPatch {
            document,
            order: self.widgets.iter().map(|w| w.element.id.clone()).collect(),
            widgets: self
                .widgets
                .iter()
                .filter(|w| before.get(w.element.id.as_str()) != Some(w))
                .cloned()
                .collect(),
        }
    }
}

impl UiPatch {
    /// The document the patch describes, given the base it was made against.
    pub fn apply(&self, base: &UiDocument) -> Result<UiDocument, String> {
        let mut known: std::collections::HashMap<&str, &UiWidget> = base
            .widgets
            .iter()
            .map(|w| (w.element.id.as_str(), w))
            .collect();
        for w in &self.widgets {
            known.insert(w.element.id.as_str(), w);
        }
        let mut document = self.document.clone();
        document.widgets = self
            .order
            .iter()
            .map(|id| {
                known
                    .get(id.as_str())
                    .map(|w| (*w).clone())
                    .ok_or_else(|| format!("Patch names an unknown widget: {id}"))
            })
            .collect::<Result<_, _>>()?;
        Ok(document)
    }
}
