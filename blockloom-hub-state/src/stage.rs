//! Putting a payload into an installation slot in one rename, so a slot is
//! always either the old editor or the whole new one.

use crate::installs::{MANIFEST, exe};
use crate::run::copy_tree;
use crate::{Error, Result, checkpoint, read_json, write_json};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Both editor binaries are there as regular files.
pub fn validate_payload(directory: &Path) -> Result<()> {
    for name in ["blockloom", "blockloom-runtime"] {
        let binary = directory.join(exe(name));
        if !std::fs::symlink_metadata(&binary).is_ok_and(|m| m.is_file()) {
            return Err(Error::new(format!(
                "Missing regular binary: {}",
                binary.display()
            )));
        }
    }
    Ok(())
}

fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or(path.to_path_buf()))
}

fn check_apart(source: &Path, destination: &Path) -> Result<()> {
    let (s, d) = (absolute(source), absolute(destination));
    if s == d || d.starts_with(&s) || s.starts_with(&d) {
        return Err(Error::new(
            "Installation source and destination cannot contain each other",
        ));
    }
    if std::fs::symlink_metadata(destination).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::new("Installation destination cannot be a symlink"));
    }
    Ok(())
}

/// Swaps `payload` in at `destination`, putting the old one back if that
/// fails.
fn swap(payload: &Path, destination: &Path, temporary: &Path) -> Result<()> {
    let backup = temporary.join("previous");
    if destination.exists() {
        std::fs::rename(destination, &backup)?;
    }
    if let Err(e) = std::fs::rename(payload, destination) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, destination);
        }
        return Err(e.into());
    }
    Ok(())
}

fn write_manifest(directory: &Path, manifest: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(manifest)?;
    text.push('\n');
    std::fs::write(directory.join(MANIFEST), text)?;
    Ok(())
}

/// Moves a finished private payload into place without copying it again.
/// Only a dev installation may be replaced.
pub fn promote(source: &Path, destination: &Path, manifest: &Value, replace: bool) -> Result<()> {
    validate_payload(source)?;
    check_apart(source, destination)?;
    if destination.exists() {
        if !replace {
            return Err(Error::new(format!(
                "Installation already exists: {}",
                destination.display()
            )));
        }
        if read_json(&destination.join(MANIFEST))?["kind"] != "dev" {
            return Err(Error::new(
                "Released or unrecognized installations cannot be replaced",
            ));
        }
    }
    let parent = destination.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    write_manifest(source, manifest)?;
    checkpoint()?;
    let temporary = tempfile::Builder::new()
        .prefix(".promote-")
        .tempdir_in(parent)?;
    swap(source, destination, temporary.path())?;
    if temporary.path().join("previous").exists() {
        eprintln!("Removing the previous prepared payload...");
    }
    Ok(())
}

/// Copies `source` into `destination` (leaving out `excluded` relative
/// paths), writes `manifest` if given, and swaps it in.
pub fn stage(
    source: &Path,
    destination: &Path,
    manifest: Option<&Value>,
    replace: bool,
    excluded: &[String],
) -> Result<()> {
    validate_payload(source)?;
    check_apart(source, destination)?;
    if destination.exists() && !replace {
        return Err(Error::new(format!(
            "Installation already exists: {}",
            destination.display()
        )));
    }
    if destination.join(MANIFEST).is_file() {
        match read_json(&destination.join(MANIFEST))?["kind"].as_str() {
            Some("release") => return Err(Error::new("Released installations cannot be replaced")),
            Some("dev") => {}
            _ => return Err(Error::new("Unrecognized installation manifest")),
        }
    } else if destination.exists() {
        validate_payload(destination)?;
    }
    let parent = destination.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temporary = tempfile::Builder::new()
        .prefix(".stage-")
        .tempdir_in(parent)?;
    let payload = temporary.path().join("payload");
    copy_tree(source, &payload, &|rel| excluded.iter().any(|e| e == rel))?;
    if let Some(manifest) = manifest {
        write_json(&payload.join(MANIFEST), manifest)?;
    }
    validate_payload(&payload)?;
    checkpoint()?;
    swap(&payload, destination, temporary.path())
}
