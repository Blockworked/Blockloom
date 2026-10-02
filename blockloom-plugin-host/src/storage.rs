//! Where a plugin keeps its own bytes.
//!
//! Two stores, both namespaced by plugin id: project blobs (shipped with the
//! project, read-only in a built game) and player saves (written while a game
//! plays). A [`BlobStore`] is a flat key space with atomic multi-key commits;
//! [`DiskStore`] is a folder, [`MemoryStore`] is for tests and the browser.
//!
//! Plugins reach a store through the `storage.*` and `save.*` services (see
//! [`crate::services`]). Keys are plain relative names, content blobs live
//! under `blobs/<sha256>`, and every write is checked against the limits.

use blockloom_plugin_api::assets::check_output_path;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What one plugin may keep in one store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreLimits {
    pub max_blob_bytes: usize,
    pub max_total_bytes: u64,
    pub max_keys: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 16 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_keys: 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobOp {
    Write { key: String, data: Vec<u8> },
    Delete { key: String },
}

impl BlobOp {
    pub fn key(&self) -> &str {
        match self {
            BlobOp::Write { key, .. } | BlobOp::Delete { key } => key,
        }
    }
}

pub trait BlobStore: Send + Sync {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    /// Every key under `prefix` with its size, sorted.
    fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, String>;
    /// Applies every op or none of them.
    fn commit(&self, ops: &[BlobOp]) -> Result<(), String>;
    fn read_only(&self) -> bool {
        false
    }
}

pub fn hash_of(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `text` could be the name of a content blob.
pub fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A key as a plugin spells it, checked.
pub fn check_key(key: &str) -> Result<(), String> {
    check_output_path(key).map_err(|e| e.replace("output path", "storage key"))
}

const TMP: &str = ".blockloom-tmp";
const BAK: &str = ".blockloom-bak";

/// A folder. Keys map to files below it; a commit stages every write beside
/// its target and renames, so a failure puts the old files back.
pub struct DiskStore {
    root: PathBuf,
    read_only: bool,
}

impl DiskStore {
    pub fn new(root: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            root: root.into(),
            read_only,
        }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, u64)>) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(TMP) || name.ends_with(BAK) {
            continue;
        }
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        if meta.is_dir() {
            walk(&path, base, out)?;
        } else if let Ok(rel) = path.strip_prefix(base) {
            let key = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((key, meta.len()));
        }
    }
    Ok(())
}

impl BlobStore for DiskStore {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(self.path(key)) {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{key}: {e}")),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out)?;
        out.retain(|(key, _)| key.starts_with(prefix));
        out.sort();
        Ok(out)
    }

    fn commit(&self, ops: &[BlobOp]) -> Result<(), String> {
        if self.read_only {
            return Err("this store is read-only".to_string());
        }
        // Stage the writes first: nothing live changes if one cannot be made.
        let mut staged = Vec::new();
        let stage = |staged: &mut Vec<(PathBuf, PathBuf)>| -> Result<(), String> {
            for op in ops {
                if let BlobOp::Write { key, data } = op {
                    let target = self.path(key);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| format!("{key}: {e}"))?;
                    }
                    let tmp = PathBuf::from(format!("{}{TMP}", target.display()));
                    std::fs::write(&tmp, data).map_err(|e| format!("{key}: {e}"))?;
                    staged.push((tmp, target));
                }
            }
            Ok(())
        };
        if let Err(e) = stage(&mut staged) {
            for (tmp, _) in &staged {
                let _ = std::fs::remove_file(tmp);
            }
            return Err(e);
        }
        // Move originals aside, then the new files in. `undo` holds what to put back.
        let mut undo: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        let apply = |undo: &mut Vec<(PathBuf, Option<PathBuf>)>| -> Result<(), String> {
            let aside = |target: &Path| -> Result<Option<PathBuf>, String> {
                if !target.exists() {
                    return Ok(None);
                }
                let bak = PathBuf::from(format!("{}{BAK}", target.display()));
                std::fs::rename(target, &bak).map_err(|e| e.to_string())?;
                Ok(Some(bak))
            };
            for op in ops {
                if let BlobOp::Delete { key } = op {
                    let target = self.path(key);
                    let bak = aside(&target)?;
                    undo.push((target, bak));
                }
            }
            for (tmp, target) in &staged {
                let bak = aside(target)?;
                undo.push((target.clone(), bak));
                std::fs::rename(tmp, target).map_err(|e| e.to_string())?;
            }
            Ok(())
        };
        match apply(&mut undo) {
            Ok(()) => {
                for (_, bak) in undo {
                    if let Some(bak) = bak {
                        let _ = std::fs::remove_file(bak);
                    }
                }
                Ok(())
            }
            Err(e) => {
                for (target, bak) in undo.into_iter().rev() {
                    let _ = std::fs::remove_file(&target);
                    if let Some(bak) = bak {
                        let _ = std::fs::rename(bak, target);
                    }
                }
                for (tmp, _) in &staged {
                    let _ = std::fs::remove_file(tmp);
                }
                Err(e)
            }
        }
    }

    fn read_only(&self) -> bool {
        self.read_only
    }
}

/// A built game's project data: read through [`crate::files`], so it works from
/// an APK or a page's mounted files, listed by the index the build wrote.
pub struct PackStore {
    root: PathBuf,
    index: blockloom_plugin_api::data::DataIndex,
}

impl PackStore {
    /// The data folder `root` (a game folder's `.blockloom/plugin-data`). A
    /// missing or unreadable index is an empty store.
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let index = crate::files::read(&root.join(blockloom_plugin_api::data::INDEX_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { root, index }
    }
}

impl BlobStore for PackStore {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        if !self.index.files.contains_key(key) {
            return Ok(None);
        }
        crate::files::read(&self.root.join(key))
            .map(Some)
            .map_err(|e| format!("{key}: {e}"))
    }

    fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        Ok(self
            .index
            .files
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), *v))
            .collect())
    }

    fn commit(&self, _ops: &[BlobOp]) -> Result<(), String> {
        Err("a built game's project data is read-only".to_string())
    }

    fn read_only(&self) -> bool {
        true
    }
}

/// Keys in memory, for tests and the browser, where there is no disk.
#[derive(Default)]
pub struct MemoryStore {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    read_only: bool,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that starts with `files` and refuses writes.
    pub fn read_only(files: BTreeMap<String, Vec<u8>>) -> Self {
        Self {
            files: Mutex::new(files),
            read_only: true,
        }
    }
}

impl BlobStore for MemoryStore {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self
            .files
            .lock()
            .map_err(|e| e.to_string())?
            .get(key)
            .cloned())
    }

    fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        let files = self.files.lock().map_err(|e| e.to_string())?;
        Ok(files
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.len() as u64))
            .collect())
    }

    fn commit(&self, ops: &[BlobOp]) -> Result<(), String> {
        if self.read_only {
            return Err("this store is read-only".to_string());
        }
        let mut files = self.files.lock().map_err(|e| e.to_string())?;
        let mut next = files.clone();
        for op in ops {
            match op {
                BlobOp::Write { key, data } => {
                    next.insert(key.clone(), data.clone());
                }
                BlobOp::Delete { key } => {
                    next.remove(key);
                }
            }
        }
        *files = next;
        Ok(())
    }

    fn read_only(&self) -> bool {
        self.read_only
    }
}

/// Checks a batch against `limits` for `plugin`'s namespace and applies it.
/// `ops` carry keys relative to the plugin; the namespace is added here.
pub fn commit_for(
    store: &dyn BlobStore,
    plugin: &str,
    ops: &[BlobOp],
    limits: &StoreLimits,
) -> Result<(), String> {
    if ops.is_empty() {
        return Ok(());
    }
    let mut full = Vec::with_capacity(ops.len());
    for op in ops {
        check_key(op.key())?;
        match op {
            BlobOp::Write { key, data } => {
                if data.len() > limits.max_blob_bytes {
                    return Err(format!(
                        "{key}: {} bytes is over the {} byte limit for one blob",
                        data.len(),
                        limits.max_blob_bytes
                    ));
                }
                full.push(BlobOp::Write {
                    key: format!("{plugin}/{key}"),
                    data: data.clone(),
                });
            }
            BlobOp::Delete { key } => full.push(BlobOp::Delete {
                key: format!("{plugin}/{key}"),
            }),
        }
    }
    // What the namespace would hold afterwards.
    let existing: BTreeMap<String, u64> = store.list(&format!("{plugin}/"))?.into_iter().collect();
    let mut after = existing;
    for op in &full {
        match op {
            BlobOp::Write { key, data } => {
                after.insert(key.clone(), data.len() as u64);
            }
            BlobOp::Delete { key } => {
                after.remove(key);
            }
        }
    }
    if after.len() > limits.max_keys {
        return Err(format!("over the limit of {} keys", limits.max_keys));
    }
    let total: u64 = after.values().sum();
    if total > limits.max_total_bytes {
        return Err(format!(
            "{total} bytes is over the {} byte limit for one plugin",
            limits.max_total_bytes
        ));
    }
    store.commit(&full)
}

/// Every content hash a payload mentions as `"blob:<sha256>"`, anywhere in it.
pub fn blob_refs(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(hash) = s.strip_prefix("blob:")
                && is_hash(hash)
            {
                out.insert(hash.to_string());
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| blob_refs(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| blob_refs(v, out)),
        _ => {}
    }
}

/// The content blobs of one plugin that nothing in `referenced` names.
pub fn unreferenced_blobs(
    store: &dyn BlobStore,
    plugin: &str,
    referenced: &std::collections::BTreeSet<String>,
) -> Result<Vec<(String, u64)>, String> {
    let prefix = format!("{plugin}/blobs/");
    Ok(store
        .list(&prefix)?
        .into_iter()
        .filter(|(key, _)| {
            key.strip_prefix(&prefix)
                .is_some_and(|hash| is_hash(hash) && !referenced.contains(hash))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(key: &str, data: &[u8]) -> BlobOp {
        BlobOp::Write {
            key: key.to_string(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn a_disk_commit_applies_every_op_and_lists_the_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = DiskStore::new(dir.path(), false);
        let limits = StoreLimits::default();
        commit_for(
            &store,
            "p",
            &[write("a/one", b"1"), write("two", b"22")],
            &limits,
        )
        .unwrap();
        assert_eq!(store.read("p/a/one").unwrap().unwrap(), b"1");
        assert_eq!(store.list("p/").unwrap().len(), 2);
        commit_for(
            &store,
            "p",
            &[BlobOp::Delete { key: "two".into() }, write("a/one", b"x")],
            &limits,
        )
        .unwrap();
        assert_eq!(store.read("p/two").unwrap(), None);
        assert_eq!(store.read("p/a/one").unwrap().unwrap(), b"x");
        // No staging files are left behind.
        assert!(
            store
                .list("")
                .unwrap()
                .iter()
                .all(|(k, _)| !k.ends_with(TMP) && !k.ends_with(BAK))
        );
    }

    #[test]
    fn a_failing_commit_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = DiskStore::new(dir.path(), false);
        let limits = StoreLimits::default();
        commit_for(&store, "p", &[write("keep", b"old")], &limits).unwrap();
        // The second write's parent is a file, so staging it fails.
        let result = store.commit(&[write("p/keep", b"new"), write("p/keep/inner", b"no")]);
        assert!(result.is_err());
        assert_eq!(store.read("p/keep").unwrap().unwrap(), b"old");
    }

    #[test]
    fn keys_and_sizes_are_checked_before_anything_is_written() {
        let store = MemoryStore::new();
        let limits = StoreLimits {
            max_blob_bytes: 4,
            max_total_bytes: 6,
            max_keys: 2,
        };
        assert!(commit_for(&store, "p", &[write("../x", b"1")], &limits).is_err());
        assert!(commit_for(&store, "p", &[write(".hidden", b"1")], &limits).is_err());
        assert!(commit_for(&store, "p", &[write("big", b"12345")], &limits).is_err());
        commit_for(
            &store,
            "p",
            &[write("a", b"1234"), write("b", b"12")],
            &limits,
        )
        .unwrap();
        // A third key and a seventh byte are both over.
        assert!(commit_for(&store, "p", &[write("c", b"1")], &limits).is_err());
        assert!(
            commit_for(
                &store,
                "p",
                &[write("a", b"1234"), write("b", b"123")],
                &limits
            )
            .is_err()
        );
        // Another plugin has its own space.
        commit_for(&store, "q", &[write("a", b"1234")], &limits).unwrap();
    }

    #[test]
    fn a_pack_store_reads_what_its_index_lists() {
        use blockloom_plugin_api::data::{DataIndex, INDEX_FILE};
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("p")).unwrap();
        std::fs::write(dir.path().join("p/a"), b"1234").unwrap();
        std::fs::write(dir.path().join("p/unlisted"), b"x").unwrap();
        let index = DataIndex {
            files: BTreeMap::from([("p/a".to_string(), 4)]),
        };
        std::fs::write(
            dir.path().join(INDEX_FILE),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let store = PackStore::open(dir.path());
        assert_eq!(store.read("p/a").unwrap().unwrap(), b"1234");
        assert_eq!(store.read("p/unlisted").unwrap(), None);
        assert_eq!(store.list("p/").unwrap(), vec![("p/a".to_string(), 4)]);
        assert!(store.read_only());
        assert!(store.commit(&[]).is_err());
        assert!(
            PackStore::open(dir.path().join("none"))
                .list("")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_read_only_store_refuses_writes() {
        let store = MemoryStore::read_only(BTreeMap::from([("p/a".to_string(), b"1".to_vec())]));
        assert_eq!(store.read("p/a").unwrap().unwrap(), b"1");
        assert!(commit_for(&store, "p", &[write("b", b"2")], &StoreLimits::default()).is_err());
    }

    #[test]
    fn content_blobs_not_named_by_a_payload_are_unreferenced() {
        let store = MemoryStore::new();
        let kept = hash_of(b"kept");
        let lost = hash_of(b"lost");
        commit_for(
            &store,
            "p",
            &[
                write(&format!("blobs/{kept}"), b"kept"),
                write(&format!("blobs/{lost}"), b"lost"),
                write("named", b"x"),
            ],
            &StoreLimits::default(),
        )
        .unwrap();
        let mut referenced = std::collections::BTreeSet::new();
        blob_refs(
            &serde_json::json!({"deep": [{"chunk": format!("blob:{kept}")}], "x": "blob:nothash"}),
            &mut referenced,
        );
        assert_eq!(referenced.len(), 1);
        let gone = unreferenced_blobs(&store, "p", &referenced).unwrap();
        assert_eq!(gone.len(), 1);
        assert!(gone[0].0.ends_with(&lost));
    }
}
