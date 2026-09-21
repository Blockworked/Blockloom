//! What a built game carries: the document, packed beside the player binary.
//!
//! A Windows build is a folder like this; Linux adds launchers, while macOS
//! places the same `game/` under `Pond Game.app/Contents/Resources`:
//!
//! ```text
//! Pond Game/
//!   Pond Game.exe        the player - `blockloom-runtime`, renamed
//!   game/
//!     game.pack          this file: the whole document
//!     assets/...         the project's assets
//!     .blockloom/build/  native blocks and script libraries
//! ```
//!
//! The player finds `game/game.pack` next to its executable, or in the app's
//! Resources folder on macOS. `game/` is handed to the runtime as the project
//! folder, so assets and built libraries keep their project spelling.

use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Bumped when the pack changes shape. A player refuses one it doesn't speak
/// rather than half-loading it.
pub const PACK_VERSION: u32 = 1;

/// The folder a build keeps everything but its binary in.
pub const GAME_DIR: &str = "game";

/// The pack itself, inside [`GAME_DIR`].
pub const PACK_FILE: &str = "game.pack";

/// A project, ready to run on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GamePack {
    /// [`PACK_VERSION`] at the time it was built.
    pub pack: u32,
    /// Which Blockloom built it, so a bug report can quote it.
    #[serde(default)]
    pub engine: String,
    pub project: Project,
}

impl GamePack {
    pub fn new(project: Project) -> Self {
        Self {
            pack: PACK_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_string(),
            project,
        }
    }

    /// What the window is called.
    pub fn title(&self) -> &str {
        &self.project.name
    }

    /// The key a game's saved data belongs under. The project's id rather
    /// than its name, so renaming a build doesn't orphan somebody's progress.
    pub fn save_id(&self) -> &str {
        &self.project.id
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut pack: GamePack =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if pack.pack != PACK_VERSION {
            return Err(format!(
                "{} is a version {} game pack, this player reads version {PACK_VERSION}",
                path.display(),
                pack.pack
            ));
        }
        pack.project.normalize();
        Ok(pack)
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The `game/` folder inside a build.
pub fn game_dir(build_dir: &Path) -> PathBuf {
    build_dir.join(GAME_DIR)
}

/// The pack inside a `game/` folder.
pub fn pack_path(game_dir: &Path) -> PathBuf {
    game_dir.join(PACK_FILE)
}

/// The pack this binary was shipped with, if it was shipped with one: the
/// question "am I a built game?" is this returning `Some`.
pub fn beside_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    let direct = pack_path(&game_dir(parent));
    if direct.is_file() {
        return Some(direct);
    }
    let bundled = pack_path(&game_dir(&parent.parent()?.join("Resources")));
    bundled.is_file().then_some(bundled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Mode;

    #[test]
    fn a_pack_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("blockloom-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = pack_path(&dir);

        let pack = GamePack::new(Project::starter("Pond", Mode::TwoD));
        pack.write(&path).unwrap();
        let read = GamePack::read(&path).unwrap();

        assert_eq!(read.pack, PACK_VERSION);
        assert_eq!(read.title(), "Pond");
        assert_eq!(read.save_id(), pack.save_id());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pack_from_another_version_is_refused_rather_than_half_read() {
        let dir = std::env::temp_dir().join(format!("blockloom-pack-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = pack_path(&dir);

        let mut pack = GamePack::new(Project::starter("Pond", Mode::TwoD));
        pack.pack = PACK_VERSION + 1;
        pack.write(&path).unwrap();

        let error = GamePack::read(&path).unwrap_err();
        assert!(error.contains("game pack"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
