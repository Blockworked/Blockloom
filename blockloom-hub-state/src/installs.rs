//! Reading the editors the Hub has installed. Installing them is in
//! `manage`.

use crate::{Error, Hub, Result, read_json};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "blockloom-installation.json";
pub const ANDROID_TARGETS: [&str; 2] = ["aarch64-linux-android", "x86_64-linux-android"];
pub const OPTIONAL_TOOLS: [&str; 3] = ["java", "android-sdk", "android-ndk"];

/// Spelled the way `blockloom_core::build` spells it.
pub fn host_target() -> &'static str {
    let arm = cfg!(target_arch = "aarch64");
    if cfg!(windows) {
        if arm {
            "aarch64-pc-windows-msvc"
        } else {
            "x86_64-pc-windows-msvc"
        }
    } else if cfg!(target_os = "macos") {
        if arm {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else if arm {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

pub fn editor_name() -> String {
    exe("blockloom")
}

fn valid_id(id: &str) -> bool {
    if let Some(rest) = id.strip_prefix("release-") {
        return !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'));
    }
    if let Some(rest) = id.strip_prefix("dev-") {
        return rest.len() == 16 && rest.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    }
    false
}

fn is_regular(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

pub fn validate_tools(directory: &Path, tools: &Value) -> Result<()> {
    let tools = tools
        .as_object()
        .ok_or_else(|| Error::new("Installation tools must be an object"))?;
    let rust = directory.join("tools").join("rust");
    let channel_ok = tools
        .get("rust")
        .and_then(|r| r.get("channel"))
        .and_then(|c| c.as_str())
        .is_some_and(|c| {
            let parts: Vec<_> = c.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.chars().all(|d| d.is_ascii_digit()))
        });
    if !channel_ok {
        return Err(Error::new(
            "Every installation must bundle its pinned Rust toolchain",
        ));
    }
    for binary in ["rustc", "cargo"] {
        if !rust.join("bin").join(exe(binary)).is_file() {
            return Err(Error::new(format!(
                "Bundled Rust toolchain is missing {binary}"
            )));
        }
    }
    if !rust
        .join("lib")
        .join("rustlib")
        .join(host_target())
        .join("lib")
        .is_dir()
    {
        return Err(Error::new(
            "Bundled Rust toolchain is missing host standard libraries",
        ));
    }
    for name in OPTIONAL_TOOLS {
        let Some(meta) = tools.get(name) else {
            continue;
        };
        if meta
            .get("version")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
        {
            return Err(Error::new(format!(
                "Component must declare its pinned version: {name}"
            )));
        }
        let root = directory.join("tools").join(name);
        if !root.is_dir() {
            return Err(Error::new(format!(
                "Bundle is missing selected component: {name}"
            )));
        }
        let missing = match name {
            "java" => !root.join("bin").join(exe("java")).is_file(),
            "android-sdk" => !root.join("platform-tools").join(exe("adb")).is_file(),
            _ => !root.join("source.properties").is_file(),
        };
        if missing {
            return Err(Error::new(format!("Bundled {name} is incomplete")));
        }
    }
    for target in tools
        .get("android-rust-targets")
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str())
    {
        if !ANDROID_TARGETS.contains(&target)
            || !rust
                .join("lib")
                .join("rustlib")
                .join(target)
                .join("lib")
                .is_dir()
        {
            return Err(Error::new(format!(
                "Bundle is missing Android Rust target: {target}"
            )));
        }
    }
    Ok(())
}

impl Hub {
    pub fn slot(&self, identity: &str) -> Result<PathBuf> {
        if !valid_id(identity) {
            return Err(Error::new("Invalid installation ID"));
        }
        Ok(self.root.join("installations").join(identity))
    }

    /// An installation's manifest. With `ready`, also that it is built and
    /// its payload and tools are all there.
    pub fn installation(&self, identity: &str, ready: bool) -> Result<Value> {
        let directory = self.slot(identity)?;
        if std::fs::symlink_metadata(&directory).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Error::new("Installation directory cannot be a symlink"));
        }
        let manifest = read_json(&directory.join(MANIFEST))?;
        if manifest["id"] != identity || manifest["target"] != host_target() {
            return Err(Error::new(format!(
                "Invalid installation manifest: {}",
                directory.display()
            )));
        }
        if !matches!(manifest["kind"].as_str(), Some("release" | "dev")) {
            return Err(Error::new("Unknown installation kind"));
        }
        if ready {
            if manifest["status"] != "ready" {
                return Err(Error::new("Build this developer installation first"));
            }
            for name in ["blockloom", "blockloom-runtime"] {
                let binary = directory.join(exe(name));
                if !is_regular(&binary) {
                    return Err(Error::new(format!(
                        "Missing regular binary: {}",
                        binary.display()
                    )));
                }
            }
            validate_tools(&directory, manifest.get("tools").unwrap_or(&json!({})))?;
        }
        Ok(manifest)
    }

    /// Every installation, with how many editors run it and whether a dev
    /// build is prepared.
    pub fn installations(&self) -> Result<Vec<Value>> {
        let directory = self.root.join("installations");
        let Ok(read) = std::fs::read_dir(&directory) else {
            return Ok(vec![]);
        };
        let mut names: Vec<String> = read
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        let active = self.running_editors();
        let mut rows = vec![];
        for name in names {
            let mut entry = self.installation(&name, false)?;
            entry["path"] = json!(self.slot(&name)?.to_string_lossy());
            let count = active
                .iter()
                .filter(|r| r["installation"] == entry["id"])
                .count();
            if count > 0 {
                entry["running"] = json!(count);
            }
            let candidate = self.root.join("pending").join(&name).join(MANIFEST);
            if entry["kind"] == "dev" && candidate.exists() {
                entry["prepared"] = json!(true);
            }
            rows.push(entry);
        }
        Ok(rows)
    }

    /// The editor a project with none gets: the newest release, else any
    /// ready installation.
    pub fn default_installation(&self) -> Result<Option<String>> {
        let ready: Vec<Value> = self
            .installations()?
            .into_iter()
            .filter(|i| i["status"] == "ready")
            .collect();
        let version_key = |i: &Value| -> Vec<u64> {
            i["version"]
                .as_str()
                .unwrap_or("")
                .split(['.', '-'])
                .map(|n| n.parse().unwrap_or(0))
                .collect()
        };
        let mut releases: Vec<&Value> = ready.iter().filter(|i| i["kind"] == "release").collect();
        releases.sort_by_key(|i| version_key(i));
        let pick = releases.last().copied().or(ready.last());
        Ok(pick.and_then(|i| i["id"].as_str().map(str::to_string)))
    }

    /// Who uses an installation, for the guards that refuse to remove or
    /// rebuild one that is in use.
    pub fn installation_usage(&self, identity: &str) -> Result<Value> {
        self.slot(identity)?;
        let running = self
            .running_editors()
            .iter()
            .filter(|r| r["installation"] == identity)
            .count();
        let mut bound = vec![];
        let mut active = vec![];
        for entry in self.projects()? {
            if entry["installation"] != identity || entry.get("error").is_some_and(|e| !e.is_null())
            {
                continue;
            }
            let name = entry["name"].as_str().unwrap_or("").to_string();
            let label = if name.is_empty() {
                entry["path"].as_str().unwrap_or("").to_string()
            } else {
                name
            };
            bound.push(label.clone());
            if entry["active"] == true {
                active.push(label);
            }
        }
        Ok(json!({"running": running, "bound": bound, "active": active}))
    }
}
