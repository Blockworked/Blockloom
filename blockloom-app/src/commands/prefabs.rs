//! Prefab commands: make one from an actor, place it, push an instance's
//! changes back into it, and drop overrides. Instances are rebuilt from the
//! file whenever it changes, here or on disk.

use super::{auto_save, emit, lock, project_dir, push_undo, reindex, sync_runtime};
use crate::AppHandle;
use crate::state::{AppState, SharedState};
use blockloom_core::assets;
use blockloom_core::components::ActorComponent;
use blockloom_core::prefab::{self, PrefabLink};
use blockloom_core::project::{Actor, Project};
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

/// Rebuilds every instance of `path` (every instance at all with `None`)
/// from its file. Answers how many changed.
pub(crate) fn sync_instances(project: &mut Project, dir: &Path, only: Option<&str>) -> usize {
    let index = ResourceIndex::load(dir);
    let mut cache: std::collections::HashMap<String, Option<Actor>> = Default::default();
    let mut changed = 0;
    for scene in &mut project.scenes {
        for actor in &mut scene.actors {
            let Some(link) = prefab::link(actor).cloned() else {
                continue;
            };
            let path = source_path(&index, &link);
            if only.is_some_and(|only| only != path) {
                continue;
            }
            let source = cache
                .entry(path.clone())
                .or_insert_with(|| read_prefab(dir, &path).ok());
            let Some(source) = source else {
                continue;
            };
            match prefab::rebuild(actor, source) {
                Ok(mut next) => {
                    if let Some(link) = prefab::link_mut(&mut next) {
                        link.source.path = path.clone();
                        if link.source.id.is_empty() {
                            link.source = ResourceRef::to(&index, &path);
                        }
                    }
                    if !blockloom_core::change::same_actor(&next, actor) {
                        *actor = next;
                        changed += 1;
                    }
                }
                Err(e) => tracing::warn!("{e}"),
            }
        }
    }
    changed
}

/// Records overrides for every instance an edit just touched, against the
/// last committed copy. Runs before each commit.
pub(crate) fn mark_overrides(s: &mut AppState) {
    let Some(open) = &mut s.open else {
        return;
    };
    let Some(shadow) = open.tracker.shadow() else {
        return;
    };
    let dir = open.dir.clone();
    let mut index: Option<ResourceIndex> = None;
    for scene in &mut open.project.scenes {
        let Some(before_scene) = shadow.scene(&scene.id) else {
            continue;
        };
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
            let Ok(source) = read_prefab(&dir, &path) else {
                continue;
            };
            let mut overrides = link.overrides.clone();
            prefab::mark(before, actor, &source, &mut overrides);
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

/// `create-prefab`: saves an actor as a prefab and makes it the first
/// instance. Answers the prefab's path.
pub(crate) fn create_prefab(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: Option<String>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let actor = s
        .project()
        .and_then(|p| p.actor(&actor_id))
        .cloned()
        .ok_or("Actor not found")?;
    if prefab::link(&actor).is_some() {
        return Err(format!(
            "\"{}\" is already an instance of a prefab",
            actor.name
        ));
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
    prefab::write(&dir, &path, &actor)?;
    reindex(&dir);
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        actor.components.0.push(ActorComponent::Prefab {
            prefab: PrefabLink {
                source: ResourceRef::to(&index, &path),
                overrides: Vec::new(),
            },
        });
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(path)
}

/// `instantiate-prefab`: places a prefab in the open scene. Answers the new
/// actor's id.
pub(crate) fn instantiate_prefab(
    state: &SharedState,
    app: &AppHandle,
    path: String,
    position: Option<[f32; 3]>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let path = assets::normalize(&path).ok_or("That isn't a path in this project")?;
    let mut actor = read_prefab(&dir, &path)?;
    actor.id = uuid::Uuid::new_v4().simple().to_string();
    actor.refresh_physics_ids();
    if let Some(position) = position {
        actor.components.placement_mut().position = position;
    }
    let index = ResourceIndex::load(&dir);
    actor.components.0.push(ActorComponent::Prefab {
        prefab: PrefabLink {
            source: ResourceRef::to(&index, &path),
            overrides: Vec::new(),
        },
    });
    push_undo(&mut s);
    // There is one camera, so a placed copy doesn't take it.
    actor.components.remove("Camera");
    let project = s.project_mut().ok_or("No project is open")?;
    let id = project.add_actor(actor);
    s.selected_actor = Some(id.clone());
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(id)
}

/// `apply-prefab`: makes an instance's current state the prefab's, and
/// rebuilds every other instance from it. Answers how many others changed.
pub(crate) fn apply_prefab(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<usize, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let actor = s
        .project()
        .and_then(|p| p.actor(&actor_id))
        .cloned()
        .ok_or("Actor not found")?;
    let link = prefab::link(&actor)
        .cloned()
        .ok_or("That actor isn't a prefab instance")?;
    let path = source_path(&ResourceIndex::load(&dir), &link);
    let mut source = actor.clone();
    // The prefab keeps its own place and name; the rest is the instance's.
    if let Ok(old) = read_prefab(&dir, &path) {
        source.name = old.name.clone();
        source.components.set_placement(old.placement());
    }
    prefab::write(&dir, &path, &source)?;
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    if let Some(link) = project.actor_mut(&actor_id).and_then(prefab::link_mut) {
        link.overrides.clear();
    }
    let changed = sync_instances(project, &dir, Some(&path));
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(changed)
}

/// `revert-prefab`: drops one override (`path`) or all of them, taking the
/// prefab's values back.
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
    let source = read_prefab(&dir, &source_path(&index, link))?;
    *actor = prefab::rebuild(actor, &source)?;
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `unpack-prefab`: keeps the actor as it is but forgets its prefab.
pub(crate) fn unpack_prefab(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    let actor = project.actor_mut(&actor_id).ok_or("Actor not found")?;
    if !actor.components.remove("Prefab") {
        return Err("That actor isn't a prefab instance".into());
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}
