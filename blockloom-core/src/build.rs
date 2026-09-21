//! Building a project into a game that runs on its own.
//!
//! A build is the player binary, the project's [`crate::pack::GamePack`], its
//! assets and its compiled scripts, laid out in one folder (see [`crate::pack`]
//! for the shape). Nothing here compiles the blocks - the player runs the same
//! VM the editor does - so a build costs a file copy and works on any machine,
//! with or without a Rust toolchain.
//!
//! Only the platform doing the building is a target today: the player copied
//! is the one beside the editor. [`player_binary`] looks in `players/<target>/`
//! first so other platforms' payloads can be dropped in later without moving
//! anything else.

use crate::pack::{self, GamePack};
use crate::project::{self, Project};
use crate::script;
use std::path::{Path, PathBuf};

/// Where a build landed, and what went into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Build {
    /// The build folder itself.
    pub dir: PathBuf,
    /// The renamed player, which is what a player double-clicks.
    pub binary: PathBuf,
    pub assets: usize,
    pub scripts: usize,
}

/// This machine's target triple, as the `players/` layout spells it. Built
/// from what `cfg!` knows rather than a build script, so an unrecognised
/// combination reads as unknown instead of silently claiming to be something.
pub fn host_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => "unknown",
    }
}

/// The player to copy into a build: a payload staged in `players/<target>/`
/// beside this executable if there is one, otherwise `fallback` - which the
/// caller names, since it is the runtime the editor itself plays with.
pub fn player_binary(fallback: &Path) -> Result<PathBuf, String> {
    let staged = fallback.file_name().and_then(|name| {
        let path = std::env::current_exe()
            .ok()?
            .parent()?
            .join("players")
            .join(host_target())
            .join(name);
        path.is_file().then_some(path)
    });
    if let Some(staged) = staged {
        return Ok(staged);
    }
    if fallback.is_file() {
        return Ok(fallback.to_path_buf());
    }
    Err(format!(
        "There is no player to build with: expected {}. Build the whole workspace, not just the editor.",
        fallback.display()
    ))
}

/// What the build folder is called, and what the binary inside it is called.
pub fn build_name(project: &Project) -> String {
    project::folder_name(&project.name)
}

/// Builds `project` into a folder under `parent`, replacing a previous build
/// of the same name.
///
/// Scripts are shipped as the libraries the editor already built, so whoever
/// calls this compiles them first: a script with no library is an error here
/// rather than an actor that quietly does nothing in the shipped game.
pub fn build(
    project: &Project,
    project_dir: &Path,
    player: &Path,
    parent: &Path,
) -> Result<Build, String> {
    let name = build_name(project);
    let dir = parent.join(&name);
    clear_build_dir(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let binary = dir.join(binary_name(&name));
    std::fs::copy(player, &binary).map_err(|e| {
        format!(
            "couldn't copy the player {} -> {}: {e}",
            player.display(),
            binary.display()
        )
    })?;
    // Copying a file doesn't carry its mode everywhere, and a game nobody can
    // execute isn't one.
    make_executable(&binary)?;

    let game = pack::game_dir(&dir);
    std::fs::create_dir_all(&game).map_err(|e| format!("{}: {e}", game.display()))?;
    GamePack::new(project.clone()).write(&pack::pack_path(&game))?;

    let assets = copy_assets(project_dir, &game)?;
    let scripts = copy_scripts(project, project_dir, &game)?;

    Ok(Build {
        dir,
        binary,
        assets,
        scripts,
    })
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Makes room for a fresh build. A folder that isn't a previous build is left
/// alone: this deletes things, and the only thing it may delete is its own
/// output.
fn clear_build_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    if !pack::pack_path(&pack::game_dir(dir)).is_file() {
        return Err(format!(
            "{} is already there and isn't a built game, so it won't be replaced",
            dir.display()
        ));
    }
    std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Copies the project's `assets/` into the build, minus the script sources -
/// what a built game runs is the library beside them, not the `.rs`.
fn copy_assets(project_dir: &Path, game: &Path) -> Result<usize, String> {
    let from = project_dir.join(project::ASSETS_DIR);
    if !from.is_dir() {
        return Ok(0);
    }
    let scripts = script::scripts_dir(project_dir);
    copy_tree(&from, &game.join(project::ASSETS_DIR), &|path| {
        !(path.starts_with(&scripts) && path.extension().is_some_and(|ext| ext == "rs"))
    })
}

/// Copies each scripted actor's built library to where the runtime looks for
/// it, which is the same place inside the build as inside a project.
fn copy_scripts(project: &Project, project_dir: &Path, game: &Path) -> Result<usize, String> {
    let mut paths: Vec<&str> = project
        .actors
        .iter()
        .filter_map(|actor| actor.components.script())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    if paths.is_empty() {
        return Ok(0);
    }

    let build = script::build_dir(game);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;
    for relative in &paths {
        let library = script::library_path(project_dir, relative);
        if !library.is_file() {
            return Err(format!(
                "{relative} hasn't been built, so the game would ship without it"
            ));
        }
        let Some(file) = library.file_name() else {
            continue;
        };
        let to = build.join(file);
        std::fs::copy(&library, &to)
            .map_err(|e| format!("{} -> {}: {e}", library.display(), to.display()))?;
        make_executable(&to)?;
    }
    Ok(paths.len())
}

/// Copies a folder recursively, keeping the files `wanted` says yes to.
/// Returns how many files were copied.
fn copy_tree(from: &Path, to: &Path, wanted: &dyn Fn(&Path) -> bool) -> Result<usize, String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let mut copied = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", from.display()))?;
        let path = entry.path();
        if !wanted(&path) {
            continue;
        }
        let Some(file) = path.file_name() else {
            continue;
        };
        let target = to.join(file);
        if path.is_dir() {
            copied += copy_tree(&path, &target, wanted)?;
        } else {
            std::fs::copy(&path, &target)
                .map_err(|e| format!("{} -> {}: {e}", path.display(), target.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Mode;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "blockloom-build-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A project folder with one asset, one script source, and a stand-in for
    /// the player binary.
    fn a_project(root: &Path) -> (Project, PathBuf, PathBuf) {
        let project_dir = root.join("project");
        std::fs::create_dir_all(project_dir.join("assets/sprites")).unwrap();
        std::fs::create_dir_all(script::scripts_dir(&project_dir)).unwrap();
        std::fs::write(project_dir.join("assets/sprites/ball.png"), b"png").unwrap();
        std::fs::write(project_dir.join("assets/scripts/player.rs"), b"// rust").unwrap();

        let player = root.join("player-binary");
        std::fs::write(&player, b"MZ").unwrap();

        (
            Project::starter("Pond Game", Mode::TwoD),
            project_dir,
            player,
        )
    }

    #[test]
    fn a_build_is_a_binary_beside_a_pack_and_the_projects_assets() {
        let root = temp("layout");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");

        let built = build(&project, &project_dir, &player, &out).unwrap();

        assert_eq!(built.dir, out.join("Pond Game"));
        assert!(built.binary.is_file());
        assert!(
            built
                .binary
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Pond Game")
        );
        let game = pack::game_dir(&built.dir);
        assert!(pack::pack_path(&game).is_file());
        assert!(game.join("assets/sprites/ball.png").is_file());
        assert_eq!(built.assets, 1);
        // The source stays behind; a build runs the library, not the `.rs`.
        assert!(!game.join("assets/scripts/player.rs").exists());

        let pack = GamePack::read(&pack::pack_path(&game)).unwrap();
        assert_eq!(pack.title(), "Pond Game");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn building_twice_replaces_the_first_build() {
        let root = temp("replace");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");

        let first = build(&project, &project_dir, &player, &out).unwrap();
        std::fs::write(first.dir.join("leftover.txt"), b"old").unwrap();
        let second = build(&project, &project_dir, &player, &out).unwrap();

        assert_eq!(first.dir, second.dir);
        assert!(!second.dir.join("leftover.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_isnt_a_build_is_never_replaced() {
        let root = temp("stranger");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");
        std::fs::create_dir_all(out.join("Pond Game")).unwrap();
        std::fs::write(out.join("Pond Game/taxes.txt"), b"mine").unwrap();

        let error = build(&project, &project_dir, &player, &out).unwrap_err();

        assert!(error.contains("isn't a built game"), "{error}");
        assert!(out.join("Pond Game/taxes.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
