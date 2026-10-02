//! The importer and the build hook, run the way the editor runs them: over a
//! real project folder, through the host, from the C entry point in this
//! process and from the WebAssembly module a package ships.
//!
//! The wasm and cdylib tests build the crate with the toolchain running them.
//! A missing `wasm32-unknown-unknown` target skips the wasm ones unless
//! `BLOCKLOOM_REQUIRE_WASM` is set, which CI does.

use blockloom_plugin_api::manifest::PortableEntry;
use blockloom_plugin_host::native::{NativeModule, default_services};
use blockloom_plugin_host::package::{self, Package};
use blockloom_plugin_host::portable::PortableModule;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
}

/// Builds this crate (for `target`, or the host) and returns the artifact
/// that is not an rlib.
fn build(target: Option<&str>) -> PathBuf {
    let mut command = cargo();
    command.args([
        "build",
        "--release",
        "--package",
        "blockloom-example-palette",
        "--message-format=json",
    ]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().expect("cargo runs");
    assert!(
        output.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| {
            message["reason"] == "compiler-artifact"
                && message["target"]["name"] == "blockloom_example_palette"
        })
        .flat_map(|message| message["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|name| name.as_str().map(PathBuf::from))
        .find(|path| path.extension().is_some_and(|e| e != "rlib" && e != "d"))
        .expect("the build reported a library")
}

/// The wasm module, or `None` when the target is not installed here.
fn wasm() -> Option<PathBuf> {
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
    Some(build(Some(WASM_TARGET)))
}

use blockloom_plugin_api::schema::Contributions;
use blockloom_plugin_host::imports::{self, ImportState};
use blockloom_plugin_host::module::CodeModule;

fn contributions() -> Contributions {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("package/schemas/palette.json");
    let c: Contributions = serde_json::from_str(&std::fs::read_to_string(schema).unwrap()).unwrap();
    c.check_definition().unwrap();
    c
}

fn native() -> CodeModule {
    CodeModule::Native(
        unsafe {
            NativeModule::from_entry(
                blockloom_example_palette::blockloom_plugin_entry_v1,
                BTreeSet::new(),
                default_services("0.0.1".to_string()),
            )
        }
        .unwrap(),
    )
}

fn portable(path: &Path) -> CodeModule {
    CodeModule::Portable(Box::new(
        PortableModule::load(
            &std::fs::read(path).unwrap(),
            &PortableEntry {
                module: "portable/palette.wasm".to_string(),
                memory_limit_mib: 16,
                call_limit_ms: 20,
            },
            BTreeSet::new(),
            default_services("0.0.1".to_string()),
        )
        .unwrap(),
    ))
}

const GPL: &str = "GIMP Palette\nName: Sunset\n255 0 0 Red\n0 128 255 Sky blue\nbad line\n";

/// What every way of running the plugin has to agree on.
fn exercise(module: &mut CodeModule) {
    let c = contributions();
    let importer = &c.importers[0];
    let hook = &c.build_hooks[0];
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path();
    std::fs::create_dir_all(project.join("assets/art")).unwrap();
    std::fs::write(project.join("assets/art/sunset.gpl"), GPL).unwrap();

    // Before any import the build hook refuses the palette.
    let error = imports::run_build_hook(
        module,
        "com.example.palette",
        hook,
        project,
        "Game",
        "x86_64-unknown-linux-gnu",
    )
    .unwrap_err();
    assert!(error.contains("sunset.gpl was never imported"), "{error}");

    let made = imports::run_import(
        module,
        "com.example.palette",
        importer,
        project,
        "Game",
        "assets/art/sunset.gpl",
    )
    .unwrap();
    assert_eq!(
        made.outputs,
        [
            "assets/art/sunset.gpl.imported/palette.png",
            "assets/art/sunset.gpl.imported/palette.json"
        ]
    );
    assert_eq!(made.warnings.len(), 1);
    let png = image::open(project.join(&made.outputs[0]))
        .unwrap()
        .to_rgba8();
    assert_eq!((png.width(), png.height()), (2, 1));
    assert_eq!(png.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(png.get_pixel(1, 0).0, [0, 128, 255, 255]);
    let listing: Value =
        serde_json::from_slice(&std::fs::read(project.join(&made.outputs[1])).unwrap()).unwrap();
    assert_eq!(listing["name"], "Sunset");
    assert_eq!(
        listing["colors"][1],
        json!({"name": "Sky blue", "hex": "#0080ff"})
    );
    assert_eq!(imports::status(project)[0].state, ImportState::Fresh);

    // A source that changes is noticed; importing again replaces the outputs.
    std::fs::write(
        project.join("assets/art/sunset.gpl"),
        "GIMP Palette\n1 2 3 One\n4 5 6 Two\n7 8 9 Three\n",
    )
    .unwrap();
    assert_eq!(
        imports::status(project)[0].state,
        ImportState::SourceChanged
    );
    imports::run_import(
        module,
        "com.example.palette",
        importer,
        project,
        "Game",
        "assets/art/sunset.gpl",
    )
    .unwrap();
    let png = image::open(project.join(&made.outputs[0]))
        .unwrap()
        .to_rgba8();
    assert_eq!(png.width(), 3);
    assert_eq!(imports::status(project)[0].state, ImportState::Fresh);

    // The hook now passes and ships the list of palettes.
    let run = imports::run_build_hook(
        module,
        "com.example.palette",
        hook,
        project,
        "Game",
        "x86_64-unknown-linux-gnu",
    )
    .unwrap();
    assert_eq!(run.files.len(), 1);
    assert_eq!(run.files[0].path, "palettes.json");
    let listed: Value = serde_json::from_slice(&run.files[0].data).unwrap();
    assert_eq!(listed, json!({"palettes": ["assets/art/sunset.gpl"]}));

    // Refusals: a file the importer doesn't take, a file that isn't a palette,
    // a path outside assets, and an import's own output.
    std::fs::write(project.join("assets/art/a.png"), b"x").unwrap();
    let wrong = imports::run_import(
        module,
        "com.example.palette",
        importer,
        project,
        "Game",
        "assets/art/a.png",
    );
    assert!(wrong.unwrap_err().contains("doesn't take .png"));
    std::fs::write(project.join("assets/art/bad.gpl"), "not a palette").unwrap();
    let bad = imports::run_import(
        module,
        "com.example.palette",
        importer,
        project,
        "Game",
        "assets/art/bad.gpl",
    );
    assert!(bad.unwrap_err().contains("couldn't import"));
    assert!(!project.join("assets/art/bad.gpl.imported").exists());
    for path in ["../x.gpl", "assets/../x.gpl", "x.gpl"] {
        assert!(
            imports::run_import(
                module,
                "com.example.palette",
                importer,
                project,
                "Game",
                path
            )
            .is_err()
        );
    }
    let own = imports::run_import(
        module,
        "com.example.palette",
        importer,
        project,
        "Game",
        "assets/art/sunset.gpl.imported/palette.json",
    );
    assert!(own.is_err());

    // An answer for an op the plugin doesn't have is its Unsupported.
    let missing = module
        .call_bytes("importer.nope", b"{}\n", 100)
        .unwrap_err();
    assert!(missing.contains("Unsupported"), "{missing}");

    // Forgetting a deleted source removes what it made.
    assert_eq!(
        imports::forget(project, "assets/art/sunset.gpl").unwrap(),
        2
    );
    assert!(!project.join("assets/art/sunset.gpl.imported").exists());
}

#[test]
fn the_entry_point_imports_and_cooks_in_this_process() {
    exercise(&mut native());
}

#[test]
fn the_wasm_module_imports_and_cooks_in_the_portable_executor() {
    let Some(path) = wasm() else { return };
    let mut module = portable(&path);
    exercise(&mut module);
    assert!(!module.is_stopped());
    println!(
        "palette.wasm is {} bytes",
        std::fs::metadata(&path).unwrap().len()
    );
}

#[test]
fn the_package_seals_and_is_an_editor_tool_that_does_not_ship() {
    let Some(wasm) = wasm() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("com.example.palette");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("package");
    package::copy_dir(&source, &root).unwrap();
    std::fs::create_dir_all(root.join("portable")).unwrap();
    std::fs::copy(&wasm, root.join("portable/palette.wasm")).unwrap();
    package::seal(&root).unwrap();
    let package = Package::load(&root).unwrap();
    assert_eq!(package.manifest.id, "com.example.palette");
    assert_eq!(package.contributions.importers.len(), 1);
    assert_eq!(package.contributions.build_hooks.len(), 1);
    assert!(package.target_hashes().contains_key("portable"));
}
