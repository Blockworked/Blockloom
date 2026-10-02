//! Running a package's importers and build hooks.
//!
//! The host does all the file work, so a module needs no capability to be an
//! importer: it is handed a source file's bytes and answers with the files it
//! made. Those are written under `<source>.imported/` beside the source, and a
//! ledger in `.blockloom/imports.json` remembers what each source made, from
//! which source and dependency hashes, so a changed source or a missing output
//! is noticed and a re-import replaces exactly what the last one wrote.

use crate::module::CodeModule;
use crate::package::sha256_hex;
use blockloom_plugin_api::assets::{
    Produced, ProducedFile, Request, build_op, check_output_path, importer_op,
};
use blockloom_plugin_api::schema::{BuildHookSchema, ImporterSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where the ledger lives, relative to the project folder.
pub const LEDGER_FILE: &str = ".blockloom/imports.json";
/// The suffix of the folder an import writes into.
pub const OUTPUT_SUFFIX: &str = ".imported";
/// The largest source file an importer is handed.
pub const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;

/// The folder an import of `source` writes into, relative to the project.
pub fn output_folder(source: &str) -> String {
    format!("{source}{OUTPUT_SUFFIX}")
}

/// Whether `path` is somewhere an import wrote, which no importer is handed.
pub fn is_imported_output(path: &str) -> bool {
    path.split('/').any(|part| part.ends_with(OUTPUT_SUFFIX))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashedFile {
    pub path: String,
    pub hash: String,
}

/// What one source made, as the ledger keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    pub plugin: String,
    pub importer: String,
    pub source_hash: String,
    pub outputs: Vec<HashedFile>,
    #[serde(default)]
    pub dependencies: Vec<HashedFile>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub imports: BTreeMap<String, ImportRecord>,
}

impl Ledger {
    pub fn read(project_dir: &Path) -> Ledger {
        std::fs::read(project_dir.join(LEDGER_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write(&self, project_dir: &Path) -> Result<(), String> {
        let path = project_dir.join(LEDGER_FILE);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text).map_err(|e| format!("{}: {e}", temp.display()))?;
        std::fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Whether what a source made still matches what it was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ImportState {
    Fresh,
    /// The source changed since it was imported.
    SourceChanged,
    /// A file the import also read changed or went away.
    DependencyChanged {
        path: String,
    },
    /// A file the import wrote is gone.
    OutputMissing {
        path: String,
    },
    /// A file the import wrote was edited by hand since.
    OutputEdited {
        path: String,
    },
    /// The source is gone.
    SourceMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportStatus {
    pub source: String,
    pub plugin: String,
    pub importer: String,
    #[serde(flatten)]
    pub state: ImportState,
    pub outputs: Vec<String>,
    pub warnings: Vec<String>,
}

/// What an import produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Imported {
    pub source: String,
    pub plugin: String,
    pub importer: String,
    pub outputs: Vec<String>,
    pub warnings: Vec<String>,
}

/// A project-relative path under `assets/`: forward slashes, plain names.
pub fn check_project_path(path: &str) -> Result<(), String> {
    let bad = |why: &str| {
        Err(format!(
            "\"{path}\" isn't a file in this project's assets: {why}"
        ))
    };
    if !path.starts_with("assets/") {
        return bad("it must start with assets/");
    }
    if path.contains('\\') || path.contains(':') || path.contains("//") {
        return bad("use forward slashes and plain names");
    }
    if path.split('/').any(|part| {
        part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
    }) {
        return bad("a name is empty, dotted or has control characters");
    }
    Ok(())
}

fn hash_file(project_dir: &Path, relative: &str) -> Option<String> {
    std::fs::read(project_dir.join(relative))
        .ok()
        .map(|bytes| sha256_hex(&bytes))
}

/// The folder `relative` is inside, checked to really be inside the project:
/// a symlink out of it is refused rather than followed.
fn checked_destination(project_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = project_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", project_dir.display()))?;
    let target = project_dir.join(relative);
    let parent = target
        .parent()
        .ok_or_else(|| format!("{relative} has no folder"))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let inside = parent
        .canonicalize()
        .map_err(|e| format!("{}: {e}", parent.display()))?;
    if !inside.starts_with(&root) {
        return Err(format!("{relative} leads outside the project"));
    }
    if let Ok(meta) = std::fs::symlink_metadata(&target)
        && meta.file_type().is_symlink()
    {
        return Err(format!("{relative} is a link and won't be overwritten"));
    }
    Ok(target)
}

/// Runs `importer` over the project file `source` and writes what it makes.
/// `module` is the importer's plugin loaded. Replaces what an earlier import
/// of the same source wrote, and nothing else.
pub fn run_import(
    module: &mut CodeModule,
    plugin: &str,
    importer: &ImporterSchema,
    project_dir: &Path,
    project_name: &str,
    source: &str,
) -> Result<Imported, String> {
    check_project_path(source)?;
    if is_imported_output(source) {
        return Err(format!(
            "{source} was made by an import, so it isn't imported again"
        ));
    }
    let extension = Path::new(source)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !importer.handles(&extension) {
        return Err(format!(
            "{plugin}/{} doesn't take .{extension} files",
            importer.name
        ));
    }
    let full = project_dir.join(source);
    let meta = std::fs::metadata(&full).map_err(|e| format!("{source}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{source} isn't a file"));
    }
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "{source} is larger than the {} MiB an importer takes",
            MAX_SOURCE_BYTES / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(&full).map_err(|e| format!("{source}: {e}"))?;
    let request = Request {
        what: "importer".to_string(),
        name: importer.name.clone(),
        path: source.to_string(),
        extension,
        project: project_name.to_string(),
        ..Default::default()
    };
    let answer = module
        .call_bytes(
            &importer_op(&importer.name),
            &request.encode(&bytes),
            importer.limit_ms,
        )
        .map_err(|e| format!("{plugin}/{}: {e}", importer.name))?;
    let produced =
        Produced::decode(&answer).map_err(|e| format!("{plugin}/{}: {e}", importer.name))?;
    if !produced.errors.is_empty() {
        return Err(format!(
            "{plugin}/{} couldn't import {source}: {}",
            importer.name,
            produced.errors.join("; ")
        ));
    }

    let mut dependencies = Vec::new();
    for dependency in &produced.dependencies {
        check_project_path(dependency)?;
        let hash = hash_file(project_dir, dependency).ok_or_else(|| {
            format!(
                "{plugin}/{} read {dependency}, which isn't there",
                importer.name
            )
        })?;
        dependencies.push(HashedFile {
            path: dependency.clone(),
            hash,
        });
    }

    let folder = output_folder(source);
    let mut ledger = Ledger::read(project_dir);
    let old: Vec<HashedFile> = ledger
        .imports
        .get(source)
        .map(|record| record.outputs.clone())
        .unwrap_or_default();
    let mut outputs = Vec::new();
    for ProducedFile { path, data } in &produced.files {
        let relative = format!("{folder}/{path}");
        let destination = checked_destination(project_dir, &relative)?;
        std::fs::write(&destination, data).map_err(|e| format!("{relative}: {e}"))?;
        outputs.push(HashedFile {
            path: relative,
            hash: sha256_hex(data),
        });
    }
    // What the last import wrote and this one didn't is stale now.
    for gone in old
        .iter()
        .filter(|o| !outputs.iter().any(|n| n.path == o.path))
    {
        let _ = std::fs::remove_file(project_dir.join(&gone.path));
    }
    prune_empty_dirs(project_dir, &folder);

    ledger.imports.insert(
        source.to_string(),
        ImportRecord {
            plugin: plugin.to_string(),
            importer: importer.name.clone(),
            source_hash: sha256_hex(&bytes),
            outputs: outputs.clone(),
            dependencies,
            warnings: produced.warnings.clone(),
        },
    );
    ledger.write(project_dir)?;
    Ok(Imported {
        source: source.to_string(),
        plugin: plugin.to_string(),
        importer: importer.name.clone(),
        outputs: outputs.into_iter().map(|o| o.path).collect(),
        warnings: produced.warnings,
    })
}

/// Removes the folders under `folder` that nothing is left in.
fn prune_empty_dirs(project_dir: &Path, folder: &str) {
    fn walk(path: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(path) else {
            return false;
        };
        let mut empty = true;
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() && walk(&child) {
                continue;
            }
            empty = false;
        }
        empty && std::fs::remove_dir(path).is_ok()
    }
    walk(&project_dir.join(folder));
}

/// How each remembered import stands against the disk now.
pub fn status(project_dir: &Path) -> Vec<ImportStatus> {
    Ledger::read(project_dir)
        .imports
        .into_iter()
        .map(|(source, record)| {
            let state = state_of(project_dir, &source, &record);
            ImportStatus {
                source,
                plugin: record.plugin,
                importer: record.importer,
                state,
                outputs: record.outputs.into_iter().map(|o| o.path).collect(),
                warnings: record.warnings,
            }
        })
        .collect()
}

fn state_of(project_dir: &Path, source: &str, record: &ImportRecord) -> ImportState {
    let Some(hash) = hash_file(project_dir, source) else {
        return ImportState::SourceMissing;
    };
    if hash != record.source_hash {
        return ImportState::SourceChanged;
    }
    for dependency in &record.dependencies {
        if hash_file(project_dir, &dependency.path).as_deref() != Some(dependency.hash.as_str()) {
            return ImportState::DependencyChanged {
                path: dependency.path.clone(),
            };
        }
    }
    for output in &record.outputs {
        match hash_file(project_dir, &output.path) {
            None => {
                return ImportState::OutputMissing {
                    path: output.path.clone(),
                };
            }
            Some(hash) if hash != output.hash => {
                return ImportState::OutputEdited {
                    path: output.path.clone(),
                };
            }
            Some(_) => {}
        }
    }
    ImportState::Fresh
}

/// Forgets what `source` made and deletes the files it wrote, for a source
/// the user removed.
pub fn forget(project_dir: &Path, source: &str) -> Result<usize, String> {
    let mut ledger = Ledger::read(project_dir);
    let Some(record) = ledger.imports.remove(source) else {
        return Ok(0);
    };
    for output in &record.outputs {
        let _ = std::fs::remove_file(project_dir.join(&output.path));
    }
    prune_empty_dirs(project_dir, &output_folder(source));
    ledger.write(project_dir)?;
    Ok(record.outputs.len())
}

/// What one build hook answered: files to add to the game, and warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRun {
    pub plugin: String,
    pub hook: String,
    pub warnings: Vec<String>,
    pub files: Vec<ProducedFile>,
}

/// Every file under the project's `assets/`, relative to the project and
/// sorted, skipping dotted names. At most `limit` of them.
pub fn list_assets(project_dir: &Path, limit: usize) -> Vec<String> {
    fn walk(root: &Path, at: &Path, out: &mut Vec<String>, limit: usize) {
        let Ok(entries) = std::fs::read_dir(at) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if out.len() >= limit {
                return;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                walk(root, &path, out, limit);
            } else if kind.is_file()
                && let Ok(relative) = path.strip_prefix(root)
            {
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(project_dir, &project_dir.join("assets"), &mut out, limit);
    out
}

/// Runs a build hook for `target`. An error from the module, or a non-empty
/// `errors` in its answer, fails the hook.
pub fn run_build_hook(
    module: &mut CodeModule,
    plugin: &str,
    hook: &BuildHookSchema,
    project_dir: &Path,
    project_name: &str,
    target: &str,
) -> Result<HookRun, String> {
    let request = Request {
        what: "build".to_string(),
        name: hook.name.clone(),
        target: target.to_string(),
        project: project_name.to_string(),
        assets: list_assets(project_dir, 100_000),
        ..Default::default()
    };
    let answer = module
        .call_bytes(&build_op(&hook.name), &request.encode(&[]), hook.limit_ms)
        .map_err(|e| format!("{plugin}/{}: {e}", hook.name))?;
    let produced = Produced::decode(&answer).map_err(|e| format!("{plugin}/{}: {e}", hook.name))?;
    if !produced.errors.is_empty() {
        return Err(format!(
            "{plugin}/{} stopped the build: {}",
            hook.name,
            produced.errors.join("; ")
        ));
    }
    for file in &produced.files {
        check_output_path(&file.path)?;
    }
    Ok(HookRun {
        plugin: plugin.to_string(),
        hook: hook.name.clone(),
        warnings: produced.warnings,
        files: produced.files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_paths_stay_under_assets() {
        for bad in [
            "x.png",
            "assets/../x",
            "assets//x",
            "assets/a\\b",
            "/assets/x",
            "assets/./x",
            "assets/C:x",
            "other/x",
        ] {
            assert!(check_project_path(bad).is_err(), "{bad}");
        }
        assert!(check_project_path("assets/sprites/a b.png").is_ok());
        assert!(is_imported_output("assets/a.gpl.imported/palette.png"));
        assert!(!is_imported_output("assets/a.gpl"));
    }

    #[test]
    fn a_ledger_round_trips_and_status_follows_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("assets/p.gpl.imported")).unwrap();
        std::fs::write(dir.path().join("assets/p.gpl"), b"source").unwrap();
        std::fs::write(dir.path().join("assets/p.gpl.imported/out.png"), b"made").unwrap();
        let mut ledger = Ledger::default();
        ledger.imports.insert(
            "assets/p.gpl".to_string(),
            ImportRecord {
                plugin: "com.example.p".to_string(),
                importer: "gpl".to_string(),
                source_hash: sha256_hex(b"source"),
                outputs: vec![HashedFile {
                    path: "assets/p.gpl.imported/out.png".to_string(),
                    hash: sha256_hex(b"made"),
                }],
                dependencies: Vec::new(),
                warnings: Vec::new(),
            },
        );
        ledger.write(dir.path()).unwrap();
        assert_eq!(Ledger::read(dir.path()), ledger);
        assert_eq!(status(dir.path())[0].state, ImportState::Fresh);

        std::fs::write(dir.path().join("assets/p.gpl.imported/out.png"), b"hand").unwrap();
        assert!(matches!(
            status(dir.path())[0].state,
            ImportState::OutputEdited { .. }
        ));
        std::fs::remove_file(dir.path().join("assets/p.gpl.imported/out.png")).unwrap();
        assert!(matches!(
            status(dir.path())[0].state,
            ImportState::OutputMissing { .. }
        ));
        std::fs::write(dir.path().join("assets/p.gpl"), b"changed").unwrap();
        assert_eq!(status(dir.path())[0].state, ImportState::SourceChanged);
        std::fs::remove_file(dir.path().join("assets/p.gpl")).unwrap();
        assert_eq!(status(dir.path())[0].state, ImportState::SourceMissing);
        assert_eq!(forget(dir.path(), "assets/p.gpl").unwrap(), 1);
        assert!(status(dir.path()).is_empty());
    }

    #[test]
    fn assets_are_listed_sorted_without_dotted_names() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "assets/b.png",
            "assets/a/z.txt",
            "assets/a/.hidden",
            "assets/.cache/x",
        ] {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        assert_eq!(
            list_assets(dir.path(), 10),
            ["assets/a/z.txt", "assets/b.png"]
        );
        assert_eq!(list_assets(dir.path(), 1).len(), 1);
    }
}
