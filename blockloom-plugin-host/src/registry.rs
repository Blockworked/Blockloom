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
        unpack_verified(&bytes, entry, id, version, staging)
    }
}

/// Checks the archive against the index hash, then unpacks it into `staging`.
fn unpack_verified(
    bytes: &[u8],
    entry: &IndexEntry,
    id: &str,
    version: &Version,
    staging: &Path,
) -> Result<(), String> {
    if sha256_hex(bytes) != entry.archive_sha256 {
        return Err(format!(
            "{id} {version}: archive does not match the index hash"
        ));
    }
    let temp = staging.with_extension("download.zip");
    fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    let unpacked = unpack_archive(&temp, staging);
    let _ = fs::remove_file(&temp);
    unpacked
}

/// The registry a `plugins.json` entry names: an `https://` URL (or `http://`
/// on this machine) is read over HTTP, anything else is a folder under
/// `base`.
pub fn open_registry(base: &Path, location: &str) -> Result<Box<dyn Registry>, String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        return Ok(Box::new(HttpRegistry::open(location)?));
        #[cfg(any(target_arch = "wasm32", target_os = "android"))]
        return Err("this build can't read an HTTP registry".to_string());
    }
    Ok(Box::new(DirRegistry::open(base.join(location))))
}

/// Largest `index.json` and archive an HTTP registry may send.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

/// A registry served over HTTP: the same `index.json` and `archives/` a
/// [`DirRegistry`] writes, on any static host. Read only; publishing is
/// putting a published folder there. The index is read once per value; every
/// archive is checked against the hash in it, so only the index has to come
/// over a channel you trust (HTTPS, or this machine).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub struct HttpRegistry {
    base: String,
    agent: ureq::Agent,
    index: std::sync::OnceLock<Index>,
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
impl HttpRegistry {
    pub fn open(url: &str) -> Result<Self, String> {
        let base = url.trim_end_matches('/').to_string();
        let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
            .iter()
            .any(|prefix| {
                base.strip_prefix(prefix)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
            });
        if !base.starts_with("https://") && !local {
            return Err(format!(
                "{url}: a registry must be https:// (http:// is only for this machine)"
            ));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .https_only(!local)
            .build()
            .into();
        Ok(Self {
            base,
            agent,
            index: std::sync::OnceLock::new(),
        })
    }

    fn get(&self, relative: &str, limit: u64) -> Result<Vec<u8>, String> {
        let url = format!("{}/{relative}", self.base);
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| format!("{url}: {e}"))?;
        response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| format!("{url}: {e}"))
    }

    fn index(&self) -> Result<&Index, String> {
        if let Some(index) = self.index.get() {
            return Ok(index);
        }
        let bytes = self.get("index.json", MAX_INDEX_BYTES)?;
        let index: Index =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}/index.json: {e}", self.base))?;
        Ok(self.index.get_or_init(|| index))
    }
}

/// An archive path an index may name: relative, plain segments.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '?', '#'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
impl Registry for HttpRegistry {
    fn versions(&self, id: &str) -> Result<Vec<IndexEntry>, String> {
        Ok(self
            .index()?
            .packages
            .get(id)
            .map(|v| v.values().cloned().collect())
            .unwrap_or_default())
    }

    fn fetch(&self, id: &str, version: &Version, staging: &Path) -> Result<(), String> {
        let entry = self
            .index()?
            .packages
            .get(id)
            .and_then(|v| v.get(&version.to_string()))
            .ok_or_else(|| format!("{id} {version} is not in this registry"))?;
        if !plain_relative(&entry.archive) {
            return Err(format!(
                "{id} {version}: the index names an archive path \"{}\" that isn't plain",
                entry.archive
            ));
        }
        let bytes = self.get(&entry.archive, MAX_ARCHIVE_BYTES)?;
        unpack_verified(&bytes, entry, id, version, staging)
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

    /// Serves `root` over HTTP on a loopback port until the process ends,
    /// counting requests; `tamper` rewrites a path's body.
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    fn serve(
        root: PathBuf,
        tamper: fn(&str, Vec<u8>) -> Vec<u8>,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{BufRead, BufReader, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                counted.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let mut stream = stream;
                match fs::read(root.join(path.trim_start_matches('/'))) {
                    Ok(body) => {
                        let body = tamper(&path, body);
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(&body);
                    }
                    Err(_) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    }
                }
            }
        });
        (url, hits)
    }

    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    #[test]
    fn an_http_registry_lists_and_fetches_what_a_folder_published() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("reg");
        let published = DirRegistry::open(&folder);
        let pkg = declarative(dir.path(), "com.example.a", "1.0.0", json!({}));
        let entry = published.publish(&pkg).unwrap();
        let (url, hits) = serve(folder, |_, body| body);

        let registry = open_registry(dir.path(), &url).unwrap();
        let versions = registry.versions("com.example.a").unwrap();
        assert_eq!(versions[0].content_hash, entry.content_hash);
        assert!(registry.versions("com.example.none").unwrap().is_empty());
        // The index is read once however many questions follow.
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);

        let staging = dir.path().join("staging");
        registry
            .fetch("com.example.a", &Version::new(1, 0, 0), &staging)
            .unwrap();
        assert_eq!(
            Package::load(&staging).unwrap().content_hash,
            entry.content_hash
        );
        assert!(!dir.path().join("staging.download.zip").exists());
        let missing = registry
            .fetch(
                "com.example.a",
                &Version::new(2, 0, 0),
                &dir.path().join("s2"),
            )
            .unwrap_err();
        assert!(missing.contains("not in this registry"), "{missing}");
    }

    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    #[test]
    fn an_http_archive_that_does_not_match_the_index_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("reg");
        DirRegistry::open(&folder)
            .publish(&declarative(
                dir.path(),
                "com.example.a",
                "1.0.0",
                json!({}),
            ))
            .unwrap();
        let (url, _) = serve(folder, |path, body| {
            if path.ends_with(".zip") {
                b"tampered".to_vec()
            } else {
                body
            }
        });
        let registry = HttpRegistry::open(&url).unwrap();
        let error = registry
            .fetch(
                "com.example.a",
                &Version::new(1, 0, 0),
                &dir.path().join("s"),
            )
            .unwrap_err();
        assert!(error.contains("index hash"), "{error}");
    }

    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    #[test]
    fn http_registries_need_https_except_on_this_machine() {
        for bad in [
            "http://example.com/reg",
            "ftp://example.com",
            "http://127.0.0.1.evil.com",
        ] {
            assert!(HttpRegistry::open(bad).is_err(), "{bad}");
        }
        for good in [
            "https://example.com/reg/",
            "http://localhost:8080",
            "http://127.0.0.1/reg",
        ] {
            assert!(HttpRegistry::open(good).is_ok(), "{good}");
        }
        assert!(plain_relative("archives/a-1.0.0.zip"));
        for bad in [
            "",
            "/x",
            "../x",
            "a/../x",
            "a//b",
            "a\\b",
            "https://x/y",
            "a?b",
            "a#b",
        ] {
            assert!(!plain_relative(bad), "{bad}");
        }
    }
}
