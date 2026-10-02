//! `plugin-new`: a package folder an author can build on.
//!
//! A declarative package is sealed and ready to install. A portable one is a
//! Cargo crate beside its manifest and schema; it is sealed once its wasm is
//! built (`build.sh`).

use blockloom_plugin_api::id::{validate_plugin_id, validate_type_id};
use blockloom_plugin_api::manifest::{MANIFEST_FILE, PluginManifest};
use blockloom_plugin_api::schema::Contributions;
use blockloom_plugin_api::versions;
use blockloom_plugin_host::package;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

/// The portable template's source, which the SDK's own tests compile.
const STARTER_SOURCE: &str = include_str!("../../blockloom-plugin-sdk/templates/lib.rs");
const SDK_GIT: &str = "https://github.com/Blockworked/Blockloom";

/// What `plugin-new` made.
pub struct Scaffold {
    pub files: Vec<String>,
    pub sealed: bool,
}

fn write(root: &Path, files: &mut Vec<String>, name: &str, text: &str) -> Result<(), String> {
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    files.push(name.to_string());
    Ok(())
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("json prints");
    text.push('\n');
    text
}

/// The last id segment as a file and crate name.
fn slug_of(id: &str) -> String {
    id.rsplit('.').next().unwrap_or(id).replace('-', "_")
}

/// `Hit Points` becomes `HitPoints`, or `Thing` when nothing usable is left.
fn type_id_of(name: &str) -> String {
    let words: String = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            let first = chars.next().expect("not empty").to_ascii_uppercase();
            format!("{first}{}", chars.as_str())
        })
        .collect();
    if validate_type_id(&words).is_ok() {
        words
    } else {
        "Thing".to_string()
    }
}

fn declarative_schema(name: &str) -> Value {
    let type_id = type_id_of(name);
    json!({
        "components": [{
            "type_id": type_id,
            "display_name": name,
            "version": 1,
            "fields": [
                {"name": "amount", "type": "int", "min": 0, "max": 1000, "default": 10}
            ]
        }],
        "commands": [
            {
                "name": "give",
                "summary": format!("Give an actor a {name} component."),
                "args": [
                    {"name": "actor", "type": "actor"},
                    {"name": "amount", "type": "int", "min": 0, "max": 1000, "default": 10}
                ],
                "action": {"do": "add_component", "component": type_id}
            },
            {
                "name": "set_amount",
                "summary": format!("Set the amount on an actor's {name} component."),
                "args": [
                    {"name": "actor", "type": "actor"},
                    {"name": "value", "type": "int", "min": 0, "max": 1000}
                ],
                "action": {"do": "set_field", "component": type_id, "field": "amount"}
            }
        ],
        "blocks": [{
            "type_id": "set_amount",
            "kind": "statement",
            "category": name,
            "label": "set {actor} amount to {value}",
            "slots": [
                {"name": "actor", "type": "actor"},
                {"name": "value", "type": "int", "min": 0, "max": 1000, "default": 10}
            ],
            "command": "set_amount"
        }]
    })
}

fn portable_schema(name: &str) -> Value {
    json!({
        "commands": [
            {
                "name": "greet",
                "summary": "Greet someone and count it.",
                "args": [{"name": "name", "type": "text", "default": "world"}],
                "action": {"do": "module", "op": "greet"}
            },
            {
                "name": "count",
                "summary": "Answer how many greetings there have been.",
                "action": {"do": "module", "op": "count"}
            }
        ],
        "blocks": [
            {
                "type_id": "greet",
                "kind": "statement",
                "category": name,
                "label": "greet {name}",
                "slots": [{"name": "name", "type": "text", "default": "world"}],
                "command": "greet"
            },
            {
                "type_id": "greeted",
                "kind": "reporter",
                "category": name,
                "label": "times greeted",
                "slots": [],
                "returns": {"type": "int"},
                "command": "count"
            }
        ]
    })
}

fn manifest(id: &str, name: &str, template: &str, slug: &str) -> Value {
    let mut manifest = json!({
        "format": versions::MANIFEST_FORMAT,
        "id": id,
        "name": name,
        "version": "0.1.0",
        "description": format!("{name}, a Blockloom plugin."),
        "license": "MIT",
        "engine": ">=0.0.1",
        "tier": template,
        "contributions": [format!("schemas/{slug}.json")],
    });
    if template == "portable" {
        manifest["abi"] = json!(versions::PLUGIN_ABI);
        manifest["sdk"] = json!("^0.0");
        manifest["runtime"] = json!({
            "portable": {
                "module": format!("portable/{slug}.wasm"),
                "memory_limit_mib": 16,
                "call_limit_ms": 20
            }
        });
    }
    manifest
}

fn cargo_toml(slug: &str, sdk: Option<&str>) -> String {
    // A path that exists on disk is a checkout; anything else is a git URL.
    let source = match sdk {
        Some(path) if Path::new(path).is_dir() => format!("path = {path:?}"),
        Some(url) => format!("git = {url:?}"),
        None => format!("git = {SDK_GIT:?}"),
    };
    format!(
        r#"[package]
name = "{slug}"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
blockloom-plugin-sdk = {{ {source} }}

[dev-dependencies]
blockloom-plugin-sdk = {{ {source}, features = ["testing"] }}

[profile.release]
opt-level = "s"
lto = true
"#
    )
}

fn build_script(slug: &str) -> String {
    format!(
        r#"#!/bin/sh
# Builds the portable module into the package and seals it. Needs
# `rustup target add wasm32-unknown-unknown` and blockloom-shell on PATH
# (or set BLOCKLOOM_SHELL).
set -eu
cargo build --release --target wasm32-unknown-unknown
mkdir -p portable
cp target/wasm32-unknown-unknown/release/{slug}.wasm portable/{slug}.wasm
"${{BLOCKLOOM_SHELL:-blockloom-shell}}" --no-state --eval "plugin-seal path=$(pwd)"
"#
    )
}

fn readme(name: &str, id: &str, template: &str, slug: &str) -> String {
    if template == "declarative" {
        format!(
            "# {name}\n\nA declarative plugin (`{id}`): schemas only, so it runs everywhere.\n\n\
             Edit `schemas/{slug}.json`, then reseal:\n\n    blockloom-shell --no-state --eval 'plugin-seal path=.'\n\n\
             Install it in a project with `plugin-install id={id} source=path:<this folder>`.\n"
        )
    } else {
        format!(
            "# {name}\n\nA portable plugin (`{id}`): Rust in `src/lib.rs`, schemas in `schemas/{slug}.json`.\n\n\
             - `cargo test` runs it through the SDK harness (`blockloom-plugin-sdk` feature `testing`).\n\
             - `./build.sh` builds the wasm module and seals the package.\n\
             - `plugin-install id={id} source=path:<this folder>` adds it to a project.\n\n\
             The same crate also builds as a native library (`cargo build --release`); see the plugin guide for native packages.\n"
        )
    }
}

fn workflow() -> &'static str {
    r#"name: Plugin
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: rustup target add wasm32-unknown-unknown
      - run: cargo test
      - run: cargo build --release --target wasm32-unknown-unknown
"#
}

/// Writes a new package folder at `path`. `template` is `declarative` or
/// `portable`; `sdk` is a checkout path or git URL for the portable crate's SDK.
pub fn plugin_new(
    path: &Path,
    id: &str,
    name: &str,
    template: &str,
    sdk: Option<&str>,
) -> Result<Scaffold, String> {
    validate_plugin_id(id)?;
    if name.trim().is_empty() {
        return Err("a plugin needs a name".to_string());
    }
    if !matches!(template, "declarative" | "portable") {
        return Err(format!(
            "template must be declarative or portable, not \"{template}\""
        ));
    }
    if path.exists()
        && fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err(format!("{} is not empty", path.display()));
    }
    let slug = slug_of(id);
    let mut files = Vec::new();
    let schema = if template == "declarative" {
        declarative_schema(name)
    } else {
        portable_schema(name)
    };
    write(
        path,
        &mut files,
        MANIFEST_FILE,
        &pretty(&manifest(id, name, template, &slug)),
    )?;
    write(
        path,
        &mut files,
        &format!("schemas/{slug}.json"),
        &pretty(&schema),
    )?;
    write(
        path,
        &mut files,
        "README.md",
        &readme(name, id, template, &slug),
    )?;
    if template == "portable" {
        write(path, &mut files, "Cargo.toml", &cargo_toml(&slug, sdk))?;
        write(path, &mut files, "src/lib.rs", STARTER_SOURCE)?;
        write(path, &mut files, "build.sh", &build_script(&slug))?;
        write(path, &mut files, ".gitignore", "target/\n")?;
        write(path, &mut files, ".github/workflows/plugin.yml", workflow())?;
    }
    // A package with no code can be sealed now; one with a module waits for it.
    let sealed = template == "declarative";
    if sealed {
        package::seal(path)?;
    } else {
        let text = fs::read_to_string(path.join(MANIFEST_FILE)).map_err(|e| e.to_string())?;
        // Hashes come with the seal, so only the shape is checked here.
        serde_json::from_str::<PluginManifest>(&text).map_err(|e| e.to_string())?;
        let schema = fs::read_to_string(path.join(format!("schemas/{slug}.json")))
            .map_err(|e| e.to_string())?;
        let contributions: Contributions =
            serde_json::from_str(&schema).map_err(|e| e.to_string())?;
        contributions.check_definition()?;
    }
    Ok(Scaffold { files, sealed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declarative_package_is_sealed_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("hit-points");
        let made =
            plugin_new(&root, "com.example.hits", "Hit Points", "declarative", None).unwrap();
        assert!(made.sealed);
        let package = package::Package::load(&root).unwrap();
        assert_eq!(package.contributions.components[0].type_id, "HitPoints");
        assert_eq!(package.contributions.commands.len(), 2);
    }

    #[test]
    fn a_portable_package_has_a_crate_and_a_valid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("greeter");
        let made = plugin_new(&root, "com.example.greeter", "Greeter", "portable", None).unwrap();
        assert!(!made.sealed);
        for file in [
            "Cargo.toml",
            "src/lib.rs",
            "build.sh",
            "schemas/greeter.json",
        ] {
            assert!(root.join(file).is_file(), "{file}");
        }
        let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"greeter\""), "{cargo}");
        assert!(cargo.contains("features = [\"testing\"]"), "{cargo}");
    }

    #[test]
    fn a_bad_id_template_or_busy_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("x");
        assert!(plugin_new(&root, "nodots", "X", "declarative", None).is_err());
        assert!(plugin_new(&root, "com.example.x", "X", "native", None).is_err());
        plugin_new(&root, "com.example.x", "X", "declarative", None).unwrap();
        assert!(plugin_new(&root, "com.example.x", "X", "declarative", None).is_err());
    }
}
