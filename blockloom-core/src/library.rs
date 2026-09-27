//! The set of projects the Dashboard lists.
//!
//! A project folder can sit anywhere the user pointed the New Project dialog
//! at, so there is no directory to scan. Instead Blockloom remembers the
//! folders it has opened in `<data dir>/projects.json` and reads each one's
//! name and dimension back off disk, so a project renamed or moved outside the
//! app is never shown wrong. A folder that has gone away is dropped from the
//! list.

use crate::project;
use crate::scene::Mode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One project on the Dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub name: String,
    pub mode: Mode,
    /// Unix seconds, for the "newest first" order the Dashboard shows.
    pub opened_at: u64,
}

/// What `projects.json` holds: paths and when each was last opened. Names and
/// dimensions are not cached here - they come from the project itself.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    projects: Vec<Remembered>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Remembered {
    path: PathBuf,
    #[serde(default)]
    opened_at: u64,
}

/// Just enough of a project file to list it, so opening the Dashboard doesn't
/// build every document in the library. New folders hold an index over scene
/// assets; old ones hold embedded scenes - both read here.
#[derive(Deserialize)]
struct Meta {
    name: String,
    #[serde(default)]
    world: MetaWorld,
    #[serde(default)]
    scenes: Vec<MetaScene>,
    #[serde(default)]
    active_scene: String,
}

#[derive(Default, Deserialize)]
struct MetaScene {
    #[serde(default)]
    id: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    world: MetaWorld,
}

#[derive(Default, Deserialize)]
struct MetaWorld {
    #[serde(default)]
    mode: Mode,
}

fn registry_path() -> PathBuf {
    project::data_dir().join("projects.json")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

fn read_registry() -> Registry {
    let path = registry_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Registry::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!(
            "Ignoring an unreadable project list ({}): {e}",
            path.display()
        );
        Registry::default()
    })
}

fn write_registry(registry: &Registry) {
    let path = registry_path();
    let write = || -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(registry).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())
    };
    if let Err(e) = write() {
        tracing::warn!("Couldn't save the project list ({}): {e}", path.display());
    }
}

/// The name and dimension of the project in `dir`, without building the whole
/// document. New folders read the active scene's mode off the index; old ones
/// read the embedded world.
fn read_meta(dir: &Path) -> Option<(String, Mode)> {
    let path = project::project_file(dir);
    let text = std::fs::read_to_string(&path).ok()?;
    let meta: Meta = serde_json::from_str(&text)
        .map_err(|e| tracing::warn!("Skipping an unreadable project ({}): {e}", path.display()))
        .ok()?;
    if meta.scenes.is_empty() {
        return Some((meta.name, meta.world.mode));
    }
    // Index format: entries carry their mode, so the Dashboard needs no scene
    // file. Old embedded scenes carry a world each instead.
    if meta.scenes.iter().any(|s| !s.path.is_empty()) {
        let mode = meta
            .scenes
            .iter()
            .find(|s| s.id == meta.active_scene)
            .or(meta.scenes.first())
            .map(|s| s.mode)
            .unwrap_or_default();
        return Some((meta.name, mode));
    }
    let mode = meta
        .scenes
        .iter()
        .find(|s| s.id == meta.active_scene)
        .or(meta.scenes.first())
        .map(|s| s.world.mode)
        .unwrap_or(meta.world.mode);
    Some((meta.name, mode))
}

/// Every remembered project, most recently opened first. Folders that are
/// gone or unreadable are dropped, here and from the file.
pub fn list() -> Vec<ProjectEntry> {
    let mut registry = read_registry();
    let mut entries = Vec::with_capacity(registry.projects.len());
    let before = registry.projects.len();
    registry.projects.retain(|remembered| {
        let Some((name, mode)) = read_meta(&remembered.path) else {
            return false;
        };
        entries.push(ProjectEntry {
            path: remembered.path.clone(),
            name,
            mode,
            opened_at: remembered.opened_at,
        });
        true
    });
    if registry.projects.len() != before {
        write_registry(&registry);
    }
    entries.sort_by(|a, b| {
        b.opened_at
            .cmp(&a.opened_at)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Adds `dir` to the list, or moves it to the front if it is already there.
pub fn remember(dir: &Path) {
    let dir = canonical(dir);
    let mut registry = read_registry();
    registry.projects.retain(|p| p.path != dir);
    registry.projects.push(Remembered {
        path: dir,
        opened_at: now(),
    });
    write_registry(&registry);
}

/// Drops `dir` from the list, leaving the folder itself alone.
pub fn forget(dir: &Path) {
    let dir = canonical(dir);
    let mut registry = read_registry();
    let before = registry.projects.len();
    registry.projects.retain(|p| p.path != dir);
    if registry.projects.len() != before {
        write_registry(&registry);
    }
}

/// Points a remembered entry at `to` instead of `from`, keeping its place in
/// the list - what renaming a project does to its folder.
pub fn moved(from: &Path, to: &Path) {
    let (from, to) = (canonical(from), canonical(to));
    let mut registry = read_registry();
    match registry.projects.iter_mut().find(|p| p.path == from) {
        Some(entry) => entry.path = to,
        None => registry.projects.push(Remembered {
            path: to,
            opened_at: now(),
        }),
    }
    write_registry(&registry);
}

/// Absolute where possible, so the same folder reached two ways is one entry.
fn canonical(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// Moves projects saved before they were folders into folders of their own
/// under [`project::default_projects_dir`], and remembers them. Runs once at
/// startup; a second run finds nothing to do.
pub fn migrate_legacy_projects() {
    let legacy = project::legacy_projects_dir();
    let Ok(entries) = std::fs::read_dir(&legacy) else {
        return;
    };
    let files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext == project::PROJECT_EXTENSION)
        })
        .collect();
    if files.is_empty() {
        return;
    }

    let parent = project::default_projects_dir();
    for file in files {
        let project = match project::read_project(&file) {
            Ok(project) => project,
            Err(e) => {
                tracing::warn!("Leaving an unreadable project where it is: {e}");
                continue;
            }
        };
        match project::create_project(&project, &parent) {
            Ok(dir) => {
                remember(&dir);
                // Only now, so a failed write never loses the original.
                if let Err(e) = std::fs::remove_file(&file) {
                    tracing::warn!("Couldn't remove {}: {e}", file.display());
                }
                tracing::info!("Moved \"{}\" into {}", project.name, dir.display());
            }
            Err(e) => tracing::warn!("Couldn't move \"{}\" into a folder: {e}", project.name),
        }
    }
}
