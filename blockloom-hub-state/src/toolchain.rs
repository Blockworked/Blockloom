//! Downloading the tools an installation can carry: Android Rust targets
//! from the Rust distribution, Java from Adoptium and the Android SDK and
//! NDK from Google's repository, all checked against what they publish.

use crate::fetch::{self, Checksum, read_remote};
use crate::run::copy_tree;
use crate::tools::{self, is_channel};
use crate::{Error, Result, checkpoint};
use quick_xml::events::Event;
use serde_json::{Map, Value, json};
use sha2::Digest;
use std::collections::HashMap;
use std::path::Path;

const GIB: u64 = 1024 * 1024 * 1024;

/// Adds the Android standard libraries to a bundled toolchain, from the
/// cache or from the exact Rust release.
pub fn install_rust_targets(
    rust: &Path,
    channel: &str,
    targets: &[&str],
    temporary: &Path,
    cache: Option<&Path>,
) -> Result<()> {
    let rustlib = rust.join("lib/rustlib");
    let mut missing: Vec<&str> = targets
        .iter()
        .copied()
        .filter(|t| !rustlib.join(t).join("lib").is_dir())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if !is_channel(channel) {
        return Err(Error::new("Android targets require a pinned Rust version"));
    }
    if let Some(cache) = cache {
        let mut still = vec![];
        for target in missing {
            let cached = cache.join(tools::rust_target_key(channel, target));
            if cached.join("lib").is_dir() {
                eprintln!("Reusing cached Rust target {target} for Rust {channel}...");
                checkpoint()?;
                copy_tree(&cached, &rustlib.join(target), &|_| false)?;
            } else {
                still.push(target);
            }
        }
        missing = still;
        if missing.is_empty() {
            return Ok(());
        }
    }
    let url = format!("https://static.rust-lang.org/dist/channel-rust-{channel}.toml");
    eprintln!("Checking Android targets for Rust {channel}...");
    let manifest = read_remote(&url, 2 * 1024 * 1024)?;
    let checksum = read_remote(&format!("{url}.sha256"), 1024)?;
    let checksum = String::from_utf8_lossy(&checksum);
    let declared = fetch::digest(checksum.split_whitespace().next().unwrap_or(""))?;
    let actual: String = sha2::Sha256::digest(&manifest)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != declared {
        return Err(Error::new("Rust distribution manifest checksum mismatch"));
    }
    let data: toml::Table = toml::from_str(&String::from_utf8_lossy(&manifest))
        .map_err(|e| Error::new(format!("Rust distribution manifest is invalid: {e}")))?;
    let std = data
        .get("pkg")
        .and_then(|p| p.get("rust-std"))
        .cloned()
        .unwrap_or(toml::Value::Table(Default::default()));
    let published = std
        .get("version")
        .and_then(|v| v.as_str())
        .and_then(|v| v.split(' ').next())
        .unwrap_or("");
    if data.get("manifest-version").and_then(|v| v.as_str()) != Some("2") || published != channel {
        return Err(Error::new(
            "Rust distribution manifest does not match the pinned version",
        ));
    }
    for target in missing {
        let descriptor = std.get("target").and_then(|t| t.get(target));
        let field = |name: &str| {
            descriptor
                .and_then(|d| d.get(name))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        if !descriptor
            .and_then(|d| d.get("available"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(Error::new(format!(
                "Rust {channel} does not publish {target}"
            )));
        }
        let artifact = field("xz_url");
        if fetch::host_of(&artifact) != "static.rust-lang.org" {
            return Err(Error::new(
                "Rust component must come from the official distribution host",
            ));
        }
        eprintln!("Installing Rust target {target}...");
        let archive = temporary.join(format!("{target}.tar.xz"));
        fetch::download(
            &artifact,
            &archive,
            &field("xz_hash"),
            None,
            256 * 1024 * 1024,
            Checksum::Sha256,
        )?;
        let output = temporary.join(target);
        fetch::extract(&archive, &output, "tar.xz", fetch::Extract::bundle(GIB))?;
        let mut matches = vec![];
        for top in std::fs::read_dir(&output)?.flatten() {
            let candidate = top
                .path()
                .join(format!("rust-std-{target}"))
                .join("lib/rustlib")
                .join(target);
            if candidate.join("lib").is_dir() {
                matches.push(candidate);
            }
        }
        let has_files =
            |p: &Path| std::fs::read_dir(p.join("lib")).is_ok_and(|mut r| r.next().is_some());
        if matches.len() != 1 || !has_files(&matches[0]) {
            return Err(Error::new(
                "Rust component archive has an unexpected layout",
            ));
        }
        copy_tree(&matches[0], &rustlib.join(target), &|_| false)?;
        if let Some(cache) = cache {
            tools::store_quietly(
                cache,
                &tools::rust_target_key(channel, target),
                &rustlib.join(target),
            )?;
        }
    }
    Ok(())
}

/// The Android pins a development checkout builds with.
pub fn android_versions(repo: &Path) -> Result<Value> {
    let source = std::fs::read_to_string(repo.join("blockloom-core/src/android.rs"))?;
    let mut result = Map::new();
    for name in [
        "JDK_MAJOR",
        "PLATFORM",
        "BUILD_TOOLS",
        "NDK_MAJOR",
        "EMULATOR_IMAGE",
    ] {
        let pattern = regex::Regex::new(&format!(
            r#"pub const {name}: [^=]+ = (?:"([^"]+)"|([0-9]+));"#
        ))
        .expect("pattern");
        let value = pattern
            .captures(&source)
            .and_then(|c| c.get(1).or(c.get(2)))
            .ok_or_else(|| {
                Error::new(format!(
                    "Development repository is missing Android pin: {name}"
                ))
            })?;
        result.insert(name.into(), json!(value.as_str()));
    }
    Ok(Value::Object(result))
}

/// The pins for catalogs from before releases carried their own. Mirrors
/// `blockloom-core/src/android.rs`.
pub fn default_android_versions() -> Value {
    use blockloom_core::android as pins;
    json!({"JDK_MAJOR": pins::JDK_MAJOR.to_string(), "PLATFORM": pins::PLATFORM,
           "BUILD_TOOLS": pins::BUILD_TOOLS, "NDK_MAJOR": pins::NDK_MAJOR.to_string(),
           "EMULATOR_IMAGE": pins::EMULATOR_IMAGE})
}

fn pin<'a>(versions: &'a Value, name: &str) -> Result<&'a str> {
    versions[name]
        .as_str()
        .ok_or_else(|| Error::new(format!("Missing Android pin: {name}")))
}

/// One element of a repository manifest, names as written.
#[derive(Clone, Debug, Default)]
struct Node {
    name: String,
    local: String,
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k.rsplit(':').next() == Some(name))
            .map(|(_, v)| v.as_str())
    }
    fn child(&self, local: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.local == local)
    }
    fn path(&self, path: &str) -> Option<&Node> {
        path.split('/')
            .try_fold(self, |node, part| node.child(part))
    }
    fn text_at(&self, path: &str) -> Option<&str> {
        self.path(path).map(|n| n.text.trim())
    }
    fn write(&self, out: &mut String) {
        use quick_xml::escape::escape;
        out.push('<');
        out.push_str(&self.name);
        for (key, value) in &self.attributes {
            out.push_str(&format!(" {key}=\"{}\"", escape(value.as_str())));
        }
        if self.children.is_empty() && self.text.trim().is_empty() {
            out.push_str(" />");
            return;
        }
        out.push('>');
        out.push_str(&escape(self.text.trim()));
        for child in &self.children {
            child.write(out);
        }
        out.push_str(&format!("</{}>", self.name));
    }
}

fn parse_xml(raw: &[u8]) -> Result<Node> {
    let text =
        std::str::from_utf8(raw).map_err(|_| Error::new("Android repository is not UTF-8"))?;
    let mut reader = quick_xml::Reader::from_str(text);
    let bad =
        |e: &dyn std::fmt::Display| Error::new(format!("Android repository is not valid XML: {e}"));
    let mut stack: Vec<Node> = vec![Node::default()];
    loop {
        let event = reader.read_event().map_err(|e| bad(&e))?;
        let element = |e: &quick_xml::events::BytesStart| -> Result<Node> {
            let mut attributes = vec![];
            for attribute in e.attributes() {
                let attribute = attribute.map_err(|e| bad(&e))?;
                let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
                let value = attribute
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|e| bad(&e))?
                    .into_owned();
                attributes.push((key, value));
            }
            Ok(Node {
                name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                local: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                attributes,
                ..Default::default()
            })
        };
        match event {
            Event::Start(e) => stack.push(element(&e)?),
            Event::Empty(e) => {
                let node = element(&e)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(|| bad(&"unbalanced"))?;
                stack
                    .last_mut()
                    .ok_or_else(|| bad(&"unbalanced"))?
                    .children
                    .push(node);
            }
            Event::Text(t) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&t.decode().map_err(|e| bad(&e))?);
                }
            }
            Event::CData(t) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&t.decode().map_err(|e| bad(&e))?);
                }
            }
            Event::GeneralRef(r) => {
                let name = r.decode().map_err(|e| bad(&e))?;
                let resolved = quick_xml::escape::unescape(&format!("&{name};"))
                    .map_err(|e| bad(&e))?
                    .into_owned();
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&resolved);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut top = stack.pop().ok_or_else(|| bad(&"empty"))?;
    if !stack.is_empty() || top.children.len() != 1 {
        return Err(bad(&"one root element"));
    }
    Ok(top.children.remove(0))
}

/// One of Google's repository manifests, with the stable channel's packages.
struct Repository {
    base: String,
    root: Node,
    packages: HashMap<String, Node>,
}

impl Repository {
    fn fetch(base: &str, manifest: &str) -> Result<Self> {
        let raw = read_remote(&format!("{base}{manifest}"), 16 * 1024 * 1024)?;
        let root = parse_xml(&raw)?;
        let packages = root
            .children
            .iter()
            .filter(|n| n.local == "remotePackage")
            .filter(|n| {
                n.child("channelRef")
                    .is_none_or(|c| c.attribute("ref") == Some("channel-0"))
            })
            .filter_map(|n| Some((n.attribute("path")?.to_string(), n.clone())))
            .collect();
        Ok(Self {
            base: base.to_string(),
            root,
            packages,
        })
    }

    fn package(&self, key: &str) -> Result<&Node> {
        self.packages
            .get(key)
            .ok_or_else(|| Error::new(format!("Android repository does not publish {key}")))
    }

    /// The package's one archive for this host.
    fn archive(&self, key: &str) -> Result<&Node> {
        let (host, arch) = host();
        let archives: Vec<&Node> = self
            .package(key)?
            .child("archives")
            .map(|a| a.children.iter().filter(|c| c.local == "archive").collect())
            .unwrap_or_default();
        let fits: Vec<&Node> = archives
            .into_iter()
            .filter(|a| {
                a.text_at("host-os").unwrap_or(host) == host
                    && a.text_at("host-arch").unwrap_or(arch) == arch
            })
            .collect();
        if fits.len() != 1 {
            return Err(Error::new(format!(
                "Android package {key} does not support this host"
            )));
        }
        Ok(fits[0])
    }

    fn revision(&self, key: &str) -> Result<String> {
        self.archive(key)?;
        Ok(revision_text(self.package(key)?))
    }

    /// Downloads one package into `destination` with the `package.xml` the
    /// SDK manager reads to recognise it, answering its revision.
    fn install(
        &self,
        key: &str,
        destination: &Path,
        label: &str,
        temporary: &Path,
    ) -> Result<String> {
        let complete = self
            .archive(key)?
            .child("complete")
            .ok_or_else(|| Error::new("Android package has no archive"))?;
        let relative = complete.text_at("url").unwrap_or("");
        if relative.is_empty() || relative.contains(['/', '\\', ':']) {
            return Err(Error::new(
                "Android package must come from the official repository",
            ));
        }
        let checksum = complete
            .child("checksum")
            .ok_or_else(|| Error::new("Android package has no checksum"))?;
        let kind = match checksum.attribute("type").unwrap_or("sha1") {
            "sha-256" | "sha256" => Checksum::Sha256,
            _ => Checksum::Sha1,
        };
        let size: u64 = complete
            .text_at("size")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::new("Android package has no size"))?;
        eprintln!("Downloading {key}...");
        let root = fetch::tool_archive(
            &format!("{}{relative}", self.base),
            checksum.text.trim(),
            size,
            temporary,
            label,
            kind,
        )?;
        copy_tree(&root, destination, &|_| false)?;
        let node = self.package(key)?;
        let mut local = Node {
            name: "localPackage".into(),
            local: "localPackage".into(),
            attributes: vec![("path".into(), key.into())],
            ..Default::default()
        };
        local.children = node
            .children
            .iter()
            .filter(|c| c.local != "archives" && c.local != "channelRef")
            .cloned()
            .collect();
        let mut repository = Node {
            name: self.root.name.clone(),
            local: self.root.local.clone(),
            attributes: self
                .root
                .attributes
                .iter()
                .filter(|(k, _)| k.starts_with("xmlns"))
                .cloned()
                .collect(),
            ..Default::default()
        };
        if let Some(license) = node.child("uses-license").and_then(|l| l.attribute("ref")) {
            repository.children.extend(
                self.root
                    .children
                    .iter()
                    .filter(|c| c.local == "license" && c.attribute("id") == Some(license))
                    .cloned(),
            );
        }
        repository.children.push(local);
        let mut xml = String::from("<?xml version='1.0' encoding='utf-8'?>\n");
        repository.write(&mut xml);
        std::fs::write(destination.join("package.xml"), xml)?;
        Ok(revision_text(node))
    }
}

fn revision_parts(node: &Node) -> [u64; 3] {
    ["major", "minor", "micro"].map(|f| {
        node.text_at(&format!("revision/{f}"))
            .and_then(|t| t.parse().ok())
            .unwrap_or(0)
    })
}

fn revision_text(node: &Node) -> String {
    revision_parts(node).map(|n| n.to_string()).join(".")
}

fn host() -> (&'static str, &'static str) {
    let host = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macosx"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x64"
    };
    (host, arch)
}

const GOOGLE: &str = "https://dl.google.com/android/repository/";

/// Downloads the chosen Java, Android SDK and NDK into `directory/tools`,
/// using the cache where it already has the same version. Answers the
/// manifest's `tools` entries for them.
pub fn install_android_tools(
    directory: &Path,
    versions: &Value,
    components: &[String],
    temporary: &Path,
    cache: Option<&Path>,
) -> Result<Map<String, Value>> {
    let mut selected = Map::new();
    let wants = |name: &str| components.iter().any(|c| c == name);
    let (host, arch) = host();
    if wants("java") {
        let major = pin(versions, "JDK_MAJOR")?;
        let os = if host == "macosx" { "mac" } else { host };
        let url = format!(
            "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type=jdk&os={os}&vendor=eclipse"
        );
        eprintln!("Downloading Java {major}...");
        let assets: Value = serde_json::from_slice(&read_remote(&url, 2 * 1024 * 1024)?)?;
        let asset = match assets.as_array() {
            Some(list) if list.len() == 1 => &list[0],
            _ => return Err(Error::new("Adoptium did not publish one matching JDK")),
        };
        let version = asset["version"]["semver"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::new("Adoptium package metadata is missing the Java version"))?;
        let package = &asset["binary"]["package"];
        let link = package["link"].as_str().unwrap_or("");
        let from_adoptium = url::Url::parse(link).is_ok_and(|u| {
            u.host_str() == Some("github.com") && u.path().starts_with("/adoptium/")
        });
        if !from_adoptium {
            return Err(Error::new("Java package must come from Adoptium"));
        }
        let destination = directory.join("tools/java");
        let key = tools::java_key(version);
        let reused = match cache {
            Some(cache) => tools::reuse_or_drop(cache, &key, &destination)?,
            None => false,
        };
        if !reused {
            let mut root = fetch::tool_archive(
                link,
                package["checksum"].as_str().unwrap_or(""),
                package["size"].as_u64().unwrap_or(0),
                temporary,
                "java-download",
                Checksum::Sha256,
            )?;
            if root.join("Contents/Home").is_dir() {
                root = root.join("Contents/Home");
            }
            copy_tree(&root, &destination, &|_| false)?;
            if let Some(cache) = cache {
                tools::store_quietly(cache, &key, &destination)?;
            }
        }
        selected.insert("java".into(), json!({"version": version}));
    }
    if !wants("android-sdk") && !wants("android-ndk") {
        return Ok(selected);
    }
    eprintln!("Checking official Android packages...");
    let main = Repository::fetch(GOOGLE, "repository2-3.xml")?;
    if wants("android-sdk") {
        let sdk = directory.join("tools/android-sdk");
        let platform = pin(versions, "PLATFORM")?;
        let build_tools = pin(versions, "BUILD_TOOLS")?;
        let default_image = format!("system-images;{platform};google_apis;x86_64");
        let image = versions["EMULATOR_IMAGE"]
            .as_str()
            .unwrap_or(&default_image)
            .to_string();
        let parts: Vec<&str> = image.split(';').collect();
        let part_ok = |p: &&str| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        };
        if parts.len() != 4 || parts[0] != "system-images" || !parts.iter().all(part_ok) {
            return Err(Error::new("Repository has an invalid emulator image pin"));
        }
        let packages = [
            (
                "cmdline-tools;latest".to_string(),
                "cmdline-tools/latest".to_string(),
                "command-tools",
            ),
            (
                "platform-tools".into(),
                "platform-tools".into(),
                "platform-tools",
            ),
            (
                format!("platforms;{platform}"),
                format!("platforms/{platform}"),
                "platform",
            ),
            (
                format!("build-tools;{build_tools}"),
                format!("build-tools/{build_tools}"),
                "build-tools",
            ),
            ("emulator".into(), "emulator".into(), "emulator"),
        ];
        let images =
            Repository::fetch(&format!("{GOOGLE}sys-img/{}/", parts[2]), "sys-img2-3.xml")?;
        let mut reused = None;
        if let Some(cache) = cache {
            let mut preview = Map::new();
            for (key, _, _) in &packages {
                preview.insert(key.clone(), json!(main.revision(key)?));
            }
            preview.insert(image.clone(), json!(images.revision(&image)?));
            let version = preview["platform-tools"].as_str().unwrap_or("").to_string();
            let preview = Value::Object(preview);
            if tools::reuse_or_drop(cache, &tools::sdk_key(&preview, &version), &sdk)? {
                reused = Some((version, preview));
            }
        }
        let (version, installed) = match reused {
            Some(found) => found,
            None => {
                let mut installed = Map::new();
                for (key, relative, label) in &packages {
                    installed.insert(
                        key.clone(),
                        json!(main.install(key, &sdk.join(relative), label, temporary)?),
                    );
                }
                let image_dir = parts.iter().fold(sdk.clone(), |p, part| p.join(part));
                installed.insert(
                    image.clone(),
                    json!(images.install(&image, &image_dir, "system-image", temporary)?),
                );
                let version = installed["platform-tools"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let installed = Value::Object(installed);
                if let Some(cache) = cache {
                    tools::store_quietly(cache, &tools::sdk_key(&installed, &version), &sdk)?;
                }
                (version, installed)
            }
        };
        selected.insert(
            "android-sdk".into(),
            json!({"version": version, "packages": installed}),
        );
    }
    if wants("android-ndk") {
        let prefix = format!("ndk;{}.", pin(versions, "NDK_MAJOR")?);
        let key = main
            .packages
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .max_by_key(|(_, node)| revision_parts(node))
            .map(|(k, _)| k.clone())
            .ok_or_else(|| {
                Error::new("Android repository does not publish the pinned NDK major")
            })?;
        let destination = directory.join("tools/android-ndk");
        let preview = revision_text(main.package(&key)?);
        let ndk_key = tools::ndk_key(&preview);
        let reused = match cache {
            Some(cache) => tools::reuse_or_drop(cache, &ndk_key, &destination)?,
            None => false,
        };
        let version = if reused {
            preview
        } else {
            let version = main.install(&key, &destination, "ndk", temporary)?;
            if let Some(cache) = cache {
                tools::store_quietly(cache, &ndk_key, &destination)?;
            }
            version
        };
        selected.insert("android-ndk".into(), json!({"version": version}));
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pins_match_the_editor() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert_eq!(android_versions(repo).unwrap(), default_android_versions());
    }

    #[test]
    fn a_repository_manifest_keeps_its_stable_packages_and_namespaces() {
        let xml = br#"<?xml version="1.0"?>
<sdk:sdk-repository xmlns:sdk="http://schemas.android.com/sdk/android/repo/repository2/03" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <license id="android-sdk-license" type="text">Terms &amp; conditions</license>
  <channel id="channel-0">stable</channel>
  <remotePackage path="platform-tools">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>36</major><minor>0</minor><micro>1</micro></revision>
    <display-name>Platform-Tools</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>3</size><checksum type="sha1">a9993e364706816aba3e25717850c26c9cd0d89d</checksum><url>tools.zip</url></complete><host-os>linux</host-os></archive></archives>
  </remotePackage>
  <remotePackage path="emulator"><channelRef ref="channel-3"/></remotePackage>
</sdk:sdk-repository>"#;
        let root = parse_xml(xml).unwrap();
        assert_eq!(root.name, "sdk:sdk-repository");
        assert_eq!(root.children[0].text, "Terms & conditions");
        let package = root
            .children
            .iter()
            .find(|n| n.attribute("path") == Some("platform-tools"))
            .unwrap();
        assert_eq!(revision_text(package), "36.0.1");
        let mut out = String::new();
        root.write(&mut out);
        assert!(out.contains("xmlns:xsi="));
        assert!(out.contains("Terms &amp; conditions"));
        assert!(parse_xml(out.as_bytes()).is_ok());
    }

    #[test]
    fn rust_targets_come_from_the_exact_release_and_are_cached() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let target = "aarch64-linux-android";
        let mut tarball = tar::Builder::new(vec![]);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        let name = format!(
            "rust-std-1.98.1-{target}/rust-std-{target}/lib/rustlib/{target}/lib/libstd.rlib"
        );
        tarball.append_data(&mut header, name, &b"std"[..]).unwrap();
        let tarball = tarball.into_inner().unwrap();
        let mut xz = vec![];
        lzma_rs::xz_compress(&mut std::io::Cursor::new(tarball), &mut xz).unwrap();
        let hex = |d: &[u8]| -> String {
            sha2::Sha256::digest(d)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let artifact = format!("https://static.rust-lang.org/dist/rust-std-{target}.tar.xz");
        let manifest = format!(
            "manifest-version = \"2\"\n[pkg.rust-std]\nversion = \"1.98.1 (abc 2026-01-01)\"\n\
             [pkg.rust-std.target.{target}]\navailable = true\nxz_url = \"{artifact}\"\nxz_hash = \"{}\"\n",
            hex(&xz)
        );
        let sum = format!("{}  channel-rust-1.98.1.toml\n", hex(manifest.as_bytes()));
        let served = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = served.clone();
        crate::fetch::fake_network(move |url, _| {
            count.set(count.get() + 1);
            let body = if url.ends_with(".toml") {
                manifest.clone().into_bytes()
            } else if url.ends_with(".sha256") {
                sum.clone().into_bytes()
            } else {
                xz.clone()
            };
            Ok(crate::fetch::Remote {
                status: 200,
                content_range: None,
                body: Box::new(std::io::Cursor::new(body)),
            })
        });
        let rust = dir.path().join("rust");
        let cache = dir.path().join("cache");
        install_rust_targets(&rust, "1.98.1", &[target], dir.path(), Some(&cache)).unwrap();
        assert!(
            rust.join("lib/rustlib")
                .join(target)
                .join("lib/libstd.rlib")
                .is_file()
        );
        assert_eq!(served.get(), 3);
        // A second toolchain takes it from the cache without asking again.
        let other = dir.path().join("other");
        install_rust_targets(&other, "1.98.1", &[target], dir.path(), Some(&cache)).unwrap();
        assert!(
            other
                .join("lib/rustlib")
                .join(target)
                .join("lib/libstd.rlib")
                .is_file()
        );
        assert_eq!(served.get(), 3);
        let _ = std::io::stderr().flush();
        assert!(
            install_rust_targets(
                &dir.path().join("x"),
                "nightly",
                &[target],
                dir.path(),
                None
            )
            .is_err()
        );
    }
}
