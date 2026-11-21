//! The engine kit an install carries: the engine's sources and vendored
//! crates (for an author's own `cargo test`) and the compiled engine, every
//! rlib a project runtime links. Cargo's target can't move (its fingerprints
//! hold absolute paths), so a kit keeps only the artifacts plus the rustc
//! flags Cargo used for them, and a project's plugin crates and the final
//! link are plain rustc runs over those. The kit itself is never written.

use crate::project_runtime::{self, EngineCrate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const KIT_FILE: &str = "kit.json";
/// Stands for the kit's root in recorded flags.
const ROOT: &str = "{kit}";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kit {
    pub engine_build: String,
    /// `rustc --version` of the compiler that built it; only that one links it.
    pub rustc: String,
    pub profile: String,
    /// Codegen and search flags for a plugin library, from the engine's own.
    pub lib_args: Vec<String>,
    /// The runtime binary's flags, minus its source and output.
    pub bin_args: Vec<String>,
    /// Crate name to each build of it the engine linked.
    pub externs: BTreeMap<String, Vec<Extern>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extern {
    pub version: String,
    pub file: String,
}

pub fn read(root: &Path) -> Result<Kit, String> {
    let bytes = std::fs::read(root.join(KIT_FILE)).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", root.join(KIT_FILE).display()))
}

/// Builds a runtime with no plugins in `root` (sources in `src`, vendored
/// crates set up) and records what a later link needs. Prunes the target
/// down to the artifacts.
pub fn capture(root: &Path) -> Result<Kit, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let src = root.join("src");
    let build = root.join("build");
    let target = root.join("target");
    let profile = profile_dir();
    let dir = project_runtime::generate(&build, &src, &[])?;
    // Only units that compile print their command, so these two always do.
    if let Ok(entries) = std::fs::read_dir(target.join(profile).join(".fingerprint")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("blockloom-engine-")
                || name.starts_with("blockloom-project-runtime-")
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .current_dir(&dir)
        .args(["build", "-v", "--offline", "--message-format=json"])
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_INCREMENTAL", "0");
    if profile == "release" {
        cargo.arg("--release");
    }
    let output = cargo
        .output()
        .map_err(|e| format!("Couldn't run cargo: {e}"))?;
    let log = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(tail(&log));
    }
    let versions = versions(&String::from_utf8_lossy(&output.stdout));
    let mut lib = None;
    let mut bin = None;
    let mut externs: BTreeMap<String, Vec<Extern>> = BTreeMap::new();
    for line in log.lines() {
        let Some(command) = line
            .trim_start()
            .strip_prefix("Running `")
            .and_then(|c| c.strip_suffix('`'))
        else {
            continue;
        };
        let words = split_words(command);
        let Some(name) = value_of(&words, "--crate-name") else {
            continue;
        };
        let kind = value_of(&words, "--crate-type").unwrap_or("lib");
        if name == "blockloom_engine" {
            lib = Some(words.clone());
        } else if name == "blockloom_runtime" && kind == "bin" {
            bin = Some(words.clone());
        }
        // Host-side builds (build scripts, proc macros' own deps) aren't linked.
        if kind == "proc-macro" || name.starts_with("build_script_") {
            continue;
        }
        let mut words = words.iter();
        while let Some(word) = words.next() {
            if word != "--extern" {
                continue;
            }
            let Some((crate_name, file)) = words.next().and_then(|w| w.split_once('=')) else {
                continue;
            };
            // A pipelined build names the metadata; the link wants the library.
            let rlib = Path::new(file).with_extension("rlib");
            let file = match file.ends_with(".rmeta") && rlib.is_file() {
                true => rlib.to_string_lossy().into_owned(),
                false => file.to_string(),
            };
            let entry = Extern {
                version: versions.get(&file).cloned().unwrap_or_default(),
                file: hide_root(&file, &root),
            };
            let list = externs.entry(crate_name.to_string()).or_default();
            if !list.contains(&entry) {
                list.push(entry);
            }
        }
    }
    let lib = lib.ok_or("cargo didn't compile blockloom-engine")?;
    let bin = bin.ok_or("cargo didn't compile the runtime binary")?;
    if let Some(file) = lib_file(&versions, "blockloom_engine") {
        externs.insert(
            "blockloom_engine".into(),
            vec![Extern {
                version: env!("CARGO_PKG_VERSION").into(),
                file: hide_root(&file, &root),
            }],
        );
    }
    let kit = Kit {
        engine_build: env!("CARGO_PKG_VERSION").into(),
        rustc: rustc_version(Command::new("rustc").current_dir(&src))?,
        profile: profile.into(),
        lib_args: clean(&lib, false, &root),
        bin_args: clean(&bin, true, &root),
        externs,
    };
    prune(&target, profile)?;
    let _ = std::fs::remove_dir_all(&build);
    Ok(kit)
}

/// Compiles each plugin crate against the kit and links the runtime, in
/// `build` (writable, one build at a time). Returns the binary.
pub fn link(root: &Path, build: &Path, crates: &[EngineCrate]) -> Result<PathBuf, String> {
    let kit = read(root)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let deps = build.join("deps");
    std::fs::create_dir_all(&deps).map_err(|e| e.to_string())?;
    let lock = std::fs::File::create(build.join(".lock")).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let rustc = rustc_for(&root, &kit)?;
    let expand = |args: &[String]| -> Vec<String> {
        args.iter()
            .map(|a| a.replace(ROOT, &root.to_string_lossy()))
            .collect()
    };
    let mut plugins = Vec::new();
    for engine_crate in crates {
        let manifest = Manifest::read(&engine_crate.dir)?;
        let tag = &engine_crate.hash[..engine_crate.hash.len().min(16)];
        let mut command = Command::new(&rustc);
        command
            .current_dir(&engine_crate.dir)
            .args(expand(&kit.lib_args))
            .arg("--crate-name")
            .arg(&engine_crate.lib)
            .arg(format!("--edition={}", manifest.edition))
            .arg(&manifest.lib)
            .args(["--crate-type", "rlib", "--emit=link"])
            .arg(format!("-Cmetadata={tag}"))
            .arg(format!("-Cextra-filename=-{tag}"))
            .arg("--out-dir")
            .arg(&deps)
            .arg("-L")
            .arg(format!("dependency={}", deps.display()))
            .env("CARGO_MANIFEST_DIR", &engine_crate.dir)
            .env("CARGO_MANIFEST_PATH", engine_crate.dir.join("Cargo.toml"))
            .env("CARGO_PKG_NAME", &manifest.name)
            .env("CARGO_PKG_VERSION", &manifest.version)
            .env("CARGO_CRATE_NAME", &engine_crate.lib);
        for dep in &manifest.deps {
            let found = pick(&kit, dep).ok_or_else(|| {
                format!(
                    "{}: uses the crate {}, which this engine doesn't have (an engine plugin can only use the engine's own crates)",
                    engine_crate.plugin, dep.package
                )
            })?;
            command.arg("--extern").arg(format!(
                "{}={}",
                dep.name,
                found.replace(ROOT, &root.to_string_lossy())
            ));
        }
        run(&mut command, &engine_crate.plugin)?;
        plugins.push((
            engine_crate.lib.clone(),
            deps.join(format!("lib{}-{tag}.rlib", engine_crate.lib)),
        ));
    }
    let main = build.join("main.rs");
    blockloom_core::files::write_atomic(&main, project_runtime::main_source(crates).as_bytes())
        .map_err(|e| e.to_string())?;
    let binary = build.join(blockloom_protocol::RUNTIME_BINARY);
    let mut command = Command::new(&rustc);
    command
        .current_dir(build)
        .args(expand(&kit.bin_args))
        .args(["--crate-name", "blockloom_runtime", "--edition=2024"])
        .arg(&main)
        .args(["--crate-type", "bin", "--emit=link", "-o"])
        .arg(&binary)
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .env("CARGO_MANIFEST_DIR", build)
        .env("CARGO_PKG_NAME", "blockloom-project-runtime")
        .env("CARGO_PKG_VERSION", "0.0.0")
        .env("CARGO_CRATE_NAME", "blockloom_runtime");
    for (lib, file) in &plugins {
        command
            .arg("--extern")
            .arg(format!("{lib}={}", file.display()));
    }
    run(&mut command, "the runtime")?;
    Ok(binary)
}

fn run(command: &mut Command, what: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|e| format!("Couldn't run rustc: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "Compiling {what} failed:\n{}",
        tail(&String::from_utf8_lossy(&output.stderr))
    ))
}

fn tail(log: &str) -> String {
    let lines: Vec<_> = log.lines().rev().take(40).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// The install's rustc, else the kit's toolchain through rustup, held to the
/// exact version that built the kit.
fn rustc_for(root: &Path, kit: &Kit) -> Result<PathBuf, String> {
    let (program, channel) = match blockloom_core::script::bundled_rustc() {
        Some(rustc) => (rustc, None),
        None => (PathBuf::from("rustc"), channel(&root.join("src"))),
    };
    let mut probe = Command::new(&program);
    if let Some(channel) = &channel {
        probe.env("RUSTUP_TOOLCHAIN", channel);
    }
    let found = rustc_version(&mut probe)?;
    if found != kit.rustc {
        return Err(format!(
            "This engine kit was built with {}, but the compiler here is {found}",
            kit.rustc
        ));
    }
    match channel {
        Some(channel) => Ok(which_toolchain(&program, &channel).unwrap_or(program)),
        None => Ok(program),
    }
}

/// The real rustc rustup would run for `channel`, so later runs need no env.
fn which_toolchain(program: &Path, channel: &str) -> Option<PathBuf> {
    let output = Command::new(program)
        .env("RUSTUP_TOOLCHAIN", channel)
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let rustc = sysroot
        .join("bin")
        .join(if cfg!(windows) { "rustc.exe" } else { "rustc" });
    rustc.is_file().then_some(rustc)
}

fn channel(src: &Path) -> Option<String> {
    let text = std::fs::read_to_string(src.join("rust-toolchain.toml")).ok()?;
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("channel")?;
        Some(
            rest.trim_start()
                .strip_prefix('=')?
                .trim()
                .trim_matches('"')
                .to_string(),
        )
    })
}

fn rustc_version(command: &mut Command) -> Result<String, String> {
    let output = command
        .arg("--version")
        .output()
        .map_err(|e| format!("Couldn't run rustc: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn profile_dir() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Artifact file to its package's version, from Cargo's JSON messages.
fn versions(stdout: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in stdout.lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if message["reason"] != "compiler-artifact" {
            continue;
        }
        let id = message["package_id"].as_str().unwrap_or_default();
        let fragment = id.rsplit('#').next().unwrap_or_default();
        let version = fragment.rsplit('@').next().unwrap_or_default().to_string();
        for file in message["filenames"].as_array().into_iter().flatten() {
            if let Some(file) = file.as_str() {
                out.insert(file.to_string(), version.clone());
            }
        }
    }
    out
}

fn lib_file(versions: &HashMap<String, String>, lib: &str) -> Option<String> {
    let prefix = format!("lib{lib}-");
    versions
        .keys()
        .find(|file| {
            Path::new(file)
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
                && file.ends_with(".rlib")
        })
        .cloned()
}

fn hide_root(text: &str, root: &Path) -> String {
    text.replace(&*root.to_string_lossy(), ROOT)
}

fn value_of<'a>(words: &'a [String], flag: &str) -> Option<&'a str> {
    let at = words.iter().position(|w| w == flag)?;
    words.get(at + 1).map(String::as_str)
}

/// A recorded command without its program, crate, source, outputs and
/// per-build hashes. A library keeps no `--extern`s or `--cfg`s either,
/// since a plugin names its own.
fn clean(words: &[String], keep_externs: bool, root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut words = words.iter().skip(1);
    while let Some(word) = words.next() {
        let dropped_with_value = [
            "--crate-name",
            "--crate-type",
            "--out-dir",
            "--cfg",
            "--check-cfg",
        ];
        if dropped_with_value.contains(&word.as_str()) {
            words.next();
            continue;
        }
        if word == "--extern" {
            let value = words.next();
            if keep_externs && let Some(value) = value {
                out.push(word.clone());
                out.push(hide_root(value, root));
            }
            continue;
        }
        if word == "-C" {
            let Some(value) = words.next() else { break };
            if !per_build(value) {
                out.push(word.clone());
                out.push(hide_root(value, root));
            }
            continue;
        }
        if let Some(value) = word.strip_prefix("-C")
            && per_build(value)
        {
            continue;
        }
        let dropped = ["--error-format=", "--json=", "--emit=", "--edition="];
        if dropped.iter().any(|p| word.starts_with(p)) || word.ends_with(".rs") {
            continue;
        }
        out.push(hide_root(word, root));
    }
    out
}

fn per_build(codegen: &str) -> bool {
    ["incremental=", "metadata=", "extra-filename="]
        .iter()
        .any(|p| codegen.starts_with(p))
}

/// Splits a command as Cargo prints it (shell-quoted for this platform).
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !cfg!(windows) => {
                started = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    word.push(c);
                }
            }
            '"' => {
                started = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' if matches!(chars.peek(), Some('"' | '\\')) => {
                            word.extend(chars.next());
                        }
                        c => word.push(c),
                    }
                }
            }
            '\\' if !cfg!(windows) => {
                started = true;
                word.extend(chars.next());
            }
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

fn prune(target: &Path, profile: &str) -> Result<(), String> {
    let dir = target.join(profile);
    for name in [".fingerprint", "incremental", "examples", ".cargo-lock"] {
        let path = dir.join(name);
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
    }
    for entry in std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        if entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    for entry in std::fs::read_dir(dir.join("deps"))
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".d") || name.starts_with("blockloom_runtime-") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    // A build script's `out` may hold native libraries the link searches.
    for entry in std::fs::read_dir(dir.join("build"))
        .map_err(|e| e.to_string())?
        .flatten()
    {
        for inner in std::fs::read_dir(entry.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            if inner.file_name() == "out" {
                continue;
            }
            let path = inner.path();
            if inner.file_type().is_ok_and(|t| t.is_dir()) {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    // Any other profile was left by someone else.
    for entry in std::fs::read_dir(target)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        if entry.file_name() != profile {
            let path = entry.path();
            let _ = std::fs::remove_dir_all(&path);
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

/// The parts of a plugin crate's manifest a direct rustc run needs.
#[derive(Debug, PartialEq)]
struct Manifest {
    name: String,
    version: String,
    edition: String,
    lib: PathBuf,
    deps: Vec<Dep>,
}

#[derive(Debug, PartialEq)]
struct Dep {
    /// What the crate's code calls it.
    name: String,
    /// The package it names.
    package: String,
    version: String,
}

impl Manifest {
    fn read(dir: &Path) -> Result<Self, String> {
        let path = dir.join("Cargo.toml");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut manifest = Self::parse(&text);
        if dir.join("build.rs").exists() || text.lines().any(|l| l.trim().starts_with("build =")) {
            return Err(format!(
                "{}: engine plugin crates can't have a build script",
                dir.display()
            ));
        }
        manifest.lib = dir.join(&manifest.lib);
        if manifest.name.is_empty() {
            return Err(format!("{}: no package name", path.display()));
        }
        Ok(manifest)
    }

    /// A line reader over the few tables that matter, not a TOML parser.
    fn parse(text: &str) -> Self {
        let mut manifest = Manifest {
            name: String::new(),
            version: "0.0.0".into(),
            edition: "2015".into(),
            lib: PathBuf::from("src/lib.rs"),
            deps: Vec::new(),
        };
        let mut section = String::new();
        let mut table_dep: Option<Dep> = None;
        let string = |rest: &str| -> Option<String> {
            let value = rest.trim_start().strip_prefix('=')?.trim();
            value
                .starts_with('"')
                .then(|| value.trim_matches('"').to_string())
        };
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                manifest.deps.extend(table_dep.take());
                section = line.trim_matches(['[', ']']).trim().to_string();
                if let Some(name) = section.strip_prefix("dependencies.") {
                    table_dep = Some(Dep {
                        name: name.replace('-', "_"),
                        package: name.to_string(),
                        version: String::new(),
                    });
                }
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, rest)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let rest = format!("={rest}");
            match section.as_str() {
                "package" => match key {
                    "name" => manifest.name = string(&rest).unwrap_or_default(),
                    "version" => manifest.version = string(&rest).unwrap_or_default(),
                    "edition" => manifest.edition = string(&rest).unwrap_or_default(),
                    _ => {}
                },
                "lib" if key == "path" => {
                    if let Some(path) = string(&rest) {
                        manifest.lib = PathBuf::from(path);
                    }
                }
                "dependencies" => {
                    let name = key.split('.').next().unwrap_or(key).trim();
                    let inline = |field: &str| {
                        let at = rest.find(&format!("{field} = \""))? + field.len() + 4;
                        Some(rest[at..].split('"').next()?.to_string())
                    };
                    if rest.contains("optional = true") {
                        continue;
                    }
                    manifest.deps.push(Dep {
                        name: name.replace('-', "_"),
                        package: inline("package").unwrap_or_else(|| name.to_string()),
                        version: string(&rest)
                            .or_else(|| inline("version"))
                            .unwrap_or_default(),
                    });
                }
                _ => {
                    if let Some(dep) = table_dep.as_mut() {
                        match key {
                            "package" => dep.package = string(&rest).unwrap_or_default(),
                            "version" => dep.version = string(&rest).unwrap_or_default(),
                            "optional" if rest.contains("true") => dep.package.clear(),
                            _ => {}
                        }
                    }
                }
            }
        }
        manifest.deps.extend(table_dep.take());
        manifest.deps.retain(|d| !d.package.is_empty());
        manifest
    }
}

/// The kit's build of a dependency: the only one, or the one whose version
/// the requirement names.
fn pick<'a>(kit: &'a Kit, dep: &Dep) -> Option<&'a str> {
    let builds = kit.externs.get(&dep.package.replace('-', "_"))?;
    let wanted = dep.version.trim_start_matches(['^', '=', '~', ' ']);
    builds
        .iter()
        .find(|b| !wanted.is_empty() && b.version.starts_with(wanted))
        .or_else(|| builds.first())
        .map(|b| b.file.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_commands_split_like_a_shell() {
        let words =
            split_words(r#"/bin/rustc --cfg 'feature="std"' -C opt-level=3 a\ b 'it'\''s'"#);
        if cfg!(windows) {
            return;
        }
        assert_eq!(
            words,
            [
                "/bin/rustc",
                "--cfg",
                r#"feature="std""#,
                "-C",
                "opt-level=3",
                "a b",
                "it's"
            ]
        );
    }

    #[test]
    fn a_recorded_command_keeps_only_what_another_build_shares() {
        let root = Path::new("/kit");
        let words: Vec<String> = "rustc --crate-name blockloom_engine --edition=2024 /kit/src/e/lib.rs \
             --error-format=json --json=artifacts --crate-type lib --emit=dep-info,metadata,link \
             -C opt-level=3 -C embed-bitcode=no -C metadata=abc -C extra-filename=-abc \
             --out-dir /kit/target/debug/deps -L dependency=/kit/target/debug/deps \
             --extern bevy=/kit/target/debug/deps/libbevy-1.rlib --cfg feature=\"x\" -Cincremental=/tmp"
            .split_whitespace()
            .map(String::from)
            .collect();
        assert_eq!(
            clean(&words, false, root),
            [
                "-C",
                "opt-level=3",
                "-C",
                "embed-bitcode=no",
                "-L",
                "dependency={kit}/target/debug/deps"
            ]
        );
        assert!(
            clean(&words, true, root)
                .contains(&"bevy={kit}/target/debug/deps/libbevy-1.rlib".to_string())
        );
    }

    #[test]
    fn a_plugin_manifest_names_its_crates() {
        let manifest = Manifest::parse(
            "[package]\nname = \"my-plugin\"\nversion = \"0.2.0\"\nedition = \"2024\"\n\n\
             [lib]\npath = \"lib.rs\"\n\n\
             [dependencies]\nblockloom-engine = \"0.1\"\n\
             bevy = { version = \"0.20.1\", default-features = false }\n\
             rng = { package = \"rand\", version = \"0.9\" }\n\
             extra = { version = \"1\", optional = true }\n\n\
             [dependencies.serde]\nversion = \"1\"\n\n[dev-dependencies]\ntempfile = \"3\"\n",
        );
        assert_eq!(manifest.name, "my-plugin");
        assert_eq!(manifest.edition, "2024");
        assert_eq!(manifest.lib, PathBuf::from("lib.rs"));
        let deps: Vec<_> = manifest
            .deps
            .iter()
            .map(|d| (d.name.as_str(), d.package.as_str(), d.version.as_str()))
            .collect();
        assert_eq!(
            deps,
            [
                ("blockloom_engine", "blockloom-engine", "0.1"),
                ("bevy", "bevy", "0.20.1"),
                ("rng", "rand", "0.9"),
                ("serde", "serde", "1"),
            ]
        );
    }

    #[test]
    fn a_dependency_picks_the_build_its_version_names() {
        let kit = Kit {
            engine_build: String::new(),
            rustc: String::new(),
            profile: "debug".into(),
            lib_args: Vec::new(),
            bin_args: Vec::new(),
            externs: BTreeMap::from([(
                "rand".to_string(),
                vec![
                    Extern {
                        version: "0.8.5".into(),
                        file: "old".into(),
                    },
                    Extern {
                        version: "0.9.1".into(),
                        file: "new".into(),
                    },
                ],
            )]),
        };
        let dep = |version: &str| Dep {
            name: "rand".into(),
            package: "rand".into(),
            version: version.into(),
        };
        assert_eq!(pick(&kit, &dep("0.9")), Some("new"));
        assert_eq!(pick(&kit, &dep("^0.8")), Some("old"));
        assert_eq!(pick(&kit, &dep("")), Some("old"));
        assert_eq!(
            pick(
                &kit,
                &Dep {
                    package: "serde".into(),
                    ..dep("1")
                }
            ),
            None
        );
    }
}
