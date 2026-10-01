//! The project's two plugin files and how they are written.
//!
//! - `plugins.json` is what the author asked for: direct dependencies.
//! - `plugins.lock` is what that resolved to: every package, its source and
//!   its content hash, plus per-target artifact identities.
//! - `plugins.local.json` holds local-development overrides, kept out of the
//!   other two so they never travel with a shared project.
//!
//! Opening a project reads them; only an explicit install, update or removal
//! writes them.

use crate::source::Source;
use blockloom_plugin_api::versions::MANIFEST_FORMAT;
use blockloom_plugin_api::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const PLUGINS_FILE: &str = "plugins.json";
pub const LOCK_FILE: &str = "plugins.lock";
pub const LOCAL_FILE: &str = "plugins.local.json";
/// Where each change's previous files are kept, so it can be rolled back.
pub const HISTORY_DIR: &str = ".blockloom/plugin-history";

/// Writes `bytes` to `path` so a reader sees the old file or the new one,
/// never half of either.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

fn format_one() -> u32 {
    MANIFEST_FORMAT
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectDependency {
    pub version: VersionReq,
    /// Where to get it. Absent means the registries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Optional features of the plugin to switch on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

/// `plugins.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginsFile {
    #[serde(default = "format_one")]
    pub format: u32,
    #[serde(default)]
    pub plugins: BTreeMap<String, DirectDependency>,
    /// Registry name to folder, relative to the project.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub registries: BTreeMap<String, String>,
}

impl Default for PluginsFile {
    fn default() -> Self {
        Self {
            format: MANIFEST_FORMAT,
            plugins: BTreeMap::new(),
            registries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedPackage {
    pub id: String,
    pub version: Version,
    pub source: Source,
    /// The package's content hash.
    pub hash: String,
    /// Ids of the plugins this one depends on, in the graph.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Artifact sha256 by target triple (and `portable`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub targets: BTreeMap<String, String>,
    /// Component types a missing copy of this package would not block a
    /// build for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub editor_only: Vec<String>,
    /// Whether the package runs code (portable or native), so a missing copy
    /// stops Play even when no record names it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_code: bool,
    /// Loaded in place from a local-development override; not hash-checked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev: bool,
}

/// `plugins.lock`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockFile {
    #[serde(default = "format_one")]
    pub format: u32,
    #[serde(default)]
    pub packages: Vec<LockedPackage>,
}

impl Default for LockFile {
    fn default() -> Self {
        Self {
            format: MANIFEST_FORMAT,
            packages: Vec::new(),
        }
    }
}

impl LockFile {
    pub fn get(&self, id: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.id == id)
    }

    /// Every package that depends on `id`, directly.
    pub fn dependents(&self, id: &str) -> Vec<&LockedPackage> {
        self.packages
            .iter()
            .filter(|p| p.dependencies.iter().any(|d| d == id))
            .collect()
    }

    fn sorted(mut self) -> Self {
        self.packages.sort_by(|a, b| a.id.cmp(&b.id));
        self
    }
}

/// `plugins.local.json`: plugin id to a folder loaded in place.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalOverrides {
    #[serde(default)]
    pub overrides: BTreeMap<String, PathBuf>,
}

/// The plugin files of one project folder.
#[derive(Debug, Clone)]
pub struct ProjectPlugins {
    pub dir: PathBuf,
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

impl ProjectPlugins {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn plugins_path(&self) -> PathBuf {
        self.dir.join(PLUGINS_FILE)
    }

    pub fn lock_path(&self) -> PathBuf {
        self.dir.join(LOCK_FILE)
    }

    pub fn read_plugins(&self) -> Result<PluginsFile, String> {
        let file: PluginsFile = read_json(&self.plugins_path())?;
        if file.format > MANIFEST_FORMAT {
            return Err(format!(
                "{PLUGINS_FILE} is format {}, newer than this editor understands ({MANIFEST_FORMAT})",
                file.format
            ));
        }
        Ok(file)
    }

    pub fn read_lock(&self) -> Result<LockFile, String> {
        let file: LockFile = read_json(&self.lock_path())?;
        if file.format > MANIFEST_FORMAT {
            return Err(format!(
                "{LOCK_FILE} is format {}, newer than this editor understands ({MANIFEST_FORMAT})",
                file.format
            ));
        }
        Ok(file)
    }

    pub fn read_local(&self) -> Result<LocalOverrides, String> {
        read_json(&self.dir.join(LOCAL_FILE))
    }

    /// Whether this project uses plugins at all.
    pub fn exists(&self) -> bool {
        self.plugins_path().exists() || self.lock_path().exists()
    }

    fn history_dir(&self) -> PathBuf {
        self.dir.join(HISTORY_DIR)
    }

    fn history_numbers(&self) -> Vec<u32> {
        let mut numbers: Vec<u32> = fs::read_dir(self.history_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_string_lossy().parse().ok())
            .collect();
        numbers.sort_unstable();
        numbers
    }

    /// Every lock file in history plus the current one: what the cache must
    /// keep alive.
    pub fn history_locks(&self) -> Vec<LockFile> {
        self.history_numbers()
            .into_iter()
            .filter_map(|n| {
                let text =
                    fs::read_to_string(self.history_dir().join(n.to_string()).join(LOCK_FILE))
                        .ok()?;
                serde_json::from_str(&text).ok()
            })
            .collect()
    }

    pub fn history_len(&self) -> usize {
        self.history_numbers().len()
    }

    /// Replaces both files: the previous pair is snapshotted first, then the
    /// lock and `plugins.json` are renamed into place (the lock last, so a
    /// crash in between leaves a lock that [`crate::install`] sees is behind
    /// `plugins.json` and re-installs from, rather than a half-written file).
    pub fn commit(&self, plugins: &PluginsFile, lock: &LockFile) -> Result<(), String> {
        let lock = lock.clone().sorted();
        let next = self.history_numbers().last().map_or(1, |n| n + 1);
        let snapshot = self.history_dir().join(next.to_string());
        fs::create_dir_all(&snapshot).map_err(|e| e.to_string())?;
        for name in [PLUGINS_FILE, LOCK_FILE] {
            let from = self.dir.join(name);
            if from.exists() {
                fs::copy(&from, snapshot.join(name)).map_err(|e| format!("{name}: {e}"))?;
            }
        }
        let result = write_json(&self.plugins_path(), plugins)
            .and_then(|_| write_json(&self.lock_path(), &lock));
        if result.is_err() {
            let _ = fs::remove_dir_all(&snapshot);
        }
        result
    }

    /// Puts back the files from before the last [`commit`](Self::commit) and
    /// drops that snapshot. Answers `None` when there is nothing to undo.
    pub fn rollback(&self) -> Result<Option<(PluginsFile, LockFile)>, String> {
        let Some(last) = self.history_numbers().last().copied() else {
            return Ok(None);
        };
        let snapshot = self.history_dir().join(last.to_string());
        for name in [PLUGINS_FILE, LOCK_FILE] {
            let saved = snapshot.join(name);
            let target = self.dir.join(name);
            if saved.exists() {
                let bytes = fs::read(&saved).map_err(|e| e.to_string())?;
                write_atomic(&target, &bytes)?;
            } else if target.exists() {
                fs::remove_file(&target).map_err(|e| e.to_string())?;
            }
        }
        fs::remove_dir_all(&snapshot).map_err(|e| e.to_string())?;
        Ok(Some((self.read_plugins()?, self.read_lock()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locked(id: &str, deps: &[&str]) -> LockedPackage {
        LockedPackage {
            id: id.to_string(),
            version: Version::new(1, 0, 0),
            source: Source::Registry("main".to_string()),
            hash: "0".repeat(64),
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            targets: BTreeMap::new(),
            editor_only: vec![],
            has_code: false,
            dev: false,
        }
    }

    #[test]
    fn files_round_trip_and_missing_files_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectPlugins::new(dir.path());
        assert!(!project.exists());
        assert!(project.read_plugins().unwrap().plugins.is_empty());
        assert!(project.read_lock().unwrap().packages.is_empty());

        let mut plugins = PluginsFile::default();
        plugins.plugins.insert(
            "com.example.a".into(),
            DirectDependency {
                version: "^1".parse().unwrap(),
                source: Some(Source::Path("../a".into())),
                features: vec!["fx".into()],
            },
        );
        let lock = LockFile {
            packages: vec![
                locked("com.example.b", &[]),
                locked("com.example.a", &["com.example.b"]),
            ],
            ..LockFile::default()
        };
        project.commit(&plugins, &lock).unwrap();
        assert_eq!(project.read_plugins().unwrap(), plugins);
        let read = project.read_lock().unwrap();
        assert_eq!(
            read.packages[0].id, "com.example.a",
            "sorted for stable diffs"
        );
        assert_eq!(read.dependents("com.example.b").len(), 1);
    }

    #[test]
    fn newer_formats_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(LOCK_FILE),
            r#"{"format": 99, "packages": []}"#,
        )
        .unwrap();
        let error = ProjectPlugins::new(dir.path()).read_lock().unwrap_err();
        assert!(error.contains("newer"), "{error}");
    }

    #[test]
    fn commit_snapshots_and_rollback_restores() {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectPlugins::new(dir.path());
        assert!(project.rollback().unwrap().is_none());

        let first = LockFile {
            packages: vec![locked("com.example.a", &[])],
            ..LockFile::default()
        };
        project.commit(&PluginsFile::default(), &first).unwrap();
        let second = LockFile {
            packages: vec![locked("com.example.a", &[]), locked("com.example.b", &[])],
            ..LockFile::default()
        };
        project.commit(&PluginsFile::default(), &second).unwrap();
        assert_eq!(project.history_len(), 2);
        // The first change had no files before it, so only one lock was saved.
        assert_eq!(project.history_locks().len(), 1);

        let (_, back) = project.rollback().unwrap().unwrap();
        assert_eq!(back.packages.len(), 1);
        assert_eq!(project.history_len(), 1);
        // Rolling back the first change returns to a project without plugins.
        let (plugins, lock) = project.rollback().unwrap().unwrap();
        assert!(plugins.plugins.is_empty() && lock.packages.is_empty());
        assert!(!project.exists());
    }

    #[test]
    fn atomic_writes_leave_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/x.json");
        write_atomic(&path, b"1").unwrap();
        write_atomic(&path, b"2").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"2");
        assert_eq!(fs::read_dir(dir.path().join("sub")).unwrap().count(), 1);
    }
}
