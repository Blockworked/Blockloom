//! Schema migration, components (prefabs) and their instances.
use super::*;
use std::collections::{BTreeMap, HashSet};

impl UiDocument {
    /// Brings an older document up to the current schema. Each step is
    /// deterministic and keeps widget IDs, order and geometry. Version 1 only
    /// stamps the version: version 0 documents already have its shape. A
    /// newer document is left alone and refused by `validate`.
    pub fn migrate(&mut self) {
        if self.version < UI_SCHEMA_VERSION {
            self.version = UI_SCHEMA_VERSION;
        }
    }

    pub(super) fn find(&self, id: &str) -> Option<usize> {
        self.widgets.iter().position(|w| w.element.id == id)
    }

    /// Indices of `id` and everything below it, in document order.
    pub(super) fn subtree(&self, id: &str) -> Vec<usize> {
        let mut ids = vec![id.to_string()];
        let mut i = 0;
        while i < ids.len() {
            let parent = ids[i].clone();
            for w in &self.widgets {
                if w.element.parent == parent && !ids.contains(&w.element.id) {
                    ids.push(w.element.id.clone());
                }
            }
            i += 1;
        }
        (0..self.widgets.len())
            .filter(|i| ids.contains(&self.widgets[*i].element.id))
            .collect()
    }

    pub(super) fn create_widget(&mut self, widget: &UiWidget) -> Result<(), String> {
        if !widget.world_actor.is_empty() {
            return Err("Projected widgets cannot be created in the editor".into());
        }
        let mut widget = widget.clone();
        let parent = widget.element.parent.clone();
        if !parent.is_empty() {
            let target = self
                .find(&parent)
                .map(|i| &self.widgets[i])
                .ok_or_else(|| format!("Unknown parent: {parent}"))?;
            if !target.world_actor.is_empty() {
                return Err("Cannot create inside a projected widget".into());
            }
            let free = target.element.kind == UiKind::Canvas;
            if free {
                widget.layout.get_or_insert_with(UiLayout::default).absolute = true;
            } else if let Some(layout) = &mut widget.layout {
                layout.absolute = false;
            }
        }
        if let Some(layout) = &widget.layout {
            layout.validate_edit()?;
        }
        self.widgets.push(widget);
        Ok(())
    }

    pub(super) fn delete_subtree(&mut self, id: &str) -> Result<(), String> {
        if self.find(id).is_none() {
            return Err(format!("Unknown widget: {id}"));
        }
        let gone = self.subtree(id);
        let removed: HashSet<String> = gone
            .iter()
            .map(|i| self.widgets[*i].element.id.clone())
            .collect();
        let mut slot = 0;
        self.widgets.retain(|_| {
            slot += 1;
            !gone.contains(&(slot - 1))
        });
        for w in &mut self.widgets {
            if removed.contains(&w.scroll_target) {
                w.scroll_target.clear();
            }
        }
        Ok(())
    }

    pub(super) fn duplicate_subtree(
        &mut self,
        id: &str,
        new_id: &str,
        offset: Option<[f32; 2]>,
    ) -> Result<(), String> {
        let root = self
            .find(id)
            .ok_or_else(|| format!("Unknown widget: {id}"))?;
        if !self.widgets[root].world_actor.is_empty() {
            return Err("Projected widgets cannot be duplicated".into());
        }
        let members = self.subtree(id);
        let rename = |old: &str| {
            if old == id {
                new_id.to_string()
            } else {
                format!("{new_id}.{old}")
            }
        };
        let old_ids: HashSet<String> = members
            .iter()
            .map(|i| self.widgets[*i].element.id.clone())
            .collect();
        let mut copies: Vec<UiWidget> = members.iter().map(|i| self.widgets[*i].clone()).collect();
        for w in &mut copies {
            if old_ids.contains(&w.element.parent) {
                w.element.parent = rename(&w.element.parent);
            }
            if old_ids.contains(&w.scroll_target) {
                w.scroll_target = rename(&w.scroll_target);
            }
            w.element.id = rename(&w.element.id);
        }
        if let Some(offset) = offset {
            let root_copy = copies.iter_mut().find(|w| w.element.id == new_id).unwrap();
            let parent = &root_copy.element.parent;
            let free = parent.is_empty()
                || root_copy.layout.as_ref().is_some_and(|l| l.absolute)
                || self
                    .find(parent)
                    .is_some_and(|i| self.widgets[i].element.kind == UiKind::Canvas);
            if !free {
                return Err("This widget is positioned by its parent layout".into());
            }
            root_copy.element.offset = offset;
        }
        let at = members.iter().max().copied().unwrap_or(root) + 1;
        self.widgets.splice(at..at, copies);
        Ok(())
    }

    pub(super) fn save_prefab(
        &mut self,
        name: &str,
        root: &str,
        update: bool,
    ) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("A prefab needs a name".into());
        }
        let root_index = self
            .find(root)
            .ok_or_else(|| format!("Unknown widget: {root}"))?;
        let members = self.subtree(root);
        if members
            .iter()
            .any(|i| !self.widgets[*i].world_actor.is_empty())
        {
            return Err("Projected widgets cannot be saved as a prefab".into());
        }
        let mut copies: Vec<UiWidget> = members.iter().map(|i| self.widgets[*i].clone()).collect();
        copies[0].element.parent.clear();
        for w in &mut copies {
            w.instance_of.clear();
        }
        // The source stays as it is; only instances elsewhere are rebuilt.
        let old = self.prefabs.insert(name.to_string(), copies);
        let _ = root_index;
        if !update {
            return Ok(());
        }
        let Some(old) = old else { return Ok(()) };
        let old_root = old
            .iter()
            .find(|w| w.element.parent.is_empty())
            .map(|w| w.element.id.clone())
            .ok_or("The old prefab has no root")?;
        let roots: Vec<String> = self
            .widgets
            .iter()
            .filter(|w| w.instance_of == name)
            .map(|w| w.element.id.clone())
            .collect();
        let mut old_doc = self.clone();
        old_doc.prefabs.insert(name.to_string(), old);
        for instance in roots {
            let Some(at) = self.find(&instance) else {
                continue;
            };
            let prefix = instance
                .strip_suffix(&old_root)
                .ok_or_else(|| format!("Instance {instance} does not follow its prefab's naming"))?
                .to_string();
            let parent = self.widgets[at].element.parent.clone();
            let members = self.subtree(&instance);
            let expected = old_doc.instantiate(name, &prefix, &parent)?;
            let fresh = self.instantiate(name, &prefix, &parent)?;
            let current: Vec<UiWidget> = members.iter().map(|i| self.widgets[*i].clone()).collect();
            let mut rebuilt: Vec<UiWidget> = Vec::new();
            for w in fresh {
                let kept = current.iter().find(|c| c.element.id == w.element.id);
                let was = expected.iter().find(|e| e.element.id == w.element.id);
                match (kept, was) {
                    (Some(c), Some(e)) if c != e => rebuilt.push(merge_widget(e, &w, c)?),
                    _ => rebuilt.push(w),
                }
            }
            for c in &current {
                let known = expected.iter().find(|e| e.element.id == c.element.id);
                let placed = rebuilt.iter().any(|r| r.element.id == c.element.id);
                if !placed && known.is_none_or(|e| e != c) {
                    rebuilt.push(c.clone());
                }
            }
            let first = members[0];
            let mut slot = 0;
            self.widgets.retain(|_| {
                slot += 1;
                !members.contains(&(slot - 1))
            });
            self.widgets.splice(first..first, rebuilt);
        }
        Ok(())
    }

    pub(super) fn expose(
        &mut self,
        prefab: &str,
        name: &str,
        widget: &str,
        path: &str,
        slot: bool,
    ) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("An exposed name cannot be empty".into());
        }
        let members = self
            .prefabs
            .get(prefab)
            .ok_or_else(|| format!("No UI prefab named {prefab}"))?;
        if !members.iter().any(|w| w.element.id == widget) {
            return Err(format!("{prefab} has no widget {widget}"));
        }
        let list = self.exports.entry(prefab.to_string()).or_default();
        if list.iter().any(|e| e.name == name) {
            return Err(format!("{prefab} already exposes {name}"));
        }
        list.push(UiExport {
            name: name.to_string(),
            widget: widget.to_string(),
            path: path.to_string(),
            slot,
        });
        Ok(())
    }

    /// The prefab export `name` of the instance at `instance`, and the id
    /// prefix its widgets carry.
    pub(super) fn exported(
        &self,
        instance: &str,
        name: &str,
        slot: bool,
    ) -> Result<(UiExport, String), String> {
        let at = self
            .find(instance)
            .ok_or_else(|| format!("Unknown widget: {instance}"))?;
        let prefab = self.widgets[at].instance_of.clone();
        let root = self
            .prefabs
            .get(&prefab)
            .and_then(|p| p.iter().find(|w| w.element.parent.is_empty()))
            .map(|w| w.element.id.clone())
            .ok_or_else(|| format!("{instance} is not the root of a component instance"))?;
        let prefix = instance
            .strip_suffix(&root)
            .ok_or_else(|| format!("Instance {instance} does not follow its prefab's naming"))?
            .to_string();
        let export = self
            .exports
            .get(&prefab)
            .and_then(|l| l.iter().find(|e| e.name == name && e.slot == slot))
            .ok_or_else(|| {
                format!(
                    "{prefab} exposes no {} called {name}",
                    if slot { "slot" } else { "property" }
                )
            })?;
        Ok((export.clone(), prefix))
    }

    /// What an instance's exposed properties currently hold, by name.
    pub fn exposed_values(
        &self,
        instance: &str,
    ) -> Result<Vec<(String, serde_json::Value)>, String> {
        let at = self
            .find(instance)
            .ok_or_else(|| format!("Unknown widget: {instance}"))?;
        let prefab = &self.widgets[at].instance_of;
        let mut out = Vec::new();
        for export in self
            .exports
            .get(prefab)
            .into_iter()
            .flatten()
            .filter(|e| !e.slot)
        {
            let (_, prefix) = self.exported(instance, &export.name, false)?;
            let Some(i) = self.find(&format!("{prefix}{}", export.widget)) else {
                continue;
            };
            let mut value = to_json(&self.widgets[i])?;
            for step in export.path.split('.') {
                value = value.get(step).cloned().unwrap_or(serde_json::Value::Null);
            }
            out.push((export.name.clone(), value));
        }
        Ok(out)
    }

    /// Rebuilds the instance rooted at `id` exactly as its prefab says.
    pub(super) fn reset_instance(&mut self, id: &str) -> Result<(), String> {
        let at = self
            .find(id)
            .ok_or_else(|| format!("Unknown widget: {id}"))?;
        let name = self.widgets[at].instance_of.clone();
        if name.is_empty() {
            return Err(format!("{id} is not the root of a component instance"));
        }
        let old_root = self
            .prefabs
            .get(&name)
            .and_then(|p| p.iter().find(|w| w.element.parent.is_empty()))
            .map(|w| w.element.id.clone())
            .ok_or_else(|| format!("No UI prefab named {name}"))?;
        let prefix = id
            .strip_suffix(&old_root)
            .ok_or_else(|| format!("Instance {id} does not follow its prefab's naming"))?
            .to_string();
        let parent = self.widgets[at].element.parent.clone();
        let members = self.subtree(id);
        let fresh = self.instantiate(&name, &prefix, &parent)?;
        let first = members[0];
        let mut slot = 0;
        self.widgets.retain(|_| {
            slot += 1;
            !members.contains(&(slot - 1))
        });
        self.widgets.splice(first..first, fresh);
        Ok(())
    }

    /// Puts one changed field of `widget` (inside the instance rooted at
    /// `instance`) back to what its prefab says.
    pub(super) fn reset_field(
        &mut self,
        instance: &str,
        widget: &str,
        path: &str,
    ) -> Result<(), String> {
        if path.is_empty() || path == "added" || path == "removed" {
            return Err(
                "Only a changed field can be reset; use Reset or Detach for the rest".into(),
            );
        }
        let at = self
            .find(instance)
            .ok_or_else(|| format!("Unknown widget: {instance}"))?;
        let name = self.widgets[at].instance_of.clone();
        let old_root = self
            .prefabs
            .get(&name)
            .and_then(|p| p.iter().find(|w| w.element.parent.is_empty()))
            .map(|w| w.element.id.clone())
            .ok_or_else(|| format!("{instance} is not the root of a component instance"))?;
        let prefix = instance
            .strip_suffix(&old_root)
            .ok_or_else(|| format!("Instance {instance} does not follow its prefab's naming"))?
            .to_string();
        if !self
            .subtree(instance)
            .iter()
            .any(|&i| self.widgets[i].element.id == widget)
        {
            return Err(format!("{widget} is not part of {instance}"));
        }
        let parent = self.widgets[at].element.parent.clone();
        let fresh = self.instantiate(&name, &prefix, &parent)?;
        let base = fresh
            .iter()
            .find(|w| w.element.id == widget)
            .ok_or_else(|| format!("{widget} is not in {name}"))?;
        let index = self
            .find(widget)
            .ok_or_else(|| format!("Unknown widget: {widget}"))?;
        let base_json = to_json(base)?;
        let mut current = to_json(&self.widgets[index])?;
        let steps: Vec<&str> = path.split('.').collect();
        let mut from = &base_json;
        for step in &steps {
            from = match from.get(*step) {
                Some(v) => v,
                None => &serde_json::Value::Null,
            };
        }
        let wanted = from.clone();
        let mut into = &mut current;
        for step in &steps[..steps.len() - 1] {
            if into.get(*step).is_none_or(|v| !v.is_object()) {
                into[*step] = serde_json::json!({});
            }
            into = into.get_mut(*step).unwrap();
        }
        let last = steps[steps.len() - 1];
        match (wanted, into.as_object_mut()) {
            (serde_json::Value::Null, Some(map)) => {
                map.remove(last);
            }
            (value, Some(map)) => {
                map.insert(last.to_string(), value);
            }
            _ => return Err(format!("{path} is not a field of {widget}")),
        }
        self.widgets[index] =
            serde_json::from_value(current).map_err(|e| format!("Could not reset {path}: {e}"))?;
        Ok(())
    }

    /// Gives `id` a new id and follows the references inside this document.
    pub(super) fn rename_widget(&mut self, id: &str, new_id: &str) -> Result<(), String> {
        let new_id = new_id.trim();
        if new_id.is_empty() {
            return Err("A widget needs a name".into());
        }
        if new_id == id {
            return Ok(());
        }
        if self.find(new_id).is_some() {
            return Err(format!("A widget named {new_id} already exists"));
        }
        let at = self
            .find(id)
            .ok_or_else(|| format!("Unknown widget: {id}"))?;
        self.widgets[at].element.id = new_id.to_string();
        for w in &mut self.widgets {
            if w.element.parent == id {
                w.element.parent = new_id.to_string();
            }
            if w.scroll_target == id {
                w.scroll_target = new_id.to_string();
            }
        }
        for animation in self.animations.values_mut() {
            for track in &mut animation.tracks {
                if track.widget == id {
                    track.widget = new_id.to_string();
                }
            }
        }
        Ok(())
    }

    /// The fields an instance changed from its prefab, as `widget id: path`.
    pub fn instance_overrides(&self, id: &str) -> Result<Vec<String>, String> {
        let at = self
            .find(id)
            .ok_or_else(|| format!("Unknown widget: {id}"))?;
        let name = &self.widgets[at].instance_of;
        let old_root = self
            .prefabs
            .get(name)
            .and_then(|p| p.iter().find(|w| w.element.parent.is_empty()))
            .map(|w| w.element.id.clone())
            .ok_or_else(|| format!("{id} is not the root of a component instance"))?;
        let prefix = id
            .strip_suffix(&old_root)
            .ok_or_else(|| format!("Instance {id} does not follow its prefab's naming"))?;
        let parent = &self.widgets[at].element.parent;
        let fresh = self.instantiate(name, prefix, parent)?;
        let mut out = Vec::new();
        for member in self.subtree(id) {
            let current = &self.widgets[member];
            match fresh.iter().find(|w| w.element.id == current.element.id) {
                Some(base) => {
                    let (a, b) = (to_json(base)?, to_json(current)?);
                    changed_paths(&a, &b, "", &mut |path| {
                        out.push(format!("{}: {path}", current.element.id))
                    });
                }
                None => out.push(format!("{}: added", current.element.id)),
            }
        }
        for w in &fresh {
            if self.find(&w.element.id).is_none() {
                out.push(format!("{}: removed", w.element.id));
            }
        }
        Ok(out)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version > UI_SCHEMA_VERSION {
            return Err(format!(
                "This interface uses schema version {} but this Blockloom understands up to {UI_SCHEMA_VERSION}; update Blockloom to edit it",
                self.version
            ));
        }
        self.tokens.validate()?;
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
        let known: HashSet<&str> = self.widgets.iter().map(|w| w.element.id.as_str()).collect();
        let mut ids = HashSet::new();
        for (name, animation) in &self.animations {
            animation
                .validate()
                .map_err(|e| format!("Animation {name}: {e}"))?;
            if let Some(t) = animation
                .tracks
                .iter()
                .find(|t| !known.contains(t.widget.as_str()))
            {
                return Err(format!(
                    "Animation {name} animates a missing widget: {}",
                    t.widget
                ));
            }
        }
        for (prefab, list) in &self.exports {
            let members = self
                .prefabs
                .get(prefab)
                .ok_or_else(|| format!("Exports for a missing prefab: {prefab}"))?;
            if list.iter().any(|e| {
                e.name.trim().is_empty() || !members.iter().any(|w| w.element.id == e.widget)
            }) {
                return Err(format!("{prefab} exposes something that is not in it"));
            }
        }
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
            if !widget.instance_of.is_empty() && !self.prefabs.contains_key(&widget.instance_of) {
                return Err(format!("{id} is an instance of a missing prefab"));
            }
            if widget.bindings.iter().any(|b| {
                !matches!(
                    b.property,
                    UiProp::Text | UiProp::Value | UiProp::Visible | UiProp::SelectedIndex
                )
            }) {
                return Err(format!("Unsupported binding property on {id}"));
            }
            check_actions(&widget.actions).map_err(|e| format!("{id}: {e}"))?;
            check_variants(&widget.variants).map_err(|e| format!("{id}: {e}"))?;
            if let Some(layout) = &widget.layout
                && (layout.columns == 0
                    || !layout.row_height.is_finite()
                    || layout.row_height <= 0.)
            {
                return Err(format!("Invalid layout on {id}"));
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
        // Each chain is walked once: ids already known to reach a root stop it.
        let parents: std::collections::HashMap<&str, &str> = self
            .widgets
            .iter()
            .map(|w| (w.element.id.as_str(), w.element.parent.as_str()))
            .collect();
        let mut verified: HashSet<&str> = HashSet::new();
        for widget in &self.widgets {
            let mut seen: HashSet<&str> = HashSet::new();
            let mut current = widget.element.id.as_str();
            while !verified.contains(current) {
                if !seen.insert(current) {
                    return Err(format!("Widget parent cycle: {current}"));
                }
                let parent = parents[current];
                if parent.is_empty() {
                    break;
                }
                if !parents.contains_key(parent) {
                    return Err(format!("Missing widget parent: {parent}"));
                }
                current = parent;
            }
            verified.extend(seen);
        }
        Ok(())
    }
    pub fn with_stylesheets(&self, dir: &std::path::Path) -> Result<Self, String> {
        let mut result = self.clone();
        let mut styles = BTreeMap::new();
        let mut tokens = UiTokens::default();
        for path in &self.stylesheets {
            let full = crate::assets::resolve(dir, path)
                .ok_or_else(|| format!("Invalid stylesheet path: {path}"))?;
            let text = crate::vfs::read_to_string(&full).map_err(|e| format!("{path}: {e}"))?;
            let is_theme = path
                .rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(STYLE_EXTENSION));
            if is_theme {
                let file: UiStyleFile =
                    serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
                if file.version > STYLE_VERSION {
                    return Err(format!(
                        "{path} was saved by a newer Blockloom (style format {})",
                        file.version
                    ));
                }
                file.tokens.validate().map_err(|e| format!("{path}: {e}"))?;
                result.theme = file.base;
                tokens.layer(file.tokens);
                styles.extend(file.classes);
            } else {
                let sheet: BTreeMap<String, UiStyles> =
                    serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
                styles.extend(sheet);
            }
        }
        styles.extend(self.styles.clone());
        tokens.layer(self.tokens.clone());
        result.styles = styles;
        result.tokens = tokens;
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
                w.instance_of = name.into();
                parent.into()
            } else {
                format!("{prefix}{}", w.element.parent)
            };
        }
        Ok(widgets)
    }
}
