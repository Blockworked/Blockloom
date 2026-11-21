//! The window's change stream: rather than the whole state on every edit,
//! each message carries only the top-level fields whose JSON changed and,
//! for the project, the fields and actors an edit touched.
//!
//! A message is `{"full": state}` or `{"patch": {"set": {key: value},
//! "project": {"set": {key: value}, "actors": {"order": [ids]?, "set": {id:
//! actor}, "remove": [ids]}}}}`. The window applies them in order (see
//! `Main.qml`'s `applyState`); the shell and the attach socket still read
//! whole snapshots through `state_dto`.

use crate::state::{AppState, state_dto_with};
use blockloom_core::project::FrontendView;
use blockloom_core::wire::to_wire;
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};

/// What the window was last sent.
#[derive(Default)]
pub(crate) struct Publisher {
    /// Top-level fields other than `project`, as JSON text.
    fields: HashMap<String, String>,
    /// The project the window holds: its id, active scene and actor order.
    project: Option<SentProject>,
    /// The project's fields other than `actors`, as JSON text.
    project_fields: HashMap<String, String>,
}

struct SentProject {
    id: String,
    scene: String,
    order: Vec<String>,
}

impl Publisher {
    /// Starts over, so the next message is whole.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The next message for the window, or `None` when nothing changed.
pub(crate) fn next_message(s: &mut AppState) -> Option<String> {
    let unpublished = s.open.as_mut().map(|open| open.tracker.take_unpublished());
    let dto = state_dto_with(s, None);
    let Value::Object(mut fields) = serde_json::to_value(&dto).ok()? else {
        return None;
    };
    fields.remove("project");

    let publisher = &mut s.publisher;
    let project = s.open.as_ref().map(|open| &open.project);
    let whole = match (&publisher.project, project) {
        (None, None) => false,
        (Some(_), None) | (None, Some(_)) => true,
        (Some(sent), Some(p)) => {
            sent.id != p.id
                || sent.scene != p.active_scene
                || unpublished.as_ref().is_none_or(|u| u.everything)
        }
    };
    let first = publisher.fields.is_empty();
    let mut set = Map::new();
    for (key, value) in fields {
        let text = value.to_string();
        if first || publisher.fields.get(&key) != Some(&text) {
            publisher.fields.insert(key.clone(), text);
            set.insert(key, value);
        }
    }

    let mut project_patch = Value::Null;
    match project {
        None => {
            if publisher.project.take().is_some() {
                set.insert("project".into(), Value::Null);
            }
            publisher.project_fields.clear();
        }
        Some(p) if whole || first || publisher.project.is_none() => {
            let value = to_wire(&FrontendView {
                project: p,
                actors: true,
            })
            .ok()?;
            publisher.project_fields = project_fields(&value);
            publisher.project = Some(SentProject {
                id: p.id.clone(),
                scene: p.active_scene.clone(),
                order: p.actors.iter().map(|a| a.id.clone()).collect(),
            });
            set.insert("project".into(), value);
        }
        Some(p) => {
            let unpublished = unpublished.unwrap_or_default();
            let mut patch = Map::new();
            if unpublished.project {
                let value = to_wire(&FrontendView {
                    project: p,
                    actors: false,
                })
                .ok()?;
                let mut changed = Map::new();
                for (key, text) in project_fields(&value) {
                    if publisher.project_fields.get(&key) != Some(&text) {
                        changed.insert(key.clone(), value[&key].clone());
                        publisher.project_fields.insert(key, text);
                    }
                }
                if !changed.is_empty() {
                    patch.insert("set".into(), Value::Object(changed));
                }
            }
            if let Some(touched) = unpublished.actors.get(&p.active_scene) {
                let sent = publisher.project.as_mut().expect("a sent project");
                let order: Vec<String> = p.actors.iter().map(|a| a.id.clone()).collect();
                let all: BTreeSet<String>;
                let touched = match touched {
                    Some(ids) => ids,
                    None => {
                        all = order.iter().cloned().collect();
                        &all
                    }
                };
                let mut actors = Map::new();
                let mut set_actors = Map::new();
                let mut remove = Vec::new();
                for id in touched {
                    match p.actor(id) {
                        Some(actor) => {
                            set_actors.insert(id.clone(), to_wire(actor).ok()?);
                        }
                        None => remove.push(Value::String(id.clone())),
                    }
                }
                if order != sent.order {
                    actors.insert("order".into(), json!(order));
                    sent.order = order;
                }
                if !set_actors.is_empty() {
                    actors.insert("set".into(), Value::Object(set_actors));
                }
                if !remove.is_empty() {
                    actors.insert("remove".into(), Value::Array(remove));
                }
                if !actors.is_empty() {
                    patch.insert("actors".into(), Value::Object(actors));
                }
            }
            if !patch.is_empty() {
                project_patch = Value::Object(patch);
            }
        }
    }

    if first {
        // The window holds nothing to patch yet.
        set.entry("project").or_insert(Value::Null);
        return Some(json!({ "full": set }).to_string());
    }
    if set.is_empty() && project_patch.is_null() {
        return None;
    }
    let mut patch = Map::new();
    if !set.is_empty() {
        patch.insert("set".into(), Value::Object(set));
    }
    if !project_patch.is_null() {
        patch.insert("project".into(), project_patch);
    }
    Some(json!({ "patch": patch }).to_string())
}

fn project_fields(value: &Value) -> HashMap<String, String> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter(|(key, _)| key.as_str() != "actors")
                .map(|(key, value)| (key.clone(), value.to_string()))
                .collect()
        })
        .unwrap_or_default()
}
