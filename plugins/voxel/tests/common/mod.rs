//! Shared by the integration tests: building the plugin as a wasm module.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
}

pub fn build_wasm() -> Option<PathBuf> {
    let installed = Command::new("rustc")
        .args(["--print", "target-libdir", "--target", WASM_TARGET])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .is_some_and(|dir| dir.exists());
    if !installed {
        assert!(
            std::env::var_os("BLOCKLOOM_REQUIRE_WASM").is_none(),
            "{WASM_TARGET} is required but not installed"
        );
        eprintln!("skipping: {WASM_TARGET} is not installed (rustup target add {WASM_TARGET})");
        return None;
    }
    let output = cargo()
        .args([
            "build",
            "--release",
            "--package",
            "blockloom-voxel",
            "--target",
            WASM_TARGET,
            "--message-format=json",
        ])
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "blockloom_voxel")
        .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|name| name.as_str().map(PathBuf::from))
        .find(|p| p.extension().is_some_and(|e| e == "wasm"))
}
