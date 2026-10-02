//! `plugin.json`: what a package says about itself.
//!
//! The manifest is a claim, not a fact. The host checks it against the
//! engine, the target and the payload actually on disk before a package is
//! trusted with anything.

use crate::id::{validate_package_path, validate_plugin_id};
use crate::versions;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The file a package is identified by, at its root.
pub const MANIFEST_FILE: &str = "plugin.json";

/// How a package's code, if any, runs. The host never loads a Rust
/// dynamic library and assumes its Bevy or Rust layouts are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Components, blocks, commands and assets only, run by host services.
    #[default]
    Declarative,
    /// A WebAssembly module in isolated memory with execution limits.
    Portable,
    /// A prebuilt library behind the versioned C ABI, per target.
    Native,
    /// Source-linked against one exact engine build. Not binary compatible.
    Adapter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    ProjectStorage,
    Network,
    ExternalFiles,
    Subprocess,
    /// Runs native code in the host's process. Cannot be sandboxed.
    NativeExecution,
    /// Ships QML or native editor modules with host-level access.
    TrustedEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DependencyScope {
    /// Needed by the editor and by a built game.
    #[default]
    Runtime,
    /// Needed only while authoring; a player build leaves it out.
    Editor,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dependency {
    pub version: VersionReq,
    pub optional: bool,
    pub scope: DependencyScope,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DependencyRepr {
    Short(VersionReq),
    Full {
        version: VersionReq,
        #[serde(default)]
        optional: bool,
        #[serde(default)]
        scope: DependencyScope,
    },
}

impl<'de> Deserialize<'de> for Dependency {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match DependencyRepr::deserialize(d)? {
            DependencyRepr::Short(version) => Dependency {
                version,
                optional: false,
                scope: DependencyScope::Runtime,
            },
            DependencyRepr::Full {
                version,
                optional,
                scope,
            } => Dependency {
                version,
                optional,
                scope,
            },
        })
    }
}

/// An optional feature: extra dependencies switched on by name. A feature
/// may not change what a saved record means.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
}

/// What a native target needs from the GPU, and what to do without it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuRequirements {
    /// wgpu feature names the artifact needs.
    #[serde(default)]
    pub features: Vec<String>,
    /// Minimum limits, by wgpu limit name.
    #[serde(default)]
    pub limits: BTreeMap<String, u64>,
    /// `"cpu"` or `"portable"`; absent means the GPU is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeEntry {
    /// The library, relative to the package root.
    pub library: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuRequirements>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableEntry {
    /// The `.wasm` module, relative to the package root.
    pub module: String,
    #[serde(default = "default_memory")]
    pub memory_limit_mib: u32,
    #[serde(default = "default_time")]
    pub call_limit_ms: u32,
}

fn default_memory() -> u32 {
    64
}

fn default_time() -> u32 {
    50
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterEntry {
    /// The exact engine build this adapter is compiled against.
    pub engine_build: String,
    /// The crate directory inside the package.
    pub crate_dir: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Runtime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable: Option<PortableEntry>,
    /// Native libraries keyed by target triple.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub native: BTreeMap<String, NativeEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<AdapterEntry>,
}

/// Editor-only files. A player build omits all of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Editor {
    /// QML or other editor modules, relative to the package root.
    #[serde(default)]
    pub modules: Vec<String>,
    /// Whether the world hosts the plugin's code while nothing plays, so its
    /// meshes show in the scene view. It starts with `preview: true`.
    #[serde(default)]
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The engine versions this package runs on.
    pub engine: VersionReq,
    /// The SDK range it was built with, for packages with code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk: Option<VersionReq>,
    /// The C ABI version native and portable modules were built to.
    #[serde(default)]
    pub abi: u32,
    #[serde(default)]
    pub editor_api: u32,
    #[serde(default)]
    pub shader_api: u32,
    #[serde(default)]
    pub tier: Tier,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
    #[serde(default)]
    pub features: BTreeMap<String, Feature>,
    /// Services this package supplies. Two packages providing the same one
    /// cannot be active together.
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub runtime: Runtime,
    #[serde(default)]
    pub editor: Editor,
    /// Schema files holding the package's [`crate::schema::Contributions`].
    #[serde(default)]
    pub contributions: Vec<String>,
    /// The sha256 of every file in the package except `plugin.json`.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

/// How a package can run on one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetSupport {
    /// Runs natively (or needs no code) here.
    Native,
    /// Runs through the portable module.
    Portable,
    /// Not available, and why.
    Unsupported { reason: String },
}

impl TargetSupport {
    pub fn is_supported(&self) -> bool {
        !matches!(self, TargetSupport::Unsupported { .. })
    }
}

pub const WEB_TARGET: &str = "wasm32-unknown-unknown";

impl PluginManifest {
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("{MANIFEST_FILE}: {e}"))
    }

    /// Everything checkable without the payload on disk.
    pub fn validate(&self) -> Result<(), String> {
        let fail = |m: String| Err(format!("{}: {m}", self.id));
        if self.format != versions::MANIFEST_FORMAT {
            return fail(format!(
                "manifest format {} is not supported (expected {})",
                self.format,
                versions::MANIFEST_FORMAT
            ));
        }
        validate_plugin_id(&self.id)?;
        if self.name.trim().is_empty() {
            return fail("a package needs a name".to_string());
        }
        if !self.version.build.is_empty() {
            // Build metadata does not order versions, so it cannot identify one.
            return fail("build metadata is not allowed in a version".to_string());
        }
        for (dep, _) in self.dependencies.iter().chain(
            self.features
                .values()
                .flat_map(|feature| feature.dependencies.iter()),
        ) {
            validate_plugin_id(dep)?;
            if dep == &self.id {
                return fail("depends on itself".to_string());
            }
        }
        let has_code = self.tier != Tier::Declarative;
        if has_code {
            if self.abi != versions::PLUGIN_ABI && self.tier != Tier::Adapter {
                return fail(format!(
                    "built for plugin ABI {}, this engine speaks {}",
                    self.abi,
                    versions::PLUGIN_ABI
                ));
            }
            if self.sdk.is_none() {
                return fail("a package with code must declare its sdk range".to_string());
            }
        }
        let tier_error = match self.tier {
            Tier::Declarative => (self.runtime != Runtime::default())
                .then_some("a declarative package has no runtime entries"),
            Tier::Portable => self
                .runtime
                .portable
                .is_none()
                .then_some("a portable package needs runtime.portable"),
            Tier::Native => (self.runtime.native.is_empty() && self.runtime.portable.is_none())
                .then_some("a native package needs at least one runtime.native target"),
            Tier::Adapter => self
                .runtime
                .adapter
                .is_none()
                .then_some("an adapter package needs runtime.adapter"),
        };
        if let Some(message) = tier_error {
            return fail(message.to_string());
        }
        if matches!(self.tier, Tier::Native | Tier::Adapter)
            && !self.capabilities.contains(&Capability::NativeExecution)
        {
            return fail("native code must declare the native-execution capability".to_string());
        }
        if !self.editor.modules.is_empty()
            && !self.capabilities.contains(&Capability::TrustedEditor)
        {
            return fail("editor modules must declare the trusted-editor capability".to_string());
        }
        if let Some(other) = self.editor.modules.iter().find(|m| !m.ends_with(".qml")) {
            return fail(format!("{other}: an editor module is a .qml file"));
        }
        for (triple, entry) in &self.runtime.native {
            if triple.is_empty() || triple.contains('/') {
                return fail(format!("\"{triple}\" is not a target triple"));
            }
            if triple == WEB_TARGET {
                return fail("native libraries cannot be loaded on the web target".to_string());
            }
            if let Some(gpu) = &entry.gpu
                && let Some(fallback) = &gpu.fallback
                && !matches!(fallback.as_str(), "cpu" | "portable")
            {
                return fail(format!("{triple}: gpu fallback must be cpu or portable"));
            }
        }
        for path in self.referenced_paths() {
            validate_package_path(&path)?;
            if path == MANIFEST_FILE {
                return fail("a file cannot list itself".to_string());
            }
        }
        for (path, hash) in &self.files {
            validate_package_path(path)?;
            if !is_sha256(hash) {
                return fail(format!("{path}: \"{hash}\" is not a sha256 hex digest"));
            }
        }
        for path in self.referenced_paths() {
            if !self.files.contains_key(&path) {
                return fail(format!("{path} is referenced but has no hash in files"));
            }
        }
        Ok(())
    }

    /// Every package path the manifest points at.
    pub fn referenced_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.contributions.clone();
        paths.extend(self.editor.modules.iter().cloned());
        if let Some(p) = &self.runtime.portable {
            paths.push(p.module.clone());
        }
        paths.extend(self.runtime.native.values().map(|n| n.library.clone()));
        if let Some(a) = &self.runtime.adapter {
            paths.push(format!("{}/Cargo.toml", a.crate_dir));
        }
        paths
    }

    /// Whether this package can run on `triple`, honouring a GPU-gated
    /// native artifact's declared fallback.
    pub fn support_for(&self, triple: &str) -> TargetSupport {
        match self.tier {
            Tier::Declarative => TargetSupport::Native,
            Tier::Portable => TargetSupport::Portable,
            Tier::Native => {
                if self.runtime.native.contains_key(triple) {
                    TargetSupport::Native
                } else if self.runtime.portable.is_some() {
                    TargetSupport::Portable
                } else {
                    TargetSupport::Unsupported {
                        reason: format!(
                            "no native artifact for {triple} (has {})",
                            self.runtime
                                .native
                                .keys()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    }
                }
            }
            Tier::Adapter => TargetSupport::Unsupported {
                reason: "an engine adapter is compiled into a custom player".to_string(),
            },
        }
    }

    /// Dependencies a built game needs: runtime scope, required.
    pub fn runtime_dependencies(&self) -> impl Iterator<Item = (&String, &Dependency)> {
        self.dependencies
            .iter()
            .filter(|(_, d)| d.scope == DependencyScope::Runtime)
    }
}

pub fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn base() -> serde_json::Value {
        json!({
            "format": 1,
            "id": "com.example.hello",
            "name": "Hello",
            "version": "1.2.3",
            "engine": ">=0.0.1",
        })
    }

    fn parse(value: serde_json::Value) -> PluginManifest {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_minimal_declarative_manifest_is_valid() {
        let m = parse(base());
        m.validate().unwrap();
        assert_eq!(
            m.support_for("x86_64-unknown-linux-gnu"),
            TargetSupport::Native
        );
        assert_eq!(m.support_for(WEB_TARGET), TargetSupport::Native);
    }

    #[test]
    fn dependencies_take_a_short_or_long_spelling() {
        let mut v = base();
        v["dependencies"] = json!({
            "com.example.a": "^1.2",
            "com.example.b": {"version": "~2", "optional": true, "scope": "editor"}
        });
        let m = parse(v);
        m.validate().unwrap();
        assert!(!m.dependencies["com.example.a"].optional);
        assert_eq!(
            m.dependencies["com.example.b"].scope,
            DependencyScope::Editor
        );
        assert_eq!(m.runtime_dependencies().count(), 1);
    }

    #[test]
    fn bad_ids_formats_and_self_dependencies_are_rejected() {
        let mut v = base();
        v["id"] = json!("hello");
        assert!(parse(v).validate().is_err());
        let mut v = base();
        v["format"] = json!(9);
        assert!(parse(v).validate().is_err());
        let mut v = base();
        v["dependencies"] = json!({"com.example.hello": "1"});
        assert!(parse(v).validate().is_err());
    }

    #[test]
    fn native_code_needs_abi_sdk_capability_and_matching_files() {
        let mut v = base();
        v["tier"] = json!("native");
        v["runtime"] =
            json!({"native": {"x86_64-unknown-linux-gnu": {"library": "runtime/x/lib.so"}}});
        assert!(parse(v.clone()).validate().is_err(), "abi");
        v["abi"] = json!(1);
        assert!(parse(v.clone()).validate().is_err(), "sdk");
        v["sdk"] = json!("^0.1");
        assert!(parse(v.clone()).validate().is_err(), "capability");
        v["capabilities"] = json!(["native-execution"]);
        assert!(parse(v.clone()).validate().is_err(), "hash");
        v["files"] = json!({"runtime/x/lib.so": HASH});
        let m = parse(v);
        m.validate().unwrap();
        assert!(m.support_for("x86_64-unknown-linux-gnu").is_supported());
        let other = m.support_for("aarch64-apple-darwin");
        assert!(!other.is_supported());
        assert!(!m.support_for(WEB_TARGET).is_supported());
    }

    #[test]
    fn an_abi_mismatch_names_both_versions() {
        let mut v = base();
        v["tier"] = json!("portable");
        v["abi"] = json!(7);
        v["sdk"] = json!("*");
        v["runtime"] = json!({"portable": {"module": "portable/m.wasm"}});
        v["files"] = json!({"portable/m.wasm": HASH});
        let error = parse(v).validate().unwrap_err();
        assert!(error.contains("ABI 7") && error.contains('1'), "{error}");
    }

    #[test]
    fn a_native_package_falls_back_to_its_portable_module_elsewhere() {
        let mut v = base();
        v["tier"] = json!("native");
        v["abi"] = json!(1);
        v["sdk"] = json!("*");
        v["capabilities"] = json!(["native-execution"]);
        v["runtime"] = json!({
            "native": {"x86_64-unknown-linux-gnu": {"library": "a.so"}},
            "portable": {"module": "p.wasm"}
        });
        v["files"] = json!({"a.so": HASH, "p.wasm": HASH});
        let m = parse(v);
        m.validate().unwrap();
        assert_eq!(m.support_for(WEB_TARGET), TargetSupport::Portable);
    }

    #[test]
    fn paths_must_stay_inside_and_be_hashed() {
        let mut v = base();
        v["contributions"] = json!(["../outside.json"]);
        v["files"] = json!({"../outside.json": HASH});
        assert!(parse(v).validate().is_err());
        let mut v = base();
        v["contributions"] = json!(["schemas/a.json"]);
        assert!(parse(v).validate().is_err(), "unhashed reference");
    }

    #[test]
    fn editor_modules_are_trusted_code() {
        let mut v = base();
        v["editor"] = json!({"modules": ["editor/Panel.qml"]});
        v["files"] = json!({"editor/Panel.qml": HASH});
        assert!(parse(v.clone()).validate().is_err());
        v["capabilities"] = json!(["trusted-editor"]);
        parse(v.clone()).validate().unwrap();
        // Only QML is loaded into the editor.
        v["editor"] = json!({"modules": ["editor/panel.so"]});
        v["files"] = json!({"editor/panel.so": HASH});
        assert!(parse(v).validate().is_err());
    }
}
