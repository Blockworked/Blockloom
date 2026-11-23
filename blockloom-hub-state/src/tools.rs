//! The shared tool cache: one copy per Java, Android SDK/NDK and Rust
//! version under `<Hub>/tool-cache/<key>/`, copied into each installation
//! that wants it and deleted once no installed editor names it.

use crate::installs::{ANDROID_TARGETS, exe, host_target};
use crate::run::{copy_tree, size_of};
use crate::{Error, Result, checkpoint};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const CACHE_DIR_NAME: &str = "tool-cache";

pub fn cache_dir(hub_root: &Path) -> PathBuf {
    hub_root.join(CACHE_DIR_NAME)
}

fn host_tag() -> (&'static str, &'static str) {
    let host = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macosx"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x64"
    };
    (host, arch)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect()
}

pub fn is_channel(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

pub fn rust_key(channel: &str) -> String {
    format!("rust-{}-{}", sanitize(channel), host_target())
}

pub fn rust_target_key(channel: &str, target: &str) -> String {
    format!("rust-std-{}-{}", sanitize(channel), sanitize(target))
}

pub fn java_key(version: &str) -> String {
    let (host, arch) = host_tag();
    format!("java-{}-{host}-{arch}", sanitize(version))
}

pub fn ndk_key(version: &str) -> String {
    let (host, arch) = host_tag();
    format!("android-ndk-{}-{host}-{arch}", sanitize(version))
}

/// Keyed by the platform tools' version plus a hash of every package's.
pub fn sdk_key(packages: &Value, version: &str) -> String {
    let (host, arch) = host_tag();
    let canonical = serde_json::to_string(&sorted(packages)).unwrap_or_default();
    let digest = Sha256::digest(canonical.as_bytes());
    let fingerprint: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "android-sdk-{}-{host}-{arch}-{}",
        sanitize(version),
        &fingerprint[..12]
    )
}

/// Keys in order at every level, as the old service's `sort_keys` dump had them.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let ordered: std::collections::BTreeMap<_, _> =
                map.iter().map(|(k, v)| (k.clone(), sorted(v))).collect();
            serde_json::to_value(ordered).unwrap_or(Value::Null)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Cache keys the installed editors still use.
pub fn live_keys(installations: &[Value]) -> BTreeSet<String> {
    let mut live = BTreeSet::new();
    for manifest in installations {
        let Some(tools) = manifest.get("tools").and_then(Value::as_object) else {
            continue;
        };
        if let Some(channel) = tools.get("rust").and_then(|r| r["channel"].as_str())
            && is_channel(channel)
        {
            live.insert(rust_key(channel));
            for target in tools
                .get("android-rust-targets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if ANDROID_TARGETS.contains(&target) {
                    live.insert(rust_target_key(channel, target));
                }
            }
        }
        let version = |name: &str| {
            tools
                .get(name)
                .and_then(|t| t["version"].as_str())
                .filter(|v| !v.is_empty())
        };
        if let Some(v) = version("java") {
            live.insert(java_key(v));
        }
        if let Some(v) = version("android-sdk") {
            let packages = tools["android-sdk"]
                .get("packages")
                .filter(|p| p.is_object())
                .cloned()
                .unwrap_or(json!({}));
            live.insert(sdk_key(&packages, v));
        }
        if let Some(v) = version("android-ndk") {
            live.insert(ndk_key(v));
        }
    }
    live
}

fn valid_rust_base(path: &Path) -> bool {
    path.join("bin").join(exe("rustc")).is_file()
        && path.join("bin").join(exe("cargo")).is_file()
        && path
            .join("lib/rustlib")
            .join(host_target())
            .join("lib")
            .is_dir()
}

/// Whether a cache entry (or a copy of one) holds what its key says.
pub fn valid_entry(key: &str, path: &Path) -> bool {
    if key.starts_with("rust-std-") {
        path.join("lib").is_dir()
    } else if key.starts_with("rust-") {
        valid_rust_base(path)
    } else if key.starts_with("java-") {
        path.join("bin").join(exe("java")).is_file()
    } else if key.starts_with("android-sdk-") {
        path.join("platform-tools").join(exe("adb")).is_file()
    } else if key.starts_with("android-ndk-") {
        path.join("source.properties").is_file()
    } else {
        path.is_dir()
    }
}

pub fn is_cached(cache: &Path, key: &str) -> bool {
    let path = cache.join(key);
    path.is_dir() && valid_entry(key, &path)
}

/// Copies a cached version to `destination`. False when there is none.
pub fn reuse(cache: &Path, key: &str, destination: &Path) -> Result<bool> {
    if !is_cached(cache, key) {
        return Ok(false);
    }
    if destination.exists() {
        return Err(Error::new(format!(
            "Tool destination already exists: {}",
            destination.display()
        )));
    }
    checkpoint()?;
    eprintln!("Reusing cached {key}...");
    copy_tree(&cache.join(key), destination, &|_| false)?;
    if !valid_entry(key, destination) {
        let _ = std::fs::remove_dir_all(destination);
        return Err(Error::new(format!("Cached tool failed validation: {key}")));
    }
    Ok(true)
}

/// Like [`reuse`], but a broken cache entry is dropped and counts as a miss.
pub fn reuse_or_drop(cache: &Path, key: &str, destination: &Path) -> Result<bool> {
    match reuse(cache, key, destination) {
        Err(e) if !e.cancelled => {
            let _ = std::fs::remove_dir_all(cache.join(key));
            let _ = std::fs::remove_dir_all(destination);
            Ok(false)
        }
        other => other,
    }
}

/// Saves an installed tool for future editors. `skip` leaves out paths
/// relative to `source`.
fn store_filtered(
    cache: &Path,
    key: &str,
    source: &Path,
    skip: &dyn Fn(&str) -> bool,
) -> Result<bool> {
    if !source.is_dir() {
        return Ok(false);
    }
    if is_cached(cache, key) {
        return Ok(true);
    }
    if !valid_entry(key, source) {
        return Ok(false);
    }
    std::fs::create_dir_all(cache)?;
    checkpoint()?;
    eprintln!("Caching {key} for future editors...");
    let temporary = tempfile::Builder::new()
        .prefix(".tool-cache-")
        .tempdir_in(cache)?;
    let staging = temporary.path().join("entry");
    copy_tree(source, &staging, skip)?;
    if !valid_entry(key, &staging) {
        return Err(Error::new(format!(
            "Tool failed validation before caching: {key}"
        )));
    }
    checkpoint()?;
    let destination = cache.join(key);
    if std::fs::rename(&staging, &destination).is_err() && !is_cached(cache, key) {
        // Another operation left something unusable there first.
        let _ = std::fs::remove_dir_all(&destination);
        std::fs::rename(&staging, &destination)?;
    }
    Ok(true)
}

pub fn store(cache: &Path, key: &str, source: &Path) -> Result<bool> {
    store_filtered(cache, key, source, &|_| false)
}

/// [`store`], where a failed validation is not worth failing the install.
pub fn store_quietly(cache: &Path, key: &str, source: &Path) -> Result<()> {
    match store(cache, key, source) {
        Err(e) if e.cancelled => Err(e),
        _ => Ok(()),
    }
}

/// Caches a toolchain without its Android target libraries, which have
/// keys of their own.
fn store_rust_base(cache: &Path, rust: &Path, channel: &str) -> Result<bool> {
    if !valid_rust_base(rust) {
        return Ok(false);
    }
    store_filtered(cache, &rust_key(channel), rust, &|rel| {
        rel.strip_prefix("lib/rustlib/")
            .is_some_and(|t| ANDROID_TARGETS.contains(&t))
    })
}

/// Saves every cacheable tool of a finished installation.
pub fn store_installation(cache: &Path, installation: &Path, tools: &Value) -> Result<()> {
    if let Some(channel) = tools.get("rust").and_then(|r| r["channel"].as_str())
        && is_channel(channel)
    {
        let rust = installation.join("tools/rust");
        if rust.is_dir() {
            store_rust_base(cache, &rust, channel)?;
            for target in tools
                .get("android-rust-targets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|t| ANDROID_TARGETS.contains(t))
            {
                let source = rust.join("lib/rustlib").join(target);
                if source.is_dir() {
                    store(cache, &rust_target_key(channel, target), &source)?;
                }
            }
        }
    }
    for (name, key) in [
        ("java", java_key as fn(&str) -> String),
        ("android-ndk", ndk_key),
    ] {
        if let Some(version) = tools
            .get(name)
            .and_then(|t| t["version"].as_str())
            .filter(|v| !v.is_empty())
        {
            store_quietly(cache, &key(version), &installation.join("tools").join(name))?;
        }
    }
    if let Some(version) = tools
        .get("android-sdk")
        .and_then(|t| t["version"].as_str())
        .filter(|v| !v.is_empty())
    {
        let packages = tools["android-sdk"]
            .get("packages")
            .filter(|p| p.is_object())
            .cloned()
            .unwrap_or(json!({}));
        store_quietly(
            cache,
            &sdk_key(&packages, version),
            &installation.join("tools/android-sdk"),
        )?;
    }
    Ok(())
}

fn entries(cache: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(cache) else {
        return vec![];
    };
    let mut list: Vec<PathBuf> = read
        .flatten()
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    list.sort();
    list
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Deletes cached versions no installed editor names.
pub fn collect_garbage(hub_root: &Path, installations: &[Value]) -> Vec<String> {
    let live = live_keys(installations);
    let mut removed = vec![];
    for entry in entries(&cache_dir(hub_root)) {
        let name = name_of(&entry);
        if !live.contains(&name) {
            eprintln!("Removing unused cached tool {name}...");
            let _ = std::fs::remove_dir_all(&entry);
            removed.push(name);
        }
    }
    removed
}

/// What is cached, how big it is and whether an editor still uses it.
pub fn summary(hub_root: &Path, installations: &[Value]) -> Value {
    let root = cache_dir(hub_root);
    let live = live_keys(installations);
    let rows: Vec<Value> = entries(&root)
        .into_iter()
        .map(|entry| {
            let name = name_of(&entry);
            json!({"key": name, "size": size_of(&entry), "live": live.contains(&name),
                   "valid": valid_entry(&name, &entry)})
        })
        .collect();
    json!({"directory": root.to_string_lossy(), "entries": rows, "live": live})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_manifest_tools() {
        let manifest = json!({"tools": {
            "rust": {"channel": "1.98.1"}, "android-rust-targets": ["aarch64-linux-android", "bogus"],
            "java": {"version": "25.0.1+8"}, "android-ndk": {"version": "27.3.1"},
            "android-sdk": {"version": "36.0.0", "packages": {"platform-tools": "36.0.0"}}}});
        let live = live_keys(&[manifest, json!({"tools": {"rust": {"channel": "nightly"}}})]);
        assert!(live.contains(&rust_key("1.98.1")));
        assert!(live.contains("rust-std-1.98.1-aarch64-linux-android"));
        assert!(live.contains(&java_key("25.0.1+8")));
        assert!(java_key("25.0.1+8").contains("25.0.1_8"));
        assert!(live.contains(&ndk_key("27.3.1")));
        assert!(live.contains(&sdk_key(&json!({"platform-tools": "36.0.0"}), "36.0.0")));
        assert_eq!(live.len(), 5);
    }

    #[test]
    fn a_stored_tool_is_reused_and_pruned_when_unused() {
        let hub = tempfile::tempdir().unwrap();
        let java = hub.path().join("java");
        std::fs::create_dir_all(java.join("bin")).unwrap();
        std::fs::write(java.join("bin").join(exe("java")), "x").unwrap();
        let cache = cache_dir(hub.path());
        assert!(store(&cache, &java_key("25"), &java).unwrap());
        let copy = hub.path().join("copy");
        assert!(reuse(&cache, &java_key("25"), &copy).unwrap());
        assert!(copy.join("bin").join(exe("java")).is_file());
        assert!(!reuse(&cache, &java_key("26"), &hub.path().join("none")).unwrap());
        let used = json!({"tools": {"java": {"version": "25"}}});
        assert!(collect_garbage(hub.path(), &[used]).is_empty());
        assert_eq!(collect_garbage(hub.path(), &[]), vec![java_key("25")]);
    }
}
