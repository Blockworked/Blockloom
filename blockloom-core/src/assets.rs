//! A project folder's files, as the editor's asset tray sees them.
//!
//! A project is a folder, so its assets are just files in it: the tray lists
//! them, makes folders, imports, renames, moves and deletes. Every path
//! crossing this boundary is relative to the project folder and spelled with
//! forward slashes (`assets/sprites/player.png`), the same way a `Script`
//! component names its file - so what the tray hands an input is what the
//! document stores.
//!
//! Nothing here trusts a path it is given: [`normalize`] is the only way in,
//! and it refuses anything that would climb out of the folder.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The biggest file the editor will read back for a preview. Thumbnails go
/// over the command channel as base64, so a 50MB texture is not worth it.
pub const MAX_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;

/// What an entry is, so the tray can pick an icon and an input can refuse the
/// wrong sort of file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Folder,
    Image,
    Audio,
    Font,
    Model,
    Script,
    Shader,
    Text,
    /// High dynamic range images (`.hdr`, `.exr`): skies, IBL, emissive masks.
    Hdr,
    /// 3D data: colour grading LUTs (`.cube`).
    Volume,
    /// Photometric light profiles (`.ies`).
    Light,
    /// Headerless height samples (`.r16`, `.r32`, `.raw`).
    Height,
    Other,
}

/// One row in the tray.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetEntry {
    /// The file's own name, with its extension.
    pub name: String,
    /// Where it is, relative to the project folder.
    pub path: String,
    pub kind: AssetKind,
    /// Bytes, 0 for a folder.
    pub size: u64,
    /// Unix seconds, 0 if the filesystem wouldn't say.
    pub modified: u64,
    /// Whether the editor refuses to rename, move or delete it - the project
    /// document itself.
    pub protected: bool,
}

/// A name a file can have: not empty, not a path, and nothing Windows would
/// reject either.
pub fn is_valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed != "."
        && trimmed != ".."
        && !trimmed.starts_with('.')
        && !trimmed.ends_with('.')
        && !trimmed.chars().any(|c| {
            matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
        })
}

/// A project-relative path spelled the one way the rest of the app expects:
/// forward slashes, no empty or dotted parts, nothing absolute. The project
/// folder itself is `""`. `None` means the path was not one a project can
/// hold.
pub fn normalize(relative: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in relative.split(['/', '\\']) {
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." || part.contains(':') || part.chars().any(char::is_control) {
            return None;
        }
        parts.push(part);
    }
    Some(parts.join("/"))
}

/// Where a project-relative path points on disk, or `None` if it isn't one.
/// The runtime uses this to load an image or a font: Bevy's asset root is the
/// process's own folder, not the project's.
pub fn resolve(project_dir: &Path, relative: &str) -> Option<PathBuf> {
    let relative = normalize(relative)?;
    let mut path = project_dir.to_path_buf();
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    Some(path)
}

fn resolve_or_err(project_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    resolve(project_dir, relative)
        .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))
}

/// The parent of a project-relative path, as another one - `""` at the top.
pub fn parent_of(relative: &str) -> String {
    match relative.rfind('/') {
        Some(cut) => relative[..cut].to_string(),
        None => String::new(),
    }
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// The project document, which the tray shows but won't let anyone move: the
/// app finds a project by that exact name.
fn is_protected(relative: &str) -> bool {
    relative == crate::project::PROJECT_FILE
}

/// What the tray calls a file, from its extension.
pub fn kind_of(name: &str) -> AssetKind {
    let extension = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ktx2" | "basis" | "tga" | "dds" => {
            AssetKind::Image
        }
        "ogg" | "wav" | "mp3" | "flac" => AssetKind::Audio,
        "ttf" | "otf" => AssetKind::Font,
        "gltf" | "glb" | "obj" | "fbx" => AssetKind::Model,
        "rs" => AssetKind::Script,
        "wgsl" | "wesl" | "shader" | "hlsl" => AssetKind::Shader,
        "txt" | "json" | "toml" | "md" | "csv" | "ron" | "yaml" | "yml" => AssetKind::Text,
        "hdr" | "exr" => AssetKind::Hdr,
        "cube" => AssetKind::Volume,
        "ies" => AssetKind::Light,
        "r16" | "r32" | "raw" => AssetKind::Height,
        _ => AssetKind::Other,
    }
}

/// The media type a preview data URL carries. Only the kinds the editor
/// actually shows need one.
fn media_type(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tga" => "image/x-tga",
        _ => "application/octet-stream",
    }
}

fn modified_at(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Files the script tooling owns at the project root: the `Cargo.toml` synced
/// for rust-analyzer, plus the `Cargo.lock` and `target/` a `cargo check`
/// leaves behind. They are not assets, so the tray leaves them out - a
/// same-named file deeper in the tree is still listed.
fn is_hidden_at_root(relative: &str, name: &str) -> bool {
    relative.is_empty() && matches!(name, "Cargo.toml" | "Cargo.lock" | "target")
}

/// Everything in one folder of a project: folders first, then files, each run
/// alphabetical. Dotted names are left out - `.blockloom` is the build cache,
/// not an asset.
pub fn list(project_dir: &Path, relative: &str) -> Result<Vec<AssetEntry>, String> {
    let relative = normalize(relative).ok_or("That isn't a folder in this project")?;
    let dir = resolve_or_err(project_dir, &relative)?;
    let read = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || is_hidden_at_root(&relative, &name) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let path = join(&relative, &name);
        let folder = metadata.is_dir();
        entries.push(AssetEntry {
            kind: if folder {
                AssetKind::Folder
            } else {
                kind_of(&name)
            },
            size: if folder { 0 } else { metadata.len() },
            modified: modified_at(&metadata),
            protected: is_protected(&path),
            name,
            path,
        });
    }
    entries.sort_by(|a, b| {
        let folder = (b.kind == AssetKind::Folder).cmp(&(a.kind == AssetKind::Folder));
        folder.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// A name nothing in `parent` has yet - `player.png`, then `player 2.png`.
pub fn unused_name(project_dir: &Path, parent: &str, name: &str) -> String {
    let taken = |candidate: &str| {
        resolve(project_dir, &join(parent, candidate)).is_some_and(|path| path.exists())
    };
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.').filter(|cut| *cut > 0) {
        Some(cut) => (&name[..cut], &name[cut..]),
        None => (name, ""),
    };
    (2..)
        .map(|n| format!("{stem} {n}{extension}"))
        .find(|candidate| !taken(candidate))
        .expect("an unused name always exists")
}

/// Makes a folder under `parent`, returning where it landed.
pub fn create_folder(project_dir: &Path, parent: &str, name: &str) -> Result<String, String> {
    let parent = normalize(parent).ok_or("That isn't a folder in this project")?;
    let name = name.trim();
    if !is_valid_name(name) {
        return Err(format!("\"{name}\" isn't a name a folder can have"));
    }
    let path = join(&parent, &unused_name(project_dir, &parent, name));
    let full = resolve_or_err(project_dir, &path)?;
    std::fs::create_dir_all(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    Ok(path)
}

/// Makes a file under `parent` holding `contents`, returning where it landed.
pub fn create_file(
    project_dir: &Path,
    parent: &str,
    name: &str,
    contents: &str,
) -> Result<String, String> {
    let parent = normalize(parent).ok_or("That isn't a folder in this project")?;
    let name = name.trim();
    if !is_valid_name(name) {
        return Err(format!("\"{name}\" isn't a name a file can have"));
    }
    let path = join(&parent, &unused_name(project_dir, &parent, name));
    let full = resolve_or_err(project_dir, &path)?;
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&full, contents).map_err(|e| format!("{}: {e}", full.display()))?;
    Ok(path)
}

/// Copies files from anywhere on the machine into `parent`, returning where
/// each landed. A folder is copied whole.
pub fn import(
    project_dir: &Path,
    parent: &str,
    sources: &[PathBuf],
) -> Result<Vec<String>, String> {
    let parent = normalize(parent).ok_or("That isn't a folder in this project")?;
    let dir = resolve_or_err(project_dir, &parent)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut landed = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{}: no file name", source.display()))?;
        if !is_valid_name(&name) {
            return Err(format!("\"{name}\" isn't a name this project can hold"));
        }
        let path = join(&parent, &unused_name(project_dir, &parent, &name));
        let target = resolve_or_err(project_dir, &path)?;
        if source.is_dir() {
            copy_tree(source, &target)?;
        } else {
            std::fs::copy(source, &target)
                .map_err(|e| format!("{} -> {}: {e}", source.display(), target.display()))?;
        }
        landed.push(path);
    }
    Ok(landed)
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))?;
    let read = std::fs::read_dir(source).map_err(|e| format!("{}: {e}", source.display()))?;
    for entry in read.flatten() {
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// Renames one entry in place, returning its new path.
pub fn rename(project_dir: &Path, relative: &str, name: &str) -> Result<String, String> {
    let relative = normalize(relative).ok_or("That isn't a path in this project")?;
    if relative.is_empty() || is_protected(&relative) {
        return Err("That one can't be renamed".to_string());
    }
    let name = name.trim();
    if !is_valid_name(name) {
        return Err(format!("\"{name}\" isn't a name a file can have"));
    }
    // The name it already has is a no-op, not a clash with itself.
    if relative.rsplit('/').next() == Some(name) {
        return Ok(relative);
    }
    let parent = parent_of(&relative);
    let from = resolve_or_err(project_dir, &relative)?;
    let path = join(&parent, &unused_name(project_dir, &parent, name));
    let to = resolve_or_err(project_dir, &path)?;
    std::fs::rename(&from, &to)
        .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
    Ok(path)
}

/// Moves one entry into `parent`, returning its new path. A folder can't be
/// moved into itself.
pub fn move_to(project_dir: &Path, relative: &str, parent: &str) -> Result<String, String> {
    let relative = normalize(relative).ok_or("That isn't a path in this project")?;
    let parent = normalize(parent).ok_or("That isn't a folder in this project")?;
    if relative.is_empty() || is_protected(&relative) {
        return Err("That one can't be moved".to_string());
    }
    if parent == relative || parent.starts_with(&format!("{relative}/")) {
        return Err("A folder can't be moved inside itself".to_string());
    }
    if parent_of(&relative) == parent {
        return Ok(relative);
    }
    let name = relative.rsplit('/').next().unwrap_or(&relative).to_string();
    let from = resolve_or_err(project_dir, &relative)?;
    let path = join(&parent, &unused_name(project_dir, &parent, &name));
    let to = resolve_or_err(project_dir, &path)?;
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::rename(&from, &to)
        .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
    Ok(path)
}

/// Deletes one entry, a folder and its contents included.
pub fn delete(project_dir: &Path, relative: &str) -> Result<(), String> {
    let relative = normalize(relative).ok_or("That isn't a path in this project")?;
    if relative.is_empty() || is_protected(&relative) {
        return Err("That one can't be deleted".to_string());
    }
    let path = resolve_or_err(project_dir, &relative)?;
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    }
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// A file's bytes as a `data:` URL, for the tray's thumbnails - the editor is
/// a web page and has no other way to see them.
pub fn data_url(project_dir: &Path, relative: &str) -> Result<String, String> {
    let relative = normalize(relative).ok_or("That isn't a path in this project")?;
    let path = resolve_or_err(project_dir, &relative)?;
    let metadata = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Err(format!("{relative} is too big to preview"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let encoded = base64_encode(&bytes);
    Ok(format!("data:{};base64,{encoded}", media_type(&relative)))
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_refuses_anything_that_climbs_out() {
        assert_eq!(
            normalize("assets/player.png").as_deref(),
            Some("assets/player.png")
        );
        assert_eq!(
            normalize("assets\\player.png").as_deref(),
            Some("assets/player.png")
        );
        assert_eq!(normalize("").as_deref(), Some(""));
        assert_eq!(
            normalize("assets//player.png").as_deref(),
            Some("assets/player.png")
        );
        assert_eq!(normalize("../etc/passwd"), None);
        assert_eq!(normalize("assets/./player.png"), None);
        assert_eq!(normalize("C:/Windows"), None);
    }

    #[test]
    fn names_are_checked_the_way_a_path_needs() {
        assert!(is_valid_name("player.png"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name(".."));
        assert!(!is_valid_name(".hidden"));
        assert!(!is_valid_name("a/b"));
        assert!(!is_valid_name("what?.png"));
    }

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(kind_of("player.PNG"), AssetKind::Image);
        assert_eq!(kind_of("jump.ogg"), AssetKind::Audio);
        assert_eq!(kind_of("player.rs"), AssetKind::Script);
        assert_eq!(kind_of("notes"), AssetKind::Other);
        assert_eq!(kind_of("sky.EXR"), AssetKind::Hdr);
        assert_eq!(kind_of("grade.cube"), AssetKind::Volume);
        assert_eq!(kind_of("spot.ies"), AssetKind::Light);
        assert_eq!(kind_of("island.r16"), AssetKind::Height);
    }

    #[test]
    fn folders_files_and_moves_stay_inside_the_project() {
        let dir = std::env::temp_dir().join(format!("blockloom-assets-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let sprites = create_folder(&dir, "assets", "sprites").unwrap();
        assert_eq!(sprites, "assets/sprites");
        let again = create_folder(&dir, "assets", "sprites").unwrap();
        assert_eq!(again, "assets/sprites 2");

        let notes = create_file(&dir, "", "notes.txt", "hello").unwrap();
        let moved = move_to(&dir, &notes, "assets/sprites").unwrap();
        assert_eq!(moved, "assets/sprites/notes.txt");
        let renamed = rename(&dir, &moved, "readme.txt").unwrap();
        assert_eq!(renamed, "assets/sprites/readme.txt");
        // Renaming a file to the name it already has leaves it alone rather
        // than treating the file as its own clash.
        assert_eq!(rename(&dir, &renamed, "readme.txt").unwrap(), renamed);

        let listed = list(&dir, "assets/sprites").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, AssetKind::Text);

        assert!(move_to(&dir, "assets", "assets/sprites").is_err());
        assert!(delete(&dir, "../elsewhere").is_err());
        delete(&dir, "assets").unwrap();
        assert!(!dir.join("assets").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn root_tooling_files_are_hidden_but_nothing_else_is() {
        let dir = std::env::temp_dir().join(format!("blockloom-assets-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), b"[package]").unwrap();
        std::fs::write(dir.join("Cargo.lock"), b"lock").unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("project.blockloom"), b"{}").unwrap();
        // Same names deeper in the tree are somebody's files, not tooling's.
        std::fs::write(dir.join("assets/Cargo.toml"), b"[package]").unwrap();

        let listed = list(&dir, "").unwrap();
        let root: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
        assert!(root.contains(&"assets"), "{root:?}");
        assert!(root.contains(&"project.blockloom"), "{root:?}");
        assert!(!root.contains(&"Cargo.toml"), "{root:?}");
        assert!(!root.contains(&"Cargo.lock"), "{root:?}");
        assert!(!root.contains(&"target"), "{root:?}");

        let listed_inner = list(&dir, "assets").unwrap();
        let inner: Vec<&str> = listed_inner
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert!(inner.contains(&"Cargo.toml"), "{inner:?}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
