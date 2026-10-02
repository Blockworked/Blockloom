//! The shared, immutable package cache.
//!
//! Every version of every package lives once, in a folder named for its
//! content hash, shared by all projects. Nothing in it is ever edited; a
//! package is published into it whole, by rename, after it verified.

use crate::package::{Package, copy_dir};
use blockloom_plugin_api::Version;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch folder inside the cache's own filesystem that deletes itself,
/// so a failed install leaves nothing behind.
pub struct Staging {
    path: PathBuf,
}

impl Staging {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A fresh empty subfolder.
    pub fn subdir(&self, name: &str) -> Result<PathBuf, String> {
        let dir = self.path.join(name);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// `<data dir>/blockloom/plugin-cache`, or under `BLOCKLOOM_DATA_DIR`.
pub fn default_root() -> PathBuf {
    std::env::var_os("BLOCKLOOM_DATA_DIR")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("blockloom")
        .join("plugin-cache")
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn packages(&self) -> PathBuf {
        self.root.join("packages")
    }

    pub fn staging(&self) -> Result<Staging, String> {
        let n = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = self
            .root
            .join("staging")
            .join(format!("{}-{n}", std::process::id()));
        fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Staging { path })
    }

    fn entry_dir(&self, id: &str, version: &Version, hash: &str) -> PathBuf {
        self.packages()
            .join(id)
            .join(format!("{version}-{}", &hash[..16.min(hash.len())]))
    }

    /// The folder holding exactly this package content, if cached.
    pub fn get(&self, id: &str, version: &Version, hash: &str) -> Option<PathBuf> {
        let dir = self.entry_dir(id, version, hash);
        dir.join("plugin.json").exists().then_some(dir)
    }

    /// Verifies the package in `source` and publishes it. Returns the
    /// verified package at its cache location.
    pub fn publish(&self, source: &Path) -> Result<Package, String> {
        let package = Package::load(source)?;
        let id = &package.manifest.id;
        let version = &package.manifest.version;
        let dest = self.entry_dir(id, version, &package.content_hash);
        if dest.join("plugin.json").exists() {
            return Package::load(&dest);
        }
        // Same id and version under another hash is a different package
        // pretending to be this one.
        for other in self.versions(id)? {
            if other.manifest.version == *version && other.content_hash != package.content_hash {
                return Err(format!(
                    "{id} {version} is already cached with different content; versions are immutable"
                ));
            }
        }
        let parent = dest.parent().expect("entry has a parent");
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = parent.join(format!(
            ".incoming-{}-{}",
            std::process::id(),
            STAGING_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let result = copy_dir(source, &temp)
            .and_then(|_| fs::rename(&temp, &dest).map_err(|e| format!("{}: {e}", dest.display())));
        if let Err(e) = result {
            let _ = fs::remove_dir_all(&temp);
            // Another process may have published the same content first.
            if dest.join("plugin.json").exists() {
                return Package::load(&dest);
            }
            return Err(e);
        }
        Package::load(&dest)
    }

    /// Every cached version of `id`.
    pub fn versions(&self, id: &str) -> Result<Vec<Package>, String> {
        let dir = self.packages().join(id);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if let Ok(package) = Package::load(&entry.path()) {
                out.push(package);
            }
        }
        out.sort_by(|a, b| a.manifest.version.cmp(&b.manifest.version));
        Ok(out)
    }

    /// Every cached package id.
    pub fn ids(&self) -> Vec<String> {
        fs::read_dir(self.packages())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// Removes cached packages not named in `keep` (id, content hash).
    /// Callers build `keep` from every lock file that must stay installable:
    /// current, history and any open project's. Returns what was removed.
    pub fn collect_garbage(
        &self,
        keep: &BTreeSet<(String, String)>,
    ) -> Result<Vec<String>, String> {
        let mut removed = Vec::new();
        for id in self.ids() {
            for package in self.versions(&id)? {
                if !keep.contains(&(id.clone(), package.content_hash.clone())) {
                    fs::remove_dir_all(&package.root).map_err(|e| e.to_string())?;
                    removed.push(format!("{id} {}", package.manifest.version));
                }
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::fixtures::declarative;
    use serde_json::json;

    #[test]
    fn publish_is_idempotent_and_lists_versions_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("cache"));
        let a1 = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        let a2 = declarative(dir.path(), "com.example.a", "1.1.0", json!({}));
        let first = cache.publish(&a1).unwrap();
        cache.publish(&a1).unwrap();
        cache.publish(&a2).unwrap();
        let versions = cache.versions("com.example.a").unwrap();
        assert_eq!(versions.len(), 2);
        assert!(versions[0].manifest.version < versions[1].manifest.version);
        assert!(
            cache
                .get("com.example.a", &Version::new(1, 0, 0), &first.content_hash)
                .is_some()
        );
    }

    #[test]
    fn a_version_cannot_be_republished_with_different_content() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("cache"));
        let a = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        cache.publish(&a).unwrap();
        fs::write(a.join("schemas/extra.json"), "{}").unwrap();
        crate::package::seal(&a).unwrap();
        assert!(cache.publish(&a).unwrap_err().contains("immutable"));
    }

    #[test]
    fn staging_cleans_up_after_itself() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("cache"));
        let path = {
            let staging = cache.staging().unwrap();
            fs::write(staging.path().join("x"), "1").unwrap();
            staging.path().to_path_buf()
        };
        assert!(!path.exists());
    }

    #[test]
    fn garbage_collection_keeps_what_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("cache"));
        let a1 = cache
            .publish(&declarative(
                dir.path(),
                "com.example.a",
                "1.0.0",
                json!({}),
            ))
            .unwrap();
        cache
            .publish(&declarative(
                dir.path(),
                "com.example.a",
                "2.0.0",
                json!({}),
            ))
            .unwrap();
        let keep = BTreeSet::from([("com.example.a".to_string(), a1.content_hash.clone())]);
        let removed = cache.collect_garbage(&keep).unwrap();
        assert_eq!(removed, vec!["com.example.a 2.0.0".to_string()]);
        assert_eq!(cache.versions("com.example.a").unwrap().len(), 1);
    }

    #[test]
    fn a_tampered_cache_entry_stops_loading() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("cache"));
        let pkg = cache
            .publish(&declarative(
                dir.path(),
                "com.example.a",
                "1.0.0",
                json!({}),
            ))
            .unwrap();
        fs::write(pkg.root.join("schemas/main.json"), "{}").unwrap();
        assert!(cache.versions("com.example.a").unwrap().is_empty());
    }
}
