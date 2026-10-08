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

use crate::blocks::{ActorGraph, BlockKind, DictDef, InstructionKind, ListDef, VariableDef};
use crate::components::{ActorComponent, CameraAttach, CameraView, Components};
use crate::scene::{Mode, Physics, Placement, Visual, World};
use crate::value::Evaluated;
use blockloom_plugin_api::record::PluginRecord;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

/// File extension of a saved scene asset. Each scene lives as its own file
/// under [`SCENES_DIR`], Unity-style, so scenes can be shared, duplicated
/// and versioned like any other asset.
pub const SCENE_EXTENSION: &str = "blockscene";

/// Where a project folder keeps its scene assets, relative to the folder.
/// Under `assets/` so the asset tray lists them beside everything else.
pub const SCENES_DIR: &str = "assets/scenes";

/// The scene file shape [`SceneFile`] writes. Bumped when it changes.
pub const SCENE_FORMAT: u32 = 1;

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
                    layer: 0,
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

    /// Re-expresses everything here that is a length (where it stands, what
    /// it looks like, where it hangs off its parent, its joint) in `to`'s
    /// units, when `from` is the other dimension. A 2D z is only a draw
    /// order, so it does not survive the way a 3D one does.
    pub fn convert_units(&mut self, from: Mode, to: Mode) {
        if from == to {
            return;
        }
        let length = |value: f32| to.length_from(from, value);
        let flatten = |position: [f32; 3]| {
            [
                length(position[0]),
                length(position[1]),
                if to.is_3d() { length(position[2]) } else { 0.0 },
            ]
        };
        let placement = self.components.placement_mut();
        placement.position = flatten(placement.position);
        if let Some(visual) = self.components.visual() {
            let converted = visual_for_mode(visual, to);
            self.components.set_visual(converted);
        }
        if let Some(offset) = self.components.parent_offset() {
            self.components.set_parent_offset(Some(flatten(offset)));
        }
        if let Some(ActorComponent::Joint { joint }) = self.components.get_mut("Joint") {
            joint.anchor = joint.anchor.map(length);
            joint.length = length(joint.length);
        }
        for component in self.components.iter_mut() {
            if let ActorComponent::Constraint { constraint } = component {
                constraint.anchor = constraint.anchor.map(length);
                constraint.connected_anchor = constraint.connected_anchor.map(length);
                constraint.min_distance = length(constraint.min_distance);
                constraint.max_distance = length(constraint.max_distance);
                constraint.spring.rest_length = length(constraint.spring.rest_length);
                if matches!(
                    constraint.kind,
                    crate::physics::ConstraintKind::Slider | crate::physics::ConstraintKind::Wheel
                ) {
                    constraint.limit.min = length(constraint.limit.min);
                    constraint.limit.max = length(constraint.limit.max);
                }
            }
        }
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

    /// Whether this actor survives `switch scene to`: an opt-in `Persist`
    /// component, authored or attached mid-run.
    pub fn persists(&self) -> bool {
        self.components.persists()
    }
}

/// One scene: its own world settings and its own actors. A project holds a
/// list of these; the editor shows one at a time and the runtime loads one.
/// Each scene carries its own `World` (including `mode`), so v1 supports
/// mixed 2D/3D scenes - a scene switch across dimensions rebuilds the
/// dim2/dim3 pipeline the way a project dimension switch does today.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default = "default_scene_name")]
    pub name: String,
    /// Project-relative asset path of this scene's `.blockscene` file, e.g.
    /// `assets/scenes/Level 1.blockscene`. Empty in memory means the legacy
    /// id-named file; [`Scene::asset_path`] falls back to that.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub world: World,
    #[serde(default)]
    pub actors: Vec<Actor>,
}

fn default_scene_name() -> String {
    "Scene 1".to_string()
}

impl Scene {
    pub fn new(name: impl Into<String>, mode: Mode) -> Self {
        let mut world = World {
            mode,
            gravity: World::default_gravity(mode),
            ..World::default()
        };
        if mode.is_3d() {
            world.camera = crate::scene::Camera::default();
        }
        let name = {
            let name = name.into();
            let trimmed = name.trim();
            if trimmed.is_empty() {
                default_scene_name()
            } else {
                trimmed.to_string()
            }
        };
        let path = scene_path_for_name(&name);
        Self {
            id: new_id(),
            name,
            path,
            world,
            actors: Vec::new(),
        }
    }

    /// Converts the scene to the target dimension. Actors keep their color and
    /// approximate size, while positions move between pixels and metres.
    pub fn switch_mode(&mut self, mode: Mode) {
        let previous = self.world.mode;
        if previous == mode {
            return;
        }

        self.world.gravity = if self.world.gravity == World::default_gravity(previous) {
            World::default_gravity(mode)
        } else {
            self.world.gravity.map(|g| mode.length_from(previous, g))
        };
        for actor in &mut self.actors {
            actor.convert_units(previous, mode);
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
        let removed = self.actors.remove(index);
        // Its children are left where they are rather than going with it -
        // a parent is an attachment, not an owner.
        for actor in &mut self.actors {
            if actor.components.parent() == Some(id) {
                actor.components.remove("Parent");
            }
            if actor
                .components
                .joint()
                .is_some_and(|joint| joint.target == id)
            {
                actor.components.remove("Joint");
            }
            let dangling: Vec<_> = actor
                .components
                .constraints()
                .filter(|c| c.target == id)
                .map(|c| c.id.clone())
                .collect();
            for constraint in dangling {
                actor.components.remove_constraint(&constraint);
            }
            if let Some(ActorComponent::Brain { brain }) = actor.components.get_mut("Brain")
                && (brain.target == id || brain.target.eq_ignore_ascii_case(&removed.name))
            {
                brain.target.clear();
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
            if let Some(ActorComponent::Brain { brain }) = actor.components.get_mut("Brain")
                && brain.target.eq_ignore_ascii_case(&old)
            {
                brain.target.clone_from(&trimmed);
            }
            actor
                .graph
                .walk_instructions_mut(&mut |ins| match &mut ins.kind {
                    InstructionKind::PointTowards { target } if *target == old => {
                        *target = trimmed.clone()
                    }
                    InstructionKind::WhenCollision { with, .. } if *with == old => {
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

    fn normalize_scene(&mut self) {
        self.migrate_camera_follow();
        self.prune_parents();
        let known: std::collections::HashSet<String> =
            self.actors.iter().map(|a| a.id.clone()).collect();
        for actor in &mut self.actors {
            if actor.components.joint().is_some_and(|joint| {
                joint.target == actor.id
                    || (!joint.target.is_empty() && !known.contains(&joint.target))
            }) {
                actor.components.remove("Joint");
            }
        }
        self.world.input.normalize();
        for actor in &mut self.actors {
            if let Some(ActorComponent::Material { material }) =
                actor.components.get_mut("Material")
            {
                material.normalize();
            }
            if let Some(ActorComponent::Terrain { terrain }) = actor.components.get_mut("Terrain") {
                terrain.normalize();
            }
            if let Some(ActorComponent::Animation { animation }) =
                actor.components.get_mut("Animation")
            {
                animation.normalize();
            }
            if let Some(ActorComponent::Sprite { sprite }) = actor.components.get_mut("Sprite") {
                sprite.normalize();
            }
            if let Some(ActorComponent::Water { water }) = actor.components.get_mut("Water") {
                water.normalize();
            }
            if let Some(ActorComponent::Buoyancy { buoyancy }) =
                actor.components.get_mut("Buoyancy")
            {
                buoyancy.normalize();
            }
            if let Some(ActorComponent::Parallax { parallax }) =
                actor.components.get_mut("Parallax")
            {
                parallax.normalize();
            }
            if let Some(ActorComponent::Room { room }) = actor.components.get_mut("Room") {
                room.normalize();
            }
            if let Some(ActorComponent::Look {
                visual: crate::scene::Visual::Tilemap { tilemap },
            }) = actor.components.get_mut("Look")
            {
                tilemap.normalize();
            }
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
}

// ─── Scene assets ──────────────────────────────────────────────────────────
// A scene is a Unity-style asset: one `.blockscene` file per scene under
// `assets/scenes/`, carrying its settings as components on the scene itself.
// The project file is an index over those assets; the runtime, packs and
// exports still carry full scenes in memory.

/// One entry in the project file's scene index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneRef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Project-relative path to the scene asset, e.g.
    /// `assets/scenes/Level 1.blockscene`.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub mode: Mode,
}

/// The project file on disk: an index over scene assets plus everything that
/// isn't per-scene. Scene files hold the actors and the world settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub android: crate::android::AndroidSettings,
    #[serde(default)]
    pub active_scene: String,
    /// Which scene a fresh open - and a built game - boots into. Empty in
    /// older files means the active scene.
    #[serde(default)]
    pub default_scene: String,
    #[serde(default)]
    pub scenes: Vec<SceneRef>,
    #[serde(default)]
    pub globals: Vec<VariableDef>,
    #[serde(default)]
    pub global_lists: Vec<ListDef>,
    #[serde(default)]
    pub global_dicts: Vec<DictDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin_resources: Vec<PluginRecord>,
    #[serde(
        default,
        skip_serializing_if = "crate::physics::PhysicsSettings::is_default"
    )]
    pub physics: crate::physics::PhysicsSettings,
    #[serde(
        default,
        skip_serializing_if = "crate::multiplayer::MultiplayerSettings::is_default"
    )]
    pub multiplayer: crate::multiplayer::MultiplayerSettings,
    #[serde(
        default,
        skip_serializing_if = "crate::locale::Localization::is_default"
    )]
    pub localization: crate::locale::Localization,
}

/// A scene asset file: its settings as components plus its actors. Older
/// scene documents kept a fixed `world`; those still load - a file naming
/// `components` wins, otherwise `world` is used, otherwise the default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default = "default_scene_name")]
    pub name: String,
    #[serde(default = "scene_format_default")]
    pub format: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<crate::scene_components::SceneComponents>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<World>,
    #[serde(default)]
    pub actors: Vec<Actor>,
}

fn scene_format_default() -> u32 {
    SCENE_FORMAT
}

impl Scene {
    /// The project-relative asset path for this scene. New scenes are
    /// name-based (`assets/scenes/<name>.blockscene`) so the tray filename is
    /// the scene name; older id-named files keep working through the fallback.
    pub fn asset_path(&self) -> String {
        if self.path.is_empty() {
            scene_asset_path(&self.id)
        } else {
            self.path.clone()
        }
    }

    /// This scene as a file: its world as components plus its actors.
    pub fn to_file(&self) -> SceneFile {
        SceneFile {
            id: self.id.clone(),
            name: self.name.clone(),
            format: SCENE_FORMAT,
            components: Some(crate::scene_components::SceneComponents::from_world(
                &self.world,
            )),
            world: None,
            actors: self.actors.clone(),
        }
    }

    /// A file back into a scene. Prefers `components`; falls back to a fixed
    /// `world` for hand-written or older files.
    pub fn from_file(file: SceneFile) -> Self {
        let world = match file.components {
            Some(components) if !components.0.is_empty() => components.to_world(),
            _ => file.world.unwrap_or_default(),
        };
        Self {
            id: if file.id.is_empty() {
                new_id()
            } else {
                file.id
            },
            name: if file.name.trim().is_empty() {
                default_scene_name()
            } else {
                file.name
            },
            // The index sets the real file on load; elsewhere the caller
            // does (see `import_scene`).
            path: String::new(),
            world,
            actors: file.actors,
        }
    }
}

/// The project-relative asset path for the scene with `scene_id`. Legacy
/// shape; new scenes are name-based - see [`scene_path_for_name`].
pub fn scene_asset_path(scene_id: &str) -> String {
    format!("{SCENES_DIR}/{scene_id}.{SCENE_EXTENSION}")
}

/// The asset path a scene called `name` lives at: the scenes folder, the
/// scene name as the filename, so renaming the file renames the scene and
/// back. Characters no file can carry become `_`.
pub fn scene_path_for_name(name: &str) -> String {
    let mut stem: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .trim()
        .to_string();
    if stem.is_empty() {
        stem = "Scene".to_string();
    }
    // Keep the `.blockscene` extension intact: a trailing dot would merge.
    if stem.ends_with('.') {
        stem.pop();
    }
    format!("{SCENES_DIR}/{stem}.{SCENE_EXTENSION}")
}

/// What a scene file at `relative` (project-relative) calls its scene: the
/// filename without its extension. Empty when the path has no stem.
pub fn scene_name_for_path(relative: &str) -> String {
    let normalized = relative.replace('\\', "/");
    let file = normalized.rsplit('/').next().unwrap_or(&normalized);
    let stem = file
        .strip_suffix(&format!(".{SCENE_EXTENSION}"))
        .or_else(|| file.strip_suffix(&format!(".{}", SCENE_EXTENSION.to_uppercase())))
        .unwrap_or(file);
    // A case-only mismatch still counts; compare lowercased at the call site.
    stem.trim().to_string()
}

/// The folder inside a project dir that holds scene assets.
pub fn scenes_dir(dir: &Path) -> PathBuf {
    dir.join(SCENES_DIR)
}

/// Whether `relative` (project-relative) is a scene asset file: anything
/// ending in `.blockscene`, wherever it lives. Scenes are normal tray files.
pub fn is_scene_asset(relative: &str) -> bool {
    let normalized = relative.replace('\\', "/");
    let lower = normalized.to_lowercase();
    lower.ends_with(&format!(".{SCENE_EXTENSION}"))
        && !lower.ends_with(&format!("/{SCENE_EXTENSION}"))
        && normalized
            .rsplit('/')
            .next()
            .is_some_and(|file| file.len() > SCENE_EXTENSION.len() + 1)
}

/// Reads one scene asset file.
pub fn read_scene_file(path: &Path) -> Result<Scene, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: SceneFile =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Scene::from_file(file))
}

/// Finds a scene asset by the scene id it carries, wherever the tray keeps
/// it. What lets an open follow a file renamed or moved outside the editor.
fn find_scene_file(dir: &Path, scene_id: &str) -> Option<String> {
    fn visit(dir: &Path, base: &Path, scene_id: &str, out: &mut Option<String>) {
        if out.is_some() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if out.is_some() {
                return;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                visit(&path, base, scene_id, out);
            } else if name
                .to_lowercase()
                .ends_with(&format!(".{SCENE_EXTENSION}"))
            {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(file): Result<SceneFile, _> = serde_json::from_str(&text) else {
                    continue;
                };
                if file.id == scene_id
                    && let Ok(relative) = path.strip_prefix(base)
                {
                    *out = Some(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut out = None;
    visit(dir, dir, scene_id, &mut out);
    out
}

/// Writes one scene asset file, making the scenes dir as needed. Answers its
/// project-relative path.
pub fn save_scene_file(dir: &Path, scene: &Scene) -> Result<String, String> {
    let relative = scene.asset_path();
    let full = dir.join(&relative);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&scene.to_file()).map_err(|e| e.to_string())?;
    std::fs::write(&full, json).map_err(|e| format!("{}: {e}", full.display()))?;
    Ok(relative)
}

/// A whole project: what the editor edits, what the runtime is handed, and
/// what a `.blockloom` file holds. The project holds the scene list; each
/// scene has its own actors and `World` settings, and one scene is active.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// An image asset used to brand packaged builds. Empty uses Blockloom's
    /// bundled icon.
    pub icon: String,
    /// applicationId, version and friends for Android builds. Old files
    /// carry none and read as the defaults.
    pub android: crate::android::AndroidSettings,
    pub scenes: Vec<Scene>,
    pub active_scene: String,
    /// Which scene a fresh open - and a built game - boots into. The editor
    /// keeps editing wherever it is; opening the project loads this one.
    pub default_scene: String,
    /// Variables every actor can read and write - see the module docs.
    pub globals: Vec<VariableDef>,
    /// Lists every actor can read and change - the shared half of the list
    /// model, parallel to [`Project::globals`]. An actor's own list of the
    /// same name shadows this one for that actor.
    pub global_lists: Vec<ListDef>,
    /// Dicts every actor can read and change - the shared half of the dict
    /// model, parallel to [`Project::global_lists`]. An actor's own dict of
    /// the same name shadows this one for that actor.
    pub global_dicts: Vec<DictDef>,
    /// Records plugins keep on the project itself rather than on an actor.
    /// Opaque to the document: see [`PluginRecord`].
    pub plugin_resources: Vec<PluginRecord>,
    /// Physics schema version, compatibility profile and stored materials. Absent
    /// (and written absent) for a project that has none of that.
    pub physics: crate::physics::PhysicsSettings,
    pub multiplayer: crate::multiplayer::MultiplayerSettings,
    /// Localized interface strings plus the language a fresh run speaks.
    /// Absent (and written absent) for a project with no translations.
    pub localization: crate::locale::Localization,
}

// ─── Scene-backed project ────────────────────────────────────────────────

impl Deref for Project {
    type Target = Scene;
    fn deref(&self) -> &Scene {
        self.active_scene_ref()
    }
}

impl DerefMut for Project {
    fn deref_mut(&mut self) -> &mut Scene {
        self.active_scene_mut()
    }
}

impl Serialize for Project {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let active = self.active_scene_ref();
        let mut s = serializer.serialize_struct("Project", 14)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("name", &self.name)?;
        s.serialize_field("icon", &self.icon)?;
        s.serialize_field("android", &self.android)?;
        s.serialize_field("scenes", &self.scenes)?;
        s.serialize_field("active_scene", &self.active_scene)?;
        s.serialize_field("default_scene", &self.default_scene)?;
        // Compat: the active scene flattened, so older readers and the
        // current QML (`project.world`, `project.actors`) keep working.
        s.serialize_field("world", &active.world)?;
        s.serialize_field("actors", &active.actors)?;
        s.serialize_field("globals", &self.globals)?;
        s.serialize_field("global_lists", &self.global_lists)?;
        s.serialize_field("global_dicts", &self.global_dicts)?;
        if !self.plugin_resources.is_empty() {
            s.serialize_field("plugin_resources", &self.plugin_resources)?;
        }
        if !self.physics.is_default() {
            s.serialize_field("physics", &self.physics)?;
        }
        if !self.multiplayer.is_default() {
            s.serialize_field("multiplayer", &self.multiplayer)?;
        }
        if !self.localization.is_default() {
            s.serialize_field("localization", &self.localization)?;
        }
        s.end()
    }
}

impl<'de> Deserialize<'de> for Project {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct ProjectDe {
            #[serde(default = "new_id")]
            id: String,
            #[serde(default)]
            name: String,
            #[serde(default)]
            icon: String,
            #[serde(default)]
            android: crate::android::AndroidSettings,
            #[serde(default)]
            scenes: Option<Vec<Scene>>,
            #[serde(default)]
            active_scene: Option<String>,
            #[serde(default)]
            default_scene: Option<String>,
            #[serde(default)]
            world: Option<World>,
            #[serde(default)]
            actors: Option<Vec<Actor>>,
            #[serde(default)]
            globals: Vec<VariableDef>,
            #[serde(default)]
            global_lists: Vec<ListDef>,
            #[serde(default)]
            global_dicts: Vec<DictDef>,
            #[serde(default)]
            plugin_resources: Vec<PluginRecord>,
            #[serde(default)]
            physics: crate::physics::PhysicsSettings,
            #[serde(default)]
            multiplayer: crate::multiplayer::MultiplayerSettings,
            #[serde(default)]
            localization: crate::locale::Localization,
        }
        let de = ProjectDe::deserialize(deserializer)?;
        de.physics
            .check_version()
            .map_err(<D::Error as serde::de::Error>::custom)?;
        let mut scenes = de.scenes.unwrap_or_default();
        if scenes.is_empty() {
            // Old single-scene document: migrate as scene one.
            let name = default_scene_name();
            scenes.push(Scene {
                id: new_id(),
                path: scene_path_for_name(&name),
                name,
                world: de.world.unwrap_or_default(),
                actors: de.actors.unwrap_or_default(),
            });
        }
        let active_scene = de.active_scene.unwrap_or_default();
        let active_scene = if scenes.iter().any(|s| s.id == active_scene) {
            active_scene
        } else {
            scenes[0].id.clone()
        };
        let default_scene = de.default_scene.unwrap_or_default();
        let default_scene = if scenes.iter().any(|s| s.id == default_scene) {
            default_scene
        } else {
            active_scene.clone()
        };
        let mut project = Self {
            id: if de.id.is_empty() { new_id() } else { de.id },
            name: de.name,
            icon: de.icon,
            android: de.android,
            scenes,
            active_scene,
            default_scene,
            globals: de.globals,
            global_lists: de.global_lists,
            global_dicts: de.global_dicts,
            plugin_resources: de.plugin_resources,
            physics: de.physics,
            multiplayer: de.multiplayer,
            localization: de.localization,
        };
        project.ensure_scene_invariants();
        Ok(project)
    }
}

/// What shape of plugin block a canvas uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginBlockShape {
    Statement,
    Reporter,
    Hat,
}

/// One plugin block on a canvas, as [`Project::plugin_blocks`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBlockUse {
    pub place: String,
    pub plugin: String,
    pub block: String,
    pub slots: usize,
    pub shape: PluginBlockShape,
}

/// Plugin reporters inside `value`: plugin, block id, slot count.
fn plugin_reads(value: &crate::value::Value, found: &mut Vec<(String, String, usize)>) {
    use crate::value::{Op, PLUGIN_READ, Value};
    if let Value::Op { op, args, .. } = value {
        if let Op::Ext(name) = op
            && &**name == PLUGIN_READ
        {
            let text = |i: usize| match args.get(i) {
                Some(Value::Text { value }) => value.clone(),
                _ => String::new(),
            };
            found.push((text(0), text(1), args.len().saturating_sub(2)));
        }
        for arg in args {
            plugin_reads(arg, found);
        }
    } else if let Value::Call { args, .. } = value {
        for arg in args {
            plugin_reads(arg, found);
        }
    }
}

impl Project {
    /// Every record a plugin owns in the whole document, each with where it
    /// lives (`actor Player`, `actor Player in scene Level 1`, `resource`).
    pub fn plugin_records(&self) -> Vec<(String, &PluginRecord)> {
        let several = self.scenes.len() > 1;
        let mut out = Vec::new();
        for scene in &self.scenes {
            for actor in &scene.actors {
                for record in actor.components.plugin_records() {
                    let place = if several {
                        format!("actor {} in scene {}", actor.name, scene.name)
                    } else {
                        format!("actor {}", actor.name)
                    };
                    out.push((place, record));
                }
            }
        }
        out.extend(
            self.plugin_resources
                .iter()
                .map(|record| ("resource".to_string(), record)),
        );
        out
    }

    /// Every plugin block placed on a canvas (statements, reporters and
    /// hats): where it sits, its plugin, its block id and how many slots the
    /// instruction carries.
    pub fn plugin_blocks(&self) -> Vec<PluginBlockUse> {
        use crate::blocks::InstructionKind as K;
        let several = self.scenes.len() > 1;
        let mut out = Vec::new();
        for scene in &self.scenes {
            for actor in &scene.actors {
                let place = if several {
                    format!("actor {} in scene {}", actor.name, scene.name)
                } else {
                    format!("actor {}", actor.name)
                };
                actor
                    .graph
                    .walk_instructions(&mut |instruction| match &instruction.kind {
                        K::PluginBlock {
                            plugin,
                            block,
                            args,
                        } => out.push(PluginBlockUse {
                            place: place.clone(),
                            plugin: plugin.clone(),
                            block: block.clone(),
                            slots: args.len(),
                            shape: PluginBlockShape::Statement,
                        }),
                        K::WhenPlugin {
                            plugin,
                            block,
                            args,
                            ..
                        } => out.push(PluginBlockUse {
                            place: place.clone(),
                            plugin: plugin.clone(),
                            block: block.clone(),
                            slots: args.len(),
                            shape: PluginBlockShape::Hat,
                        }),
                        _ => {}
                    });
                let mut reads = Vec::new();
                let mut graph = actor.graph.clone();
                graph.visit_values_mut(&mut |value, _| plugin_reads(value, &mut reads));
                out.extend(
                    reads
                        .into_iter()
                        .map(|(plugin, block, slots)| PluginBlockUse {
                            place: place.clone(),
                            plugin,
                            block,
                            slots,
                            shape: PluginBlockShape::Reporter,
                        }),
                );
            }
        }
        out
    }

    /// Ids of the plugins the document holds records for.
    pub fn plugin_ids(&self) -> std::collections::BTreeSet<String> {
        self.plugin_records()
            .into_iter()
            .map(|(_, record)| record.plugin.clone())
            .collect()
    }

    /// Sets a project resource, replacing the one of the same name.
    pub fn set_plugin_resource(&mut self, record: PluginRecord) {
        match self
            .plugin_resources
            .iter_mut()
            .find(|existing| existing.name() == record.name())
        {
            Some(slot) => *slot = record,
            None => self.plugin_resources.push(record),
        }
    }

    pub fn remove_plugin_resource(&mut self, name: &str) -> bool {
        let before = self.plugin_resources.len();
        self.plugin_resources.retain(|record| record.name() != name);
        self.plugin_resources.len() != before
    }

    /// Replaces every record named like one in `records` (components on any
    /// actor, and resources) with its new version, in place. Answers how
    /// many were replaced. This is how a migration is swapped in whole.
    pub fn replace_plugin_records(&mut self, records: &[PluginRecord]) -> usize {
        let mut replaced = 0;
        let mut swap = |slot: &mut PluginRecord| {
            if let Some(new) = records
                .iter()
                .find(|r| r.name() == slot.name() && r.schema_version != slot.schema_version)
            {
                *slot = new.clone();
                replaced += 1;
            }
        };
        for scene in &mut self.scenes {
            for actor in &mut scene.actors {
                for component in &mut actor.components.0 {
                    if let ActorComponent::Plugin { record } = component {
                        swap(record);
                    }
                }
            }
        }
        for record in &mut self.plugin_resources {
            swap(record);
        }
        replaced
    }
}

impl Project {
    /// A fresh project: one scene with one actor to move and one static
    /// floor to land on, so gravity means something the moment Play is pressed.
    pub fn starter(name: impl Into<String>, mode: crate::scene::Mode) -> Self {
        let mut world = World {
            mode,
            gravity: World::default_gravity(mode),
            input: crate::input::InputConfig::starter(),
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

        let scene = Scene {
            id: new_id(),
            path: scene_path_for_name(&default_scene_name()),
            name: default_scene_name(),
            world,
            actors: vec![player, ground],
        };
        let active_scene = scene.id.clone();
        Self {
            id: new_id(),
            name: name.into(),
            icon: String::new(),
            android: crate::android::AndroidSettings::default(),
            scenes: vec![scene],
            active_scene: active_scene.clone(),
            default_scene: active_scene,
            globals: Vec::new(),
            global_lists: Vec::new(),
            global_dicts: Vec::new(),
            plugin_resources: Vec::new(),
            physics: crate::physics::PhysicsSettings::default(),
            multiplayer: Default::default(),
            localization: Default::default(),
        }
    }

    fn active_scene_ref(&self) -> &Scene {
        if let Some(scene) = self.scenes.iter().find(|s| s.id == self.active_scene) {
            return scene;
        }
        &self.scenes[0]
    }

    /// The scene the editor shows and the runtime loads.
    pub fn active_scene(&self) -> &Scene {
        self.active_scene_ref()
    }

    pub fn active_scene_mut(&mut self) -> &mut Scene {
        self.ensure_scene_invariants();
        let active = self.active_scene.clone();
        if let Some(i) = self.scenes.iter().position(|s| s.id == active) {
            return &mut self.scenes[i];
        }
        &mut self.scenes[0]
    }

    pub fn scene(&self, id: &str) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    pub fn scene_mut(&mut self, id: &str) -> Option<&mut Scene> {
        self.scenes.iter_mut().find(|s| s.id == id)
    }

    /// The scene whose asset lives at `relative` (project-relative), if any.
    /// Scene files are normal tray files; this is how the tray maps one back
    /// to its scene.
    pub fn scene_for_asset(&self, relative: &str) -> Option<&Scene> {
        let normalized = relative.replace('\\', "/");
        self.scenes
            .iter()
            .find(|scene| scene.asset_path() == normalized)
    }

    /// The same, mutable.
    pub fn scene_for_asset_mut(&mut self, relative: &str) -> Option<&mut Scene> {
        let normalized = relative.replace('\\', "/");
        self.scenes
            .iter_mut()
            .find(|scene| scene.asset_path() == normalized)
    }

    /// The scene whose file stem names it, if any. How a double-click finds
    /// its scene when the file was renamed outside the editor.
    pub fn scene_for_stem(&self, stem: &str) -> Option<&Scene> {
        self.scenes.iter().find(|scene| {
            scene.name == stem
                || scene_name_for_path(&scene.asset_path()).to_lowercase() == stem.to_lowercase()
        })
    }

    /// Which scene a fresh open - and a built game - boots into: the default
    /// while it names a scene, else the active one.
    pub fn boot_scene_id(&self) -> String {
        if self.scenes.iter().any(|s| s.id == self.default_scene) {
            self.default_scene.clone()
        } else {
            self.active_scene.clone()
        }
    }

    /// Makes `id` the boot scene: what a fresh open and a built game load.
    pub fn set_default_scene(&mut self, id: &str) -> Result<(), String> {
        if !self.scenes.iter().any(|s| s.id == id) {
            return Err("Scene not found".to_string());
        }
        self.default_scene = id.to_string();
        Ok(())
    }

    fn ensure_scene_invariants(&mut self) {
        if self.scenes.is_empty() {
            self.scenes
                .push(Scene::new(default_scene_name(), Mode::TwoD));
        }
        if !self.scenes.iter().any(|s| s.id == self.active_scene) {
            self.active_scene = self.scenes[0].id.clone();
        }
        if !self.scenes.iter().any(|s| s.id == self.default_scene) {
            self.default_scene = self.active_scene.clone();
        }
        // Every scene knows its own file; legacy id-named ones keep working.
        for scene in &mut self.scenes {
            if scene.path.is_empty() {
                scene.path = scene_asset_path(&scene.id);
            }
        }
        // Scene names stay unique; a hand-edited file could repeat one.
        let mut seen = std::collections::HashSet::new();
        for scene in &mut self.scenes {
            let base = if scene.name.trim().is_empty() {
                default_scene_name()
            } else {
                scene.name.clone()
            };
            let mut candidate = base.clone();
            let mut n = 2;
            while !seen.insert(candidate.clone()) {
                candidate = format!("{base} {n}");
                n += 1;
            }
            scene.name = candidate;
        }
    }

    /// An unused scene name based on `base`.
    pub fn unique_scene_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Scene" } else { base };
        if !self.scenes.iter().any(|s| s.name == base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|c| !self.scenes.iter().any(|s| &s.name == c))
            .expect("an unused scene name always exists")
    }

    /// Adds an empty scene of `mode` (active scene's mode when omitted) and
    /// makes it active. Answers its id. The file is name-based, so the tray
    /// filename is the scene name.
    pub fn add_scene(&mut self, name: &str, mode: Option<Mode>) -> String {
        let mode = mode.unwrap_or_else(|| self.active_scene_ref().world.mode);
        let mut scene = Scene::new(self.unique_scene_name(name), mode);
        // A fresh input config per scene; starter bindings when empty so a
        // new scene plays like the first one.
        if scene.world.input.actions.is_empty() {
            scene.world.input = crate::input::InputConfig::starter();
        }
        scene.path = self.unused_scene_path(&scene.name);
        let id = scene.id.clone();
        self.scenes.push(scene);
        self.active_scene = id.clone();
        id
    }

    /// Adds an empty scene at `path` (project-relative), for the tray's New
    /// scene: the filename is the scene name. Answers the new scene's id.
    pub fn add_scene_at(&mut self, path: &str, mode: Option<Mode>) -> Result<String, String> {
        let normalized = path.replace('\\', "/");
        if !is_scene_asset(&normalized) {
            return Err("A scene file ends in .blockscene".to_string());
        }
        if self.scene_for_asset(&normalized).is_some() {
            return Err("That file is already a scene".to_string());
        }
        let stem = scene_name_for_path(&normalized);
        if stem.is_empty() {
            return Err("A scene needs a name".to_string());
        }
        let mode = mode.unwrap_or_else(|| self.active_scene_ref().world.mode);
        let mut scene = Scene::new(self.unique_scene_name(&stem), mode);
        if scene.world.input.actions.is_empty() {
            scene.world.input = crate::input::InputConfig::starter();
        }
        // The tray dedupes the filename; the scene name follows the actual
        // file, with a numeric tail when the name was taken.
        scene.path = normalized.clone();
        // A duplicate scene name from another folder gets its tail here.
        if self.scenes.iter().any(|s| s.name == scene.name) {
            scene.name = self.unique_scene_name(&scene.name);
        }
        let id = scene.id.clone();
        self.scenes.push(scene);
        self.active_scene = id.clone();
        Ok(id)
    }

    /// A scene path nothing names yet, near `wanted`: name-based, with a
    /// numeric tail when the file is taken.
    pub fn unused_scene_path(&self, name: &str) -> String {
        let wanted = scene_path_for_name(name);
        if !self.scenes.iter().any(|s| s.asset_path() == wanted) {
            return wanted;
        }
        let stem = scene_name_for_path(&wanted);
        (2..)
            .map(|n| scene_path_for_name(&format!("{stem} {n}")))
            .find(|candidate| !self.scenes.iter().any(|s| s.asset_path() == *candidate))
            .expect("an unused scene path always exists")
    }

    /// Copies `id` under a fresh name and id, with its own actor ids so the
    /// two scenes never share one. The copy becomes active.
    pub fn duplicate_scene(&mut self, id: &str) -> Result<String, String> {
        let Some(from) = self.scene(id).cloned() else {
            return Err("Scene not found".to_string());
        };
        let mut copy = from;
        copy.id = new_id();
        copy.name = self.unique_scene_name(&format!("{} copy", copy.name));
        copy.path = self.unused_scene_path(&copy.name);
        // Actor ids only need to be unique within a run, but fresh ones keep
        // cross-scene references from leaking between the two.
        let mut remap = HashMap::new();
        for actor in &mut copy.actors {
            let next = new_id();
            remap.insert(actor.id.clone(), next.clone());
            actor.id = next;
        }
        for actor in &mut copy.actors {
            if let Some(parent) = actor.parent()
                && let Some(next) = remap.get(parent)
            {
                actor.components.set_parent(next);
            }
            if let Some(joint) = actor.components.joint()
                && let Some(next) = remap.get(&joint.target)
            {
                let mut joint = joint.clone();
                joint.target = next.clone();
                actor.components.insert(ActorComponent::Joint { joint });
            }
            for component in actor.components.iter_mut() {
                if let ActorComponent::Constraint { constraint } = component
                    && let Some(next) = remap.get(&constraint.target)
                {
                    constraint.target = next.clone();
                }
            }
        }
        let new_id = copy.id.clone();
        self.scenes.push(copy);
        self.active_scene = new_id.clone();
        Ok(new_id)
    }

    pub fn rename_scene(&mut self, id: &str, name: &str) -> Result<String, String> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err("A scene needs a name".to_string());
        }
        if trimmed.contains('/') || trimmed.contains('\\') {
            return Err("A scene name can't contain a slash".to_string());
        }
        if self.scenes.iter().any(|s| s.id != id && s.name == trimmed) {
            return Err(format!("A scene named \"{trimmed}\" already exists"));
        }
        let current = self
            .scene(id)
            .map(|s| s.asset_path())
            .ok_or("Scene not found".to_string())?;
        // The file follows the name, keeping its folder.
        let parent = current
            .rfind('/')
            .map(|cut| current[..cut].to_string())
            .unwrap_or_default();
        let stem: String = trimmed
            .chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                    || c.is_control()
                {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let wanted = if parent.is_empty() {
            format!("{stem}.{SCENE_EXTENSION}")
        } else {
            format!("{parent}/{stem}.{SCENE_EXTENSION}")
        };
        // Names stay unique project-wide, so paths only clash on
        // sanitization; a tail keeps the file intact then.
        let mut candidate = wanted.clone();
        let mut n = 2;
        while self
            .scenes
            .iter()
            .any(|s| s.id != id && s.asset_path() == candidate)
        {
            candidate = if parent.is_empty() {
                format!("{stem} {n}.{SCENE_EXTENSION}")
            } else {
                format!("{parent}/{stem} {n}.{SCENE_EXTENSION}")
            };
            n += 1;
        }
        let Some(scene) = self.scene_mut(id) else {
            return Err("Scene not found".to_string());
        };
        scene.name = trimmed.clone();
        if candidate != current {
            scene.path = candidate;
        }
        Ok(trimmed)
    }

    /// Renames a scene to its file's stem: what the tray's rename means.
    /// Answers the new name.
    pub fn rename_scene_to_stem(&mut self, id: &str, stem: &str) -> Result<String, String> {
        let trimmed = stem.trim().to_string();
        if trimmed.is_empty() {
            return Err("A scene needs a name".to_string());
        }
        let unique = if self.scenes.iter().any(|s| s.id != id && s.name == trimmed) {
            self.unique_scene_name(&trimmed)
        } else {
            trimmed
        };
        self.rename_scene(id, &unique)
    }

    /// Removes a scene. The last one stays; the active one falls back to the
    /// first remaining scene, as does the default.
    pub fn remove_scene(&mut self, id: &str) -> Result<(), String> {
        if self.scenes.len() <= 1 {
            return Err("A project needs at least one scene".to_string());
        }
        let Some(index) = self.scenes.iter().position(|s| s.id == id) else {
            return Err("Scene not found".to_string());
        };
        self.scenes.remove(index);
        if self.active_scene == id {
            self.active_scene = self.scenes[0].id.clone();
        }
        if self.default_scene == id {
            self.default_scene = self.active_scene.clone();
        }
        Ok(())
    }

    /// Makes `id` the edited scene. Actor selection is per scene in the
    /// editor, so switching scenes clears it there.
    pub fn set_active_scene(&mut self, id: &str) -> Result<(), String> {
        if !self.scenes.iter().any(|s| s.id == id) {
            return Err("Scene not found".to_string());
        }
        self.active_scene = id.to_string();
        Ok(())
    }

    /// Finds an actor in any scene, active first. The runtime only runs the
    /// active scene, but project-wide renames need every scene.
    pub fn find_actor(&self, id: &str) -> Option<&Actor> {
        if let Some(actor) = self.active_scene_ref().actor(id) {
            return Some(actor);
        }
        self.scenes.iter().filter_map(|s| s.actor(id)).next()
    }

    /// Declares a project-wide variable starting at `0`.
    pub fn create_global(&mut self, name: &str) -> Result<String, String> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err("Variable name can't be empty".to_string());
        }
        // `~` opens a runtime slot (`~t0` holds a reporter's value while it
        // suspends), so no project variable may start with one.
        if trimmed.starts_with('~') {
            return Err("Variable name can't start with \"~\"".to_string());
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
        if trimmed.starts_with('~') {
            return Err("Variable name can't start with \"~\"".to_string());
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
        for scene in &mut self.scenes {
            for actor in &mut scene.actors {
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
        if let Some(actor) = self.find_actor(actor_id) {
            env.extend(actor.graph.variable_values());
        }
        env
    }

    /// True if `name` is a global rather than one of `actor_id`'s own - which
    /// list a `set`/`change` block writes back to.
    pub fn is_global(&self, actor_id: &str, name: &str) -> bool {
        let actor_owns = self
            .find_actor(actor_id)
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
        for scene in &mut self.scenes {
            for actor in &mut scene.actors {
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
                    blockstitch_core::graph::rename_list_in_value(
                        &mut floating.value,
                        old,
                        &trimmed,
                    );
                }
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
            .find_actor(actor_id)
            .is_some_and(|actor| actor.graph.lists.iter().any(|list| list.name == name));
        !actor_owns && self.global_lists.iter().any(|list| list.name == name)
    }

    /// Declares a project-wide dict starting empty.
    pub fn create_global_dict(&mut self, name: &str) -> Result<String, String> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err("Dict name can't be empty".to_string());
        }
        if self.global_dicts.iter().any(|dict| dict.name == trimmed) {
            return Err(format!("A dict named \"{trimmed}\" already exists"));
        }
        self.global_dicts.push(DictDef {
            name: trimmed.clone(),
            entries: Vec::new(),
            editor_visible: false,
            editor_x: 0,
            editor_y: 0,
        });
        Ok(trimmed)
    }

    /// Renames a project-wide dict and every read of it, in every actor that
    /// doesn't shadow it with its own dict of that name.
    pub fn rename_global_dict(&mut self, old: &str, new: &str) -> Result<String, String> {
        let trimmed = new.trim().to_string();
        if trimmed.is_empty() {
            return Err("Dict name can't be empty".to_string());
        }
        if trimmed != old && self.global_dicts.iter().any(|dict| dict.name == trimmed) {
            return Err(format!("A dict named \"{trimmed}\" already exists"));
        }
        let Some(dict) = self.global_dicts.iter_mut().find(|dict| dict.name == old) else {
            return Err("Dict not found".to_string());
        };
        if trimmed == old {
            return Ok(trimmed);
        }
        dict.name = trimmed.clone();
        for scene in &mut self.scenes {
            for actor in &mut scene.actors {
                // An actor with its own dict of that name reads its own, so its
                // references must stay put.
                if actor.graph.dicts.iter().any(|dict| dict.name == old) {
                    continue;
                }
                for strand in &mut actor.graph.strands {
                    for instruction in &mut strand.instructions {
                        instruction.rename_dict(old, &trimmed);
                    }
                }
                for floating in &mut actor.graph.floating_values {
                    blockstitch_core::graph::rename_dict_in_value(
                        &mut floating.value,
                        old,
                        &trimmed,
                    );
                }
            }
        }
        Ok(trimmed)
    }

    /// Drops a project-wide dict. Reads of it are left alone and default to
    /// empty, the same as an actor's own removed dict.
    pub fn remove_global_dict(&mut self, name: &str) {
        self.global_dicts.retain(|dict| dict.name != name);
    }

    /// True if `name` is a shared dict rather than one of `actor_id`'s own.
    pub fn is_global_dict(&self, actor_id: &str, name: &str) -> bool {
        let actor_owns = self
            .find_actor(actor_id)
            .is_some_and(|actor| actor.graph.dicts.iter().any(|dict| dict.name == name));
        !actor_owns && self.global_dicts.iter().any(|dict| dict.name == name)
    }

    /// Repairs and canonicalizes a just-loaded document, once. Runs per
    /// scene since each scene has its own world and actors.
    pub fn normalize(&mut self) {
        self.ensure_scene_invariants();
        self.migrate_sky();
        for scene in &mut self.scenes {
            scene.normalize_scene();
            scene.normalize_physics_ids();
        }
    }

    /// An HDR sky used to be a path on the lighting. It is an HDRI sky now,
    /// at the same brightness. Runs per scene since each scene has its own
    /// world settings.
    fn migrate_sky(&mut self) {
        for scene in &mut self.scenes {
            let world = &mut scene.world;
            let path = std::mem::take(&mut world.lighting.sky);
            if !path.is_empty() {
                let brightness = world.lighting.sky_brightness;
                world.sky.kind = crate::sky::SkyKind::Hdri;
                world.sky.hdri.path = path;
                world.sky.hdri.brightness = brightness;
            }
            world.sky.normalize();
            world.fog.normalize();
            world.clouds.normalize();
            crate::cloud_layers::normalize(&mut world.cloud_layers);
            world.lightning.normalize();
            world.wind.normalize();
            world.director.normalize();
            for cutscene in &mut world.cutscenes {
                cutscene.normalize();
            }
            if world.cutscenes.len() > 32 {
                world.cutscenes.truncate(32);
            }
            world.surface.normalize();
            world.vfx.normalize();
            world.quality.normalize();
        }
    }

    /// Declares a named input action with no bindings. Input lives per
    /// scene (each scene has its own `World`), so a new action is added to
    /// every scene to keep them in sync.
    pub fn create_input_action(&mut self, name: &str) -> Result<String, String> {
        let renamed = self.active_scene_mut().world.input.add_action(name)?;
        for scene in &mut self.scenes {
            if scene.world.input.find(&renamed).is_none() {
                let _ = scene.world.input.add_action(&renamed);
            }
        }
        Ok(renamed)
    }

    /// Renames an input action and every block that names it: the
    /// `when action pressed` header, the `bind`/`clear` slots holding plain
    /// text, and the action reporter args. Renames in every scene.
    pub fn rename_input_action(&mut self, old: &str, new: &str) -> Result<String, String> {
        let renamed = self
            .active_scene_mut()
            .world
            .input
            .rename_action(old, new)?;
        for scene in &mut self.scenes {
            let _ = scene.world.input.rename_action(old, &renamed);
        }
        for scene in &mut self.scenes {
            for actor in &mut scene.actors {
                actor.graph.walk_instructions_mut(&mut |ins| {
                    match &mut ins.kind {
                        InstructionKind::WhenActionPressed { action }
                            if action.eq_ignore_ascii_case(old) =>
                        {
                            *action = renamed.clone();
                        }
                        InstructionKind::BindAction { action, .. } => {
                            if let crate::value::Value::Text { value: text } = action
                                && text.eq_ignore_ascii_case(old)
                            {
                                *text = renamed.clone();
                            }
                        }
                        InstructionKind::ClearActionBindings { action } => {
                            if let crate::value::Value::Text { value: text } = action
                                && text.eq_ignore_ascii_case(old)
                            {
                                *text = renamed.clone();
                            }
                        }
                        _ => {}
                    }
                    ins.kind.visit_values_mut(&mut |value, _| {
                        rename_action_in_value(value, old, &renamed);
                    });
                });
                for floating in &mut actor.graph.floating_values {
                    rename_action_in_value(&mut floating.value, old, &renamed);
                }
            }
        }
        Ok(renamed)
    }

    /// Drops an input action. Blocks naming it are left alone and read as
    /// unheld, the same as an unknown key. Removes from every scene.
    pub fn remove_input_action(&mut self, name: &str) -> bool {
        let mut removed = false;
        for scene in &mut self.scenes {
            removed |= scene.world.input.remove_action(name);
        }
        removed
    }

    /// Refreshes shared lighting while preserving inline fallback for missing assets.
    pub fn resolve_lighting_assets(&mut self, dir: &Path) {
        for scene in &mut self.scenes {
            let path = scene.world.lighting.asset.clone();
            if !path.is_empty()
                && let Ok(mut lighting) = crate::assets::read_lighting(dir, &path)
            {
                lighting.asset = path;
                scene.world.lighting = lighting;
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
        for scene in &mut self.scenes {
            let world = &mut scene.world;
            repoint(&mut world.lighting.asset);
            for style in world.interface.styles.values_mut() {
                for paint in [
                    &mut style.normal,
                    &mut style.hover,
                    &mut style.pressed,
                    &mut style.disabled,
                    &mut style.focused,
                ] {
                    for font in &mut paint.fonts {
                        repoint(font);
                    }
                }
            }
            for path in &mut world.interface.stylesheets {
                repoint(path);
            }
            let (widgets, prefabs) = (&mut world.interface.widgets, &mut world.interface.prefabs);
            for widget in widgets.iter_mut().chain(prefabs.values_mut().flatten()) {
                if widget.element.kind == crate::ui::UiKind::Image {
                    repoint(&mut widget.element.content);
                }
                for paint in [
                    &mut widget.style.normal,
                    &mut widget.style.hover,
                    &mut widget.style.pressed,
                    &mut widget.style.disabled,
                    &mut widget.style.focused,
                ] {
                    for font in &mut paint.fonts {
                        repoint(font);
                    }
                }
            }
            repoint(&mut world.sky.hdri.path);
            repoint(&mut world.sky.stars.milky_way);
            repoint(&mut world.lightning.thunder_sound);
            repoint(&mut world.clouds.shape_volume);
            repoint(&mut world.clouds.detail_volume);
            for layer in &mut world.cloud_layers {
                repoint(&mut layer.coverage_texture);
                repoint(&mut layer.flow_map);
            }
            if let Some(font) = world.speech_bubble.font_asset.as_mut() {
                repoint(font);
            }
            for actor in &mut scene.actors {
                for component in actor.components.iter_mut() {
                    match component {
                        ActorComponent::Look {
                            visual: Visual::Image { path, .. } | Visual::Model { path, .. },
                        } => repoint(path),
                        ActorComponent::Look {
                            visual: Visual::Tilemap { tilemap },
                        } => repoint(&mut tilemap.tileset),
                        ActorComponent::Material { material } => {
                            repoint(&mut material.albedo_texture);
                            repoint(&mut material.normal_texture);
                            repoint(&mut material.roughness_texture);
                        }
                        ActorComponent::Fracture { fracture } => {
                            repoint(&mut fracture.bounce_sound);
                            repoint(&mut fracture.interior.albedo_texture);
                            repoint(&mut fracture.interior.normal_texture);
                            repoint(&mut fracture.interior.roughness_texture);
                        }
                        ActorComponent::Script { path } => repoint(path),
                        _ => {}
                    }
                }
            }
        }
        changed
    }
}

/// Renames an action where a value tree means one: the first arg of the
/// four action reporters. Anything else - a bare text slot, a `say` arg -
/// is left alone, so a binding that happens to share the spelling keeps it.
fn rename_action_in_value(value: &mut crate::value::Value, old: &str, new: &str) {
    match value {
        crate::value::Value::Op { op, args, saved } => {
            let is_action = matches!(
                op.name(),
                "ActionDown" | "ActionPressed" | "ActionReleased" | "ActionValue"
            );
            for (index, arg) in args.iter_mut().enumerate() {
                if is_action
                    && index == 0
                    && let crate::value::Value::Text { value: text } = arg
                    && text.eq_ignore_ascii_case(old)
                {
                    *text = new.to_string();
                } else {
                    rename_action_in_value(arg, old, new);
                }
            }
            rename_action_in_value(saved, old, new);
        }
        crate::value::Value::Call { args, saved, .. } => {
            for arg in args.iter_mut() {
                rename_action_in_value(arg, old, new);
            }
            rename_action_in_value(saved, old, new);
        }
        _ => {}
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

pub fn visual_for_mode(visual: &Visual, mode: Mode) -> Visual {
    use crate::scene::PIXELS_PER_METRE;

    if visual.is_3d() == mode.is_3d() {
        return visual.clone();
    }
    // A tilemap renders in both dimensions, so it never converts.
    if let Visual::Tilemap { .. } = visual {
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
        // A model in 2D reads as its footprint box; a 2D visual never
        // becomes a model on its own since there is no file to name.
        Visual::Model { tint, scale, .. } => Visual::Rect {
            color: tint.clone(),
            size: [scale[0] * PIXELS_PER_METRE, scale[1] * PIXELS_PER_METRE],
        },
        // Reached only when a hand-edited document disagrees with itself;
        // the early return above keeps every real tilemap as it is.
        Visual::Tilemap { tilemap } => Visual::Tilemap {
            tilemap: tilemap.clone(),
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

/// Where the Export dialog points: `~/Blockloom/exports`, so a `.blockloom`
/// file doesn't land among the project folders.
pub fn default_exports_dir() -> PathBuf {
    sibling_dir("exports")
}

/// Where the Build dialog puts games: `~/Blockloom/builds`, for the same
/// reason - build output stays out of the project library.
pub fn default_builds_dir() -> PathBuf {
    sibling_dir("builds")
}

fn sibling_dir(name: &str) -> PathBuf {
    let projects = default_projects_dir();
    match projects.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
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

/// Reads one project file: a full embedded document, as Export writes and
/// old project folders held. New project folders hold an index instead -
/// see [`read_project_dir`].
pub fn read_project(path: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut project: Project =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    project.normalize();
    Ok(project)
}

/// Reads the project a folder holds: its index plus one scene asset per
/// entry. Folders written before scene assets carry embedded scenes instead
/// and still load - the next save rewrites them as assets.
pub fn read_project_dir(dir: &Path) -> Result<Project, String> {
    let path = project_file(dir);
    if !path.is_file() {
        return Err(format!(
            "{} isn't a Blockloom project folder",
            dir.display()
        ));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let is_index = value
        .get("scenes")
        .and_then(|scenes| scenes.as_array())
        .is_some_and(|scenes| scenes.is_empty() || scenes.iter().all(|s| s.get("path").is_some()));
    if !is_index {
        // Old folder: embedded scenes (or a single scene). Load it whole;
        // the next save splits it into assets.
        let mut project: Project =
            serde_json::from_value(value).map_err(|e| format!("{}: {e}", path.display()))?;
        project.normalize();
        project.resolve_lighting_assets(dir);
        return Ok(project);
    }
    let file: ProjectFile =
        serde_json::from_value(value).map_err(|e| format!("{}: {e}", path.display()))?;
    file.physics
        .check_version()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut scenes = Vec::with_capacity(file.scenes.len());
    for scene_ref in &file.scenes {
        let mut relative = if scene_ref.path.is_empty() {
            scene_asset_path(&scene_ref.id)
        } else {
            scene_ref.path.clone()
        };
        let mut full = dir.join(&relative);
        if !full.is_file() {
            // The file moved outside the editor - a tray rename in the OS
            // file manager, say. Follow the scene id to its new home rather
            // than failing the open.
            if let Some(found) = find_scene_file(dir, &scene_ref.id) {
                relative = found;
                full = dir.join(&relative);
            }
        }
        let mut scene = read_scene_file(&full)
            .map_err(|e| format!("scene \"{}\" ({}): {e}", scene_ref.name, full.display()))?;
        // The index wins on identity; the file wins on content. The tray
        // filename is the scene name, so a file renamed outside the editor
        // renames its scene - except legacy id-named files, whose stem is
        // the id rather than a title.
        scene.id = scene_ref.id.clone();
        scene.path = relative.clone();
        let stem = scene_name_for_path(&relative);
        if !stem.is_empty() && stem != scene_ref.id {
            if scene_ref.name.trim().is_empty() || stem != scene_ref.name {
                // Prefer the filename when it disagrees with the index: it
                // is what the tray shows. The index catches up on next save.
                scene.name = stem;
            } else {
                scene.name = scene_ref.name.clone();
            }
        } else if !scene_ref.name.trim().is_empty() {
            scene.name = scene_ref.name.clone();
        }
        scenes.push(scene);
    }
    let mut project = Project {
        id: if file.id.is_empty() {
            new_id()
        } else {
            file.id
        },
        name: file.name,
        icon: file.icon,
        android: file.android,
        scenes,
        active_scene: file.active_scene,
        default_scene: file.default_scene,
        globals: file.globals,
        global_lists: file.global_lists,
        global_dicts: file.global_dicts,
        plugin_resources: file.plugin_resources,
        physics: file.physics,
        multiplayer: file.multiplayer,
        localization: file.localization,
    };
    project.normalize();
    project.resolve_lighting_assets(dir);
    Ok(project)
}

/// The index over `project`'s scene assets.
pub fn project_to_file(project: &Project) -> ProjectFile {
    ProjectFile {
        id: project.id.clone(),
        name: project.name.clone(),
        icon: project.icon.clone(),
        android: project.android.clone(),
        active_scene: project.active_scene.clone(),
        default_scene: project.default_scene.clone(),
        scenes: project
            .scenes
            .iter()
            .map(|scene| SceneRef {
                id: scene.id.clone(),
                name: scene.name.clone(),
                path: scene.asset_path(),
                mode: scene.world.mode,
            })
            .collect(),
        globals: project.globals.clone(),
        global_lists: project.global_lists.clone(),
        global_dicts: project.global_dicts.clone(),
        plugin_resources: project.plugin_resources.clone(),
        physics: project.physics.clone(),
        multiplayer: project.multiplayer.clone(),
        localization: project.localization.clone(),
    }
}

/// Writes `project` into its folder as an index plus one scene asset per
/// scene, making the folders as needed. Answers the new revision: every save
/// bumps the folder's counter, so an idle backend can tell its in-memory
/// copy went stale. Export stays a single embedded file - see
/// [`export_project`].
pub fn save_project(project: &Project, dir: &Path) -> Result<u64, String> {
    std::fs::create_dir_all(assets_dir(dir)).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::create_dir_all(scenes_dir(dir)).map_err(|e| format!("{}: {e}", dir.display()))?;
    for scene in &project.scenes {
        save_scene_file(dir, scene)?;
    }
    // Renames move their file in the command that asked (see
    // `commands::rename_scene`); anything else under the scenes folder is
    // left alone, so a staged import never vanishes on save.
    let path = project_file(dir);
    let json =
        serde_json::to_string_pretty(&project_to_file(project)).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(crate::sync::bump_revision(dir))
}

/// Writes a project to an arbitrary path, for "Export".
pub fn export_project(project: &Project, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
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
    fn plugin_records_of_a_missing_plugin_survive_open_and_save() {
        let temp = TempDir::new();
        let mut project = Project::starter("Plugged", Mode::TwoD);
        let ball = Visual::Circle {
            color: "#ffffff".into(),
            radius: 10.0,
        };
        let id = project.add_actor(Actor::new("Ball", ball));
        let payload = serde_json::json!({"hp": 7, "future_field": [1, 2]});
        project
            .actor_mut(&id)
            .unwrap()
            .components
            .insert(ActorComponent::Plugin {
                record: PluginRecord::new("com.example.gone", "Health", 3, payload.clone()),
            });
        project.set_plugin_resource(PluginRecord::new(
            "com.example.gone",
            "Rules",
            1,
            serde_json::json!({"x": "y"}),
        ));
        let dir = create_project(&project, &temp.0).unwrap();
        let first = read_project_dir(&dir).unwrap();
        save_project(&first, &dir).unwrap();
        let second = read_project_dir(&dir).unwrap();
        let record = second
            .actor(&id)
            .unwrap()
            .components
            .plugin_record("com.example.gone/Health")
            .unwrap();
        assert_eq!((record.schema_version, &record.payload), (3, &payload));
        assert_eq!(second.plugin_resources.len(), 1);
        assert_eq!(second.plugin_ids().len(), 1);
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
    fn android_settings_survive_a_save_and_old_files_default() {
        let temp = TempDir::new();
        let mut project = Project::starter("Pond Game", Mode::TwoD);
        project.android = crate::android::AndroidSettings {
            application_id: "com.example.pond".to_string(),
            version_code: 7,
            version_name: "2.1".to_string(),
            keystore: "/keys/release.keystore".to_string(),
            key_alias: "upload".to_string(),
        };
        let dir = create_project(&project, &temp.0).unwrap();
        save_project(&project, &dir).unwrap();
        assert_eq!(read_project_dir(&dir).unwrap(), project);

        // A file from before the settings existed reads as the defaults.
        let text = std::fs::read_to_string(project_file(&dir)).unwrap();
        let mut index: serde_json::Value = serde_json::from_str(&text).unwrap();
        index.as_object_mut().unwrap().remove("android");
        std::fs::write(project_file(&dir), serde_json::to_string(&index).unwrap()).unwrap();
        let loaded = read_project_dir(&dir).unwrap();
        assert_eq!(loaded.android, crate::android::AndroidSettings::default());
        assert_eq!(
            loaded.android.application_id_for(&loaded.name).unwrap(),
            "com.blockloom.game.pond_game"
        );

        // A file from before release signing reads empty key rows.
        let mut index: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(project_file(&dir)).unwrap()).unwrap();
        index.as_object_mut().unwrap().insert(
            "android".to_string(),
            serde_json::json!({"application_id": "", "version_code": 1, "version_name": "1.0.0"}),
        );
        std::fs::write(project_file(&dir), serde_json::to_string(&index).unwrap()).unwrap();
        let loaded = read_project_dir(&dir).unwrap();
        assert_eq!(loaded.android.keystore, "");
        assert_eq!(loaded.android.key_alias, "");
    }

    #[test]
    fn scenes_save_as_one_asset_each_plus_an_index() {
        let temp = TempDir::new();
        let mut project = Project::starter("Scenes", Mode::TwoD);
        project.add_scene("Level 2", None);
        let dir = create_project(&project, &temp.0).unwrap();

        // One file per scene, under assets/scenes.
        for scene in &project.scenes {
            let full = dir.join(scene.asset_path());
            assert!(full.is_file(), "{}", full.display());
            let file: SceneFile =
                serde_json::from_str(&std::fs::read_to_string(&full).unwrap()).unwrap();
            assert_eq!(file.id, scene.id);
            assert!(file.components.is_some());
            assert!(file.world.is_none());
        }
        // The project file is an index, not embedded scenes.
        let index: ProjectFile =
            serde_json::from_str(&std::fs::read_to_string(project_file(&dir)).unwrap()).unwrap();
        assert_eq!(index.scenes.len(), 2);
        assert!(index.scenes.iter().all(|s| !s.path.is_empty()));
        assert_eq!(read_project_dir(&dir).unwrap(), project);
    }

    #[test]
    fn an_old_folder_with_embedded_scenes_still_loads() {
        let temp = TempDir::new();
        let project = Project::starter("Old", Mode::TwoD);
        let dir = create_project(&project, &temp.0).unwrap();
        // Rewrite the index as a full embedded document, the pre-asset shape.
        let json = serde_json::to_string_pretty(&project).unwrap();
        std::fs::write(project_file(&dir), json).unwrap();

        let loaded = read_project_dir(&dir).unwrap();
        assert_eq!(loaded.scenes.len(), 1);
        assert_eq!(loaded.actors.len(), 2);
        // The next save splits it into assets.
        save_project(&loaded, &dir).unwrap();
        let index: ProjectFile =
            serde_json::from_str(&std::fs::read_to_string(project_file(&dir)).unwrap()).unwrap();
        assert_eq!(index.scenes.len(), 1);
        assert!(dir.join(&index.scenes[0].path).is_file());
    }

    #[test]
    fn a_scene_file_is_named_for_its_scene_and_renames_with_it() {
        let mut project = Project::starter("Scenes", Mode::TwoD);
        assert_eq!(
            project.active_scene().asset_path(),
            "assets/scenes/Scene 1.blockscene"
        );
        let id = project.add_scene("Level 2", None);
        assert_eq!(
            project.scene(&id).unwrap().asset_path(),
            "assets/scenes/Level 2.blockscene"
        );
        // Renaming the scene moves its file, keeping the folder.
        project.rename_scene(&id, "Boss").unwrap();
        assert_eq!(
            project.scene(&id).unwrap().asset_path(),
            "assets/scenes/Boss.blockscene"
        );
        // A tray rename is the same thing by stem.
        project.rename_scene_to_stem(&id, "Final Boss").unwrap();
        assert_eq!(project.scene(&id).unwrap().name, "Final Boss");
        assert_eq!(
            project.scene(&id).unwrap().asset_path(),
            "assets/scenes/Final Boss.blockscene"
        );
        // Slashes never reach the file.
        assert!(project.rename_scene(&id, "a/b").is_err());
    }

    #[test]
    fn any_blockscene_file_reads_as_a_scene_asset() {
        assert!(is_scene_asset("assets/scenes/Level 1.blockscene"));
        assert!(is_scene_asset("stages/Boss.BLOCKSCENE"));
        assert!(!is_scene_asset("assets/sprites/player.png"));
        assert!(!is_scene_asset("assets/scenes"));
        assert_eq!(
            scene_name_for_path("assets/scenes/Level 1.blockscene"),
            "Level 1"
        );
        assert_eq!(
            scene_path_for_name("Level 1"),
            "assets/scenes/Level 1.blockscene"
        );
    }

    #[test]
    fn the_default_scene_round_trips_and_boots() {
        let temp = TempDir::new();
        let mut project = Project::starter("Scenes", Mode::TwoD);
        let second = project.add_scene("Level 2", None);
        project.set_default_scene(&second).unwrap();
        assert_eq!(project.boot_scene_id(), second);
        let dir = create_project(&project, &temp.0).unwrap();
        let loaded = read_project_dir(&dir).unwrap();
        assert_eq!(loaded.default_scene, second);
        // The file the scene lives at is the filename, not the id.
        let index: ProjectFile =
            serde_json::from_str(&std::fs::read_to_string(project_file(&dir)).unwrap()).unwrap();
        let entry = index.scenes.iter().find(|s| s.id == second).unwrap();
        assert_eq!(entry.path, "assets/scenes/Level 2.blockscene");
    }

    #[test]
    fn persist_marks_its_actor_as_a_scene_survivor() {
        let mut project = Project::starter("Scenes", Mode::TwoD);
        let id = project.actors[0].id.clone();
        assert!(!project.actors[0].persists());
        project.actors[0].components.insert(ActorComponent::Persist);
        assert!(project.actors[0].persists());
        // It round-trips through the scene file like any other component.
        let temp = TempDir::new();
        let dir = create_project(&project, &temp.0).unwrap();
        let loaded = read_project_dir(&dir).unwrap();
        assert!(
            loaded
                .scene(&loaded.active_scene)
                .unwrap()
                .actor(&id)
                .unwrap()
                .persists()
        );
        // And detaching drops it again.
        let mut project = loaded;
        let scene_id = project.active_scene.clone();
        project
            .scene_mut(&scene_id)
            .unwrap()
            .actor_mut(&id)
            .unwrap()
            .components
            .remove("Persist");
        assert!(!project.active_scene().actor(&id).unwrap().persists());
    }

    #[test]
    fn visuals_convert_across_dimensions_for_carried_survivors() {
        use crate::scene::Visual;
        // 2D square becomes a 3D cuboid; 3D sphere becomes a 2D ball.
        let square = Visual::Rect {
            color: "#FFFFFF".to_string(),
            size: [60.0, 60.0],
        };
        let cuboid = visual_for_mode(&square, Mode::ThreeD);
        assert!(cuboid.is_3d());
        let ball = Visual::Sphere {
            color: "#FFFFFF".to_string(),
            radius: 0.5,
        };
        let circle = visual_for_mode(&ball, Mode::TwoD);
        assert!(!circle.is_3d());
        // A tilemap renders in both, so it never converts.
        let map = Visual::Tilemap {
            tilemap: crate::material::Tilemap::default(),
        };
        assert_eq!(visual_for_mode(&map, Mode::ThreeD), map);
    }

    #[test]
    fn a_scene_file_moved_outside_the_editor_is_followed() {
        let temp = TempDir::new();
        let project = Project::starter("Scenes", Mode::TwoD);
        let dir = create_project(&project, &temp.0).unwrap();
        let id = project.scenes[0].id.clone();
        // Rename the file the way an OS file manager would.
        std::fs::rename(
            dir.join("assets/scenes/Scene 1.blockscene"),
            dir.join("assets/scenes/Opened.blockscene"),
        )
        .unwrap();
        let loaded = read_project_dir(&dir).unwrap();
        let scene = loaded.scene(&id).unwrap();
        assert_eq!(scene.name, "Opened");
        assert_eq!(scene.asset_path(), "assets/scenes/Opened.blockscene");
    }

    #[test]
    fn a_scene_with_a_fixed_world_loads_through_its_asset() {
        let mut scene = Scene::new("Legacy", Mode::TwoD);
        scene.world.background = "#102030".to_string();
        let file = SceneFile {
            id: scene.id.clone(),
            name: scene.name.clone(),
            format: SCENE_FORMAT,
            components: None,
            world: Some(scene.world.clone()),
            actors: scene.actors.clone(),
        };
        assert_eq!(Scene::from_file(file).world.background, "#102030");
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
    fn switching_dimensions_converts_every_length_an_actor_carries() {
        let mut project = Project::starter("Untitled", Mode::TwoD);
        project.world.gravity = [0.0, -500.0, 0.0];
        let id = project.actors[0].id.clone();
        let other = project.actors[1].id.clone();
        let actor = &mut project.actors[0];
        actor.components.insert(ActorComponent::Parent {
            parent: other,
            offset: Some([200.0, 100.0, 5.0]),
        });
        actor.components.insert(ActorComponent::Joint {
            joint: crate::components::JointSpec {
                anchor: [50.0, 0.0, 0.0],
                length: 300.0,
                ..Default::default()
            },
        });

        project.switch_mode(Mode::ThreeD);
        assert!((project.world.gravity[1] + 5.0).abs() < 1e-4);
        let actor = project.actor(&id).unwrap();
        let [x, y, z] = actor.parent_offset().unwrap();
        assert!((x - 2.0).abs() < 1e-4 && (y - 1.0).abs() < 1e-4 && (z - 0.05).abs() < 1e-4);
        let joint = actor.components.joint().unwrap();
        assert!((joint.anchor[0] - 0.5).abs() < 1e-4);
        assert!((joint.length - 3.0).abs() < 1e-4);

        project.switch_mode(Mode::TwoD);
        let actor = project.actor(&id).unwrap();
        // A 2D z is only a draw order, so it doesn't come back.
        assert_eq!(actor.parent_offset().unwrap()[2], 0.0);
        assert!((actor.parent_offset().unwrap()[0] - 200.0).abs() < 1e-2);
        assert!((actor.components.joint().unwrap().length - 300.0).abs() < 1e-2);
        assert!((project.world.gravity[1] + 500.0).abs() < 1e-2);
    }

    #[test]
    fn a_length_converts_by_a_hundred_between_pixels_and_metres() {
        assert_eq!(Mode::ThreeD.length_from(Mode::TwoD, 250.0), 2.5);
        assert_eq!(Mode::TwoD.length_from(Mode::ThreeD, 2.5), 250.0);
        assert_eq!(Mode::TwoD.length_from(Mode::TwoD, 7.0), 7.0);
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
    fn actor_references_in_brains_and_joints_follow_edits() {
        let mut project = Project::starter("p", Mode::TwoD);
        let target = project.actors[1].id.clone();
        project.actors[0].components.insert(ActorComponent::Brain {
            brain: crate::ai::BrainSpec {
                target: "Ground".to_string(),
                ..Default::default()
            },
        });
        project.actors[0].components.insert(ActorComponent::Joint {
            joint: crate::components::JointSpec {
                target: target.clone(),
                ..Default::default()
            },
        });
        project.rename_actor(&target, "Floor").unwrap();
        assert_eq!(
            project.actors[0].components.brain().unwrap().target,
            "Floor"
        );
        assert_eq!(project.actors[0].components.joint().unwrap().target, target);
        project.remove_actor(&target);
        assert!(project.actors[0].components.joint().is_none());
        assert_eq!(project.actors[0].components.brain().unwrap().target, "");
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
        actor.components.insert(ActorComponent::Material {
            material: crate::material::SurfaceMaterial {
                albedo_texture: "assets/sprites/player.png".to_string(),
                normal_texture: "assets/sprites/normal.png".to_string(),
                roughness_texture: "assets/sprites/rough.png".to_string(),
                ..Default::default()
            },
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
        let material = actor.components.material().unwrap();
        assert_eq!(material.albedo_texture, "art/hero.png");
        assert_eq!(material.normal_texture, "art/normal.png");
        assert_eq!(material.roughness_texture, "art/rough.png");
    }

    #[test]
    fn an_old_lighting_sky_becomes_an_hdri_sky() {
        let mut project = Project::starter("Sky", Mode::ThreeD);
        let mut json = serde_json::to_value(&project).unwrap();
        // New documents carry both `scenes` (canonical) and a compat `world`.
        // A legacy edit touches the scene copy; keep both in sync here.
        json["scenes"][0]["world"]["lighting"]["sky"] = "assets/sky.hdr".into();
        json["scenes"][0]["world"]["lighting"]["sky_brightness"] = 2500.0.into();
        json["world"]["lighting"]["sky"] = "assets/sky.hdr".into();
        json["world"]["lighting"]["sky_brightness"] = 2500.0.into();
        project = serde_json::from_value(json).unwrap();
        project.normalize();
        let sky = &project.world.sky;
        assert_eq!(sky.kind, crate::sky::SkyKind::Hdri);
        assert_eq!(sky.hdri.path, "assets/sky.hdr");
        assert_eq!(sky.hdri.brightness, 2500.0);
        assert!(project.world.lighting.sky.is_empty());
        // Written back, only the new place remains.
        let json = serde_json::to_value(&project).unwrap();
        assert!(json["world"]["lighting"].get("sky").is_none());
        assert_eq!(json["world"]["sky"]["hdri"]["path"], "assets/sky.hdr");
        // Old single-scene files without `scenes` still migrate as scene one.
        let mut old = serde_json::to_value(&project).unwrap();
        old.as_object_mut().unwrap().remove("scenes");
        old.as_object_mut().unwrap().remove("active_scene");
        let migrated: Project = serde_json::from_value(old).unwrap();
        assert_eq!(migrated.scenes.len(), 1);
        assert_eq!(migrated.scenes[0].name, "Scene 1");
    }

    #[test]
    fn scenes_hold_their_own_actors_world_and_mode() {
        let mut project = Project::starter("Scenes", Mode::TwoD);
        assert_eq!(project.scenes.len(), 1);
        assert_eq!(project.active_scene().name, "Scene 1");
        assert_eq!(project.world.mode, Mode::TwoD);
        assert_eq!(project.actors.len(), 2);

        // A new scene starts empty in the active scene's dimension.
        let second = project.add_scene("Level 2", None);
        assert_eq!(project.scenes.len(), 2);
        assert_eq!(project.active_scene, second);
        assert!(project.actors.is_empty());
        assert_eq!(project.world.mode, Mode::TwoD);

        // Mixed dimensions: the second scene converts on its own.
        project.switch_mode(Mode::ThreeD);
        assert_eq!(project.world.mode, Mode::ThreeD);
        project
            .set_active_scene(&project.scenes[0].id.clone())
            .unwrap();
        assert_eq!(project.world.mode, Mode::TwoD);

        // Rename refuses duplicates and blanks.
        let first = project.scenes[0].id.clone();
        assert!(project.rename_scene(&first, "Level 2").is_err());
        assert!(project.rename_scene(&first, "  ").is_err());
        assert_eq!(project.rename_scene(&first, "Menu").unwrap(), "Menu");

        // Duplicate copies actors with fresh ids and becomes active.
        let copy = project.duplicate_scene(&first).unwrap();
        assert_eq!(project.scenes.len(), 3);
        assert_eq!(project.active_scene, copy);
        assert_eq!(project.actors.len(), 2);
        assert_ne!(
            project.actors[0].id,
            project.scene(&first).unwrap().actors[0].id
        );

        // Removing the active scene falls back to the first one; the last stays.
        project.remove_scene(&copy).unwrap();
        assert_eq!(project.active_scene, project.scenes[0].id);
        let last = project
            .scenes
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        for id in &last[1..] {
            project.remove_scene(id).unwrap();
        }
        assert_eq!(project.scenes.len(), 1);
        assert!(project.remove_scene(&project.scenes[0].id.clone()).is_err());
    }

    #[test]
    fn scene_actor_names_stay_within_their_scene() {
        let mut project = Project::starter("Names", Mode::TwoD);
        let second = project.add_scene("Second", None);
        // Both scenes can hold a "Player": uniqueness is per scene.
        let id = project.add_actor(Actor::new("Player", default_visual()));
        assert_eq!(
            project.scene(&second).unwrap().actor(&id).unwrap().name,
            "Player"
        );
        let dup = project.add_actor(Actor::new("Player", default_visual()));
        assert_eq!(
            project.scene(&second).unwrap().actor(&dup).unwrap().name,
            "Player 2"
        );
        project
            .set_active_scene(&project.scenes[0].id.clone())
            .unwrap();
        assert!(project.actor(&id).is_none());
        assert!(project.find_actor(&id).is_some());
    }

    #[test]
    fn switching_scenes_keeps_globals_but_not_actors() {
        let mut project = Project::starter("Globals", Mode::TwoD);
        project.create_global("score").unwrap();
        let first_actor = project.actors[0].id.clone();
        project.add_scene("Other", None);
        // Globals cross scenes; actor locals do not.
        assert!(project.globals.iter().any(|v| v.name == "score"));
        assert!(project.find_actor(&first_actor).is_some());
        assert!(project.actor(&first_actor).is_none());
        assert!(project.is_global("nobody", "score"));
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

    #[test]
    fn shared_dicts_are_created_renamed_and_removed() {
        let mut project = Project::starter("p", Mode::TwoD);
        assert!(project.create_global_dict("  ").is_err());
        project.create_global_dict("save").unwrap();
        assert!(project.create_global_dict("save").is_err());

        let id = project.actors[0].id.clone();
        assert!(project.is_global_dict(&id, "save"));
        project.rename_global_dict("save", "slot").unwrap();
        assert!(project.is_global_dict(&id, "slot"));
        assert!(project.rename_global_dict("slot", "slot").is_ok());
        project.remove_global_dict("slot");
        assert!(!project.is_global_dict(&id, "slot"));
    }

    #[test]
    fn renaming_a_shared_dict_follows_reads_except_where_an_actor_shadows_it() {
        let mut project = Project::starter("p", Mode::TwoD);
        project.create_global_dict("save").unwrap();
        let shadowing = project.actors[1].id.clone();
        project
            .actor_mut(&shadowing)
            .unwrap()
            .graph
            .create_dict("save")
            .unwrap();
        for index in 0..2 {
            let sets = Instruction::new(InstructionKind::SetDictValue {
                key: crate::value::Value::text("hp"),
                value: crate::value::Value::number(1.0),
                name: "save".to_string(),
            });
            let reads = Instruction::new(InstructionKind::Say {
                text: crate::value::Value::Op {
                    op: crate::value::Op::from_name("DictValue"),
                    args: vec![
                        crate::value::Value::text("hp"),
                        crate::value::Value::text("save"),
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
                    vec![sets, reads],
                ));
        }
        project.rename_global_dict("save", "slot").unwrap();

        let target_of = |actor: &Actor| match &actor.graph.strands[0].instructions[0].kind {
            InstructionKind::SetDictValue { name, .. } => name.clone(),
            other => panic!("expected a set, got {other:?}"),
        };
        let read_of = |actor: &Actor| match &actor.graph.strands[0].instructions[1].kind {
            InstructionKind::Say {
                text: crate::value::Value::Op { args, .. },
            } => match &args[1] {
                crate::value::Value::Text { value } => value.clone(),
                other => panic!("expected a dict name, got {other:?}"),
            },
            other => panic!("expected a say, got {other:?}"),
        };
        assert_eq!(target_of(&project.actors[0]), "slot");
        assert_eq!(read_of(&project.actors[0]), "slot");
        assert_eq!(target_of(&project.actors[1]), "save");
        assert_eq!(read_of(&project.actors[1]), "save");
    }

    #[test]
    fn renaming_an_input_action_follows_the_blocks_that_name_it() {
        let mut project = Project::starter("p", Mode::TwoD);
        project.create_input_action("Dash").unwrap();
        project.actors[0]
            .graph
            .strands
            .push(crate::blocks::Strand::with_instructions(
                0,
                0,
                vec![
                    Instruction::new(InstructionKind::WhenActionPressed {
                        action: "Dash".to_string(),
                    }),
                    Instruction::new(InstructionKind::Say {
                        text: crate::value::Value::Op {
                            op: crate::value::Op::from_name("ActionDown"),
                            args: vec![crate::value::Value::text("Dash")],
                            saved: Box::new(crate::value::Value::number(0.0)),
                        },
                    }),
                    Instruction::new(InstructionKind::BindAction {
                        action: crate::value::Value::text("Dash"),
                        binding: crate::value::Value::text("shift"),
                    }),
                ],
            ));
        project.rename_input_action("dash", "Sprint").unwrap();

        let strand = &project.actors[0].graph.strands[0].instructions;
        let InstructionKind::WhenActionPressed { action } = &strand[0].kind else {
            panic!("expected an action header");
        };
        assert_eq!(action, "Sprint");
        let InstructionKind::Say { text } = &strand[1].kind else {
            panic!("expected a say");
        };
        let crate::value::Value::Op { args, .. } = text else {
            panic!("expected a reporter");
        };
        assert!(matches!(
            &args[0],
            crate::value::Value::Text { value } if value == "Sprint"
        ));
        let InstructionKind::BindAction { action, binding } = &strand[2].kind else {
            panic!("expected a bind");
        };
        assert!(matches!(
            action,
            crate::value::Value::Text { value } if value == "Sprint"
        ));
        // The binding slot keeps its own spelling: it names a key, not the action.
        assert!(matches!(
            binding,
            crate::value::Value::Text { value } if value == "shift"
        ));
    }
}
