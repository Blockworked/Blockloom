//! The analysis project a script editor opens: a `Cargo.toml` at the project
//! root plus the `blockloom` crate it points at, so rust-analyzer can index
//! what Play compiles directly with `rustc`.
//!
//! Play keeps the fast two-`rustc` build in [`super`], so everything here is
//! analysis-only and must never break a run: syncing rewrites two small
//! generated files, and diagnostics are best-effort - a missing toolchain is
//! an answer, not an error.

use super::abi;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the analysis crate lives inside a project folder. Dotted like the
/// build output it is, and out of the way of Play's own `.blockloom/build`.
pub const IDE_DIR: &str = ".blockloom/ide";
/// The assembled `blockloom` crate rust-analyzer indexes: [`abi`] plus the
/// API over it, as real source so `use blockloom::*` and `export!` resolve.
pub const IDE_CRATE_DIR: &str = ".blockloom/ide/blockloom";
/// The entry point an editor opens, next to `project.blockloom` and `assets/`.
pub const MANIFEST_FILE: &str = "Cargo.toml";

/// One script rust-analyzer should index, as a `[[test]]` target (see
/// [`root_manifest`] for why tests and not bins).
#[derive(Debug, Clone, PartialEq)]
pub struct IdeScript {
    /// The `[[bin]]` name, which is what rustc would accept as a crate name.
    pub name: String,
    /// Forward-slash path relative to the project folder.
    pub path: String,
}

/// What syncing the analysis project did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncReport {
    pub scripts: usize,
    pub manifest: String,
}

/// One error or warning pinned to a line, for the editor to show inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptDiagnostic {
    /// Forward-slash script path, as the project holds it.
    pub path: String,
    /// 1-based line and column where the diagnostic starts.
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// `"error"` or `"warning"`.
    pub level: String,
    pub message: String,
}

/// Whether this machine can build scripts at all, and with what.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolchainStatus {
    pub available: bool,
    pub rustc_version: Option<String>,
    pub rustc_error: Option<String>,
    pub cargo_version: Option<String>,
    pub cargo_error: Option<String>,
    /// What to tell somebody whose scripts won't build.
    pub help: String,
}

const INSTALL_HELP: &str = "Install a Rust toolchain from https://rustup.rs and reopen Blockloom. Blocks still run without one - only scripts need it.";

pub fn ide_crate_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".blockloom").join("ide").join("blockloom")
}

pub fn manifest_path(project_dir: &Path) -> PathBuf {
    project_dir.join(MANIFEST_FILE)
}

/// Every `.rs` file under `assets/scripts`, deepest first, as project-relative
/// forward-slash paths. Anything that isn't valid UTF-8 or climbs out is
/// skipped rather than failing the sync.
pub fn list_scripts(project_dir: &Path) -> Vec<String> {
    let root = super::scripts_dir(project_dir);
    let mut found = Vec::new();
    collect_scripts(&root, &root, &mut found);
    found.sort();
    found
}

fn collect_scripts(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_scripts(root, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && let Ok(relative) = path.strip_prefix(root)
            && let Some(rest) = relative.to_str()
        {
            let rest = rest.replace('\\', "/");
            let candidate = format!("{}/{rest}", super::SCRIPTS_DIR);
            if super::is_valid_path(&candidate) {
                out.push(candidate);
            }
        }
    }
}

/// A `[[bin]]` name per script, deduplicated: two files that sanitize to the
/// same stem get `_2`, `_3`, like [`super::unused_path`] does for paths.
pub fn ide_scripts(project_dir: &Path) -> Vec<IdeScript> {
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    list_scripts(project_dir)
        .into_iter()
        .map(|path| {
            let mut name = unique_crate_name(&path, &mut used);
            if name.is_empty() {
                name = "script".to_string();
            }
            IdeScript { name, path }
        })
        .collect()
}

fn unique_crate_name(relative: &str, used: &mut std::collections::HashSet<String>) -> String {
    let stem = relative
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(relative)
        .trim_end_matches(".rs");
    let base: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let base = base.trim_matches('_').to_lowercase();
    let base = if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
        format!("script_{base}")
    } else {
        base
    };
    if used.insert(base.clone()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|candidate| used.insert(candidate.clone()))
        .expect("a unique crate name always exists")
}

/// A package name Cargo accepts, from the project folder's name.
fn package_name(project_dir: &Path) -> String {
    let base = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("blockloom-project");
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().to_string()
            } else {
                "-".to_string()
            }
        })
        .collect();
    name = name.trim_matches('-').to_string();
    if name.is_empty() {
        name = "blockloom-project".to_string();
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name = format!("project-{name}");
    }
    name
}

/// The root manifest: one `[[test]]` per script plus a path dependency on the
/// assembled `blockloom` crate, so `use blockloom::*` and `export!` resolve
/// against real source. Tests rather than bins because a script is a `cdylib`
/// root with `export!` entry points and no `fn main`, which a bin target
/// would flag on every file. A `[workspace]` of its own keeps Cargo from
/// looking for one above the project folder.
fn root_manifest(project_dir: &Path, scripts: &[IdeScript]) -> String {
    let mut manifest = format!(
        "# Generated by Blockloom for rust-analyzer. Analysis only - Play still\n\
         # compiles scripts directly with rustc. Regenerated whenever scripts change.\n\
         # Scripts are test targets because a bin target would demand a `fn main`\n\
         # no script has; `cargo check --tests` is what checks them.\n\
         [package]\n\
         name = \"{}-scripts\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\n\
         [workspace]\n\n\
         [dependencies]\n\
         blockloom = {{ path = \".blockloom/ide/blockloom\" }}\n",
        package_name(project_dir)
    );
    for script in scripts {
        manifest.push_str(&format!(
            "\n[[test]]\nname = \"{}\"\npath = \"{}\"\n",
            script.name, script.path
        ));
    }
    manifest
}

fn ide_crate_manifest() -> String {
    format!(
        "# Generated by Blockloom: the script API as real source for rust-analyzer.\n\
         [package]\n\
         name = \"blockloom\"\n\
         version = \"0.0.{}\"\n\
         edition = \"2024\"\n\n\
         [lib]\n\
         path = \"src/lib.rs\"\n",
        abi::ABI_VERSION
    )
}

/// Writes the analysis project: the assembled `blockloom` crate plus the root
/// manifest naming every script. Best-effort by design - a project that can't
/// write two small files still plays.
pub fn sync_ide_project(project_dir: &Path) -> Result<SyncReport, String> {
    let scripts = ide_scripts(project_dir);
    let crate_dir = ide_crate_dir(project_dir);
    let src_dir = crate_dir.join("src");
    std::fs::create_dir_all(&src_dir).map_err(|e| format!("{}: {e}", src_dir.display()))?;
    let lib = src_dir.join("lib.rs");
    std::fs::write(&lib, super::PRELUDE_SOURCE).map_err(|e| format!("{}: {e}", lib.display()))?;
    let crate_manifest = crate_dir.join("Cargo.toml");
    std::fs::write(&crate_manifest, ide_crate_manifest())
        .map_err(|e| format!("{}: {e}", crate_manifest.display()))?;
    let manifest = manifest_path(project_dir);
    std::fs::write(&manifest, root_manifest(project_dir, &scripts))
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    Ok(SyncReport {
        scripts: scripts.len(),
        manifest: MANIFEST_FILE.to_string(),
    })
}

/// Whether this machine can compile scripts, and with what. Missing is a
/// status, not a failure: blocks still run, only scripts need it.
pub fn toolchain_status() -> ToolchainStatus {
    let (rustc_version, rustc_error) = match super::toolchain_version() {
        Ok(version) => (Some(version), None),
        Err(error) => (None, Some(error)),
    };
    let (cargo_version, cargo_error) = match cargo_version() {
        Ok(version) => (Some(version), None),
        Err(error) => (None, Some(error)),
    };
    ToolchainStatus {
        available: rustc_version.is_some(),
        rustc_version,
        rustc_error,
        cargo_version,
        cargo_error,
        help: INSTALL_HELP.to_string(),
    }
}

fn cargo_version() -> Result<String, String> {
    let output = Command::new("cargo")
        .arg("--version")
        .output()
        .map_err(|e| format!("`cargo` couldn't be run ({e}). {INSTALL_HELP}"))?;
    if !output.status.success() {
        return Err("`cargo --version` failed".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Diagnostics for one script, newest write first: `cargo check` over the
/// analysis project when Cargo is here, else one `rustc` run with JSON errors.
/// Either way a missing toolchain is an empty answer, not an error - the run
/// log already says what's missing.
pub fn diagnostics_for(project_dir: &Path, relative: &str) -> Vec<ScriptDiagnostic> {
    let _ = sync_ide_project(project_dir);
    if cargo_version().is_ok() && manifest_path(project_dir).is_file() {
        let checked = diagnostics_via_cargo(project_dir, relative);
        if !checked.is_empty() || toolchain_status().cargo_version.is_some() {
            return checked;
        }
    }
    diagnostics_via_rustc(project_dir, relative).unwrap_or_default()
}

fn diagnostics_via_cargo(project_dir: &Path, relative: &str) -> Vec<ScriptDiagnostic> {
    let output = Command::new("cargo")
        .arg("check")
        .arg("--message-format=json")
        .arg("--tests")
        .current_dir(project_dir)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut diagnostics = Vec::new();
    for line in stdout.lines() {
        diagnostics.extend(cargo_message_diagnostics(line, relative));
    }
    diagnostics.sort_by_key(|d| (d.line, d.column));
    diagnostics
}

fn cargo_message_diagnostics(line: &str, relative: &str) -> Vec<ScriptDiagnostic> {
    let message: serde_json::Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(_) => return Vec::new(),
    };
    if message.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
        return Vec::new();
    }
    let Some(detail) = message.get("message") else {
        return Vec::new();
    };
    let level = detail
        .get("level")
        .and_then(|l| l.as_str())
        .unwrap_or("error");
    if level != "error" && level != "warning" {
        return Vec::new();
    }
    let text = detail
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("something is wrong here");
    let code = detail
        .get("code")
        .and_then(|c| c.get("code"))
        .and_then(|c| c.as_str())
        .map(|c| format!("[{c}] "))
        .unwrap_or_default();
    let mut out = Vec::new();
    let spans = detail.get("spans").and_then(|s| s.as_array());
    let empty: Vec<serde_json::Value> = Vec::new();
    for span in spans.unwrap_or(&empty) {
        let file = span
            .get("file_name")
            .and_then(|f| f.as_str())
            .unwrap_or_default();
        if !span.is_object() || span.get("is_primary").and_then(|p| p.as_bool()) != Some(true) {
            continue;
        }
        if !script_file_matches(file, relative) {
            continue;
        }
        out.push(ScriptDiagnostic {
            path: relative.to_string(),
            line: span.get("line_start").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            column: span
                .get("column_start")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u32,
            end_line: span.get("line_end").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            end_column: span.get("column_end").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            level: level.to_string(),
            message: format!("{code}{text}"),
        });
    }
    out
}

/// Whether a span's file is this script: Cargo reports an absolute path while
/// the project holds a relative one, so match on the tail.
fn script_file_matches(file_name: &str, relative: &str) -> bool {
    let file = file_name.replace('\\', "/");
    let relative = relative.replace('\\', "/");
    file == relative || file.ends_with(&format!("/{relative}"))
}

fn diagnostics_via_rustc(
    project_dir: &Path,
    relative: &str,
) -> Result<Vec<ScriptDiagnostic>, String> {
    if !super::is_valid_path(relative) {
        return Err(format!("\"{relative}\" isn't a script path"));
    }
    let source = super::source_path(project_dir, relative);
    if !source.is_file() {
        return Ok(Vec::new());
    }
    let build = super::build_dir(project_dir);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;
    let rlib = super::compile(project_dir, relative).ok();
    let mut command = Command::new("rustc");
    command
        .arg("--edition")
        .arg("2024")
        .arg("--crate-type")
        .arg("lib")
        .arg("--emit=metadata")
        .arg("--error-format=json")
        .arg("--out-dir")
        .arg(&build);
    if let Some(rlib) = rlib {
        command
            .arg("--extern")
            .arg(format!("blockloom={}", rlib.display()));
    }
    command.arg(&source);
    let output = command
        .output()
        .map_err(|e| format!("couldn't run rustc: {e}"))?;
    if output.status.success() {
        return Ok(Vec::new());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(parse_rustc_json_diagnostics(&stderr, relative))
}

fn parse_rustc_json_diagnostics(stderr: &str, relative: &str) -> Vec<ScriptDiagnostic> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let detail: serde_json::Value = match serde_json::from_str(line) {
            Ok(detail) => detail,
            Err(_) => continue,
        };
        let level = detail
            .get("level")
            .and_then(|l| l.as_str())
            .unwrap_or("error");
        if level != "error" && level != "warning" {
            continue;
        }
        let text = detail
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("something is wrong here");
        let code = detail
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str())
            .map(|c| format!("[{c}] "))
            .unwrap_or_default();
        let spans = detail.get("spans").and_then(|s| s.as_array());
        let empty: Vec<serde_json::Value> = Vec::new();
        let mut pushed = false;
        for span in spans.unwrap_or(&empty) {
            if span.get("is_primary").and_then(|p| p.as_bool()) != Some(true) {
                continue;
            }
            pushed = true;
            out.push(ScriptDiagnostic {
                path: relative.to_string(),
                line: span.get("line_start").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                column: span
                    .get("column_start")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32,
                end_line: span.get("line_end").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                end_column: span.get("column_end").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                level: level.to_string(),
                message: format!("{code}{text}"),
            });
        }
        if !pushed {
            out.push(ScriptDiagnostic {
                path: relative.to_string(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
                level: level.to_string(),
                message: format!("{code}{text}"),
            });
        }
    }
    out.sort_by_key(|d| (d.line, d.column));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "blockloom-ide-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(super::super::scripts_dir(&dir)).unwrap();
        dir
    }

    #[test]
    fn only_scripts_are_listed() {
        let dir = temp_project("list");
        std::fs::write(dir.join("assets/scripts/player.rs"), b"// rust").unwrap();
        std::fs::create_dir_all(dir.join("assets/scripts/enemies")).unwrap();
        std::fs::write(dir.join("assets/scripts/enemies/chaser.rs"), b"// rust").unwrap();
        std::fs::write(dir.join("assets/scripts/notes.txt"), b"nope").unwrap();

        let scripts = list_scripts(&dir);
        assert_eq!(
            scripts,
            vec![
                "assets/scripts/enemies/chaser.rs".to_string(),
                "assets/scripts/player.rs".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn syncing_writes_a_manifest_pointing_at_every_script() {
        let dir = temp_project("sync");
        std::fs::write(dir.join("assets/scripts/player.rs"), b"// rust").unwrap();
        std::fs::write(dir.join("assets/scripts/My Actor!.rs"), b"// rust").unwrap();

        let report = sync_ide_project(&dir).unwrap();
        assert_eq!(report.scripts, 2);

        let manifest = std::fs::read_to_string(manifest_path(&dir)).unwrap();
        assert!(manifest.contains("[workspace]"), "{manifest}");
        assert!(
            manifest.contains("path = \".blockloom/ide/blockloom\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("path = \"assets/scripts/player.rs\""),
            "{manifest}"
        );
        assert!(manifest.contains("name = \"my_actor\""), "{manifest}");

        let lib = std::fs::read_to_string(ide_crate_dir(&dir).join("src/lib.rs")).unwrap();
        assert!(lib.contains("pub struct Actor"), "{lib}");
        assert!(lib.contains("macro_rules! export"), "{lib}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clashing_stems_get_their_own_targets() {
        let dir = temp_project("clash");
        std::fs::create_dir_all(dir.join("assets/scripts/a")).unwrap();
        std::fs::create_dir_all(dir.join("assets/scripts/b")).unwrap();
        std::fs::write(dir.join("assets/scripts/a/boss.rs"), b"// rust").unwrap();
        std::fs::write(dir.join("assets/scripts/b/boss.rs"), b"// rust").unwrap();

        let scripts = ide_scripts(&dir);
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["boss", "boss_2"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cargo_messages_for_other_files_are_ignored() {
        let line = serde_json::json!({
            "reason": "compiler-message",
            "message": {
                "message": "cannot find value `foo`",
                "level": "error",
                "spans": [
                    {"file_name": "/tmp/demo/assets/scripts/player.rs", "is_primary": true,
                     "line_start": 3, "line_end": 3, "column_start": 5, "column_end": 8},
                    {"file_name": "/tmp/demo/assets/scripts/other.rs", "is_primary": true,
                     "line_start": 1, "line_end": 1, "column_start": 1, "column_end": 2},
                ],
            }
        })
        .to_string();
        let found = cargo_message_diagnostics(&line, "assets/scripts/player.rs");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].column), (3, 5));
    }

    #[test]
    fn rustc_json_without_a_primary_span_still_pins_line_one() {
        let line =
            r#"{"message": "aborting due to previous error", "level": "error", "spans": []}"#;
        let found = parse_rustc_json_diagnostics(line, "assets/scripts/player.rs");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 1);
    }
}
