//! Material asset commands: save an actor's surface as a `.blockmaterial`,
//! put one on an actor, push an instance's changes back into the file and
//! drop overrides. Instances are rebuilt whenever the file changes.

use super::{auto_save, emit, lock, project_dir, push_undo, reindex, sync_runtime};
use crate::AppHandle;
use crate::state::{AppState, SharedState};
use blockloom_core::assets;
use blockloom_core::components::ActorComponent;
use blockloom_core::linked::AssetLink;
use blockloom_core::material::SurfaceMaterial;
use blockloom_core::material_asset as asset;
use blockloom_core::project::{Actor, Project};
use blockloom_core::resources::{ResourceIndex, ResourceRef};
use std::collections::HashMap;
use std::path::Path;

fn source_path(index: &ResourceIndex, link: &AssetLink) -> String {
    let mut source = link.source.clone();
    source.resolve(index);
    source.path
}

fn material_of(actor: &Actor) -> Option<&SurfaceMaterial> {
    actor.components.iter().find_map(|c| match c {
        ActorComponent::Material { material, .. } => Some(material),
        _ => None,
    })
}

/// Rebuilds every actor following `only` (or any file) from it. Answers how
/// many changed.
pub(crate) fn sync_instances(project: &mut Project, dir: &Path, only: Option<&str>) -> usize {
    let index = ResourceIndex::load(dir);
    let mut cache: HashMap<String, Option<SurfaceMaterial>> = HashMap::new();
    let mut changed = 0;
    for scene in &mut project.scenes {
        for actor in &mut scene.actors {
            let Some((material, link)) = asset::linked_mut(actor) else {
                continue;
            };
            let path = source_path(&index, link);
            if only.is_some_and(|only| only != path) {
                continue;
            }
            let Some(file) = cache
                .entry(path.clone())
                .or_insert_with(|| asset::read(dir, &path).ok())
            else {
                continue;
            };
            let next = asset::rebuild(material, file, link);
            if link.source.path != path || link.source.id.is_empty() {
                link.source = ResourceRef::to(&index, &path);
            }
            if next != *material {
                *material = next;
                changed += 1;
            }
        }
    }
    changed
}

/// Records overrides for every linked material an edit just touched.
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
            let Some(before) = before_scene
                .actors
                .iter()
                .find(|a| a.id == actor.id)
                .and_then(material_of)
            else {
                continue;
            };
            let Some((now, link)) = asset::linked_mut(actor) else {
                continue;
            };
            if before == now {
                continue;
            }
            let index = index.get_or_insert_with(|| ResourceIndex::load(&dir));
            let Ok(file) = asset::read(&dir, &source_path(index, link)) else {
                continue;
            };
            asset::mark(before, now, &file, link);
        }
    }
}

fn new_material_path(dir: &Path, name: &str) -> String {
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
        "Material".to_string()
    } else {
        stem.trim().to_string()
    };
    (1..)
        .map(|n| match n {
            1 => format!("assets/materials/{stem}.{}", asset::EXTENSION),
            n => format!("assets/materials/{stem} {n}.{}", asset::EXTENSION),
        })
        .find(|path| !dir.join(path).exists())
        .expect("an unused name always exists")
}

fn checked_path(path: &str) -> Result<String, String> {
    let path = assets::normalize(path).ok_or("That isn't a path in this project")?;
    if !asset::is_material(&path) {
        return Err(format!("A material's file ends in .{}", asset::EXTENSION));
    }
    Ok(path)
}

/// Puts `material` on the actor, following `path`.
fn link_actor(actor: &mut Actor, material: SurfaceMaterial, index: &ResourceIndex, path: &str) {
    actor.components.insert(ActorComponent::Material {
        material,
        source: Some(AssetLink {
            source: ResourceRef::to(index, path),
            overrides: Vec::new(),
            ..Default::default()
        }),
    });
}

/// `create-material-asset`: saves an actor's material (a plain one if it has
/// none) as a file and makes the actor follow it. Answers the path.
pub(crate) fn create_material_asset(
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
        .ok_or("That actor isn't in this scene: pick one from the actor list")?;
    let path = match path.filter(|p| !p.trim().is_empty()) {
        Some(path) => checked_path(&path)?,
        None => new_material_path(&dir, &actor.name),
    };
    let material = material_of(&actor).cloned().unwrap_or_default();
    asset::write(&dir, &path, &material)?;
    reindex(&dir);
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    if let Some(actor) = s.project_mut().and_then(|p| p.actor_mut(&actor_id)) {
        link_actor(actor, material, &index, &path);
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(path)
}

/// `assign-material`: makes an actor follow a material file, replacing its
/// own material.
pub(crate) fn assign_material(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let path = checked_path(&path)?;
    let material = asset::read(&dir, &path)?;
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    let actor = s
        .project_mut()
        .and_then(|p| p.actor_mut(&actor_id))
        .ok_or("That actor isn't in this scene: pick one from the actor list")?;
    link_actor(actor, material, &index, &path);
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `apply-material`: writes an instance's material into its file and
/// rebuilds every other actor following it. Answers how many changed.
pub(crate) fn apply_material(
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
        .ok_or("That actor isn't in this scene: pick one from the actor list")?;
    let link = asset::link(&actor)
        .cloned()
        .ok_or("That actor's material doesn't follow a file")?;
    let path = source_path(&ResourceIndex::load(&dir), &link);
    let material = material_of(&actor).cloned().unwrap_or_default();
    asset::write(&dir, &path, &material)?;
    push_undo(&mut s);
    let project = s
        .project_mut()
        .ok_or("No project is open: open or create one on the Dashboard")?;
    if let Some((_, link)) = project.actor_mut(&actor_id).and_then(asset::linked_mut) {
        link.overrides.clear();
    }
    let changed = sync_instances(project, &dir, Some(&path));
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(changed)
}

/// `revert-material`: drops one override (`path`) or all of them.
pub(crate) fn revert_material(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
    path: Option<String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    let project = s
        .project_mut()
        .ok_or("No project is open: open or create one on the Dashboard")?;
    let actor = project
        .actor_mut(&actor_id)
        .ok_or("That actor isn't in this scene: pick one from the actor list")?;
    let (material, link) =
        asset::linked_mut(actor).ok_or("That actor's material doesn't follow a file")?;
    match path.filter(|p| !p.is_empty()) {
        Some(path) => link.overrides.retain(|o| *o != path),
        None => link.overrides.clear(),
    }
    let file = asset::read(&dir, &source_path(&index, link))?;
    *material = asset::rebuild(material, &file, link);
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `unlink-material`: keeps the actor's material as it is but forgets the file.
pub(crate) fn unlink_material(
    state: &SharedState,
    app: &AppHandle,
    actor_id: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let actor = s
        .project_mut()
        .and_then(|p| p.actor_mut(&actor_id))
        .ok_or("That actor isn't in this scene: pick one from the actor list")?;
    let mut found = false;
    for component in actor.components.iter_mut() {
        if let ActorComponent::Material { source, .. } = component {
            found = source.take().is_some();
        }
    }
    if !found {
        return Err("That actor's material doesn't follow a file".into());
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}
