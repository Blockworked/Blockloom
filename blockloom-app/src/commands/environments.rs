//! Environment asset commands: save a scene's look as a `.blockenv`, make a
//! scene follow one, push a scene's changes back into the file and drop
//! overrides. Scenes are rebuilt whenever the file changes.

use super::{auto_save, emit, lock, project_dir, push_undo, reindex, sync_runtime};
use crate::AppHandle;
use crate::state::{AppState, SharedState};
use blockloom_core::assets;
use blockloom_core::environment_asset as asset;
use blockloom_core::linked::AssetLink;
use blockloom_core::project::Project;
use blockloom_core::resources::{ResourceIndex, ResourceRef};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

fn source_path(index: &ResourceIndex, link: &AssetLink) -> String {
    let mut source = link.source.clone();
    source.resolve(index);
    source.path
}

/// Rebuilds every scene following `only` (or any file) from it. Answers how
/// many changed.
pub(crate) fn sync_scenes(project: &mut Project, dir: &Path, only: Option<&str>) -> usize {
    let index = ResourceIndex::load(dir);
    let mut cache: HashMap<String, Option<Value>> = HashMap::new();
    let mut changed = 0;
    for scene in &mut project.scenes {
        let Some(link) = scene.world.environment.clone() else {
            continue;
        };
        let path = source_path(&index, &link);
        if only.is_some_and(|only| only != path) {
            continue;
        }
        let Some(file) = cache
            .entry(path.clone())
            .or_insert_with(|| asset::read(dir, &path).ok())
        else {
            continue;
        };
        let mut next = asset::rebuild(&scene.world, file, &link);
        if let Some(link) = &mut next.environment
            && (link.source.path != path || link.source.id.is_empty())
        {
            link.source = ResourceRef::to(&index, &path);
        }
        if next != scene.world {
            scene.world = next;
            changed += 1;
        }
    }
    changed
}

/// Records overrides for every linked scene an edit just touched.
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
        let Some(before) = shadow.scene(&scene.id).map(|s| &s.world) else {
            continue;
        };
        if *before == scene.world || scene.world.environment.is_none() {
            continue;
        }
        let index = index.get_or_insert_with(|| ResourceIndex::load(&dir));
        let world = &mut scene.world;
        let Some(mut link) = world.environment.take() else {
            continue;
        };
        if let Ok(file) = asset::read(&dir, &source_path(index, &link)) {
            let mut was = before.clone();
            was.environment = None;
            asset::mark(&was, world, &file, &mut link);
        }
        world.environment = Some(link);
    }
}

fn checked_path(path: &str) -> Result<String, String> {
    let path = assets::normalize(path).ok_or("That isn't a path in this project")?;
    if !asset::is_environment(&path) {
        return Err(format!(
            "An environment's file ends in .{}",
            asset::EXTENSION
        ));
    }
    Ok(path)
}

fn new_path(dir: &Path, name: &str) -> String {
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
        "Environment".to_string()
    } else {
        stem.trim().to_string()
    };
    (1..)
        .map(|n| match n {
            1 => format!("assets/environments/{stem}.{}", asset::EXTENSION),
            n => format!("assets/environments/{stem} {n}.{}", asset::EXTENSION),
        })
        .find(|path| !dir.join(path).exists())
        .expect("an unused name always exists")
}

fn link_to(index: &ResourceIndex, path: &str) -> AssetLink {
    AssetLink {
        source: ResourceRef::to(index, path),
        overrides: Vec::new(),
        ..Default::default()
    }
}

/// `create-environment-asset`: saves the open scene's look as a file and
/// makes the scene follow it. Answers the path.
pub(crate) fn create_environment_asset(
    state: &SharedState,
    app: &AppHandle,
    path: Option<String>,
) -> Result<String, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let project = s.project().ok_or("No project is open")?;
    let path = match path.filter(|p| !p.trim().is_empty()) {
        Some(path) => checked_path(&path)?,
        None => new_path(&dir, &project.active_scene().name),
    };
    asset::write(&dir, &path, &asset::extract(&project.world))?;
    reindex(&dir);
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    project.world.environment = Some(link_to(&index, &path));
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(path)
}

/// `assign-environment`: makes the open scene follow an environment file,
/// taking its whole look.
pub(crate) fn assign_environment(
    state: &SharedState,
    app: &AppHandle,
    path: String,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let path = checked_path(&path)?;
    let file = asset::read(&dir, &path)?;
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    let mut world = asset::apply(&project.world, &file)?;
    world.environment = Some(link_to(&index, &path));
    project.world = world;
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `apply-environment`: writes the open scene's look into its file and
/// rebuilds every other scene following it. Answers how many changed.
pub(crate) fn apply_environment(state: &SharedState, app: &AppHandle) -> Result<usize, String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let project = s.project().ok_or("No project is open")?;
    let link = project
        .world
        .environment
        .clone()
        .ok_or("This scene doesn't follow an environment")?;
    let path = source_path(&ResourceIndex::load(&dir), &link);
    asset::write(&dir, &path, &asset::extract(&project.world))?;
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    if let Some(link) = &mut project.world.environment {
        link.overrides.clear();
    }
    let changed = sync_scenes(project, &dir, Some(&path));
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(changed)
}

/// `revert-environment`: drops one override (`path`) or all of them.
pub(crate) fn revert_environment(
    state: &SharedState,
    app: &AppHandle,
    path: Option<String>,
) -> Result<(), String> {
    let mut s = lock(state)?;
    let dir = project_dir(&s)?;
    let index = ResourceIndex::load(&dir);
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    let link = project
        .world
        .environment
        .as_mut()
        .ok_or("This scene doesn't follow an environment")?;
    match path.filter(|p| !p.is_empty()) {
        Some(path) => link.overrides.retain(|o| *o != path),
        None => link.overrides.clear(),
    }
    let link = link.clone();
    let file = asset::read(&dir, &source_path(&index, &link))?;
    project.world = asset::rebuild(&project.world, &file, &link);
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}

/// `unlink-environment`: keeps the scene's look but forgets the file.
pub(crate) fn unlink_environment(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let mut s = lock(state)?;
    push_undo(&mut s);
    let project = s.project_mut().ok_or("No project is open")?;
    if project.world.environment.take().is_none() {
        return Err("This scene doesn't follow an environment".into());
    }
    auto_save(&mut s);
    sync_runtime(&mut s);
    emit(app, &mut s);
    Ok(())
}
