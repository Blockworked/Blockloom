//! How this process was started: as the editor's child, or as a built game.
//!
//! The two differ in where the world comes from and in who is listening. The
//! editor's child waits for a project to arrive down its stdin and reports
//! everything back; a built game reads the pack sitting beside its own binary
//! (see `blockloom_core::pack`), presses its own green flag, and talks to
//! nobody. Everything after that - the plugins, the schedules, the systems -
//! is the same world either way.

use crate::bridge;
use crate::engine::Engine;
use blockloom_core::pack::{self, GamePack};
use blockloom_core::scene::Mode;
use blockloom_protocol::EditorMessage;
use std::path::{Path, PathBuf};

/// What this process is.
pub enum Launch {
    /// Started by the editor: the world is whatever comes down the pipe, and
    /// the dimension is a launch argument because the plugin set depends on it.
    Editor { mode: Mode },
    /// Started on its own, from a pack. The dimension is in the pack, so
    /// nothing has to be told.
    Player { pack: Box<GamePack>, dir: PathBuf },
}

impl Launch {
    /// Reads the command line, and the folder this binary sits in. A pack
    /// beside the binary is what makes a built game a built game.
    pub fn from_args(args: impl Iterator<Item = String>) -> Self {
        let args: Vec<String> = args.collect();
        if let Some(asked) = flag(&args, "--play") {
            return Self::player(Path::new(&asked));
        }
        match pack::beside_exe() {
            Some(path) => Self::player(&path),
            None => Self::Editor {
                mode: mode_from_args(&args),
            },
        }
    }

    /// Loads the pack `path` points at, or gives up loudly: a built game that
    /// can't find its own world has nothing else to do.
    fn player(path: &Path) -> Self {
        let Some(pack_file) = locate_pack(path) else {
            fatal(&format!(
                "{} doesn't hold a Blockloom game - expected a {} in it",
                path.display(),
                pack::PACK_FILE
            ));
        };
        let pack = match GamePack::read(&pack_file) {
            Ok(pack) => pack,
            Err(e) => fatal(&e),
        };
        // The pack's own folder is the project folder as far as the rest of
        // the runtime is concerned, so assets and script libraries resolve
        // exactly as they do when the editor plays a project.
        let dir = pack_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::Player {
            pack: Box::new(pack),
            dir,
        }
    }

    /// Which pipelines to build. A built game takes it from its own document
    /// rather than an argument, so it can never be launched into the wrong one.
    pub fn mode(&self) -> Mode {
        match self {
            Self::Editor { mode } => *mode,
            Self::Player { pack, .. } => pack.project.world.mode,
        }
    }

    /// Whether this run may leave SDR: a build made SDR-only never does.
    pub fn allows_hdr(&self) -> bool {
        match self {
            Self::Player { pack, .. } => pack.hdr,
            _ => true,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Editor { .. } => "Blockloom".to_string(),
            Self::Player { pack, .. } => pack.title().to_string(),
        }
    }

    /// Builds the engine this launch implies. The editor's child listens; a
    /// built game hands itself the Load and Start the editor would have sent,
    /// so both go through one code path in `world::pump_editor`.
    pub fn into_engine(self) -> Engine {
        let mode = self.mode();
        match self {
            Self::Editor { .. } => Engine::new(bridge::listen(), mode),
            Self::Player { pack, dir } => {
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(EditorMessage::Load {
                    project: Box::new(pack.project),
                    dir: Some(dir.to_string_lossy().into_owned()),
                });
                let _ = tx.send(EditorMessage::Start);
                let mut engine = Engine::new(rx, mode);
                engine.link = Some(tx);
                engine
            }
        }
    }
}

/// A pack from what someone pointed at: the file itself, the `game/` folder
/// holding it, or the build folder holding that.
fn locate_pack(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let inside = pack::pack_path(path);
    if inside.is_file() {
        return Some(inside);
    }
    let nested = pack::pack_path(&pack::game_dir(path));
    if nested.is_file() {
        return Some(nested);
    }
    let bundled = pack::pack_path(&pack::game_dir(&path.join("Contents/Resources")));
    if bundled.is_file() {
        return Some(bundled);
    }
    std::fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| candidate.extension().is_some_and(|ext| ext == "app"))
        .map(|app| pack::pack_path(&pack::game_dir(&app.join("Contents/Resources"))))
        .find(|candidate| candidate.is_file())
}

fn fatal(message: &str) -> ! {
    eprintln!("blockloom: {message}");
    std::process::exit(1)
}

/// `--name value` or `--name=value`.
fn flag(args: &[String], name: &str) -> Option<String> {
    let mut wants_value = false;
    for arg in args {
        if wants_value {
            return Some(arg.clone());
        }
        match arg.split_once('=') {
            Some((key, value)) if key == name => return Some(value.to_string()),
            _ if arg == name => wants_value = true,
            _ => {}
        }
    }
    None
}

/// `--mode 3d` (or `--mode=3d`) picks the 3D pipeline; 2D is the default.
fn mode_from_args(args: &[String]) -> Mode {
    match flag(args, "--mode") {
        Some(value) => parse_mode(&value),
        None => Mode::TwoD,
    }
}

fn parse_mode(value: &str) -> Mode {
    match value.trim().to_lowercase().as_str() {
        "3d" | "threed" | "3" => Mode::ThreeD,
        _ => Mode::TwoD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn the_mode_argument_is_read_either_way_round() {
        assert_eq!(mode_from_args(&args(&["--mode", "3d"])), Mode::ThreeD);
        assert_eq!(mode_from_args(&args(&["--mode=3D"])), Mode::ThreeD);
        assert_eq!(mode_from_args(&args(&["--mode", "2d"])), Mode::TwoD);
        assert_eq!(mode_from_args(&args(&[])), Mode::TwoD);
    }

    #[test]
    fn a_pack_is_found_from_the_file_its_folder_or_the_build() {
        let root = std::env::temp_dir().join(format!("blockloom-locate-{}", std::process::id()));
        let game = pack::game_dir(&root);
        std::fs::create_dir_all(&game).unwrap();
        let file = pack::pack_path(&game);
        std::fs::write(&file, b"{}").unwrap();

        assert_eq!(locate_pack(&file), Some(file.clone()));
        assert_eq!(locate_pack(&game), Some(file.clone()));
        assert_eq!(locate_pack(&root), Some(file.clone()));
        assert_eq!(locate_pack(&root.join("nowhere")), None);

        let app = root.join("Pond.app");
        let bundled_game = app.join("Contents/Resources/game");
        std::fs::create_dir_all(&bundled_game).unwrap();
        let bundled = pack::pack_path(&bundled_game);
        std::fs::write(&bundled, b"{}").unwrap();
        std::fs::remove_file(&file).unwrap();
        assert_eq!(locate_pack(&app), Some(bundled.clone()));
        assert_eq!(locate_pack(&root), Some(bundled));
        let _ = std::fs::remove_dir_all(&root);
    }
}
