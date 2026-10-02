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
pub const PACK_VERSION: u32 = 2;

/// The version a pack without plugins is written at, so players from before
/// plugins still run it. Only a pack that carries plugins is version 2.
const PACK_BASE: u32 = 1;

/// The folder a build keeps everything but its binary in.
pub const GAME_DIR: &str = "game";

/// The pack itself, inside [`GAME_DIR`].
pub const PACK_FILE: &str = "game.pack";

/// One plugin a built game carries, recorded so a player can check it has
/// what the game was made with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackedPlugin {
    pub id: String,
    pub version: String,
    /// The package's content hash, as the project's lock recorded it.
    pub hash: String,
    /// `declarative`, `portable` or `native`.
    pub tier: String,
    /// Where the plugin's files are, relative to the game folder.
    pub dir: String,
    /// The package paths copied there.
    pub files: Vec<String>,
}

/// The folder under `game/` that holds shipped plugins.
pub const PLUGINS_DIR: &str = "plugins";

/// A project, ready to run on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GamePack {
    /// [`PACK_VERSION`] at the time it was built.
    pub pack: u32,
    /// Which Blockloom built it, so a bug report can quote it.
    #[serde(default)]
    pub engine: String,
    pub project: Project,
    /// False when the target was built SDR-only: 8-bit frame, no HDR output.
    #[serde(default = "yes")]
    pub hdr: bool,
    /// The plugins the game ships with, by what the lock resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PackedPlugin>,
}

fn yes() -> bool {
    true
}

impl GamePack {
    pub fn new(project: Project) -> Self {
        Self {
            pack: PACK_BASE,
            engine: env!("CARGO_PKG_VERSION").to_string(),
            project,
            hdr: true,
            plugins: Vec::new(),
        }
    }

    /// Records the plugins this game ships with. A pack that carries any is
    /// written at the newer version, so an older player refuses it rather
    /// than running a game whose plugins it knows nothing about.
    pub fn with_plugins(mut self, plugins: Vec<PackedPlugin>) -> Self {
        if !plugins.is_empty() {
            self.pack = PACK_VERSION;
        }
        self.plugins = plugins;
        self
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
        Self::from_json(&text, &path.display().to_string())
    }

    /// Parses a pack from JSON text. The web player feeds it the pack inlined
    /// in its page rather than a file on disk.
    pub fn from_json(text: &str, origin: &str) -> Result<Self, String> {
        let mut pack: GamePack =
            serde_json::from_str(text).map_err(|e| format!("{origin}: {e}"))?;
        if !(PACK_BASE..=PACK_VERSION).contains(&pack.pack) {
            return Err(format!(
                "{origin} is a version {} game pack, this player reads version {PACK_VERSION}",
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

        assert_eq!(
            read.pack, PACK_BASE,
            "a pack without plugins stays readable by older players"
        );
        assert_eq!(read.title(), "Pond");
        assert_eq!(read.save_id(), pack.save_id());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pack_parses_from_json_text_without_touching_disk() {
        let pack = GamePack::new(Project::starter("Pond", Mode::TwoD));
        let text = serde_json::to_string(&pack).unwrap();

        let parsed = GamePack::from_json(&text, "inline").unwrap();
        assert_eq!(parsed.title(), "Pond");
        assert_eq!(parsed.save_id(), pack.save_id());

        let error = GamePack::from_json("not json", "inline").unwrap_err();
        assert!(error.contains("inline"), "{error}");
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

    #[test]
    fn a_pack_with_plugins_is_the_newer_version_and_keeps_them() {
        let plugin = PackedPlugin {
            id: "com.example.a".into(),
            version: "1.0.0".into(),
            hash: "0".repeat(64),
            tier: "declarative".into(),
            dir: "plugins/com.example.a".into(),
            files: vec!["schemas/main.json".into()],
        };
        let pack =
            GamePack::new(Project::starter("Pond", Mode::TwoD)).with_plugins(vec![plugin.clone()]);
        assert_eq!(pack.pack, PACK_VERSION);
        let text = serde_json::to_string(&pack).unwrap();
        let read = GamePack::from_json(&text, "inline").unwrap();
        assert_eq!(read.plugins, vec![plugin]);
        // No plugins: no field in the file, and the old version.
        let plain = GamePack::new(Project::starter("Pond", Mode::TwoD)).with_plugins(vec![]);
        assert_eq!(plain.pack, PACK_BASE);
        assert!(!serde_json::to_string(&plain).unwrap().contains("plugins"));
    }
}
