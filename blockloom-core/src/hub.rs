//! What the Hub says about a project and about the editor running it.
//!
//! The Hub binds a project to one installation in `.blockloom/hub.json`. An
//! editor started from the Hub knows its own installation, either from
//! `BLOCKLOOM_INSTALLATION` or from the `blockloom-installation.json` beside
//! its executable. Every route that opens a project asks [`conflict`] first.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The installation manifest the Hub puts beside an editor.
const MANIFEST: &str = "blockloom-installation.json";
/// The editor's installation identity, set by the Hub when it launches one.
pub const INSTALLATION_ENV: &str = "BLOCKLOOM_INSTALLATION";

#[derive(Deserialize)]
struct Binding {
    #[serde(default)]
    installation: Option<String>,
}

#[derive(Deserialize)]
struct Installed {
    #[serde(default)]
    id: String,
}

/// The installation this editor was installed as, when the Hub manages it.
pub fn running_installation() -> Option<String> {
    if let Ok(id) = std::env::var(INSTALLATION_ENV) {
        let id = id.trim().to_string();
        return (!id.is_empty()).then_some(id);
    }
    let exe = std::env::current_exe().ok()?;
    let text = std::fs::read_to_string(exe.parent()?.join(MANIFEST)).ok()?;
    let id = serde_json::from_str::<Installed>(&text).ok()?.id;
    (!id.is_empty()).then_some(id)
}

/// The installation `dir` is bound to, if the Hub bound it to one.
pub fn bound_installation(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".blockloom").join("hub.json")).ok()?;
    serde_json::from_str::<Binding>(&text)
        .ok()?
        .installation
        .filter(|id| !id.is_empty())
}

/// What a person calls an installation: "Blockloom 0.2.0", or "Development
/// build" for a repository build. Anything else is shown as it is.
pub fn friendly_name(id: &str) -> String {
    if let Some(version) = id.strip_prefix("release-") {
        return format!("Blockloom {version}");
    }
    if id.starts_with("dev-") {
        return "Development build".to_string();
    }
    id.to_string()
}

/// Binds a project to the installation running now, whatever it was bound to.
/// Returns whether a binding was written; an editor the Hub doesn't manage
/// has nothing to bind to.
pub fn rebind(dir: &Path) -> Result<bool, String> {
    let Some(id) = running_installation() else {
        return Ok(false);
    };
    let folder = dir.join(".blockloom");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let json = serde_json::json!({"schema": 1, "installation": id});
    std::fs::write(folder.join("hub.json"), json.to_string()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Copies the project document aside before another editor opens it, since
/// opening is what may migrate it. Returns the copy's path.
pub fn backup_document(dir: &Path) -> Result<std::path::PathBuf, String> {
    let source = crate::project::project_file(dir);
    let folder = dir.join(".blockloom").join("backups");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let target = folder.join(format!("project-{stamp}.blockloom"));
    std::fs::copy(&source, &target).map_err(|e| format!("Couldn't back up the project file: {e}"))?;
    Ok(target)
}

/// Binds a project to the installation running now, unless it already has a
/// binding. Does nothing for an editor the Hub doesn't manage.
pub fn bind_new_project(dir: &Path) {
    let Some(id) = running_installation() else {
        return;
    };
    if bound_installation(dir).is_some() {
        return;
    }
    let folder = dir.join(".blockloom");
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&folder)?;
        let json = serde_json::json!({"schema": 1, "installation": id});
        std::fs::write(folder.join("hub.json"), json.to_string())
    };
    if let Err(e) = write() {
        tracing::warn!("Couldn't record the project's editor: {e}");
    }
}

/// Why opening `dir` here needs a decision, or `None` when it is fine: the
/// project is unbound, or bound to the installation running now.
pub fn conflict(dir: &Path) -> Option<String> {
    let bound = bound_installation(dir)?;
    let running = running_installation();
    if running.as_deref() == Some(bound.as_str()) {
        return None;
    }
    Some(match running {
        Some(running) => format!(
            "This project is set to {}, but this is {}. Open it from the Hub so it uses {}, or open it here anyway.",
            friendly_name(&bound),
            friendly_name(&running),
            friendly_name(&bound),
        ),
        None => format!(
            "This project is set to {}, but this editor isn't managed by the Hub. Open it from the Hub so it uses {}, or open it here anyway.",
            friendly_name(&bound),
            friendly_name(&bound),
        ),
    })
}

// ─── Editor presence and the Hub's open requests ──────────────────────────
//
// An editor at its Dashboard is a process the Hub can reuse instead of
// starting another. Each editor publishes `<data dir>/editors/<pid>.json`
// every poll; the Hub drops a request beside it and the editor answers with
// a reply. All three are written to a temp file and renamed, so a reader
// never sees half of one.

/// A request older than this is stale and ignored.
pub const REQUEST_MAX_AGE_SECS: u64 = 30;

/// What an editor says about itself, refreshed every poll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub schema: u32,
    pub pid: u32,
    /// The installation the editor is, when the Hub manages it.
    pub installation: Option<String>,
    /// The project folder it has open, or `None` at the Dashboard.
    pub project: Option<String>,
    /// Unix seconds of the last publish.
    pub heartbeat: u64,
}

/// What the Hub asks an idle editor to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub schema: u32,
    /// Echoed in the reply, so an answer belongs to exactly one request.
    pub id: String,
    /// `"open"` (with `path`) or `"create"` (with `project`).
    pub action: String,
    #[serde(default)]
    pub path: Option<String>,
    /// The arguments `create_project` takes, for `"create"`.
    #[serde(default)]
    pub project: Option<serde_json::Value>,
    pub requested_at: u64,
}

/// How a request went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub schema: u32,
    pub id: String,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// `<data dir>/editors`, where presence, requests and replies live.
pub fn editors_dir() -> PathBuf {
    crate::project::data_dir().join("editors")
}

pub fn presence_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{pid}.json"))
}

pub fn request_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{pid}.request.json"))
}

pub fn reply_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{pid}.reply.json"))
}

/// Writes `text` to `path` through a temp file beside it.
pub fn write_file_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Says this editor is here and what it has open. Best effort.
pub fn publish_presence(dir: &Path, project: Option<&Path>) {
    let presence = Presence {
        schema: 1,
        pid: std::process::id(),
        installation: running_installation(),
        project: project.map(|p| p.to_string_lossy().into_owned()),
        heartbeat: crate::sync::now_secs(),
    };
    if let Ok(text) = serde_json::to_string(&presence) {
        let _ = write_file_atomic(&presence_path(dir, presence.pid), &text);
    }
}

/// Takes this editor's waiting request, if a fresh one is there.
pub fn take_request(dir: &Path) -> Option<Request> {
    let path = request_path(dir, std::process::id());
    let text = std::fs::read_to_string(&path).ok()?;
    let parsed = serde_json::from_str::<Request>(&text).ok();
    let _ = std::fs::remove_file(&path);
    let request = parsed?;
    (crate::sync::now_secs().saturating_sub(request.requested_at) < REQUEST_MAX_AGE_SECS)
        .then_some(request)
}

/// Answers a request taken with [`take_request`].
pub fn publish_reply(dir: &Path, id: &str, outcome: Result<(), String>) {
    let reply = Reply {
        schema: 1,
        id: id.to_string(),
        ok: outcome.is_ok(),
        error: outcome.err(),
    };
    if let Ok(text) = serde_json::to_string(&reply) {
        let _ = write_file_atomic(&reply_path(dir, std::process::id()), &text);
    }
}

/// Removes this editor's presence, so the Hub stops offering it work.
pub fn retire_presence(dir: &Path) {
    let pid = std::process::id();
    let _ = std::fs::remove_file(presence_path(dir, pid));
    let _ = std::fs::remove_file(request_path(dir, pid));
    let _ = std::fs::remove_file(reply_path(dir, pid));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installations_get_names_a_person_would_say() {
        assert_eq!(friendly_name("release-0.2.0"), "Blockloom 0.2.0");
        assert_eq!(friendly_name("dev-0123456789abcdef"), "Development build");
        assert_eq!(friendly_name("odd"), "odd");
    }

    #[test]
    fn a_request_is_taken_once_and_answered() {
        let dir = std::env::temp_dir().join(format!("blockloom-editors-{}", uuid::Uuid::new_v4()));
        let pid = std::process::id();
        assert!(take_request(&dir).is_none());
        let request = Request {
            schema: 1,
            id: "r1".into(),
            action: "open".into(),
            path: Some("/p".into()),
            project: None,
            requested_at: crate::sync::now_secs(),
        };
        write_file_atomic(&request_path(&dir, pid), &serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(take_request(&dir), Some(request));
        assert!(take_request(&dir).is_none());
        publish_reply(&dir, "r1", Err("no".into()));
        let reply: Reply =
            serde_json::from_str(&std::fs::read_to_string(reply_path(&dir, pid)).unwrap()).unwrap();
        assert!(!reply.ok);
        assert_eq!(reply.error.as_deref(), Some("no"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_request_is_dropped() {
        let dir = std::env::temp_dir().join(format!("blockloom-editors-{}", uuid::Uuid::new_v4()));
        let request = Request {
            schema: 1,
            id: "old".into(),
            action: "open".into(),
            path: Some("/p".into()),
            project: None,
            requested_at: 1,
        };
        write_file_atomic(
            &request_path(&dir, std::process::id()),
            &serde_json::to_string(&request).unwrap(),
        )
        .unwrap();
        assert!(take_request(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unbound_projects_have_no_conflict() {
        let dir = std::env::temp_dir().join(format!("blockloom-hub-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(conflict(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
