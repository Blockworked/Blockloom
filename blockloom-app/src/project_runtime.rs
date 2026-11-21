//! Engine-tier (adapter) plugins: a project whose plugins carry engine
//! crates gets its own runtime, the stock one with those crates linked in,
//! built into `.blockloom/runtime/` and started as the editor's child.
//! From a source tree it builds with Cargo; an install links its engine kit's
//! prebuilt crates with rustc instead (`engine_kit`). Either way only the
//! plugin crates and the link are new.

use crate::engine_kit;
use blockloom_plugin_api::manifest::Tier;
use blockloom_plugin_host::active::ActivePlugins;
use blockloom_plugin_host::source::Source;
use blockloom_plugin_host::trust::TrustLedger;
use serde_json::Value;
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

/// Where engine plugins are built.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The engine's sources (an author's crate patches onto them).
    pub src: PathBuf,
    /// The generated crate (a source tree) or the link's work folder (a kit).
    pub build: PathBuf,
    pub target: PathBuf,
    /// An install's kit, linked with rustc rather than built with Cargo.
    pub kit: Option<PathBuf>,
}

/// The engine to build against: a source tree when there is one, else the
/// install's kit, read in place, with its link folder in the data folder.
pub fn engine() -> Result<Engine, String> {
    if let Some(src) = engine_source() {
        let target = std::env::var_os("BLOCKLOOM_ENGINE_TARGET")
            .map(PathBuf::from)
            .unwrap_or_else(|| src.join("target"));
        return Ok(Engine {
            build: target.join("project-runtime"),
            src,
            target,
            kit: None,
        });
    }
    let kit = kit_dir().ok_or(
        "Engine plugins need the engine kit beside the editor (or set BLOCKLOOM_ENGINE_SOURCE)",
    )?;
    let stamp = std::fs::read(kit.join(engine_kit::KIT_FILE)).map_err(|e| e.to_string())?;
    let name = format!("{}-{}", env!("CARGO_PKG_VERSION"), short_hash(&stamp));
    Ok(Engine {
        src: kit.join("src"),
        build: blockloom_core::project::data_dir()
            .join("engine-build")
            .join(name),
        target: kit.join("target"),
        kit: Some(kit),
    })
}

fn short_hash(bytes: &[u8]) -> String {
    blockloom_plugin_host::package::sha256_hex(bytes)[..12].to_string()
}

/// `engine/` beside the editor (or one level up, as `tools/rust` is found).
fn kit_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BLOCKLOOM_ENGINE_KIT").map(PathBuf::from) {
        return dir.join(engine_kit::KIT_FILE).is_file().then_some(dir);
    }
    let exe = std::env::current_exe().ok()?;
    exe.parent()?
        .ancestors()
        .take(2)
        .map(|dir| dir.join("engine"))
        .find(|dir| dir.join(engine_kit::KIT_FILE).is_file())
}

fn copy_keeping_times(from: &Path, to: &Path) -> Result<(), String> {
    let fail = |path: &Path, e: std::io::Error| format!("{}: {e}", path.display());
    std::fs::create_dir_all(to).map_err(|e| fail(to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| fail(from, e))? {
        let entry = entry.map_err(|e| fail(from, e))?;
        let source = entry.path();
        let dest = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| fail(&source, e))?;
        if kind.is_dir() {
            copy_keeping_times(&source, &dest)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &dest).map_err(|e| fail(&source, e))?;
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .map_err(|e| fail(&source, e))?;
            std::fs::File::options()
                .write(true)
                .open(&dest)
                .and_then(|f| f.set_modified(modified))
                .map_err(|e| fail(&dest, e))?;
        }
    }
    Ok(())
}

/// Writes an engine kit into `out` from the source tree `source`: the
/// tracked sources, every crate they use vendored, and the engine's compiled
/// crates with the flags that link them (`engine_kit::capture`).
pub fn make_kit(source: &Path, out: &Path) -> Result<Value, String> {
    let src = out.join("src");
    std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
    let listed = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| format!("Couldn't run git: {e}"))?;
    if !listed.status.success() {
        return Err(format!("{} is not a git checkout", source.display()));
    }
    let mut files = 0;
    for name in listed.stdout.split(|b| *b == 0).filter(|n| !n.is_empty()) {
        let name = String::from_utf8_lossy(name);
        let from = source.join(&*name);
        if !from.is_file() {
            continue;
        }
        let to = src.join(&*name);
        std::fs::create_dir_all(to.parent().unwrap_or(&src)).map_err(|e| e.to_string())?;
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        files += 1;
    }
    // Patched copies of upstream crates, made by prepare-patched-deps.
    let patched = source.join(".patched-deps");
    if patched.is_dir() {
        copy_keeping_times(&patched, &src.join(".patched-deps"))?;
    }
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    let vendored = cargo
        .current_dir(&src)
        .args(["vendor", "--versioned-dirs"])
        .arg(out.join("vendor"))
        .output()
        .map_err(|e| format!("Couldn't run cargo vendor: {e}"))?;
    if !vendored.status.success() {
        return Err(String::from_utf8_lossy(&vendored.stderr).into_owned());
    }
    repair_vendored_libs(&src, &out.join("vendor"))?;
    // Its printed config names the vendor folder absolutely; the kit moves.
    let config = String::from_utf8_lossy(&vendored.stdout).replace(
        &format!("{:?}", out.join("vendor").to_string_lossy()),
        "\"vendor\"",
    );
    let dot_cargo = out.join(".cargo");
    std::fs::create_dir_all(&dot_cargo).map_err(|e| e.to_string())?;
    std::fs::write(dot_cargo.join("config.toml"), config).map_err(|e| e.to_string())?;
    let kit = engine_kit::capture(out)?;
    std::fs::write(
        out.join(engine_kit::KIT_FILE),
        serde_json::to_vec_pretty(&kit).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // The kit has to link as an install will use it, from its own files.
    let check = out.join("link-check");
    let linked = engine_kit::link(out, &check, &[]).map(|binary| binary.exists());
    let _ = std::fs::remove_dir_all(&check);
    let linked = linked?;
    Ok(serde_json::json!({
        "kit": out.to_string_lossy(),
        "files": files,
        "rustc": kit.rustc,
        "linked": linked,
    }))
}

/// `cargo vendor` keeps only a git crate's own folder, so one whose library
/// lives elsewhere in its repository (`path = "../../src/lib.rs"`, as Rapier's
/// do) arrives with no library. Copies that source in and names it again.
fn repair_vendored_libs(src: &Path, vendor: &Path) -> Result<(), String> {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(src)
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|e| format!("Couldn't run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
    for package in packages {
        let git = package["source"]
            .as_str()
            .is_some_and(|s| s.starts_with("git+"));
        let (Some(name), Some(version), Some(manifest)) = (
            package["name"].as_str(),
            package["version"].as_str(),
            package["manifest_path"].as_str(),
        ) else {
            continue;
        };
        let home = Path::new(manifest).parent().unwrap_or(Path::new(manifest));
        let dest = vendor.join(format!("{name}-{version}"));
        if !git || !dest.is_dir() {
            continue;
        }
        let targets = package["targets"].as_array().cloned().unwrap_or_default();
        for target in targets {
            let kinds: Vec<&str> = target["kind"]
                .as_array()
                .map(|k| k.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let is_lib = kinds.iter().any(|k| {
                matches!(
                    *k,
                    "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
                )
            });
            let Some(lib_src) = target["src_path"].as_str().map(lexical) else {
                continue;
            };
            let lib_src = lib_src.as_path();
            if !is_lib || lib_src.starts_with(home) {
                continue;
            }
            // The repository folder both share, and the first folder under it.
            let common: PathBuf = home
                .components()
                .zip(lib_src.components())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a)
                .collect();
            let rest = lib_src.strip_prefix(&common).map_err(|e| e.to_string())?;
            let Some(top) = rest.components().next() else {
                continue;
            };
            copy_keeping_times(&common.join(top), &dest.join(top))?;
            let lib_name = target["name"].as_str().unwrap_or(name).replace('-', "_");
            let mut table = format!(
                "\n[lib]\nname = \"{lib_name}\"\npath = {:?}\n",
                rest.to_string_lossy().replace('\\', "/")
            );
            if kinds != ["lib"] {
                table.push_str(&format!("crate-type = {kinds:?}\n"));
            }
            if let Some(features) = target["required-features"].as_array() {
                let features: Vec<&str> = features.iter().filter_map(Value::as_str).collect();
                table.push_str(&format!("required-features = {features:?}\n"));
            }
            let toml = dest.join("Cargo.toml");
            let mut text = std::fs::read_to_string(&toml).map_err(|e| e.to_string())?;
            text = text.replace("autolib = false\n", "");
            text.push_str(&table);
            std::fs::write(&toml, text).map_err(|e| e.to_string())?;
            // The manifest no longer matches its recorded hash; unlisted files go unchecked.
            let sums = dest.join(".cargo-checksum.json");
            if let Ok(bytes) = std::fs::read(&sums) {
                let mut json: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if let Some(files) = json["files"].as_object_mut() {
                    files.remove("Cargo.toml");
                }
                std::fs::write(&sums, serde_json::to_vec(&json).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// `a/b/../c` as `a/c`, without touching the disk.
fn lexical(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
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

/// Where a project's runtime binary and its stamp land.
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
pub fn generate(dir: &Path, engine: &Path, crates: &[EngineCrate]) -> Result<PathBuf, String> {
    let dir = dir.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // The engine's paths are written relative to the build crate, so a kit
    // still builds after it is copied somewhere else.
    let quote = |path: &Path| format!("{:?}", relative(&dir, path).to_string_lossy());
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
    manifest.push_str(&engine_tables_from(engine, &relative(&dir, engine))?);
    let main = main_source(crates);
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

/// The runtime's `main`: adds each plugin's module and runs the world.
pub fn main_source(crates: &[EngineCrate]) -> String {
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
    main
}

/// `to` as seen from `from`: relative when the two share a root, else `to`.
fn relative(from: &Path, to: &Path) -> PathBuf {
    let from = from.canonicalize().unwrap_or_else(|_| from.to_path_buf());
    let to = to.canonicalize().unwrap_or_else(|_| to.to_path_buf());
    let common = from
        .components()
        .zip(to.components())
        .take_while(|(a, b)| a == b)
        .count();
    // Only the filesystem root in common: nothing is gained by walking up.
    if common <= 1 {
        return to;
    }
    let mut out = PathBuf::new();
    for _ in from.components().skip(common) {
        out.push("..");
    }
    for part in to.components().skip(common) {
        out.push(part);
    }
    out
}

/// The engine workspace's profile and patch tables, paths made absolute.
fn engine_tables(engine: &Path) -> Result<String, String> {
    engine_tables_from(engine, engine)
}

/// The same tables with their paths under `root` (absolute, or relative to
/// the manifest they land in).
fn engine_tables_from(engine: &Path, root: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(engine.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let start = text
        .lines()
        .position(|line| line.starts_with("[profile.") || line.starts_with("[patch"))
        .unwrap_or(usize::MAX);
    let root = root.to_string_lossy();
    Ok(text
        .lines()
        .skip(start)
        .map(|line| line.replace("path = \"", &format!("path = \"{root}/")))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// A Cargo config for a plugin author's own crate, so `cargo test` in it
/// finds `blockloom-engine` (and the engine's patched and vendored crates)
/// on this machine.
pub fn author_config(engine: &Engine) -> Result<String, String> {
    let tables = engine_tables(&engine.src)?;
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
    if engine.kit.is_some() {
        // The kit's vendored crates, named absolutely from here.
        let root = engine.src.parent().unwrap_or(&engine.src);
        let kit = std::fs::read_to_string(root.join(".cargo/config.toml")).unwrap_or_default();
        let vendor = format!("{:?}", root.join("vendor").to_string_lossy());
        out.push('\n');
        out.push_str(&kit.replace("\"vendor\"", &vendor));
        out.push_str("\n[net]\noffline = true\n");
    }
    Ok(out)
}

/// Builds the project's runtime into `.blockloom/runtime/bin/` and stamps
/// what it was built from. Returns the binary.
pub fn build(project: &Path, crates: &[EngineCrate]) -> Result<PathBuf, String> {
    let engine = engine()?;
    let built = compile(&engine, crates)?;
    let out = binary(project);
    std::fs::create_dir_all(out.parent().unwrap_or(project)).map_err(|e| e.to_string())?;
    // Copied rather than linked: the shared target is rebuilt by others.
    std::fs::copy(&built, &out).map_err(|e| format!("{}: {e}", built.display()))?;
    let stamp = serde_json::to_vec_pretty(&stamp_of(crates)).map_err(|e| e.to_string())?;
    blockloom_core::files::replace_file(&stamp_path(project), &stamp).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Links the kit, or generates the build crate and runs Cargo on it, one
/// build at a time per engine. Returns the binary.
fn compile(engine: &Engine, crates: &[EngineCrate]) -> Result<PathBuf, String> {
    if let Some(kit) = &engine.kit {
        return engine_kit::link(kit, &engine.build, crates);
    }
    std::fs::create_dir_all(&engine.build).map_err(|e| e.to_string())?;
    let lock = std::fs::File::create(engine.build.join(".lock")).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let dir = generate(&engine.build, &engine.src, crates)?;
    let release = !cfg!(debug_assertions);
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .current_dir(&dir)
        .arg("build")
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &engine.target)
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
    Ok(engine
        .target
        .join(if release { "release" } else { "debug" })
        .join(blockloom_protocol::RUNTIME_BINARY))
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
        let dir = generate(&project.path().join("build"), engine.path(), &crates).unwrap();
        let main = std::fs::read_to_string(dir.join("main.rs")).unwrap();
        assert!(main.contains("add_engine_module(tally_engine::blockloom_module())"));
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("tally-engine = { path = \"/plugins/tally/engine\" }"));
        let root = engine.path().to_string_lossy();
        let near = relative(&dir, engine.path());
        let near = near.to_string_lossy();
        assert!(manifest.contains(&format!("foo = {{ path = \"{near}/vendor/foo\" }}")));
        assert!(manifest.contains("[profile.release]\nlto = true"));
        let config = author_config(&Engine {
            src: engine.path().to_path_buf(),
            build: PathBuf::new(),
            target: PathBuf::new(),
            kit: None,
        })
        .unwrap();
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
    fn engine_paths_are_relative_inside_one_root() {
        assert_eq!(
            relative(
                Path::new("/kit/build"),
                Path::new("/kit/src/blockloom-engine")
            ),
            PathBuf::from("../src/blockloom-engine")
        );
        assert_eq!(
            relative(Path::new("/kit/build"), Path::new("/plugins/x")),
            PathBuf::from("/plugins/x")
        );
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
