//! A package on disk: loading it, checking the payload against what the
//! manifest claims, sealing one (writing its hashes) and packing it into an
//! archive.

use blockloom_plugin_api::manifest::{MANIFEST_FILE, PluginManifest};
use blockloom_plugin_api::schema::Contributions;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Refuse to unpack more than this from one archive (zip bombs).
const MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

/// A package whose manifest, schemas and payload have all been verified.
#[derive(Debug, Clone)]
pub struct Package {
    pub root: PathBuf,
    pub manifest: PluginManifest,
    pub contributions: Contributions,
    /// Identifies this exact content: the manifest and every file's hash.
    pub content_hash: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// The identity of a package's content: the manifest bytes and the hash of
/// every other file, in path order. Renaming, adding or editing any file
/// changes it.
pub fn content_hash(manifest_bytes: &[u8], files: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"plugin.json\0");
    hasher.update(sha256_hex(manifest_bytes).as_bytes());
    for (path, hash) in files {
        hasher.update(b"\n");
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(hash.as_bytes());
    }
    hex(&hasher.finalize())
}

/// Hashes every file under `root` except the manifest, by package path.
/// Symlinks are refused: a package must be self-contained.
pub fn scan_files(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err(format!(
                    "{} is a symlink; packages must be self-contained",
                    path.display()
                ));
            }
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if relative == MANIFEST_FILE {
                continue;
            }
            out.insert(relative, hash_file(&path)?);
        }
    }
    Ok(out)
}

impl Package {
    /// Loads and fully verifies the package at `root`.
    pub fn load(root: &Path) -> Result<Package, String> {
        let manifest_path = root.join(MANIFEST_FILE);
        let bytes =
            fs::read(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        let text = std::str::from_utf8(&bytes).map_err(|e| format!("{MANIFEST_FILE}: {e}"))?;
        let manifest = PluginManifest::from_json(text)?;
        manifest.validate()?;
        let actual = scan_files(root)?;
        let mut problems = Vec::new();
        for (path, declared) in &manifest.files {
            match actual.get(path) {
                None => problems.push(format!("{path} is declared but missing")),
                Some(found) if found != declared => {
                    problems.push(format!("{path} does not match its declared hash"))
                }
                Some(_) => {}
            }
        }
        for path in actual.keys() {
            if !manifest.files.contains_key(path) {
                problems.push(format!("{path} is in the package but not declared"));
            }
        }
        if !problems.is_empty() {
            return Err(format!("{}: {}", manifest.id, problems.join("; ")));
        }
        let mut contributions = Contributions::default();
        for path in &manifest.contributions {
            let file = root.join(path);
            let text = fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?;
            let part: Contributions =
                serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
            contributions.merge(part);
        }
        contributions
            .check_definition()
            .map_err(|e| format!("{}: {e}", manifest.id))?;
        let content_hash = content_hash(&bytes, &manifest.files);
        Ok(Package {
            root: root.to_path_buf(),
            manifest,
            contributions,
            content_hash,
        })
    }

    /// The artifact hashes a lockfile records per target: each native library
    /// by triple, the portable module as `portable`.
    pub fn target_hashes(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (triple, entry) in &self.manifest.runtime.native {
            if let Some(hash) = self.manifest.files.get(&entry.library) {
                out.insert(triple.clone(), hash.clone());
            }
        }
        if let Some(portable) = &self.manifest.runtime.portable
            && let Some(hash) = self.manifest.files.get(&portable.module)
        {
            out.insert("portable".to_string(), hash.clone());
        }
        out
    }

    /// Component types nothing at run time reads.
    pub fn editor_only_types(&self) -> Vec<String> {
        self.contributions
            .components
            .iter()
            .chain(&self.contributions.resources)
            .filter(|c| c.editor_only)
            .map(|c| c.type_id.clone())
            .collect()
    }
}

/// Author tooling: recomputes `files` in the manifest from what is on disk.
/// Unknown manifest keys survive.
pub fn seal(root: &Path) -> Result<(), String> {
    let path = root.join(MANIFEST_FILE);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{MANIFEST_FILE}: {e}"))?;
    let files = scan_files(root)?;
    value["files"] = serde_json::to_value(&files).map_err(|e| e.to_string())?;
    let mut out = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    out.push('\n');
    fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    Package::load(root).map(|_| ())
}

/// Copies a directory tree. Refuses symlinks.
pub fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            return Err(format!("{} is a symlink", entry.path().display()));
        } else if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Packs a package folder into a zip archive, manifest first.
pub fn pack_archive(root: &Path, destination: &Path) -> Result<(), String> {
    Package::load(root)?;
    let mut paths: Vec<String> = scan_files(root)?.into_keys().collect();
    paths.insert(0, MANIFEST_FILE.to_string());
    let file =
        fs::File::create(destination).map_err(|e| format!("{}: {e}", destination.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for path in paths {
        zip.start_file(&path, options).map_err(|e| e.to_string())?;
        let bytes = fs::read(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Unpacks an archive into `destination`, refusing paths that escape it.
pub fn unpack_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", archive.display()))?;
    if zip.len() > MAX_FILES {
        return Err("archive has too many files".to_string());
    }
    let mut total = 0u64;
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        blockloom_plugin_api::id::validate_package_path(&name)
            .map_err(|e| format!("archive entry {e}"))?;
        total += entry.size();
        if total > MAX_UNPACKED_BYTES {
            return Err("archive unpacks to too much data".to_string());
        }
        let target = destination.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = fs::File::create(&target).map_err(|e| format!("{name}: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use serde_json::json;

    /// Writes a sealed declarative package and returns its root.
    pub fn declarative(dir: &Path, id: &str, version: &str, deps: serde_json::Value) -> PathBuf {
        let root = dir.join(format!("{id}-{version}"));
        fs::create_dir_all(root.join("schemas")).unwrap();
        fs::write(
            root.join("schemas/main.json"),
            serde_json::to_string(&json!({
                "components": [{
                    "type_id": "Health", "display_name": "Health",
                    "fields": [{"name": "hp", "type": "int", "min": 0, "max": 100, "default": 10}]
                }],
                "commands": [{
                    "name": "set_hp", "summary": "Set health",
                    "args": [{"name": "actor", "type": "actor"}, {"name": "value", "type": "int"}],
                    "action": {"do": "set_field", "component": "Health", "field": "hp"}
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(MANIFEST_FILE),
            serde_json::to_string(&json!({
                "format": 1, "id": id, "name": id, "version": version,
                "engine": ">=0.0.1", "dependencies": deps,
                "contributions": ["schemas/main.json"]
            }))
            .unwrap(),
        )
        .unwrap();
        seal(&root).unwrap();
        root
    }

    /// Writes a sealed portable package around the test wasm module: two
    /// module commands (`echo`, `spin`), a block for each, and one hook.
    pub fn portable(dir: &Path, id: &str, version: &str) -> PathBuf {
        let root = dir.join(format!("{id}-{version}"));
        fs::create_dir_all(root.join("schemas")).unwrap();
        fs::create_dir_all(root.join("portable")).unwrap();
        fs::write(
            root.join("portable/m.wasm"),
            crate::portable::fixture::wasm(),
        )
        .unwrap();
        fs::write(
            root.join("schemas/main.json"),
            serde_json::to_string(&json!({
                "hooks": [{"name": "tick", "stage": "fixed_simulation"}],
                "commands": [
                    {"name": "echo", "summary": "Answer the arguments.",
                     "args": [{"name": "actor", "type": "actor"}, {"name": "word", "type": "text", "default": "hi"}],
                     "action": {"do": "module", "op": "echo"}},
                    {"name": "spin", "summary": "Never returns.",
                     "action": {"do": "module", "op": "spin"}},
                    {"name": "set_hp", "summary": "Not a module op.",
                     "args": [{"name": "actor", "type": "actor"}, {"name": "value", "type": "int"}],
                     "action": {"do": "set_field", "component": "Health", "field": "hp"}}
                ],
                "components": [{
                    "type_id": "Health", "display_name": "Health",
                    "fields": [{"name": "hp", "type": "int", "min": 0, "max": 100, "default": 10}]
                }],
                "blocks": [
                    {"type_id": "echo_block", "kind": "statement", "category": "Test",
                     "label": "echo {word}", "slots": [{"name": "word", "type": "text", "default": "hi"}],
                     "command": "echo"},
                    {"type_id": "spin_block", "kind": "statement", "category": "Test",
                     "label": "spin", "command": "spin"},
                    {"type_id": "hp_block", "kind": "statement", "category": "Test",
                     "label": "hp {actor} {value}",
                     "slots": [{"name": "actor", "type": "actor"}, {"name": "value", "type": "int"}],
                     "command": "set_hp"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(MANIFEST_FILE),
            serde_json::to_string(&json!({
                "format": 1, "id": id, "name": id, "version": version,
                "engine": ">=0.0.1", "tier": "portable", "abi": 1, "sdk": "^0.1",
                "runtime": {"portable": {"module": "portable/m.wasm", "call_limit_ms": 5}},
                "contributions": ["schemas/main.json"]
            }))
            .unwrap(),
        )
        .unwrap();
        seal(&root).unwrap();
        root
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::declarative;
    use super::*;
    use serde_json::json;

    #[test]
    fn a_sealed_package_loads_and_hashes_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let root = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        let first = Package::load(&root).unwrap();
        assert_eq!(first.contributions.components.len(), 1);
        assert_eq!(
            Package::load(&root).unwrap().content_hash,
            first.content_hash
        );
        assert_eq!(first.content_hash.len(), 64);
    }

    #[test]
    fn tampering_is_detected_in_every_form() {
        let dir = tempfile::tempdir().unwrap();
        let root = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        let schema = root.join("schemas/main.json");
        let original = fs::read(&schema).unwrap();

        fs::write(&schema, b"{}").unwrap();
        assert!(Package::load(&root).unwrap_err().contains("hash"));
        fs::write(&schema, &original).unwrap();

        fs::write(root.join("extra.txt"), b"x").unwrap();
        assert!(Package::load(&root).unwrap_err().contains("not declared"));
        fs::remove_file(root.join("extra.txt")).unwrap();

        fs::remove_file(&schema).unwrap();
        assert!(Package::load(&root).unwrap_err().contains("missing"));
    }

    #[test]
    fn archives_round_trip_and_keep_the_identity() {
        let dir = tempfile::tempdir().unwrap();
        let root = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        let zip = dir.path().join("a.zip");
        pack_archive(&root, &zip).unwrap();
        let out = dir.path().join("out");
        unpack_archive(&zip, &out).unwrap();
        assert_eq!(
            Package::load(&out).unwrap().content_hash,
            Package::load(&root).unwrap().content_hash
        );
    }

    #[test]
    fn archives_cannot_escape_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("evil.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        zip.start_file("../evil.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let out = dir.path().join("out");
        assert!(unpack_archive(&zip_path, &out).is_err());
        assert!(!dir.path().join("evil.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        std::os::unix::fs::symlink("/etc/passwd", root.join("link")).unwrap();
        assert!(Package::load(&root).unwrap_err().contains("symlink"));
    }
}
