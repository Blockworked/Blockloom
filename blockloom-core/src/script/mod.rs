//! Script assets: real Rust files in a project's folder, compiled to shared
//! libraries the runtime loads.
//!
//! A script is a normal crate root. It links against one generated crate,
//! `blockloom`, which is [`prelude.rs`] over the C boundary in [`abi`], and
//! names its entry points with `blockloom::export!`. Compiling is two `rustc`
//! runs and no network: the `blockloom` rlib (cached per toolchain), then the
//! script itself as a `cdylib`. There is no Cargo and so no third-party
//! crates - a script gets `std` and the host API, which is what game logic
//! needs and keeps a build under a second.
//!
//! Everything a script can do it does through [`abi::HostApi`], and every one
//! of those becomes the same [`crate::vm::Effect`] a block produces. A script
//! is another way to drive an actor, not another engine.
//!
//! The cost of real Rust here is a real toolchain: `rustc` has to be on the
//! machine that presses Play. [`compile`] says so plainly when it isn't.

pub mod abi;
pub mod complete;
pub mod data;
pub mod ide;
pub mod symbols;
pub mod timing;
pub mod typed;
pub mod wit;

use std::path::{Path, PathBuf};
use std::process::Command;

/// The crate a script links against, assembled from the two halves that make
/// it: the boundary the host also compiles, and the API over it.
pub(crate) const PRELUDE_SOURCE: &str = concat!(
    include_str!("abi.rs"),
    include_str!("timing.rs"),
    include_str!("typed.rs"),
    include_str!("prelude.rs")
);

/// Where scripts live inside a project folder.
pub const SCRIPTS_DIR: &str = "assets/scripts";

/// Shared library code for scripts, as `mod` sources rather than runnable
/// scripts. A file under here is never built on its own; a script pulls it in
/// with `#[path] mod`, and its edits invalidate every script build.
pub const SHARED_DIR: &str = "assets/scripts/shared";

/// Whether `relative` names shared library code rather than a runnable script.
pub fn is_shared_path(relative: &str) -> bool {
    let relative = relative.replace('\\', "/");
    relative.starts_with(&format!("{SHARED_DIR}/")) && relative.ends_with(".rs")
}

/// Whether `relative` is a runnable script: a valid path outside the shared
/// tree. Only these attach to actors and compile to libraries.
pub fn is_script_entry(relative: &str) -> bool {
    is_valid_path(relative) && !is_shared_path(relative)
}

/// Where built libraries and the generated `blockloom` crate go. Inside the
/// project folder because that is all the runtime is told about, and dotted
/// so it reads as the build output it is.
pub const BUILD_DIR: &str = ".blockloom/build";

/// The target triple whose build output the script sandbox loads: the same
/// wasm module a web build ships, so one artifact serves both.
pub const WEB_TARGET: &str = "wasm32-unknown-unknown";

/// The edition a script is compiled as, which is the one the workspace uses.
const EDITION: &str = "2024";

/// What a new script file starts as. The canonical example: the docs'
/// Rosetta page (`docs/script-rosetta.md`) maps every block to its call here.
pub fn starter(actor: &str) -> String {
    format!(
        r#"// {actor}'s script. Real Rust, compiled when you press Play.
//
// `std` is available; other crates aren't - there's no Cargo behind this, so
// a build stays under a second. Everything you can do to the world is on
// `Actor`, and it lands as the same effect the blocks produce.
//
// Prefer the checked spellings: `ForceMode`, `WindDial`, `WaterDial`,
// `CloudDial`, `Precipitation`, `Easing` and `Color` over dial strings, and
// `symbols::actors::...` over string literals (see docs/script-rosetta.md).
use blockloom::*;

fn start(me: &Actor) {{
    me.say("Hello from Rust");
}}

fn tick(me: &Actor, dt: f32) {{
    if me.key_down("right arrow") {{
        me.change_position(Axis::X, 200.0 * dt);
    }}
    if me.key_down("left arrow") {{
        me.change_position(Axis::X, -200.0 * dt);
    }}
}}

// What the hat blocks start on: messages, keys, clicks, touches, particles...
fn event(me: &Actor, event: &Event) {{
    if let Event::Message(message) = event {{
        me.say(message);
    }}
}}

blockloom::export!(start = start, tick = tick, event = event);
"#
    )
}

/// A script path the project can hold: under `assets/scripts`, ending in
/// `.rs`, and with nothing in it that would climb out of the folder.
pub fn is_valid_path(relative: &str) -> bool {
    let relative = relative.replace('\\', "/");
    relative.starts_with(&format!("{SCRIPTS_DIR}/"))
        && relative.ends_with(".rs")
        && !relative.split('/').any(|part| part == ".." || part == ".")
}

/// The default path for an actor's own script.
pub fn path_for(actor: &str) -> String {
    let stem: String = actor
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let stem = stem.trim_matches('_');
    let stem = if stem.is_empty() { "script" } else { stem };
    format!("{SCRIPTS_DIR}/{}.rs", stem.to_lowercase())
}

pub fn scripts_dir(project_dir: &Path) -> PathBuf {
    project_dir.join("assets").join("scripts")
}

pub fn build_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".blockloom").join("build")
}

/// Where a build for `target` goes. `None` is this machine, and stays at the
/// top of the build folder where Play and the runtime have always looked;
/// another platform gets a folder named after its triple, so building for
/// three of them doesn't have them overwriting each other.
pub fn build_dir_for(project_dir: &Path, target: Option<&str>) -> PathBuf {
    match target {
        Some(triple) => build_dir(project_dir).join(triple),
        None => build_dir(project_dir),
    }
}

/// Where a script's source sits on disk.
pub fn source_path(project_dir: &Path, relative: &str) -> PathBuf {
    let mut path = project_dir.to_path_buf();
    for part in relative.split(['/', '\\']).filter(|p| !p.is_empty()) {
        path.push(part);
    }
    path
}

/// A crate name rustc will accept, from a script's path.
fn crate_name(relative: &str) -> String {
    let stem = relative
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(relative)
        .trim_end_matches(".rs");
    let name: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let name = name.trim_matches('_').to_lowercase();
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("script_{name}")
    } else {
        name
    }
}

/// What a shared library is called on `target`, which is not always this
/// machine: a build for another platform has to name the file that platform's
/// way or nothing there will load it.
pub(crate) fn dylib_name(stem: &str, target: Option<&str>) -> String {
    // A browser loads a script as a wasm module of its own.
    if target.map_or(cfg!(target_arch = "wasm32"), is_web) {
        return format!("{stem}.wasm");
    }
    let (windows, apple) = match target {
        Some(triple) => (
            triple.contains("windows"),
            triple.contains("apple") || triple.contains("darwin"),
        ),
        None => (cfg!(windows), cfg!(target_os = "macos")),
    };
    if windows {
        format!("{stem}.dll")
    } else if apple {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

/// Whether `triple` is the browser, where a script is a wasm module the
/// player instantiates rather than a library it opens.
pub fn is_web(triple: &str) -> bool {
    triple.starts_with("wasm32")
}

/// Where a script's built library lands. The runtime looks here rather than
/// compiling anything itself.
pub fn library_path(project_dir: &Path, relative: &str) -> PathBuf {
    library_path_for(project_dir, relative, None)
}

/// [`library_path`] for a build aimed at another platform.
pub fn library_path_for(project_dir: &Path, relative: &str, target: Option<&str>) -> PathBuf {
    build_dir_for(project_dir, target).join(dylib_name(&crate_name(relative), target))
}

fn stamp_path(project_dir: &Path, relative: &str, target: Option<&str>) -> PathBuf {
    build_dir_for(project_dir, target).join(format!("{}.stamp", crate_name(relative)))
}

/// Uses the workspace toolchain even when the editor starts elsewhere. Public
/// so out-of-tree guest builds (and their tests) pin the same toolchain Play
/// does.
pub fn rustc_command() -> Command {
    let mut command = Command::new("rustc");
    if std::env::var_os("RUSTUP_TOOLCHAIN").is_none()
        && let Some(root) = crate::android::workspace_root()
    {
        // Preserve relative source paths when selecting rustup's toolchain.
        if let Ok(text) = std::fs::read_to_string(root.join("rust-toolchain.toml"))
            && let Some(channel) = text.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("channel = ")
                    .map(|value| value.trim().trim_matches('"'))
            })
        {
            command.env("RUSTUP_TOOLCHAIN", channel);
        }
    }
    command
}

/// The rustc this machine has, or why there isn't one.
pub fn toolchain_version() -> Result<String, String> {
    let output = crate::build_control::output(rustc_command().arg("--version")).map_err(|e| {
        format!(
            "Scripts need a Rust toolchain, and `rustc` couldn't be run ({e}). \
             Install one from https://rustup.rs and reopen Blockloom."
        )
    })?;
    if !output.status.success() {
        return Err("`rustc --version` failed, so scripts can't be built".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Whether this machine can compile for `triple`: rustc has to know the
/// target and its `std` has to be installed, which is a separate download.
/// A linker for it is a third thing nothing here can check - that one shows
/// up as rustc's own error when the build runs.
pub fn target_installed(triple: &str) -> Result<(), String> {
    let output = crate::build_control::output(
        rustc_command()
            .arg("--print")
            .arg("target-libdir")
            .arg("--target")
            .arg(triple),
    )
    .map_err(|e| format!("`rustc` couldn't be run ({e})"))?;
    if !output.status.success() {
        return Err(format!("rustc doesn't know the target {triple}"));
    }
    let libdir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !Path::new(&libdir).is_dir() {
        let command = rustc_command();
        let toolchain = command
            .get_envs()
            .find_map(|(key, value)| {
                (key == "RUSTUP_TOOLCHAIN")
                    .then(|| value.map(|value| value.to_string_lossy().into_owned()))
                    .flatten()
            })
            .or_else(|| std::env::var("RUSTUP_TOOLCHAIN").ok());
        let toolchain = toolchain
            .map(|name| format!(" --toolchain {name}"))
            .unwrap_or_default();
        return Err(format!(
            "the standard library for {triple} isn't installed - `rustup target add {triple}{toolchain}`"
        ));
    }
    Ok(())
}

/// What a built library has to match to be reused: change any of it and the
/// script is compiled again.
fn stamp_for(toolchain: &str, target: Option<&str>, source: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hash);
    format!(
        "abi {}\n{}\ntarget {}\nsource {} bytes {:016x}\n",
        abi::ABI_VERSION,
        toolchain,
        target.unwrap_or("host"),
        source.len(),
        hash.finish()
    )
}

/// Every shared source file, sorted, as project-relative paths.
pub fn shared_sources(project_dir: &Path) -> Vec<String> {
    crate::script::ide::list_scripts(project_dir)
        .into_iter()
        .filter(|path| is_shared_path(path))
        .collect()
}

/// Fingerprint of the shared tree: any edit, add or delete rebuilds every
/// script, since each one may `mod` it in. Empty when nothing is shared.
fn shared_stamp(project_dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut sources = shared_sources(project_dir);
    sources.sort();
    let mut bytes = 0usize;
    for relative in &sources {
        let content = std::fs::read(source_path(project_dir, relative)).unwrap_or_default();
        bytes += content.len();
        relative.hash(&mut hash);
        content.hash(&mut hash);
    }
    format!(
        "shared {} files {} bytes {:016x}\n",
        sources.len(),
        bytes,
        hash.finish(),
    )
}

/// Builds `relative` into a shared library and returns where it landed.
/// Re-uses the last build when the source, the toolchain and the ABI are all
/// unchanged, so pressing Play twice costs nothing the second time.
pub fn compile(project_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    compile_for(project_dir, relative, None)
}

/// [`compile`] aimed at another platform, for a game being built for one.
/// Nothing else changes: the same two rustc runs, the same cache, one folder
/// down.
pub fn compile_for(
    project_dir: &Path,
    relative: &str,
    target: Option<&str>,
) -> Result<PathBuf, String> {
    compile_for_with_linker(project_dir, relative, target, None)
}

/// [`compile_for`] with an explicit linker for `target`: Android links
/// through the NDK clang wrapper, which rustc only takes as `-C linker=`.
/// `None` links the way the toolchain defaults to, as Play does.
pub fn compile_for_with_linker(
    project_dir: &Path,
    relative: &str,
    target: Option<&str>,
    linker: Option<&Path>,
) -> Result<PathBuf, String> {
    if !is_script_entry(relative) {
        if is_shared_path(relative) {
            return Err(format!(
                "\"{relative}\" is shared library code under {SHARED_DIR}/, not a runnable script"
            ));
        }
        return Err(format!(
            "\"{relative}\" isn't a script path - a script lives in {SCRIPTS_DIR}/ and ends in .rs"
        ));
    }
    let source_path = source_path(project_dir, relative);
    let source = std::fs::read_to_string(&source_path)
        .map_err(|e| format!("{}: {e}", source_path.display()))?;
    let toolchain = toolchain_version()?;
    if let Some(triple) = target {
        target_installed(triple)?;
    }

    let build = build_dir_for(project_dir, target);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;

    // Analysis-only, so it must never break Play: a project that can't spare
    // two small files still compiles.
    let _ = ide::sync_ide_project(project_dir);

    let library = library_path_for(project_dir, relative, target);
    let stamp = stamp_path(project_dir, relative, target);
    let mut wanted = stamp_for(&toolchain, target, &source);
    wanted.push_str(&stamp_for(&toolchain, target, PRELUDE_SOURCE));
    wanted.push_str(&shared_stamp(project_dir));
    // Project symbols ride in the `blockloom` rlib: a rename changes them,
    // so it must rebuild every script even when no source changed.
    wanted.push_str(&symbols::project_symbols_for_dir(project_dir).1);
    wanted.push('\n');
    // A moved SDK row moves the linker: without it in the stamp a stale
    // library would survive the move.
    if let Some(linker) = linker {
        wanted.push_str(&format!("linker {}\n", linker.display()));
    }
    if library.is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|previous| previous == wanted)
        && is_newer(&library, &source_path)
    {
        return Ok(library);
    }

    let rlib = build_prelude(&build, project_dir, &toolchain, target)?;
    let mut command = rustc_command();
    if let Some(triple) = target {
        command.arg("--target").arg(triple);
        if let Some(linker) = linker {
            command
                .arg("-C")
                .arg(format!("linker={}", linker.display()));
        }
        // A page carries its scripts inside it; debug info would be most of
        // each module. A panic's message names the script as the project
        // does, not by where this machine keeps it.
        if is_web(triple) {
            command.arg("-C").arg("strip=symbols");
            let mut prefix = project_dir.as_os_str().to_owned();
            prefix.push(format!("{}=", std::path::MAIN_SEPARATOR));
            command.arg("--remap-path-prefix").arg(prefix);
        }
    }
    let _ = std::fs::remove_file(&stamp);
    let status = crate::build_control::output(
        command
            .arg("--edition")
            .arg(EDITION)
            .arg("--crate-type")
            .arg("cdylib")
            .arg("--crate-name")
            .arg(crate_name(relative))
            .arg("--extern")
            .arg(format!("blockloom={}", rlib.display()))
            .arg("-C")
            .arg("opt-level=2")
            .arg("-o")
            .arg(&library)
            .arg(&source_path),
    )
    .map_err(|e| format!("couldn't run rustc: {e}"))?;
    if !status.status.success() {
        // Let the compiler speak for itself: its diagnostics point at the
        // script's own lines, which is what a script author needs to see.
        let _ = std::fs::remove_file(&stamp);
        return Err(String::from_utf8_lossy(&status.stderr).trim().to_string());
    }
    std::fs::write(&stamp, wanted).map_err(|e| format!("{}: {e}", stamp.display()))?;
    Ok(library)
}

/// Builds (or reuses) the `blockloom` crate every script links against. It
/// depends only on this build of Blockloom and on the toolchain, so one copy
/// per project folder is enough - plus the project's own symbols, so a
/// rename rebuilds every script against the new names.
fn build_prelude(
    build: &Path,
    project_dir: &Path,
    toolchain: &str,
    target: Option<&str>,
) -> Result<PathBuf, String> {
    let source_path = build.join("blockloom.rs");
    let rlib = build.join("libblockloom.rlib");
    let stamp = build.join("blockloom.stamp");
    let (symbols_source, symbols_stamp) = symbols::project_symbols_for_dir(project_dir);
    let mut wanted = stamp_for(toolchain, target, PRELUDE_SOURCE);
    wanted.push_str(&symbols_stamp);
    wanted.push('\n');
    if rlib.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|previous| previous == wanted) {
        return Ok(rlib);
    }

    let mut assembled = String::with_capacity(PRELUDE_SOURCE.len() + symbols_source.len() + 1);
    assembled.push_str(PRELUDE_SOURCE);
    assembled.push('\n');
    assembled.push_str(&symbols_source);
    std::fs::write(&source_path, assembled)
        .map_err(|e| format!("{}: {e}", source_path.display()))?;
    let mut command = rustc_command();
    if let Some(triple) = target {
        command.arg("--target").arg(triple);
    }
    let _ = std::fs::remove_file(&stamp);
    let status = crate::build_control::output(
        command
            .arg("--edition")
            .arg(EDITION)
            .arg("--crate-type")
            .arg("rlib")
            .arg("--crate-name")
            .arg("blockloom")
            .arg("-C")
            .arg("opt-level=2")
            .arg("-o")
            .arg(&rlib)
            .arg(&source_path),
    )
    .map_err(|e| format!("couldn't run rustc: {e}"))?;
    if !status.status.success() {
        // A failure here is Blockloom's bug, not the script author's, so say
        // so rather than showing them somebody else's compiler errors.
        return Err(format!(
            "Blockloom's own script API didn't compile with {toolchain}:\n{}",
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    std::fs::write(&stamp, wanted).map_err(|e| format!("{}: {e}", stamp.display()))?;
    Ok(rlib)
}

fn is_newer(built: &Path, source: &Path) -> bool {
    let time = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    };
    match (time(built), time(source)) {
        (Some(built), Some(source)) => built >= source,
        // Without timestamps the stamp file is the only check, which is
        // already enough to be correct - just not to skip a rebuild.
        _ => false,
    }
}

/// Makes a script file from the starter template, failing if one is already
/// there. Returns the path relative to the project folder. Shared library
/// code is not a starter script: make it as a text asset instead.
pub fn create(project_dir: &Path, relative: &str, actor: &str) -> Result<String, String> {
    if !is_script_entry(relative) {
        if is_shared_path(relative) {
            return Err(format!(
                "\"{relative}\" is shared library code under {SHARED_DIR}/, not a runnable script"
            ));
        }
        return Err(format!(
            "\"{relative}\" isn't a script path - a script lives in {SCRIPTS_DIR}/ and ends in .rs"
        ));
    }
    let path = source_path(project_dir, relative);
    if path.exists() {
        return Ok(relative.to_string());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, starter(actor)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(relative.to_string())
}

/// An unused script path based on `actor`'s name - `player.rs`, then
/// `player_2.rs`.
pub fn unused_path(project_dir: &Path, actor: &str) -> String {
    let wanted = path_for(actor);
    if !source_path(project_dir, &wanted).exists() {
        return wanted;
    }
    let stem = wanted.trim_end_matches(".rs").to_string();
    (2..)
        .map(|n| format!("{stem}_{n}.rs"))
        .find(|candidate| !source_path(project_dir, candidate).exists())
        .expect("an unused name always exists")
}

#[cfg(test)]
mod tests {

    #[test]
    fn compiler_uses_workspace_toolchain_without_an_override() {
        if std::env::var_os("RUSTUP_TOOLCHAIN").is_some() {
            return;
        }
        let Some(root) = crate::android::workspace_root() else {
            return;
        };
        let text = std::fs::read_to_string(root.join("rust-toolchain.toml")).unwrap();
        let channel = text
            .lines()
            .find_map(|line| line.trim().strip_prefix("channel = "))
            .unwrap()
            .trim()
            .trim_matches('"');
        let command = super::rustc_command();
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "RUSTUP_TOOLCHAIN"
                    && value == Some(std::ffi::OsStr::new(channel)))
        );
    }
    use super::*;

    #[test]
    fn script_guards_report_panics_without_crossing_the_abi() {
        let dir =
            std::env::temp_dir().join(format!("blockloom-script-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("blockloom.rs");
        let tests = dir.join("guard_tests.rs");
        std::fs::write(
            &source,
            format!("{PRELUDE_SOURCE}\ninclude!(\"guard_tests.rs\");\n"),
        )
        .unwrap();
        std::fs::write(&tests, include_str!("guard_tests.rs")).unwrap();
        let binary = dir.join(format!("guard-tests{}", std::env::consts::EXE_SUFFIX));
        let output = rustc_command()
            .arg("--edition")
            .arg(EDITION)
            .arg("--test")
            .arg("-C")
            .arg("opt-level=2")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(binary).output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_script_path_cant_climb_out_of_the_project() {
        assert!(is_valid_path("assets/scripts/player.rs"));
        assert!(is_valid_path("assets/scripts/enemies/chaser.rs"));
        assert!(!is_valid_path("assets/scripts/../../etc/passwd.rs"));
        assert!(!is_valid_path("/etc/passwd.rs"));
        assert!(!is_valid_path("assets/player.png"));
        assert!(!is_valid_path("assets/scripts/player.png"));
    }

    #[test]
    fn shared_code_is_not_a_runnable_script() {
        assert!(is_shared_path("assets/scripts/shared/util.rs"));
        assert!(!is_shared_path("assets/scripts/player.rs"));
        assert!(is_script_entry("assets/scripts/player.rs"));
        assert!(!is_script_entry("assets/scripts/shared/util.rs"));
        // Compiling shared code alone is refused, not a library.
        let dir = std::env::temp_dir().join(format!("blockloom-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/scripts/shared")).unwrap();
        std::fs::write(dir.join("assets/scripts/shared/util.rs"), "pub fn x() {}").unwrap();
        assert!(compile(&dir, "assets/scripts/shared/util.rs").is_err());
        // Its fingerprint moves with edits, so dependents rebuild.
        let before = shared_stamp(&dir);
        std::fs::write(
            dir.join("assets/scripts/shared/util.rs"),
            "pub fn x() -> i32 { 1 }",
        )
        .unwrap();
        assert_ne!(before, shared_stamp(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crate_name_survives_an_awkward_file_name() {
        assert_eq!(crate_name("assets/scripts/player.rs"), "player");
        assert_eq!(crate_name("assets/scripts/My Actor!.rs"), "my_actor");
        assert_eq!(crate_name("assets/scripts/2fast.rs"), "script_2fast");
    }

    #[test]
    fn an_actors_script_path_is_made_of_its_name() {
        assert_eq!(path_for("Player"), "assets/scripts/player.rs");
        assert_eq!(path_for("Big Enemy 2"), "assets/scripts/big_enemy_2.rs");
        assert_eq!(path_for("!!!"), "assets/scripts/script.rs");
    }

    #[test]
    fn the_stamp_changes_with_the_abi_the_toolchain_the_target_or_the_source() {
        let base = stamp_for("rustc 1.90.0", None, "fn main() {}");
        assert_ne!(base, stamp_for("rustc 1.91.0", None, "fn main() {}"));
        assert_ne!(base, stamp_for("rustc 1.90.0", None, "fn main() {} "));
        assert_ne!(base, stamp_for("rustc 1.90.0", None, "fn main(){ }"));
        assert_ne!(
            base,
            stamp_for(
                "rustc 1.90.0",
                Some("x86_64-unknown-linux-gnu"),
                "fn main() {}"
            )
        );
    }

    #[test]
    fn a_script_built_for_the_web_is_a_module_importing_the_host_calls() {
        const WEB: &str = "wasm32-unknown-unknown";
        if toolchain_version().is_err() || target_installed(WEB).is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("blockloom-web-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let relative = "assets/scripts/player.rs";
        create(&dir, relative, "Player").unwrap();
        let built = compile_for(&dir, relative, Some(WEB)).unwrap();
        assert!(built.ends_with("wasm32-unknown-unknown/player.wasm"));
        let bytes = std::fs::read(&built).unwrap();
        assert_eq!(&bytes[..4], b"\0asm");
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        // The three calls come in as imports, and the entry points go out.
        assert!(has(abi::WASM_MODULE.as_bytes()));
        assert!(has(abi::WASM_READ_NUMBER.as_bytes()));
        assert!(has(abi::SYM_TICK));
        assert!(has(abi::SYM_EVENT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_library_is_named_and_placed_the_way_its_target_expects() {
        let dir = Path::new("/project");
        let of = |target| {
            library_path_for(dir, "assets/scripts/player.rs", target)
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        };
        assert!(of(Some("x86_64-pc-windows-msvc")).ends_with("x86_64-pc-windows-msvc/player.dll"));
        assert!(
            of(Some("x86_64-unknown-linux-gnu")).ends_with("x86_64-unknown-linux-gnu/libplayer.so")
        );
        assert!(of(Some("aarch64-apple-darwin")).ends_with("aarch64-apple-darwin/libplayer.dylib"));
        assert!(of(Some("wasm32-unknown-unknown")).ends_with("wasm32-unknown-unknown/player.wasm"));
        // This machine's own build stays where Play and the runtime look.
        assert!(!of(None).contains("x86_64"));
    }

    #[test]
    fn script_default_names_match_core() {
        // The prelude and the guest crate spell the defaults literally (both
        // are standalone sources, not core dependents), so pin the literals
        // against the real constants.
        for (name, value) in [
            ("DEFAULT_SAVE_SLOT", crate::save::DEFAULT_SLOT),
            ("DEFAULT_LANGUAGE", crate::locale::DEFAULT_LANGUAGE),
        ] {
            let line = format!(r#"pub const {name}: &str = "{value}";"#);
            assert!(
                PRELUDE_SOURCE.contains(&line),
                "prelude {name} drifted from core"
            );
            assert!(
                include_str!("../../../blockloom-script-guest/src/lib.rs").contains(&line),
                "guest {name} drifted from core"
            );
        }
    }
}
