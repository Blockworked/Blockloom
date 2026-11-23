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
const FORMAT: u32 = 2;

/// What an instance knows about its prefab.
pub type PrefabLink = crate::linked::AssetLink;

#[derive(Serialize, Deserialize)]
struct PrefabFile {
    #[serde(default)]
    format: u32,
    #[serde(rename = "type", default)]
    kind: String,
    actor: Actor,
    /// Actors hanging off the root (by `Parent`, ids local to the file).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<Actor>,
}

/// Nested prefabs deeper than this are refused, which also stops a cycle.
const MAX_NESTING: usize = 16;

/// One actor of a prefab as read: its member id (empty for the root, and
/// `nested/child` for a child a nested prefab brought), the member it
/// hangs off and its state.
#[derive(Debug, Clone)]
pub struct Member {
    pub id: String,
    pub parent: Option<String>,
    pub actor: Actor,
}

pub fn is_prefab(path: &str) -> bool {
    path.to_lowercase().ends_with(&format!(".{EXTENSION}"))
}

fn read_file(dir: &Path, relative: &str) -> Result<PrefabFile, String> {
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
    Ok(file)
}

/// The prefab's root actor.
pub fn read(dir: &Path, relative: &str) -> Result<Actor, String> {
    Ok(read_members(dir, relative)?.swap_remove(0).actor)
}

/// Every actor of the prefab, root first, nested prefabs expanded.
pub fn read_members(dir: &Path, relative: &str) -> Result<Vec<Member>, String> {
    members_at(dir, relative, &mut Vec::new())
}

fn members_at(dir: &Path, relative: &str, open: &mut Vec<String>) -> Result<Vec<Member>, String> {
    if open.iter().any(|p| p == relative) {
        return Err(format!("{relative} contains itself"));
    }
    if open.len() >= MAX_NESTING {
        return Err(format!(
            "{relative}: prefabs nest more than {MAX_NESTING} deep"
        ));
    }
    open.push(relative.to_string());
    let file = read_file(dir, relative)?;
    let root_id = file.actor.id.clone();
    let rename = |id: &str| {
        if id == root_id {
            String::new()
        } else {
            id.to_string()
        }
    };
    let mut out = Vec::new();
    let mut root = file.actor;
    root.components.remove("Parent");
    out.push(Member {
        id: String::new(),
        parent: None,
        actor: root,
    });
    for child in file.children {
        let id = rename(&child.id);
        let parent = child.components.parent().map(rename);
        out.push(Member {
            id,
            parent,
            actor: child,
        });
    }
    // A member that is itself an instance brings its prefab's actors.
    let mut expanded = Vec::new();
    for member in out {
        let Some(nested) = link(&member.actor).filter(|l| l.member.is_empty()).cloned() else {
            expanded.push(member);
            continue;
        };
        let inner = members_at(dir, &nested.source.path, open)?;
        let mut head = rebuild(&member.actor, &inner[0].actor)?;
        // The nested root stands where this file put it.
        head.components.set_placement(member.actor.placement());
        let at = member.id.clone();
        expanded.push(Member {
            id: at.clone(),
            parent: member.parent,
            actor: head,
        });
        for child in inner.into_iter().skip(1) {
            let parent = child.parent.map(|parent| {
                if parent.is_empty() {
                    at.clone()
                } else {
                    format!("{at}/{parent}")
                }
            });
            expanded.push(Member {
                id: format!("{at}/{}", child.id),
                parent,
                actor: child.actor,
            });
        }
    }
    open.pop();
    Ok(expanded)
}

/// Writes `actor` as the prefab at `relative`, without its own link.
pub fn write(dir: &Path, relative: &str, actor: &Actor) -> Result<(), String> {
    write_members(dir, relative, actor, &[])
}

/// Writes a prefab of `root` and `children`. Children name their parents by
/// ids in the same file; a child keeps a link only when it nests a prefab.
pub fn write_members(
    dir: &Path,
    relative: &str,
    root: &Actor,
    children: &[Actor],
) -> Result<(), String> {
    let full = crate::assets::resolve(dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
    let mut actor = root.clone();
    actor
        .components
        .0
        .retain(|c| !matches!(c, ActorComponent::Prefab { .. }));
    actor.components.remove("Parent");
    let children: Vec<Actor> = children
        .iter()
        .map(|child| {
            let mut child = child.clone();
            child.components.0.retain(|c| match c {
                ActorComponent::Prefab { prefab } => {
                    prefab.member.is_empty() && prefab.instance.is_empty()
                }
                _ => true,
            });
            child
        })
        .collect();
    let file = PrefabFile {
        // Single-actor prefabs keep the first format, which older editors read.
        format: if children.is_empty() { 1 } else { FORMAT },
        kind: "prefab".into(),
        actor,
        children,
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    text.push('\n');
    crate::files::write_atomic(&full, text)
}

/// Members of a prefab as new scene actors: fresh ids, `Parent` mapped onto
/// them, and each linked to `source`. The root comes first.
pub fn instantiate(
    members: &[Member],
    source: &crate::resources::ResourceRef,
    mut new_id: impl FnMut() -> String,
) -> Vec<Actor> {
    let ids: BTreeMap<&str, String> = members.iter().map(|m| (m.id.as_str(), new_id())).collect();
    let root = ids[""].clone();
    members
        .iter()
        .map(|member| {
            let mut actor = member.actor.clone();
            actor
                .components
                .0
                .retain(|c| !matches!(c, ActorComponent::Prefab { .. }));
            actor.id = ids[member.id.as_str()].clone();
            actor.refresh_physics_ids();
            match member.parent.as_deref().and_then(|p| ids.get(p)) {
                Some(id) if !member.id.is_empty() => actor.components.set_parent(id),
                _ => actor.components.set_parent(""),
            }
            actor.components.0.push(ActorComponent::Prefab {
                prefab: PrefabLink {
                    source: source.clone(),
                    member: member.id.clone(),
                    instance: if member.id.is_empty() {
                        String::new()
                    } else {
                        root.clone()
                    },
                    ..Default::default()
                },
            });
            actor
        })
        .collect()
}

/// The paths `actor` sets differently from `source`, as overrides.
pub fn differences(actor: &Actor, source: &Actor) -> Vec<String> {
    let (mine, _) = fields(actor);
    let (theirs, _) = fields(source);
    let keys: std::collections::BTreeSet<&String> = mine.keys().chain(theirs.keys()).collect();
    keys.into_iter()
        .filter(|key| !own(key) && mine.get(*key) != theirs.get(*key))
        .cloned()
        .collect()
}

/// An override entry recording that an instance deleted one of its members.
pub fn removed_entry(member: &str) -> String {
    format!("removed:{member}")
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
    fn a_prefab_that_nests_itself_is_refused() {
        let dir = std::env::temp_dir().join(format!("blockloom-prefab-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let project = Project::starter("P", Mode::TwoD);
        let root = project.active_scene().actors[0].clone();
        let mut child = root.clone();
        child.id = "child".into();
        child.components.set_parent(&root.id);
        child.components.0.push(ActorComponent::Prefab {
            prefab: PrefabLink {
                source: crate::resources::ResourceRef {
                    id: String::new(),
                    path: "assets/a.blockprefab".into(),
                },
                ..Default::default()
            },
        });
        write_members(&dir, "assets/a.blockprefab", &root, &[child]).unwrap();
        let error = read_members(&dir, "assets/a.blockprefab").unwrap_err();
        assert!(error.contains("contains itself"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
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
