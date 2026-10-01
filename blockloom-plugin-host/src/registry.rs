//! Registries: where versions of a package are listed and fetched from.
//!
//! [`DirRegistry`] is a folder-backed registry (a mirror, a shared drive or a
//! CI artifact folder). It carries the whole publishing protocol - archive,
//! hash, index entry, immutable versions - so an HTTP transport only has to
//! move the same files.

use crate::package::{Package, pack_archive, sha256_hex, unpack_archive};
use blockloom_plugin_api::Version;
use blockloom_plugin_api::manifest::PluginManifest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One published version: its manifest (so a resolver need not download it),
/// and where its archive is and what it hashes to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub manifest: PluginManifest,
    /// Archive path relative to the registry root.
    pub archive: String,
    /// sha256 of the archive bytes.
    pub archive_sha256: String,
    /// The package's own content hash, as the lock records it.
    pub content_hash: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    packages: BTreeMap<String, BTreeMap<String, IndexEntry>>,
}

pub trait Registry {
    fn versions(&self, id: &str) -> Result<Vec<IndexEntry>, String>;
    /// Downloads, verifies and unpacks `id@version` into the empty `staging`.
    fn fetch(&self, id: &str, version: &Version, staging: &Path) -> Result<(), String>;
}

pub struct DirRegistry {
    root: PathBuf,
}

impl DirRegistry {
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    fn read_index(&self) -> Result<Index, String> {
        match fs::read_to_string(self.index_path()) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("{}: {e}", self.index_path().display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
            Err(e) => Err(format!("{}: {e}", self.index_path().display())),
        }
    }

    /// Publishes a package folder. A version is immutable: publishing the
    /// same content again is a no-op, different content under the same
    /// version is refused.
    pub fn publish(&self, package: &Path) -> Result<IndexEntry, String> {
        let package = Package::load(package)?;
        let id = package.manifest.id.clone();
        let version = package.manifest.version.to_string();
        let mut index = self.read_index()?;
        if let Some(existing) = index.packages.get(&id).and_then(|v| v.get(&version)) {
            return if existing.content_hash == package.content_hash {
                Ok(existing.clone())
            } else {
                Err(format!(
                    "{id} {version} is already published with different content; bump the version"
                ))
            };
        }
        fs::create_dir_all(self.root.join("archives")).map_err(|e| e.to_string())?;
        let relative = format!("archives/{id}-{version}.zip");
        let archive_path = self.root.join(&relative);
        pack_archive(&package.root, &archive_path)?;
        let bytes = fs::read(&archive_path).map_err(|e| e.to_string())?;
        let entry = IndexEntry {
            manifest: package.manifest.clone(),
            archive: relative,
            archive_sha256: sha256_hex(&bytes),
            content_hash: package.content_hash.clone(),
        };
        index
            .packages
            .entry(id)
            .or_default()
            .insert(version, entry.clone());
        let mut text = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
        text.push('\n');
        crate::lock::write_atomic(&self.index_path(), text.as_bytes())?;
        Ok(entry)
    }
}

impl Registry for DirRegistry {
    fn versions(&self, id: &str) -> Result<Vec<IndexEntry>, String> {
        Ok(self
            .read_index()?
            .packages
            .remove(id)
            .map(|v| v.into_values().collect())
            .unwrap_or_default())
    }

    fn fetch(&self, id: &str, version: &Version, staging: &Path) -> Result<(), String> {
        let index = self.read_index()?;
        let entry = index
            .packages
            .get(id)
            .and_then(|v| v.get(&version.to_string()))
            .ok_or_else(|| format!("{id} {version} is not in this registry"))?;
        let archive = self.root.join(&entry.archive);
        let bytes = fs::read(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
        if sha256_hex(&bytes) != entry.archive_sha256 {
            return Err(format!(
                "{id} {version}: archive does not match the index hash"
            ));
        }
        unpack_archive(&archive, staging)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::fixtures::declarative;
    use serde_json::json;

    #[test]
    fn publish_then_list_and_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let registry = DirRegistry::open(dir.path().join("reg"));
        let pkg = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        let entry = registry.publish(&pkg).unwrap();
        let versions = registry.versions("com.example.a").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].content_hash, entry.content_hash);
        assert!(registry.versions("com.example.none").unwrap().is_empty());

        let staging = dir.path().join("staging");
        registry
            .fetch("com.example.a", &Version::new(1, 0, 0), &staging)
            .unwrap();
        assert_eq!(
            Package::load(&staging).unwrap().content_hash,
            entry.content_hash
        );
    }

    #[test]
    fn versions_are_immutable_but_republishing_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let registry = DirRegistry::open(dir.path().join("reg"));
        let pkg = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        registry.publish(&pkg).unwrap();
        registry.publish(&pkg).unwrap();
        fs::write(pkg.join("schemas/extra.json"), b"{}").unwrap();
        crate::package::seal(&pkg).unwrap();
        assert!(
            registry
                .publish(&pkg)
                .unwrap_err()
                .contains("bump the version")
        );
    }

    #[test]
    fn a_corrupted_archive_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let registry = DirRegistry::open(dir.path().join("reg"));
        let pkg = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        let entry = registry.publish(&pkg).unwrap();
        fs::write(dir.path().join("reg").join(&entry.archive), b"junk").unwrap();
        let staging = dir.path().join("s");
        let error = registry
            .fetch("com.example.a", &Version::new(1, 0, 0), &staging)
            .unwrap_err();
        assert!(error.contains("index hash"), "{error}");
    }
}
