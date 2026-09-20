//! The saved document: a world, the actors in it, and one block canvas per
//! actor.
//!
//! Each actor owns a `BlockGraph`, so switching actors in the editor swaps the
//! canvas the way switching macros does in Blockwork. Variables come in two
//! scopes: an actor's own (on its graph, private to it) and the project's
//! ([`Project::globals`], shared by every actor - a score, a level number).

use crate::blocks::{ActorGraph, InstructionKind, VariableDef};
use crate::scene::{Physics, Placement, Visual, World};
use crate::value::Evaluated;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

/// File extension of a saved project.
pub const PROJECT_EXTENSION: &str = "blockloom";

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn visible_by_default() -> bool {
    true
}

fn default_visual() -> Visual {
    Visual::Rect {
        color: "#4C97FF".to_string(),
        size: [80.0, 80.0],
    }
}

/// One thing in the world, with its own canvas. Its `BlockGraph` is
/// flattened into the same JSON object, so an actor reads as one record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub id: String,
    pub name: String,
    #[serde(default = "default_visual")]
    pub visual: Visual,
    #[serde(default)]
    pub placement: Placement,
    #[serde(default)]
    pub physics: Physics,
    #[serde(default = "visible_by_default")]
    pub visible: bool,
    #[serde(flatten)]
    pub graph: ActorGraph,
}

impl Deref for Actor {
    type Target = ActorGraph;

    fn deref(&self) -> &ActorGraph {
        &self.graph
    }
}

impl DerefMut for Actor {
    fn deref_mut(&mut self) -> &mut ActorGraph {
        &mut self.graph
    }
}

impl Actor {
    pub fn new(name: impl Into<String>, visual: Visual) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            visual,
            placement: Placement::default(),
            physics: Physics::default(),
            visible: true,
            graph: ActorGraph::new(),
        }
    }
}

/// A whole project: what the editor edits, what the runtime is handed, and
/// what a `.blockloom` file holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub world: World,
    #[serde(default)]
    pub actors: Vec<Actor>,
    /// Variables every actor can read and write - see the module docs.
    #[serde(default)]
    pub globals: Vec<VariableDef>,
}

impl Project {
    /// A fresh project: one actor to move and one static floor to land on,
    /// so gravity means something the moment Play is pressed.
    pub fn starter(name: impl Into<String>, mode: crate::scene::Mode) -> Self {
        let mut world = World {
            mode,
            gravity: World::default_gravity(mode),
            ..World::default()
        };
        let (player, ground) = if mode.is_3d() {
            world.camera = crate::scene::Camera::default();
            (
                Visual::Sphere {
                    color: "#4C97FF".to_string(),
                    radius: 0.5,
                },
                Visual::Plane {
                    color: "#3E4A5B".to_string(),
                    size: [20.0, 20.0],
                },
            )
        } else {
            (
                Visual::Rect {
                    color: "#4C97FF".to_string(),
                    size: [60.0, 60.0],
                },
                Visual::Rect {
                    color: "#3E4A5B".to_string(),
                    size: [800.0, 40.0],
                },
            )
        };

        let mut player = Actor::new("Player", player);
        player.placement.position = if mode.is_3d() {
            [0.0, 3.0, 0.0]
        } else {
            [0.0, 160.0, 0.0]
        };
        player.physics.body = crate::scene::BodyKind::Dynamic;
        player.physics.lock_rotation = true;

        let mut ground = Actor::new("Ground", ground);
        ground.placement.position = if mode.is_3d() {
            [0.0, 0.0, 0.0]
        } else {
            [0.0, -220.0, 0.0]
        };
        ground.physics.body = crate::scene::BodyKind::Static;

        Self {
            id: new_id(),
            name: name.into(),
            world,
            actors: vec![player, ground],
            globals: Vec::new(),
        }
    }

    pub fn actor(&self, id: &str) -> Option<&Actor> {
        self.actors.iter().find(|actor| actor.id == id)
    }

    pub fn actor_mut(&mut self, id: &str) -> Option<&mut Actor> {
        self.actors.iter_mut().find(|actor| actor.id == id)
    }

    /// An unused actor name based on `base` - `"Player"`, then `"Player 2"`.
    pub fn unique_actor_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Actor" } else { base };
        if !self.actors.iter().any(|a| a.name == base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|candidate| !self.actors.iter().any(|a| &a.name == candidate))
            .expect("an unused name always exists")
    }

    /// Adds `actor`, giving it a name that doesn't collide.
    pub fn add_actor(&mut self, mut actor: Actor) -> String {
        actor.name = self.unique_actor_name(&actor.name);
        let id = actor.id.clone();
        self.actors.push(actor);
        id
    }

    /// Removes an actor and every reference other actors make to it by name.
    /// Returns whether it was there.
    pub fn remove_actor(&mut self, id: &str) -> bool {
        let Some(index) = self.actors.iter().position(|actor| actor.id == id) else {
            return false;
        };
        self.actors.remove(index);
        if self.world.camera.follow.as_deref() == Some(id) {
            self.world.camera.follow = None;
        }
        true
    }

    /// Renames an actor, keeping names unique. Blocks refer to actors by
    /// name, so every `point towards`/`when I touch` mention follows along.
    pub fn rename_actor(&mut self, id: &str, name: &str) -> Result<String, String> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err("An actor needs a name".to_string());
        }
        let old = self
            .actor(id)
            .ok_or_else(|| "Actor not found".to_string())?
            .name
            .clone();
        if trimmed == old {
            return Ok(trimmed);
        }
        if self.actors.iter().any(|a| a.id != id && a.name == trimmed) {
            return Err(format!("An actor named \"{trimmed}\" already exists"));
        }
        for actor in &mut self.actors {
            actor
                .graph
                .walk_instructions_mut(&mut |ins| match &mut ins.kind {
                    InstructionKind::PointTowards { target } if *target == old => {
                        *target = trimmed.clone()
                    }
                    InstructionKind::WhenCollision { with } if *with == old => {
                        *with = trimmed.clone()
                    }
                    _ => {}
                });
        }
        self.actor_mut(id)
            .expect("looked up above")
            .name
            .clone_from(&trimmed);
        Ok(trimmed)
    }

    /// Declares a project-wide variable starting at `0`.
    pub fn create_global(&mut self, name: &str) -> Result<String, String> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err("Variable name can't be empty".to_string());
        }
        if self.globals.iter().any(|v| v.name == trimmed) {
            return Err(format!("A variable named \"{trimmed}\" already exists"));
        }
        self.globals.push(VariableDef {
            name: trimmed.clone(),
            value: Evaluated::Number(0.0),
        });
        Ok(trimmed)
    }

    /// Renames a project-wide variable and every read of it, in every actor.
    pub fn rename_global(&mut self, old: &str, new: &str) -> Result<String, String> {
        let trimmed = new.trim().to_string();
        if trimmed.is_empty() {
            return Err("Variable name can't be empty".to_string());
        }
        if trimmed != old && self.globals.iter().any(|v| v.name == trimmed) {
            return Err(format!("A variable named \"{trimmed}\" already exists"));
        }
        let Some(variable) = self.globals.iter_mut().find(|v| v.name == old) else {
            return Err("Variable not found".to_string());
        };
        if trimmed == old {
            return Ok(trimmed);
        }
        variable.name = trimmed.clone();
        for actor in &mut self.actors {
            // An actor with its own variable of that name shadows the global,
            // so its reads are about that one and must stay put.
            if actor.graph.variables.iter().any(|v| v.name == old) {
                continue;
            }
            for strand in &mut actor.graph.strands {
                for instruction in &mut strand.instructions {
                    instruction.rename_var(old, &trimmed);
                }
            }
            for floating in &mut actor.graph.floating_values {
                floating.value.rename_var(old, &trimmed);
            }
        }
        Ok(trimmed)
    }

    /// Drops a project-wide variable. Reads of it are left alone and default
    /// to `0`, the same as an actor's own removed variable.
    pub fn remove_global(&mut self, name: &str) {
        self.globals.retain(|v| v.name != name);
    }

    /// The starting variable environment for `actor`: the project's globals,
    /// then the actor's own, which shadow them on a name collision.
    pub fn env_for(&self, actor_id: &str) -> HashMap<String, Evaluated> {
        let mut env: HashMap<String, Evaluated> = self
            .globals
            .iter()
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect();
        if let Some(actor) = self.actor(actor_id) {
            env.extend(actor.graph.variable_values());
        }
        env
    }

    /// True if `name` is a global rather than one of `actor_id`'s own - which
    /// list a `set`/`change` block writes back to.
    pub fn is_global(&self, actor_id: &str, name: &str) -> bool {
        let actor_owns = self
            .actor(actor_id)
            .is_some_and(|actor| actor.graph.variables.iter().any(|v| v.name == name));
        !actor_owns && self.globals.iter().any(|v| v.name == name)
    }

    /// Repairs and canonicalizes a just-loaded document, once.
    pub fn normalize(&mut self) {
        for actor in &mut self.actors {
            actor.graph.migrate_bool_slots();
            actor.graph.normalize_block_colors();
            actor.graph.prune_orphaned_comments();
        }
    }
}

// ─── On-disk storage ───────────────────────────────────────────────────────

/// Where projects live: `<data dir>/blockloom/projects`.
pub fn projects_dir() -> PathBuf {
    let base = std::env::var_os("BLOCKLOOM_DATA_DIR")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("blockloom").join("projects")
}

fn project_path(id: &str) -> PathBuf {
    projects_dir().join(format!("{id}.{PROJECT_EXTENSION}"))
}

/// Reads one project file.
pub fn read_project(path: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut project: Project =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    project.normalize();
    Ok(project)
}

/// Writes a project to its own file in [`projects_dir`].
pub fn save_project(project: &Project) -> Result<(), String> {
    let path = project_path(&project.id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(project).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes a project to an arbitrary path, for "Export".
pub fn export_project(project: &Project, path: &Path) -> Result<(), String> {
    let json = serde_json::to_string_pretty(project).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn delete_project(id: &str) -> Result<(), String> {
    let path = project_path(id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Every saved project, name-sorted. Unreadable files are skipped with a
/// warning rather than failing the whole load.
pub fn load_projects() -> Vec<Project> {
    let dir = projects_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut projects: Vec<Project> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == PROJECT_EXTENSION))
        .filter_map(|path| match read_project(&path) {
            Ok(project) => Some(project),
            Err(e) => {
                tracing::warn!("Skipping unreadable project: {e}");
                None
            }
        })
        .collect();
    projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    projects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Instruction;
    use crate::scene::Mode;

    #[test]
    fn a_starter_project_has_something_to_drop_and_something_to_land_on() {
        let project = Project::starter("Untitled", Mode::TwoD);
        assert_eq!(project.actors.len(), 2);
        assert_eq!(
            project.actors[0].physics.body,
            crate::scene::BodyKind::Dynamic
        );
        assert_eq!(
            project.actors[1].physics.body,
            crate::scene::BodyKind::Static
        );
    }

    #[test]
    fn actor_names_stay_unique() {
        let mut project = Project::starter("p", Mode::TwoD);
        let id = project.add_actor(Actor::new("Player", default_visual()));
        assert_eq!(project.actor(&id).unwrap().name, "Player 2");
        assert!(project.rename_actor(&id, "Ground").is_err());
        assert_eq!(project.rename_actor(&id, "Enemy"), Ok("Enemy".to_string()));
    }

    #[test]
    fn renaming_an_actor_follows_the_blocks_that_name_it() {
        let mut project = Project::starter("p", Mode::TwoD);
        let ground = project.actors[1].id.clone();
        project.actors[0]
            .graph
            .strands
            .push(crate::blocks::Strand::with_instructions(
                0,
                0,
                vec![Instruction::new(InstructionKind::PointTowards {
                    target: "Ground".to_string(),
                })],
            ));
        project.rename_actor(&ground, "Floor").unwrap();
        let InstructionKind::PointTowards { target } =
            &project.actors[0].graph.strands[0].instructions[0].kind
        else {
            panic!("expected a PointTowards");
        };
        assert_eq!(target, "Floor");
    }

    #[test]
    fn renaming_a_global_follows_reads_except_where_an_actor_shadows_it() {
        let mut project = Project::starter("p", Mode::TwoD);
        project.create_global("score").unwrap();
        let shadowing = project.actors[1].id.clone();
        project
            .actor_mut(&shadowing)
            .unwrap()
            .graph
            .create_variable("score")
            .unwrap();
        for index in 0..2 {
            let read = crate::value::Value::Var {
                name: "score".to_string(),
            };
            project.actors[index]
                .graph
                .strands
                .push(crate::blocks::Strand::with_instructions(
                    0,
                    0,
                    vec![Instruction::new(InstructionKind::Say { text: read })],
                ));
        }
        project.rename_global("score", "points").unwrap();

        let read_of = |actor: &Actor| match &actor.graph.strands[0].instructions[0].kind {
            InstructionKind::Say {
                text: crate::value::Value::Var { name },
            } => name.clone(),
            other => panic!("expected a variable read, got {other:?}"),
        };
        assert_eq!(read_of(&project.actors[0]), "points");
        assert_eq!(read_of(&project.actors[1]), "score");
    }

    #[test]
    fn an_actors_own_variable_shadows_a_global_of_the_same_name() {
        let mut project = Project::starter("p", Mode::TwoD);
        project.create_global("score").unwrap();
        let id = project.actors[0].id.clone();
        project
            .actor_mut(&id)
            .unwrap()
            .graph
            .create_variable("score")
            .unwrap();
        assert!(!project.is_global(&id, "score"));
        project.actor_mut(&id).unwrap().graph.variables[0].value = Evaluated::Number(7.0);
        assert_eq!(
            project.env_for(&id).get("score"),
            Some(&Evaluated::Number(7.0))
        );
        assert!(project.is_global(&project.actors[1].id, "score"));
    }
}
