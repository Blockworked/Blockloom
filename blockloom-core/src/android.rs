//! Android builds from a desktop PC (Phase 6.5, player only).
//!
//! A project builds into an installable APK and runs standalone on a phone
//! or tablet. There is no Android editor and no Play path to a device: Qt
//! stays desktop-only and Android is a Build dialog row.
//!
//! v1 scope: arm64 devices (`aarch64-linux-android`) first, the x86_64
//! emulator (`x86_64-linux-android`) second, APK only, no 32-bit targets,
//! SDR only, no ray tracing. This module holds the pieces that need no
//! device and no Gradle run: the pinned toolchain versions, the app-level
//! config (SDK/NDK paths plus the license stamp, beside `projects.json`),
//! the status probes the Settings dialog and `just android-check` report,
//! and the `applicationId` rules. APK assembly and signing come later.

use crate::{project, script};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// arm64 phones and tablets, the v1 device target.
pub const ARM64_TRIPLE: &str = "aarch64-linux-android";
/// x86_64 emulator image, for the dev loop.
pub const EMULATOR_TRIPLE: &str = "x86_64-linux-android";

/// The one platform `sdkmanager` installs.
pub const PLATFORM: &str = "android-35";
/// The build-tools line matching that platform.
pub const BUILD_TOOLS: &str = "35.0.0";
/// The NDK major Blockloom pins. Patch bumps are fine; a new major is a
/// deliberate change, since the linker path below moves with it.
pub const NDK_MAJOR: &str = "27";
/// The lowest API a v1 APK runs on.
pub const MIN_SDK: u32 = 29;
/// The API a v1 APK targets, beside the platform pin.
pub const TARGET_SDK: u32 = 35;
/// JDK 25 is the latest LTS, and the Gradle line the APK template pins
/// needs it. Installing one silently is not v1, so a missing JDK reports
/// where to get one instead.
pub const JDK_MAJOR: u32 = 25;

/// Whether `triple` is one of the Android targets.
pub fn is_android(triple: &str) -> bool {
    triple == ARM64_TRIPLE || triple == EMULATOR_TRIPLE
}

/// The ABI folder an Android `.so` ships under inside the APK.
pub fn abi(triple: &str) -> Option<&'static str> {
    match triple {
        ARM64_TRIPLE => Some("arm64-v8a"),
        EMULATOR_TRIPLE => Some("x86_64"),
        _ => None,
    }
}

// ─── App config ──────────────────────────────────────────────────────────
// Paths are settings, never env lookups: Blockloom does not read
// `ANDROID_HOME`, `ANDROID_SDK_ROOT` or `ANDROID_NDK_HOME`.

/// What the App Settings dialog keeps: where the SDK and NDK live and
/// whether the user accepted their licenses. Keystore passwords never land
/// here; release signing asks on each build.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ndk_path: Option<PathBuf>,
    /// Set only through the licenses prompt, never by piping yes.
    #[serde(default)]
    pub licenses_accepted: bool,
}

fn config_path() -> PathBuf {
    project::data_dir().join("android.json")
}

fn load_from(path: &Path) -> AppConfig {
    let Ok(text) = std::fs::read_to_string(path) else {
        return AppConfig::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(
            "Ignoring an unreadable Android config ({}): {error}",
            path.display()
        );
        AppConfig::default()
    })
}

fn save_to(config: &AppConfig, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// The saved config, or defaults when nothing was saved yet.
pub fn load() -> AppConfig {
    load_from(&config_path())
}

/// Writes the config back to the data dir.
pub fn save(config: &AppConfig) -> Result<(), String> {
    save_to(config, &config_path())
}

/// `~/Blockloom/android-sdk`, or under `BLOCKLOOM_DATA_DIR` when set, so a
/// test run never touches the real one.
pub fn default_sdk_dir() -> PathBuf {
    if std::env::var_os("BLOCKLOOM_DATA_DIR").is_some() {
        return project::data_dir().join("android-sdk");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Blockloom")
        .join("android-sdk")
}

/// The SDK row, or the default when the user never pointed it elsewhere.
pub fn sdk_dir(config: &AppConfig) -> PathBuf {
    config.sdk_path.clone().unwrap_or_else(default_sdk_dir)
}

/// The NDK row, or the pinned NDK inside the SDK when unset.
pub fn ndk_dir(config: &AppConfig) -> PathBuf {
    config.ndk_path.clone().unwrap_or_else(|| {
        // The exact patch dir varies per install; prefer a matching major.
        let root = sdk_dir(config).join("ndk");
        if let Ok(entries) = std::fs::read_dir(&root) {
            let mut found: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(NDK_MAJOR))
                })
                .collect();
            found.sort();
            if let Some(newest) = found.pop() {
                return newest;
            }
        }
        root.join(format!("{NDK_MAJOR}.0.0"))
    })
}

// ─── applicationId ───────────────────────────────────────────────────────

/// The Play-store-style id for a project: `com.blockloom.game.<name>` with
/// anything outside `[a-z0-9_]` turned into underscores, lowercased.
pub fn sanitize_application_id(project_name: &str) -> String {
    let mut stem: String = project_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while stem.starts_with('_') {
        stem.remove(0);
    }
    if stem.is_empty() || !stem.starts_with(|c: char| c.is_ascii_alphabetic()) {
        stem.insert(0, 'g');
    }
    format!("com.blockloom.game.{stem}")
}

/// An id is segments of letters, digits and underscores, starting with a
/// letter, at least two segments deep.
pub fn validate_application_id(id: &str) -> Result<(), String> {
    let segments: Vec<&str> = id.split('.').collect();
    if segments.len() < 2 {
        return Err(format!("\"{id}\" needs at least two dot-separated parts"));
    }
    for segment in &segments {
        let mut chars = segment.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() => {}
            _ => return Err(format!("\"{id}\": each part must start with a letter")),
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!(
                "\"{id}\": use only letters, digits and underscores"
            ));
        }
    }
    Ok(())
}

// ─── Status probes ───────────────────────────────────────────────────────
// Every probe is read-only and needs no device. The Build dialog and
// `android-status` report these; the SDK install flow fixes them.

/// One row of the Settings dialog: what was found and whether it is enough.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolStatus {
    pub ok: bool,
    pub detail: String,
}

fn missing(detail: impl Into<String>) -> ToolStatus {
    ToolStatus {
        ok: false,
        detail: detail.into(),
    }
}

/// `java -version` on PATH, parsed for the major version.
pub fn jdk_status() -> ToolStatus {
    let output = Command::new("java").arg("-version").output();
    let Ok(output) = output else {
        return missing(format!(
            "No `java` on PATH. Install JDK {JDK_MAJOR} (the latest LTS), then point the SDK rows at it."
        ));
    };
    // `java -version` writes to stderr, e.g. `openjdk version "25" ...`.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let version = text
        .split('"')
        .nth(1)
        .and_then(|v| v.split('.').next())
        .and_then(|v| v.parse::<u32>().ok());
    match version {
        Some(major) if major == JDK_MAJOR => ToolStatus {
            ok: true,
            detail: format!("JDK {major} on PATH."),
        },
        Some(major) => missing(format!(
            "Found JDK {major}, but the Android toolchain pins JDK {JDK_MAJOR}. Install it and put it first on PATH."
        )),
        None => missing("`java -version` didn't name a version. Install JDK 25 (the latest LTS)."),
    }
}

fn has(path: &Path) -> bool {
    path.is_file() || path.is_dir()
}

/// The SDK rows: cmdline-tools, the pinned platform and build-tools, and
/// platform-tools for adb. Pointing at an existing install reuses it; only
/// missing pieces download.
pub fn sdk_status(config: &AppConfig) -> ToolStatus {
    let sdk = sdk_dir(config);
    if !sdk.is_dir() {
        return missing(format!(
            "No SDK at {}. Install one in App Settings or point the row at an existing install.",
            sdk.display()
        ));
    }
    let mut absent: Vec<String> = Vec::new();
    if !has(&sdk.join("cmdline-tools/latest/bin/sdkmanager"))
        && !has(&sdk.join("cmdline-tools/latest/bin/sdkmanager.bat"))
    {
        absent.push("cmdline-tools (sdkmanager)".to_string());
    }
    if !sdk.join("platforms").join(PLATFORM).is_dir() {
        absent.push(PLATFORM.to_string());
    }
    if !sdk.join("build-tools").join(BUILD_TOOLS).is_dir() {
        absent.push(format!("build-tools {BUILD_TOOLS}"));
    }
    let adb = sdk.join("platform-tools").join(exe("adb"));
    if !adb.is_file() {
        absent.push("platform-tools (adb)".to_string());
    }
    if absent.is_empty() {
        ToolStatus {
            ok: true,
            detail: format!(
                "SDK at {}: {PLATFORM}, build-tools {BUILD_TOOLS}, adb present.",
                sdk.display()
            ),
        }
    } else {
        missing(format!(
            "SDK at {} is missing: {}.",
            sdk.display(),
            absent.join(", ")
        ))
    }
}

fn exe(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// The NDK row: the directory, its `source.properties` pin, and the clang
/// wrapper the Rust linker rides on.
pub fn ndk_status(config: &AppConfig) -> ToolStatus {
    let ndk = ndk_dir(config);
    if !ndk.is_dir() {
        return missing(format!(
            "No NDK at {}. Install NDK major {NDK_MAJOR} beside the SDK or point the row at one.",
            ndk.display()
        ));
    }
    let properties = std::fs::read_to_string(ndk.join("source.properties")).unwrap_or_default();
    let revision = properties
        .lines()
        .find_map(|line| line.strip_prefix("Pkg.Revision = "))
        .unwrap_or("unknown");
    if !revision
        .split('.')
        .next()
        .is_some_and(|major| major == NDK_MAJOR)
    {
        return missing(format!(
            "NDK at {} is revision {revision}, but Blockloom pins major {NDK_MAJOR}.",
            ndk.display()
        ));
    }
    match ndk_clang(&ndk, host_tag()) {
        Some(clang) => ToolStatus {
            ok: true,
            detail: format!(
                "NDK r{revision} at {}, linker {}.",
                ndk.display(),
                clang.display()
            ),
        },
        None => missing(format!(
            "NDK at {} has no prebuilt clang for this machine, so Rust can't link for Android.",
            ndk.display()
        )),
    }
}

/// The prebuilt tuple dir name for the machine doing the building.
pub fn host_tag() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows-x86_64",
        "macos" => "darwin-x86_64",
        _ => "linux-x86_64",
    }
}

/// The NDK clang wrapper Rust links Android targets through, if present.
pub fn ndk_clang(ndk: &Path, host: &str) -> Option<PathBuf> {
    let prebuilt = ndk.join("toolchains/llvm/prebuilt").join(host).join("bin");
    // The API level on the wrapper only sets the default `-target`; rustc
    // passes `--target` itself, so any level 21+ wrapper links.
    let candidates = [
        format!("{ARM64_TRIPLE}{MIN_SDK}-clang"),
        format!("{EMULATOR_TRIPLE}{MIN_SDK}-clang"),
    ];
    candidates
        .iter()
        .map(|name| prebuilt.join(exe(name)))
        .find(|path| path.is_file())
}

/// The `CARGO_TARGET_*_LINKER` env spelling for `triple`, pointed at the
/// NDK clang wrapper. Callers merge this into the environment (or a cargo
/// snippet beside the one `blockstitch-local` uses), never overwriting the
/// user's own linker choice.
pub fn cargo_linker_env(config: &AppConfig, triple: &str) -> Option<(String, PathBuf)> {
    let clang = ndk_clang(&ndk_dir(config), host_tag())?;
    let var = format!(
        "CARGO_TARGET_{}_LINKER",
        triple.to_uppercase().replace('-', "_")
    );
    Some((var, clang))
}

/// Whether rustc's `std` for `triple` is installed. The linker half is the
/// NDK probe above; this is the `std` half `script::target_installed`
/// already covers.
pub fn rust_target_status(triple: &str) -> ToolStatus {
    match script::target_installed(triple) {
        Ok(()) => ToolStatus {
            ok: true,
            detail: format!("rustc knows {triple} with its std installed."),
        },
        Err(error) => {
            // `target_installed` already names the fix when the std is
            // missing; only add it for the other failures.
            let detail = if error.contains("rustup target add") {
                error
            } else {
                format!("{error} (`rustup target add {triple}`).")
            };
            missing(detail)
        }
    }
}

/// Everything `just android-check` reports: no device needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AndroidStatus {
    pub sdk_path: PathBuf,
    pub ndk_path: PathBuf,
    pub licenses_accepted: bool,
    pub jdk: ToolStatus,
    pub sdk: ToolStatus,
    pub ndk: ToolStatus,
    pub rust_arm64: ToolStatus,
    pub rust_emulator: ToolStatus,
    /// Whether each row could build today.
    pub ready_arm64: bool,
    pub ready_emulator: bool,
    /// The one-line reason when a row is not ready.
    pub note_arm64: String,
    pub note_emulator: String,
}

/// Probes the machine as it stands. Pure reads, safe to call headless.
pub fn status() -> AndroidStatus {
    status_for(&load())
}

fn status_for(config: &AppConfig) -> AndroidStatus {
    let jdk = jdk_status();
    let sdk = sdk_status(config);
    let ndk = ndk_status(config);
    let rust_arm64 = rust_target_status(ARM64_TRIPLE);
    let rust_emulator = rust_target_status(EMULATOR_TRIPLE);
    let (ready_arm64, note_arm64) = readiness(config, &jdk, &sdk, &ndk, &rust_arm64);
    let (ready_emulator, note_emulator) = readiness(config, &jdk, &sdk, &ndk, &rust_emulator);
    AndroidStatus {
        sdk_path: sdk_dir(config),
        ndk_path: ndk_dir(config),
        licenses_accepted: config.licenses_accepted,
        jdk,
        sdk,
        ndk,
        rust_arm64,
        rust_emulator,
        ready_arm64,
        ready_emulator,
        note_arm64,
        note_emulator,
    }
}

fn readiness(
    config: &AppConfig,
    jdk: &ToolStatus,
    sdk: &ToolStatus,
    ndk: &ToolStatus,
    rust: &ToolStatus,
) -> (bool, String) {
    for row in [jdk, sdk, ndk, rust] {
        if !row.ok {
            return (false, row.detail.clone());
        }
    }
    if !config.licenses_accepted {
        return (
            false,
            "The SDK licenses aren't accepted yet. Read them in App Settings first.".to_string(),
        );
    }
    (
        true,
        "Ready: the NDK cross-builds the runtime, no staged player needed.".to_string(),
    )
}

/// Whether `triple` could build today, and why not. The desktop NDK
/// cross-builds the runtime, so unlike desktop rows there is no staged
/// player under `players/<triple>/` to look for.
pub fn readiness_for(triple: &str) -> (bool, String) {
    let config = load();
    let jdk = jdk_status();
    let sdk = sdk_status(&config);
    let ndk = ndk_status(&config);
    let rust = rust_target_status(triple);
    readiness(&config, &jdk, &sdk, &ndk, &rust)
}

/// One connected device or emulator, as `adb devices` names it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub serial: String,
    pub state: String,
    pub emulator: bool,
}

/// `adb devices` from the installed platform-tools, or why there is none.
pub fn device_status() -> Result<Vec<Device>, String> {
    let config = load();
    let adb = sdk_dir(&config).join("platform-tools").join(exe("adb"));
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let output = Command::new(&adb)
        .arg("devices")
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err("`adb devices` failed. Is the SDK's platform-tools intact?".to_string());
    }
    let mut devices = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines().skip(1) {
        let mut parts = line.split_whitespace();
        let (Some(serial), Some(state)) = (parts.next(), parts.next()) else {
            continue;
        };
        devices.push(Device {
            emulator: serial.starts_with("emulator-"),
            serial: serial.to_string(),
            state: state.to_string(),
        });
    }
    Ok(devices)
}

// ─── SDK install flow ────────────────────────────────────────────────────
// The Settings button and `just android-sdk-install` run this same code.
// Pointing a row at an existing install reuses it: every step checks what
// is already there and only missing pieces download. Licenses are never
// accepted here; that is `accept_licenses` below, on the user's click.

/// Google's cmdline-tools bootstrap per host OS. The build number goes
/// stale over time; sdkmanager updates itself once it runs.
pub fn cmdline_tools_url(os: &str) -> &'static str {
    match os {
        "macos" => {
            "https://dl.google.com/android/repository/commandlinetools-mac-11076708_latest.zip"
        }
        "windows" => {
            "https://dl.google.com/android/repository/commandlinetools-win-11076708_latest.zip"
        }
        _ => "https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip",
    }
}

pub fn host_cmdline_tools_url() -> &'static str {
    cmdline_tools_url(std::env::consts::OS)
}

/// `sdkmanager`, wherever the SDK row keeps it.
pub fn sdkmanager_path(config: &AppConfig) -> Option<PathBuf> {
    let root = sdk_dir(config).join("cmdline-tools/latest/bin");
    let path = root.join(exe("sdkmanager"));
    path.is_file().then_some(path)
}

fn run_sdkmanager(
    manager: &Path,
    args: &[&str],
    stdin: std::process::Stdio,
) -> Result<std::process::Output, String> {
    Command::new(manager)
        .args(args)
        .stdin(stdin)
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", manager.display()))
}

/// Streams `url` to `dest`, returning the byte count. Retries a few times:
/// a 150 MB bootstrap plus storefront Wi-Fi means transient resets happen.
/// Desktop only: the install flow never runs in a browser page.
#[cfg(not(target_arch = "wasm32"))]
pub fn download(url: &str, dest: &Path) -> Result<u64, String> {
    let mut error = String::new();
    for attempt in 1..=3 {
        match download_once(url, dest) {
            Ok(bytes) => return Ok(bytes),
            Err(failure) => {
                error = failure;
                std::thread::sleep(std::time::Duration::from_secs(2 * attempt));
            }
        }
    }
    Err(error)
}

#[cfg(not(target_arch = "wasm32"))]
fn download_once(url: &str, dest: &Path) -> Result<u64, String> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("Couldn't download {url}: {e}"))?;
    if !(200..300).contains(&response.status().as_u16()) {
        return Err(format!(
            "Couldn't download {url}: HTTP {}",
            response.status()
        ));
    }
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("Couldn't write {}: {e}", dest.display()))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("Couldn't save {url}: {e}"))
}

/// Unpacks a cmdline-tools zip into the SDK row: Google's archive roots
/// everything at `cmdline-tools/`, which becomes `latest` on disk.
pub fn unzip_cmdline_tools(zip_path: &Path, sdk: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let latest = sdk.join("cmdline-tools/latest");
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("{}: {e}", zip_path.display()))?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let mut parts = name.components();
        // Strip the archive's own `cmdline-tools/` root.
        if parts.next().is_none() {
            continue;
        }
        let relative: PathBuf = parts.collect();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let dest = latest.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            let mut out = std::fs::File::create(&dest)
                .map_err(|e| format!("Couldn't write {}: {e}", dest.display()))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Couldn't write {}: {e}", dest.display()))?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

/// Makes sure `sdkmanager` exists, downloading the bootstrap when the row
/// has none. Returns what it fetched, if anything. Desktop only.
#[cfg(not(target_arch = "wasm32"))]
fn ensure_cmdline_tools(config: &AppConfig) -> Result<Option<(String, u64)>, String> {
    if sdkmanager_path(config).is_some() {
        return Ok(None);
    }
    let sdk = sdk_dir(config);
    std::fs::create_dir_all(&sdk).map_err(|e| format!("{}: {e}", sdk.display()))?;
    let url = host_cmdline_tools_url();
    let dest = std::env::temp_dir().join(format!("blockloom-cmdtools-{}.zip", std::process::id()));
    let bytes = download(url, &dest)?;
    let fetched = (url.to_string(), bytes);
    if let Err(error) = unzip_cmdline_tools(&dest, &sdk) {
        let _ = std::fs::remove_file(&dest);
        return Err(error);
    }
    let _ = std::fs::remove_file(&dest);
    if sdkmanager_path(config).is_none() {
        return Err(
            "The download unpacked but holds no sdkmanager. Google may have moved the bootstrap."
                .to_string(),
        );
    }
    Ok(Some(fetched))
}

/// Picks NDK `27.x` revisions out of `sdkmanager --list` output, oldest
/// first. Pure text, so tests cover it without running anything.
pub fn available_ndk_revisions_from(listing: &str) -> Vec<String> {
    let mut revisions: Vec<String> = listing
        .lines()
        .filter_map(|line| line.split('|').next())
        .map(str::trim)
        .filter_map(|path| path.strip_prefix("ndk;"))
        .filter(|rev| {
            rev.split('.')
                .next()
                .is_some_and(|major| major == NDK_MAJOR)
        })
        .map(str::to_string)
        .collect();
    revisions.sort();
    revisions.dedup();
    revisions
}

/// The newest installable NDK on the pinned major, as `sdkmanager` spells
/// the package (`ndk;<revision>`).
pub fn newest_ndk_package(manager: &Path) -> Result<String, String> {
    let output = run_sdkmanager(manager, &["--list"], std::process::Stdio::null())?;
    let listing = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    available_ndk_revisions_from(&listing)
        .pop()
        .map(|rev| format!("ndk;{rev}"))
        .ok_or_else(|| {
            "sdkmanager lists no NDK on major 27. Check the network or proxy.".to_string()
        })
}

/// Whether an sdkmanager package name is already on disk under `sdk`.
fn package_present(sdk: &Path, package: &str) -> bool {
    if package == "platform-tools" {
        return sdk.join("platform-tools").join(exe("adb")).is_file();
    }
    if let Some(platform) = package.strip_prefix("platforms;") {
        return sdk.join("platforms").join(platform).is_dir();
    }
    if let Some(tools) = package.strip_prefix("build-tools;") {
        return sdk.join("build-tools").join(tools).is_dir();
    }
    false
}

/// What happened when the install flow ran. Licenses are never accepted
/// here; `still_missing` says what is left, including that step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallReport {
    pub sdk_path: PathBuf,
    pub downloaded_cmdline_tools: bool,
    pub bytes_downloaded: u64,
    pub installed: Vec<String>,
    pub already_present: Vec<String>,
    pub still_missing: Vec<String>,
}

/// Downloads the bootstrap when needed and installs the pinned platform,
/// build-tools, platform-tools and NDK. Offline or proxy failure reports
/// what is missing and keeps any existing SDK usable. Desktop only.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_sdk() -> Result<InstallReport, String> {
    let config = load();
    let fetched = ensure_cmdline_tools(&config)?;
    let manager = sdkmanager_path(&config)
        .ok_or_else(|| "No sdkmanager even after the bootstrap. See above.".to_string())?;

    let mut installed = Vec::new();
    let mut already_present = Vec::new();
    let sdk = sdk_dir(&config);
    let wanted: Vec<String> = vec![
        "platform-tools".to_string(),
        format!("platforms;{PLATFORM}"),
        format!("build-tools;{BUILD_TOOLS}"),
    ];
    let ndk_present = ndk_dir(&config).join("source.properties").is_file();
    let ndk_package = if ndk_present {
        None
    } else {
        Some(newest_ndk_package(&manager)?)
    };
    for package in &wanted {
        if package_present(&sdk, package) {
            already_present.push(package.clone());
        }
    }
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|package| !package_present(&sdk, package))
        .map(String::as_str)
        .collect();
    // sdkmanager exits 0 even when the licenses make it install nothing,
    // so every package is re-probed below rather than trusted.
    if !missing.is_empty() {
        let mut install_args = vec!["--install"];
        install_args.extend(missing.iter().copied());
        let output = run_sdkmanager(&manager, &install_args, std::process::Stdio::inherit())?;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let refused: Vec<&str> = missing
            .iter()
            .copied()
            .filter(|package| !package_present(&sdk, package))
            .collect();
        if !refused.is_empty() {
            if !config.licenses_accepted {
                return Err(format!(
                    "sdkmanager installed nothing: the SDK licenses aren't accepted yet, so {} stayed missing. Read them with android-accept-licenses, then re-run with accept=true.",
                    refused.join(", ")
                ));
            }
            return Err(format!(
                "sdkmanager couldn't install {}. {stderr}",
                refused.join(", ")
            ));
        }
        installed.extend(missing.iter().map(|package| package.to_string()));
    }
    if let Some(package) = ndk_package {
        let output = run_sdkmanager(
            &manager,
            &["--install", &package],
            std::process::Stdio::inherit(),
        )?;
        if !output.status.success() || !ndk_dir(&config).join("source.properties").is_file() {
            if !config.licenses_accepted {
                return Err(format!(
                    "sdkmanager installed nothing: the SDK licenses aren't accepted yet, so {package} stayed missing. Read them with android-accept-licenses, then re-run with accept=true."
                ));
            }
            return Err(format!(
                "sdkmanager couldn't install {package}. {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        installed.push(package);
    } else {
        already_present.push(format!("ndk (at {})", ndk_dir(&config).display()));
    }

    let mut still_missing = Vec::new();
    if !config.licenses_accepted {
        still_missing
            .push("SDK licenses: read them with android-accept-licenses, then accept.".to_string());
    }
    let after = status_for(&load());
    for (label, ok) in [
        ("JDK", after.jdk.ok),
        ("SDK packages", after.sdk.ok),
        ("NDK", after.ndk.ok),
    ] {
        if !ok {
            still_missing.push(format!("{label} still probes red; see android-status."));
        }
    }
    Ok(InstallReport {
        sdk_path: sdk_dir(&config),
        downloaded_cmdline_tools: fetched.is_some(),
        bytes_downloaded: fetched.map(|(_, bytes)| bytes).unwrap_or(0),
        installed,
        already_present,
        still_missing,
    })
}

/// The license step: the texts to read, and whether they were accepted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LicenseReport {
    pub text: String,
    pub accepted: bool,
}

/// Shows the SDK licenses (`accept` false), or accepts them on the user's
/// explicit opt-in and records the stamp. Piping yes with no prompt is not
/// allowed, which is why showing is the default.
pub fn accept_licenses(accept: bool) -> Result<LicenseReport, String> {
    let config = load();
    let manager = sdkmanager_path(&config)
        .ok_or_else(|| "No sdkmanager yet. Run android-install-sdk first.".to_string())?;
    if !accept {
        let output = run_sdkmanager(&manager, &["--licenses"], std::process::Stdio::null())?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let text = text.trim().to_string();
        return Ok(LicenseReport {
            text: if text.is_empty() {
                "sdkmanager showed no licenses. They may all be accepted already.".to_string()
            } else {
                text
            },
            accepted: false,
        });
    }
    // The user has read the texts above and opted in: answer every prompt
    // with yes, then stamp the config.
    let mut child = Command::new(&manager)
        .arg("--licenses")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run {}: {e}", manager.display()))?;
    if let Some(stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            use std::io::Write;
            let mut stdin = stdin;
            // Bounded: enough yeses for every license, then stop even if the
            // child keeps asking.
            for _ in 0..1000 {
                if stdin.write_all(b"y\n").is_err() || stdin.flush().is_err() {
                    break;
                }
            }
        });
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Couldn't finish {}: {e}", manager.display()))?;
    if !output.status.success() {
        return Err(format!(
            "sdkmanager --licenses failed. {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut config = config;
    config.licenses_accepted = true;
    save(&config)?;
    Ok(LicenseReport {
        text: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .trim()
        .to_string(),
        accepted: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_ids_sanitize_and_validate() {
        assert_eq!(
            sanitize_application_id("Pond Game"),
            "com.blockloom.game.pond_game"
        );
        assert_eq!(
            sanitize_application_id("Slime 3000!"),
            "com.blockloom.game.slime_3000_"
        );
        assert_eq!(sanitize_application_id("!!!"), "com.blockloom.game.g");
        assert!(validate_application_id("com.blockloom.game.pond_game").is_ok());
        assert!(validate_application_id("single").is_err());
        assert!(validate_application_id("com.9lives.game").is_err());
        assert!(validate_application_id("com.block loom.game").is_err());
    }

    #[test]
    fn config_round_trips_through_a_file() {
        let path =
            std::env::temp_dir().join(format!("blockloom-android-{}.json", std::process::id()));
        let config = AppConfig {
            sdk_path: Some(PathBuf::from("/opt/android-sdk")),
            ndk_path: None,
            licenses_accepted: true,
        };
        save_to(&config, &path).unwrap();
        assert_eq!(load_from(&path), config);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn android_triples_know_themselves_and_their_abis() {
        assert!(is_android(ARM64_TRIPLE));
        assert!(is_android(EMULATOR_TRIPLE));
        assert!(!is_android("x86_64-unknown-linux-gnu"));
        assert_eq!(abi(ARM64_TRIPLE), Some("arm64-v8a"));
        assert_eq!(abi(EMULATOR_TRIPLE), Some("x86_64"));
        assert_eq!(abi("wasm32-unknown-unknown"), None);
    }

    #[test]
    fn linker_env_spelling_matches_cargo() {
        let config = AppConfig {
            sdk_path: Some(PathBuf::from("/nonexistent-sdk")),
            ..AppConfig::default()
        };
        // No NDK there, so nothing to merge; the shape is checked below.
        assert!(cargo_linker_env(&config, ARM64_TRIPLE).is_none());
        let var = format!(
            "CARGO_TARGET_{}_LINKER",
            ARM64_TRIPLE.to_uppercase().replace('-', "_")
        );
        assert_eq!(var, "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER");
    }

    #[test]
    fn empty_config_is_never_ready() {
        let ready = status_for(&AppConfig::default());
        assert!(!ready.ready_arm64);
        assert!(!ready.ready_emulator);
        assert!(!ready.note_arm64.is_empty());
    }

    #[test]
    fn cmdline_tools_urls_name_the_host() {
        for (os, infix) in [("linux", "linux"), ("macos", "mac"), ("windows", "win")] {
            let url = cmdline_tools_url(os);
            assert!(url.contains(infix), "{url}");
            assert!(url.ends_with("_latest.zip"), "{url}");
        }
    }

    #[test]
    fn ndk_revision_picking_reads_sdkmanager_listings() {
        let listing = "\
Installed packages:\n  \
ndk;26.1.10909125 | 26.1.10909125\n\
Available Packages:\n  \
ndk;27.0.12077973 | 27.0.12077973 | Android NDK\n  \
ndk;27.2.12479018 | 27.2.12479018 | Android NDK\n  \
cmake;3.22.1 | 3.22.1 | CMake\n";
        assert_eq!(
            available_ndk_revisions_from(listing),
            vec!["27.0.12077973".to_string(), "27.2.12479018".to_string()]
        );
        assert!(available_ndk_revisions_from("nothing here\n").is_empty());
    }

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("blockloom-android-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_cmdline_tools_zip_unpacks_into_latest() {
        let root = temp_root("unzip");
        let zip_path = root.join("tools.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            for name in [
                "cmdline-tools/bin/sdkmanager",
                "cmdline-tools/lib/common.jar",
            ] {
                writer.start_file(name, options).unwrap();
                use std::io::Write;
                writer.write_all(b"fake").unwrap();
            }
            writer.finish().unwrap();
        }
        let sdk = root.join("sdk");
        unzip_cmdline_tools(&zip_path, &sdk).unwrap();
        assert!(sdk.join("cmdline-tools/latest/bin/sdkmanager").is_file());
        assert!(sdk.join("cmdline-tools/latest/lib/common.jar").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn package_presence_reads_the_sdk_tree() {
        let root = temp_root("packages");
        let sdk = root.join("sdk");
        assert!(!package_present(&sdk, "platform-tools"));
        std::fs::create_dir_all(sdk.join("platform-tools")).unwrap();
        assert!(!package_present(&sdk, "platform-tools"));
        std::fs::write(sdk.join("platform-tools").join(exe("adb")), b"fake").unwrap();
        assert!(package_present(&sdk, "platform-tools"));
        assert!(!package_present(&sdk, "platforms;android-35"));
        std::fs::create_dir_all(sdk.join("platforms").join(PLATFORM)).unwrap();
        assert!(package_present(&sdk, "platforms;android-35"));
        assert!(!package_present(&sdk, "no-such-package"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sdk_and_ndk_probes_read_a_fabricated_tree() {
        let root = temp_root("tree");
        let sdk = root.join("sdk");
        std::fs::create_dir_all(sdk.join("platforms").join(PLATFORM)).unwrap();
        std::fs::create_dir_all(sdk.join("build-tools").join(BUILD_TOOLS)).unwrap();
        std::fs::create_dir_all(sdk.join("platform-tools")).unwrap();
        std::fs::write(sdk.join("platform-tools").join(exe("adb")), b"fake").unwrap();
        // cmdline-tools missing on purpose: the sdk row lists it first.
        let config = AppConfig {
            sdk_path: Some(sdk.clone()),
            ndk_path: None,
            licenses_accepted: false,
        };
        let row = sdk_status(&config);
        assert!(!row.ok, "{row:?}");
        assert!(row.detail.contains("cmdline-tools"), "{row:?}");

        std::fs::create_dir_all(sdk.join("cmdline-tools/latest/bin")).unwrap();
        std::fs::write(
            sdk.join("cmdline-tools/latest/bin").join(exe("sdkmanager")),
            b"fake",
        )
        .unwrap();
        let row = sdk_status(&config);
        assert!(row.ok, "{row:?}");

        let ndk = sdk.join("ndk").join("27.2.12479018");
        std::fs::create_dir_all(&ndk).unwrap();
        std::fs::write(
            ndk.join("source.properties"),
            b"Pkg.Revision = 27.2.12479018\n",
        )
        .unwrap();
        // No prebuilt clang on this fake NDK, so the row names the linker.
        let row = ndk_status(&config);
        assert!(!row.ok, "{row:?}");
        assert!(row.detail.contains("clang"), "{row:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
