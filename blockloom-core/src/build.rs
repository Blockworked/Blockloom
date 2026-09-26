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

use crate::codegen;
use crate::distribution;
use crate::pack::{self, GamePack};
use crate::project::{self, Project};
use crate::script;
use serde::{Deserialize, Serialize};
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

    /// Whether a build for it renders HDR unless told otherwise, and why.
    /// ARM64 Linux is mostly single-board computers, where FP16 targets cost
    /// more than they give and HDR displays are rare.
    pub fn hdr_default(self) -> (bool, &'static str) {
        if self.is_web() {
            (false, "SDR only: browsers give a page no HDR output yet.")
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

fn status(
    target: &Target,
    has_scripts: bool,
    fast_source: &Result<(), String>,
    fallback_player: &Path,
) -> TargetStatus {
    let host = is_host(target);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BuildOptions {
    /// Ship the blocks as one native library as well as the document.
    pub fast: bool,
    /// Clamp the player to an 8-bit SDR frame, for targets too weak for
    /// FP16 targets and HDR output.
    pub sdr_only: bool,
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
    /// Bytes of what ships: the ZIP, or for the web the one `.html`.
    pub size: u64,
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
    if target.is_web() {
        return build_web(project, project_dir, target, player, parent);
    }
    let fast = options.fast;
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

    let game = layout.game.clone();
    std::fs::create_dir_all(&game).map_err(|e| format!("{}: {e}", game.display()))?;
    let mut game_pack = GamePack::new(project.clone());
    game_pack.hdr = !options.sdr_only;
    game_pack.write(&pack::pack_path(&game))?;

    let assets = copy_assets(project_dir, &game)?;
    let atlas = bake_sprite_atlas(project, project_dir, &game)?;
    let sky = bake_sky(project, project_dir, &game)?;
    copy_probes(project, project_dir, &game)?;
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
    })
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
    let assets = copy_assets(project_dir, &game)?;
    let atlas = bake_sprite_atlas(project, project_dir, &game)?;
    let sky = bake_sky(project, project_dir, &game)?;
    copy_probes(project, project_dir, &game)?;
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
    })
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
    let mut sources: Vec<&str> = project
        .actors
        .iter()
        .filter_map(|actor| actor.components.material()?.shader.as_ref())
        .map(|effect| effect.source.trim())
        .filter(|source| !source.is_empty())
        .collect();
    sources.sort_unstable();
    sources.dedup();
    let dim3 = project.world.mode.is_3d();
    let mut errors = Vec::new();
    for source in &sources {
        let verdict = crate::assets::resolve(project_dir, source)
            .ok_or_else(|| "isn't a path in this project".to_string())
            .and_then(|full| {
                std::fs::read_to_string(&full).map_err(|error| format!("couldn't be read: {error}"))
            })
            .and_then(|text| crate::material::check_surface_wesl(&text, dim3));
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
/// what a built game runs is the library beside them, not the `.rs`.
fn copy_assets(project_dir: &Path, game: &Path) -> Result<usize, String> {
    let from = project_dir.join(project::ASSETS_DIR);
    if !from.is_dir() {
        return Ok(0);
    }
    let scripts = script::scripts_dir(project_dir);
    copy_tree(&from, &game.join(project::ASSETS_DIR), &|path| {
        !(path.starts_with(&scripts) && path.extension().is_some_and(|ext| ext == "rs"))
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
    for actor in &project.actors {
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
    let sky = &project.world.sky;
    if project.world.mode != crate::scene::Mode::ThreeD
        || sky.active_kind() != crate::sky::SkyKind::Hdri
    {
        return Ok(false);
    }
    let Some(relative) = crate::assets::normalize(&sky.hdri.path) else {
        return Ok(false);
    };
    let Ok(mut image) = hdr::load_hdr(project_dir, &relative) else {
        return Ok(false);
    };
    let manifest = pipeline::load_manifest(project_dir);
    image.bias(manifest.bias_of(&relative));
    image.fix_seam(sky.hdri.seam_fix);
    let cube = hdr::HdrCube::from_image(&image, (manifest.settings.hdr_max / 2).max(64));
    let out = game.join(pipeline::baked_sky_path(&relative));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&out, bc6h::write_dds_cube_levels(&cube.mip_chain(4)))
        .map_err(|e| format!("{}: {e}", out.display()))?;
    if let Some(copied) = crate::assets::resolve(game, &relative) {
        let _ = std::fs::remove_file(copied);
    }
    Ok(true)
}

/// Copies the light probes' bakes, so a built game lights the way the
/// editor did. A probe that was never baked ships dark, as it plays.
fn copy_probes(project: &Project, project_dir: &Path, game: &Path) -> Result<(), String> {
    use crate::probe;
    for actor in &project.actors {
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
    Ok(())
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
        .actors
        .iter()
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
    fn arm64_linux_builds_sdr_unless_told_otherwise() {
        for target in TARGETS {
            let (hdr, note) = target.hdr_default();
            assert_eq!(
                hdr,
                target.triple != "aarch64-unknown-linux-gnu" && !target.is_web(),
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

        let pack = GamePack::read(&pack::pack_path(&game)).unwrap();
        assert_eq!(pack.title(), "Pond Game");
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
        // Nothing is staged in this test's folder, so no other platform is.
        for status in statuses.iter().filter(|status| !status.host) {
            assert!(!status.ready, "{status:?}");
            assert!(status.note.contains("players/"), "{status:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
