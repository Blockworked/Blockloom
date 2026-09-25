//! Live project sync between the editor window and headless backends.
//!
//! The shell and the MCP server used to fork a silent second copy of whatever
//! project the editor had open, and the last writer won without saying so.
//! This module is the shared footing that replaces that:
//!
//! - a per-folder owner lock (`.blockloom/lock.json`) naming the PID, session
//!   and heartbeat of whoever owns the folder, so a second opener attaches or
//!   takes over explicitly instead of forking quietly;
//! - a revision counter (`.blockloom/revision.json`) bumped on every save, so
//!   an idle backend can tell its in-memory copy went stale and reload it
//!   straight off disk.
//!
//! Both files live under `.blockloom/` beside the pipeline manifest and the
//! script build cache. They are best-effort coordination, not a database: a
//! missing or unreadable file reads as "no lock" or "revision 0", and a bump
//! that cannot be written never fails the save it rode along with.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Folder inside a project dir holding coordination files.
pub const SYNC_DIR: &str = ".blockloom";
/// Owner lock file, under [`SYNC_DIR`].
pub const LOCK_FILE: &str = "lock.json";
/// Revision counter file, under [`SYNC_DIR`].
pub const REVISION_FILE: &str = "revision.json";

/// Seconds without a heartbeat after which a lock counts as stale on machines
/// where the owner's PID cannot be checked.
pub const HEARTBEAT_STALE_SECS: u64 = 30;
/// Minimum seconds between heartbeat rewrites by one backend.
pub const HEARTBEAT_TOUCH_EVERY_SECS: u64 = 10;

/// Who owns a project folder right now: one backend's claim on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockInfo {
    /// OS process holding the folder.
    pub pid: u32,
    /// Backend instance holding it - one editor window and one shell run
    /// never share one, so a lock with our own session is ours.
    pub session: String,
    /// Unix seconds of the last heartbeat.
    pub heartbeat: u64,
    /// Human name for the holder, like `"editor"` or `"shell"`.
    #[serde(default)]
    pub app: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RevisionFile {
    #[serde(default)]
    revision: u64,
}

/// Unix seconds now, or 0 when the clock is unreadable.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// A fresh backend session id.
pub fn new_session() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// This process's PID.
pub fn own_pid() -> u32 {
    std::process::id()
}

fn sync_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(SYNC_DIR).join(name)
}

/// Where the owner lock for `dir` lives.
pub fn lock_path(dir: &Path) -> PathBuf {
    sync_file(dir, LOCK_FILE)
}

/// Where the revision counter for `dir` lives.
pub fn revision_path(dir: &Path) -> PathBuf {
    sync_file(dir, REVISION_FILE)
}

/// Writes `text` to `path` atomically, through a temp file beside it.
fn write_atomic(path: &Path, text: &str) {
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let tmp = path.with_extension(format!("tmp.{}", own_pid()));
    if std::fs::write(&tmp, text).is_err() {
        return;
    }
    let _ = std::fs::rename(&tmp, path);
}

/// The lock currently on `dir`, if it parses.
pub fn read_lock(dir: &Path) -> Option<LockInfo> {
    let text = std::fs::read_to_string(lock_path(dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Claims `dir` for `lock`, overwriting whatever was there.
pub fn write_lock(dir: &Path, lock: &LockInfo) -> Result<(), String> {
    if let Some(parent) = lock_path(dir).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string(lock).map_err(|e| e.to_string())?;
    let path = lock_path(dir);
    let tmp = path.with_extension(format!("tmp.{}", own_pid()));
    std::fs::write(&tmp, &text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Drops the lock on `dir`, but only when it is ours. Returns whether one
/// was removed.
pub fn release_lock(dir: &Path, session: &str) -> bool {
    match read_lock(dir) {
        Some(lock) if lock.session == session => std::fs::remove_file(lock_path(dir)).is_ok(),
        _ => false,
    }
}

/// Whether `pid` names a live process. Linux reads `/proc` directly; other
/// platforms have no dependency-free check, so they answer false and let the
/// heartbeat decide instead.
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 || pid == own_pid() {
        return pid == own_pid();
    }
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Whether a lock still means someone is there: its process is alive, or its
/// heartbeat is fresh. Our own lock is always live to us.
pub fn lock_is_live(lock: &LockInfo) -> bool {
    pid_alive(lock.pid) || now_secs().saturating_sub(lock.heartbeat) < HEARTBEAT_STALE_SECS
}

/// A live lock on `dir` held by anyone but `our_session`, if there is one. A
/// backend seeing this should attach or take over explicitly, never fork a
/// silent second copy.
pub fn live_owner(dir: &Path, our_session: &str) -> Option<LockInfo> {
    let lock = read_lock(dir)?;
    if lock.session == our_session || !lock_is_live(&lock) {
        return None;
    }
    Some(lock)
}

/// Refreshes the heartbeat on `dir` when its lock is ours. Returns whether
/// the file was rewritten.
pub fn refresh_heartbeat(dir: &Path, session: &str, app: &str) -> bool {
    let mut lock = match read_lock(dir) {
        Some(lock) if lock.session == session => lock,
        _ => return false,
    };
    lock.heartbeat = now_secs();
    if app.is_empty() {
        // Keep whoever named the holder first.
    } else {
        lock.app = app.to_string();
    }
    write_lock(dir, &lock).is_ok()
}

/// The revision `dir` is at. Missing or unreadable means 0.
pub fn read_revision(dir: &Path) -> u64 {
    let Ok(text) = std::fs::read_to_string(revision_path(dir)) else {
        return 0;
    };
    serde_json::from_str::<RevisionFile>(&text)
        .map(|file| file.revision)
        .unwrap_or(0)
}

/// Bumps `dir` to the next revision and answers it. Best-effort: when the
/// file cannot be written the save it rode with still counts, so the next
/// save retries from the same base.
pub fn bump_revision(dir: &Path) -> u64 {
    let next = read_revision(dir).saturating_add(1);
    let text = serde_json::json!({ "revision": next }).to_string();
    write_atomic(&revision_path(dir), &text);
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "blockloom-sync-{}-{}",
            name,
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("a temp dir");
        path
    }

    fn lock(session: &str) -> LockInfo {
        LockInfo {
            pid: own_pid(),
            session: session.to_string(),
            heartbeat: now_secs(),
            app: "test".to_string(),
        }
    }

    #[test]
    fn a_missing_lock_reads_as_none() {
        assert!(read_lock(&temp_dir("missing")).is_none());
    }

    #[test]
    fn a_lock_round_trips() {
        let dir = temp_dir("roundtrip");
        let info = lock("s1");
        write_lock(&dir, &info).unwrap();
        assert_eq!(read_lock(&dir).unwrap(), info);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn our_own_lock_is_never_a_foreign_owner() {
        let dir = temp_dir("self");
        write_lock(&dir, &lock("s1")).unwrap();
        assert!(live_owner(&dir, "s1").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_foreign_lock_shows_its_owner() {
        let dir = temp_dir("foreign");
        // A fresh heartbeat counts as live even where the PID is our own.
        write_lock(&dir, &lock("other")).unwrap();
        let owner = live_owner(&dir, "s1").expect("a live owner");
        assert_eq!(owner.session, "other");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dead_lock_is_not_an_owner() {
        let dir = temp_dir("dead");
        let mut info = lock("gone");
        // A PID that cannot exist, with an old heartbeat.
        info.pid = u32::MAX - 1000;
        info.heartbeat = now_secs().saturating_sub(HEARTBEAT_STALE_SECS + 5);
        write_lock(&dir, &info).unwrap();
        assert!(!lock_is_live(&info));
        assert!(live_owner(&dir, "s1").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_only_removes_our_own_lock() {
        let dir = temp_dir("release");
        write_lock(&dir, &lock("other")).unwrap();
        assert!(!release_lock(&dir, "s1"));
        assert!(read_lock(&dir).is_some());
        assert!(release_lock(&dir, "other"));
        assert!(read_lock(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn heartbeat_refresh_keeps_ours_and_ignores_theirs() {
        let dir = temp_dir("heartbeat");
        let mut info = lock("s1");
        info.heartbeat = now_secs().saturating_sub(HEARTBEAT_STALE_SECS + 5);
        write_lock(&dir, &info).unwrap();
        assert!(refresh_heartbeat(&dir, "s1", "test"));
        assert!(lock_is_live(&read_lock(&dir).unwrap()));
        assert!(!refresh_heartbeat(&dir, "s2", "test"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revisions_start_at_zero_and_bump_by_one() {
        let dir = temp_dir("revision");
        assert_eq!(read_revision(&dir), 0);
        assert_eq!(bump_revision(&dir), 1);
        assert_eq!(bump_revision(&dir), 2);
        assert_eq!(read_revision(&dir), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
