//! Crash-safe file writes for documents a project folder holds.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Suffix of an in-flight write. Builds and the asset tray skip these.
pub const TEMP_SUFFIX: &str = ".blockloom-tmp";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A hidden temp path beside `path`, unique per process and call.
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}-{n}{TEMP_SUFFIX}", std::process::id()))
}

/// Writes `bytes` to `path` so a reader (or a crash) sees the old file or the
/// new one, never half of either: temp file, fsync, rename, then fsync the
/// folder where the platform allows it. Makes the parent folders.
pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    replace(path, bytes.as_ref(), true)
}

/// [`write_atomic`] without the fsyncs: whole against a process crash, not a
/// power cut. For bookkeeping rewritten often and rebuilt if lost.
pub fn replace_file(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    replace(path, bytes.as_ref(), false)
}

fn replace(path: &Path, bytes: &[u8], durable: bool) -> Result<(), String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let temp = temp_path(path);
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        if durable {
            file.sync_all()?;
        }
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{}: {e}", path.display()));
    }
    #[cfg(unix)]
    if durable
        && let Some(parent) = parent
        && let Ok(dir) = std::fs::File::open(parent)
    {
        // Makes the rename itself durable; failing here loses nothing.
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Removes temp files a crashed write left directly inside `dir`: not this
/// process's, and untouched for a minute, so another live writer keeps its own.
pub fn sweep_temp_files(dir: &Path) {
    sweep_older_than(dir, std::time::Duration::from_secs(60));
}

fn sweep_older_than(dir: &Path, age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let own = format!(".{}-", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with('.') && name.ends_with(TEMP_SUFFIX)) || name.contains(&own) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|elapsed| elapsed >= age);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("blockloom-files-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_temp_behind() {
        let dir = temp_dir();
        let path = dir.join("deep/Scene 1.blockscene");
        write_atomic(&path, "old").unwrap();
        write_atomic(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names(&dir.join("deep")), ["Scene 1.blockscene"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file() {
        let dir = temp_dir();
        let path = dir.join("project.blockloom");
        write_atomic(&path, "old").unwrap();
        // A folder where the file should go makes the rename fail.
        let blocked = dir.join("blocked");
        std::fs::create_dir_all(blocked.join("child")).unwrap();
        assert!(write_atomic(&blocked, "new").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names(&dir), ["blocked", "project.blockloom"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sweeping_removes_only_other_processes_leftovers() {
        let dir = temp_dir();
        let stale = dir.join(format!(".a.blockscene.999999999-0{TEMP_SUFFIX}"));
        let ours = temp_path(&dir.join("b.blockscene"));
        std::fs::write(&stale, "half").unwrap();
        std::fs::write(&ours, "in flight").unwrap();
        std::fs::write(dir.join("kept.blockscene"), "{}").unwrap();
        sweep_temp_files(&dir);
        assert!(stale.exists(), "too fresh to be a crash's");
        sweep_older_than(&dir, std::time::Duration::ZERO);
        assert!(!stale.exists());
        assert!(ours.exists());
        assert!(dir.join("kept.blockscene").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
