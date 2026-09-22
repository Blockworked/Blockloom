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
}
