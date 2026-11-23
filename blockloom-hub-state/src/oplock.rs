//! The Hub's operation lock: one mutating operation at a time.

use crate::{Error, Result, process, read_json, write_json};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct OperationLock {
    path: PathBuf,
}

fn is_stale(lock: &Path) -> bool {
    if let Ok(owner) = read_json(&lock.join("owner.json"))
        && let Some(pid) = owner.get("pid").and_then(|p| p.as_u64())
    {
        return !process::alive(pid as u32);
    }
    // No owner file: an older Hub's lock, or one still being made.
    std::fs::metadata(lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_none_or(|age| age.as_secs() > 60)
}

impl OperationLock {
    pub fn acquire(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let path = root.join(".operation-lock");
        for attempt in 0..2 {
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    let lock = Self { path };
                    write_json(
                        &lock.path.join("owner.json"),
                        &serde_json::json!({"pid": std::process::id()}),
                    )?;
                    return Ok(lock);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt == 0 && is_stale(&path) {
                        let _ = std::fs::remove_dir_all(&path);
                        continue;
                    }
                    return Err(Error::new(
                        "Another Hub operation is running. Wait for it to finish, then try again.",
                    ));
                }
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!("the second attempt returns")
    }
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
