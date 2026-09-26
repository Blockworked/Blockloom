//! Reading a project's files. Natively that is the disk. In the browser there
//! is no disk: the single-file web build carries every file inside its page,
//! and the player mounts them here before the world starts (see
//! `blockloom-runtime/src/web.rs`), so a read that names `assets/cloud.png`
//! finds the same bytes either way.
//!
//! Only reads go through here. Writes stay `std::fs`, which a web build never
//! reaches for its own game files.

use std::io;
use std::path::Path;

/// The table key for `path`: forward slashes, no leading `./` or `/`. A web
/// run's project folder is the empty path, so a joined path is already one.
pub fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut rest = text.as_str();
    loop {
        if let Some(stripped) = rest.strip_prefix("./") {
            rest = stripped;
        } else if let Some(stripped) = rest.strip_prefix('/') {
            rest = stripped;
        } else {
            break;
        }
    }
    rest.to_string()
}

#[cfg(target_arch = "wasm32")]
mod table {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    pub static FILES: RwLock<Option<HashMap<String, Arc<[u8]>>>> = RwLock::new(None);

    pub fn get(key: &str) -> Option<Arc<[u8]>> {
        FILES.read().ok()?.as_ref()?.get(key).cloned()
    }
}

/// Mounts the files a web page carried, keyed by their path in the game
/// folder. Replaces whatever was mounted before.
#[cfg(target_arch = "wasm32")]
pub fn mount(files: impl IntoIterator<Item = (String, Vec<u8>)>) {
    let files = files
        .into_iter()
        .map(|(path, bytes)| (key(Path::new(&path)), std::sync::Arc::from(bytes)))
        .collect();
    if let Ok(mut table) = table::FILES.write() {
        *table = Some(files);
    }
}

/// Whether any files are mounted. Without them a web player fetches what it
/// needs from the server instead (a folder deploy).
#[cfg(target_arch = "wasm32")]
pub fn is_mounted() -> bool {
    table::FILES.read().is_ok_and(|table| table.is_some())
}

/// One mounted file, shared rather than copied. Always `None` natively.
pub fn mounted(path: &Path) -> Option<std::sync::Arc<[u8]>> {
    #[cfg(target_arch = "wasm32")]
    {
        table::get(&key(path))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = path;
        None
    }
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match mounted(path) {
        Some(bytes) => Ok(bytes.to_vec()),
        None => std::fs::read(path),
    }
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    match mounted(path) {
        Some(bytes) => String::from_utf8(bytes.to_vec())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        None => std::fs::read_to_string(path),
    }
}

pub fn is_file(path: &Path) -> bool {
    mounted(path).is_some() || path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_the_path_inside_the_game_folder() {
        assert_eq!(key(Path::new("assets/a.png")), "assets/a.png");
        assert_eq!(key(Path::new("./assets/a.png")), "assets/a.png");
        assert_eq!(key(Path::new("/assets/a.png")), "assets/a.png");
        assert_eq!(key(&Path::new("").join("assets/a.png")), "assets/a.png");
        assert_eq!(
            key(Path::new(".blockloom\\atlas.png")),
            ".blockloom/atlas.png"
        );
    }

    #[test]
    fn a_native_read_is_the_disk() {
        let dir = std::env::temp_dir().join(format!("blockloom-vfs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "hi").unwrap();
        assert!(is_file(&file));
        assert_eq!(read_to_string(&file).unwrap(), "hi");
        assert!(!is_file(&dir.join("missing")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
