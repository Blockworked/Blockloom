//! Prefabs: an actor saved as a `.blockprefab` resource, placed any number
//! of times. An instance is a whole actor carrying a `Prefab` component
//! (its source and the property paths it overrides), so the runtime and
//! every other system treat it as any other actor. Editing the prefab
//! rebuilds each instance from the prefab plus that instance's overrides.
//!
//! A property path is one top-level field: `Look.visual`, `Body.physics`,
//! `Collider#1.collider`, or a graph key such as `strands`. Where the actor
//! stands (`Place`), what it hangs off (`Parent`) and its name always stay
//! the instance's own.

use crate::components::ActorComponent;
use crate::project::Actor;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;

pub const EXTENSION: &str = "blockprefab";
const FORMAT: u32 = 1;

/// What an instance knows about its prefab.
pub type PrefabLink = crate::linked::AssetLink;

#[derive(Serialize, Deserialize)]
struct PrefabFile {
    #[serde(default)]
    format: u32,
    #[serde(rename = "type", default)]
    kind: String,
    actor: Actor,
}

pub fn is_prefab(path: &str) -> bool {
    path.to_lowercase().ends_with(&format!(".{EXTENSION}"))
}

pub fn read(dir: &Path, relative: &str) -> Result<Actor, String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let text = crate::vfs::read_to_string(&full).map_err(|e| format!("{relative}: {e}"))?;
    let file: PrefabFile = serde_json::from_str(&text).map_err(|e| format!("{relative}: {e}"))?;
    if file.format > FORMAT {
        return Err(format!(
            "{relative} was saved by a newer Blockloom (prefab format {})",
            file.format
        ));
    }
    Ok(file.actor)
}

/// Writes `actor` as the prefab at `relative`, without its own link.
pub fn write(dir: &Path, relative: &str, actor: &Actor) -> Result<(), String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let mut actor = actor.clone();
    actor
        .components
        .0
        .retain(|c| !matches!(c, ActorComponent::Prefab { .. }));
    let file = PrefabFile {
        format: FORMAT,
        kind: "prefab".into(),
        actor,
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    text.push('\n');
    crate::files::write_atomic(&full, text)
}

/// The link on `actor`, if it is an instance.
pub fn link(actor: &Actor) -> Option<&PrefabLink> {
    actor.components.iter().find_map(|c| match c {
        ActorComponent::Prefab { prefab } => Some(prefab),
        _ => None,
    })
}

pub fn link_mut(actor: &mut Actor) -> Option<&mut PrefabLink> {
    actor.components.iter_mut().find_map(|c| match c {
        ActorComponent::Prefab { prefab } => Some(prefab),
        _ => None,
    })
}

/// Paths that are always the instance's own.
fn own(path: &str) -> bool {
    path == "name"
        || path
            .split('.')
            .next()
            .is_some_and(|base| base == "Place" || base == "Parent")
}

/// One component's slot: its path base and its JSON tag.
struct Slot {
    base: String,
    tag: String,
}

/// An actor as property paths, plus its components' order.
fn fields(actor: &Actor) -> (BTreeMap<String, Value>, Vec<Slot>) {
    let mut out = BTreeMap::new();
    let mut slots = Vec::new();
    let Ok(Value::Object(object)) = serde_json::to_value(actor) else {
        return (out, slots);
    };
    for (key, value) in &object {
        if key != "id" && key != "components" {
            out.insert(key.clone(), value.clone());
        }
    }
    let components = object
        .get("components")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (typed, value) in actor.components.iter().zip(components) {
        if matches!(typed, ActorComponent::Prefab { .. }) {
            continue;
        }
        let name = typed.name().to_string();
        let count = seen.entry(name.clone()).or_default();
        let base = if *count == 0 {
            name
        } else {
            format!("{name}#{count}")
        };
        *count += 1;
        let Value::Object(map) = value else {
            continue;
        };
        let tag = map
            .get("component")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut any = false;
        for (key, value) in map {
            if key != "component" {
                out.insert(format!("{base}.{key}"), value);
                any = true;
            }
        }
        if !any {
            out.insert(base.clone(), Value::Bool(true));
        }
        slots.push(Slot { base, tag });
    }
    (out, slots)
}

/// Updates `overrides` after an edit took the instance from `before` to
/// `now`: a changed path that differs from the prefab is overridden, one
/// put back to the prefab's value is not.
pub fn mark(before: &Actor, now: &Actor, prefab: &Actor, overrides: &mut Vec<String>) {
    let (before, _) = fields(before);
    let (now, _) = fields(now);
    let (source, _) = fields(prefab);
    let keys: std::collections::BTreeSet<&String> = before.keys().chain(now.keys()).collect();
    for key in keys {
        if own(key) || before.get(key) == now.get(key) {
            continue;
        }
        let at = overrides.binary_search(key);
        let matches = now.get(key) == source.get(key);
        match (matches, at) {
            (true, Ok(i)) => {
                overrides.remove(i);
            }
            (false, Err(i)) => overrides.insert(i, key.clone()),
            _ => {}
        }
    }
}

/// The instance as `prefab` says now, keeping its own id, link, place and
/// every overridden path.
pub fn rebuild(instance: &Actor, prefab: &Actor) -> Result<Actor, String> {
    let overrides = link(instance)
        .map(|l| l.overrides.clone())
        .unwrap_or_default();
    let keep = |key: &str| own(key) || overrides.binary_search(&key.to_string()).is_ok();
    let (mine, my_slots) = fields(instance);
    let (theirs, their_slots) = fields(prefab);
    let mut merged: BTreeMap<String, Value> = BTreeMap::new();
    for (key, value) in &theirs {
        if keep(key) {
            if let Some(own) = mine.get(key) {
                merged.insert(key.clone(), own.clone());
            }
        } else {
            merged.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in &mine {
        if !theirs.contains_key(key) && keep(key) {
            merged.insert(key.clone(), value.clone());
        }
    }
    let mut object = Map::new();
    object.insert("id".into(), json!(instance.id));
    let mut slots: Vec<&Slot> = their_slots.iter().collect();
    for slot in &my_slots {
        if !slots.iter().any(|s| s.base == slot.base) {
            slots.push(slot);
        }
    }
    let mut components = Vec::new();
    for slot in slots {
        let mut component = Map::new();
        let mut any = merged.remove(&slot.base).is_some();
        let prefix = format!("{}.", slot.base);
        let keys: Vec<String> = merged
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in keys {
            if let Some(value) = merged.remove(&key) {
                component.insert(key[prefix.len()..].to_string(), value);
                any = true;
            }
        }
        if any {
            component.insert("component".into(), json!(slot.tag));
            components.push(Value::Object(component));
        }
    }
    // Whatever is left is the actor's own keys: its name and graph.
    for (key, value) in merged {
        if !key.contains('.') {
            object.insert(key, value);
        }
    }
    object.insert("components".into(), Value::Array(components));
    let mut actor: Actor = serde_json::from_value(Value::Object(object)).map_err(|e| {
        format!(
            "couldn't rebuild \"{}\" from its prefab: {e}",
            instance.name
        )
    })?;
    if let Some(own_link) = link(instance).cloned() {
        actor
            .components
            .0
            .push(ActorComponent::Prefab { prefab: own_link });
    }
    Ok(actor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use crate::scene::{Mode, Visual};

    fn look(actor: &mut Actor, color: &str) {
        for component in actor.components.iter_mut() {
            if let ActorComponent::Look { visual } = component {
                *visual = Visual::Rect {
                    color: color.into(),
                    size: [10.0, 10.0],
                };
            }
        }
    }

    fn color(actor: &Actor) -> String {
        actor
            .visual()
            .map(|v| match v {
                Visual::Rect { color, .. } => color.clone(),
                _ => String::new(),
            })
            .unwrap_or_default()
    }

    #[test]
    fn an_override_survives_a_prefab_edit_and_the_rest_follows() {
        let project = Project::starter("P", Mode::TwoD);
        let mut prefab = project.active_scene().actors[0].clone();
        look(&mut prefab, "#FF0000");
        let mut instance = prefab.clone();
        instance.id = "instance".into();
        instance.components.0.push(ActorComponent::Prefab {
            prefab: PrefabLink::default(),
        });

        // Recolor the instance: an override.
        let before = instance.clone();
        look(&mut instance, "#00FF00");
        let mut overrides = Vec::new();
        mark(&before, &instance, &prefab, &mut overrides);
        assert_eq!(overrides, ["Look.visual"]);
        link_mut(&mut instance).unwrap().overrides = overrides.clone();

        // The prefab changes its look and gains a component.
        let mut next = prefab.clone();
        look(&mut next, "#0000FF");
        next.components.0.push(ActorComponent::Persist);
        let rebuilt = rebuild(&instance, &next).unwrap();
        assert_eq!(rebuilt.id, "instance");
        assert_eq!(color(&rebuilt), "#00FF00", "the override stays");
        assert!(
            rebuilt
                .components
                .iter()
                .any(|c| matches!(c, ActorComponent::Persist))
        );
        assert_eq!(link(&rebuilt).unwrap().overrides, overrides);

        // Without the override the prefab's look comes through.
        let mut plain = instance.clone();
        link_mut(&mut plain).unwrap().overrides.clear();
        assert_eq!(color(&rebuild(&plain, &next).unwrap()), "#0000FF");

        // Setting it back to the prefab's value drops the override.
        let before = instance.clone();
        look(&mut instance, "#FF0000");
        mark(&before, &instance, &prefab, &mut overrides);
        assert!(overrides.is_empty());
    }

    #[test]
    fn place_and_name_stay_the_instances() {
        let project = Project::starter("P", Mode::TwoD);
        let prefab = project.active_scene().actors[0].clone();
        let mut instance = prefab.clone();
        instance.id = "i".into();
        instance.name = "Mine".into();
        instance.components.0.push(ActorComponent::Prefab {
            prefab: PrefabLink::default(),
        });
        if let Some(ActorComponent::Place { placement }) = instance
            .components
            .iter_mut()
            .find(|c| matches!(c, ActorComponent::Place { .. }))
        {
            placement.position = [123.0, 0.0, 0.0];
        }
        let mut overrides = Vec::new();
        mark(&prefab, &instance, &prefab, &mut overrides);
        assert!(overrides.is_empty(), "{overrides:?}");
        let rebuilt = rebuild(&instance, &prefab).unwrap();
        assert_eq!(rebuilt.name, "Mine");
        assert_eq!(rebuilt.placement().position[0], 123.0);
        assert_eq!(rebuilt.components.0.len(), instance.components.0.len());
    }
}
