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

use std::path::{Path, PathBuf};
use std::process::Command;

/// The crate a script links against, assembled from the two halves that make
/// it: the boundary the host also compiles, and the API over it.
const PRELUDE_SOURCE: &str = concat!(include_str!("abi.rs"), include_str!("prelude.rs"));

/// Where scripts live inside a project folder.
pub const SCRIPTS_DIR: &str = "assets/scripts";

/// Where built libraries and the generated `blockloom` crate go. Inside the
/// project folder because that is all the runtime is told about, and dotted
/// so it reads as the build output it is.
pub const BUILD_DIR: &str = ".blockloom/build";

/// The edition a script is compiled as, which is the one the workspace uses.
const EDITION: &str = "2024";

/// What a new script file starts as.
pub fn starter(actor: &str) -> String {
    format!(
        r#"// {actor}'s script. Real Rust, compiled when you press Play.
//
// `std` is available; other crates aren't - there's no Cargo behind this, so
// a build stays under a second. Everything you can do to the world is on
// `Actor`, and it lands as the same effect the blocks produce.
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

blockloom::export!(start = start, tick = tick);
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

fn dylib_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

/// Where a script's built library lands. The runtime looks here rather than
/// compiling anything itself.
pub fn library_path(project_dir: &Path, relative: &str) -> PathBuf {
    build_dir(project_dir).join(dylib_name(&crate_name(relative)))
}

fn stamp_path(project_dir: &Path, relative: &str) -> PathBuf {
    build_dir(project_dir).join(format!("{}.stamp", crate_name(relative)))
}

/// The rustc this machine has, or why there isn't one.
pub fn toolchain_version() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("--version")
        .output()
        .map_err(|e| {
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

/// What a built library has to match to be reused: change any of it and the
/// script is compiled again.
fn stamp_for(toolchain: &str, source: &str) -> String {
    format!(
        "abi {}\n{}\nsource {} bytes\n",
        abi::ABI_VERSION,
        toolchain,
        source.len()
    )
}

/// Builds `relative` into a shared library and returns where it landed.
/// Re-uses the last build when the source, the toolchain and the ABI are all
/// unchanged, so pressing Play twice costs nothing the second time.
pub fn compile(project_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    if !is_valid_path(relative) {
        return Err(format!(
            "\"{relative}\" isn't a script path - a script lives in {SCRIPTS_DIR}/ and ends in .rs"
        ));
    }
    let source_path = source_path(project_dir, relative);
    let source = std::fs::read_to_string(&source_path)
        .map_err(|e| format!("{}: {e}", source_path.display()))?;
    let toolchain = toolchain_version()?;

    let build = build_dir(project_dir);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;

    let library = library_path(project_dir, relative);
    let stamp = stamp_path(project_dir, relative);
    let wanted = stamp_for(&toolchain, &source);
    if library.is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|previous| previous == wanted)
        && is_newer(&library, &source_path)
    {
        return Ok(library);
    }

    let rlib = build_prelude(&build, &toolchain)?;
    let status = Command::new("rustc")
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
        .arg(&source_path)
        .output()
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
/// per project folder is enough.
fn build_prelude(build: &Path, toolchain: &str) -> Result<PathBuf, String> {
    let source_path = build.join("blockloom.rs");
    let rlib = build.join("libblockloom.rlib");
    let stamp = build.join("blockloom.stamp");
    let wanted = stamp_for(toolchain, PRELUDE_SOURCE);
    if rlib.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|previous| previous == wanted) {
        return Ok(rlib);
    }

    std::fs::write(&source_path, PRELUDE_SOURCE)
        .map_err(|e| format!("{}: {e}", source_path.display()))?;
    let status = Command::new("rustc")
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
        .arg(&source_path)
        .output()
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
/// there. Returns the path relative to the project folder.
pub fn create(project_dir: &Path, relative: &str, actor: &str) -> Result<String, String> {
    if !is_valid_path(relative) {
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
    use super::*;

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
    fn the_stamp_changes_with_the_abi_the_toolchain_or_the_source() {
        let base = stamp_for("rustc 1.90.0", "fn main() {}");
        assert_ne!(base, stamp_for("rustc 1.91.0", "fn main() {}"));
        assert_ne!(base, stamp_for("rustc 1.90.0", "fn main() {} "));
    }
}
