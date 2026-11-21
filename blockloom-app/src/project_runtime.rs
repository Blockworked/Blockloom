//! Engine-tier (adapter) plugins: a project whose plugins carry engine
//! crates gets its own runtime, the stock one with those crates linked in,
//! built into `.blockloom/runtime/` and started as the editor's child.
//! It builds with Cargo against the engine's source tree; the install's
//! prebuilt engine rlibs replace that later.

use blockloom_plugin_api::manifest::Tier;
use blockloom_plugin_host::active::ActivePlugins;
use blockloom_plugin_host::source::Source;
use blockloom_plugin_host::trust::TrustLedger;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One plugin crate a project runtime links.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineCrate {
    pub plugin: String,
    pub dir: PathBuf,
    /// The crate's library name (`-` as `_`).
    pub lib: String,
    /// The package's content hash, which a trust grant and the build stamp name.
    pub hash: String,
    /// Installed from a folder on this machine (`path:`): the author's own code.
    pub local: bool,
}

/// The engine crates the active plugins carry, in plugin-id order.
pub fn engine_crates(active: &ActivePlugins) -> Result<Vec<EngineCrate>, String> {
    let mut crates = Vec::new();
    for (id, loaded) in &active.plugins {
        let manifest = &loaded.package.manifest;
        let Some(adapter) = manifest.runtime.adapter.as_ref() else {
            continue;
        };
        if manifest.tier != Tier::Adapter {
            continue;
        }
        let dir = loaded.package.root.join(&adapter.crate_dir);
        let lib = package_name(&dir.join("Cargo.toml"))
            .ok_or_else(|| format!("{id}: no package name in {}", dir.display()))?
            .replace('-', "_");
        crates.push(EngineCrate {
            plugin: id.clone(),
            dir,
            lib,
            hash: loaded.package.content_hash.clone(),
            local: matches!(loaded.locked.source, Source::Path(_)),
        });
    }
    Ok(crates)
}

fn package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let mut in_package = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && let Some(rest) = line.strip_prefix("name") {
            let value = rest.trim_start().strip_prefix('=')?.trim();
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// The engine's source tree: `BLOCKLOOM_ENGINE_SOURCE`, else the tree this
/// editor was built from.
pub fn engine_source() -> Option<PathBuf> {
    let dir = std::env::var_os("BLOCKLOOM_ENGINE_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(".."));
    dir.join("blockloom-engine/Cargo.toml")
        .exists()
        .then(|| dir.canonicalize().unwrap_or(dir))
}

/// An engine crate is compiled into the runtime with the editor's own access,
/// so one that came from elsewhere must be trusted at this exact content
/// first. A `path:` package is the author's own folder and needs none.
pub fn needs_trust(engine_crate: &EngineCrate, ledger: &TrustLedger) -> bool {
    !engine_crate.local && !ledger.is_trusted(&engine_crate.plugin, &engine_crate.hash)
}

/// What the runtime was last built from: plugin id to content hash.
fn stamp_path(project: &Path) -> PathBuf {
    crate_dir(project).join("bin/plugins.json")
}

fn stamp_of(crates: &[EngineCrate]) -> BTreeMap<String, String> {
    crates
        .iter()
        .map(|c| (c.plugin.clone(), c.hash.clone()))
        .collect()
}

/// Whether the built runtime is missing, or was built from other packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Built {
    Missing,
    Stale,
    Current,
}

pub fn built(project: &Path, crates: &[EngineCrate]) -> Built {
    if !binary(project).exists() {
        return Built::Missing;
    }
    let stamp = std::fs::read(stamp_path(project))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, String>>(&bytes).ok());
    if stamp.as_ref() == Some(&stamp_of(crates)) {
        Built::Current
    } else {
        Built::Stale
    }
}

/// Where a project's runtime is built and where its binary lands.
pub fn crate_dir(project: &Path) -> PathBuf {
    project.join(".blockloom/runtime")
}

pub fn binary(project: &Path) -> PathBuf {
    crate_dir(project)
        .join("bin")
        .join(blockloom_protocol::RUNTIME_BINARY)
}

/// The generated crate: a `main` that adds each plugin's module and runs
/// the world, and a manifest carrying the engine's patches and profiles.
pub fn generate(project: &Path, engine: &Path, crates: &[EngineCrate]) -> Result<PathBuf, String> {
    let dir = crate_dir(project);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let quote = |path: &Path| format!("{:?}", path.to_string_lossy());
    let mut manifest = format!(
        "# Generated by Blockloom for this project's engine plugins. Do not edit.\n\
         [package]\nname = \"blockloom-project-runtime\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
         [[bin]]\nname = \"blockloom-runtime\"\npath = \"main.rs\"\n\n\
         [dependencies]\nblockloom-engine = {{ path = {} }}\n",
        quote(&engine.join("blockloom-engine"))
    );
    for engine_crate in crates {
        manifest.push_str(&format!(
            "{} = {{ path = {} }}\n",
            engine_crate.lib.replace('_', "-"),
            quote(&engine_crate.dir)
        ));
    }
    manifest.push_str("\n[workspace]\n\n");
    manifest.push_str(&engine_tables(engine)?);
    let mut main = String::from(
        "// Generated by Blockloom for this project's engine plugins. Do not edit.\n\
         fn main() {\n    blockloom_engine::run(|| {\n",
    );
    for engine_crate in crates {
        main.push_str(&format!(
            "        blockloom_engine::add_engine_module({}::blockloom_module());\n",
            engine_crate.lib
        ));
    }
    main.push_str("    });\n}\n");
    blockloom_core::files::write_atomic(&dir.join("Cargo.toml"), manifest.as_bytes())
        .map_err(|e| e.to_string())?;
    blockloom_core::files::write_atomic(&dir.join("main.rs"), main.as_bytes())
        .map_err(|e| e.to_string())?;
    // The engine's own toolchain and lock, so the shared target's artifacts fit.
    if let Ok(toolchain) = std::fs::read(engine.join("rust-toolchain.toml")) {
        let _ = blockloom_core::files::replace_file(&dir.join("rust-toolchain.toml"), &toolchain);
    }
    if let Ok(lock) = std::fs::read(engine.join("Cargo.lock")) {
        let _ = blockloom_core::files::replace_file(&dir.join("Cargo.lock"), &lock);
    }
    Ok(dir)
}

/// The engine workspace's profile and patch tables, paths made absolute.
fn engine_tables(engine: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(engine.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let start = text
        .lines()
        .position(|line| line.starts_with("[profile.") || line.starts_with("[patch"))
        .unwrap_or(usize::MAX);
    let root = engine.to_string_lossy();
    Ok(text
        .lines()
        .skip(start)
        .map(|line| line.replace("path = \"", &format!("path = \"{root}/")))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// A Cargo config for a plugin author's own crate, so `cargo test` in it
/// finds `blockloom-engine` (and the engine's patched dependencies) here.
pub fn author_config(engine: &Path) -> Result<String, String> {
    let tables = engine_tables(engine)?;
    let mut out = String::from(
        "# Points this crate at this machine's Blockloom engine. Local, not packaged.\n",
    );
    let mut keep = false;
    for line in tables.lines() {
        if line.starts_with('[') {
            keep = line.starts_with("[patch");
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// Builds the project's runtime, sharing the engine's target directory so
/// only the plugin crates and the link are new. Returns the binary.
pub fn build(project: &Path, crates: &[EngineCrate]) -> Result<PathBuf, String> {
    let engine = engine_source().ok_or(
        "Engine plugins need the engine's source (set BLOCKLOOM_ENGINE_SOURCE)".to_string(),
    )?;
    let dir = generate(project, &engine, crates)?;
    let target = std::env::var_os("BLOCKLOOM_ENGINE_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| engine.join("target"));
    let release = !cfg!(debug_assertions);
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .current_dir(&dir)
        .arg("build")
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        // Nobody iterates on the generated crate; incremental caches only fill the disk.
        .env("CARGO_INCREMENTAL", "0");
    if release {
        cargo.arg("--release");
    }
    let output = cargo
        .output()
        .map_err(|e| format!("Couldn't run cargo: {e}"))?;
    if !output.status.success() {
        let log = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<_> = log.lines().rev().take(40).collect();
        return Err(tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    let built = target
        .join(if release { "release" } else { "debug" })
        .join(blockloom_protocol::RUNTIME_BINARY);
    let out = binary(project);
    std::fs::create_dir_all(out.parent().unwrap_or(project)).map_err(|e| e.to_string())?;
    // Copied rather than linked: the shared target is rebuilt by others.
    std::fs::copy(&built, &out).map_err(|e| format!("{}: {e}", built.display()))?;
    let stamp = serde_json::to_vec_pretty(&stamp_of(crates)).map_err(|e| e.to_string())?;
    blockloom_core::files::replace_file(&stamp_path(project), &stamp).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_main_adds_every_module() {
        let project = tempfile::tempdir().unwrap();
        let engine = tempfile::tempdir().unwrap();
        std::fs::write(
            engine.path().join("Cargo.toml"),
            "[workspace]\n\n[profile.release]\nlto = true\n\n[patch.crates-io]\nfoo = { path = \"vendor/foo\" }\n",
        )
        .unwrap();
        let crates = vec![EngineCrate {
            plugin: "com.example.tally".into(),
            dir: PathBuf::from("/plugins/tally/engine"),
            lib: "tally_engine".into(),
            hash: "h1".into(),
            local: false,
        }];
        let dir = generate(project.path(), engine.path(), &crates).unwrap();
        let main = std::fs::read_to_string(dir.join("main.rs")).unwrap();
        assert!(main.contains("add_engine_module(tally_engine::blockloom_module())"));
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("tally-engine = { path = \"/plugins/tally/engine\" }"));
        let root = engine.path().to_string_lossy();
        assert!(manifest.contains(&format!("foo = {{ path = \"{root}/vendor/foo\" }}")));
        assert!(manifest.contains("[profile.release]\nlto = true"));
        let config = author_config(engine.path()).unwrap();
        assert!(config.contains(&format!("foo = {{ path = \"{root}/vendor/foo\" }}")));
        assert!(!config.contains("[profile"));
    }

    #[test]
    fn a_stamp_says_whether_the_runtime_matches_its_packages() {
        let project = tempfile::tempdir().unwrap();
        let mut crates = vec![EngineCrate {
            plugin: "com.example.tally".into(),
            dir: PathBuf::new(),
            lib: "tally".into(),
            hash: "h1".into(),
            local: false,
        }];
        assert_eq!(built(project.path(), &crates), Built::Missing);
        std::fs::create_dir_all(binary(project.path()).parent().unwrap()).unwrap();
        std::fs::write(binary(project.path()), b"").unwrap();
        assert_eq!(built(project.path(), &crates), Built::Stale);
        let stamp = serde_json::to_vec(&stamp_of(&crates)).unwrap();
        std::fs::write(stamp_path(project.path()), stamp).unwrap();
        assert_eq!(built(project.path(), &crates), Built::Current);
        crates[0].hash = "h2".into();
        assert_eq!(built(project.path(), &crates), Built::Stale);
    }

    #[test]
    fn only_a_package_from_elsewhere_needs_trust() {
        let mut ledger = TrustLedger::default();
        let mut engine_crate = EngineCrate {
            plugin: "com.example.tally".into(),
            dir: PathBuf::new(),
            lib: "tally".into(),
            hash: "h1".into(),
            local: true,
        };
        assert!(!needs_trust(&engine_crate, &ledger));
        engine_crate.local = false;
        assert!(needs_trust(&engine_crate, &ledger));
        ledger.grant("com.example.tally", "h1");
        assert!(!needs_trust(&engine_crate, &ledger));
        engine_crate.hash = "h2".into();
        assert!(needs_trust(&engine_crate, &ledger));
    }

    #[test]
    fn package_names_come_from_the_package_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(
            &path,
            "[lib]\nname = \"x\"\n[package]\nname = \"my-plugin\"\n",
        )
        .unwrap();
        assert_eq!(package_name(&path).as_deref(), Some("my-plugin"));
    }
}
