//! Prefab commands: make one from an actor and whatever hangs off it, place
//! it, push an instance's changes back into it, and drop overrides.
//! Instances are rebuilt from the file whenever it changes, here or on disk.

use super::{auto_save, emit, lock, project_dir, push_undo, reindex, sync_runtime};
use crate::AppHandle;
use crate::state::{AppState, SharedState};
use blockloom_core::assets;
use blockloom_core::components::ActorComponent;
use blockloom_core::prefab::{self, Member, PrefabLink};
use blockloom_core::project::{Actor, Project, Scene};
use blockloom_core::resources::{ResourceIndex, ResourceRef};
use std::path::Path;

/// Where a link's prefab is now, its path repaired from the id.
fn source_path(index: &ResourceIndex, link: &PrefabLink) -> String {
    let mut source = link.source.clone();
    source.resolve(index);
    source.path
}

/// The prefab file at `path`, or the error naming it.
fn read_prefab(dir: &Path, path: &str) -> Result<Actor, String> {
    prefab::read(dir, path)
}

/// The file's state of the member `link` names.
fn member_of<'a>(members: &'a [Member], link: &PrefabLink) -> Option<&'a Actor> {
    members
        .iter()
        .find(|m| m.id == link.member)
        .map(|m| &m.actor)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The root of the instance `actor` belongs to, by id.
fn root_of(actor: &Actor) -> Option<String> {
    let link = prefab::link(actor)?;
    Some(if link.instance.is_empty() {
        actor.id.clone()
    } else {
        link.instance.clone()
    })
}

/// Every actor of the instance rooted at `root`, root first.
fn instance_ids(scene: &Scene, root: &str) -> Vec<String> {
    let mut ids = vec![root.to_string()];
    ids.extend(
        scene
            .actors
            .iter()
            .filter(|a| prefab::link(a).is_some_and(|l| l.instance == root))
            .map(|a| a.id.clone()),
    );
    ids
}

/// Rebuilds one instance from its file's members: existing members follow
/// the file, ones the file gained are added and ones it lost are removed,
/// unless the instance deleted them itself. True if anything changed.
fn sync_instance(
    scene: &mut Scene,
    root: &str,
    members: &[Member],
    index: &ResourceIndex,
    path: &str,
) -> bool {
    let mut changed = false;
    let Some(root_actor) = scene.actor(root).cloned() else {
        return false;
    };
    let removed: Vec<String> = prefab::link(&root_actor)
        .map(|l| {
            l.overrides
                .iter()
                .filter_map(|o| o.strip_prefix("removed:").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // Member id to scene id, for parents and the members still to add.
    let mut placed: std::collections::BTreeMap<String, String> = Default::default();
    placed.insert(String::new(), root.to_string());
    for actor in &scene.actors {
        if let Some(link) = prefab::link(actor).filter(|l| l.instance == root) {
            placed.insert(link.member.clone(), actor.id.clone());
        }
    }
    // Members the file no longer has go.
    let gone: Vec<String> = placed
        .iter()
        .filter(|(member, _)| !member.is_empty() && !members.iter().any(|m| &m.id == *member))
        .map(|(_, id)| id.clone())
        .collect();
    for id in gone {
        scene.remove_actor(&id);
        changed = true;
    }
    let shift = {
        let file = members[0].actor.placement().position;
        let here = root_actor.placement().position;
        [here[0] - file[0], here[1] - file[1], here[2] - file[2]]
    };
    let mut added: Vec<String> = Vec::new();
    let mut after = scene
        .actors
        .iter()
        .rposition(|a| placed.values().any(|id| *id == a.id))
        .unwrap_or(0);
    for member in members {
        match placed.get(&member.id).cloned() {
            Some(id) => {
                let Some(actor) = scene.actor_mut(&id) else {
                    continue;
                };
                match prefab::rebuild(actor, &member.actor) {
                    Ok(mut next) => {
                        if let Some(link) = prefab::link_mut(&mut next) {
                            link.source.path = path.to_string();
                            if link.source.id.is_empty() {
                                link.source = ResourceRef::to(index, path);
                            }
                        }
                        if !blockloom_core::change::same_actor(&next, actor) {
                            *actor = next;
                            changed = true;
                        }
                    }
                    Err(e) => tracing::warn!("{e}"),
                }
            }
            None if removed.contains(&member.id) => {}
            None => {
                let mut actor = member.actor.clone();
                actor
                    .components
                    .0
                    .retain(|c| !matches!(c, ActorComponent::Prefab { .. }));
                actor.id = new_id();
                actor.refresh_physics_ids();
                actor.components.remove("Camera");
                let mut place = actor.placement();
                for (axis, d) in shift.iter().enumerate() {
                    place.position[axis] += d;
                }
                actor.components.set_placement(place);
                actor.components.0.push(ActorComponent::Prefab {
                    prefab: PrefabLink {
                        source: ResourceRef::to(index, path),
                        member: member.id.clone(),
                        instance: root.to_string(),
                        ..Default::default()
                    },
                });
                actor.name = scene.unique_actor_name(&actor.name);
                placed.insert(member.id.clone(), actor.id.clone());
                added.push(member.id.clone());
                after = (after + 1).min(scene.actors.len());
                scene.actors.insert(after, actor);
                changed = true;
            }
        }
    }
    // Parents of added members name scene ids.
    for member in members.iter().skip(1).filter(|m| added.contains(&m.id)) {
        let target = member
            .parent
            .as_deref()
            .and_then(|p| placed.get(p))
            .cloned();
        if let Some(actor) = scene.actor_mut(&placed[&member.id]) {
            actor.components.set_parent(target.as_deref().unwrap_or(""));
        }
    }
    changed
}

/// Rebuilds every instance of `path` (every instance at all with `None`)
/// from its file. Answers how many changed.
pub(crate) fn sync_instances(project: &mut Project, dir: &Path, only: Option<&str>) -> usize {
    let index = ResourceIndex::load(dir);
    let mut cache: std::collections::HashMap<String, Option<Vec<Member>>> = Default::default();
    let mut changed = 0;
    for scene in &mut project.scenes {
        let roots: Vec<(String, String)> = scene
            .actors
            .iter()
            .filter_map(|a| {
                let link = prefab::link(a)?;
                (link.member.is_empty() && link.instance.is_empty())
                    .then(|| (a.id.clone(), source_path(&index, link)))
            })
            .collect();
        for (root, path) in roots {
            if only.is_some_and(|only| only != path) {
                continue;
            }
            let members = cache.entry(path.clone()).or_insert_with(|| {
                match prefab::read_members(dir, &path) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        tracing::warn!("{e}");
                        None
                    }
                }
            });
            let Some(members) = members else {
                continue;
            };
            if sync_instance(scene, &root, members, &index, &path) {
                changed += 1;
            }
        }
    }
    changed
}

/// Records overrides for every instance an edit just touched, against the
/// last committed copy, and the members an instance deleted. Runs before
/// each commit.
pub(crate) fn mark_overrides(s: &mut AppState) {
    let Some(open) = &mut s.open else {
        return;
    };
    let Some(shadow) = open.tracker.shadow() else {
        return;
    };
    let dir = open.dir.clone();
    let mut index: Option<ResourceIndex> = None;
    let mut cache: std::collections::HashMap<String, Option<Vec<Member>>> = Default::default();
    for scene in &mut open.project.scenes {
        let Some(before_scene) = shadow.scene(&scene.id) else {
            continue;
        };
        // Members gone since the last commit, by their instance root.
        let mut deleted: Vec<(String, String)> = Vec::new();
        for before in &before_scene.actors {
            if let Some(link) = prefab::link(before).filter(|l| !l.instance.is_empty())
                && scene.actor(&before.id).is_none()
            {
                deleted.push((link.instance.clone(), link.member.clone()));
            }
        }
        for (root, member) in deleted {
            if let Some(link) = scene.actor_mut(&root).and_then(prefab::link_mut) {
                let entry = prefab::removed_entry(&member);
                if let Err(at) = link.overrides.binary_search(&entry) {
                    link.overrides.insert(at, entry);
                }
            }
        }
        for actor in &mut scene.actors {
            let Some(link) = prefab::link(actor) else {
                continue;
            };
            let Some(before) = before_scene.actors.iter().find(|a| a.id == actor.id) else {
                continue;
            };
            if before == actor && blockloom_core::change::same_actor(before, actor) {
                continue;
            }
            let index = index.get_or_insert_with(|| ResourceIndex::load(&dir));
            let path = source_path(index, link);
            let members = cache
                .entry(path.clone())
                .or_insert_with(|| prefab::read_members(&dir, &path).ok());
            let Some(source) = members.as_deref().and_then(|m| member_of(m, link)) else {
                continue;
            };
            let mut overrides = link.overrides.clone();
            prefab::mark(before, actor, source, &mut overrides);
            if let Some(link) = prefab::link_mut(actor) {
                link.overrides = overrides;
            }
        }
    }
}

/// A file name for a new prefab of `name` under `assets/prefabs`.
fn new_prefab_path(dir: &Path, name: &str) -> Result<String, String> {
    let folder = "assets/prefabs";
    std::fs::create_dir_all(dir.join(folder)).map_err(|e| e.to_string())?;
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.trim().is_empty() {
        "Prefab".to_string()
    } else {
        stem.trim().to_string()
    };
    for n in 1.. {
        let file = if n == 1 {
            format!("{stem}.{}", prefab::EXTENSION)
        } else {
            format!("{stem} {n}.{}", prefab::EXTENSION)
        };
        let path = format!("{folder}/{file}");
        if !dir.join(&path).exists() {
            return Ok(path);
        }
    }
    unreachable!()
}

/// `actor` and every actor hanging off it in `scene`, root first, parents
/// before their children.
fn subtree(scene: &Scene, root: &str) -> Vec<Actor> {
    let mut out: Vec<Actor> = scene.actor(root).cloned().into_iter().collect();
    let mut at = 0;
    while at < out.len() {
        let id = out[at].id.clone();
        out.extend(
            scene
                .actors
                .iter()
                .filter(|a| a.components.parent() == Some(id.as_str()))
                .cloned(),
        );
        at += 1;
    }
    out
}

/// The file layout of an instance being written: members a nested prefab
/// brings are left to that prefab, a nested root keeps its link with fresh
/// overrides, and parents name member ids. `file_root` is the root's id in
/// the file.
fn file_children(
    dir: &Path,
    actors: &[Actor],
    member_ids: &std::collections::BTreeMap<String, String>,
    file_root: &str,
    nested: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<Actor>, String> {
    let index = ResourceIndex::load(dir);
    let mut out = Vec::new();
    for actor in actors.iter().skip(1) {
        let member = &member_ids[&actor.id];
        if member.contains('/') {
            continue;
        }
        let mut child = actor.clone();
        child.id = member.clone();
        if let Some(parent) = actor.components.parent() {
            match member_ids.get(parent) {
                Some(p) if p.is_empty() => child.components.set_parent(file_root),
                Some(p) => child.components.set_parent(p),
                None => child.components.set_parent(""),
            }
        }
        // A nested instance root stays a link to its own prefab.
        let nested = nested.get(&actor.id).cloned();
        child
            .components
            .0
            .retain(|c| !matches!(c, ActorComponent::Prefab { .. }));
        if let Some(path) = nested {
            let source = read_prefab(dir, &path)?;
            child.components.0.push(ActorComponent::Prefab {
                prefab: PrefabLink {
                    source: ResourceRef::to(&index, &path),
                    overrides: prefab::differences(&child, &source),
                    ..Default::default()
                },
            });
        }
        out.push(child);
    }
    Ok(out)
}

/// `create-prefab`: saves an actor and everything hanging off it as a
/// prefab and makes them its first instance. A child that is a prefab's
/// instance is nested by reference. Answers the prefab's path.
pub(crate) fn create_prefab(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: Option<String>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let scene = s
        .project()
        .map(|p| p.active_scene().clone())
        .ok_or("No project is open")?;
    let mut actors = subtree(&scene, &actor_id);
    // A nested instance comes whole, even members not hung off its root.
    let roots: Vec<String> = actors
        .iter()
        .skip(1)
        .filter(|a| prefab::link(a).is_some_and(|l| l.member.is_empty() && l.instance.is_empty()))
        .map(|a| a.id.clone())
        .collect();
    for root in roots {
        for id in instance_ids(&scene, &root) {
            if !actors.iter().any(|a| a.id == id)
                && let Some(a) = scene.actor(&id)
            {
                actors.push(a.clone());
            }
        }
    }
    let actor = actors.first().cloned().ok_or("Actor not found")?;
    if prefab::link(&actor).is_some() {
        return Err(format!(
            "\"{}\" is already an instance of a prefab",
            actor.name
        ));
    }
    // Member ids: a nested instance's members sit under its root's id.
    let inside: std::collections::BTreeSet<&str> = actors.iter().map(|a| a.id.as_str()).collect();
    let mut member_ids: std::collections::BTreeMap<String, String> = Default::default();
    member_ids.insert(actor.id.clone(), String::new());
    for a in actors.iter().skip(1) {
        let id = match prefab::link(a) {
            Some(l) if !l.instance.is_empty() => {
                if !inside.contains(l.instance.as_str()) {
                    return Err(format!(
                        "\"{}\" belongs to a prefab instance outside \"{}\"",
                        a.name, actor.name
                    ));
                }
                format!("{}/{}", l.instance, l.member)
            }
            _ => a.id.clone(),
        };
        member_ids.insert(a.id.clone(), id);
    }
    let path = match path.filter(|p| !p.trim().is_empty()) {
        Some(path) => {
            let path = assets::normalize(&path).ok_or("That isn't a path in this project")?;
            if !prefab::is_prefab(&path) {
                return Err(format!("A prefab's file ends in .{}", prefab::EXTENSION));
            }
            if let Some(parent) = Path::new(&path).parent() {
                std::fs::create_dir_all(dir.join(parent)).map_err(|e| e.to_string())?;
            }
            path
        }
        None => new_prefab_path(&dir, &actor.name)?,
    };
    let index = ResourceIndex::load(&dir);
    let nested: std::collections::BTreeMap<String, String> = actors
        .iter()
        .skip(1)
        .filter_map(|a| {
            let l = prefab::link(a).filter(|l| l.member.is_empty() && l.instance.is_empty())?;
            Some((a.id.clone(), source_path(&index, l)))
        })
        .collect();
    let children = file_children(&dir, &actors, &member_ids, &actor.id, &nested)?;
    prefab::write_members(&dir, &path, &actor, &children)?;
    reindex(&dir);
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    for a in &actors {
        let Some(scene_actor) = project.actor_mut(&a.id) else {
            continue;
        };
        scene_actor
            .components
            .0
            .retain(|c| !matches!(c, ActorComponent::Prefab { .. }));
        let member = member_ids[&a.id].clone();
        scene_actor.components.0.push(ActorComponent::Prefab {
            prefab: PrefabLink {
                source: ResourceRef::to(&index, &path),
                instance: if member.is_empty() {
                    String::new()
                } else {
                    actor.id.clone()
                },
                member,
                ..Default::default()
            },
        });
    }
    // A nested member's overrides are against the expanded file.
    sync_instances(project, &dir, Some(&path));
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(path)
}

/// `instantiate-prefab`: places a prefab, children and all, in the open
/// scene. Answers the root actor's id.
pub(crate) fn instantiate_prefab(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    position: Option<[f32; 3]>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let path = assets::normalize(&path).ok_or("That isn't a path in this project")?;
    let members = prefab::read_members(&dir, &path)?;
    let index = ResourceIndex::load(&dir);
    let mut actors = prefab::instantiate(&members, &ResourceRef::to(&index, &path), new_id);
    if let Some(position) = position {
        let from = actors[0].placement().position;
        let shift = [
            position[0] - from[0],
            position[1] - from[1],
            position[2] - from[2],
        ];
        for actor in &mut actors {
            let mut place = actor.placement();
            for (axis, d) in shift.iter().enumerate() {
                place.position[axis] += d;
            }
            actor.components.set_placement(place);
        }
    }
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    let mut root = String::new();
    for mut actor in actors {
        // There is one camera, so a placed copy doesn't take it.
        actor.components.remove("Camera");
        let id = project.add_actor(actor);
        if root.is_empty() {
            root = id;
        }
    }
    s.selected_actor = Some(root.clone());
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(root)
}

/// `apply-prefab`: makes an instance's current state the prefab's (from any
/// of its actors), and rebuilds every other instance from it. Actors hung
/// off the instance since become members. Answers how many others changed.
pub(crate) fn apply_prefab(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<usize, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let scene = s
        .project()
        .map(|p| p.active_scene().clone())
        .ok_or("No project is open")?;
    let picked = scene.actor(&actor_id).ok_or("Actor not found")?;
    let root_id = root_of(picked).ok_or("That actor isn't a prefab instance")?;
    let root = scene
        .actor(&root_id)
        .cloned()
        .ok_or("The instance's root is gone")?;
    let link = prefab::link(&root)
        .cloned()
        .ok_or("That actor isn't a prefab instance")?;
    let index = ResourceIndex::load(&dir);
    let path = source_path(&index, &link);
    let old = prefab::read_members(&dir, &path).ok();
    // The instance's actors: its members plus anything hung off them since.
    let mut actors = subtree(&scene, &root_id);
    for id in instance_ids(&scene, &root_id) {
        if !actors.iter().any(|a| a.id == id)
            && let Some(a) = scene.actor(&id)
        {
            actors.push(a.clone());
        }
    }
    let mut member_ids: std::collections::BTreeMap<String, String> = Default::default();
    for a in &actors {
        let id = match prefab::link(a) {
            Some(l) if a.id == root_id => {
                let _ = l;
                String::new()
            }
            Some(l) if l.instance == root_id => l.member.clone(),
            Some(l) if !l.instance.is_empty() => {
                return Err(format!("\"{}\" belongs to another prefab instance", a.name));
            }
            _ => a.id.clone(),
        };
        member_ids.insert(a.id.clone(), id);
    }
    let mut source = root.clone();
    let file_root = old
        .as_ref()
        .map(|_| read_root_id(&dir, &path))
        .unwrap_or_else(|| root.id.clone());
    // The prefab keeps its own place and name; the rest is the instance's.
    if let Some(old) = old.as_ref().map(|m| &m[0].actor) {
        source.name = old.name.clone();
        source.components.set_placement(old.placement());
    }
    source.id = file_root.clone();
    let shift = {
        let file = source.placement().position;
        let here = root.placement().position;
        [file[0] - here[0], file[1] - here[1], file[2] - here[2]]
    };
    // Members that nest a prefab keep doing so.
    let nested: std::collections::BTreeMap<String, String> = actors
        .iter()
        .filter_map(|a| {
            let member = &member_ids[&a.id];
            let held = old.as_ref()?.iter().find(|m| &m.id == member)?;
            let l = prefab::link(&held.actor)
                .filter(|l| l.member.is_empty() && l.instance.is_empty())?;
            Some((a.id.clone(), source_path(&index, l)))
        })
        .collect();
    let mut children = file_children(&dir, &actors, &member_ids, &file_root, &nested)?;
    for child in &mut children {
        let mut place = child.placement();
        for (axis, d) in shift.iter().enumerate() {
            place.position[axis] += d;
        }
        child.components.set_placement(place);
    }
    prefab::write_members(&dir, &path, &source, &children)?;
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    // Every actor of this instance now matches the file.
    for a in &actors {
        let Some(actor) = project.actor_mut(&a.id) else {
            continue;
        };
        let member = member_ids[&a.id].clone();
        actor
            .components
            .0
            .retain(|c| !matches!(c, ActorComponent::Prefab { .. }));
        actor.components.0.push(ActorComponent::Prefab {
            prefab: PrefabLink {
                source: link.source.clone(),
                instance: if member.is_empty() {
                    String::new()
                } else {
                    root_id.clone()
                },
                member,
                ..Default::default()
            },
        });
    }
    // Prefabs nesting this one change too, so every instance is rebuilt.
    let changed = sync_instances(project, &dir, None);
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(changed)
}

/// The root actor's id as the file at `path` spells it.
fn read_root_id(dir: &Path, path: &str) -> String {
    let full = dir.join(path);
    std::fs::read_to_string(full)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["actor"]["id"].as_str().map(str::to_string))
        .unwrap_or_else(new_id)
}

/// `revert-prefab`: drops one override (`path`) or all of them, taking the
/// prefab's values back. On a root, a `removed:` entry reverted brings that
/// member back.
pub(crate) fn revert_prefab(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: Option<String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    let actor = project.actor_mut(&actor_id).ok_or("Actor not found")?;
    let link = prefab::link_mut(actor).ok_or("That actor isn't a prefab instance")?;
    match path.filter(|p| !p.is_empty()) {
        Some(path) => link.overrides.retain(|o| *o != path),
        None => link.overrides.clear(),
    }
    let is_root = link.member.is_empty() && link.instance.is_empty();
    let source_at = source_path(&index, link);
    let members = prefab::read_members(&dir, &source_at)?;
    let link = link.clone();
    let source = member_of(&members, &link).ok_or("The prefab no longer has this actor")?;
    *actor = prefab::rebuild(actor, source)?;
    // A root's revert may bring back a member it deleted.
    if is_root {
        let scene = project.active_scene_mut();
        sync_instance(scene, &actor_id, &members, &index, &source_at);
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `unpack-prefab`: keeps the instance as it is but forgets its prefab,
/// for every actor of it.
pub(crate) fn unpack_prefab(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    let actor = project.actor(&actor_id).ok_or("Actor not found")?;
    let root = root_of(actor).ok_or("That actor isn't a prefab instance")?;
    let scene = project.active_scene_mut();
    for id in instance_ids(scene, &root) {
        if let Some(actor) = scene.actor_mut(&id) {
            actor.components.remove("Prefab");
        }
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}
