//! The saved document: a world, the actors in it, and one block canvas per
//! actor.
//!
//! Each actor owns a `BlockGraph`, so switching actors in the editor swaps the
//! canvas the way switching macros does in Blockwork. Variables come in two
//! scopes: an actor's own (on its graph, private to it) and the project's
//! ([`Project::globals`], shared by every actor - a score, a level number).
//!
//! On disk a project is a folder, not a file: `<name>/project.blockloom` next
//! to an `assets/` the project's own files live in. The app owns the folder
//! name and keeps it matched to the project's; where the folder sits is the
//! user's choice, so [`crate::library`] remembers the ones it has opened.

use crate::blocks::{ActorGraph, InstructionKind, ListDef, VariableDef};
use crate::components::{ActorComponent, CameraAttach, CameraView, Components};
use crate::scene::{Mode, Physics, Placement, Visual, World};
use crate::value::Evaluated;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

/// File extension of a saved project.
pub const PROJECT_EXTENSION: &str = "blockloom";

/// The document inside a project folder. A project is a folder so it has
/// somewhere to keep its assets; this is the part that holds the blocks.
pub const PROJECT_FILE: &str = "project.blockloom";

/// Where a project folder keeps its assets.
pub const ASSETS_DIR: &str = "assets";

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn default_visual() -> Visual {
    Visual::Rect {
        color: "#4C97FF".to_string(),
        size: [80.0, 80.0],
    }
}

/// What an actor created mid-run looks like until something says otherwise.
fn blank_visual(mode: Mode) -> Visual {
    if mode.is_3d() {
        Visual::Cuboid {
            color: "#4C97FF".to_string(),
            size: [1.0, 1.0, 1.0],
        }
    } else {
        Visual::Rect {
            color: "#4C97FF".to_string(),
            size: [60.0, 60.0],
        }
    }
}

/// One thing in the world, with its own canvas. What the actor *is* lives in
/// [`Actor::components`]; its `BlockGraph` is flattened into the same JSON
/// object, so an actor still reads as one record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "ActorRepr")]
pub struct Actor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub components: Components,
    #[serde(flatten)]
    pub graph: ActorGraph,
}

/// What an actor deserializes through, so pre-component documents - which
/// kept the same four properties as flat fields - still load. Each one that
/// is present and has no component yet becomes that component.
#[derive(Deserialize)]
struct ActorRepr {
    id: String,
    name: String,
    #[serde(default)]
    components: Components,
    #[serde(default)]
    visual: Option<Visual>,
    #[serde(default)]
    placement: Option<Placement>,
    #[serde(default)]
    physics: Option<Physics>,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(flatten)]
    graph: ActorGraph,
}

impl From<ActorRepr> for Actor {
    fn from(repr: ActorRepr) -> Self {
        let ActorRepr {
            id,
            name,
            mut components,
            visual,
            placement,
            physics,
            visible,
            graph,
        } = repr;
        // An actor written before components had all four, so a document with
        // none of them at all is a component-era one that dropped them.
        let legacy =
            visual.is_some() || placement.is_some() || physics.is_some() || visible.is_some();
        if legacy {
            if !components.contains("Place") {
                components.0.push(ActorComponent::Place {
                    placement: placement.unwrap_or_default(),
                });
            }
            if !components.contains("Look") {
                components.0.push(ActorComponent::Look {
                    visual: visual.unwrap_or_else(default_visual),
                });
            }
            if !components.contains("Render") {
                components.0.push(ActorComponent::Render {
                    visible: visible.unwrap_or(true),
                });
            }
            if !components.contains("Body")
                && let Some(physics) = physics
            {
                components.0.push(ActorComponent::Body { physics });
            }
        }
        Self {
            id,
            name,
            components,
            graph,
        }
    }
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
            components: Components::new(visual),
            graph: ActorGraph::new(),
        }
    }

    /// A brand-new actor made mid-run by a `create actor` block or a script:
    /// somewhere to stand, something plain to see, and no blocks at all. What
    /// it does next is whatever another actor's blocks do to it.
    pub fn blank(name: impl Into<String>, mode: Mode) -> Self {
        Self::new(name, blank_visual(mode))
    }

    /// What the actor looks like, or `None` when it has no `Look` component.
    pub fn visual(&self) -> Option<&Visual> {
        self.components.visual()
    }

    /// The actor this one hangs off, by id.
    pub fn parent(&self) -> Option<&str> {
        self.components.parent()
    }

    /// Where this actor stands in its parent's frame, if it was authored
    /// that way rather than in world coordinates.
    pub fn parent_offset(&self) -> Option<[f32; 3]> {
        self.components.parent_offset()
    }

    pub fn placement(&self) -> Placement {
        self.components.placement()
    }

    pub fn physics(&self) -> Physics {
        self.components.physics()
    }

    pub fn visible(&self) -> bool {
        self.components.visible()
    }

    pub fn camera(&self) -> Option<&CameraAttach> {
        self.components.camera()
    }
}

/// A whole project: what the editor edits, what the runtime is handed, and
/// what a `.blockloom` file holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    /// An image asset used to brand packaged builds. Empty uses Blockloom's
    /// bundled icon.
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub world: World,
    #[serde(default)]
    pub actors: Vec<Actor>,
    /// Variables every actor can read and write - see the module docs.
    #[serde(default)]
    pub globals: Vec<VariableDef>,
    /// Lists every actor can read and change - the shared half of the list
    /// model, parallel to [`Project::globals`]. An actor's own list of the
    /// same name shadows this one for that actor.
    #[serde(default)]
    pub global_lists: Vec<ListDef>,
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
        player.components.placement_mut().position = if mode.is_3d() {
            [0.0, 3.0, 0.0]
        } else {
            [0.0, 160.0, 0.0]
        };
        player.components.set_physics(Physics {
            body: crate::scene::BodyKind::Dynamic,
            lock_rotation: true,
            ..Physics::default()
        });

        let mut ground = Actor::new("Ground", ground);
        ground.components.placement_mut().position = if mode.is_3d() {
            [0.0, 0.0, 0.0]
        } else {
            [0.0, -220.0, 0.0]
        };
        ground.components.set_physics(Physics {
            body: crate::scene::BodyKind::Static,
            ..Physics::default()
        });

        Self {
            id: new_id(),
            name: name.into(),
            icon: String::new(),
            world,
            actors: vec![player, ground],
            globals: Vec::new(),
            global_lists: Vec::new(),
        }
    }

    /// Converts the scene to the target dimension. Actors keep their color and
    /// approximate size, while positions move between pixels and metres.
    pub fn switch_mode(&mut self, mode: Mode) {
        let previous = self.world.mode;
        if previous == mode {
            return;
        }

        if self.world.gravity == World::default_gravity(previous) {
            self.world.gravity = World::default_gravity(mode);
        }
        let scale = if mode.is_3d() { 0.01 } else { 100.0 };
        for actor in &mut self.actors {
            let placement = actor.components.placement_mut();
            placement.position[0] *= scale;
            placement.position[1] *= scale;
            placement.position[2] = if mode.is_3d() {
                placement.position[2] * scale
            } else {
                0.0
            };
            if let Some(visual) = actor.components.visual() {
                let converted = visual_for_mode(visual, mode);
                actor.components.set_visual(converted);
            }
        }
        self.world.mode = mode;
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
        // Its children are left where they are rather than going with it -
        // a parent is an attachment, not an owner.
        for actor in &mut self.actors {
            if actor.components.parent() == Some(id) {
                actor.components.remove("Parent");
            }
        }
        true
    }

    /// Moves an actor within the list and optionally under another one, in
    /// one step: `parent` is the id it hangs off afterwards (empty for the
    /// top level) and `before` the level-mate it lands in front of (empty
    /// for the end of the level). Setting a parent keeps the authored offset,
    /// so the actor doesn't jump the moment its new parent places it.
    /// Returns whether anything changed - dropping an actor where it already
    /// is validates but leaves the document alone.
    pub fn move_actor(&mut self, id: &str, parent: &str, before: &str) -> Result<bool, String> {
        let Some(from) = self.actors.iter().position(|actor| actor.id == id) else {
            return Err("Actor not found".to_string());
        };
        if before == id {
            return Err("An actor can't move before itself".to_string());
        }
        if !parent.is_empty() {
            if parent == id {
                return Err("An actor can't hang off itself".to_string());
            }
            let Some(other) = self.actor(parent) else {
                return Err("No such actor to hang off".to_string());
            };
            let parents: HashMap<String, String> = self
                .actors
                .iter()
                .filter_map(|actor| Some((actor.id.clone(), actor.parent()?.to_string())))
                .collect();
            if reaches(&parents, parent, id) {
                return Err(format!(
                    "\"{}\" already hangs off this actor, so it can't be its parent",
                    other.name
                ));
            }
        }
        if !before.is_empty() {
            let Some(other) = self.actor(before) else {
                return Err("No such actor to move before".to_string());
            };
            if other.parent().unwrap_or_default() != parent {
                return Err(format!(
                    "\"{}\" isn't in that level of the list",
                    other.name
                ));
            }
        }
        // Dropping an actor where it already stands changes nothing: the
        // level-mate after it is already `before` (empty means last).
        let current = self
            .actor(id)
            .and_then(|actor| actor.parent())
            .unwrap_or_default();
        if current == parent {
            let level: Vec<&str> = self
                .actors
                .iter()
                .filter(|actor| actor.parent().unwrap_or_default() == parent)
                .map(|actor| actor.id.as_str())
                .collect();
            let next = level
                .iter()
                .position(|sibling| *sibling == id)
                .and_then(|i| level.get(i + 1).copied())
                .unwrap_or_default();
            if next == before {
                return Ok(false);
            }
        }
        self.actors[from].components.set_parent(parent);
        let actor = self.actors.remove(from);
        // Inserting right before `before` lands the actor before it within
        // the level whatever other levels interleave in the document; the
        // end of the document is the end of every level.
        let to = if before.is_empty() {
            self.actors.len()
        } else {
            self.actors
                .iter()
                .position(|actor| actor.id == before)
                .unwrap_or(self.actors.len())
        };
        self.actors.insert(to.min(self.actors.len()), actor);
        Ok(true)
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

    /// Declares a project-wide list starting empty.
    pub fn create_global_list(&mut self, name: &str) -> Result<String, String> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err("List name can't be empty".to_string());
        }
        if self.global_lists.iter().any(|list| list.name == trimmed) {
            return Err(format!("A list named \"{trimmed}\" already exists"));
        }
        self.global_lists.push(ListDef {
            name: trimmed.clone(),
            items: Vec::new(),
            editor_visible: false,
            editor_x: 0,
            editor_y: 0,
        });
        Ok(trimmed)
    }

    /// Renames a project-wide list and every read of it, in every actor that
    /// doesn't shadow it with its own list of that name.
    pub fn rename_global_list(&mut self, old: &str, new: &str) -> Result<String, String> {
        let trimmed = new.trim().to_string();
        if trimmed.is_empty() {
            return Err("List name can't be empty".to_string());
        }
        if trimmed != old && self.global_lists.iter().any(|list| list.name == trimmed) {
            return Err(format!("A list named \"{trimmed}\" already exists"));
        }
        let Some(list) = self.global_lists.iter_mut().find(|list| list.name == old) else {
            return Err("List not found".to_string());
        };
        if trimmed == old {
            return Ok(trimmed);
        }
        list.name = trimmed.clone();
        for actor in &mut self.actors {
            // An actor with its own list of that name reads its own, so its
            // references must stay put.
            if actor.graph.lists.iter().any(|list| list.name == old) {
                continue;
            }
            for strand in &mut actor.graph.strands {
                for instruction in &mut strand.instructions {
                    instruction.rename_list(old, &trimmed);
                }
            }
            for floating in &mut actor.graph.floating_values {
                blockstitch_core::graph::rename_list_in_value(&mut floating.value, old, &trimmed);
            }
        }
        Ok(trimmed)
    }

    /// Drops a project-wide list. Reads of it are left alone and default to
    /// empty, the same as an actor's own removed list.
    pub fn remove_global_list(&mut self, name: &str) {
        self.global_lists.retain(|list| list.name != name);
    }

    /// True if `name` is a shared list rather than one of `actor_id`'s own.
    pub fn is_global_list(&self, actor_id: &str, name: &str) -> bool {
        let actor_owns = self
            .actor(actor_id)
            .is_some_and(|actor| actor.graph.lists.iter().any(|list| list.name == name));
        !actor_owns && self.global_lists.iter().any(|list| list.name == name)
    }

    /// Repairs and canonicalizes a just-loaded document, once.
    pub fn normalize(&mut self) {
        self.migrate_camera_follow();
        self.prune_parents();
        for actor in &mut self.actors {
            actor.graph.migrate_bool_slots();
            actor.graph.normalize_block_colors();
            actor.graph.prune_orphaned_comments();
        }
    }

    /// Pre-component projects named the followed actor on the world camera.
    /// That is a camera component on the actor now, so move it there once.
    fn migrate_camera_follow(&mut self) {
        let Some(follow) = self.world.camera.legacy_follow.take() else {
            return;
        };
        if let Some(actor) = self.actor_mut(&follow) {
            actor.components.insert(ActorComponent::Camera {
                camera: CameraAttach {
                    view: CameraView::Follow,
                    ..CameraAttach::default()
                },
            });
        }
    }

    /// Drops a `Parent` naming an actor that isn't here any more, or one that
    /// would put an actor in a loop - a hand-edited document could say either,
    /// and both would leave `apply_parenting` with nowhere to start.
    fn prune_parents(&mut self) {
        let parents: HashMap<String, String> = self
            .actors
            .iter()
            .filter_map(|actor| Some((actor.id.clone(), actor.parent()?.to_string())))
            .collect();
        let known: std::collections::HashSet<&str> =
            self.actors.iter().map(|actor| actor.id.as_str()).collect();
        let drop: Vec<String> = parents
            .iter()
            .filter(|(id, parent)| {
                !known.contains(parent.as_str()) || reaches(&parents, parent, id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in drop {
            if let Some(actor) = self.actor_mut(&id) {
                actor.components.remove("Parent");
            }
        }
    }

    /// The actor the camera is attached to, if any.
    pub fn camera_actor(&self) -> Option<&Actor> {
        self.actors.iter().find(|actor| actor.camera().is_some())
    }

    /// Leaves `actor_id` the only actor with a camera component. There is one
    /// camera, so attaching it somewhere takes it off wherever it was.
    pub fn claim_camera(&mut self, actor_id: &str) {
        for actor in &mut self.actors {
            if actor.id != actor_id {
                actor.components.remove("Camera");
            }
        }
    }

    /// Points every asset path that named `from` at `to` instead, so renaming
    /// a sprite in the asset tray doesn't leave the actor using it blank.
    /// `from` may be a folder, in which case everything under it follows.
    /// True if anything changed.
    pub fn repoint_asset(&mut self, from: &str, to: &str) -> bool {
        let mut changed = false;
        let mut repoint = |path: &mut String| {
            if let Some(next) = moved_path(path, from, to) {
                *path = next;
                changed = true;
            }
        };
        repoint(&mut self.icon);
        if let Some(font) = self.world.speech_bubble.font_asset.as_mut() {
            repoint(font);
        }
        for actor in &mut self.actors {
            for component in actor.components.iter_mut() {
                match component {
                    ActorComponent::Look {
                        visual: Visual::Image { path, .. },
                    } => repoint(path),
                    ActorComponent::Script { path } => repoint(path),
                    _ => {}
                }
            }
        }
        changed
    }
}

/// Whether following `start`'s parents ever arrives at `target` - which is
/// what makes attaching `target` to `start` a loop. The visited set is what
/// ends the walk when the chain is already one.
pub fn reaches(parents: &HashMap<String, String>, start: &str, target: &str) -> bool {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut at = start;
    loop {
        if at == target {
            return true;
        }
        if !seen.insert(at) {
            return false;
        }
        match parents.get(at) {
            Some(next) => at = next.as_str(),
            None => return false,
        }
    }
}

/// Where `path` ends up when `from` is renamed to `to` - the path itself, or
/// anything inside it when `from` is a folder. `None` when it isn't affected.
fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    path.strip_prefix(&format!("{from}/"))
        .map(|rest| format!("{to}/{rest}"))
}

fn visual_for_mode(visual: &Visual, mode: Mode) -> Visual {
    const PIXELS_PER_METRE: f32 = 100.0;

    if visual.is_3d() == mode.is_3d() {
        return visual.clone();
    }
    match visual {
        Visual::Rect { color, size } => Visual::Cuboid {
            color: color.clone(),
            size: [size[0] / PIXELS_PER_METRE, size[1] / PIXELS_PER_METRE, 1.0],
        },
        Visual::Circle { color, radius } => Visual::Sphere {
            color: color.clone(),
            radius: radius / PIXELS_PER_METRE,
        },
        Visual::Image { size, .. } => Visual::Cuboid {
            color: "#FFFFFF".to_string(),
            size: [size[0] / PIXELS_PER_METRE, size[1] / PIXELS_PER_METRE, 0.1],
        },
        Visual::Cuboid { color, size } => Visual::Rect {
            color: color.clone(),
            size: [size[0] * PIXELS_PER_METRE, size[1] * PIXELS_PER_METRE],
        },
        Visual::Sphere { color, radius } => Visual::Circle {
            color: color.clone(),
            radius: radius * PIXELS_PER_METRE,
        },
        Visual::Capsule {
            color,
            radius,
            height,
        } => Visual::Rect {
            color: color.clone(),
            size: [
                radius * 2.0 * PIXELS_PER_METRE,
                (height + radius * 2.0) * PIXELS_PER_METRE,
            ],
        },
        Visual::Plane { color, size } => Visual::Rect {
            color: color.clone(),
            size: [size[0] * PIXELS_PER_METRE, 40.0],
        },
    }
}

// ─── On-disk storage ───────────────────────────────────────────────────────

/// Blockloom's own data directory - the project registry lives here, not the
/// projects themselves.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("BLOCKLOOM_DATA_DIR")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("blockloom")
}

/// Where the New Project dialog points unless the user picks somewhere else:
/// `~/Blockloom/projects`, or under `BLOCKLOOM_DATA_DIR` when that is set, so
/// a test run never touches the real one.
pub fn default_projects_dir() -> PathBuf {
    if std::env::var_os("BLOCKLOOM_DATA_DIR").is_some() {
        return data_dir().join("projects");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Blockloom")
        .join("projects")
}

/// Where flat `<id>.blockloom` files used to live, before projects became
/// folders. Read once at startup and migrated.
pub fn legacy_projects_dir() -> PathBuf {
    data_dir().join("projects")
}

/// The document inside a project folder.
pub fn project_file(dir: &Path) -> PathBuf {
    dir.join(PROJECT_FILE)
}

/// A project folder's assets, made on creation so there is somewhere to put
/// them.
pub fn assets_dir(dir: &Path) -> PathBuf {
    dir.join(ASSETS_DIR)
}

/// Whether `dir` is a project folder.
pub fn is_project_dir(dir: &Path) -> bool {
    project_file(dir).is_file()
}

/// A folder name for a project called `name`. The app owns the folder name,
/// so anything a path can't hold becomes `-`.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    // Trailing dots and spaces are legal here but not on Windows, and a
    // leading dot would hide the folder.
    let trimmed = cleaned.trim().trim_end_matches('.').trim_start_matches('.');
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        "Project".to_string()
    } else {
        trimmed.to_string()
    }
}

/// A folder under `parent` for a project called `name`, skipping past any
/// that is already taken - `Pong`, then `Pong 2`.
pub fn unused_project_dir(parent: &Path, name: &str) -> PathBuf {
    let base = folder_name(name);
    let first = parent.join(&base);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| parent.join(format!("{base} {n}")))
        .find(|candidate| !candidate.exists())
        .expect("an unused folder always exists")
}

/// Makes a folder for `project` under `parent` and writes it there, returning
/// the folder. The project keeps its name; only the folder is deduplicated.
pub fn create_project(project: &Project, parent: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let dir = unused_project_dir(parent, &project.name);
    std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    save_project(project, &dir)?;
    Ok(dir)
}

/// Reads one project file.
pub fn read_project(path: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut project: Project =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    project.normalize();
    Ok(project)
}

/// Reads the project a folder holds.
pub fn read_project_dir(dir: &Path) -> Result<Project, String> {
    let path = project_file(dir);
    if !path.is_file() {
        return Err(format!(
            "{} isn't a Blockloom project folder",
            dir.display()
        ));
    }
    read_project(&path)
}

/// Writes `project` into its folder, making the folder and its `assets` if
/// they aren't there.
pub fn save_project(project: &Project, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(assets_dir(dir)).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = project_file(dir);
    let json = serde_json::to_string_pretty(project).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes a project to an arbitrary path, for "Export".
pub fn export_project(project: &Project, path: &Path) -> Result<(), String> {
    let json = serde_json::to_string_pretty(project).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

/// Moves a project folder so its name matches `name` again, returning where it
/// now is. A folder already named that - including this one - is left alone.
pub fn rename_project_dir(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let parent = dir.parent().ok_or("A project folder needs a parent")?;
    let wanted = folder_name(name);
    if dir
        .file_name()
        .is_some_and(|current| current == wanted.as_str())
    {
        return Ok(dir.to_path_buf());
    }
    let target = unused_project_dir(parent, &wanted);
    std::fs::rename(dir, &target)
        .map_err(|e| format!("{} -> {}: {e}", dir.display(), target.display()))?;
    Ok(target)
}

/// Deletes a project folder and everything in it. Refuses anything that isn't
/// one, so a mistyped path can't take a home directory with it.
pub fn delete_project_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    if !is_project_dir(dir) {
        return Err(format!(
            "{} isn't a Blockloom project folder, so it wasn't deleted",
            dir.display()
        ));
    }
    std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Instruction;

    /// An empty directory of its own, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("blockloom-{}", new_id()));
            std::fs::create_dir_all(&path).expect("a temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_folder_name_holds_nothing_a_path_cant() {
        assert_eq!(folder_name("Pong"), "Pong");
        assert_eq!(folder_name("  Pong 2  "), "Pong 2");
        assert_eq!(folder_name("a/b:c*d?"), "a-b-c-d-");
        assert_eq!(folder_name(".hidden."), "hidden");
        assert_eq!(folder_name("   "), "Project");
        // Slashes become dashes, which is already a usable folder name.
        assert_eq!(folder_name("///"), "---");
    }

    #[test]
    fn a_second_project_of_the_same_name_gets_a_folder_of_its_own() {
        let temp = TempDir::new();
        let project = Project::starter("Pong", Mode::TwoD);

        let first = create_project(&project, &temp.0).unwrap();
        let second = create_project(&project, &temp.0).unwrap();

        assert_eq!(first.file_name().unwrap(), "Pong");
        assert_eq!(second.file_name().unwrap(), "Pong 2");
        // Both are projects, and both have somewhere to put assets.
        assert!(is_project_dir(&first) && is_project_dir(&second));
        assert!(assets_dir(&first).is_dir());
        assert_eq!(read_project_dir(&second).unwrap().name, "Pong");
    }

    #[test]
    fn saving_and_reading_a_folder_round_trips() {
        let temp = TempDir::new();
        let mut project = Project::starter("Pong", Mode::ThreeD);
        let dir = create_project(&project, &temp.0).unwrap();

        project.create_global("score").unwrap();
        save_project(&project, &dir).unwrap();

        assert_eq!(read_project_dir(&dir).unwrap(), project);
    }

    #[test]
    fn renaming_a_project_moves_its_folder() {
        let temp = TempDir::new();
        let project = Project::starter("Pong", Mode::TwoD);
        let dir = create_project(&project, &temp.0).unwrap();

        let same = rename_project_dir(&dir, "Pong").unwrap();
        assert_eq!(same, dir);

        let moved = rename_project_dir(&dir, "Breakout: the sequel").unwrap();
        assert_eq!(moved.file_name().unwrap(), "Breakout- the sequel");
        assert!(!dir.exists());
        assert!(is_project_dir(&moved));
    }

    #[test]
    fn deleting_refuses_a_folder_that_isnt_a_project() {
        let temp = TempDir::new();
        let innocent = temp.0.join("not-a-project");
        std::fs::create_dir(&innocent).unwrap();

        assert!(delete_project_dir(&innocent).is_err());
        assert!(innocent.exists());

        let dir = create_project(&Project::starter("Pong", Mode::TwoD), &temp.0).unwrap();
        delete_project_dir(&dir).unwrap();
        assert!(!dir.exists());
        // Deleting what is already gone is not an error.
        delete_project_dir(&dir).unwrap();
    }

    #[test]
    fn a_starter_project_has_something_to_drop_and_something_to_land_on() {
        let project = Project::starter("Untitled", Mode::TwoD);
        assert_eq!(project.actors.len(), 2);
        assert_eq!(
            project.actors[0].physics().body,
            crate::scene::BodyKind::Dynamic
        );
        assert_eq!(
            project.actors[1].physics().body,
            crate::scene::BodyKind::Static
        );
    }

    #[test]
    fn switching_dimensions_converts_actor_visuals_and_units() {
        let mut project = Project::starter("Untitled", Mode::TwoD);
        project.switch_mode(Mode::ThreeD);

        assert_eq!(project.world.mode, Mode::ThreeD);
        assert_eq!(project.world.gravity, World::default_gravity(Mode::ThreeD));
        assert!((project.actors[0].placement().position[1] - 1.6).abs() < 0.001);
        assert!(
            project
                .actors
                .iter()
                .all(|actor| actor.visual().is_some_and(Visual::is_3d))
        );

        project.switch_mode(Mode::TwoD);
        assert_eq!(project.world.mode, Mode::TwoD);
        assert!((project.actors[0].placement().position[1] - 160.0).abs() < 0.001);
        assert!(
            project
                .actors
                .iter()
                .all(|actor| actor.visual().is_some_and(|visual| !visual.is_3d()))
        );
    }

    #[test]
    fn a_pre_component_actor_loads_as_components() {
        let json = r##"{
            "id": "a1",
            "name": "Player",
            "visual": {"shape": "Circle", "color": "#FFAB19", "radius": 30.0},
            "placement": {"position": [1.0, 2.0, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": 2.0},
            "physics": {"body": "Dynamic", "gravity_scale": 1.0, "lock_rotation": true,
                        "restitution": 0.0, "friction": 0.5},
            "visible": false,
            "strands": [], "floating_values": [], "comments": [],
            "variables": [], "block_defs": []
        }"##;
        let actor: Actor = serde_json::from_str(json).unwrap();

        assert_eq!(actor.placement().scale, 2.0);
        assert_eq!(actor.physics().body, crate::scene::BodyKind::Dynamic);
        assert!(!actor.visible());
        assert!(matches!(actor.visual(), Some(Visual::Circle { .. })));
        // Saving it again writes components, and nothing else.
        let json = serde_json::to_value(&actor).unwrap();
        assert!(json.get("visual").is_none());
        assert_eq!(json["components"][0]["component"], "Place");
    }

    #[test]
    fn a_component_era_actor_keeps_the_components_it_has_and_no_others() {
        let json = r#"{
            "id": "a1", "name": "Logic",
            "components": [{"component": "Place",
                            "placement": {"position": [0,0,0], "rotation": [0,0,0], "scale": 1.0}}],
            "strands": [], "floating_values": [], "comments": [],
            "variables": [], "block_defs": []
        }"#;
        let actor: Actor = serde_json::from_str(json).unwrap();

        // No `Look` means nothing to draw, not a default square.
        assert!(actor.visual().is_none());
        assert_eq!(actor.components.0.len(), 1);
    }

    #[test]
    fn a_followed_actor_gains_a_camera_component_on_load() {
        let mut project = Project::starter("p", Mode::ThreeD);
        let followed = project.actors[0].id.clone();
        project.world.camera.legacy_follow = Some(followed.clone());
        project.normalize();

        assert_eq!(project.world.camera.legacy_follow, None);
        assert_eq!(project.camera_actor().map(|a| a.id.clone()), Some(followed));
        assert_eq!(
            project.actors[0].camera().map(|c| c.view),
            Some(CameraView::Follow)
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

    #[test]
    fn a_renamed_asset_is_followed_through_the_document() {
        let mut project = Project::starter("p", Mode::TwoD);
        project.icon = "assets/sprites/player.png".to_string();
        let id = project.actors[0].id.clone();
        let actor = project.actor_mut(&id).unwrap();
        actor.components.insert(ActorComponent::Look {
            visual: Visual::Image {
                path: "assets/sprites/player.png".to_string(),
                size: [80.0, 80.0],
            },
        });
        actor.components.insert(ActorComponent::Script {
            path: "assets/scripts/player.rs".to_string(),
        });

        // The file itself.
        assert!(project.repoint_asset("assets/sprites/player.png", "assets/sprites/hero.png"));
        // A folder takes everything under it with it.
        assert!(project.repoint_asset("assets/sprites", "art"));
        // Nothing that matches is nothing to save.
        assert!(!project.repoint_asset("assets/nothing.png", "assets/still-nothing.png"));

        let actor = project.actor(&id).unwrap();
        assert_eq!(project.icon, "art/hero.png");
        assert!(matches!(
            actor.visual(),
            Some(Visual::Image { path, .. }) if path == "art/hero.png"
        ));
        assert_eq!(actor.components.script(), Some("assets/scripts/player.rs"));
    }

    #[test]
    fn a_dangling_or_looping_parent_is_dropped_when_the_document_loads() {
        let mut project = Project::starter("Parents", Mode::TwoD);
        let player = project.actors[0].id.clone();
        let ground = project.actors[1].id.clone();

        // A good one survives.
        project.actors[0].components.set_parent(&ground);
        project.normalize();
        assert_eq!(
            project.actor(&player).unwrap().parent(),
            Some(ground.as_str())
        );

        // One that names nobody doesn't.
        project.actors[0].components.set_parent("nobody");
        project.normalize();
        assert_eq!(project.actor(&player).unwrap().parent(), None);

        // Nor does a loop: one of the two ends up free rather than both
        // waiting on each other.
        project.actors[0].components.set_parent(&ground);
        project.actors[1].components.set_parent(&player);
        project.normalize();
        let still_hanging = project
            .actors
            .iter()
            .filter(|a| a.parent().is_some())
            .count();
        assert!(still_hanging <= 1);
    }

    #[test]
    fn removing_an_actor_lets_go_of_whatever_hung_off_it() {
        let mut project = Project::starter("Parents", Mode::TwoD);
        let player = project.actors[0].id.clone();
        let ground = project.actors[1].id.clone();
        project.actors[0].components.set_parent(&ground);

        assert!(project.remove_actor(&ground));
        // The child stays in the world; it just isn't hanging off anything.
        assert_eq!(project.actor(&player).unwrap().parent(), None);
    }

    #[test]
    fn moving_an_actor_reorders_it_and_optionally_reparents_it() {
        let mut project = Project::starter("Moves", Mode::TwoD);
        let player = project.actors[0].id.clone();
        let ground = project.actors[1].id.clone();
        let prop = project.add_actor(Actor::new(
            "Prop",
            crate::scene::Visual::Rect {
                color: "#FFAB19".to_string(),
                size: [10.0, 10.0],
            },
        ));
        fn order(project: &Project) -> Vec<String> {
            project
                .actors
                .iter()
                .map(|actor| actor.id.clone())
                .collect()
        }

        // Reordering the top level: Prop first.
        assert!(
            project
                .move_actor(&prop, "", &player)
                .is_ok_and(|moved| moved)
        );
        assert_eq!(
            order(&project),
            vec![prop.clone(), player.clone(), ground.clone()]
        );

        // Dropping it where it already is changes nothing.
        assert!(
            project
                .move_actor(&prop, "", &player)
                .is_ok_and(|moved| !moved)
        );
        assert_eq!(
            order(&project),
            vec![prop.clone(), player.clone(), ground.clone()]
        );

        // Hanging it off Ground puts it at the end of that level.
        assert!(
            project
                .move_actor(&prop, &ground, "")
                .is_ok_and(|moved| moved)
        );
        assert_eq!(
            project.actor(&prop).unwrap().parent(),
            Some(ground.as_str())
        );
        // Its level-mate order follows the document.
        assert!(
            project
                .move_actor(&player, &ground, &prop)
                .is_ok_and(|moved| moved)
        );
        let level: Vec<String> = project
            .actors
            .iter()
            .filter(|actor| actor.parent() == Some(ground.as_str()))
            .map(|actor| actor.id.clone())
            .collect();
        assert_eq!(level, vec![player.clone(), prop.clone()]);

        // Taking it back out leaves it where it stands.
        assert!(project.move_actor(&prop, "", "").is_ok_and(|moved| moved));
        assert_eq!(project.actor(&prop).unwrap().parent(), None);
        assert_eq!(order(&project).last(), Some(&prop));
    }

    #[test]
    fn moving_an_actor_refuses_loops_strangers_and_other_levels() {
        let mut project = Project::starter("Moves", Mode::TwoD);
        let player = project.actors[0].id.clone();
        let ground = project.actors[1].id.clone();
        project.actors[0].components.set_parent(&ground);

        // Under its own child is a loop.
        assert!(project.move_actor(&ground, &player, "").is_err());
        // Off itself is nonsense too.
        assert!(project.move_actor(&player, &player, "").is_err());
        // Nobody to hang off, nothing to land before, itself to pass.
        assert!(project.move_actor(&player, "nobody", "").is_err());
        assert!(project.move_actor(&player, "", "nobody").is_err());
        assert!(project.move_actor(&player, "", &player).is_err());
        assert!(project.move_actor("nobody", "", "").is_err());
        // `before` has to live in the level being moved to: Ground is a
        // root, not one of Player's (empty) level.
        assert!(project.move_actor(&player, &ground, &ground).is_err());
        // Nothing above failed halfway.
        assert_eq!(
            project.actor(&player).unwrap().parent(),
            Some(ground.as_str())
        );
    }

    #[test]
    fn an_actor_made_mid_run_has_somewhere_to_stand_and_something_to_see() {
        let flat = Actor::blank("Bullet", Mode::TwoD);
        assert!(matches!(flat.visual(), Some(Visual::Rect { .. })));
        assert!(flat.components.contains("Place"));
        assert!(flat.graph.strands.is_empty());

        let solid = Actor::blank("Bullet", Mode::ThreeD);
        assert!(matches!(solid.visual(), Some(Visual::Cuboid { .. })));
    }

    #[test]
    fn shared_lists_are_created_renamed_and_removed() {
        let mut project = Project::starter("p", Mode::TwoD);
        assert!(project.create_global_list("  ").is_err());
        project.create_global_list("queue").unwrap();
        assert!(project.create_global_list("queue").is_err());

        let id = project.actors[0].id.clone();
        assert!(project.is_global_list(&id, "queue"));
        project.rename_global_list("queue", "line").unwrap();
        assert!(project.is_global_list(&id, "line"));
        assert!(project.rename_global_list("line", "line").is_ok());
        project.remove_global_list("line");
        assert!(!project.is_global_list(&id, "line"));
    }

    #[test]
    fn renaming_a_shared_list_follows_reads_except_where_an_actor_shadows_it() {
        let mut project = Project::starter("p", Mode::TwoD);
        project.create_global_list("queue").unwrap();
        let shadowing = project.actors[1].id.clone();
        project
            .actor_mut(&shadowing)
            .unwrap()
            .graph
            .create_list("queue")
            .unwrap();
        for index in 0..2 {
            let adds = Instruction::new(InstructionKind::AddToList {
                value: crate::value::Value::number(1.0),
                name: "queue".to_string(),
            });
            let reads = Instruction::new(InstructionKind::Say {
                text: crate::value::Value::Op {
                    op: crate::value::Op::from_name("ListItem"),
                    args: vec![
                        crate::value::Value::number(1.0),
                        crate::value::Value::text("queue"),
                    ],
                    saved: Box::new(crate::value::Value::number(0.0)),
                },
            });
            project.actors[index]
                .graph
                .strands
                .push(crate::blocks::Strand::with_instructions(
                    0,
                    0,
                    vec![adds, reads],
                ));
        }
        project.rename_global_list("queue", "line").unwrap();

        let target_of = |actor: &Actor| match &actor.graph.strands[0].instructions[0].kind {
            InstructionKind::AddToList { name, .. } => name.clone(),
            other => panic!("expected an add, got {other:?}"),
        };
        let read_of = |actor: &Actor| match &actor.graph.strands[0].instructions[1].kind {
            InstructionKind::Say {
                text: crate::value::Value::Op { args, .. },
            } => match &args[1] {
                crate::value::Value::Text { value } => value.clone(),
                other => panic!("expected a list name, got {other:?}"),
            },
            other => panic!("expected a say, got {other:?}"),
        };
        assert_eq!(target_of(&project.actors[0]), "line");
        assert_eq!(read_of(&project.actors[0]), "line");
        assert_eq!(target_of(&project.actors[1]), "queue");
        assert_eq!(read_of(&project.actors[1]), "queue");
    }
}
