//! Building a project into a game that runs on its own.
//!
//! A build is the player binary, the project's [`crate::pack::GamePack`], its
//! assets, a baked sprite atlas, compiled scripts, optional native block
//! logic, platform wrapper, icons and shareable ZIP (see [`crate::pack`] for
//! the shape).
//!
//! Which platforms an install can build for is a question about what it has
//! beside it. The player is a native binary that nothing here can produce, so
//! one per platform is staged under `players/<triple>/` next to the editor
//! (`just player`), and the machine doing the building always has its own.
//! Scripts are the other half: they are native too, so a project with one can
//! only be built for a platform this machine's rustc can compile for.
//! [`targets`] answers both questions at once, which is what the Build dialog
//! shows.

use crate::android;
use crate::codegen;
use crate::distribution;
use crate::pack::{self, GamePack};
use crate::project::{self, Project};
use crate::script;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// One platform a game can be built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The rustc target triple, which doubles as the `players/` folder name.
    pub triple: &'static str,
    /// What the Build dialog calls it.
    pub label: &'static str,
    /// Whether an executable there wants `.exe` on the end.
    pub windows: bool,
}

impl Target {
    pub fn is_macos(self) -> bool {
        self.triple.ends_with("apple-darwin")
    }

    pub fn is_linux(self) -> bool {
        self.triple.ends_with("linux-gnu")
    }

    /// The browser: one `.html` file rather than a folder with a binary.
    pub fn is_web(self) -> bool {
        script::is_web(self.triple)
    }

    /// An installable APK cross-built through the NDK, never a staged
    /// player binary.
    pub fn is_android(self) -> bool {
        android::is_android(self.triple)
    }

    /// Whether a build for it renders HDR unless told otherwise, and why.
    /// ARM64 Linux is mostly single-board computers, where FP16 targets cost
    /// more than they give and HDR displays are rare. Android is SDR-only in
    /// v1 like the web player: weak mobile GPUs with no HDR output.
    pub fn hdr_default(self) -> (bool, &'static str) {
        if self.is_web() {
            (false, "SDR only: browsers give a page no HDR output yet.")
        } else if self.is_android() {
            (
                false,
                "SDR only: Android v1 targets weak mobile GPUs with no HDR output.",
            )
        } else if self.triple == "aarch64-unknown-linux-gnu" {
            (
                false,
                "SDR only by default: ARM64 Linux boards rarely have the GPU for FP16 frames.",
            )
        } else {
            (
                true,
                "HDR frames, and HDR output where the display offers it.",
            )
        }
    }
}

/// Every platform Blockloom knows how to lay a build out for. Whether one is
/// available on this machine is a separate question - see [`targets`].
pub const TARGETS: &[Target] = &[
    Target {
        triple: "x86_64-pc-windows-msvc",
        label: "Windows x64",
        windows: true,
    },
    Target {
        triple: "aarch64-pc-windows-msvc",
        label: "Windows ARM64",
        windows: true,
    },
    Target {
        triple: "x86_64-unknown-linux-gnu",
        label: "Linux x64",
        windows: false,
    },
    Target {
        triple: "aarch64-unknown-linux-gnu",
        label: "Linux ARM64",
        windows: false,
    },
    Target {
        triple: "x86_64-apple-darwin",
        label: "macOS Intel",
        windows: false,
    },
    Target {
        triple: "aarch64-apple-darwin",
        label: "macOS Apple Silicon",
        windows: false,
    },
    Target {
        triple: "wasm32-unknown-unknown",
        label: "Web",
        windows: false,
    },
    Target {
        triple: "aarch64-linux-android",
        label: "Android (arm64)",
        windows: false,
    },
    Target {
        triple: "x86_64-linux-android",
        label: "Android Emulator (x64)",
        windows: false,
    },
];

/// The target `triple` names, if it is one Blockloom knows.
pub fn target(triple: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|target| target.triple == triple)
}

/// The platform doing the building. `None` means one Blockloom has no name
/// for, which leaves it unable to say what it would even be building.
pub fn host() -> Option<&'static Target> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    };
    target(triple)
}

pub fn is_host(target: &Target) -> bool {
    host().is_some_and(|host| host.triple == target.triple)
}

/// How a script is compiled for `target`: this machine's own build is the one
/// Play already made, anywhere else is a cross build of its own.
pub fn script_target(target: &Target) -> Option<&'static str> {
    (!is_host(target)).then_some(target.triple)
}

/// What one platform can do for this project right now, as the Build dialog
/// lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetStatus {
    pub triple: String,
    pub label: String,
    /// Whether this is the machine doing the building.
    pub host: bool,
    /// Whether a build for it would get anywhere.
    pub ready: bool,
    /// What it would build with, or what is missing.
    pub note: String,
    /// Whether this project can compile its blocks natively for the target.
    pub fast_ready: bool,
    /// Why native blocks are or are not available.
    pub fast_note: String,
    /// Whether a build renders HDR unless told otherwise.
    pub hdr: bool,
    /// Why, for the dialog.
    pub hdr_note: String,
}

/// Every platform, in the order the dialog lists them: this machine first,
/// since it is the one that always works.
///
/// `has_scripts` is whether the project has any Rust in it, which is what
/// decides whether a toolchain gets a say.
pub fn targets(
    has_scripts: bool,
    fast_source: Result<(), String>,
    fallback_player: &Path,
) -> Vec<TargetStatus> {
    let mut statuses: Vec<TargetStatus> = TARGETS
        .iter()
        .map(|target| status(target, has_scripts, &fast_source, fallback_player))
        .collect();
    statuses.sort_by_key(|status| !status.host);
    statuses
}

/// The Build dialog row for an Android target. Readiness is the toolchain
/// probes, since there is no player payload to stage; native block logic
/// ships as one more `.so` with the VM as fallback.
fn android_status(
    target: &Target,
    host: bool,
    ready: bool,
    note: String,
    fast_source: &Result<(), String>,
) -> TargetStatus {
    let (fast_ready, fast_note) = if !ready {
        (
            false,
            "The platform is not available for a build.".to_string(),
        )
    } else if let Err(error) = fast_source {
        (false, error.clone())
    } else if let Err(error) = script::target_installed(target.triple) {
        (false, error)
    } else {
        (
            true,
            "Blocks will be compiled to a native library in the APK.".to_string(),
        )
    };
    TargetStatus {
        triple: target.triple.to_string(),
        label: target.label.to_string(),
        host,
        ready,
        note,
        fast_ready,
        fast_note,
        hdr: target.hdr_default().0,
        hdr_note: target.hdr_default().1.to_string(),
    }
}

fn status(
    target: &Target,
    has_scripts: bool,
    fast_source: &Result<(), String>,
    fallback_player: &Path,
) -> TargetStatus {
    let host = is_host(target);
    // Android has no staged player: the desktop NDK cross-builds the
    // runtime, so readiness is SDK plus NDK plus JDK plus Rust target.
    if target.is_android() {
        let (ready, note) = android::readiness_for(target.triple);
        return android_status(target, host, ready, note, fast_source);
    }
    let (ready, note) = match player_for(target, fallback_player) {
        None => (
            false,
            format!(
                "No player for this platform. Stage one in players/{}/ beside Blockloom.",
                target.triple
            ),
        ),
        Some(_) if target.is_web() && !has_scripts => (
            true,
            "One .html file with the whole game inside; opens in any browser with WebGPU."
                .to_string(),
        ),
        Some(_) if !has_scripts => (
            true,
            if host {
                "This machine.".to_string()
            } else {
                "Using the staged player.".to_string()
            },
        ),
        // A script is native code like the player is, so it has to be built
        // for wherever the game is going.
        Some(_) => match script_target(target) {
            None => match script::toolchain_version() {
                Ok(_) => (true, "This machine, scripts included.".to_string()),
                Err(e) => (false, e),
            },
            Some(triple) => match script::target_installed(triple) {
                Ok(()) if target.is_web() => (
                    true,
                    "One .html file with the whole game inside; scripts compile to wasm."
                        .to_string(),
                ),
                Ok(()) => (true, "Scripts will be cross-compiled for it.".to_string()),
                Err(e) => (false, format!("This project has scripts, and {e}.")),
            },
        },
    };
    let fast_toolchain = match script_target(target) {
        None => script::toolchain_version().map(|_| ()),
        Some(triple) => script::target_installed(triple),
    };
    let (fast_ready, fast_note) = if !ready {
        (
            false,
            "The platform is not available for a build.".to_string(),
        )
    } else if target.is_web() {
        (false, "Blocks run on the VM in a browser.".to_string())
    } else if let Err(error) = fast_source {
        (false, error.clone())
    } else if let Err(error) = fast_toolchain {
        (false, error)
    } else {
        (
            true,
            "Blocks will be compiled to optimized native code.".to_string(),
        )
    };
    TargetStatus {
        triple: target.triple.to_string(),
        label: target.label.to_string(),
        host,
        ready,
        note,
        fast_ready,
        fast_note,
        hdr: target.hdr_default().0,
        hdr_note: target.hdr_default().1.to_string(),
    }
}

/// The player to copy into a build for `target`: the payload staged for it,
/// or - for this machine only - `fallback`, which the caller names since it is
/// the runtime the editor itself plays with.
pub fn player_for(target: &Target, fallback: &Path) -> Option<PathBuf> {
    if let Some(staged) = staged_player(target, fallback) {
        return Some(staged);
    }
    if is_host(target) && fallback.is_file() {
        return Some(fallback.to_path_buf());
    }
    None
}

/// A payload under `players/<triple>/` beside this executable, or under
/// `BLOCKLOOM_PLAYERS` when that is set. It wins even for this machine: it is
/// the copy meant for shipping (see `just player`). The web player is its
/// wasm file, with its JS glue beside it (`just web-player`).
fn staged_player(target: &Target, fallback: &Path) -> Option<PathBuf> {
    let stem = fallback.file_stem()?.to_string_lossy().into_owned();
    let name = if target.is_web() {
        crate::web_build::PLAYER_WASM.to_string()
    } else if target.windows {
        format!("{stem}.exe")
    } else {
        stem
    };
    let players = match std::env::var_os("BLOCKLOOM_PLAYERS") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_exe().ok()?.parent()?.join("players"),
    };
    let path = players.join(target.triple).join(name);
    let complete = !target.is_web() || path.with_file_name(crate::web_build::PLAYER_GLUE).is_file();
    (path.is_file() && complete).then_some(path)
}

/// What a build carries beyond the project itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildOptions {
    /// Ship the blocks as one native library as well as the document.
    pub fast: bool,
    /// Clamp the player to an 8-bit SDR frame, for targets too weak for
    /// FP16 targets and HDR output.
    pub sdr_only: bool,
    /// Release keystore password for this build only. `None` reads the env,
    /// then the OS keyring (see `android::STORE_PASS_ENV`); ignored off
    /// Android. Never stored unless `remember_passwords` says so.
    pub store_pass: Option<String>,
    /// Release key password when it differs from the store's. Falls back
    /// to the store password; ignored off Android. Never stored unless
    /// `remember_passwords` says so.
    pub key_pass: Option<String>,
    /// Keep the release passwords in the OS keyring after a successful
    /// build, so the next one can skip typing them. Opt-in per build, and
    /// only written when the APK signed: a failed build remembers nothing.
    pub remember_passwords: bool,
    /// The plugins the game ships with, resolved by the editor from the
    /// project's lock: what to record in the pack and which files to copy.
    pub plugins: Vec<PluginPayload>,
}

/// One plugin to ship: what the pack records and where its files are now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPayload {
    pub entry: pack::PackedPlugin,
    /// The verified package folder the files are copied from.
    pub root: PathBuf,
}

/// Copies each plugin's manifest and shipped files to `game/plugins/<id>/`.
/// A file that is not where the verified package says it is fails the build.
fn copy_plugins(plugins: &[PluginPayload], game: &Path) -> Result<usize, String> {
    for plugin in plugins {
        let dest = game.join(&plugin.entry.dir);
        // The player verifies the rest against the manifest, so it ships too.
        let manifest = blockloom_plugin_api::manifest::MANIFEST_FILE;
        for file in std::iter::once(manifest).chain(plugin.entry.files.iter().map(String::as_str)) {
            let from = plugin.root.join(file);
            let to = dest.join(file);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::copy(&from, &to)
                .map_err(|e| format!("plugin {}: couldn't copy {file}: {e}", plugin.entry.id))?;
        }
    }
    Ok(plugins.len())
}

/// Where a build landed, and what went into it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Build {
    /// The build folder itself.
    pub dir: PathBuf,
    /// What somebody launches: the executable, wrapper, or app binary.
    pub binary: PathBuf,
    /// The ZIP somebody can send to another machine.
    pub archive: PathBuf,
    pub target: &'static str,
    pub assets: usize,
    pub scripts: usize,
    pub compiled: bool,
    /// How many Image looks the baked sprite atlas carries. 0 means none
    /// was worth baking and every sprite draws from its own file.
    pub atlas: usize,
    /// How many surface shader files were checked before shipping.
    pub shaders: usize,
    /// Whether the HDR sky was baked to BC6H.
    pub sky: bool,
    /// Whether the DLSS redistributable rode along beside the player.
    /// True when the staged player had its DLLs and they were copied;
    /// a false here just means the game falls back to TAA plus spatial.
    #[serde(default)]
    pub dlss: bool,
    /// The APK's package id, for the Build dialog's install step to launch.
    /// Empty on every non-Android target.
    #[serde(default)]
    pub application_id: String,
    /// `debug` or `release` on Android, empty everywhere else.
    #[serde(default)]
    pub signed: String,
    /// Bytes of what ships: the ZIP, or for the web the one `.html`.
    pub size: u64,
    /// True when nothing changed and the previous output was reused.
    /// Only Android sets this; other targets always rebuild.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cached: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What the build folder is called. The platform is in the name because one
/// output folder holds a build per platform, and three folders called the
/// same thing would be three chances to ship the wrong one.
pub fn build_name(project: &Project, target: &Target) -> String {
    format!("{} ({})", project::folder_name(&project.name), target.label)
}

/// Builds `project` for `target` into a folder under `parent`, replacing a
/// previous build for the same platform.
///
/// Scripts are shipped as the libraries the editor already built for that
/// platform, so whoever calls this compiles them first: a script with no
/// library is an error here rather than an actor that quietly does nothing in
/// the shipped game.
pub fn build(
    project: &Project,
    project_dir: &Path,
    target: &'static Target,
    player: &Path,
    parent: &Path,
    options: BuildOptions,
) -> Result<Build, String> {
    if (target.is_web() || target.is_android())
        && let Some(plugin) = options
            .plugins
            .iter()
            .find(|p| p.entry.tier != "declarative")
    {
        return Err(format!(
            "plugin {} has code, and plugin code is not supported on {} builds yet",
            plugin.entry.id, target.label
        ));
    }
    if target.is_web() {
        return build_web(project, project_dir, target, player, parent);
    }
    if target.is_android() {
        return build_android(project, project_dir, target, player, parent, options);
    }
    let fast = options.fast;
    crate::build_control::step("Checking shaders")?;
    let shaders = check_shaders(project, project_dir)?;
    let dir = parent.join(build_name(project, target));
    clear_build_dir(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join(BUILD_MARKER), target.triple)
        .map_err(|error| format!("{}: {error}", dir.display()))?;

    let name = project::folder_name(&project.name);
    let layout = layout(&dir, &name, target)?;
    std::fs::copy(player, &layout.player).map_err(|e| {
        format!(
            "couldn't copy the player {} -> {}: {e}",
            player.display(),
            layout.player.display()
        )
    })?;
    // Copying a file doesn't carry its mode everywhere, and a game nobody can
    // execute isn't one.
    make_executable(&layout.player)?;
    // The DLSS redistributable, when the staged player carries it: the DLLs
    // live beside the player payload, and ship beside the built one.
    let dlss = copy_dlss_redistributable(player, &layout.player)?;

    let game = layout.game.clone();
    std::fs::create_dir_all(&game).map_err(|e| format!("{}: {e}", game.display()))?;
    let mut game_pack = GamePack::new(project.clone())
        .with_plugins(options.plugins.iter().map(|p| p.entry.clone()).collect());
    game_pack.hdr = !options.sdr_only;
    game_pack.write(&pack::pack_path(&game))?;

    crate::build_control::step("Copying game assets")?;
    let assets = copy_assets(project_dir, &game)?;
    copy_plugins(&options.plugins, &game)?;
    crate::build_control::step("Baking sprite atlas")?;
    let atlas = bake_sprite_atlas(project, project_dir, &game)?;
    crate::build_control::step("Baking sky")?;
    let sky = bake_sky(project, project_dir, &game)?;
    copy_probes(project, project_dir, &game)?;
    crate::build_control::step("Packing terrain and scripts")?;
    copy_terrain(project, project_dir, &game)?;
    let scripts = copy_scripts(project, project_dir, &game, target)?;
    let compiled = if fast {
        copy_logic(project_dir, &game, target)?;
        true
    } else {
        false
    };

    let icons = distribution::Icons::load(project_dir, &project.icon)?;
    decorate(project, target, &layout, &icons)?;
    let archive = parent.join(format!("{}.zip", build_name(project, target)));
    crate::build_control::step("Creating shareable archive")?;
    distribution::archive(&dir, &archive, &layout.executables)?;
    let size = file_size(&archive);

    Ok(Build {
        dir,
        binary: layout.binary,
        archive,
        size,
        target: target.triple,
        assets,
        scripts,
        compiled,
        atlas,
        shaders,
        sky,
        dlss,
        application_id: String::new(),
        signed: String::new(),
        cached: false,
    })
}

/// Copies the DLSS redistributable beside a built player, when the staged
/// player carries it. The SDK is fetched per clone (`just dlss-sdk`), so a
/// DLSS build stages its DLLs beside the player payload (`just player-dlss`
/// copies them there); a build then carries them beside its own player, and
/// the run-time probe picks them up. Answers whether anything rode along -
/// missing DLLs are not an error, the game just falls back to TAA.
///
/// Windows carries `nvngx_dlss.dll` (plus `nvngx_dlssd.dll` for ray
/// reconstruction); Linux carries `libnvidia-ngx-dlss.so.*` (plus the
/// `dlssd` twin). Frame generation (`dlssg`) stays behind: nothing here
/// uses it. A license blurb beside the player (`DLSS_LICENSE.txt`,
/// the section 9.5 text the SDK license asks shippers to include) rides too.
fn copy_dlss_redistributable(player: &Path, built: &Path) -> Result<bool, String> {
    let Some(payload_dir) = player.parent() else {
        return Ok(false);
    };
    let Some(built_dir) = built.parent() else {
        return Ok(false);
    };
    let mut carried = false;
    let mut entries: Vec<String> = Vec::new();
    if let Ok(listing) = std::fs::read_dir(payload_dir) {
        for entry in listing.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            let dll = dlss_binary(&lower)
                || lower == "dlss_license.txt"
                || lower == "nvngx_license.txt"
                || lower == "license.dlss.txt";
            if dll {
                entries.push(name);
            }
        }
    }
    entries.sort();
    for name in entries {
        let from = payload_dir.join(&name);
        // Directories named like a DLL never ride; only files do.
        if !from.is_file() {
            continue;
        }
        let to = built_dir.join(&name);
        // The player copy above already made the dir, but a macOS bundle
        // lays the player two levels deep; make sure either way.
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(&from, &to)
            .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
        if dlss_binary(&name.to_ascii_lowercase()) {
            carried = true;
        }
        // Linux players run through a wrapper that must stay executable,
        // and the .so files need no such bit; DLLs never need it either.
    }
    Ok(carried)
}

/// A DLSS super-resolution or ray-reconstruction binary, lowercased:
/// exactly what the probe can load, and nothing else (frame generation
/// stays behind).
fn dlss_binary(lower: &str) -> bool {
    lower == "nvngx_dlss.dll"
        || lower == "nvngx_dlssd.dll"
        || (!lower.ends_with(".debug")
            && (lower.starts_with("libnvidia-ngx-dlss.so")
                || lower.starts_with("libnvidia-ngx-dlssd.so")))
}

/// A web build: one `.html` holding the player, the pack, the game files and
/// each script's wasm module (see [`crate::web_build`]). The game folder is
/// laid out exactly as a native build's, in a scratch folder, then packed
/// into the page. Always SDR and always the VM.
fn build_web(
    project: &Project,
    project_dir: &Path,
    target: &'static Target,
    player: &Path,
    parent: &Path,
) -> Result<Build, String> {
    crate::build_control::step("Checking shaders")?;
    let shaders = check_shaders(project, project_dir)?;
    let glue_path = player.with_file_name(crate::web_build::PLAYER_GLUE);
    let player_wasm = std::fs::read(player).map_err(|e| format!("{}: {e}", player.display()))?;
    let player_glue =
        std::fs::read(&glue_path).map_err(|e| format!("{}: {e}", glue_path.display()))?;

    let dir = parent.join(build_name(project, target));
    clear_build_dir(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join(BUILD_MARKER), target.triple)
        .map_err(|error| format!("{}: {error}", dir.display()))?;

    let game = dir.join(".game");
    std::fs::create_dir_all(&game).map_err(|e| format!("{}: {e}", game.display()))?;
    let mut game_pack = GamePack::new(project.clone());
    game_pack.hdr = false;
    game_pack.write(&pack::pack_path(&game))?;
    crate::build_control::step("Copying game assets")?;
    let assets = copy_assets(project_dir, &game)?;
    crate::build_control::step("Baking sprite atlas")?;
    let atlas = bake_sprite_atlas(project, project_dir, &game)?;
    crate::build_control::step("Baking sky")?;
    let sky = bake_sky(project, project_dir, &game)?;
    copy_probes(project, project_dir, &game)?;
    crate::build_control::step("Packing terrain and scripts")?;
    copy_terrain(project, project_dir, &game)?;
    let scripts = copy_scripts(project, project_dir, &game, target)?;

    let mut paths = Vec::new();
    distribution::collect_files(&game, &mut paths)?;
    let mut files = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(&game)
            .map_err(|_| format!("{} is outside the game folder", path.display()))?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        files.push((crate::vfs::key(relative), bytes));
    }
    files.sort();
    let icons = distribution::Icons::load(project_dir, &project.icon)?;
    crate::build_control::step("Packaging web page")?;
    let html = crate::web_build::page(crate::web_build::Page {
        title: &project.name,
        icon: &icons.png,
        player_wasm,
        player_glue,
        files,
    })?;
    std::fs::remove_dir_all(&game).map_err(|e| format!("{}: {e}", game.display()))?;

    let binary = dir.join(format!("{}.html", project::folder_name(&project.name)));
    std::fs::write(&binary, &html).map_err(|e| format!("{}: {e}", binary.display()))?;
    let archive = parent.join(format!("{}.zip", build_name(project, target)));
    crate::build_control::step("Creating shareable archive")?;
    distribution::archive(&dir, &archive, &[])?;

    Ok(Build {
        dir,
        size: file_size(&binary),
        binary,
        archive,
        target: target.triple,
        assets,
        scripts,
        compiled: false,
        atlas,
        shaders,
        sky,
        // No DLSS in a browser: WebGPU has no SDK path.
        dlss: false,
        application_id: String::new(),
        signed: String::new(),
        cached: false,
    })
}

/// An Android build: the same game-folder staging desktop and web share,
/// then APK assembly straight from the installed SDK build-tools (see
/// `android`). `runtime_so` is the NDK cross-build of `blockloom-runtime`
/// the caller resolved - the APK's `lib/<abi>/` entry everything runs
/// through. Scripts and native logic ride beside it as more `.so` files;
/// whoever calls this compiled them first, same as every other target.
fn build_android(
    project: &Project,
    project_dir: &Path,
    target: &'static Target,
    runtime_so: &Path,
    parent: &Path,
    options: BuildOptions,
) -> Result<Build, String> {
    let config = android::load();
    build_android_with_config(
        project,
        project_dir,
        target,
        runtime_so,
        parent,
        options,
        &config,
    )
}

// Seven params because the test seam takes what production loads globally;
// splitting the struct up further would just move the list.
#[allow(clippy::too_many_arguments)]
fn build_android_with_config(
    project: &Project,
    project_dir: &Path,
    target: &'static Target,
    runtime_so: &Path,
    parent: &Path,
    options: BuildOptions,
    config: &android::AppConfig,
) -> Result<Build, String> {
    let dir = parent.join(build_name(project, target));
    // Fast path first: when nothing changed the previous APK is still good,
    // so skip staging, baking and signing entirely. No passwords needed in
    // that case since nothing signs.
    if let Some(cached) = android_cached_build(
        project,
        project_dir,
        target,
        runtime_so,
        &options,
        config,
        &dir,
    ) {
        return Ok(cached);
    }
    let (ready, note) = android::readiness_for_config(config, target.triple);
    if !ready {
        return Err(format!("Can't build for {} yet: {note}", target.label));
    }
    // The key before anything expensive: a release row with no password
    // stops the build now, not after minutes of baking.
    let signing = android::resolve_signing(
        &project.android,
        options.store_pass.as_deref(),
        options.key_pass.as_deref(),
    )?;
    if !runtime_so.is_file() {
        return Err(format!(
            "The runtime library {} is missing, so there is nothing to run.",
            runtime_so.display()
        ));
    }
    // Resolve the tools before staging anything: a missing binary stops
    // the build now, not after minutes of baking.
    let tools = android::apk_tools_for(config)?;

    crate::build_control::step("Checking shaders")?;
    let shaders = check_shaders(project, project_dir)?;
    clear_build_dir(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join(BUILD_MARKER), target.triple)
        .map_err(|error| format!("{}: {error}", dir.display()))?;

    // The staged game folder becomes the APK's `assets/`, read through
    // Bevy's Android asset reader rather than the disk.
    let game = dir.join("assets");
    std::fs::create_dir_all(&game).map_err(|e| format!("{}: {e}", game.display()))?;
    let mut game_pack = GamePack::new(project.clone());
    game_pack.hdr = false;
    game_pack.write(&pack::pack_path(&game))?;

    crate::build_control::step("Copying game assets")?;
    let assets = copy_assets(project_dir, &game)?;
    crate::build_control::step("Baking sprite atlas")?;
    let atlas = bake_sprite_atlas(project, project_dir, &game)?;
    crate::build_control::step("Baking sky")?;
    let sky = bake_sky(project, project_dir, &game)?;
    copy_probes(project, project_dir, &game)?;
    crate::build_control::step("Packing terrain and scripts")?;
    copy_terrain(project, project_dir, &game)?;
    let native_libs = android_native_libs(project, project_dir, target, runtime_so, options.fast)?;

    let manifest = android::render_manifest(&project.android, &project.name)?;
    let icons = android::launcher_icons(project_dir, &project.icon)?;
    let contents = android::ApkContents {
        manifest,
        icons,
        assets_dir: game,
        native_libs,
    };
    let apk_name = format!("{}.apk", project::folder_name(&project.name));
    let binary = dir.join(&apk_name);
    let mut report =
        android::assemble_apk(&contents, &dir.join("apk-work"), &tools, &signing, &binary)?;
    report.application_id = project.android.application_id_for(&project.name)?;
    report.version_name = project.android.version_name_or_default();
    report.version_code = project.android.version_code_or_default();
    if options.remember_passwords && signing.release {
        // The APK just signed with these, so they are worth keeping. A
        // keyring that won't keep them only affects the next build's
        // typing, never this one's success.
        let _ = android::remember_signing(
            &project.android,
            options.store_pass.as_deref(),
            options.key_pass.as_deref(),
        );
    }

    let archive = parent.join(format!("{}.zip", build_name(project, target)));
    crate::build_control::step("Creating shareable archive")?;
    distribution::archive(&dir, &archive, &[])?;
    let built = Build {
        dir: dir.clone(),
        binary: binary.clone(),
        archive: archive.clone(),
        size: report.size,
        target: target.triple,
        assets,
        scripts: script_lib_count(project),
        compiled: options.fast,
        atlas,
        shaders,
        sky,
        dlss: false,
        application_id: report.application_id.clone(),
        signed: report.signed.clone(),
        cached: false,
    };
    write_android_cache(
        &dir,
        project,
        project_dir,
        target,
        runtime_so,
        &options,
        config,
        &built,
    );
    Ok(built)
}

/// The fingerprint sidecar in an Android output dir. Rebuilding deletes the
/// dir, so a missing file just means a first build.
const ANDROID_FINGERPRINT_FILE: &str = ".blockloom-android-fingerprint.json";

/// What a cache hit returns without rebuilding: the counts a fresh build
/// would have reported, plus the fingerprint they were built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AndroidBuildCache {
    fingerprint: String,
    assets: usize,
    scripts: usize,
    compiled: bool,
    atlas: usize,
    shaders: usize,
    sky: bool,
    application_id: String,
    signed: String,
}

/// The previous APK when nothing changed: its fingerprint still matches and
/// both outputs are still on disk. Passwords are not needed since nothing
/// signs; a missing file is not an error, just a rebuild.
fn android_cached_build(
    project: &Project,
    project_dir: &Path,
    target: &'static Target,
    runtime_so: &Path,
    options: &BuildOptions,
    config: &android::AppConfig,
    dir: &Path,
) -> Option<Build> {
    let wanted = android_build_fingerprint(
        project,
        project_dir,
        target,
        runtime_so,
        options,
        config,
        dir,
    )
    .ok()?;
    let text = std::fs::read_to_string(dir.join(ANDROID_FINGERPRINT_FILE)).ok()?;
    let cache: AndroidBuildCache = serde_json::from_str(&text).ok()?;
    if cache.fingerprint != wanted {
        return None;
    }
    let apk_name = format!("{}.apk", project::folder_name(&project.name));
    let binary = dir.join(&apk_name);
    let archive = dir
        .parent()
        .map(|parent| parent.join(format!("{}.zip", build_name(project, target))))
        .unwrap_or_else(|| dir.join("build.zip"));
    if !binary.is_file() || !archive.is_file() {
        return None;
    }
    Some(Build {
        dir: dir.to_path_buf(),
        binary,
        archive,
        size: file_size(&dir.join(&apk_name)),
        target: target.triple,
        assets: cache.assets,
        scripts: cache.scripts,
        compiled: cache.compiled,
        atlas: cache.atlas,
        shaders: cache.shaders,
        sky: cache.sky,
        dlss: false,
        application_id: cache.application_id,
        signed: cache.signed,
        cached: true,
    })
}

/// Records a fresh Android build for the next launch to reuse. Best effort:
/// a write failure just means the next build rebuilds.
#[allow(clippy::too_many_arguments)]
fn write_android_cache(
    dir: &Path,
    project: &Project,
    project_dir: &Path,
    target: &Target,
    runtime_so: &Path,
    options: &BuildOptions,
    config: &android::AppConfig,
    built: &Build,
) {
    let Ok(fingerprint) = android_build_fingerprint(
        project,
        project_dir,
        target,
        runtime_so,
        options,
        config,
        dir,
    ) else {
        return;
    };
    let cache = AndroidBuildCache {
        fingerprint,
        assets: built.assets,
        scripts: built.scripts,
        compiled: built.compiled,
        atlas: built.atlas,
        shaders: built.shaders,
        sky: built.sky,
        application_id: built.application_id.clone(),
        signed: built.signed.clone(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(dir.join(ANDROID_FINGERPRINT_FILE), text);
    }
}

/// What an Android APK is built from, as one hash. Project JSON covers
/// blocks and settings; the file walk covers assets on disk; native lib
/// mtimes cover the compiled inputs; toolchain and NDK cover the tools.
/// Passwords never land here since they do not change the bytes.
#[allow(clippy::too_many_arguments)]
fn android_build_fingerprint(
    project: &Project,
    project_dir: &Path,
    target: &Target,
    runtime_so: &Path,
    options: &BuildOptions,
    config: &android::AppConfig,
    output_dir: &Path,
) -> Result<String, String> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let project_json =
        serde_json::to_string(project).map_err(|e| format!("couldn't hash the project: {e}"))?;
    project_json.hash(&mut hasher);
    target.triple.hash(&mut hasher);
    options.fast.hash(&mut hasher);
    hash_file_meta(&mut hasher, runtime_so);
    for relative in script_paths(project) {
        let library = script::library_path_for(project_dir, relative, Some(target.triple));
        hash_file_meta(&mut hasher, &library);
    }
    if options.fast {
        hash_file_meta(
            &mut hasher,
            &codegen::library_path_for(project_dir, Some(target.triple)),
        );
    }
    hash_project_files(&mut hasher, project_dir, output_dir)?;
    // Toolchain moves rebuild the runtime first, whose mtime then busts
    // this too; still hash them so a tools-only change is caught directly.
    script::toolchain_version()
        .unwrap_or_default()
        .hash(&mut hasher);
    android::ndk_revision(config).hash(&mut hasher);
    config.sdk_path.hash(&mut hasher);
    config.ndk_path.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

/// A file's identity for the fingerprint: its bytes when small enough to
/// hash cheaply are ideal, but mtime plus size is enough here since the
/// runtime and script stamps already guard content. Missing hashes as
/// missing so a first build never hits.
fn hash_file_meta(hasher: &mut std::collections::hash_map::DefaultHasher, path: &Path) {
    match std::fs::metadata(path) {
        Ok(meta) => {
            path.to_string_lossy().hash(&mut *hasher);
            meta.len().hash(&mut *hasher);
            if let Ok(mtime) = meta.modified()
                && let Ok(age) = mtime.duration_since(std::time::UNIX_EPOCH)
            {
                age.as_secs().hash(&mut *hasher);
                age.subsec_nanos().hash(&mut *hasher);
            }
        }
        Err(_) => {
            path.to_string_lossy().hash(&mut *hasher);
            "missing".hash(&mut *hasher);
        }
    }
}

/// Every file under the project folder, sorted, minus the script build
/// cache and the output dir. Host Play builds touch that cache constantly;
/// the triple libs above already cover what Android ships. The output dir
/// is skipped since a previous APK inside the project would bust every
/// fingerprint on its own mtime.
fn hash_project_files(
    hasher: &mut std::collections::hash_map::DefaultHasher,
    project_dir: &Path,
    output_dir: &Path,
) -> Result<(), String> {
    let build_cache = project_dir.join(".blockloom").join("build");
    let mut files: Vec<(String, u64, u64, u32)> = Vec::new();
    let mut stack = vec![project_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.starts_with(&build_cache) || dir.starts_with(output_dir) {
            continue;
        }
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.starts_with(&build_cache) || path.starts_with(output_dir) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(relative) = path.strip_prefix(project_dir) else {
                continue;
            };
            let (len, secs, nanos) = match std::fs::metadata(&path) {
                Ok(meta) => {
                    let (secs, nanos) = meta
                        .modified()
                        .ok()
                        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|age| (age.as_secs(), age.subsec_nanos()))
                        .unwrap_or((0, 0));
                    (meta.len(), secs, nanos)
                }
                Err(_) => continue,
            };
            files.push((
                relative.to_string_lossy().replace('\\', "/"),
                len,
                secs,
                nanos,
            ));
        }
    }
    files.sort();
    for (relative, len, secs, nanos) in files {
        relative.hash(&mut *hasher);
        len.hash(&mut *hasher);
        secs.hash(&mut *hasher);
        nanos.hash(&mut *hasher);
    }
    Ok(())
}

/// Every script's prebuilt `.so` for `target`, runtime first: the exact
/// files the APK packs under `lib/<abi>/`. A script nobody compiled for
/// the target stops the build here, the way `copy_scripts` does.
fn android_native_libs(
    project: &Project,
    project_dir: &Path,
    target: &Target,
    runtime_so: &Path,
    fast: bool,
) -> Result<Vec<(String, PathBuf)>, String> {
    let abi = android::abi(target.triple)
        .ok_or_else(|| format!("{} isn't an Android target", target.triple))?
        .to_string();
    let triple = Some(target.triple);
    let mut libs = vec![(abi.clone(), runtime_so.to_path_buf())];
    for relative in script_paths(project) {
        let library = script::library_path_for(project_dir, relative, triple);
        if !library.is_file() {
            return Err(format!(
                "{relative} hasn't been built for {}, so the game would ship without it",
                target.label
            ));
        }
        libs.push((abi.clone(), library));
    }
    if fast {
        let logic = crate::codegen::library_path_for(project_dir, triple);
        if !logic.is_file() {
            return Err(format!(
                "the blocks haven't been compiled for {}, so a fast build can't be made",
                target.label
            ));
        }
        libs.push((abi, logic));
    }
    Ok(libs)
}

/// The distinct script sources the project's actors name, like
/// `copy_scripts` collects them.
fn script_paths(project: &Project) -> Vec<&str> {
    let mut paths: Vec<&str> = project
        .scenes
        .iter()
        .flat_map(|scene| scene.actors.iter())
        .filter_map(|actor| actor.components.script())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// How many script libraries an Android build carries, for the report.
fn script_lib_count(project: &Project) -> usize {
    script_paths(project).len()
}

/// A byte count the way the Build dialog says it.
pub fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} bytes"),
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// Validates every `.wesl` surface file the project's looks draw with, the
/// way the GPU will compile it. The player warms its pipelines before its
/// first frame, so a file that won't compile has to stop the build here
/// rather than reach a player's screen.
pub fn check_shaders(project: &Project, project_dir: &Path) -> Result<usize, String> {
    // Per scene, since a 2D scene and a 3D scene check against different heads.
    let mut sources: Vec<(&str, bool)> = Vec::new();
    for scene in &project.scenes {
        crate::build_control::check()?;
        let dim3 = scene.world.mode.is_3d();
        for actor in &scene.actors {
            let Some(material) = actor.components.material() else {
                continue;
            };
            let Some(shader) = material.shader.as_ref() else {
                continue;
            };
            let source = shader.source.trim();
            if source.is_empty()
                || sources
                    .iter()
                    .any(|(known, d)| *known == source && *d == dim3)
            {
                continue;
            }
            sources.push((source, dim3));
        }
    }
    sources.sort_unstable();
    sources.dedup();
    let mut errors = Vec::new();
    for (source, dim3) in &sources {
        let verdict = crate::assets::resolve(project_dir, source)
            .ok_or_else(|| "isn't a path in this project".to_string())
            .and_then(|full| {
                std::fs::read_to_string(&full).map_err(|error| format!("couldn't be read: {error}"))
            })
            .and_then(|text| crate::material::check_surface_wesl(&text, *dim3));
        if let Err(error) = verdict {
            errors.push(format!("{source}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(sources.len())
    } else {
        Err(format!(
            "A surface shader doesn't compile, so the game wasn't built:\n{}",
            errors.join("\n")
        ))
    }
}

const BUILD_MARKER: &str = ".blockloom-build";

struct Layout {
    /// What the user launches: an executable, a Linux wrapper, or the binary
    /// inside a macOS app.
    binary: PathBuf,
    /// The native player copied from the payload.
    player: PathBuf,
    game: PathBuf,
    resources: PathBuf,
    executables: Vec<PathBuf>,
}

fn layout(dir: &Path, name: &str, target: &Target) -> Result<Layout, String> {
    if target.is_macos() {
        let contents = dir.join(format!("{name}.app")).join("Contents");
        let macos = contents.join("MacOS");
        let resources = contents.join("Resources");
        std::fs::create_dir_all(&macos).map_err(|e| format!("{}: {e}", macos.display()))?;
        std::fs::create_dir_all(&resources).map_err(|e| format!("{}: {e}", resources.display()))?;
        let player = macos.join(name);
        return Ok(Layout {
            binary: player.clone(),
            player: player.clone(),
            game: pack::game_dir(&resources),
            resources,
            executables: vec![player],
        });
    }
    if target.is_linux() {
        let wrapper = dir.join(name);
        let player = dir.join(".blockloom-player");
        return Ok(Layout {
            binary: wrapper.clone(),
            player: player.clone(),
            game: pack::game_dir(dir),
            resources: dir.to_path_buf(),
            executables: vec![wrapper, player],
        });
    }
    let player = dir.join(binary_name(name, target));
    Ok(Layout {
        binary: player.clone(),
        player: player.clone(),
        game: pack::game_dir(dir),
        resources: dir.to_path_buf(),
        executables: vec![player],
    })
}

fn decorate(
    project: &Project,
    target: &Target,
    layout: &Layout,
    icons: &distribution::Icons,
) -> Result<(), String> {
    let name = project::folder_name(&project.name);
    if target.windows {
        std::fs::write(layout.resources.join(format!("{name}.ico")), &icons.ico)
            .map_err(|error| format!("couldn't write the Windows icon: {error}"))?;
        return distribution::apply_windows_icon(&layout.player, &icons.ico);
    }
    if target.is_macos() {
        std::fs::write(layout.resources.join("GameIcon.icns"), &icons.icns)
            .map_err(|error| format!("couldn't write the macOS icon: {error}"))?;
        let contents = layout
            .resources
            .parent()
            .ok_or("the macOS app has no Contents folder")?;
        std::fs::write(contents.join("Info.plist"), info_plist(project, &name))
            .map_err(|error| format!("couldn't write Info.plist: {error}"))?;
        return std::fs::write(contents.join("PkgInfo"), "APPL????")
            .map_err(|error| format!("couldn't write PkgInfo: {error}"));
    }
    if target.is_linux() {
        std::fs::write(layout.resources.join(format!("{name}.png")), &icons.png)
            .map_err(|error| format!("couldn't write the Linux icon: {error}"))?;
        std::fs::write(
            &layout.binary,
            "#!/bin/sh\nHERE=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$HERE/.blockloom-player\" \"$@\"\n",
        )
        .map_err(|error| format!("couldn't write the Linux launcher: {error}"))?;
        make_executable(&layout.binary)?;
        let desktop = layout.resources.join(format!("{name}.desktop"));
        std::fs::write(&desktop, desktop_entry(&name))
            .map_err(|error| format!("couldn't write the Linux desktop launcher: {error}"))?;
        make_executable(&desktop)?;
    }
    Ok(())
}

fn info_plist(project: &Project, executable: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"https://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
<key>CFBundleDisplayName</key><string>{}</string>\n\
<key>CFBundleExecutable</key><string>{}</string>\n\
<key>CFBundleIconFile</key><string>GameIcon</string>\n\
<key>CFBundleIdentifier</key><string>com.blockloom.game.{}</string>\n\
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>\n\
<key>CFBundleName</key><string>{}</string>\n\
<key>CFBundlePackageType</key><string>APPL</string>\n\
<key>CFBundleShortVersionString</key><string>1.0</string>\n\
<key>CFBundleVersion</key><string>1</string>\n\
<key>NSHighResolutionCapable</key><true/>\n\
</dict></plist>\n",
        xml(&project.name),
        xml(executable),
        xml(&project.id),
        xml(&project.name),
    )
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn desktop_entry(name: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=Built with Blockloom\n\
Exec=sh -c 'cd \"$(dirname \"$1\")\" && exec \"./$(basename \"${{1%.desktop}}\")\"' sh %k\n\
Icon=./{name}.png\nTerminal=false\nCategories=Game;\n"
    )
}

fn copy_logic(project_dir: &Path, game: &Path, target: &Target) -> Result<(), String> {
    let library = codegen::library_path_for(project_dir, script_target(target));
    if !library.is_file() {
        return Err(format!(
            "the blocks haven't been compiled for {}, so a fast build can't be made",
            target.label
        ));
    }
    let build = script::build_dir(game);
    std::fs::create_dir_all(&build).map_err(|error| format!("{}: {error}", build.display()))?;
    let file = library
        .file_name()
        .ok_or_else(|| format!("{} has no file name", library.display()))?;
    let target = build.join(file);
    std::fs::copy(&library, &target)
        .map_err(|error| format!("{} -> {}: {error}", library.display(), target.display()))?;
    make_executable(&target)
}

fn binary_name(name: &str, target: &Target) -> String {
    if target.windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Makes room for a fresh build. A folder that isn't a previous build is left
/// alone: this deletes things, and the only thing it may delete is its own
/// output.
fn clear_build_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    if !dir.join(BUILD_MARKER).is_file() && !pack::pack_path(&pack::game_dir(dir)).is_file() {
        return Err(format!(
            "{} is already there and isn't a built game, so it won't be replaced",
            dir.display()
        ));
    }
    std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Copies the project's `assets/` into the build, minus the script sources -
/// what a built game runs is the library beside them, not the `.rs` - and
/// minus the scene assets, which the pack already carries embedded.
fn copy_assets(project_dir: &Path, game: &Path) -> Result<usize, String> {
    let from = project_dir.join(project::ASSETS_DIR);
    if !from.is_dir() {
        return Ok(0);
    }
    let scripts = script::scripts_dir(project_dir);
    copy_tree(&from, &game.join(project::ASSETS_DIR), &|path| {
        !(path.starts_with(&scripts) && path.extension().is_some_and(|ext| ext == "rs"))
            // Scene assets ride in the pack, wherever the tray keeps them.
            && !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(project::SCENE_EXTENSION))
    })
}

/// Bakes the project's Image looks into one sheet the player draws them out
/// of. The files still ship, since a block or a material can name them too.
/// Sprites too big to share a sheet stay out, and if the rest overflow one
/// sheet the biggest are dropped until they fit; one sprite alone gains
/// nothing, so it bakes no atlas at all.
fn bake_sprite_atlas(project: &Project, project_dir: &Path, game: &Path) -> Result<usize, String> {
    use crate::pipeline;
    let mut sprites: Vec<(String, u64)> = Vec::new();
    for scene in &project.scenes {
        for actor in &scene.actors {
            let Some(crate::scene::Visual::Image { path, .. }) = actor.visual() else {
                continue;
            };
            let Some(relative) = crate::assets::normalize(path) else {
                continue;
            };
            if sprites.iter().any(|(known, _)| *known == relative) {
                continue;
            }
            // A file that won't decode is left to fail the way it does in Play.
            let Some(full) = crate::assets::resolve(project_dir, &relative) else {
                continue;
            };
            let Ok((width, height)) = image::image_dimensions(&full) else {
                continue;
            };
            if width.max(height) > pipeline::ATLAS_SPRITE_MAX {
                continue;
            }
            sprites.push((relative, width as u64 * height as u64));
        }
    }
    // Smallest first, so dropping from the end drops the biggest.
    sprites.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    while sprites.len() >= 2 {
        let paths: Vec<String> = sprites.iter().map(|(path, _)| path.clone()).collect();
        match pipeline::bake_atlas(project_dir, &paths, 2048, 2) {
            Ok(atlas) => {
                pipeline::write_atlas(
                    &atlas,
                    &game.join(pipeline::BAKED_ATLAS_IMAGE),
                    &game.join(pipeline::BAKED_ATLAS_LAYOUT),
                )?;
                return Ok(paths.len());
            }
            Err(_) => {
                sprites.pop();
            }
        }
    }
    Ok(0)
}

/// Bakes a 3D HDRI sky's file into a BC6H cube with its mip chain, which
/// the player loads as is, with its exposure bias and seam fix applied, and
/// drops the source from the build. A file that won't decode ships as it is
/// and fails the way it does in Play.
fn bake_sky(project: &Project, project_dir: &Path, game: &Path) -> Result<bool, String> {
    use crate::pipeline::{self, bc6h, hdr};
    // Every 3D scene with an HDRI sky bakes its own cube; scenes sharing one
    // file bake it once.
    let mut baked_any = false;
    let mut done: Vec<String> = Vec::new();
    for scene in &project.scenes {
        if scene.world.mode != crate::scene::Mode::ThreeD {
            continue;
        }
        let sky = &scene.world.sky;
        if sky.active_kind() != crate::sky::SkyKind::Hdri {
            continue;
        }
        let Some(relative) = crate::assets::normalize(&sky.hdri.path) else {
            continue;
        };
        if done.contains(&relative) {
            baked_any = true;
            continue;
        }
        done.push(relative.clone());
        let Ok(mut image) = hdr::load_hdr(project_dir, &relative) else {
            continue;
        };
        let manifest = pipeline::load_manifest(project_dir);
        image.bias(manifest.bias_of(&relative));
        image.fix_seam(sky.hdri.seam_fix);
        let cube = hdr::HdrCube::from_image(&image, (manifest.settings.hdr_max / 2).max(64));
        let out = game.join(pipeline::baked_sky_path(&relative));
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let bytes = bc6h::write_dds_cube_levels(&cube.mip_chain(4));
        crate::build_control::check()?;
        std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
        if let Some(copied) = crate::assets::resolve(game, &relative) {
            let _ = std::fs::remove_file(copied);
        }
        baked_any = true;
    }
    Ok(baked_any)
}

/// Copies the light probes' bakes, so a built game lights the way the
/// editor did. A probe that was never baked ships dark, as it plays.
fn copy_probes(project: &Project, project_dir: &Path, game: &Path) -> Result<(), String> {
    use crate::probe;
    for scene in &project.scenes {
        for actor in &scene.actors {
            if actor.components.probe().is_none() {
                continue;
            }
            for path in [
                probe::info_path(project_dir, &actor.id),
                probe::cube_path(project_dir, &actor.id),
                probe::grid_path(project_dir, &actor.id),
            ] {
                let Ok(relative) = path.strip_prefix(project_dir) else {
                    continue;
                };
                if !path.is_file() {
                    continue;
                }
                let to = game.join(relative);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                std::fs::copy(&path, &to)
                    .map_err(|e| format!("{} -> {}: {e}", path.display(), to.display()))?;
            }
        }
    }
    Ok(())
}

/// Copies the terrain grids the document names, so a built game stands on
/// the same ground the editor sculpted. Without them the runtime loads
/// nothing and falls back to flat.
fn copy_terrain(project: &Project, project_dir: &Path, game: &Path) -> Result<usize, String> {
    use crate::terrain::store;
    use std::collections::HashSet;
    let names = store::project_names(project);
    if names.is_empty() {
        return Ok(0);
    }
    let from = store::dir(project_dir);
    if !from.is_dir() {
        return Ok(0);
    }
    let to = store::dir(game);
    let mut copied: HashSet<String> = HashSet::new();
    let mut count = 0;
    for name in &names {
        let Ok(manifest) = store::read_manifest(project_dir, name) else {
            continue;
        };
        let files = [format!("{name}.json")]
            .into_iter()
            .chain(manifest.tiles.iter().map(|tile| format!("{tile}.tile")));
        for file in files {
            if !copied.insert(file.clone()) {
                continue;
            }
            let src = from.join(&file);
            if !src.is_file() {
                continue;
            }
            if count == 0 {
                std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
            }
            std::fs::copy(&src, to.join(&file))
                .map_err(|e| format!("{} -> {}: {e}", src.display(), to.join(&file).display()))?;
            count += 1;
        }
    }
    Ok(count)
}

/// Copies each scripted actor's library for this platform to where the
/// runtime looks for it. The build's own copy is flat, the way a project's
/// is: which platform it was built for stops mattering once it has shipped.
fn copy_scripts(
    project: &Project,
    project_dir: &Path,
    game: &Path,
    target: &Target,
) -> Result<usize, String> {
    let mut paths: Vec<&str> = project
        .scenes
        .iter()
        .flat_map(|scene| scene.actors.iter())
        .filter_map(|actor| actor.components.script())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    if paths.is_empty() {
        return Ok(0);
    }

    let build = script::build_dir(game);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;
    for relative in &paths {
        let library = script::library_path_for(project_dir, relative, script_target(target));
        if !library.is_file() {
            return Err(format!(
                "{relative} hasn't been built for {}, so the game would ship without it",
                target.label
            ));
        }
        let Some(file) = library.file_name() else {
            continue;
        };
        let to = build.join(file);
        std::fs::copy(&library, &to)
            .map_err(|e| format!("{} -> {}: {e}", library.display(), to.display()))?;
        make_executable(&to)?;
    }
    Ok(paths.len())
}

/// Copies a folder recursively, keeping the files `wanted` says yes to.
/// Returns how many files were copied.
fn copy_tree(from: &Path, to: &Path, wanted: &dyn Fn(&Path) -> bool) -> Result<usize, String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let mut copied = 0;
    for entry in entries {
        crate::build_control::check()?;
        let entry = entry.map_err(|e| format!("{}: {e}", from.display()))?;
        let path = entry.path();
        if !wanted(&path) {
            continue;
        }
        let Some(file) = path.file_name() else {
            continue;
        };
        let target = to.join(file);
        if path.is_dir() {
            copied += copy_tree(&path, &target, wanted)?;
        } else {
            std::fs::copy(&path, &target)
                .map_err(|e| format!("{} -> {}: {e}", path.display(), target.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Mode;

    #[test]
    fn a_shipped_plugin_carries_its_manifest_beside_its_files() {
        let root = temp("plugin-src");
        let game = temp("plugin-game");
        std::fs::create_dir_all(root.join("schemas")).unwrap();
        std::fs::write(root.join("plugin.json"), "{}").unwrap();
        std::fs::write(root.join("schemas/a.json"), "{}").unwrap();
        let payload = PluginPayload {
            entry: pack::PackedPlugin {
                id: "com.example.a".to_string(),
                version: "1.0.0".to_string(),
                hash: "h".to_string(),
                tier: "portable".to_string(),
                dir: "plugins/com.example.a".to_string(),
                files: vec!["schemas/a.json".to_string()],
            },
            root,
        };
        copy_plugins(&[payload], &game).unwrap();
        let there = game.join("plugins/com.example.a");
        assert!(
            there.join("plugin.json").is_file(),
            "the player verifies against it"
        );
        assert!(there.join("schemas/a.json").is_file());
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "blockloom-build-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A project folder with one asset, one script source, and a stand-in for
    /// the player binary.
    fn a_project(root: &Path) -> (Project, PathBuf, PathBuf) {
        let project_dir = root.join("project");
        std::fs::create_dir_all(project_dir.join("assets/sprites")).unwrap();
        std::fs::create_dir_all(script::scripts_dir(&project_dir)).unwrap();
        std::fs::write(project_dir.join("assets/sprites/ball.png"), b"png").unwrap();
        std::fs::write(project_dir.join("assets/scripts/player.rs"), b"// rust").unwrap();

        let player = root.join(if cfg!(windows) {
            "player-binary.exe"
        } else {
            "player-binary"
        });
        std::fs::copy(std::env::current_exe().unwrap(), &player).unwrap();

        (
            Project::starter("Pond Game", Mode::TwoD),
            project_dir,
            player,
        )
    }

    /// The platform under test: this machine, so the paths a build reads from
    /// are the ones Play writes to.
    fn a_target() -> &'static Target {
        host().expect("the tests run on a platform Blockloom knows")
    }

    #[test]
    fn weak_targets_build_sdr_unless_told_otherwise() {
        for target in TARGETS {
            let (hdr, note) = target.hdr_default();
            assert_eq!(
                hdr,
                target.triple != "aarch64-unknown-linux-gnu"
                    && !target.is_web()
                    && !target.is_android(),
                "{}",
                target.triple
            );
            assert!(!note.is_empty());
        }
    }

    #[test]
    fn a_build_is_a_binary_beside_a_pack_and_the_projects_assets() {
        let root = temp("layout");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");
        let target = a_target();

        let built = build(
            &project,
            &project_dir,
            target,
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap();

        assert_eq!(built.dir, out.join(format!("Pond Game ({})", target.label)));
        assert!(built.binary.is_file());
        assert!(built.archive.is_file());
        assert_eq!(&std::fs::read(&built.archive).unwrap()[..2], b"PK");
        assert!(
            built
                .binary
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Pond Game")
        );
        let game = pack::game_dir(&built.dir);
        assert!(pack::pack_path(&game).is_file());
        assert!(game.join("assets/sprites/ball.png").is_file());
        assert_eq!(built.assets, 1);
        // The source stays behind; a build runs the library, not the `.rs`.
        assert!(!game.join("assets/scripts/player.rs").exists());
        // No DLSS DLLs staged, so none ride along - and that is fine.
        assert!(!built.dlss);

        let pack = GamePack::read(&pack::pack_path(&game)).unwrap();
        assert_eq!(pack.title(), "Pond Game");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_build_carries_dlss_dlls_staged_beside_its_player() {
        let root = temp("dlss");
        let (project, project_dir, player) = a_project(&root);
        // What `just player-dlss` stages beside the payload: the SDK DLLs
        // plus the license blurb the SDK asks shippers to include.
        let dll = if cfg!(windows) {
            "nvngx_dlss.dll"
        } else {
            "libnvidia-ngx-dlss.so.310.9.1"
        };
        std::fs::write(player.parent().unwrap().join(dll), b"dlss").unwrap();
        std::fs::write(
            player.parent().unwrap().join("DLSS_LICENSE.txt"),
            b"license",
        )
        .unwrap();
        // Frame generation rides in the same SDK folder but stays behind:
        // nothing here uses it.
        let fg = if cfg!(windows) {
            "nvngx_dlssg.dll"
        } else {
            "libnvidia-ngx-dlssg.so.310.9.1"
        };
        std::fs::write(player.parent().unwrap().join(fg), b"fg").unwrap();
        let out = root.join("out");
        let target = a_target();

        let built = build(
            &project,
            &project_dir,
            target,
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap();

        assert!(built.dlss);
        // The DLLs sit beside the built player - which for Linux is behind
        // the wrapper, but in the same folder as the binary either way
        // (and inside Contents/MacOS on macOS).
        let player_dir = built.binary.parent().unwrap();
        assert!(player_dir.join(dll).is_file());
        assert!(player_dir.join("DLSS_LICENSE.txt").is_file());
        assert!(!player_dir.join(fg).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_web_build_is_one_page_carrying_the_player_and_the_game() {
        let root = temp("web");
        let (project, project_dir, _) = a_project(&root);
        let player = root.join(crate::web_build::PLAYER_WASM);
        std::fs::write(&player, b"\0asm-player").unwrap();
        std::fs::write(
            root.join(crate::web_build::PLAYER_GLUE),
            b"export default 1",
        )
        .unwrap();
        let target = target("wasm32-unknown-unknown").unwrap();
        let out = root.join("out");

        let built = build(
            &project,
            &project_dir,
            target,
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap();

        assert_eq!(built.dir, out.join("Pond Game (Web)"));
        assert_eq!(built.binary, built.dir.join("Pond Game.html"));
        assert!(built.archive.is_file());
        assert!(!built.compiled);
        assert_eq!(built.size, std::fs::metadata(&built.binary).unwrap().len());
        // Only the page ships; the scratch game folder is gone.
        assert!(!built.dir.join(".game").exists());

        let html = std::fs::read_to_string(&built.binary).unwrap();
        let open = "id=\"blockloom-data\">";
        let start = html.find(open).unwrap() + open.len();
        let end = start + html[start..].find("</script>").unwrap();
        use base64::Engine;
        let gzipped = base64::engine::general_purpose::STANDARD
            .decode(&html[start..end])
            .unwrap();
        let entries = crate::web_build::unarchive(&gzipped);
        let find = |name: &str| entries.iter().find(|(n, _)| n == name).map(|(_, b)| b);
        assert_eq!(find("player.wasm").unwrap(), b"\0asm-player");
        assert_eq!(find("game/assets/sprites/ball.png").unwrap(), b"png");
        assert!(find("game/assets/scripts/player.rs").is_none());
        let pack = GamePack::from_json(
            std::str::from_utf8(find("game/game.pack").unwrap()).unwrap(),
            "game.pack",
        )
        .unwrap();
        // A browser has no HDR output.
        assert!(!pack.hdr);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shader_that_wont_compile_stops_the_build() {
        use crate::components::ActorComponent;
        use crate::material::{GraphEffect, SurfaceMaterial};
        let root = temp("shaders");
        let (mut project, project_dir, player) = a_project(&root);
        std::fs::create_dir_all(project_dir.join("assets/shaders")).unwrap();
        std::fs::write(
            project_dir.join("assets/shaders/bad.wesl"),
            "fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> { return nope; }",
        )
        .unwrap();
        let material = SurfaceMaterial {
            shader: Some(GraphEffect {
                source: "assets/shaders/bad.wesl".into(),
                ..GraphEffect::default()
            }),
            ..SurfaceMaterial::default()
        };
        project.actors[0]
            .components
            .insert(ActorComponent::Material { material });
        let out = root.join("out");

        let error = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap_err();
        assert!(error.contains("assets/shaders/bad.wesl"), "{error}");
        assert!(!out.exists());

        std::fs::write(
            project_dir.join("assets/shaders/bad.wesl"),
            "fn graph_main(uv: vec2<f32>, time: f32) -> vec4<f32> { return tint; }",
        )
        .unwrap();
        let built = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap();
        assert_eq!(built.shaders, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_build_bakes_its_image_looks_into_one_sheet() {
        use crate::scene::Visual;
        let root = temp("atlas");
        let (mut project, project_dir, player) = a_project(&root);
        for (name, size) in [("a", 8), ("b", 16), ("huge", 600)] {
            image::RgbaImage::new(size, size)
                .save(project_dir.join(format!("assets/sprites/{name}.png")))
                .unwrap();
            project.actors.push(crate::project::Actor::new(
                name,
                Visual::Image {
                    path: format!("assets/sprites/{name}.png"),
                    size: [32.0, 32.0],
                },
            ));
        }

        let built = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &root.join("out"),
            BuildOptions::default(),
        )
        .unwrap();

        let game = pack::game_dir(&built.dir);
        assert!(game.join(crate::pipeline::BAKED_ATLAS_IMAGE).is_file());
        let layout = crate::pipeline::read_baked_layout(&game).unwrap();
        assert_eq!(built.atlas, 2);
        assert!(layout.entry("assets/sprites/a.png").is_some());
        assert!(layout.entry("assets/sprites/b.png").is_some());
        // Too big to share a sheet: it keeps its own texture.
        assert!(layout.entry("assets/sprites/huge.png").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_3d_sky_ships_as_a_bc6h_cube_and_an_sdr_build_says_so() {
        let root = temp("sky");
        let (mut project, project_dir, player) = a_project(&root);
        project.world.mode = Mode::ThreeD;
        project.world.sky.kind = crate::sky::SkyKind::Hdri;
        project.world.sky.hdri.path = "assets/sky.hdr".to_string();
        let mut bytes = Vec::new();
        image::codecs::hdr::HdrEncoder::new(&mut bytes)
            .encode(&vec![image::Rgb([4.0f32, 2.0, 1.0]); 64 * 32], 64, 32)
            .unwrap();
        std::fs::write(project_dir.join("assets/sky.hdr"), bytes).unwrap();

        let options = BuildOptions {
            fast: false,
            sdr_only: true,
            ..BuildOptions::default()
        };
        let built = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &root.join("out"),
            options,
        )
        .unwrap();

        assert!(built.sky);
        let game = pack::game_dir(&built.dir);
        let baked =
            std::fs::read(game.join(crate::pipeline::baked_sky_path("assets/sky.hdr"))).unwrap();
        let cube = crate::pipeline::bc6h::read_dds_cube_levels(&baked).unwrap();
        // 16 down to one 4x4 block.
        assert_eq!((cube.size, cube.mips), (16, 3));
        assert!(!game.join("assets/sky.hdr").exists());
        assert!(!GamePack::read(&pack::pack_path(&game)).unwrap().hdr);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn building_twice_replaces_the_first_build() {
        let root = temp("replace");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");

        let first = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap();
        std::fs::write(first.dir.join("leftover.txt"), b"old").unwrap();
        let second = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap();

        assert_eq!(first.dir, second.dir);
        assert!(!second.dir.join("leftover.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_fast_build_carries_the_native_block_library() {
        let root = temp("native-logic");
        let (project, project_dir, player) = a_project(&root);
        let library = codegen::library_path(&project_dir);
        std::fs::create_dir_all(library.parent().unwrap()).unwrap();
        std::fs::write(&library, b"native logic").unwrap();

        let built = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &root.join("out"),
            BuildOptions {
                fast: true,
                ..BuildOptions::default()
            },
        )
        .unwrap();

        assert!(built.compiled);
        assert!(
            codegen::library_path(&pack::game_dir(&built.dir)).is_file(),
            "the player must find native logic in its normal build folder"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_build_carries_the_terrain_it_stands_on() {
        use crate::terrain::{Heightfield, store};
        let root = temp("terrain");
        let (mut project, project_dir, player) = a_project(&root);
        let mut field = Heightfield::flat(129, 0.0);
        field.samples[64 * 129 + 64] = 1.0;
        let grid = store::Grid::from_heights(&field);
        let name = store::save(&project_dir, &grid).unwrap();
        let spec = crate::terrain::TerrainSpec {
            resolution: 129,
            heights: name.clone(),
            ..Default::default()
        };
        project.actors[0]
            .components
            .insert(crate::components::ActorComponent::Terrain { terrain: spec });

        let built = build(
            &project,
            &project_dir,
            a_target(),
            &player,
            &root.join("out"),
            BuildOptions::default(),
        )
        .unwrap();

        let game = pack::game_dir(&built.dir);
        assert!(
            store::dir(&game).join(format!("{name}.json")).is_file(),
            "the built game must ship the heights manifest"
        );
        let back = store::heights_for(Some(&game), project.actors[0].components.terrain().unwrap());
        assert!(
            back.samples.iter().any(|s| *s > 0.5),
            "the shipped heights must still hold the sculpted hill"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn macos_is_a_real_app_bundle() {
        let root = temp("mac-app");
        let (project, project_dir, player) = a_project(&root);
        let target = target("aarch64-apple-darwin").unwrap();

        let built = build(
            &project,
            &project_dir,
            target,
            &player,
            &root.join("out"),
            BuildOptions::default(),
        )
        .unwrap();

        let contents = built.dir.join("Pond Game.app/Contents");
        assert_eq!(built.binary, contents.join("MacOS/Pond Game"));
        assert!(contents.join("Info.plist").is_file());
        assert_eq!(
            std::fs::read(contents.join("PkgInfo")).unwrap(),
            b"APPL????"
        );
        assert!(contents.join("Resources/GameIcon.icns").is_file());
        assert!(
            pack::pack_path(&contents.join("Resources/game")).is_file(),
            "the app keeps game data in Resources"
        );
        assert!(built.archive.is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn linux_gets_a_portable_launcher_and_desktop_entry() {
        let root = temp("linux-wrapper");
        let (project, project_dir, player) = a_project(&root);
        let target = target("x86_64-unknown-linux-gnu").unwrap();

        let built = build(
            &project,
            &project_dir,
            target,
            &player,
            &root.join("out"),
            BuildOptions::default(),
        )
        .unwrap();

        assert!(built.binary.is_file());
        assert!(built.dir.join(".blockloom-player").is_file());
        assert!(built.dir.join("Pond Game.desktop").is_file());
        assert!(built.dir.join("Pond Game.png").is_file());
        let launcher = std::fs::read_to_string(&built.binary).unwrap();
        assert!(launcher.contains(".blockloom-player"), "{launcher}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_that_isnt_a_build_is_never_replaced() {
        let root = temp("stranger");
        let (project, project_dir, player) = a_project(&root);
        let out = root.join("out");
        let target = a_target();
        let taken = out.join(build_name(&project, target));
        std::fs::create_dir_all(&taken).unwrap();
        std::fs::write(taken.join("taxes.txt"), b"mine").unwrap();

        let error = build(
            &project,
            &project_dir,
            target,
            &player,
            &out,
            BuildOptions::default(),
        )
        .unwrap_err();

        assert!(error.contains("isn't a built game"), "{error}");
        assert!(taken.join("taxes.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn each_platform_gets_a_build_folder_of_its_own() {
        let project = Project::starter("Pond Game", Mode::TwoD);
        let names: Vec<String> = TARGETS
            .iter()
            .map(|target| build_name(&project, target))
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "{names:?}");
    }

    #[test]
    fn a_platform_with_no_player_staged_for_it_is_not_ready() {
        let root = temp("targets");
        let fallback = root.join("blockloom-runtime");
        std::fs::write(&fallback, b"MZ").unwrap();

        let statuses = targets(false, Ok(()), &fallback);
        let host = statuses.first().expect("at least one target");

        // This machine comes first and can always build, since the runtime the
        // editor plays with is a player.
        assert!(host.host);
        assert!(host.ready);
        // Nothing is staged in this test's folder, so no other desktop
        // platform is. Android rows never mention a staged player: they
        // report the toolchain instead (checked below).
        for status in statuses
            .iter()
            .filter(|status| !status.host && !android::is_android(&status.triple))
        {
            assert!(!status.ready, "{status:?}");
            assert!(status.note.contains("players/"), "{status:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn android_rows_report_the_toolchain_not_a_staged_player() {
        let root = temp("android-targets");
        let fallback = root.join("blockloom-runtime");
        std::fs::write(&fallback, b"MZ").unwrap();

        let statuses = targets(false, Ok(()), &fallback);
        let android: Vec<_> = statuses
            .iter()
            .filter(|status| android::is_android(&status.triple))
            .collect();
        assert_eq!(android.len(), 2, "{statuses:?}");
        for status in android {
            assert!(!status.host);
            assert!(!status.note.contains("players/"), "{status:?}");
            assert!(!status.hdr, "{status:?}");
            assert!(!status.hdr_note.is_empty());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_android_build_without_a_toolchain_says_so() {
        let root = temp("android-toolchain");
        let (project, project_dir, player) = a_project(&root);
        let target = target(android::ARM64_TRIPLE).unwrap();
        // An empty SDK row is never ready, on any machine.
        let config = android::AppConfig {
            sdk_path: Some(root.join("no-sdk-here")),
            ndk_path: None,
            licenses_accepted: false,
        };
        let error = build_android_with_config(
            &project,
            &project_dir,
            target,
            &player,
            &root.join("out"),
            BuildOptions::default(),
            &config,
        )
        .unwrap_err();
        assert!(
            error.contains("Can't build for Android (arm64) yet"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn android_native_libs_start_with_the_runtime_then_the_scripts() {
        let root = temp("android-libs");
        let (mut project, project_dir, _) = a_project(&root);
        let target = target(android::ARM64_TRIPLE).unwrap();
        let runtime = root.join("libblockloom_runtime.so");
        std::fs::write(&runtime, b"fake-so").unwrap();

        // No scripts: the runtime rides alone.
        let libs = android_native_libs(&project, &project_dir, target, &runtime, false).unwrap();
        assert_eq!(libs, vec![("arm64-v8a".to_string(), runtime.clone())]);

        // A script nobody compiled for the target stops the build.
        project.scenes[0].actors[0]
            .components
            .insert(crate::components::ActorComponent::Script {
                path: "assets/scripts/player.rs".to_string(),
            });
        let error =
            android_native_libs(&project, &project_dir, target, &runtime, false).unwrap_err();
        assert!(
            error.contains("hasn't been built for Android (arm64)"),
            "{error}"
        );

        // Compiled (a stand-in file at the cross-build path), it rides along.
        let library = crate::script::library_path_for(
            &project_dir,
            "assets/scripts/player.rs",
            Some(target.triple),
        );
        std::fs::create_dir_all(library.parent().unwrap()).unwrap();
        std::fs::write(&library, b"fake-script").unwrap();
        let libs = android_native_libs(&project, &project_dir, target, &runtime, false).unwrap();
        assert_eq!(
            libs,
            vec![
                ("arm64-v8a".to_string(), runtime.clone()),
                ("arm64-v8a".to_string(), library),
            ]
        );

        // Fast asks for the logic library too.
        let error =
            android_native_libs(&project, &project_dir, target, &runtime, true).unwrap_err();
        assert!(error.contains("haven't been compiled"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A fabricated SDK tree with stub tools: aapt2 `compile` touches its
    /// `-o`, `link` writes an empty zip there, zipalign/apksigner copy
    /// input to output. Unix-only, like the assembly test it exercises.
    #[cfg(unix)]
    fn stub_apk_tools(root: &Path) -> android::AppConfig {
        use std::os::unix::fs::PermissionsExt;
        let sdk = root.join("sdk");
        let bin = sdk.join("build-tools").join(android::BUILD_TOOLS);
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(sdk.join("cmdline-tools/latest/bin")).unwrap();
        std::fs::write(sdk.join("cmdline-tools/latest/bin/sdkmanager"), b"fake").unwrap();
        std::fs::create_dir_all(sdk.join("platforms").join(android::PLATFORM)).unwrap();
        std::fs::write(
            sdk.join("platforms")
                .join(android::PLATFORM)
                .join("android.jar"),
            b"fake",
        )
        .unwrap();
        std::fs::create_dir_all(sdk.join("platform-tools")).unwrap();
        std::fs::write(sdk.join("platform-tools").join("adb"), b"fake").unwrap();
        let aapt2_body = "#!/bin/sh\nout=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"-o\" ]; then out=\"$a\"; fi\n  prev=\"$a\"\ndone\nif [ \"$1\" = \"link\" ]; then\n  printf '\\120\\113\\005\\006\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000' > \"$out\"\nelse\n  touch \"$out\"\nfi\n";
        let aapt2 = bin.join("aapt2");
        std::fs::write(&aapt2, aapt2_body).unwrap();
        std::fs::set_permissions(&aapt2, std::fs::Permissions::from_mode(0o755)).unwrap();
        let zipalign = bin.join("zipalign");
        std::fs::write(&zipalign, "#!/bin/sh\ncp \"$3\" \"$4\"\n").unwrap();
        std::fs::set_permissions(&zipalign, std::fs::Permissions::from_mode(0o755)).unwrap();
        let apksigner = bin.join("apksigner");
        std::fs::write(
            &apksigner,
            "#!/bin/sh\nout=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"--out\" ]; then out=\"$a\"; fi\n  prev=\"$a\"\ndone\nlast=\"\"\nfor a in \"$@\"; do last=\"$a\"; done\ncp \"$last\" \"$out\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&apksigner, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A matching-major NDK with a linker wrapper for the probes.
        let ndk_bin = sdk
            .join("ndk/27.0.0/toolchains/llvm/prebuilt")
            .join(android::host_tag())
            .join("bin");
        std::fs::create_dir_all(&ndk_bin).unwrap();
        for name in [
            format!("{}{}-clang", android::ARM64_TRIPLE, android::MIN_SDK),
            "llvm-ar".to_string(),
        ] {
            std::fs::write(ndk_bin.join(name), b"fake").unwrap();
        }
        std::fs::write(
            sdk.join("ndk/27.0.0/source.properties"),
            b"Pkg.Revision = 27.0.0\n",
        )
        .unwrap();
        android::AppConfig {
            sdk_path: Some(sdk),
            ndk_path: None,
            licenses_accepted: true,
        }
    }

    #[test]
    #[cfg(unix)]
    fn an_android_build_stages_the_game_and_assembles_an_apk() {
        if !android::jdk_status().ok
            || !android::rust_target_status(android::ARM64_TRIPLE).ok
            || android::keytool().is_err()
        {
            eprintln!("SKIP: needs JDK 25, the Android Rust std and keytool");
            return;
        }
        let root = temp("android-apk");
        let (project, project_dir, _) = a_project(&root);
        let target = target(android::ARM64_TRIPLE).unwrap();
        let config = stub_apk_tools(&root);
        let runtime = root.join("libblockloom_runtime.so");
        std::fs::write(&runtime, b"fake-so").unwrap();

        let built = build_android_with_config(
            &project,
            &project_dir,
            target,
            &runtime,
            &root.join("out"),
            BuildOptions::default(),
            &config,
        )
        .unwrap();

        assert!(built.binary.is_file());
        assert_eq!(built.binary.extension().unwrap().to_string_lossy(), "apk");
        assert!(built.size > 0);
        assert!(built.archive.is_file());
        assert!(!built.compiled);
        assert!(!built.cached);
        // The staging is real even though the stub link packed nothing:
        // the game folder sits under the build dir as the APK's assets.
        let staged = built.dir.join("assets");
        assert!(pack::pack_path(&staged).is_file());
        assert!(staged.join("assets/sprites/ball.png").is_file());
        let pack = GamePack::read(&pack::pack_path(&staged)).unwrap();
        assert_eq!(pack.title(), "Pond Game");
        assert!(!pack.hdr);
        // The stub link wrote an empty zip; the real injection added the .so.
        let file = std::fs::File::open(&built.binary).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(
            archive
                .by_name("lib/arm64-v8a/libblockloom_runtime.so")
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn an_android_build_reuses_the_apk_when_nothing_changed() {
        if !android::jdk_status().ok
            || !android::rust_target_status(android::ARM64_TRIPLE).ok
            || android::keytool().is_err()
        {
            eprintln!("SKIP: needs JDK 25, the Android Rust std and keytool");
            return;
        }
        let root = temp("android-cache");
        let (project, project_dir, _) = a_project(&root);
        let target = target(android::ARM64_TRIPLE).unwrap();
        let config = stub_apk_tools(&root);
        let runtime = root.join("libblockloom_runtime.so");
        std::fs::write(&runtime, b"fake-so").unwrap();
        let out = root.join("out");

        let first = build_android_with_config(
            &project,
            &project_dir,
            target,
            &runtime,
            &out,
            BuildOptions::default(),
            &config,
        )
        .unwrap();
        assert!(!first.cached);
        // A leftover proves the second build did not clear the dir.
        std::fs::write(first.dir.join("leftover.txt"), b"old").unwrap();

        let second = build_android_with_config(
            &project,
            &project_dir,
            target,
            &runtime,
            &out,
            BuildOptions::default(),
            &config,
        )
        .unwrap();
        assert!(second.cached);
        assert_eq!(second.binary, first.binary);
        assert!(second.dir.join("leftover.txt").is_file());

        // A new asset busts the fingerprint, so the next build rebuilds
        // and clears the leftover.
        std::fs::write(project_dir.join("assets/sprites/new.png"), b"png").unwrap();
        let third = build_android_with_config(
            &project,
            &project_dir,
            target,
            &runtime,
            &out,
            BuildOptions::default(),
            &config,
        )
        .unwrap();
        assert!(!third.cached);
        assert!(!third.dir.join("leftover.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
