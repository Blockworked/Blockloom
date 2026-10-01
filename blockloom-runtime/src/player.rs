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
    /// Running in a browser: the pack arrives as JSON inlined in the page, and
    /// the game folder is the empty path, which names the files the page
    /// mounted (see `blockloom_core::vfs`). Saves live in localStorage.
    Web { pack: Box<GamePack> },
    /// Running inside its APK on a phone or tablet: the pack comes from the
    /// APK's assets (see `crate::android`), the game folder is the empty
    /// path like a web run, and saves live in the app's data dir. Always
    /// SDR, like the browser.
    Android { pack: Box<GamePack> },
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
            Self::Player { pack, .. } | Self::Web { pack } | Self::Android { pack } => {
                pack.project.world.mode
            }
        }
    }

    /// Builds the player side of a launch from an already-parsed pack: the
    /// browser entry point's whole job. Web builds ship SDR-only, since no
    /// browser swapchain takes the HDR takeover.
    pub fn from_pack(mut pack: GamePack) -> Self {
        pack.hdr = false;
        Self::Web {
            pack: Box::new(pack),
        }
    }

    /// The same pack from the APK's assets: the Android entry point's whole
    /// job. Android builds ship SDR-only, like web ones.
    pub fn from_android(mut pack: GamePack) -> Self {
        pack.hdr = false;
        Self::Android {
            pack: Box::new(pack),
        }
    }

    /// Whether this run may leave SDR: a build made SDR-only never does, and
    /// neither does the browser or a phone.
    pub fn allows_hdr(&self) -> bool {
        match self {
            Self::Player { pack, .. } => pack.hdr,
            Self::Web { .. } | Self::Android { .. } => false,
            _ => true,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Editor { .. } => "Blockloom".to_string(),
            Self::Player { pack, .. } | Self::Web { pack } | Self::Android { pack } => {
                pack.title().to_string()
            }
        }
    }

    /// Builds the engine this launch implies. The editor's child listens; a
    /// built game hands itself the Load and Start the editor would have sent,
    /// so both go through one code path in `world::pump_editor`.
    pub fn into_engine(self) -> Engine {
        let mode = self.mode();
        match self {
            Self::Editor { .. } => Engine::new(bridge::listen(), mode),
            Self::Player { mut pack, dir } => {
                // A built game boots into its default scene, not wherever the
                // editor was looking at build time.
                pack.project.active_scene = pack.project.boot_scene_id();
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(EditorMessage::Load {
                    project: Box::new(pack.project),
                    dir: Some(dir.to_string_lossy().into_owned()),
                });
                // The plugin code the build shipped, hosted as the editor's
                // Play would host it.
                if !pack.plugins.is_empty() {
                    match crate::plugins::shipped_loadout(&dir, &pack.plugins) {
                        Ok(loadout) if !loadout.is_empty() => {
                            let _ = tx.send(EditorMessage::Plugins { loadout });
                        }
                        Ok(_) => {}
                        Err(problems) => fatal(&format!(
                            "this game's plugins can't run:\n- {}",
                            problems.join("\n- ")
                        )),
                    }
                }
                let _ = tx.send(EditorMessage::Start);
                let mut engine = Engine::new(rx, mode);
                engine.link = Some(tx);
                engine
            }
            // The game folder is the page's mounted files, reached through
            // relative paths; the green flag is the same. Android reads the
            // same way, through the asset-reader hook instead of a mount.
            Self::Web { mut pack } | Self::Android { mut pack } => {
                pack.project.active_scene = pack.project.boot_scene_id();
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(EditorMessage::Load {
                    project: Box::new(pack.project),
                    dir: Some(String::new()),
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
    // A built game that can't load has nothing else to do. On the web there
    // is no process to exit, so the message lands on the page instead.
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::show_error(message);
        panic!("blockloom: {message}");
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("blockloom: {message}");
        std::process::exit(1)
    }
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
    fn a_web_launch_takes_its_mode_from_the_pack_and_stays_sdr() {
        let mut pack = GamePack::new(blockloom_core::project::Project::starter(
            "Pond",
            blockloom_core::scene::Mode::ThreeD,
        ));
        pack.hdr = true;
        let launch = Launch::from_pack(pack);
        assert_eq!(launch.mode(), blockloom_core::scene::Mode::ThreeD);
        assert!(!launch.allows_hdr());
        assert_eq!(launch.title(), "Pond");
    }

    #[test]
    fn an_android_launch_takes_its_mode_from_the_pack_and_stays_sdr() {
        let mut pack = GamePack::new(blockloom_core::project::Project::starter(
            "Pond",
            blockloom_core::scene::Mode::TwoD,
        ));
        pack.hdr = true;
        let launch = Launch::from_android(pack);
        assert_eq!(launch.mode(), blockloom_core::scene::Mode::TwoD);
        assert!(!launch.allows_hdr());
        assert_eq!(launch.title(), "Pond");
        // Same synthetic Load and Start as every other player launch: the
        // pack's project arrives on the engine's channel before the first
        // pump, with the game folder as the empty path like a web run.
        let engine = launch.into_engine();
        let EditorMessage::Load { project, dir } = engine.incoming.try_recv().unwrap() else {
            panic!("an Android launch starts with Load");
        };
        assert_eq!(project.name, "Pond");
        assert_eq!(dir.as_deref(), Some(""));
        assert!(matches!(
            engine.incoming.try_recv().unwrap(),
            EditorMessage::Start
        ));
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
