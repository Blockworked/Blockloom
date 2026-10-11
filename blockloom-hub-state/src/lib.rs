//! What the Hub knows about projects and the editors running them.
//!
//! One implementation of the project list, who owns each project, binding a
//! project to an editor, snapshots, and starting or reusing an editor. The
//! Hub window and `scripts/hub.py` both reach it through [`cli::run`], so
//! there is no second copy of these rules to drift. Installing, downloading
//! and building editors stay in the Python service.

pub mod backup;
pub mod cli;
pub mod editors;
pub mod installs;
pub mod oplock;
pub mod process;
pub mod projects;

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A failure, or a cancellation the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub message: String,
    pub cancelled: bool,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cancelled: false,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new(e.to_string())
    }
}
impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Self::new(e.to_string())
    }
}
impl From<String> for Error {
    fn from(e: String) -> Self {
        Self::new(e)
    }
}
impl From<&str> for Error {
    fn from(e: &str) -> Self {
        Self::new(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Seconds a freshly launched editor may take to claim its project.
pub const STARTUP_GRACE: u64 = 20;
/// Seconds an editor's presence stays believable without a refresh.
pub const PRESENCE_FRESH_SECS: u64 = 6;
/// Environment variable naming a file whose existence cancels the operation.
pub const CANCEL_FILE_ENV: &str = "BLOCKLOOM_HUB_CANCEL_FILE";

/// Where the Hub keeps its installations, logs and backups.
pub fn hub_root() -> PathBuf {
    if let Some(over) = std::env::var_os("BLOCKLOOM_HUB_DIR") {
        let path = PathBuf::from(over);
        return std::fs::canonicalize(&path).unwrap_or(path);
    }
    if cfg!(windows) {
        return dirs::data_local_dir()
            .unwrap_or_default()
            .join("Blockloom")
            .join("Hub");
    }
    if cfg!(target_os = "macos") {
        return dirs::data_dir()
            .unwrap_or_default()
            .join("Blockloom")
            .join("Hub");
    }
    dirs::data_dir().unwrap_or_default().join("blockloom-hub")
}

/// The editor's own project list, shared with the Hub.
pub fn project_registry() -> PathBuf {
    blockloom_core::project::data_dir().join("projects.json")
}

pub fn read_json(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&text)?;
    if !value.is_object() {
        return Err(Error::new(format!("Expected a JSON object: {}", path.display())));
    }
    Ok(value)
}

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    blockloom_core::hub::write_file_atomic(path, &text)?;
    Ok(())
}

/// Unix seconds, fractional.
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Fails with a cancellation when the Hub asked this operation to stop.
pub fn checkpoint() -> Result<()> {
    if let Some(path) = std::env::var_os(CANCEL_FILE_ENV)
        && Path::new(&path).exists()
    {
        return Err(Error {
            message: "Operation cancelled; previous installation kept".into(),
            cancelled: true,
        });
    }
    Ok(())
}

/// Normalises a path the way the library keys its entries.
pub fn key(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy().into_owned();
    if cfg!(windows) { text.to_lowercase() } else { text }
}

/// The Hub itself: a root for its own files and the editor's project list.
#[derive(Debug, Clone)]
pub struct Hub {
    pub root: PathBuf,
    pub registry: PathBuf,
    /// Where editors publish presence and receive requests.
    pub editors: PathBuf,
    /// How long to wait for a started editor to claim its project.
    pub open_timeout: Duration,
    /// How long to wait for an editor to hear a focus request.
    pub focus_timeout: Duration,
    /// How long an idle editor has to take an open request before a new one
    /// is started instead.
    pub reuse_timeout: Duration,
}

impl Hub {
    pub fn new(root: Option<PathBuf>, registry: Option<PathBuf>) -> Self {
        let root = root.unwrap_or_else(hub_root);
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        let registry = registry.unwrap_or_else(project_registry);
        let editors = registry
            .parent()
            .map(|p| p.join("editors"))
            .unwrap_or_else(|| PathBuf::from("editors"));
        let seconds = |name: &str, default: f64| {
            let value = std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(default);
            Duration::from_secs_f64(value.max(0.0))
        };
        Self {
            root,
            registry,
            editors,
            open_timeout: seconds("BLOCKLOOM_HUB_OPEN_TIMEOUT", 15.0),
            focus_timeout: seconds("BLOCKLOOM_HUB_FOCUS_TIMEOUT", 4.0),
            reuse_timeout: seconds("BLOCKLOOM_HUB_REUSE_TIMEOUT", 4.0),
        }
    }
}
