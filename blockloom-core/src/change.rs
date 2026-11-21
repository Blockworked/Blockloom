//! What one edit did to a project, as the parts it replaced: the index
//! fields, a scene's settings, single actors, and the order of scenes and
//! actors. A [`Change`] holds each part before and after, so it is its own
//! inverse: undo applies it backwards, redo forwards, saving writes only the
//! files it touched, and the frontend gets only the actors it names.
//!
//! [`diff`] derives one from two documents, so no command has to describe
//! its own edit.

use crate::project::{Actor, Project, Scene};
use std::collections::{HashMap, HashSet};

/// One part before and after an edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Swap<T> {
    pub before: T,
    pub after: T,
}

impl<T> Swap<T> {
    fn side(&self, forward: bool) -> &T {
        if forward { &self.after } else { &self.before }
    }
}

/// One actor, keyed by id. `None` on a side means it wasn't there.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorChange {
    pub id: String,
    pub before: Option<Box<Actor>>,
    pub after: Option<Box<Actor>>,
}

/// Everything that changed in one scene.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneChange {
    pub id: String,
    /// The scene without its actors. `None` on a side: the scene was added
    /// (before) or removed (after).
    pub settings: Option<Swap<Option<Box<Scene>>>>,
    /// Actor ids in order, when the order or the membership changed.
    pub order: Option<Swap<Vec<String>>>,
    pub actors: Vec<ActorChange>,
    /// Every actor at once, for a scene whose actor ids aren't unique.
    pub all_actors: Option<Swap<Vec<Actor>>>,
}

impl SceneChange {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            ..Self::default()
        }
    }

    fn is_empty(&self) -> bool {
        self.settings.is_none()
            && self.order.is_none()
            && self.actors.is_empty()
            && self.all_actors.is_none()
    }
}

/// What one edit (or one coalesced run of them) changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Change {
    /// The project without its scenes.
    pub header: Option<Swap<Box<Project>>>,
    /// Scene ids in order, when the order or the membership changed.
    pub scene_order: Option<Swap<Vec<String>>>,
    pub scenes: Vec<SceneChange>,
    /// The whole document, for one whose scene ids aren't unique.
    pub whole: Option<Swap<Box<Project>>>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.header.is_none()
            && self.scene_order.is_none()
            && self.scenes.is_empty()
            && self.whole.is_none()
    }

    /// Whether `project.blockloom` (the index) needs writing: the header, the
    /// scene list, or a scene's name, path or mode moved.
    pub fn touches_index(&self) -> bool {
        self.whole.is_some()
            || self.header.is_some()
            || self.scene_order.is_some()
            || self.scenes.iter().any(|s| s.settings.is_some())
    }

    /// Scenes whose files need writing, by id. `None` means all of them.
    pub fn touched_scenes(&self) -> Option<Vec<String>> {
        if self.whole.is_some() {
            return None;
        }
        Some(self.scenes.iter().map(|s| s.id.clone()).collect())
    }

    /// Actor ids this change replaced, added or removed in `scene`. `None`
    /// means every actor there may have.
    pub fn touched_actors(&self, scene: &str) -> Option<Vec<String>> {
        if self.whole.is_some() {
            return None;
        }
        let Some(change) = self.scenes.iter().find(|s| s.id == scene) else {
            return Some(Vec::new());
        };
        if change.all_actors.is_some() {
            return None;
        }
        Some(change.actors.iter().map(|a| a.id.clone()).collect())
    }

    /// Puts each part on its `after` side (`forward`) or its `before` side.
    pub fn apply(&self, project: &mut Project, forward: bool) {
        if let Some(whole) = &self.whole {
            *project = (**whole.side(forward)).clone();
            return;
        }
        if let Some(header) = &self.header {
            let scenes = std::mem::take(&mut project.scenes);
            *project = (**header.side(forward)).clone();
            project.scenes = scenes;
        }
        for change in &self.scenes {
            apply_scene(project, change, forward);
        }
        if let Some(order) = &self.scene_order {
            reorder(&mut project.scenes, order.side(forward), |s| &s.id);
        }
    }

    /// This change followed by `later`, as one step: each part keeps its
    /// first `before` and its last `after`.
    pub fn then(mut self, later: Change) -> Change {
        if self.whole.is_some() || later.whole.is_some() {
            // Rare (duplicate scene ids): rebuild both ends from a whole.
            let start = match &self.whole {
                Some(w) => (*w.before).clone(),
                None => {
                    let Some(w) = &later.whole else {
                        unreachable!()
                    };
                    let mut p = (*w.before).clone();
                    self.apply(&mut p, false);
                    p
                }
            };
            let mut end = start.clone();
            self.apply(&mut end, true);
            later.apply(&mut end, true);
            return Change {
                whole: Some(Swap {
                    before: Box::new(start),
                    after: Box::new(end),
                }),
                ..Change::default()
            };
        }
        self.header = merge(self.header, later.header);
        self.scene_order = merge(self.scene_order, later.scene_order);
        for next in later.scenes {
            match self.scenes.iter_mut().find(|s| s.id == next.id) {
                Some(scene) => {
                    scene.settings = merge(scene.settings.take(), next.settings);
                    scene.order = merge(scene.order.take(), next.order);
                    scene.all_actors = merge(scene.all_actors.take(), next.all_actors);
                    for actor in next.actors {
                        match scene.actors.iter_mut().find(|a| a.id == actor.id) {
                            Some(first) => first.after = actor.after,
                            None => scene.actors.push(actor),
                        }
                    }
                }
                None => self.scenes.push(next),
            }
        }
        self
    }
}

fn merge<T>(first: Option<Swap<T>>, later: Option<Swap<T>>) -> Option<Swap<T>> {
    match (first, later) {
        (Some(first), Some(later)) => Some(Swap {
            before: first.before,
            after: later.after,
        }),
        (first, None) => first,
        (None, later) => later,
    }
}

/// What turns `before` into `after`, part by part.
pub fn diff(before: &Project, after: &Project) -> Change {
    if !unique(before.scenes.iter().map(|s| &s.id)) || !unique(after.scenes.iter().map(|s| &s.id)) {
        return if before == after {
            Change::default()
        } else {
            Change {
                whole: Some(Swap {
                    before: Box::new(before.clone()),
                    after: Box::new(after.clone()),
                }),
                ..Change::default()
            }
        };
    }
    let mut change = Change::default();
    if !header_eq(before, after) {
        change.header = Some(Swap {
            before: Box::new(header_of(before)),
            after: Box::new(header_of(after)),
        });
    }
    let ids = |p: &Project| p.scenes.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
    let (old_ids, new_ids) = (ids(before), ids(after));
    if old_ids != new_ids {
        change.scene_order = Some(Swap {
            before: old_ids,
            after: new_ids,
        });
    }
    let old: HashMap<&str, &Scene> = before.scenes.iter().map(|s| (s.id.as_str(), s)).collect();
    let new: HashMap<&str, &Scene> = after.scenes.iter().map(|s| (s.id.as_str(), s)).collect();
    for scene in &after.scenes {
        let scene_change = diff_scene(&scene.id, old.get(scene.id.as_str()).copied(), Some(scene));
        if !scene_change.is_empty() {
            change.scenes.push(scene_change);
        }
    }
    for scene in &before.scenes {
        if !new.contains_key(scene.id.as_str()) {
            change.scenes.push(diff_scene(&scene.id, Some(scene), None));
        }
    }
    change
}

fn diff_scene(id: &str, before: Option<&Scene>, after: Option<&Scene>) -> SceneChange {
    let mut change = SceneChange::new(id);
    let settings_differ = match (before, after) {
        (Some(b), Some(a)) => !settings_eq(b, a),
        _ => true,
    };
    if settings_differ {
        change.settings = Some(Swap {
            before: before.map(|s| Box::new(settings_of(s))),
            after: after.map(|s| Box::new(settings_of(s))),
        });
    }
    let empty: &[Actor] = &[];
    let old = before.map_or(empty, |s| &s.actors);
    let new = after.map_or(empty, |s| &s.actors);
    if !unique(old.iter().map(|a| &a.id)) || !unique(new.iter().map(|a| &a.id)) {
        if old != new {
            change.all_actors = Some(Swap {
                before: old.to_vec(),
                after: new.to_vec(),
            });
        }
        return change;
    }
    let old_ids: Vec<String> = old.iter().map(|a| a.id.clone()).collect();
    let new_ids: Vec<String> = new.iter().map(|a| a.id.clone()).collect();
    if old_ids != new_ids {
        change.order = Some(Swap {
            before: old_ids,
            after: new_ids,
        });
    }
    let by_id: HashMap<&str, &Actor> = old.iter().map(|a| (a.id.as_str(), a)).collect();
    let kept: HashSet<&str> = new.iter().map(|a| a.id.as_str()).collect();
    for actor in new {
        match by_id.get(actor.id.as_str()) {
            Some(was) if same_actor(was, actor) => {}
            was => change.actors.push(ActorChange {
                id: actor.id.clone(),
                before: was.map(|a| Box::new((*a).clone())),
                after: Some(Box::new(actor.clone())),
            }),
        }
    }
    for actor in old {
        if !kept.contains(actor.id.as_str()) {
            change.actors.push(ActorChange {
                id: actor.id.clone(),
                before: Some(Box::new(actor.clone())),
                after: None,
            });
        }
    }
    change
}

/// `==`, plus instruction ids, which an instruction's own `==` ignores but
/// the canvas addresses blocks by.
fn same_actor(a: &Actor, b: &Actor) -> bool {
    if a != b {
        return false;
    }
    instruction_ids(a) == instruction_ids(b)
}

fn instruction_ids(actor: &Actor) -> u64 {
    // FNV-1a: this runs over every instruction of every actor per commit.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    actor.graph.walk_instructions(&mut |i| {
        for byte in i.id.bytes().chain(std::iter::once(0)) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    });
    hash
}

fn apply_scene(project: &mut Project, change: &SceneChange, forward: bool) {
    let at = project.scenes.iter().position(|s| s.id == change.id);
    if let Some(settings) = &change.settings {
        match (settings.side(forward), at) {
            (None, Some(at)) => {
                project.scenes.remove(at);
                return;
            }
            (None, None) => return,
            (Some(wanted), Some(at)) => {
                let actors = std::mem::take(&mut project.scenes[at].actors);
                project.scenes[at] = (**wanted).clone();
                project.scenes[at].actors = actors;
            }
            (Some(wanted), None) => project.scenes.push((**wanted).clone()),
        }
    }
    let Some(scene) = project.scenes.iter_mut().find(|s| s.id == change.id) else {
        return;
    };
    if let Some(all) = &change.all_actors {
        scene.actors = all.side(forward).clone();
        return;
    }
    for actor in &change.actors {
        let wanted = if forward { &actor.after } else { &actor.before };
        let at = scene.actors.iter().position(|a| a.id == actor.id);
        match (wanted, at) {
            (None, Some(at)) => {
                scene.actors.remove(at);
            }
            (Some(wanted), Some(at)) => scene.actors[at] = (**wanted).clone(),
            (Some(wanted), None) => scene.actors.push((**wanted).clone()),
            (None, None) => {}
        }
    }
    if let Some(order) = &change.order {
        reorder(&mut scene.actors, order.side(forward), |a| &a.id);
    }
}

/// Sorts `items` into `order`; anything it doesn't name keeps its place at
/// the end.
fn reorder<T>(items: &mut Vec<T>, order: &[String], id: impl Fn(&T) -> &String) {
    let rank: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut taken: Vec<(usize, T)> = items
        .drain(..)
        .enumerate()
        .map(|(i, item)| {
            let r = rank
                .get(id(&item).as_str())
                .copied()
                .unwrap_or(order.len() + i);
            (r, item)
        })
        .collect();
    taken.sort_by_key(|(r, _)| *r);
    items.extend(taken.into_iter().map(|(_, item)| item));
}

fn unique<'a>(ids: impl Iterator<Item = &'a String>) -> bool {
    let mut seen = HashSet::new();
    ids.into_iter().all(|id| seen.insert(id.as_str()))
}

/// The project without its scenes.
pub fn header_of(p: &Project) -> Project {
    let Project {
        id,
        name,
        icon,
        android,
        scenes: _,
        active_scene,
        default_scene,
        globals,
        global_lists,
        global_dicts,
        plugin_resources,
        physics,
        multiplayer,
        localization,
    } = p;
    Project {
        id: id.clone(),
        name: name.clone(),
        icon: icon.clone(),
        android: android.clone(),
        scenes: Vec::new(),
        active_scene: active_scene.clone(),
        default_scene: default_scene.clone(),
        globals: globals.clone(),
        global_lists: global_lists.clone(),
        global_dicts: global_dicts.clone(),
        plugin_resources: plugin_resources.clone(),
        physics: physics.clone(),
        multiplayer: multiplayer.clone(),
        localization: localization.clone(),
    }
}

fn header_eq(a: &Project, b: &Project) -> bool {
    let Project {
        id,
        name,
        icon,
        android,
        scenes: _,
        active_scene,
        default_scene,
        globals,
        global_lists,
        global_dicts,
        plugin_resources,
        physics,
        multiplayer,
        localization,
    } = a;
    *id == b.id
        && *name == b.name
        && *icon == b.icon
        && *android == b.android
        && *active_scene == b.active_scene
        && *default_scene == b.default_scene
        && *globals == b.globals
        && *global_lists == b.global_lists
        && *global_dicts == b.global_dicts
        && *plugin_resources == b.plugin_resources
        && *physics == b.physics
        && *multiplayer == b.multiplayer
        && *localization == b.localization
}

/// The scene without its actors.
pub fn settings_of(s: &Scene) -> Scene {
    let Scene {
        id,
        name,
        path,
        world,
        actors: _,
    } = s;
    Scene {
        id: id.clone(),
        name: name.clone(),
        path: path.clone(),
        world: world.clone(),
        actors: Vec::new(),
    }
}

fn settings_eq(a: &Scene, b: &Scene) -> bool {
    let Scene {
        id,
        name,
        path,
        world,
        actors: _,
    } = a;
    *id == b.id && *name == b.name && *path == b.path && *world == b.world
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Mode, Visual};

    fn ball(name: &str) -> Actor {
        Actor::new(
            name,
            Visual::Circle {
                color: "#ffffff".into(),
                radius: 10.0,
            },
        )
    }

    fn project() -> Project {
        let mut p = Project::starter("Diff", Mode::TwoD);
        p.add_actor(ball("A"));
        p.add_actor(ball("B"));
        let first = p.active_scene.clone();
        p.add_scene("Two", Some(Mode::ThreeD));
        p.active_scene = first;
        p
    }

    /// Diffing, then applying both ways, lands on each document exactly.
    fn round_trip(before: &Project, after: &Project) -> Change {
        let change = diff(before, after);
        let mut forward = before.clone();
        change.apply(&mut forward, true);
        assert_eq!(&forward, after, "forward");
        let mut back = after.clone();
        change.apply(&mut back, false);
        assert_eq!(&back, before, "backward");
        change
    }

    #[test]
    fn an_unchanged_project_has_no_change() {
        let p = project();
        assert!(diff(&p, &p.clone()).is_empty());
    }

    #[test]
    fn a_rename_touches_one_actor_and_one_scene() {
        let before = project();
        let mut after = before.clone();
        let id = after.actors[1].id.clone();
        after.actor_mut(&id).unwrap().name = "Renamed".into();
        let change = round_trip(&before, &after);
        assert!(!change.touches_index());
        assert_eq!(
            change.touched_scenes().unwrap(),
            vec![before.active_scene.clone()]
        );
        assert_eq!(
            change.touched_actors(&before.active_scene).unwrap(),
            vec![id]
        );
    }

    #[test]
    fn adding_removing_and_reordering_round_trip() {
        let before = project();
        let mut after = before.clone();
        let first = after.actors.remove(0);
        after.add_actor(ball("C"));
        after.actors.insert(1, first);
        round_trip(&before, &after);
    }

    #[test]
    fn scenes_added_removed_and_renamed_round_trip() {
        let before = project();
        let mut after = before.clone();
        after.scenes.remove(1);
        after.add_scene("Three", Some(Mode::TwoD));
        after.scenes[0].name = "Renamed".into();
        after.scenes[0].world.gravity = [0.0, -1.0, 0.0];
        after.name = "Other".into();
        let change = round_trip(&before, &after);
        assert!(change.touches_index());
        after.scenes.reverse();
        round_trip(&before, &after);
    }

    #[test]
    fn duplicate_ids_fall_back_to_wholes() {
        let before = project();
        let mut after = before.clone();
        let copy = after.actors[0].clone();
        after.actors.push(copy);
        let change = round_trip(&before, &after);
        assert!(change.touched_actors(&before.active_scene).is_none());
        let mut twice = after.clone();
        let scene = twice.scenes[0].clone();
        twice.scenes.push(scene);
        assert!(round_trip(&after, &twice).whole.is_some());
    }

    #[test]
    fn steps_merge_into_one() {
        let a = project();
        let mut b = a.clone();
        b.actors[0].name = "One".into();
        let mut c = b.clone();
        c.actors[0].name = "Two".into();
        c.add_actor(ball("D"));
        let merged = diff(&a, &b).then(diff(&b, &c));
        let mut forward = a.clone();
        merged.apply(&mut forward, true);
        assert_eq!(forward, c);
        let mut back = c.clone();
        merged.apply(&mut back, false);
        assert_eq!(back, a);
    }
}
