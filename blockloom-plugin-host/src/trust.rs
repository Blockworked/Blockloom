//! Which plugins the user has let run code inside the editor.
//!
//! A package that ships editor modules (QML with host-level access) is only
//! loaded once the user trusts it. The grant is kept per user, not in the
//! project, so a project cannot trust itself, and it names the package's exact
//! content hash: an update asks again.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One trusted plugin: the content it was trusted at, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Granted {
    pub hash: String,
    pub granted_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustLedger {
    #[serde(default)]
    trusted: BTreeMap<String, Granted>,
}

/// `<data dir>/blockloom/trusted-plugins.json`, or under `BLOCKLOOM_DATA_DIR`.
pub fn default_path() -> PathBuf {
    std::env::var_os("BLOCKLOOM_DATA_DIR")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("blockloom")
        .join("trusted-plugins.json")
}

impl TrustLedger {
    /// Reads the ledger. A missing or unreadable file trusts nothing.
    pub fn read(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, text).map_err(|e| format!("{}: {e}", temp.display()))?;
        fs::rename(&temp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Whether `id` is trusted at exactly this content.
    pub fn is_trusted(&self, id: &str, hash: &str) -> bool {
        self.trusted.get(id).is_some_and(|g| g.hash == hash)
    }

    pub fn granted(&self, id: &str) -> Option<&Granted> {
        self.trusted.get(id)
    }

    /// Trusts `id` at `hash`, replacing any earlier grant.
    pub fn grant(&mut self, id: &str, hash: &str) {
        let granted_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.trusted.insert(
            id.to_string(),
            Granted {
                hash: hash.to_string(),
                granted_at,
            },
        );
    }

    /// Takes the grant back; whether there was one.
    pub fn revoke(&mut self, id: &str) -> bool {
        self.trusted.remove(id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_is_per_plugin_and_per_content() {
        let dir = std::env::temp_dir().join(format!("blockloom-trust-{}", std::process::id()));
        let path = dir.join("trusted-plugins.json");
        let _ = fs::remove_dir_all(&dir);
        // Nothing is trusted before a file exists.
        assert!(!TrustLedger::read(&path).is_trusted("a", "h1"));

        let mut ledger = TrustLedger::default();
        ledger.grant("a", "h1");
        ledger.write(&path).unwrap();
        let again = TrustLedger::read(&path);
        assert!(again.is_trusted("a", "h1"));
        // Another plugin, or an updated package, is not.
        assert!(!again.is_trusted("b", "h1"));
        assert!(!again.is_trusted("a", "h2"));
        assert!(again.granted("a").is_some());

        let mut ledger = again;
        assert!(ledger.revoke("a"));
        assert!(!ledger.revoke("a"));
        ledger.write(&path).unwrap();
        assert!(!TrustLedger::read(&path).is_trusted("a", "h1"));

        // A damaged file trusts nothing rather than failing.
        fs::write(&path, "{ not json").unwrap();
        assert!(!TrustLedger::read(&path).is_trusted("a", "h1"));
        let _ = fs::remove_dir_all(&dir);
    }
}
