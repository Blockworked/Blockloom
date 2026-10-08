//! Per-player variable values that survive one run of a project.
//!
//! A save belongs to the project's stable id, not its name or build folder.
//! Only variables a `save variable` block names are written. Loading ignores
//! slots the current project no longer declares, so old save data is harmless.

use crate::project::Project;
use crate::value::Evaluated;
use crate::vm::{VariableSnapshot, Variables};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveData {
    #[serde(default)]
    pub globals: HashMap<String, Evaluated>,
    #[serde(default)]
    pub actors: HashMap<String, HashMap<String, Evaluated>>,
}

impl SaveData {
    /// Overlays saved values onto the freshly initialized run. Values for
    /// variables that were renamed or removed are left in the file but not
    /// introduced into the running project.
    pub fn apply(&self, project: &Project, variables: &Variables) {
        for variable in &project.globals {
            if let Some(value) = self.globals.get(&variable.name) {
                variables.write("", &variable.name, value.clone());
            }
        }
        for actor in &project.actors {
            for variable in &actor.graph.variables {
                if let Some(value) = self
                    .actors
                    .get(&actor.id)
                    .and_then(|values| values.get(&variable.name))
                {
                    variables.write(&actor.id, &variable.name, value.clone());
                }
            }
        }
    }

    /// Captures the named slot from a run. Actor variables shadow globals,
    /// exactly as a normal variable read does.
    pub fn capture(
        &mut self,
        project: &Project,
        variables: &VariableSnapshot,
        declared_actor: &str,
        live_actor: &str,
        name: &str,
    ) -> bool {
        if project
            .actor(declared_actor)
            .is_some_and(|owner| owner.graph.variables.iter().any(|v| v.name == name))
        {
            let Some(value) = variables
                .actors
                .get(live_actor)
                .and_then(|values| values.get(name))
            else {
                return false;
            };
            self.actors
                .entry(declared_actor.to_string())
                .or_default()
                .insert(name.to_string(), value.clone());
            return true;
        }
        let Some(value) = variables.globals.get(name) else {
            return false;
        };
        self.globals.insert(name.to_string(), value.clone());
        true
    }

    pub fn clear(&mut self, project: &Project, actor: &str, name: &str) -> bool {
        if project
            .actor(actor)
            .is_some_and(|owner| owner.graph.variables.iter().any(|v| v.name == name))
        {
            return self
                .actors
                .get_mut(actor)
                .is_some_and(|values| values.remove(name).is_some());
        }
        self.globals.remove(name).is_some()
    }
}

pub fn path(project_id: &str) -> PathBuf {
    crate::project::data_dir()
        .join("saves")
        .join(format!("{project_id}.json"))
}

/// The save slot a fresh run opens: the legacy single save file above, so
/// an old project re-saves byte for byte until a block names another slot.
pub const DEFAULT_SLOT: &str = "default";

/// A slot name as the blocks spell it into the file key it names. Empty
/// reads as the default slot; anything else is lowercased with runs of
/// non-plain characters folded to one hyphen, so "Slot 1" and "slot:1"
/// are the same file and nothing can escape the saves dir.
pub fn normalize_slot(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return DEFAULT_SLOT.to_string();
    }
    let mut key = String::with_capacity(trimmed.len());
    let mut dash = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            key.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            key.push('-');
            dash = true;
        }
    }
    let key = key.trim_matches('-').to_string();
    let mut key = key;
    key.truncate(32);
    let key = key.trim_matches('-').to_string();
    if key.is_empty() {
        DEFAULT_SLOT.to_string()
    } else {
        key
    }
}

/// Where one named slot of a project lives. The default slot is the legacy
/// path above; the rest sit beside it as `{id}__{slot}.json`.
pub fn slot_path(project_id: &str, slot: &str) -> PathBuf {
    if normalize_slot(slot) == DEFAULT_SLOT {
        return path(project_id);
    }
    crate::project::data_dir()
        .join("saves")
        .join(format!("{project_id}__{}.json", normalize_slot(slot)))
}

/// Every slot with a file on disk, default first then alphabetical. A
/// missing saves dir reads as none rather than failing the run.
pub fn list_slots(project_id: &str) -> Vec<String> {
    let dir = crate::project::data_dir().join("saves");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut slots = Vec::new();
    let plain = format!("{project_id}.json");
    let prefix = format!("{project_id}__");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == plain {
            slots.push(DEFAULT_SLOT.to_string());
        } else if let Some(rest) = name.strip_prefix(&prefix)
            && let Some(slot) = rest.strip_suffix(".json")
            && !slot.is_empty()
            && slot == normalize_slot(slot)
        {
            slots.push(slot.to_string());
        }
    }
    slots.sort();
    if let Some(at) = slots.iter().position(|s| s == DEFAULT_SLOT) {
        let default = slots.remove(at);
        slots.insert(0, default);
    }
    slots
}

/// Deletes one named slot's file. Answers whether a file was there.
pub fn delete_slot(project_id: &str, slot: &str) -> Result<bool, String> {
    let file = slot_path(project_id, slot);
    if !file.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(true)
}

pub fn read(path: &Path) -> Result<SaveData, String> {
    if !path.is_file() {
        return Ok(SaveData::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write(path: &Path, data: &SaveData) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Mode;

    #[test]
    fn only_declared_slots_are_restored() {
        let mut project = Project::starter("Save", Mode::TwoD);
        project.create_global("volume").unwrap();
        let actor = project.actors[0].id.clone();
        project.actors[0].graph.create_variable("name").unwrap();
        let variables = Variables::default();
        variables.load(&project);

        let data = SaveData {
            globals: HashMap::from([
                ("volume".to_string(), Evaluated::Number(42.0)),
                ("removed".to_string(), Evaluated::Number(9.0)),
            ]),
            actors: HashMap::from([(
                actor.clone(),
                HashMap::from([("name".to_string(), Evaluated::Text("Ada".to_string()))]),
            )]),
        };
        data.apply(&project, &variables);

        assert_eq!(variables.read(&actor, "volume"), Evaluated::Number(42.0));
        assert_eq!(
            variables.read(&actor, "name"),
            Evaluated::Text("Ada".to_string())
        );
        assert_eq!(variables.read(&actor, "removed"), Evaluated::Number(0.0));
    }

    #[test]
    fn actor_slots_shadow_globals_when_captured() {
        let mut project = Project::starter("Save", Mode::TwoD);
        project.create_global("score").unwrap();
        let actor = project.actors[0].id.clone();
        project.actors[0].graph.create_variable("score").unwrap();
        let variables = Variables::default();
        variables.load(&project);
        variables.write(&actor, "score", Evaluated::Number(7.0));

        let mut data = SaveData::default();
        assert!(data.capture(&project, &variables.snapshot(), &actor, &actor, "score"));
        assert_eq!(data.actors[&actor]["score"], Evaluated::Number(7.0));
        assert!(!data.globals.contains_key("score"));
    }

    #[test]
    fn clone_values_are_saved_for_the_template_actor() {
        let mut project = Project::starter("Save", Mode::TwoD);
        let actor = project.actors[0].id.clone();
        project.actors[0].graph.create_variable("score").unwrap();
        let variables = Variables::default();
        variables.load(&project);
        variables.copy_actor(&actor, "clone-1");
        variables.write("clone-1", "score", Evaluated::Number(12.0));

        let mut data = SaveData::default();
        assert!(data.capture(&project, &variables.snapshot(), &actor, "clone-1", "score"));
        assert_eq!(data.actors[&actor]["score"], Evaluated::Number(12.0));
        assert!(!data.actors.contains_key("clone-1"));
    }

    #[test]
    fn slot_names_fold_to_file_keys() {
        assert_eq!(normalize_slot(""), "default");
        assert_eq!(normalize_slot("  "), "default");
        assert_eq!(normalize_slot("Slot 1"), "slot-1");
        assert_eq!(normalize_slot("slot:1"), "slot-1");
        assert_eq!(normalize_slot("../evil"), "evil");
        assert_eq!(normalize_slot("default"), "default");
    }

    #[test]
    fn default_slot_keeps_the_legacy_path() {
        assert_eq!(slot_path("game", "default"), path("game"));
        assert_eq!(slot_path("game", ""), path("game"));
        assert!(
            slot_path("game", "Slot 1")
                .to_string_lossy()
                .ends_with("game__slot-1.json")
        );
    }
}
