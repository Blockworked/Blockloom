//! Private release catalogs and assets through the user's GitHub CLI login.

use crate::fetch::catalog_data;
use crate::run::run;
use crate::{Error, Result, checkpoint};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const DEFAULT_REPO: &str = "Blockworked/Blockloom";
pub const CATALOG_ASSET: &str = "blockloom-catalog.json";
const MIB: f64 = 1024.0 * 1024.0;

/// `owner/repo`, plainly spelled.
pub fn repository(value: &str) -> Result<&str> {
    let part = |p: &str| {
        p.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    match value.split_once('/') {
        Some((owner, name)) if part(owner) && part(name) => Ok(value),
        _ => Err(Error::new("GitHub repository must be owner/repo")),
    }
}

fn cli() -> Result<PathBuf> {
    if let Some(explicit) = std::env::var_os("BLOCKLOOM_HUB_GH") {
        return Ok(PathBuf::from(explicit));
    }
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            Error::new("Install GitHub CLI and run gh auth login before checking private releases")
        })
}

fn capture(arguments: &[&str]) -> Result<Value> {
    let output = tempfile::NamedTempFile::new()?;
    let mut command = Command::new(cli()?);
    command.args(arguments).stdout(output.reopen()?);
    run(&mut command, None)?;
    let data = std::fs::read(output.path())?;
    if data.len() > 4 * 1024 * 1024 {
        return Err(Error::new("GitHub response exceeds the size limit"));
    }
    Ok(serde_json::from_slice(&data)?)
}

/// Downloads one release asset to `destination`, reporting its growth since
/// `gh` itself says nothing while it works.
pub fn asset(
    repo: &str,
    tag: &str,
    name: &str,
    destination: &Path,
    size: Option<u64>,
) -> Result<()> {
    repository(repo)?;
    if tag.is_empty() || tag.starts_with('-') {
        return Err(Error::new("Invalid GitHub release tag"));
    }
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(Error::new("Invalid GitHub asset name"));
    }
    if destination.exists() {
        return Err(Error::new("Asset download destination already exists"));
    }
    match size {
        Some(size) => eprintln!(
            "Downloading {name} via GitHub CLI ({:.0} MiB expected)...",
            size as f64 / MIB
        ),
        None => eprintln!("Downloading {name} via GitHub CLI..."),
    }
    let done = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (done, path) = (done.clone(), destination.to_path_buf());
        std::thread::spawn(move || {
            let mut last = 0;
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
                let current = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if current != last && current > 0 {
                    match size {
                        Some(s) => eprintln!(
                            "Downloading: {:.1} MiB / {:.1} MiB",
                            current as f64 / MIB,
                            s as f64 / MIB
                        ),
                        None => eprintln!("Downloading: {:.1} MiB", current as f64 / MIB),
                    }
                    last = current;
                }
            }
        })
    };
    let mut command = Command::new(cli()?);
    command
        .args([
            "release",
            "download",
            tag,
            "--repo",
            repo,
            "--pattern",
            name,
            "--output",
        ])
        .arg(destination);
    let result = run(&mut command, None);
    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    result?;
    checkpoint()?;
    let final_size = std::fs::metadata(destination).map(|m| m.len()).unwrap_or(0);
    eprintln!("Downloaded {name} ({:.1} MiB).", final_size as f64 / MIB);
    Ok(())
}

/// Every release of `repo` that carries a catalog, with `target`'s entries.
pub fn catalog(repo: &str, target: &str) -> Result<Vec<Value>> {
    let repo = repository(repo)?;
    let releases = capture(&["api", &format!("repos/{repo}/releases?per_page=100")])?;
    let releases = releases
        .as_array()
        .ok_or_else(|| Error::new("Unexpected GitHub releases response"))?;
    let mut entries = vec![];
    let mut seen = HashSet::new();
    for release in releases {
        checkpoint()?;
        if release["draft"] == true {
            continue;
        }
        let assets = release["assets"].as_array().cloned().unwrap_or_default();
        let Some(listed) = assets.iter().find(|a| a["name"] == CATALOG_ASSET) else {
            continue;
        };
        if listed["size"].as_u64().unwrap_or(0) > 2 * 1024 * 1024 {
            return Err(Error::new("GitHub release catalog exceeds the size limit"));
        }
        let tag = release["tag_name"].as_str().unwrap_or("");
        let temporary = tempfile::Builder::new().prefix("hub-catalog-").tempdir()?;
        let path = temporary.path().join(CATALOG_ASSET);
        asset(repo, tag, CATALOG_ASSET, &path, None)?;
        if std::fs::metadata(&path)?.len() > 2 * 1024 * 1024 {
            return Err(Error::new("GitHub release catalog exceeds the size limit"));
        }
        let selected = catalog_data(&serde_json::from_slice(&std::fs::read(&path)?)?, target)?;
        for mut entry in selected {
            let name = entry["asset"].as_str().unwrap_or("").to_string();
            let matching = assets.iter().find(|a| a["name"] == name.as_str());
            if matching.is_none_or(|a| a["size"] != entry["size"]) {
                return Err(Error::new(
                    "Catalog asset is missing or has a different size",
                ));
            }
            let version = entry["version"].as_str().unwrap_or("").to_string();
            if !seen.insert(version) {
                return Err(Error::new(
                    "GitHub releases contain duplicate Blockloom versions",
                ));
            }
            entry["github"] = json!({"repo": repo, "tag": tag, "asset": name});
            entries.push(entry);
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_plain() {
        assert!(repository("Blockworked/Blockloom").is_ok());
        for bad in ["noslash", "-x/y", "a/b/c", "a/"] {
            assert!(repository(bad).is_err(), "{bad}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(asset("a/b", "--help", "x", &dir.path().join("x"), None).is_err());
        assert!(asset("a/b", "v1", "../x", &dir.path().join("x"), None).is_err());
    }
}
