//! The project list and who holds each project right now.

use crate::{
    Error, Hub, Result, STARTUP_GRACE, key, now, oplock::OperationLock, process, read_json,
    write_json,
};
use blockloom_core::{library, project::project_file, sync};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn resolve(path: &Path) -> PathBuf {
    let expanded = match path.to_str() {
        Some(text) if text == "~" || text.starts_with("~/") => dirs::home_dir()
            .map(|home| home.join(text.trim_start_matches('~').trim_start_matches('/')))
            .unwrap_or_else(|| path.to_path_buf()),
        _ => path.to_path_buf(),
    };
    std::fs::canonicalize(&expanded).unwrap_or(expanded)
}

impl Hub {
    /// A project's name, dimension and the editor it is bound to. Errors when
    /// the folder is not a readable project.
    pub fn project(&self, path: &Path) -> Result<Value> {
        let path = resolve(path);
        let (name, mode) = library::read_meta(&path)
            .ok_or_else(|| Error::new(self.unavailable(&path, "unreadable")))?;
        let mode = serde_json::to_value(mode).unwrap_or(json!("TwoD"));
        Ok(json!({
            "path": path.to_string_lossy(),
            "name": name,
            "mode": mode,
            "installation": blockloom_core::hub::bound_installation(&path),
        }))
    }

    fn unavailable(&self, path: &Path, error: &str) -> String {
        if !path.exists() {
            return "The folder is missing. Its drive may be disconnected.".into();
        }
        if !project_file(path).is_file() {
            return "The folder no longer holds a Blockloom project.".into();
        }
        format!("The project file couldn't be read: {error}")
    }

    /// Records of editors that are still running (`running.json`).
    pub fn running_editors(&self) -> Vec<Value> {
        let records = read_json(&self.root.join("running.json"))
            .ok()
            .and_then(|v| v.get("editors").and_then(|e| e.as_array().cloned()))
            .unwrap_or_default();
        records
            .into_iter()
            .filter(|r| {
                r["pid"]
                    .as_u64()
                    .is_some_and(|pid| process::is_same(pid as u32, r["token"].as_str()))
            })
            .collect()
    }

    /// Who holds a project, or `None` when it is idle. The owner lock is the
    /// truth; a launch record counts only while its editor is still starting
    /// and has not written one yet.
    pub fn project_owner(&self, project: &Path) -> Option<Value> {
        let records = self.running_editors();
        // Read leniently: the Hub needs only the pid, heartbeat and app.
        let lock_file = sync::lock_path(project);
        if lock_file.exists() {
            let lock = read_json(&lock_file).ok()?;
            let pid = lock["pid"].as_u64()? as u32;
            let heartbeat = lock["heartbeat"].as_f64().unwrap_or(0.0);
            if process::alive(pid) || now() - heartbeat < 30.0 {
                let installation = records
                    .iter()
                    .find(|r| r["pid"].as_u64() == Some(pid as u64))
                    .and_then(|r| r["installation"].as_str().map(str::to_string))
                    .or_else(|| self.presence_installation(pid));
                let app = lock["app"].as_str().unwrap_or("");
                return Some(json!({"pid": pid, "app": app, "installation": installation}));
            }
            // A lock nobody is behind holds nothing, whatever launch records say.
            return None;
        }
        let want = key(project);
        records
            .iter()
            .find(|r| {
                r["path"].as_str().is_some_and(|p| !p.is_empty() && key(Path::new(p)) == want)
                    && now() - r["launched_at"].as_f64().unwrap_or(0.0) < STARTUP_GRACE as f64
            })
            .map(|r| json!({"pid": r["pid"], "app": "editor", "installation": r["installation"], "starting": true}))
    }

    pub fn project_active(&self, project: &Path) -> bool {
        self.project_owner(project).is_some()
    }

    pub fn require_idle(&self, project: &Path) -> Result<()> {
        if self.project_active(project) {
            return Err(Error::new(
                "Close this project's editor or shell before changing its editor or opening it again",
            ));
        }
        Ok(())
    }

    /// Entries of the editor's project list, plus any older Hub-only list.
    fn remembered(&self) -> BTreeMap<String, Value> {
        let mut merged: BTreeMap<String, Value> = BTreeMap::new();
        for source in [self.registry.clone(), self.root.join("projects.json")] {
            let Ok(file) = read_json(&source) else {
                continue;
            };
            for item in file["projects"].as_array().cloned().unwrap_or_default() {
                let Some(path) = item["path"].as_str() else {
                    continue;
                };
                let k = key(Path::new(path));
                let newer = merged.get(&k).is_none_or(|old| {
                    item["opened_at"].as_u64().unwrap_or(0) > old["opened_at"].as_u64().unwrap_or(0)
                });
                if newer {
                    merged.insert(k, item);
                }
            }
        }
        merged
    }

    /// Every remembered project, available ones first, newest first.
    pub fn projects(&self) -> Result<Vec<Value>> {
        let mut rows: Vec<Value> = vec![];
        for item in self.remembered().values() {
            let path = item["path"].as_str().unwrap_or("").to_string();
            let mut row = Map::new();
            row.insert("path".into(), json!(path));
            match self.project(Path::new(&path)) {
                Ok(info) => {
                    let owner =
                        self.project_owner(Path::new(info["path"].as_str().unwrap_or(&path)));
                    for (k, v) in info.as_object().cloned().unwrap_or_default() {
                        row.insert(k, v);
                    }
                    row.insert("available".into(), json!(true));
                    row.insert("active".into(), json!(owner.is_some()));
                    row.insert("owner".into(), owner.unwrap_or(Value::Null));
                }
                Err(error) => {
                    let name = Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.clone());
                    row.insert("error".into(), json!(error.message));
                    row.insert("available".into(), json!(false));
                    row.insert("active".into(), json!(false));
                    row.insert("owner".into(), Value::Null);
                    row.insert("name".into(), json!(name));
                    row.insert("mode".into(), json!("TwoD"));
                    row.insert("installation".into(), Value::Null);
                }
            }
            row.insert(
                "opened_at".into(),
                json!(item["opened_at"].as_u64().unwrap_or(0)),
            );
            rows.push(Value::Object(row));
        }
        rows.sort_by(|a, b| {
            let name = |v: &Value| v["name"].as_str().unwrap_or("").to_lowercase();
            (a["available"] != true)
                .cmp(&(b["available"] != true))
                .then(b["opened_at"].as_u64().cmp(&a["opened_at"].as_u64()))
                .then(name(a).cmp(&name(b)))
        });
        Ok(rows)
    }

    /// Edits the editor's own list, folding in and retiring a Hub-only one.
    fn write_library(&self, mutate: impl FnOnce(&mut BTreeMap<String, Value>)) -> Result<()> {
        let mut merged = self.remembered();
        mutate(&mut merged);
        let list: Vec<Value> = merged.into_values().collect();
        write_json(&self.registry, &json!({"projects": list}))?;
        let _ = std::fs::remove_file(self.root.join("projects.json"));
        Ok(())
    }

    pub fn remember(&self, project: &Path) -> Result<Value> {
        let _lock = OperationLock::acquire(&self.root)?;
        let result = self.project(project)?;
        let path = result["path"].as_str().unwrap_or("").to_string();
        self.write_library(|projects| {
            projects
                .entry(key(Path::new(&path)))
                .or_insert_with(|| json!({"path": path, "opened_at": 0}));
        })?;
        Ok(result)
    }

    /// Removes a project from the shared list. The folder is never touched.
    pub fn forget(&self, project: &Path) -> Result<Value> {
        let path = resolve(project);
        let _lock = OperationLock::acquire(&self.root)?;
        let k = key(&path);
        self.write_library(|projects| {
            projects.remove(&k);
        })?;
        Ok(json!({"path": path.to_string_lossy()}))
    }

    /// Points a remembered project at the folder it moved to.
    pub fn relocate(&self, project: &Path, folder: &Path) -> Result<Value> {
        let old = key(&resolve(project));
        let _lock = OperationLock::acquire(&self.root)?;
        let result = self.project(folder)?;
        let path = result["path"].as_str().unwrap_or("").to_string();
        self.write_library(|projects| {
            let previous = projects.remove(&old).unwrap_or_else(|| json!({}));
            projects.insert(
                key(Path::new(&path)),
                json!({"opened_at": previous["opened_at"].as_u64().unwrap_or(0), "path": path}),
            );
        })?;
        Ok(result)
    }

    /// Selects an editor for a project; migrates no project data. With
    /// `backup`, snapshots it first when the editor changes.
    pub fn bind(&self, project: &Path, identity: &str, backup: bool) -> Result<Value> {
        let _lock = OperationLock::acquire(&self.root)?;
        self.installation(identity, true)?;
        let mut result = self.project(project)?;
        let path = PathBuf::from(result["path"].as_str().unwrap_or(""));
        self.require_idle(&path)?;
        if backup && result["installation"] != identity {
            let snapshot = self.backup_locked(&result)?;
            result["backup"] = snapshot;
            self.require_idle(&path)?;
        }
        crate::checkpoint()?;
        write_json(
            &path.join(".blockloom").join("hub.json"),
            &json!({"schema": 1, "installation": identity}),
        )?;
        result["installation"] = json!(identity);
        Ok(result)
    }
}
