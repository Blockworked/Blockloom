//! Where a package comes from, and fetching one into a staging folder.
//!
//! A fetch only ever lands in staging. Nothing becomes part of the cache or a
//! project until it has been verified as a whole.

use crate::package::{copy_dir, unpack_archive};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

/// The registry name that means "whatever the shared cache already holds".
/// It works offline.
pub const CACHE_REGISTRY: &str = "cache";

/// A package source, spelled as one string in `plugins.json` and the lock:
/// `path:<dir>`, `archive:<file.zip>`, `git:<url>#<40-hex commit>` or
/// `registry:<name>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Source {
    Path(PathBuf),
    Archive(PathBuf),
    Git { url: String, rev: String },
    Registry(String),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Path(p) => write!(f, "path:{}", p.display()),
            Source::Archive(p) => write!(f, "archive:{}", p.display()),
            Source::Git { url, rev } => write!(f, "git:{url}#{rev}"),
            Source::Registry(name) => write!(f, "registry:{name}"),
        }
    }
}

impl From<Source> for String {
    fn from(source: Source) -> String {
        source.to_string()
    }
}

impl TryFrom<String> for Source {
    type Error = String;
    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

impl FromStr for Source {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, String> {
        let (kind, rest) = text.split_once(':').ok_or_else(|| {
            format!("\"{text}\" is not a source (use path:, archive:, git: or registry:)")
        })?;
        if rest.is_empty() {
            return Err(format!("\"{text}\" names no location"));
        }
        match kind {
            "path" => Ok(Source::Path(rest.into())),
            "archive" => Ok(Source::Archive(rest.into())),
            "registry" => Ok(Source::Registry(rest.to_string())),
            "git" => {
                let (url, rev) = rest
                    .rsplit_once('#')
                    .ok_or("a git source must pin a commit: git:<url>#<commit>")?;
                if rev.len() != 40 || !rev.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("a git source must pin a full 40-character commit".to_string());
                }
                Ok(Source::Git {
                    url: url.to_string(),
                    rev: rev.to_ascii_lowercase(),
                })
            }
            other => Err(format!("unknown source kind \"{other}\"")),
        }
    }
}

impl Source {
    /// Needs the network or a registry, so an offline install can't use it.
    pub fn is_remote(&self) -> bool {
        match self {
            Source::Git { .. } => true,
            Source::Registry(name) => name != CACHE_REGISTRY,
            _ => false,
        }
    }

    /// Resolves a relative local source against `base` (the project folder).
    pub fn anchored(&self, base: &Path) -> Source {
        match self {
            Source::Path(p) if p.is_relative() => Source::Path(base.join(p)),
            Source::Archive(p) if p.is_relative() => Source::Archive(base.join(p)),
            other => other.clone(),
        }
    }
}

/// Fetches the package a non-registry source names into `staging`, which must
/// be an empty folder, and returns the package root inside it.
pub fn fetch_local(source: &Source, staging: &Path) -> Result<PathBuf, String> {
    match source {
        Source::Path(dir) => {
            copy_dir(dir, staging)?;
            Ok(staging.to_path_buf())
        }
        Source::Archive(file) => {
            unpack_archive(file, staging)?;
            Ok(staging.to_path_buf())
        }
        Source::Git { url, rev } => {
            fetch_git(url, rev, staging)?;
            Ok(staging.to_path_buf())
        }
        Source::Registry(name) => Err(format!("registry:{name} is fetched through its registry")),
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// A shallow fetch of exactly `rev`, with the repository metadata removed.
fn fetch_git(url: &str, rev: &str, staging: &Path) -> Result<(), String> {
    std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
    git(staging, &["init", "--quiet"])?;
    git(staging, &["fetch", "--quiet", "--depth", "1", url, rev])?;
    git(staging, &["checkout", "--quiet", "FETCH_HEAD"])?;
    std::fs::remove_dir_all(staging.join(".git")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;
    use crate::package::fixtures::declarative;
    use serde_json::json;

    #[test]
    fn sources_round_trip_through_their_spelling() {
        let rev = "a".repeat(40);
        for text in [
            "path:../plugins/a".to_string(),
            "archive:/tmp/a.zip".to_string(),
            format!("git:https://example.com/a.git#{rev}"),
            "registry:main".to_string(),
        ] {
            let source: Source = text.parse().unwrap();
            assert_eq!(source.to_string(), text);
            assert_eq!(
                serde_json::from_value::<Source>(serde_json::to_value(&source).unwrap()).unwrap(),
                source
            );
        }
    }

    #[test]
    fn git_sources_must_pin_a_commit() {
        assert!("git:https://example.com/a.git".parse::<Source>().is_err());
        assert!(
            "git:https://example.com/a.git#main"
                .parse::<Source>()
                .is_err()
        );
        assert!("nonsense".parse::<Source>().is_err());
        assert!("path:".parse::<Source>().is_err());
    }

    #[test]
    fn remote_sources_are_the_ones_offline_cannot_use() {
        assert!(Source::Registry("x".into()).is_remote());
        assert!(!Source::Registry(CACHE_REGISTRY.into()).is_remote());
        assert!(!Source::Path("x".into()).is_remote());
        assert!(!Source::Archive("x".into()).is_remote());
    }

    #[test]
    fn a_pinned_git_commit_fetches() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let repo = declarative(dir.path(), "com.example.g", "1.0.0", json!({}));
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "--quiet",
                "-m",
                "x",
            ],
        ] {
            git(&repo, &args).unwrap();
        }
        let rev = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let rev = String::from_utf8(rev.stdout).unwrap().trim().to_string();
        let source = Source::Git {
            url: format!("file://{}", repo.display()),
            rev,
        };
        let staging = dir.path().join("staging");
        let root = fetch_local(&source, &staging).unwrap();
        assert_eq!(Package::load(&root).unwrap().manifest.id, "com.example.g");
        assert!(!staging.join(".git").exists());
    }
}
