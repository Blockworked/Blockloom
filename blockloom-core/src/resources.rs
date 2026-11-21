//! The resource database: a stable id for every file under a project's
//! `assets/`, kept in `assets.index` beside `project.blockloom`, plus the authored
//! import choices (role, exposure bias) that used to live by path in
//! `.blockloom/pipeline.json`. A rename or move keeps the id, so a
//! [`ResourceRef`] finds its file again, and [`uses`] answers "used by".

use crate::files;
use crate::pipeline::ImportRole;
use crate::project::Project;
use blockloom_schema_derive::Schema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub const INDEX_FILE: &str = "assets.index";
const INDEX_FORMAT: u32 = 1;
const ASSETS: &str = "assets";

/// One file the index knows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceEntry {
    pub id: String,
    /// What it imports as, when the author chose over its extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ImportRole>,
    /// Stops an HDR file is scaled by when decoded.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub exposure_bias: f32,
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

/// Every indexed file by project-relative path, sorted so diffs stay small.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceIndex {
    #[serde(default = "index_format")]
    pub format: u32,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceEntry>,
}

fn index_format() -> u32 {
    INDEX_FORMAT
}

impl Default for ResourceIndex {
    fn default() -> Self {
        Self {
            format: INDEX_FORMAT,
            resources: BTreeMap::new(),
        }
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// `path` itself or anything under it when it is a folder.
fn under(path: &str, root: &str) -> bool {
    path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
}

/// Whether a file belongs in the index: under `assets/`, not a scene
/// (scenes carry their own ids) and not generated import output.
fn indexable(relative: &str) -> bool {
    if !relative.starts_with(&format!("{ASSETS}/")) {
        return false;
    }
    if crate::project::is_scene_asset(relative) {
        return false;
    }
    !relative
        .split('/')
        .any(|part| part.starts_with('.') || part.ends_with(".imported"))
}

/// Every indexable file under `dir`, project-relative.
fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![ASSETS.to_string()];
    while let Some(relative) = stack.pop() {
        let Ok(read) = std::fs::read_dir(dir.join(&relative)) else {
            continue;
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = format!("{relative}/{name}");
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if !path.ends_with(".imported") {
                        stack.push(path);
                    }
                }
                Ok(kind) if kind.is_file() && indexable(&path) => out.push(path),
                _ => {}
            }
        }
    }
    out
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl ResourceIndex {
    pub fn load(dir: &Path) -> Self {
        crate::vfs::read_to_string(&dir.join(INDEX_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        files::write_atomic(&dir.join(INDEX_FILE), text)
    }

    /// Loads, runs `edit`, and writes the index back when it changed.
    pub fn update<R>(dir: &Path, edit: impl FnOnce(&mut Self) -> R) -> Result<R, String> {
        let mut index = Self::load(dir);
        let before = index.clone();
        let out = edit(&mut index);
        if index != before || !dir.join(INDEX_FILE).is_file() {
            index.save(dir)?;
        }
        Ok(out)
    }

    pub fn id_of(&self, path: &str) -> Option<&str> {
        self.resources.get(path).map(|entry| entry.id.as_str())
    }

    pub fn path_of(&self, id: &str) -> Option<&str> {
        self.resources
            .iter()
            .find(|(_, entry)| entry.id == id)
            .map(|(path, _)| path.as_str())
    }

    /// The entry for `path`, made with a fresh id if the index lacks it.
    pub fn entry(&mut self, path: &str) -> &mut ResourceEntry {
        self.resources
            .entry(path.to_string())
            .or_insert_with(|| ResourceEntry {
                id: new_id(),
                role: None,
                exposure_bias: 0.0,
            })
    }

    /// Brings the index in line with the folder: new files get ids, gone
    /// ones are dropped. A file that moved outside the editor keeps its id
    /// when exactly one vanished entry and one new file share its name.
    pub fn reconcile(&mut self, dir: &Path) -> bool {
        let on_disk = walk(dir);
        let present: std::collections::HashSet<&str> = on_disk.iter().map(String::as_str).collect();
        let gone: Vec<String> = self
            .resources
            .keys()
            // An import choice on a file outside `assets/` stays while it does.
            .filter(|path| {
                if indexable(path) {
                    !present.contains(path.as_str())
                } else {
                    !dir.join(path.as_str()).is_file()
                }
            })
            .cloned()
            .collect();
        let fresh: Vec<&String> = on_disk
            .iter()
            .filter(|path| !self.resources.contains_key(path.as_str()))
            .collect();
        if gone.is_empty() && fresh.is_empty() {
            return false;
        }
        let mut gone_by_name: HashMap<&str, Vec<&String>> = HashMap::new();
        for path in &gone {
            gone_by_name.entry(file_name(path)).or_default().push(path);
        }
        let mut fresh_by_name: HashMap<&str, usize> = HashMap::new();
        for path in &fresh {
            *fresh_by_name.entry(file_name(path)).or_default() += 1;
        }
        let mut entries: Vec<(String, ResourceEntry)> = Vec::new();
        for path in &fresh {
            let name = file_name(path);
            let moved = match gone_by_name.get(name) {
                Some(old) if old.len() == 1 && fresh_by_name[name] == 1 => {
                    self.resources.get(old[0].as_str()).cloned()
                }
                _ => None,
            };
            let entry = moved.unwrap_or_else(|| ResourceEntry {
                id: new_id(),
                role: None,
                exposure_bias: 0.0,
            });
            entries.push(((*path).clone(), entry));
        }
        for path in &gone {
            self.resources.remove(path);
        }
        self.resources.extend(entries);
        true
    }

    /// Follows an editor rename or move of a file or folder; ids stay.
    pub fn moved(&mut self, from: &str, to: &str) {
        let moving: Vec<String> = self
            .resources
            .keys()
            .filter(|path| under(path, from))
            .cloned()
            .collect();
        for path in moving {
            if let Some(entry) = self.resources.remove(&path) {
                let next = format!("{to}{}", &path[from.len()..]);
                if indexable(&next) {
                    self.resources.insert(next, entry);
                }
            }
        }
    }

    /// Forgets a deleted file or folder.
    pub fn removed(&mut self, path: &str) {
        self.resources.retain(|at, _| !under(at, path));
    }
}

/// A reference to an indexed file: the id finds it after a rename, the
/// path is what it was last seen as (and what an unindexed file goes by).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Schema)]
pub struct ResourceRef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[schema(skip)]
    pub id: String,
    #[serde(default)]
    #[schema(asset)]
    pub path: String,
}

impl ResourceRef {
    pub fn to(index: &ResourceIndex, path: &str) -> Self {
        Self {
            id: index.id_of(path).unwrap_or_default().to_string(),
            path: path.to_string(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.path.is_empty() && self.id.is_empty()
    }

    /// Repairs `path` from the id. Answers whether it changed.
    pub fn resolve(&mut self, index: &ResourceIndex) -> bool {
        if self.id.is_empty() {
            if let Some(id) = index.id_of(&self.path) {
                self.id = id.to_string();
            }
            return false;
        }
        match index.path_of(&self.id) {
            Some(path) if path != self.path => {
                self.path = path.to_string();
                true
            }
            _ => false,
        }
    }
}

/// Where a document names a file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Use {
    pub scene_id: String,
    pub scene: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// The field path inside the actor, scene world or project.
    pub at: String,
}

fn strings<'a>(value: &'a Value, at: &mut String, out: &mut Vec<(String, &'a str)>) {
    match value {
        Value::String(text) => out.push((at.clone(), text)),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let len = at.len();
                at.push_str(&format!(".{i}"));
                strings(item, at, out);
                at.truncate(len);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let len = at.len();
                if !at.is_empty() {
                    at.push('.');
                }
                at.push_str(key);
                strings(item, at, out);
                at.truncate(len);
            }
        }
        _ => {}
    }
}

/// Every place the document names a file, by the file's path. A string
/// counts when it is an indexed path or id (an id resolves to its path), so
/// a path typed into a block slot is found as well as a component's field.
pub fn uses(project: &Project, index: &ResourceIndex) -> BTreeMap<String, Vec<Use>> {
    let by_id: HashMap<&str, &str> = index
        .resources
        .iter()
        .map(|(path, entry)| (entry.id.as_str(), path.as_str()))
        .collect();
    let mut out: BTreeMap<String, Vec<Use>> = BTreeMap::new();
    let mut record = |value: &Value, make: &dyn Fn(String) -> Use| {
        let mut found = Vec::new();
        strings(value, &mut String::new(), &mut found);
        for (at, text) in found {
            let path = if index.resources.contains_key(text) {
                text
            } else if let Some(path) = by_id.get(text) {
                path
            } else {
                continue;
            };
            let found = make(at);
            // A ResourceRef names its file twice, by id and by path.
            let stem = |at: &str| {
                at.strip_suffix(".id")
                    .or_else(|| at.strip_suffix(".path"))
                    .unwrap_or(at)
                    .to_string()
            };
            let list = out.entry(path.to_string()).or_default();
            if !list.iter().any(|u| {
                u.scene_id == found.scene_id
                    && u.actor_id == found.actor_id
                    && stem(&u.at) == stem(&found.at)
            }) {
                list.push(found);
            }
        }
    };
    let default = project
        .scene(&project.default_scene)
        .or(project.scenes.first());
    let (scene_id, scene_name) = default
        .map(|s| (s.id.clone(), s.name.clone()))
        .unwrap_or_default();
    let project_use = |at: String| Use {
        scene_id: scene_id.clone(),
        scene: scene_name.clone(),
        actor_id: None,
        actor: None,
        at: format!("project.{at}"),
    };
    record(&Value::String(project.icon.clone()), &|_| project_use("icon".into()));
    if let Ok(settings) = serde_json::to_value(&project.settings) {
        record(&settings, &|at| project_use(format!("settings.{at}")));
    }
    for scene in &project.scenes {
        if let Ok(world) = serde_json::to_value(&scene.world) {
            record(&world, &|at| Use {
                scene_id: scene.id.clone(),
                scene: scene.name.clone(),
                actor_id: None,
                actor: None,
                at: format!("world.{at}"),
            });
        }
        for actor in &scene.actors {
            if let Ok(value) = serde_json::to_value(actor) {
                record(&value, &|at| Use {
                    scene_id: scene.id.clone(),
                    scene: scene.name.clone(),
                    actor_id: Some(actor.id.clone()),
                    actor: Some(actor.name.clone()),
                    at,
                });
            }
        }
    }
    out
}

/// The uses of `path`, or of everything under it when it is a folder.
pub fn uses_of(project: &Project, index: &ResourceIndex, path: &str) -> Vec<Use> {
    uses(project, index)
        .into_iter()
        .filter(|(at, _)| under(at, path))
        .flat_map(|(_, found)| found)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Mode;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("blockloom-res-{}", new_id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, relative: &str) {
            let full = self.0.join(relative);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, relative).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reconcile_keeps_ids_and_follows_a_move_by_name() {
        let dir = TempDir::new();
        dir.file("assets/a.png");
        dir.file("assets/b.wav");
        dir.file("assets/scenes/One.blockscene");
        dir.file("assets/x.gpl.imported/palette.png");
        dir.file(".blockloom/pipeline.json");
        let mut index = ResourceIndex::default();
        assert!(index.reconcile(&dir.0));
        let paths: Vec<&String> = index.resources.keys().collect();
        assert_eq!(paths, ["assets/a.png", "assets/b.wav"]);
        let a = index.id_of("assets/a.png").unwrap().to_string();
        assert!(!index.reconcile(&dir.0), "nothing changed");
        // Moved behind the editor's back.
        std::fs::create_dir_all(dir.0.join("assets/art")).unwrap();
        std::fs::rename(dir.0.join("assets/a.png"), dir.0.join("assets/art/a.png")).unwrap();
        assert!(index.reconcile(&dir.0));
        assert_eq!(index.id_of("assets/art/a.png"), Some(a.as_str()));
        assert_eq!(index.id_of("assets/a.png"), None);
    }

    #[test]
    fn a_folder_move_keeps_every_id_and_a_ref_repairs_its_path() {
        let mut index = ResourceIndex::default();
        let id = index.entry("assets/art/a.png").id.clone();
        index.entry("assets/artwork.png");
        let mut reference = ResourceRef::to(&index, "assets/art/a.png");
        index.moved("assets/art", "assets/pictures");
        assert_eq!(index.id_of("assets/pictures/a.png"), Some(id.as_str()));
        assert!(index.id_of("assets/artwork.png").is_some(), "only the folder moved");
        assert!(reference.resolve(&index));
        assert_eq!(reference.path, "assets/pictures/a.png");
        index.removed("assets/pictures");
        assert_eq!(index.resources.len(), 1);
    }

    #[test]
    fn uses_finds_fields_and_block_slots() {
        use crate::components::ActorComponent;
        use crate::scene::Visual;
        let mut project = Project::starter("Uses", Mode::TwoD);
        let mut index = ResourceIndex::default();
        index.entry("assets/hero.png");
        index.entry("assets/unused.png");
        let scene = project.active_scene_mut();
        let actor = &mut scene.actors[0];
        for component in actor.components.iter_mut() {
            if let ActorComponent::Look { visual } = component {
                *visual = Visual::Image {
                    path: "assets/hero.png".into(),
                    size: [32.0, 32.0],
                };
            }
        }
        let name = actor.name.clone();
        let found = uses(&project, &index);
        let hero = &found["assets/hero.png"];
        assert_eq!(hero.len(), 1);
        assert_eq!(hero[0].actor.as_deref(), Some(name.as_str()));
        assert!(hero[0].at.contains("path"), "{}", hero[0].at);
        assert!(!found.contains_key("assets/unused.png"));
        assert_eq!(uses_of(&project, &index, "assets").len(), 1);
    }
}
