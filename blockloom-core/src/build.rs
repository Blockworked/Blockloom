//! Building a project into a game that runs on its own.
//!
//! A build is the player binary, the project's [`crate::pack::GamePack`], its
//! assets, compiled scripts, and optional native block logic, laid out in one
//! folder (see [`crate::pack`] for the shape).
//!
//! Which platforms an install can build for is a question about what it has
//! beside it. The player is a native binary that nothing here can produce, so
//! one per platform is staged under `players/<triple>/` next to the editor
//! (`just player`), and the machine doing the building always has its own.
//! Scripts are the other half: they are native too, so a project with one can
//! only be built for a platform this machine's rustc can compile for.
//! [`targets`] answers both questions at once, which is what the Build dialog
//! shows.

use crate::codegen;
use crate::pack::{self, GamePack};
use crate::project::{self, Project};
use crate::script;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One platform a game can be built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The rustc target triple, which doubles as the `players/` folder name.
    pub triple: &'static str,
    /// What the Build dialog calls it.
    pub label: &'static str,
    /// Whether an executable there wants `.exe` on the end.
    pub windows: bool,
}

/// Every platform Blockloom knows how to lay a build out for. Whether one is
/// available on this machine is a separate question - see [`targets`].
pub const TARGETS: &[Target] = &[
    Target {
        triple: "x86_64-pc-windows-msvc",
        label: "Windows x64",
        windows: true,
    },
    Target {
        triple: "aarch64-pc-windows-msvc",
        label: "Windows ARM64",
        windows: true,
    },
    Target {
        triple: "x86_64-unknown-linux-gnu",
        label: "Linux x64",
        windows: false,
    },
    Target {
        triple: "aarch64-unknown-linux-gnu",
        label: "Linux ARM64",
        windows: false,
    },
    Target {
        triple: "x86_64-apple-darwin",
        label: "macOS Intel",
        windows: false,
    },
    Target {
        triple: "aarch64-apple-darwin",
        label: "macOS Apple Silicon",
        windows: false,
    },
];

/// The target `triple` names, if it is one Blockloom knows.
pub fn target(triple: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|target| target.triple == triple)
}

/// The platform doing the building. `None` means one Blockloom has no name
/// for, which leaves it unable to say what it would even be building.
pub fn host() -> Option<&'static Target> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    };
    target(triple)
}

pub fn is_host(target: &Target) -> bool {
    host().is_some_and(|host| host.triple == target.triple)
}

/// How a script is compiled for `target`: this machine's own build is the one
/// Play already made, anywhere else is a cross build of its own.
pub fn script_target(target: &Target) -> Option<&'static str> {
    (!is_host(target)).then_some(target.triple)
}

/// What one platform can do for this project right now, as the Build dialog
/// lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetStatus {
    pub triple: String,
    pub label: String,
    /// Whether this is the machine doing the building.
    pub host: bool,
    /// Whether a build for it would get anywhere.
    pub ready: bool,
    /// What it would build with, or what is missing.
    pub note: String,
    /// Whether this project can compile its blocks natively for the target.
    pub fast_ready: bool,
    /// Why native blocks are or are not available.
    pub fast_note: String,
}

/// Every platform, in the order the dialog lists them: this machine first,
/// since it is the one that always works.
///
/// `has_scripts` is whether the project has any Rust in it, which is what
/// decides whether a toolchain gets a say.
pub fn targets(
    has_scripts: bool,
    fast_source: Result<(), String>,
    fallback_player: &Path,
) -> Vec<TargetStatus> {
    let mut statuses: Vec<TargetStatus> = TARGETS
        .iter()
        .map(|target| status(target, has_scripts, &fast_source, fallback_player))
        .collect();
    statuses.sort_by_key(|status| !status.host);
    statuses
}

fn status(
    target: &Target,
    has_scripts: bool,
    fast_source: &Result<(), String>,
    fallback_player: &Path,
) -> TargetStatus {
    let host = is_host(target);
    let (ready, note) = match player_for(target, fallback_player) {
        None => (
            false,
            format!(
                "No player for this platform. Stage one in players/{}/ beside Blockloom.",
                target.triple
            ),
        ),
        Some(_) if !has_scripts => (
            true,
            if host {
                "This machine.".to_string()
            } else {
                "Using the staged player.".to_string()
            },
        ),
        // A script is native code like the player is, so it has to be built
        // for wherever the game is going.
        Some(_) => match script_target(target) {
            None => match script::toolchain_version() {
                Ok(_) => (true, "This machine, scripts included.".to_string()),
                Err(e) => (false, e),
            },
            Some(triple) => match script::target_installed(triple) {
                Ok(()) => (true, "Scripts will be cross-compiled for it.".to_string()),
                Err(e) => (false, format!("This project has scripts, and {e}.")),
            },
        },
    };
    let fast_toolchain = match script_target(target) {
        None => script::toolchain_version().map(|_| ()),
        Some(triple) => script::target_installed(triple),
    };
    let (fast_ready, fast_note) = if !ready {
        (
            false,
            "The platform is not available for a build.".to_string(),
        )
    } else if let Err(error) = fast_source {
        (false, error.clone())
    } else if let Err(error) = fast_toolchain {
        (false, error)
    } else {
        (
            true,
            "Blocks will be compiled to optimized native code.".to_string(),
        )
    };
    TargetStatus {
        triple: target.triple.to_string(),
        label: target.label.to_string(),
        host,
        ready,
        note,
        fast_ready,
        fast_note,
    }
}

/// The player to copy into a build for `target`: the payload staged for it,
/// or - for this machine only - `fallback`, which the caller names since it is
/// the runtime the editor itself plays with.
pub fn player_for(target: &Target, fallback: &Path) -> Option<PathBuf> {
    if let Some(staged) = staged_player(target, fallback) {
        return Some(staged);
    }
    if is_host(target) && fallback.is_file() {
        return Some(fallback.to_path_buf());
    }
    None
}

/// A payload under `players/<triple>/` beside this executable. It wins even
/// for this machine: it is the copy meant for shipping (see `just player`).
fn staged_player(target: &Target, fallback: &Path) -> Option<PathBuf> {
    let stem = fallback.file_stem()?.to_string_lossy().into_owned();
    let name = if target.windows {
        format!("{stem}.exe")
    } else {
        stem
    };
    let path = std::env::current_exe()
        .ok()?
        .parent()?
        .join("players")
        .join(target.triple)
        .join(name);
    path.is_file().then_some(path)
}

/// Where a build landed, and what went into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Build {
    /// The build folder itself.
    pub dir: PathBuf,
    /// The renamed player, which is what somebody double-clicks.
    pub binary: PathBuf,
    pub target: &'static str,
    pub assets: usize,
    pub scripts: usize,
    pub compiled: bool,
}

/// What the build folder is called. The platform is in the name because one
/// output folder holds a build per platform, and three folders called the
/// same thing would be three chances to ship the wrong one.
pub fn build_name(project: &Project, target: &Target) -> String {
    format!("{} ({})", project::folder_name(&project.name), target.label)
}

/// Builds `project` for `target` into a folder under `parent`, replacing a
/// previous build for the same platform.
///
/// Scripts are shipped as the libraries the editor already built for that
/// platform, so whoever calls this compiles them first: a script with no
/// library is an error here rather than an actor that quietly does nothing in
/// the shipped game.
pub fn build(
    project: &Project,
    project_dir: &Path,
    target: &'static Target,
    player: &Path,
    parent: &Path,
    fast: bool,
) -> Result<Build, String> {
    let dir = parent.join(build_name(project, target));
    clear_build_dir(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let binary = dir.join(binary_name(&project::folder_name(&project.name), target));
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
    let scripts = copy_scripts(project, project_dir, &game, target)?;
    let compiled = if fast {
        copy_logic(project_dir, &game, target)?;
        true
    } else {
        false
    };

    Ok(Build {
        dir,
        binary,
        target: target.triple,
        assets,
        scripts,
        compiled,
    })
}

fn copy_logic(project_dir: &Path, game: &Path, target: &Target) -> Result<(), String> {
    let library = codegen::library_path_for(project_dir, script_target(target));
    if !library.is_file() {
        return Err(format!(
            "the blocks haven't been compiled for {}, so a fast build can't be made",
            target.label
        ));
    }
    let build = script::build_dir(game);
    std::fs::create_dir_all(&build).map_err(|error| format!("{}: {error}", build.display()))?;
    let file = library
        .file_name()
        .ok_or_else(|| format!("{} has no file name", library.display()))?;
    let target = build.join(file);
    std::fs::copy(&library, &target)
        .map_err(|error| format!("{} -> {}: {error}", library.display(), target.display()))?;
    make_executable(&target)
}

fn binary_name(name: &str, target: &Target) -> String {
    if target.windows {
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

/// Copies each scripted actor's library for this platform to where the
/// runtime looks for it. The build's own copy is flat, the way a project's
/// is: which platform it was built for stops mattering once it has shipped.
fn copy_scripts(
    project: &Project,
    project_dir: &Path,
    game: &Path,
    target: &Target,
) -> Result<usize, String> {
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
        let library = script::library_path_for(project_dir, relative, script_target(target));
        if !library.is_file() {
            return Err(format!(
                "{relative} hasn't been built for {}, so the game would ship without it",
                target.label
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

    /// The platform under test: this machine, so the paths a build reads from
    /// are the ones Play writes to.
    fn a_target() -> &'static Target {
        host().expect("the tests run on a platform Blockloom knows")
    }

    #[test]
    fn a_build_is_a_binary_beside_a_pack_and_the_projects_assets() {
        let root = temp("layout");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");
        let target = a_target();

        let built = build(&project, &project_dir, target, &player, &out, false).unwrap();

        assert_eq!(built.dir, out.join(format!("Pond Game ({})", target.label)));
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

        let first = build(&project, &project_dir, a_target(), &player, &out, false).unwrap();
        std::fs::write(first.dir.join("leftover.txt"), b"old").unwrap();
        let second = build(&project, &project_dir, a_target(), &player, &out, false).unwrap();

        assert_eq!(first.dir, second.dir);
        assert!(!second.dir.join("leftover.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_fast_build_carries_the_native_block_library() {
        let root = temp("native-logic");
        let (project, project_dir, player) = a_project(&root);
        let library = codegen::library_path(&project_dir);
        std::fs::create_dir_all(library.parent().unwrap()).unwrap();
        std::fs::write(&library, b"native logic").unwrap();

        let built = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &root.join("out"),
            true,
        )
        .unwrap();

        assert!(built.compiled);
        assert!(
            codegen::library_path(&pack::game_dir(&built.dir)).is_file(),
            "the player must find native logic in its normal build folder"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_that_isnt_a_build_is_never_replaced() {
        let root = temp("stranger");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");
        let target = a_target();
        let taken = out.join(build_name(&project, target));
        std::fs::create_dir_all(&taken).unwrap();
        std::fs::write(taken.join("taxes.txt"), b"mine").unwrap();

        let error = build(&project, &project_dir, target, &player, &out, false).unwrap_err();

        assert!(error.contains("isn't a built game"), "{error}");
        assert!(taken.join("taxes.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn each_platform_gets_a_build_folder_of_its_own() {
        let project = Project::starter("Pond Game", Mode::TwoD);
        let names: Vec<String> = TARGETS
            .iter()
            .map(|target| build_name(&project, target))
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "{names:?}");
    }

    #[test]
    fn a_platform_with_no_player_staged_for_it_is_not_ready() {
        let root = temp("targets");
        let fallback = root.join("blockloom-runtime");
        std::fs::write(&fallback, b"MZ").unwrap();

        let statuses = targets(false, Ok(()), &fallback);
        let host = statuses.first().expect("at least one target");

        // This machine comes first and can always build, since the runtime the
        // editor plays with is a player.
        assert!(host.host);
        assert!(host.ready);
        // Nothing is staged in this test's folder, so no other platform is.
        for status in statuses.iter().filter(|status| !status.host) {
            assert!(!status.ready, "{status:?}");
            assert!(status.note.contains("players/"), "{status:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
