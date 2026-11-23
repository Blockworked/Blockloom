//! Installing, downloading, building and removing editors.

use crate::fetch::{self, Extract};
use crate::github;
use crate::installs::{ANDROID_TARGETS, MANIFEST, OPTIONAL_TOOLS, host_target, validate_tools};
use crate::oplock::OperationLock;
use crate::run::{bundle_rust, run};
use crate::stage::{promote, stage, validate_payload};
use crate::toolchain::{
    android_versions, default_android_versions, install_android_tools, install_rust_targets,
};
use crate::tools;
use crate::{Error, Hub, Result, checkpoint, read_json, write_json};
use serde_json::{Map, Value, json};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Approximate download sizes in MiB; the real ones are known on download.
const MODULE_SIZES: [(&str, u64); 4] = [
    ("java", 200),
    ("android-sdk", 250),
    ("android-ndk", 1000),
    ("android-rust-targets", 60),
];
const MIB: f64 = 1024.0 * 1024.0;

fn has_android_targets(rust: &Path) -> Vec<&'static str> {
    ANDROID_TARGETS
        .into_iter()
        .filter(|t| !rust.join("lib/rustlib").join(t).join("lib").is_dir())
        .collect()
}

fn check_components(components: &[String], what: &str) -> Result<()> {
    for name in components {
        if !OPTIONAL_TOOLS.contains(&name.as_str()) {
            return Err(Error::new(format!("{what}: {name}")));
        }
    }
    Ok(())
}

fn workspace_version(repo: &Path) -> Result<String> {
    let cargo: toml::Table = toml::from_str(&std::fs::read_to_string(repo.join("Cargo.toml"))?)
        .map_err(|e| Error::new(format!("Cargo.toml is invalid: {e}")))?;
    cargo
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::new("Cargo.toml has no workspace version"))
}

fn targets_json() -> Value {
    json!(ANDROID_TARGETS)
}

fn temp_in(root: &Path, prefix: &str) -> Result<tempfile::TempDir> {
    std::fs::create_dir_all(root)?;
    Ok(tempfile::Builder::new().prefix(prefix).tempdir_in(root)?)
}

impl Hub {
    pub fn tool_cache_dir(&self) -> PathBuf {
        tools::cache_dir(&self.root)
    }

    /// Manifests for garbage collection; a corrupt slot is skipped.
    fn cache_installations(&self) -> Vec<Value> {
        let Ok(read) = std::fs::read_dir(self.root.join("installations")) else {
            return vec![];
        };
        let mut names: Vec<String> = read
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        names
            .into_iter()
            .filter_map(|n| self.installation(&n, false).ok())
            .collect()
    }

    fn collect_garbage(&self) -> Vec<String> {
        tools::collect_garbage(&self.root, &self.cache_installations())
    }

    pub fn tool_cache(&self) -> Value {
        tools::summary(&self.root, &self.cache_installations())
    }

    pub fn prune_tools(&self) -> Result<Value> {
        let _lock = OperationLock::acquire(&self.root)?;
        let removed = self.collect_garbage();
        let mut summary = self.tool_cache();
        summary["removed"] = json!(removed);
        Ok(summary)
    }

    fn require_installation_idle(&self, identity: &str) -> Result<()> {
        let usage = self.installation_usage(identity)?;
        if usage["running"].as_u64().unwrap_or(0) > 0 {
            return Err(Error::new(
                "Close editors using this installation before rebuilding it",
            ));
        }
        if usage["active"].as_array().is_some_and(|a| !a.is_empty()) {
            return Err(Error::new(
                "Close projects using this installation before rebuilding it",
            ));
        }
        Ok(())
    }

    pub fn candidate_path(&self, identity: &str) -> Result<PathBuf> {
        self.slot(identity)?;
        Ok(self.root.join("pending").join(identity))
    }

    pub fn uninstall(&self, identity: &str) -> Result<Value> {
        let _lock = OperationLock::acquire(&self.root)?;
        let manifest = self.installation(identity, false)?;
        self.require_installation_idle(identity)?;
        let usage = self.installation_usage(identity)?;
        let mut bound: Vec<String> = usage["bound"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| b.as_str().map(str::to_string))
            .collect();
        if !bound.is_empty() {
            bound.sort();
            bound.dedup();
            return Err(Error::new(format!(
                "Installation {identity} is used by projects: {}. Assign those projects another editor first",
                bound.join(", ")
            )));
        }
        checkpoint()?;
        std::fs::remove_dir_all(self.slot(identity)?)?;
        let candidate = self.candidate_path(identity)?;
        if candidate.exists() {
            std::fs::remove_dir_all(candidate)?;
        }
        self.collect_garbage();
        Ok(json!({"id": identity, "kind": manifest["kind"], "version": manifest["version"]}))
    }

    /// The Android pins an installation (or a fresh one) downloads.
    fn versions_for(&self, manifest: &Value) -> Value {
        if let Some(versions) = manifest.get("android_versions").filter(|v| v.is_object()) {
            return versions.clone();
        }
        if manifest["kind"] == "dev"
            && let Some(repo) = manifest["repo"].as_str()
            && let Ok(versions) = android_versions(Path::new(repo))
        {
            return versions;
        }
        default_android_versions()
    }

    /// Downloads optional tools into an existing installation. The editor
    /// is untouched; only `tools/` and the manifest grow.
    pub fn add_tools(
        &self,
        identity: &str,
        components: &[String],
        rust_targets: bool,
    ) -> Result<Value> {
        check_components(components, "Unknown component")?;
        let _lock = OperationLock::acquire(&self.root)?;
        let mut manifest = self.installation(identity, true)?;
        self.require_installation_idle(identity)?;
        let directory = self.slot(identity)?;
        let mut selected: Map<String, Value> =
            manifest["tools"].as_object().cloned().unwrap_or_default();
        let missing: Vec<String> = components
            .iter()
            .filter(|c| !selected.contains_key(*c))
            .cloned()
            .collect();
        let rust = directory.join("tools/rust");
        let absent = has_android_targets(&rust);
        let listed = selected
            .get("android-rust-targets")
            .and_then(Value::as_array)
            .is_some_and(|t| ANDROID_TARGETS.iter().all(|a| t.iter().any(|x| x == a)));
        let want_targets = rust_targets && (!absent.is_empty() || !listed);
        if missing.is_empty() && !want_targets {
            manifest["added"] = json!([]);
            return Ok(manifest);
        }
        let versions = self.versions_for(&manifest);
        let mut added = vec![];
        let temporary = temp_in(&self.root, ".add-tools-")?;
        let cache = self.tool_cache_dir();
        if !missing.is_empty() {
            eprintln!("Downloading optional tools: {}...", missing.join(", "));
            selected.extend(install_android_tools(
                &directory,
                &versions,
                &missing,
                temporary.path(),
                Some(&cache),
            )?);
            added.extend(missing.iter().cloned());
        }
        if want_targets {
            let channel = selected
                .get("rust")
                .and_then(|r| r["channel"].as_str())
                .unwrap_or("")
                .to_string();
            eprintln!(
                "Downloading Android Rust targets: {}...",
                ANDROID_TARGETS.join(", ")
            );
            install_rust_targets(
                &rust,
                &channel,
                &ANDROID_TARGETS,
                temporary.path(),
                Some(&cache),
            )?;
            selected.insert("android-rust-targets".into(), targets_json());
            added.push("android-rust-targets".into());
        }
        let selected = Value::Object(selected);
        validate_payload(&directory)?;
        validate_tools(&directory, &selected)?;
        checkpoint()?;
        manifest["tools"] = selected.clone();
        write_json(&directory.join(MANIFEST), &manifest)?;
        tools::store_installation(&cache, &directory, &selected)?;
        self.collect_garbage();
        eprintln!("Modules added.");
        manifest["added"] = json!(added);
        Ok(manifest)
    }

    /// The optional modules with their pinned versions and rough sizes.
    pub fn modules(&self, identity: Option<&str>) -> Result<Value> {
        let versions = match identity {
            Some(id) => self.versions_for(&self.installation(id, false)?),
            None => default_android_versions(),
        };
        let pin = |name: &str| versions[name].as_str().unwrap_or("").to_string();
        let platform = pin("PLATFORM").replace("android-", "API ");
        let mut rows = vec![
            json!({"id": "java", "label": format!("Java {}", pin("JDK_MAJOR")), "tools": ["java"]}),
            json!({"id": "android-sdk", "label": format!("Android SDK ({platform}, build tools {})", pin("BUILD_TOOLS")),
                   "tools": ["android-sdk"]}),
            json!({"id": "android-ndk", "label": format!("Android NDK {}", pin("NDK_MAJOR")), "tools": ["android-ndk"]}),
            json!({"id": "android-rust-targets", "label": "Android Rust targets (ARM64 and x86-64)",
                   "tools": ["android-rust-targets"]}),
        ];
        for row in &mut rows {
            let size = MODULE_SIZES
                .iter()
                .find(|(id, _)| row["id"] == *id)
                .map(|(_, s)| *s);
            row["approx_mib"] = json!(size);
        }
        let ids: Vec<Value> = rows.iter().map(|r| r["id"].clone()).collect();
        Ok(
            json!({"modules": rows, "defaults": {"release": [], "dev": ids}, "android_build_tools": ids}),
        )
    }

    /// Registers a Blockloom checkout as a development installation.
    pub fn add_dev(&self, repo: &Path) -> Result<Value> {
        let repo = std::fs::canonicalize(repo)?;
        let cargo: toml::Table = toml::from_str(&std::fs::read_to_string(repo.join("Cargo.toml"))?)
            .map_err(|e| Error::new(format!("Cargo.toml is invalid: {e}")))?;
        let member = cargo
            .get("workspace")
            .and_then(|w| w.get("members"))
            .and_then(|m| m.as_array())
            .is_some_and(|m| m.iter().any(|x| x.as_str() == Some("blockloom-qt")));
        if !member || !repo.join("justfile").is_file() {
            return Err(Error::new(
                "Select a Blockloom repository containing its workspace and justfile",
            ));
        }
        let mut key = repo.to_string_lossy().into_owned();
        if cfg!(windows) {
            key = key.to_lowercase();
        }
        let hash: String = sha2::Sha256::digest(key.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let identity = format!("dev-{}", &hash[..16]);
        let _lock = OperationLock::acquire(&self.root)?;
        let directory = self.slot(&identity)?;
        if directory.exists() {
            return self.installation(&identity, false);
        }
        let result = json!({"id": identity, "kind": "dev", "repo": repo.to_string_lossy(),
            "name": repo.file_name().unwrap_or_default().to_string_lossy(),
            "version": workspace_version(&repo)?, "target": host_target(), "status": "unbuilt"});
        write_json(&directory.join(MANIFEST), &result)?;
        Ok(result)
    }

    /// Installs a release bundle folder, with the chosen optional tools.
    pub fn install(
        &self,
        bundle: &Path,
        components: &[String],
        rust_targets: bool,
    ) -> Result<Value> {
        let bundle = std::fs::canonicalize(bundle)?;
        let manifest = read_json(&bundle.join(MANIFEST))?;
        let version = fetch::version(manifest["version"].as_str().unwrap_or(""))?.to_string();
        if manifest["kind"] != "release" || manifest["target"] != host_target() {
            return Err(Error::new("Bundle must be a release for this host target"));
        }
        let identity = format!("release-{version}");
        let available = manifest.get("tools").cloned().unwrap_or(json!({}));
        let available = available
            .as_object()
            .ok_or_else(|| Error::new("Installation tools must be an object"))?;
        let mut selected = Map::new();
        selected.insert(
            "rust".into(),
            available.get("rust").cloned().unwrap_or(json!({})),
        );
        for name in components {
            match available.get(name) {
                Some(tool) if OPTIONAL_TOOLS.contains(&name.as_str()) => {
                    selected.insert(name.clone(), tool.clone());
                }
                _ => {
                    return Err(Error::new(format!(
                        "Component unavailable in this bundle: {name}"
                    )));
                }
            }
        }
        if rust_targets {
            let listed = available
                .get("android-rust-targets")
                .and_then(Value::as_array);
            let complete = listed.is_some_and(|t| {
                t.len() == ANDROID_TARGETS.len()
                    && ANDROID_TARGETS.iter().all(|a| t.iter().any(|x| x == a))
            });
            if !complete {
                return Err(Error::new(
                    "Bundle must include both supported Android Rust targets",
                ));
            }
            selected.insert("android-rust-targets".into(), targets_json());
        }
        let selected = Value::Object(selected);
        validate_tools(&bundle, &selected)?;
        let mut excluded: Vec<String> = OPTIONAL_TOOLS
            .iter()
            .filter(|t| selected.get(**t).is_none())
            .map(|t| format!("tools/{t}"))
            .collect();
        if !rust_targets {
            excluded.extend(
                ANDROID_TARGETS
                    .iter()
                    .map(|t| format!("tools/rust/lib/rustlib/{t}")),
            );
        }
        let result = json!({"id": identity, "kind": "release", "version": version,
            "target": host_target(), "status": "ready", "tools": selected});
        let _lock = OperationLock::acquire(&self.root)?;
        let slot = self.slot(&identity)?;
        stage(&bundle, &slot, Some(&result), false, &excluded)?;
        // Share this bundle's tools with later editors, then drop versions
        // nothing uses any more.
        tools::store_installation(&self.tool_cache_dir(), &slot, &result["tools"])?;
        self.collect_garbage();
        Ok(result)
    }

    /// Where releases are listed, optionally changing it.
    pub fn release_settings(
        &self,
        source: Option<&str>,
        url: Option<&str>,
        repo: Option<&str>,
    ) -> Result<Value> {
        let path = self.root.join("catalog.json");
        let mut config = json!({"source": "github-cli", "repo": github::DEFAULT_REPO, "url": ""});
        if path.exists() {
            for (k, v) in read_json(&path)?.as_object().cloned().unwrap_or_default() {
                config[k] = v;
            }
        }
        if let Some(source) = source {
            if !matches!(source, "https" | "github-cli") {
                return Err(Error::new("Invalid release source"));
            }
            let current = config["repo"]
                .as_str()
                .unwrap_or(github::DEFAULT_REPO)
                .to_string();
            let repo = github::repository(repo.unwrap_or(&current))?.to_string();
            config["source"] = json!(source);
            config["repo"] = json!(repo);
            config["url"] = json!(url.unwrap_or(""));
            if source == "https" && !url.unwrap_or("").is_empty() {
                fetch::secure_url(url.unwrap_or(""))?;
            }
            let _lock = OperationLock::acquire(&self.root)?;
            write_json(&path, &config)?;
        }
        Ok(config)
    }

    pub fn check_releases(&self, url: Option<&str>) -> Result<Value> {
        let mut config = self.release_settings(None, None, None)?;
        let mark = |entries: Vec<Value>| -> Result<Vec<Value>> {
            entries
                .into_iter()
                .map(|mut e| {
                    let id = format!("release-{}", e["version"].as_str().unwrap_or(""));
                    e["installed"] = json!(self.slot(&id)?.exists());
                    Ok(e)
                })
                .collect()
        };
        if config["source"] == "github-cli" && url.is_none() {
            let repo = config["repo"]
                .as_str()
                .unwrap_or(github::DEFAULT_REPO)
                .to_string();
            config["releases"] = json!(mark(github::catalog(&repo, host_target())?)?);
            return Ok(config);
        }
        let url = url
            .map(str::to_string)
            .or_else(|| std::env::var("BLOCKLOOM_HUB_CATALOG_URL").ok())
            .or_else(|| config["url"].as_str().map(str::to_string))
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                Error::new("Enter a release catalog URL to check for Blockloom versions")
            })?;
        let entries = fetch::catalog(&url, host_target())?;
        config["source"] = json!("https");
        config["url"] = json!(url);
        {
            let _lock = OperationLock::acquire(&self.root)?;
            write_json(&self.root.join("catalog.json"), &config)?;
        }
        config["releases"] = json!(mark(entries)?);
        Ok(config)
    }

    /// Downloads a catalog release, adds the chosen tools and installs it.
    pub fn download_release(
        &self,
        version: &str,
        components: &[String],
        rust_targets: bool,
        sha256: Option<&str>,
    ) -> Result<Value> {
        eprintln!("Stage: Checking releases for Blockloom {version}");
        let releases = self.check_releases(None)?["releases"].clone();
        let entry = releases
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| e["version"] == version)
            .cloned()
            .ok_or_else(|| Error::new("Release is not available for this computer"))?;
        if entry["installed"] == true {
            return Err(Error::new("This Blockloom version is already installed"));
        }
        let declared = entry["sha256"].as_str().unwrap_or("");
        if let Some(expected) = sha256
            && fetch::digest(expected)? != fetch::digest(declared)?
        {
            return Err(Error::new(
                "The release catalog changed. Check releases again before installing",
            ));
        }
        let size = entry["size"].as_u64().unwrap_or(0);
        let temporary = temp_in(&self.root, ".download-")?;
        let archive = temporary.path().join("release.archive");
        eprintln!(
            "Stage: Downloading Blockloom {version} ({:.0} MiB)",
            size as f64 / MIB
        );
        if let Some(gh) = entry.get("github").filter(|g| g.is_object()) {
            github::asset(
                gh["repo"].as_str().unwrap_or(""),
                gh["tag"].as_str().unwrap_or(""),
                gh["asset"].as_str().unwrap_or(""),
                &archive,
                Some(size),
            )?;
            eprintln!("Stage: Verifying the download");
            fetch::verify_file(&archive, declared, size)?;
        } else {
            fetch::download(
                entry["url"].as_str().unwrap_or(""),
                &archive,
                declared,
                Some(size),
                fetch::MAX_ARCHIVE,
                fetch::Checksum::Sha256,
            )?;
        }
        let unpacked = entry["unpacked_size"].as_u64().unwrap_or(0);
        eprintln!(
            "Stage: Extracting Blockloom {version} ({:.0} MiB unpacked)",
            unpacked as f64 / MIB
        );
        let output = temporary.path().join("release");
        fetch::extract(
            &archive,
            &output,
            entry["format"].as_str().unwrap_or(""),
            Extract::bundle(unpacked),
        )?;
        let bundles: Vec<PathBuf> = if output.join(MANIFEST).is_file() {
            vec![output.clone()]
        } else {
            std::fs::read_dir(&output)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.join(MANIFEST).is_file())
                .collect()
        };
        if bundles.len() != 1 || read_json(&bundles[0].join(MANIFEST))?["version"] != version {
            return Err(Error::new(
                "Downloaded bundle does not match the selected release",
            ));
        }
        let bundle = &bundles[0];
        let mut manifest = read_json(&bundle.join(MANIFEST))?;
        // Bundles ship Rust; Java and the Android SDK/NDK come from upstream,
        // so the choices work even for catalogs that never listed them.
        let need: Vec<String> = components
            .iter()
            .filter(|c| {
                OPTIONAL_TOOLS.contains(&c.as_str()) && manifest["tools"].get(c.as_str()).is_none()
            })
            .cloned()
            .collect();
        if !need.is_empty() {
            let versions = entry
                .get("android_versions")
                .or(manifest.get("android_versions"))
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or_else(default_android_versions);
            eprintln!("Downloading optional tools: {}...", need.join(", "));
            let added = install_android_tools(
                bundle,
                &versions,
                &need,
                temporary.path(),
                Some(&self.tool_cache_dir()),
            )?;
            if !manifest["tools"].is_object() {
                manifest["tools"] = json!({});
            }
            for (k, v) in added {
                manifest["tools"][k] = v;
            }
            write_json(&bundle.join(MANIFEST), &manifest)?;
        }
        if rust_targets {
            let rust = bundle.join("tools/rust");
            let channel = manifest["tools"]["rust"]["channel"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let missing = has_android_targets(&rust);
            if !missing.is_empty() {
                eprintln!(
                    "Downloading Android Rust targets: {}...",
                    missing.join(", ")
                );
                install_rust_targets(
                    &rust,
                    &channel,
                    &missing,
                    temporary.path(),
                    Some(&self.tool_cache_dir()),
                )?;
                manifest["tools"]["android-rust-targets"] = targets_json();
                write_json(&bundle.join(MANIFEST), &manifest)?;
            }
        }
        eprintln!("Stage: Installing Blockloom {version}");
        self.install(bundle, components, rust_targets)
    }

    pub fn rebuild(&self, identity: &str, jobs: Option<u32>) -> Result<Value> {
        self.prepare_dev(identity, jobs)?;
        let all: Vec<String> = OPTIONAL_TOOLS.iter().map(|s| s.to_string()).collect();
        self.install_dev(identity, &all, true, None)
    }

    /// Builds a development checkout into a prepared payload beside its
    /// slot, so the installed editor keeps working until it is swapped in.
    pub fn prepare_dev(&self, identity: &str, jobs: Option<u32>) -> Result<Value> {
        let _lock = OperationLock::acquire(&self.root)?;
        let mut manifest = self.installation(identity, false)?;
        if manifest["kind"] != "dev" {
            return Err(Error::new("Released installations cannot be rebuilt"));
        }
        self.require_installation_idle(identity)?;
        let repo = PathBuf::from(manifest["repo"].as_str().unwrap_or(""));
        let version = workspace_version(&repo)?;
        eprintln!("Stage: Preparing the build (step 1 of 3)");
        manifest["version"] = json!(version);
        manifest["status"] = json!("ready");
        manifest["build_id"] = json!(uuid::Uuid::new_v4().simple().to_string());
        let temporary = temp_in(&self.root, ".build-")?;
        let output = temporary.path().join("installation");
        eprintln!("Stage: Building the editor (step 2 of 3)");
        let mut command = Command::new(which("just").unwrap_or_else(|| PathBuf::from("just")));
        command
            .arg("replace")
            .current_dir(&repo)
            .env("BLOCKLOOM_INSTALL_DIR", &output);
        if let Some(jobs) = jobs {
            command.arg(jobs.to_string());
        }
        run(
            &mut command,
            Some(&self.root.join("logs").join(format!("{identity}-build.log"))),
        )?;
        checkpoint()?;
        let toolchain: toml::Table =
            toml::from_str(&std::fs::read_to_string(repo.join("rust-toolchain.toml"))?)
                .map_err(|e| Error::new(format!("rust-toolchain.toml is invalid: {e}")))?;
        let channel = toolchain
            .get("toolchain")
            .and_then(|t| t.get("channel"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| Error::new("rust-toolchain.toml has no channel"))?
            .to_string();
        let cache = self.tool_cache_dir();
        let rust = output.join("tools/rust");
        if tools::reuse_or_drop(&cache, &tools::rust_key(&channel), &rust)? {
            eprintln!("Reusing cached Rust {channel} for the prepared editor...");
        } else {
            eprintln!("Bundling Rust {channel} for the prepared editor...");
            let found = Command::new("rustup")
                .args(["which", "--toolchain", &channel, "rustc"])
                .current_dir(&repo)
                .output()?;
            if !found.status.success() {
                return Err(Error::new(format!("rustup has no Rust {channel}")));
            }
            let rustc = PathBuf::from(String::from_utf8_lossy(&found.stdout).trim());
            let home = rustc
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| Error::new("rustc has no toolchain folder"))?;
            bundle_rust(home, &rust, &channel, &ANDROID_TARGETS)?;
            tools::store(&cache, &tools::rust_key(&channel), &rust)?;
        }
        manifest["tools"] = json!({"rust": {"channel": channel}});
        validate_tools(&output, &manifest["tools"])?;
        if let Some(object) = manifest.as_object_mut() {
            object.remove("sources");
            object.remove("android_targets_available");
        }
        manifest["android_versions"] = android_versions(&repo)?;
        checkpoint()?;
        promote(&output, &self.candidate_path(identity)?, &manifest, true)?;
        eprintln!("Build prepared. Choose installation options in the Hub.");
        Ok(manifest)
    }

    pub fn dev_options(&self, identity: &str) -> Result<Value> {
        if self.installation(identity, false)?["kind"] != "dev" {
            return Err(Error::new(
                "Only developer installations have prepared builds",
            ));
        }
        let manifest = read_json(&self.candidate_path(identity)?.join(MANIFEST))?;
        if manifest["id"] != identity || manifest["kind"] != "dev" {
            return Err(Error::new("Invalid prepared development installation"));
        }
        Ok(manifest)
    }

    /// Installs a prepared development build with the chosen tools.
    pub fn install_dev(
        &self,
        identity: &str,
        components: &[String],
        rust_targets: bool,
        build_id: Option<&str>,
    ) -> Result<Value> {
        let _lock = OperationLock::acquire(&self.root)?;
        if self.installation(identity, false)?["kind"] != "dev" {
            return Err(Error::new(
                "Only developer installations can use a prepared build",
            ));
        }
        self.require_installation_idle(identity)?;
        let candidate = self.candidate_path(identity)?;
        let mut manifest = read_json(&candidate.join(MANIFEST))?;
        if manifest["id"] != identity
            || manifest["kind"] != "dev"
            || manifest["target"] != host_target()
        {
            return Err(Error::new("Invalid prepared development installation"));
        }
        if build_id.is_some_and(|b| manifest["build_id"] != b) {
            return Err(Error::new(
                "A newer development build was prepared. Reopen its installation options before installing",
            ));
        }
        check_components(components, "Unknown development tool")?;
        let mut selected = Map::new();
        selected.insert("rust".into(), manifest["tools"]["rust"].clone());
        if rust_targets {
            selected.insert("android-rust-targets".into(), targets_json());
        }
        let excluded: Vec<String> = if rust_targets {
            vec![]
        } else {
            ANDROID_TARGETS
                .iter()
                .map(|t| format!("tools/rust/lib/rustlib/{t}"))
                .collect()
        };
        let temporary = temp_in(&self.root, ".configure-")?;
        let output = temporary.path().join("installation");
        eprintln!("Stage: Installing the editor (step 3 of 3)");
        stage(&candidate, &output, None, false, &excluded)?;
        let cache = self.tool_cache_dir();
        if !components.is_empty() {
            let versions = self.versions_for(&manifest);
            selected.extend(install_android_tools(
                &output,
                &versions,
                components,
                temporary.path(),
                Some(&cache),
            )?);
        }
        if rust_targets {
            let channel = selected["rust"]["channel"]
                .as_str()
                .unwrap_or("")
                .to_string();
            install_rust_targets(
                &output.join("tools/rust"),
                &channel,
                &ANDROID_TARGETS,
                temporary.path(),
                Some(&cache),
            )?;
        }
        if let Some(object) = manifest.as_object_mut() {
            object.remove("sources");
            object.remove("android_targets_available");
        }
        let selected = Value::Object(selected);
        manifest["tools"] = selected.clone();
        validate_tools(&output, &selected)?;
        checkpoint()?;
        eprintln!("Stage: Saving the installation");
        let slot = self.slot(identity)?;
        promote(&output, &slot, &manifest, true)?;
        tools::store_installation(&cache, &slot, &selected)?;
        self.collect_garbage();
        eprintln!("Development installation ready.");
        Ok(manifest)
    }
}

/// `name` on PATH, so Windows never depends on PATHEXT probing.
fn which(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&file))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installs::exe;

    struct Fixture {
        _dir: tempfile::TempDir,
        base: PathBuf,
        hub: Hub,
        bundle: PathBuf,
    }

    fn binary(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "fake binary").unwrap();
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let hub = Hub::new(
            Some(base.join("Hub")),
            Some(base.join("data/projects.json")),
        );
        let bundle = base.join("bundle");
        for name in ["blockloom", "blockloom-runtime"] {
            binary(&bundle.join(exe(name)));
        }
        let rust = bundle.join("tools/rust");
        binary(&rust.join("bin").join(exe("rustc")));
        binary(&rust.join("bin").join(exe("cargo")));
        std::fs::create_dir_all(rust.join("lib/rustlib").join(host_target()).join("lib")).unwrap();
        for target in ANDROID_TARGETS {
            binary(
                &rust
                    .join("lib/rustlib")
                    .join(target)
                    .join("lib/libstd.rlib"),
            );
        }
        binary(&bundle.join("tools/java/bin").join(exe("java")));
        write_json(
            &bundle.join(MANIFEST),
            &json!({"kind": "release", "version": "0.1.0", "target": host_target(),
                    "tools": {"rust": {"channel": "1.98.1"}, "java": {"version": "25.0.1"},
                              "android-rust-targets": ANDROID_TARGETS}}),
        )
        .unwrap();
        Fixture {
            _dir: dir,
            base,
            hub,
            bundle,
        }
    }

    #[test]
    fn a_release_installs_once_with_only_the_chosen_tools() {
        let f = fixture();
        let installed = f.hub.install(&f.bundle, &[], false).unwrap();
        assert_eq!(installed["id"], "release-0.1.0");
        let slot = f.hub.slot("release-0.1.0").unwrap();
        assert!(!slot.join("tools/java").exists());
        assert!(
            !slot
                .join("tools/rust/lib/rustlib/aarch64-linux-android")
                .exists()
        );
        assert!(
            f.hub
                .install(&f.bundle, &[], false)
                .unwrap_err()
                .message
                .contains("already exists")
        );
        assert!(
            stage(&f.bundle, &slot, None, true, &[])
                .unwrap_err()
                .message
                .contains("cannot be replaced")
        );
        let listed = f.hub.installations().unwrap();
        assert_eq!(listed.len(), 1);
        // The toolchain went into the shared cache for later editors.
        assert!(tools::is_cached(
            &f.hub.tool_cache_dir(),
            &tools::rust_key("1.98.1")
        ));
    }

    #[test]
    fn chosen_tools_are_copied_and_uninstall_prunes_them() {
        let f = fixture();
        f.hub.install(&f.bundle, &["java".into()], true).unwrap();
        let slot = f.hub.slot("release-0.1.0").unwrap();
        assert!(slot.join("tools/java/bin").join(exe("java")).is_file());
        assert!(
            slot.join("tools/rust/lib/rustlib/aarch64-linux-android/lib")
                .is_dir()
        );
        let cache = f.hub.tool_cache_dir();
        assert!(tools::is_cached(&cache, &tools::java_key("25.0.1")));
        assert!(tools::is_cached(
            &cache,
            &tools::rust_target_key("1.98.1", "x86_64-linux-android")
        ));
        assert!(
            f.hub
                .install(&f.bundle, &["android-ndk".into()], false)
                .is_err()
        );
        let removed = f.hub.uninstall("release-0.1.0").unwrap();
        assert_eq!(removed["version"], "0.1.0");
        assert!(!slot.exists());
        assert!(f.hub.tool_cache()["entries"].as_array().unwrap().is_empty());
    }

    #[test]
    fn uninstall_refuses_an_editor_a_project_uses() {
        let f = fixture();
        f.hub.install(&f.bundle, &[], false).unwrap();
        let project = f.base.join("Game");
        std::fs::create_dir_all(project.join(".blockloom")).unwrap();
        write_json(&project.join("project.blockloom"), &json!({"name": "Game"})).unwrap();
        write_json(
            &project.join(".blockloom/hub.json"),
            &json!({"schema": 1, "installation": "release-0.1.0"}),
        )
        .unwrap();
        write_json(
            &f.hub.registry,
            &json!({"projects": [{"path": project.to_string_lossy()}]}),
        )
        .unwrap();
        let error = f.hub.uninstall("release-0.1.0").unwrap_err();
        assert!(error.message.contains("used by projects: Game"), "{error}");
    }

    #[test]
    fn a_failed_swap_keeps_the_previous_payload() {
        let f = fixture();
        let destination = f.base.join("promoted");
        stage(
            &f.bundle,
            &destination,
            Some(&json!({"kind": "dev", "version": "old"})),
            false,
            &[],
        )
        .unwrap();
        // A payload without its editor is refused before anything moves.
        let broken = f.base.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        assert!(promote(&broken, &destination, &json!({"kind": "dev"}), true).is_err());
        assert_eq!(
            read_json(&destination.join(MANIFEST)).unwrap()["version"],
            "old"
        );
        // A prepared payload moves rather than copies.
        let prepared = f.base.join("prepared");
        stage(&f.bundle, &prepared, None, false, &[]).unwrap();
        promote(
            &prepared,
            &destination,
            &json!({"kind": "dev", "version": "new"}),
            true,
        )
        .unwrap();
        assert!(!prepared.exists());
        assert_eq!(
            read_json(&destination.join(MANIFEST)).unwrap()["version"],
            "new"
        );
    }

    #[test]
    fn modules_name_the_pinned_versions() {
        let f = fixture();
        let modules = f.hub.modules(None).unwrap();
        let rows = modules["modules"].as_array().unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0]["label"],
            format!("Java {}", blockloom_core::android::JDK_MAJOR)
        );
        assert_eq!(modules["defaults"]["release"], json!([]));
    }

    #[test]
    fn release_settings_check_their_source() {
        let f = fixture();
        assert_eq!(
            f.hub.release_settings(None, None, None).unwrap()["source"],
            "github-cli"
        );
        assert!(f.hub.release_settings(Some("ftp"), None, None).is_err());
        assert!(
            f.hub
                .release_settings(Some("https"), Some("http://x"), None)
                .is_err()
        );
        let saved = f
            .hub
            .release_settings(Some("https"), Some("https://example.com/c.json"), None)
            .unwrap();
        assert_eq!(saved["url"], "https://example.com/c.json");
        assert_eq!(
            f.hub.release_settings(None, None, None).unwrap()["source"],
            "https"
        );
    }

    #[test]
    fn a_checkout_registers_as_a_dev_installation() {
        let f = fixture();
        let repo = f.base.join("checkout");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[workspace]\nmembers = [\"blockloom-qt\"]\n[workspace.package]\nversion = \"0.0.9\"\n",
        )
        .unwrap();
        assert!(f.hub.add_dev(&repo).is_err());
        std::fs::write(repo.join("justfile"), "").unwrap();
        let added = f.hub.add_dev(&repo).unwrap();
        assert_eq!(added["status"], "unbuilt");
        assert_eq!(added["version"], "0.0.9");
        assert_eq!(f.hub.add_dev(&repo).unwrap()["id"], added["id"]);
        assert!(
            f.hub
                .install_dev(added["id"].as_str().unwrap(), &[], false, None)
                .is_err()
        );
    }
}
