//! The OS keyring half of Android release signing (no new crates).
//!
//! Passwords are never stored in the project file or the app config, but
//! typing them on every build gets old: with the user's opt-in per build
//! (the Build dialog's remember checkbox, or `rememberPasswords` headless)
//! they are kept in the platform's own credential store instead - macOS
//! Keychain through `security`, Linux Secret Service through `secret-tool`.
//! Windows has no scriptable retrieval in v1 (`cmdkey` cannot read back),
//! so the keyring reads as unavailable there and the env stays the
//! headless path.
//!
//! Everything here shells out to tools already on the machine, std only, so
//! there is nothing new to fetch and no daemon to link. A missing tool, a
//! locked store or no D-Bus session reads as absent or failed, never a
//! panic: the build falls back to asking, which always works.

use std::path::PathBuf;

/// The credential store label every entry is kept under.
pub const SERVICE: &str = "Blockloom";

/// Which password an entry holds: the keystore's, or the key's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Store,
    Key,
}

impl Purpose {
    fn suffix(self) -> &'static str {
        match self {
            Purpose::Store => "store",
            Purpose::Key => "key",
        }
    }
}

/// The account an entry is kept under: which key file, which alias inside
/// it, and which of its two passwords. Keyed tightly enough that two games
/// sharing one keystore file never read each other's passwords.
pub fn account_for(keystore: &str, alias: &str, purpose: Purpose) -> String {
    format!(
        "android-release-{}:{}@{}",
        purpose.suffix(),
        alias.trim(),
        keystore.trim()
    )
}

/// How passwords reach the store on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// macOS Keychain: `security add|find|delete-generic-password`.
    Security,
    /// Secret Service API: `secret-tool store|lookup|clear`.
    SecretTool,
}

/// A found credential store: the tool plus where it lives.
#[derive(Debug, Clone)]
pub struct Keyring {
    tool: PathBuf,
    kind: Tool,
}

impl Keyring {
    /// The store on this machine, if it has a scriptable one. macOS first
    /// (its `security` is always there), then `secret-tool` on PATH.
    /// Windows answers None in v1: nothing there retrieves a password.
    pub fn probe() -> Option<Keyring> {
        if cfg!(target_os = "macos") {
            let tool = PathBuf::from("/usr/bin/security");
            if tool.is_file() {
                return Some(Keyring {
                    tool,
                    kind: Tool::Security,
                });
            }
        }
        probe_path()
    }

    /// Whether `account` holds a password in the store. Missing entries and
    /// a locked or unreachable store both read as false; only real I/O
    /// failures (the tool won't run) are errors.
    pub fn has(&self, account: &str) -> Result<bool, String> {
        Ok(self.read(account)?.is_some())
    }

    /// The password under `account`, or None when nothing is stored there.
    /// A wrong-but-present entry still answers Some; apksigner is what
    /// rejects it at the end of the build.
    pub fn read(&self, account: &str) -> Result<Option<String>, String> {
        let output = match self.kind {
            Tool::Security => std::process::Command::new(&self.tool)
                .arg("find-generic-password")
                .arg("-a")
                .arg(account)
                .arg("-s")
                .arg(SERVICE)
                .arg("-w")
                .output()
                .map_err(|e| format!("Couldn't run {}: {e}", self.tool.display()))?,
            Tool::SecretTool => std::process::Command::new(&self.tool)
                .arg("lookup")
                .arg("service")
                .arg(SERVICE)
                .arg("account")
                .arg(account)
                .output()
                .map_err(|e| format!("Couldn't run {}: {e}", self.tool.display()))?,
        };
        if !output.status.success() {
            return Ok(None);
        }
        let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(if secret.is_empty() {
            None
        } else {
            Some(secret)
        })
    }

    /// Stores `secret` under `account`, replacing what was there. macOS
    /// deletes first since `add-generic-password` refuses duplicates without
    /// `-U` on older releases; `secret-tool store` already replaces.
    pub fn write(&self, account: &str, secret: &str) -> Result<(), String> {
        if secret.is_empty() {
            return Err("Won't store an empty password in the keyring.".to_string());
        }
        match self.kind {
            Tool::Security => {
                let _ = std::process::Command::new(&self.tool)
                    .arg("delete-generic-password")
                    .arg("-a")
                    .arg(account)
                    .arg("-s")
                    .arg(SERVICE)
                    .output();
                let output = std::process::Command::new(&self.tool)
                    .arg("add-generic-password")
                    .arg("-a")
                    .arg(account)
                    .arg("-s")
                    .arg(SERVICE)
                    .arg("-w")
                    .arg(secret)
                    .output()
                    .map_err(|e| format!("Couldn't run {}: {e}", self.tool.display()))?;
                if !output.status.success() {
                    return Err(format!(
                        "The keychain wouldn't keep it: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                Ok(())
            }
            Tool::SecretTool => {
                use std::io::Write;
                let mut child = std::process::Command::new(&self.tool)
                    .arg("store")
                    .arg(format!("--label=Blockloom release key ({account})"))
                    .arg("service")
                    .arg(SERVICE)
                    .arg("account")
                    .arg(account)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("Couldn't run {}: {e}", self.tool.display()))?;
                child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| "Couldn't feed secret-tool its password.".to_string())?
                    .write_all(secret.as_bytes())
                    .map_err(|e| format!("Couldn't feed secret-tool its password: {e}"))?;
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("Couldn't run {}: {e}", self.tool.display()))?;
                if !output.status.success() {
                    return Err(format!(
                        "The keyring wouldn't keep it (is a Secret Service daemon running?): {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                Ok(())
            }
        }
    }

    /// Deletes the entry under `account`. Answers whether one was there: a
    /// read first, since `secret-tool clear` exits happily with nothing to
    /// clear while `security delete` errors, and the answer should not
    /// depend on the tool. Nothing there is quiet, not an error.
    pub fn delete(&self, account: &str) -> Result<bool, String> {
        if !self.has(account)? {
            return Ok(false);
        }
        let output = match self.kind {
            Tool::Security => std::process::Command::new(&self.tool)
                .arg("delete-generic-password")
                .arg("-a")
                .arg(account)
                .arg("-s")
                .arg(SERVICE)
                .output()
                .map_err(|e| format!("Couldn't run {}: {e}", self.tool.display()))?,
            Tool::SecretTool => std::process::Command::new(&self.tool)
                .arg("clear")
                .arg("service")
                .arg(SERVICE)
                .arg("account")
                .arg(account)
                .output()
                .map_err(|e| format!("Couldn't run {}: {e}", self.tool.display()))?,
        };
        if !output.status.success() {
            return Err(format!(
                "The keyring wouldn't forget it: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(true)
    }
}

/// `secret-tool` by PATH lookup, the Linux half of `probe`. Split out so
/// tests can hand a fake tool straight to the protocol below.
fn probe_path() -> Option<Keyring> {
    if cfg!(target_os = "windows") {
        return None;
    }
    if let Some(path) = secret_tool_path() {
        return Some(Keyring {
            tool: path,
            kind: Tool::SecretTool,
        });
    }
    None
}

/// Where `secret-tool` lives: `BLOCKLOOM_ANDROID_SECRET_TOOL` wins when it
/// names a runnable file (a custom install, or a test stub), else the PATH
/// lookup. Empty clears back to the lookup.
fn secret_tool_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BLOCKLOOM_ANDROID_SECRET_TOOL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    // A bare name runs through PATH; only accept it when it exists there.
    if std::process::Command::new("secret-tool")
        .arg("--help")
        .output()
        .is_ok()
    {
        return Some(PathBuf::from("secret-tool"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn keyring_with(tool: &Path, kind: Tool) -> Keyring {
        Keyring {
            tool: tool.to_path_buf(),
            kind,
        }
    }

    /// A fake `secret-tool`: `store` appends `account secret` lines to a
    /// file, `lookup` prints the last secret for the account, `clear` drops
    /// its lines. Argv-shaped like the real one, so the protocol is what's
    /// tested, not the stub.
    #[cfg(unix)]
    fn stub_secret_tool(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let db = dir.join("entries.txt");
        let path = dir.join("secret-tool");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ndb=\"{}\"\nset -- \"$@\"\ncmd=\"$1\"; shift\nacc=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"account\" ]; then acc=\"$a\"; fi\n  prev=\"$a\"\ndone\ncase \"$cmd\" in\nstore) read secret; touch \"$db\"; grep -v \"^$acc \" \"$db\" 2>/dev/null > \"$db.tmp\" || true; mv \"$db.tmp\" \"$db\"; echo \"$acc $secret\" >> \"$db\";;\nlookup) touch \"$db\"; grep \"^$acc \" \"$db\" 2>/dev/null | tail -1 | cut -d' ' -f2-;;\nclear) touch \"$db\"; grep -v \"^$acc \" \"$db\" 2>/dev/null > \"$db.tmp\" || true; mv \"$db.tmp\" \"$db\";;\n*) exit 1;;\nesac\n",
                db.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn accounts_name_the_file_alias_and_purpose() {
        let store = account_for("/keys/r.keystore", "upload", Purpose::Store);
        let key = account_for("/keys/r.keystore", "upload", Purpose::Key);
        assert_ne!(store, key);
        assert!(store.contains("upload"));
        assert!(store.contains("/keys/r.keystore"));
        assert_eq!(
            account_for(" /keys/r.keystore ", " upload ", Purpose::Store),
            store
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_secret_tool_round_trip_replaces_and_forgets() {
        let root = std::env::temp_dir().join(format!("blockloom-keyring-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let tool = stub_secret_tool(&root);
        let ring = keyring_with(&tool, Tool::SecretTool);
        assert_eq!(ring.read("pond-store").unwrap(), None);
        assert!(!ring.has("pond-store").unwrap());
        ring.write("pond-store", "secret1").unwrap();
        assert_eq!(ring.read("pond-store").unwrap().as_deref(), Some("secret1"));
        // A second write replaces rather than piling up.
        ring.write("pond-store", "secret2").unwrap();
        assert_eq!(ring.read("pond-store").unwrap().as_deref(), Some("secret2"));
        assert!(ring.delete("pond-store").unwrap());
        assert_eq!(ring.read("pond-store").unwrap(), None);
        // Deleting twice is quiet, not an error.
        assert!(!ring.delete("pond-store").unwrap());
        // Empty secrets never reach the store.
        assert!(ring.write("pond-store", "").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn the_live_probe_never_errors_on_read() {
        // No assertion on the answer: headless CI has no Secret Service
        // daemon, a desktop does. What matters is a probed store answers
        // Ok (missing entries read as None), never Err or a panic.
        if let Some(ring) = Keyring::probe() {
            assert!(ring.read("blockloom-probe-nobody").is_ok());
        }
    }
}
