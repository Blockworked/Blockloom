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
//! the `applicationId` rules, the SDK install and license flow, the APK
//! assembly and signing (debug keystore, or the project's release key with
//! per-build passwords from the args, the env or the OS keyring), and the
//! install plus logcat tail behind the Build dialog's device rows.

use crate::{project, script};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "android_keyring.rs"]
mod keyring;

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

/// Points the SDK row at `path` (empty clears back to the default) and
/// answers the resolved dir. Never touches `ANDROID_HOME` or friends: the
/// row is the only source of truth.
pub fn set_sdk_path(path: &str) -> Result<PathBuf, String> {
    let mut config = load();
    let trimmed = path.trim();
    config.sdk_path = if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    };
    let resolved = sdk_dir(&config);
    save(&config)?;
    Ok(resolved)
}

/// Points the NDK row at `path` (empty clears back to the pinned NDK inside
/// the SDK) and answers the resolved dir.
pub fn set_ndk_path(path: &str) -> Result<PathBuf, String> {
    let mut config = load();
    let trimmed = path.trim();
    config.ndk_path = if trimmed.is_empty() {
        None
    } else {
        let dir = PathBuf::from(trimmed);
        if !dir.is_dir() {
            return Err(format!("{} isn't a directory.", dir.display()));
        }
        Some(dir)
    };
    let resolved = ndk_dir(&config);
    save(&config)?;
    Ok(resolved)
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

// ─── Per-project Android settings ────────────────────────────────────────
// applicationId, version and icons stay in Project settings (the project
// file), while SDK/NDK paths and the license stamp stay in the app config
// above. Empty means the default: the id from the project name, version
// 1 / 1.0.0.

/// The Play-store-style rows of Project settings. Old project files carry
/// none of these, so everything defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AndroidSettings {
    /// Overrides `com.blockloom.game.<sanitized name>` when non-empty.
    #[serde(default)]
    pub application_id: String,
    /// 0 in an old file means 1.
    #[serde(default)]
    pub version_code: u32,
    /// Empty in an old file means `1.0.0`.
    #[serde(default)]
    pub version_name: String,
    /// Release keystore for this game, as an absolute path. Empty signs
    /// with the debug keystore instead. Passwords are never stored here;
    /// each build asks for them (or reads the env, see below).
    #[serde(default)]
    pub keystore: String,
    /// Which key inside that keystore signs the APK. Empty is only fine
    /// when no keystore is set.
    #[serde(default)]
    pub key_alias: String,
}

impl Default for AndroidSettings {
    fn default() -> Self {
        Self {
            application_id: String::new(),
            version_code: 1,
            version_name: "1.0.0".to_string(),
            keystore: String::new(),
            key_alias: String::new(),
        }
    }
}

impl AndroidSettings {
    /// The id the APK builds with: the override, or the project name made
    /// package-safe. An invalid override stops the build, not the default.
    pub fn application_id_for(&self, project_name: &str) -> Result<String, String> {
        let id = if self.application_id.trim().is_empty() {
            sanitize_application_id(project_name)
        } else {
            self.application_id.trim().to_string()
        };
        validate_application_id(&id)?;
        Ok(id)
    }

    pub fn version_code_or_default(&self) -> u32 {
        self.version_code.max(1)
    }

    pub fn version_name_or_default(&self) -> String {
        if self.version_name.trim().is_empty() {
            "1.0.0".to_string()
        } else {
            self.version_name.trim().to_string()
        }
    }
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
    // An empty build-tools dir probes as present but assembles nothing: a
    // stopped download leaves exactly that behind. The APK assembly needs
    // these three, so each is named on its own.
    for tool in ["aapt2", "zipalign", "apksigner"] {
        if !sdk
            .join("build-tools")
            .join(BUILD_TOOLS)
            .join(exe(tool))
            .is_file()
        {
            absent.push(format!("build-tools {BUILD_TOOLS} ({tool})"));
        }
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
    // Either triple's wrapper probes whether this machine can link at all;
    // per-triple builds take `ndk_clang_for` below instead.
    let candidates = [
        format!("{ARM64_TRIPLE}{MIN_SDK}-clang"),
        format!("{EMULATOR_TRIPLE}{MIN_SDK}-clang"),
    ];
    candidates
        .iter()
        .map(|name| prebuilt.join(exe(name)))
        .find(|path| path.is_file())
}

/// The NDK clang wrapper for one triple: its own `{triple}{MIN_SDK}-clang`
/// first, falling back to whatever wrapper the NDK has. The wrapper's name
/// is its default `--target`, which rustc relies on when it shells out to
/// link - an arm64 wrapper cannot link x86_64 objects, so each triple must
/// take its own.
pub fn ndk_clang_for(ndk: &Path, host: &str, triple: &str) -> Option<PathBuf> {
    let own = ndk
        .join("toolchains/llvm/prebuilt")
        .join(host)
        .join("bin")
        .join(exe(&format!("{triple}{MIN_SDK}-clang")));
    if own.is_file() {
        return Some(own);
    }
    ndk_clang(ndk, host)
}

/// The `CARGO_TARGET_*_LINKER` env spelling for `triple`, pointed at the
/// NDK clang wrapper. Callers merge this into the environment (or a cargo
/// snippet beside the one `blockstitch-local` uses), never overwriting the
/// user's own linker choice.
pub fn cargo_linker_env(config: &AppConfig, triple: &str) -> Option<(String, PathBuf)> {
    let clang = ndk_clang_for(&ndk_dir(config), host_tag(), triple)?;
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
    /// The emulator dev loop: binary, image and AVDs with run state.
    pub emulator: EmulatorStatus,
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
        emulator: emulator_status_for(config),
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
    readiness_for_config(&load(), triple)
}

/// [`readiness_for`] against an explicit config, so builds and tests name
/// a toolchain without touching the saved one.
pub fn readiness_for_config(config: &AppConfig, triple: &str) -> (bool, String) {
    let jdk = jdk_status();
    let sdk = sdk_status(config);
    let ndk = ndk_status(config);
    let rust = rust_target_status(triple);
    readiness(config, &jdk, &sdk, &ndk, &rust)
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
    devices_with_adb(&adb)
}

fn devices_with_adb(adb: &Path) -> Result<Vec<Device>, String> {
    let output = Command::new(adb)
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

// ─── Emulator ──────────────────────────────────────────────────────────
// The x86_64 dev loop, managed from App Settings rather than a terminal:
// the install flow brings the emulator package plus one pinned system
// image, Blockloom keeps AVDs on it, and the Build dialog installs on
// whoever is booted (an emulator counts as a device on the x86_64 row).

/// The emulator system image the dev loop boots: API 35 with Google APIs,
/// x86_64 to match the emulator build triple.
pub const EMULATOR_IMAGE: &str = "system-images;android-35;google_apis;x86_64";
/// The AVD name used when none is given.
pub const DEFAULT_AVD: &str = "blockloom";
/// How long a start waits for boot by default: first boots are slow.
pub const BOOT_WAIT_SECS: u64 = 300;

/// One virtual device: its AVD name, the adb serial while running (empty
/// when off), and whether the boot completed. Joined from `emulator
/// -list-avds` and `adb devices`, so one command draws the whole section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmulatorState {
    pub name: String,
    pub serial: String,
    pub booted: bool,
}

/// The emulator rows: whether this machine can boot anything, why not, and
/// every AVD with its run state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmulatorStatus {
    pub available: bool,
    pub detail: String,
    pub avds: Vec<EmulatorState>,
}

/// What an emulator start did: the AVD, the adb serial once adb sees it
/// (empty when it hasn't yet), and whether the boot completed within the
/// wait. A started-but-booting emulator is not an error: the dialog keeps
/// polling `android_device_status` after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmulatorBoot {
    pub avd: String,
    pub serial: String,
    pub booted: bool,
}

fn emulator_bin(config: &AppConfig) -> PathBuf {
    sdk_dir(config).join("emulator").join(exe("emulator"))
}

fn avdmanager_bin(config: &AppConfig) -> Option<PathBuf> {
    let path = sdk_dir(config)
        .join("cmdline-tools/latest/bin")
        .join(exe("avdmanager"));
    path.is_file().then_some(path)
}

/// The system image dir on disk: `system-images/android-35/google_apis/x86_64`.
fn system_image_dir(config: &AppConfig) -> PathBuf {
    let mut dir = sdk_dir(config).join("system-images");
    for part in EMULATOR_IMAGE
        .strip_prefix("system-images;")
        .unwrap_or(EMULATOR_IMAGE)
        .split(';')
    {
        dir.push(part);
    }
    dir
}

/// The emulator rows as they stand: binary, image, and every AVD with its
/// run state. Pure reads, safe to call headless.
pub fn emulator_status() -> EmulatorStatus {
    emulator_status_for(&load())
}

fn emulator_status_for(config: &AppConfig) -> EmulatorStatus {
    if !emulator_bin(config).is_file() {
        return EmulatorStatus {
            available: false,
            detail: "No emulator package: run Install / update SDK to fetch it.".to_string(),
            avds: vec![],
        };
    }
    if !system_image_dir(config).is_dir() {
        return EmulatorStatus {
            available: false,
            detail: "No Android 35 x86_64 system image: run Install / update SDK to fetch it."
                .to_string(),
            avds: vec![],
        };
    }
    let names = match list_avds_with(&emulator_bin(config)) {
        Ok(names) => names,
        Err(error) => {
            return EmulatorStatus {
                available: false,
                detail: error,
                avds: vec![],
            };
        }
    };
    // Run state is best effort: without adb the AVDs still list, just with
    // no serials. `emu avd name` maps each running emulator back to its AVD.
    let adb = sdk_dir(config).join("platform-tools").join(exe("adb"));
    let mut running: Vec<(String, String, bool)> = Vec::new();
    if adb.is_file() {
        for device in devices_with_adb(&adb).unwrap_or_default() {
            if device.serial.starts_with("emulator-") {
                running.push((
                    avd_name_with_adb(&adb, &device.serial).unwrap_or_default(),
                    device.serial.clone(),
                    booted_with_adb(&adb, &device.serial),
                ));
            }
        }
    }
    EmulatorStatus {
        available: true,
        detail: "Emulator and the Android 35 x86_64 image are installed.".to_string(),
        avds: names
            .into_iter()
            .map(|name| {
                let (serial, booted) = running
                    .iter()
                    .find(|(avd, _, _)| avd == &name)
                    .map(|(_, serial, booted)| (serial.clone(), *booted))
                    .unwrap_or_default();
                EmulatorState {
                    name,
                    serial,
                    booted,
                }
            })
            .collect(),
    }
}

/// Every AVD the emulator binary knows, one name per line.
pub fn list_avds() -> Result<Vec<String>, String> {
    list_avds_with(&emulator_bin(&load()))
}

fn list_avds_with(emulator: &Path) -> Result<Vec<String>, String> {
    if !emulator.is_file() {
        return Err("No emulator: run Install / update SDK to fetch it.".to_string());
    }
    let output = Command::new(emulator)
        .arg("-list-avds")
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", emulator.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't list AVDs: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Makes an AVD on the pinned image, answering its name. Empty names the
/// managed default. Refuses names avdmanager would choke on and ones
/// already taken, with the fix attached.
pub fn create_avd(name: Option<&str>) -> Result<String, String> {
    let config = load();
    let Some(manager) = avdmanager_bin(&config) else {
        return Err("No avdmanager: run Install / update SDK to fetch it.".to_string());
    };
    create_avd_with(
        &emulator_bin(&config),
        &manager,
        system_image_dir(&config).is_dir(),
        name,
    )
}

fn create_avd_with(
    emulator: &Path,
    manager: &Path,
    image_present: bool,
    name: Option<&str>,
) -> Result<String, String> {
    if !emulator.is_file() {
        return Err("No emulator: run Install / update SDK to fetch it.".to_string());
    }
    if !image_present {
        return Err(
            "No Android 35 x86_64 system image: run Install / update SDK to fetch it.".to_string(),
        );
    }
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_AVD);
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(format!(
            "\"{name}\" can't be an AVD name: letters, digits, dots, dashes and underscores only."
        ));
    }
    if list_avds_with(emulator)?.iter().any(|avd| avd == name) {
        return Err(format!("{name} already exists. Start it instead."));
    }
    // Decline the custom hardware profile prompt on stdin: the pinned
    // image's defaults are the dev loop.
    let mut child = Command::new(manager)
        .arg("create")
        .arg("avd")
        .arg("-n")
        .arg(name)
        .arg("-k")
        .arg(EMULATOR_IMAGE)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run {}: {e}", manager.display()))?;
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "Couldn't answer avdmanager's hardware prompt.".to_string())?
        .write_all(b"no\n")
        .map_err(|e| format!("Couldn't answer avdmanager's hardware prompt: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Couldn't run {}: {e}", manager.display()))?;
    if !output.status.success() || !list_avds_with(emulator)?.iter().any(|avd| avd == name) {
        return Err(format!(
            "avdmanager couldn't create {name}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(name.to_string())
}

/// The argv an emulator boot takes: the AVD with the boot animation off,
/// so unattended starts reach the launcher sooner. Embedded boots add
/// `-no-window`: the host window never opens and the editor shows the
/// screen itself (see `mirror_frame`), Android Studio's tool-window mode.
pub fn emulator_spawn_args(avd: &str) -> Vec<String> {
    emulator_spawn_args_with(avd, false)
}

/// The argv an emulator boot takes, with or without its host window.
pub fn emulator_spawn_args_with(avd: &str, headless: bool) -> Vec<String> {
    let mut args = vec!["-avd".to_string(), avd.to_string()];
    if headless {
        args.push("-no-window".to_string());
    }
    args.push("-no-boot-anim".to_string());
    args
}

/// Boots `avd` (the managed default when unset, created on the spot when no
/// AVDs exist at all) and waits up to `wait_secs` (default 5 minutes, 0 to
/// return right after spawning) for adb to see it booted. Answers the
/// serial once adb sees it and whether the boot completed in time.
pub fn start_emulator(avd: Option<&str>, wait_secs: Option<u64>) -> Result<EmulatorBoot, String> {
    start_emulator_with_options(avd, wait_secs, false)
}

/// Boots `avd` the way `start_emulator` does, with its host window hidden
/// when `headless` is true. Embedded boots never open a window: the editor
/// shows the screen itself (see `mirror_frame`), like Android Studio's
/// embedded emulator tool window.
pub fn start_emulator_with_options(
    avd: Option<&str>,
    wait_secs: Option<u64>,
    headless: bool,
) -> Result<EmulatorBoot, String> {
    let config = load();
    start_emulator_with(
        &emulator_bin(&config),
        &sdk_dir(&config).join("platform-tools").join(exe("adb")),
        system_image_dir(&config).is_dir(),
        avd,
        wait_secs.unwrap_or(BOOT_WAIT_SECS),
        headless,
    )
}

#[allow(clippy::too_many_arguments)]
fn start_emulator_with(
    emulator: &Path,
    adb: &Path,
    image_present: bool,
    avd: Option<&str>,
    wait_secs: u64,
    headless: bool,
) -> Result<EmulatorBoot, String> {
    if !emulator.is_file() {
        return Err("No emulator: run Install / update SDK to fetch it.".to_string());
    }
    if !image_present {
        return Err(
            "No Android 35 x86_64 system image: run Install / update SDK to fetch it.".to_string(),
        );
    }
    let avd = avd
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_AVD);
    let names = list_avds_with(emulator)?;
    if names.is_empty() && avd == DEFAULT_AVD {
        // The dev loop's first Start just works: make the managed AVD.
        // avdmanager sits beside the emulator binary's SDK; find it up.
        let manager = emulator
            .parent()
            .and_then(|bin| bin.parent())
            .map(|sdk| sdk.join("cmdline-tools/latest/bin").join(exe("avdmanager")))
            .filter(|path| path.is_file())
            .ok_or_else(|| "No avdmanager: run Install / update SDK to fetch it.".to_string())?;
        create_avd_with(emulator, &manager, true, None)?;
    } else if !names.iter().any(|name| name == avd) {
        return Err(format!(
            "No AVD named {avd}. Create one in App Settings first."
        ));
    }
    let before: Vec<String> = devices_with_adb(adb)
        .unwrap_or_default()
        .into_iter()
        .map(|device| device.serial)
        .collect();
    Command::new(emulator)
        .args(emulator_spawn_args_with(avd, headless))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start the emulator: {e}"))?;
    // The new serial is the emulator adb didn't know before the spawn.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
    loop {
        let mut serial = String::new();
        for device in devices_with_adb(adb).unwrap_or_default() {
            if device.serial.starts_with("emulator-") && !before.contains(&device.serial) {
                serial = device.serial;
                break;
            }
        }
        if !serial.is_empty() && booted_with_adb(adb, &serial) {
            return Ok(EmulatorBoot {
                avd: avd.to_string(),
                serial,
                booted: true,
            });
        }
        if !serial.is_empty() && std::time::Instant::now() >= deadline {
            return Ok(EmulatorBoot {
                avd: avd.to_string(),
                serial,
                booted: false,
            });
        }
        if serial.is_empty() && (wait_secs == 0 || std::time::Instant::now() >= deadline) {
            return Ok(EmulatorBoot {
                avd: avd.to_string(),
                serial,
                booted: false,
            });
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

/// Whether `serial` finished booting: `sys.boot_completed` reads 1.
fn booted_with_adb(adb: &Path, serial: &str) -> bool {
    boot_prop_with_adb(adb, serial).is_some_and(|value| value == "1")
}

fn boot_prop_with_adb(adb: &Path, serial: &str) -> Option<String> {
    let output = Command::new(adb)
        .arg("-s")
        .arg(serial)
        .arg("shell")
        .arg("getprop")
        .arg("sys.boot_completed")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The AVD name a running emulator booted, through `adb emu avd name`.
/// Empty when adb can't say (offline emulator, old image).
fn avd_name_with_adb(adb: &Path, serial: &str) -> Option<String> {
    let output = Command::new(adb)
        .arg("-s")
        .arg(serial)
        .arg("emu")
        .arg("avd")
        .arg("name")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "OK")
        .map(str::to_string)
}

/// Stops the running emulator on `serial` (`adb emu kill`), answering it.
/// Empty stops the only running emulator; several running need naming. A
/// physical serial is refused: unplug those, don't kill them.
pub fn stop_emulator(serial: Option<&str>) -> Result<String, String> {
    // A physical serial is refused before adb is even looked up: unplug
    // those, don't kill them.
    if let Some(serial) = serial.map(str::trim).filter(|s| !s.is_empty())
        && !serial.starts_with("emulator-")
    {
        return Err(format!(
            "{serial} is not an emulator. Only emulators stop this way."
        ));
    }
    let config = load();
    let adb = sdk_dir(&config).join("platform-tools").join(exe("adb"));
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let serial = match serial.map(str::trim).filter(|s| !s.is_empty()) {
        Some(serial) => serial.to_string(),
        None => {
            let running: Vec<String> = devices_with_adb(&adb)?
                .into_iter()
                .filter(|device| device.emulator)
                .map(|device| device.serial)
                .collect();
            match running.len() {
                0 => return Err("No emulator is running.".to_string()),
                1 => running.into_iter().next().unwrap_or_default(),
                _ => {
                    return Err(format!(
                        "Several emulators are running ({}). Name one's serial.",
                        running.join(", ")
                    ));
                }
            }
        }
    };
    let output = Command::new(&adb)
        .arg("-s")
        .arg(&serial)
        .arg("emu")
        .arg("kill")
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't stop {serial}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(serial)
}

// ─── Embedded mirror ─────────────────────────────────────────────────────
// Android Studio's emulator tool window, minus the video codec: a headless
// boot (`-no-window`, see `start_emulator_with_options`) whose screen the
// editor polls as downscaled PNGs and whose touch the editor forwards
// through `adb shell input`. Polling `screencap` runs a few frames a
// second - fine for menus and turn-based games, while action games keep
// the external window. A later pass can swap the transport for scrcpy's
// H.264 stream; the QML side already speaks frames plus fractional taps.

/// The widest frame the mirror answers, in pixels. A phone screenshot is
/// megabytes as PNG; a 360-wide one is tens of kilobytes and still sharp
/// in the editor panel.
pub const MIRROR_DEFAULT_WIDTH: u32 = 360;
/// Hard bounds on the asked width, so a typo can't ask for an 8K PNG.
pub const MIRROR_MIN_WIDTH: u32 = 144;
/// Hard bounds on the asked width, so a typo can't ask for an 8K PNG.
pub const MIRROR_MAX_WIDTH: u32 = 720;

/// One polled screen: a `data:image/png;base64` URL the QML Image shows,
/// its pixel size, and the device's own pixel size for input mapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirrorFrame {
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub device_width: u32,
    pub device_height: u32,
}

/// Grabs `device`'s screen (or the only device when unset) as a downscaled
/// PNG data URL. Physical phones mirror too; anything adb sees works.
pub fn mirror_frame(device: Option<&str>, max_width: Option<u32>) -> Result<MirrorFrame, String> {
    let adb = adb_path();
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let serial = resolve_mirror_serial(&adb, device)?;
    let output = Command::new(&adb)
        .arg("-s")
        .arg(&serial)
        .arg("exec-out")
        .arg("screencap")
        .arg("-p")
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't grab {serial}'s screen: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    // `exec-out` on Windows CRLF-mangles binary stdout; adb already wrote
    // the PNG with lone LFs, so strip CRs back out before decoding.
    let raw: Vec<u8> = output.stdout.into_iter().filter(|b| *b != b'\r').collect();
    let shot = image::load_from_memory(&raw)
        .map_err(|e| format!("Couldn't decode {serial}'s screenshot: {e}"))?;
    let width = max_width
        .unwrap_or(MIRROR_DEFAULT_WIDTH)
        .clamp(MIRROR_MIN_WIDTH, MIRROR_MAX_WIDTH);
    let small = shot.thumbnail(width, u32::MAX);
    let mut png = Vec::new();
    small
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("Couldn't pack {serial}'s screenshot: {e}"))?;
    let (device_width, device_height) =
        device_size_with_adb(&adb, &serial).unwrap_or((small.width(), small.height()));
    use base64::Engine;
    Ok(MirrorFrame {
        image: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        ),
        width: small.width(),
        height: small.height(),
        device_width,
        device_height,
    })
}

/// Picks the serial a mirror command talks to: the named one, or the only
/// device when unset. Several devices need naming, like `stop_emulator`.
fn resolve_mirror_serial(adb: &Path, device: Option<&str>) -> Result<String, String> {
    if let Some(serial) = device.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(serial.to_string());
    }
    let devices = devices_with_adb(adb)?;
    match devices.len() {
        0 => Err("No devices: connect a phone or start an emulator first.".to_string()),
        1 => Ok(devices
            .into_iter()
            .next()
            .map(|d| d.serial)
            .unwrap_or_default()),
        _ => Err(format!(
            "Several devices are connected ({}). Name one's serial.",
            devices
                .into_iter()
                .map(|d| d.serial)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The device's own pixel size through `adb shell wm size`, or why not.
pub fn device_size(device: Option<&str>) -> Result<(u32, u32), String> {
    let adb = adb_path();
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let serial = resolve_mirror_serial(&adb, device)?;
    device_size_with_adb(&adb, &serial)
}

fn device_size_with_adb(adb: &Path, serial: &str) -> Result<(u32, u32), String> {
    let output = Command::new(adb)
        .arg("-s")
        .arg(serial)
        .arg("shell")
        .arg("wm")
        .arg("size")
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't read {serial}'s screen size: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_wm_size(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        format!("Couldn't read {serial}'s screen size: unexpected `wm size` output.")
    })
}

/// Reads `Physical size: 1080x2400` (or `Override size:`) out of `wm size`.
fn parse_wm_size(text: &str) -> Option<(u32, u32)> {
    for line in text.lines() {
        let (_, dims) = line.split_once(':')?;
        let (w, h) = dims.trim().split_once('x')?;
        if let (Ok(w), Ok(h)) = (w.trim().parse(), h.trim().parse()) {
            return Some((w, h));
        }
    }
    None
}

/// Maps a fractional point (0..1 across the mirror image) onto device
/// pixels. The QML side sends fractions; the device wants pixels.
fn mirror_point(x: f32, y: f32, device_width: u32, device_height: u32) -> (u32, u32) {
    (
        (x.clamp(0.0, 1.0) * device_width as f32).round() as u32,
        (y.clamp(0.0, 1.0) * device_height as f32).round() as u32,
    )
}

/// Taps `device` at the fractional point `x, y` (0..1 across the mirror
/// image). What a click on the embedded screen becomes.
pub fn mirror_tap(device: Option<&str>, x: f32, y: f32) -> Result<String, String> {
    let adb = adb_path();
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let serial = resolve_mirror_serial(&adb, device)?;
    let (dw, dh) = device_size_with_adb(&adb, &serial)?;
    let (px, py) = mirror_point(x, y, dw, dh);
    let output = Command::new(&adb)
        .arg("-s")
        .arg(&serial)
        .arg("shell")
        .arg("input")
        .arg("tap")
        .arg(px.to_string())
        .arg(py.to_string())
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't tap {serial}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(serial)
}

/// Swipes `device` from one fractional point to another over
/// `duration_ms`. What a drag across the embedded screen becomes.
pub fn mirror_swipe(
    device: Option<&str>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    duration_ms: Option<u64>,
) -> Result<String, String> {
    let adb = adb_path();
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let serial = resolve_mirror_serial(&adb, device)?;
    let (dw, dh) = device_size_with_adb(&adb, &serial)?;
    let (x1, y1) = mirror_point(x1, y1, dw, dh);
    let (x2, y2) = mirror_point(x2, y2, dw, dh);
    let output = Command::new(&adb)
        .arg("-s")
        .arg(&serial)
        .arg("shell")
        .arg("input")
        .arg("swipe")
        .arg(x1.to_string())
        .arg(y1.to_string())
        .arg(x2.to_string())
        .arg(y2.to_string())
        .arg(duration_ms.unwrap_or(300).to_string())
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't swipe on {serial}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(serial)
}

/// Presses a named key on `device`: back, home, recents, enter, delete,
/// tab, power, or volume_up/volume_down/volume_mute. The embedded screen's
/// hardware buttons.
pub fn mirror_key(device: Option<&str>, code: &str) -> Result<String, String> {
    let keycode = match code.trim().to_lowercase().as_str() {
        "back" | "escape" => "KEYCODE_BACK",
        "home" => "KEYCODE_HOME",
        "recents" | "appswitch" => "KEYCODE_APP_SWITCH",
        "enter" => "KEYCODE_ENTER",
        "delete" => "KEYCODE_DEL",
        "tab" => "KEYCODE_TAB",
        "power" => "KEYCODE_POWER",
        "volume_up" => "KEYCODE_VOLUME_UP",
        "volume_down" => "KEYCODE_VOLUME_DOWN",
        "volume_mute" => "KEYCODE_VOLUME_MUTE",
        other => {
            return Err(format!(
                "\"{other}\" isn't a mirror key: back, home, recents, enter, delete, tab, power, volume_up, volume_down, volume_mute."
            ));
        }
    };
    let adb = adb_path();
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let serial = resolve_mirror_serial(&adb, device)?;
    let output = Command::new(&adb)
        .arg("-s")
        .arg(&serial)
        .arg("shell")
        .arg("input")
        .arg("keyevent")
        .arg(keycode)
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't press {code} on {serial}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(serial)
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
/// Build-tools counts only with its assembler present: a stopped download
/// leaves an empty version dir that installs nothing.
fn package_present(sdk: &Path, package: &str) -> bool {
    if package == "platform-tools" {
        return sdk.join("platform-tools").join(exe("adb")).is_file();
    }
    if let Some(platform) = package.strip_prefix("platforms;") {
        return sdk.join("platforms").join(platform).is_dir();
    }
    if let Some(tools) = package.strip_prefix("build-tools;") {
        return sdk
            .join("build-tools")
            .join(tools)
            .join(exe("aapt2"))
            .is_file();
    }
    if package == "emulator" {
        return sdk.join("emulator").join(exe("emulator")).is_file();
    }
    if let Some(image) = package.strip_prefix("system-images;") {
        let mut dir = sdk.join("system-images");
        for part in image.split(';') {
            dir.push(part);
        }
        return dir.is_dir();
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
/// build-tools, platform-tools, NDK, emulator and one x86_64 system image.
/// Offline or proxy failure reports what is missing and keeps any existing
/// SDK usable. Desktop only.
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
        "emulator".to_string(),
        EMULATOR_IMAGE.to_string(),
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
        ("Emulator", after.emulator.available),
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

// ─── APK assembly ────────────────────────────────────────────────────────
// No Gradle: the APK is assembled straight from the SDK build-tools the
// install flow just laid down (aapt2, zipalign, apksigner) plus the host
// JDK's keytool. That keeps an Android build hermetic - no 130 MB Gradle
// distribution, no Maven round trips at build time - and every step below
// is a local binary with testable arguments. The manifest template lives
// checked in beside this module (`android-template/`); the activity is the
// framework's own NativeActivity, so v1 compiles no Java and links no dex.

/// The `.so` stem Bevy's winit backend loads through NativeActivity's
/// `android.app.lib_name`: the runtime crate's own name, without `lib`.
/// This has to match `blockloom-runtime`'s crate name; the assembly test
/// below pins the spelling.
pub const LIB_NAME: &str = "blockloom_runtime";

/// The checked-in manifest, with `__TOKENS__` for the per-build rows.
const MANIFEST_TEMPLATE: &str = include_str!("android-template/AndroidManifest.xml");

/// Launcher PNG edge lengths per density folder, the standard set.
pub const LAUNCHER_DENSITIES: &[(&str, u32)] = &[
    ("mipmap-mdpi", 48),
    ("mipmap-hdpi", 72),
    ("mipmap-xhdpi", 96),
    ("mipmap-xxhdpi", 144),
    ("mipmap-xxxhdpi", 192),
];

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Fills the manifest template: ids, versions, the label and the `.so`
/// stem. The label is XML-escaped; the id and versions are validated
/// before they get here, so they substitute raw.
pub fn render_manifest(settings: &AndroidSettings, project_name: &str) -> Result<String, String> {
    let id = settings.application_id_for(project_name)?;
    let manifest = MANIFEST_TEMPLATE
        .replace("__APPLICATION_ID__", &id)
        .replace(
            "__VERSION_CODE__",
            &settings.version_code_or_default().to_string(),
        )
        .replace(
            "__VERSION_NAME__",
            &xml_escape(&settings.version_name_or_default()),
        )
        .replace("__APP_LABEL__", &xml_escape(project_name))
        .replace("__LIB_NAME__", LIB_NAME);
    for token in [
        "__APPLICATION_ID__",
        "__VERSION_CODE__",
        "__VERSION_NAME__",
        "__APP_LABEL__",
        "__LIB_NAME__",
    ] {
        if manifest.contains(token) {
            return Err(format!("The manifest template left {token} behind."));
        }
    }
    Ok(manifest)
}

/// One density folder's launcher PNG, from the project icon (or
/// Blockloom's own when the project names none) through the same square
/// fit the desktop icons use.
pub fn launcher_icons(
    project_dir: &Path,
    icon_setting: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let icons = crate::distribution::Icons::load(project_dir, icon_setting)?;
    let source = image::load_from_memory(&icons.png)
        .map_err(|error| format!("the game icon could not be read: {error}"))?;
    let mut folders = Vec::new();
    for (folder, size) in LAUNCHER_DENSITIES {
        let rgba = crate::distribution::square_for_launcher(&source, *size);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|error| format!("couldn't encode the launcher icon: {error}"))?;
        folders.push((folder.to_string(), bytes.into_inner()));
    }
    Ok(folders)
}

// ─── Build-tools paths ─────────────────────────────────────────────────
// Resolved off the SDK row, never PATH: the install flow owns these files.

/// The pinned build-tools dir, e.g. `<sdk>/build-tools/35.0.0`.
pub fn build_tools_dir(config: &AppConfig) -> PathBuf {
    sdk_dir(config).join("build-tools").join(BUILD_TOOLS)
}

fn build_tool(config: &AppConfig, name: &str) -> Result<PathBuf, String> {
    let path = build_tools_dir(config).join(exe(name));
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "{} isn't installed. Run android-install-sdk to repair build-tools {BUILD_TOOLS}.",
            path.display()
        ))
    }
}

/// `android.jar` for the pinned platform, which aapt2 links against.
pub fn android_jar(config: &AppConfig) -> Result<PathBuf, String> {
    let path = sdk_dir(config)
        .join("platforms")
        .join(PLATFORM)
        .join("android.jar");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "{} isn't installed. Run android-install-sdk first.",
            path.display()
        ))
    }
}

/// Every local binary the assembly runs, resolved up front so a missing
/// one stops the build before any staging happens.
pub struct ApkTools {
    pub aapt2: PathBuf,
    pub zipalign: PathBuf,
    pub apksigner: PathBuf,
    pub android_jar: PathBuf,
}

pub fn apk_tools() -> Result<ApkTools, String> {
    apk_tools_for(&load())
}

pub fn apk_tools_for(config: &AppConfig) -> Result<ApkTools, String> {
    Ok(ApkTools {
        aapt2: build_tool(config, "aapt2")?,
        zipalign: build_tool(config, "zipalign")?,
        apksigner: build_tool(config, "apksigner")?,
        android_jar: android_jar(config)?,
    })
}

/// `keytool` rides the JDK, so it is a PATH lookup like `java` itself.
pub fn keytool() -> Result<PathBuf, String> {
    let name = exe("keytool");
    let path = PathBuf::from(&name);
    // A bare name runs through PATH; only accept it when it exists there.
    if std::process::Command::new(&path)
        .arg("-help")
        .output()
        .is_ok()
    {
        return Ok(path);
    }
    Err(format!(
        "No `{name}` on PATH. Install JDK {JDK_MAJOR} first (see android-status)."
    ))
}

// ─── Release signing ─────────────────────────────────────────────────
// A release key is a real identity: its keystore path plus alias live in
// Project settings, but passwords never touch the project file or the app
// config. Each build asks for them; headless builds read the env instead
// (`BLOCKLOOM_ANDROID_STORE_PASS`, and `BLOCKLOOM_ANDROID_KEY_PASS` when
// the key has its own), and an opted-in build keeps them in the OS keyring
// (`android_keyring`, macOS Keychain or Linux Secret Service) for the next
// one. The debug keystore below stays the default for the dev loop, clearly
// marked as such.

/// Env var holding the keystore password for headless release builds.
pub const STORE_PASS_ENV: &str = "BLOCKLOOM_ANDROID_STORE_PASS";
/// Env var holding the key password, when it differs from the store's.
pub const KEY_PASS_ENV: &str = "BLOCKLOOM_ANDROID_KEY_PASS";

/// What the keyring holds for the open project's release key: whether this
/// machine has a scriptable store at all, and which of the two passwords it
/// keeps. The Build dialog reads this to say when typing is optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyringStatus {
    pub available: bool,
    pub store_saved: bool,
    pub key_saved: bool,
}

/// Reads one password from each source in turn: the explicit arg, the env,
/// then the OS keyring. Empty strings never count, from anywhere.
fn password_from(arg: Option<&str>, env: &str, account: Option<&str>) -> Option<String> {
    arg.map(str::trim)
        .filter(|pass| !pass.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var(env)
                .ok()
                .filter(|pass| !pass.trim().is_empty())
        })
        .or_else(|| {
            account.and_then(|account| {
                keyring::Keyring::probe()
                    .and_then(|ring| ring.read(account).ok().flatten())
                    .filter(|pass| !pass.trim().is_empty())
            })
        })
}

/// Whether this machine can keep passwords between builds: macOS Keychain
/// or Linux Secret Service. Windows answers false in v1.
pub fn keyring_available() -> bool {
    keyring::Keyring::probe().is_some()
}

/// What the keyring holds for `settings`: an empty release row reports
/// nothing saved, since there is no key to save for.
pub fn keyring_status_for(settings: &AndroidSettings) -> KeyringStatus {
    let Some(ring) = keyring::Keyring::probe() else {
        return KeyringStatus {
            available: false,
            store_saved: false,
            key_saved: false,
        };
    };
    let (store_account, key_account) = match keyring_accounts(settings) {
        Some(accounts) => accounts,
        None => {
            return KeyringStatus {
                available: true,
                store_saved: false,
                key_saved: false,
            };
        }
    };
    KeyringStatus {
        available: true,
        store_saved: ring.has(&store_account).unwrap_or(false),
        key_saved: ring.has(&key_account).unwrap_or(false),
    }
}

/// The keyring accounts for a release row, or None when the row names no
/// usable key (empty keystore or alias).
fn keyring_accounts(settings: &AndroidSettings) -> Option<(String, String)> {
    let keystore = settings.keystore.trim();
    let alias = settings.key_alias.trim();
    if keystore.is_empty() || alias.is_empty() {
        return None;
    }
    Some((
        keyring::account_for(keystore, alias, keyring::Purpose::Store),
        keyring::account_for(keystore, alias, keyring::Purpose::Key),
    ))
}

/// Keeps the build's passwords in the OS keyring under the release row's
/// accounts, for the next build to read back. Takes the same explicit args
/// the build took and resolves them the same way (args, then env, then what
/// the keyring already holds), so remembered passwords are always the
/// effective ones. Only call with the user's opt-in (the remember checkbox)
/// and, ideally, passwords that just signed something: a typo saved here
/// asks to be typed again nowhere.
pub fn remember_signing(
    settings: &AndroidSettings,
    store_pass: Option<&str>,
    key_pass: Option<&str>,
) -> Result<(), String> {
    let ring = keyring::Keyring::probe()
        .ok_or_else(|| "This machine has no scriptable keyring to remember with.".to_string())?;
    let (store_account, key_account) = keyring_accounts(settings)
        .ok_or_else(|| "Set a release key file plus alias first.".to_string())?;
    let store = password_from(store_pass, STORE_PASS_ENV, Some(&store_account))
        .filter(|pass| !pass.trim().is_empty())
        .ok_or_else(|| "There is no keystore password to remember.".to_string())?;
    ring.write(&store_account, store.trim())?;
    // One entry is enough when the key shares the store's password.
    let key =
        password_from(key_pass, KEY_PASS_ENV, Some(&key_account)).unwrap_or_else(|| store.clone());
    if key.trim() != store.trim() {
        ring.write(&key_account, key.trim())?;
    } else {
        let _ = ring.delete(&key_account);
    }
    Ok(())
}

/// Forgets whatever the keyring keeps for the release row. Answers whether
/// anything was there to forget.
pub fn forget_signing(settings: &AndroidSettings) -> Result<bool, String> {
    let ring = keyring::Keyring::probe()
        .ok_or_else(|| "This machine has no scriptable keyring to forget with.".to_string())?;
    let (store_account, key_account) = keyring_accounts(settings)
        .ok_or_else(|| "Set a release key file plus alias first.".to_string())?;
    Ok(ring.delete(&store_account)? | ring.delete(&key_account)?)
}

/// What an APK signs with: the dev-loop debug key, or the project's own
/// release key with its passwords resolved for this build only.
pub struct Signing {
    pub keystore: PathBuf,
    pub alias: Option<String>,
    pub store_pass: String,
    pub key_pass: String,
    pub release: bool,
}

impl std::fmt::Debug for Signing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Passwords never reach logs: only which key and which mode.
        f.debug_struct("Signing")
            .field("keystore", &self.keystore)
            .field("alias", &self.alias)
            .field("release", &self.release)
            .finish()
    }
}

impl Signing {
    pub fn label(&self) -> &'static str {
        if self.release { "release" } else { "debug" }
    }
}

/// Picks the key for `settings`: the debug keystore when no release key is
/// set, else the project's keystore plus alias with passwords from the
/// explicit args, the env, then the OS keyring. Refuses a release row that
/// names nothing usable, with the fix attached.
pub fn resolve_signing(
    settings: &AndroidSettings,
    store_pass: Option<&str>,
    key_pass: Option<&str>,
) -> Result<Signing, String> {
    if settings.keystore.trim().is_empty() {
        let keystore = ensure_debug_keystore()?;
        return Ok(Signing {
            keystore,
            alias: Some("blockloom-debug".to_string()),
            store_pass: DEBUG_KEYSTORE_PASSWORD.to_string(),
            key_pass: DEBUG_KEYSTORE_PASSWORD.to_string(),
            release: false,
        });
    }
    let keystore = PathBuf::from(settings.keystore.trim());
    if !keystore.is_file() {
        return Err(format!(
            "The release keystore {} isn't there. Pick the key file again in Project settings, or clear it to sign debug.",
            keystore.display()
        ));
    }
    let alias = settings.key_alias.trim();
    if alias.is_empty() {
        return Err(
            "The release keystore names no alias. Add the key's alias in Project settings."
                .to_string(),
        );
    }
    let accounts = keyring_accounts(settings);
    let store_pass = password_from(
        store_pass,
        STORE_PASS_ENV,
        accounts.as_ref().map(|(store, _)| store.as_str()),
    )
    .ok_or_else(|| {
        format!(
            "The release key needs its keystore password: type it in the Build dialog, save it in the system keyring, or set {STORE_PASS_ENV} headless."
        )
    })?;
    let key_pass = password_from(
        key_pass,
        KEY_PASS_ENV,
        accounts.as_ref().map(|(_, key)| key.as_str()),
    )
    .unwrap_or_else(|| store_pass.clone());
    Ok(Signing {
        keystore,
        alias: Some(alias.to_string()),
        store_pass,
        key_pass,
        release: true,
    })
}

/// Makes a release key: a new RSA keypair under `alias` in the keystore at
/// `path`, creating the file when it names nothing yet. Answers the aliases
/// the file holds afterwards. Passwords come from the args, the env, then
/// the OS keyring; keytool wants at least 6 characters for each.
pub fn create_keystore(
    path: &Path,
    alias: &str,
    store_pass: Option<&str>,
    key_pass: Option<&str>,
) -> Result<Vec<String>, String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err("Name the key's alias, so the build knows which key signs.".to_string());
    }
    let path_text = path.to_string_lossy();
    let store_account = keyring::account_for(&path_text, alias, keyring::Purpose::Store);
    let key_account = keyring::account_for(&path_text, alias, keyring::Purpose::Key);
    let store_pass = password_from(store_pass, STORE_PASS_ENV, Some(&store_account)).ok_or_else(|| {
        format!("A new key needs a keystore password: type one in, or set {STORE_PASS_ENV} headless.")
    })?;
    if store_pass.len() < 6 {
        return Err("The keystore password needs at least 6 characters.".to_string());
    }
    let key_pass = password_from(key_pass, KEY_PASS_ENV, Some(&key_account))
        .unwrap_or_else(|| store_pass.clone());
    if key_pass.len() < 6 {
        return Err("The key password needs at least 6 characters.".to_string());
    }
    if path.is_file()
        && keystore_aliases(path, &store_pass)?
            .iter()
            .any(|name| name == alias)
    {
        return Err(format!(
            "{} already holds a key named \"{alias}\". Pick another alias.",
            path.display()
        ));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let output = std::process::Command::new(keytool()?)
        .arg("-genkeypair")
        .arg("-keystore")
        .arg(path)
        .arg("-alias")
        .arg(alias)
        .arg("-keyalg")
        .arg("RSA")
        .arg("-keysize")
        .arg("2048")
        .arg("-validity")
        .arg("10950")
        .arg("-storepass")
        .arg(&store_pass)
        .arg("-keypass")
        .arg(&key_pass)
        .arg("-dname")
        .arg(format!("CN={alias}"))
        .output()
        .map_err(|e| format!("Couldn't run keytool: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "keytool couldn't create the key: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    keystore_aliases(path, &store_pass)
}

/// The key aliases a keystore file holds, for the create step's duplicate
/// check. Needs the store password; a wrong one reads as keytool failing.
pub fn keystore_aliases(path: &Path, store_pass: &str) -> Result<Vec<String>, String> {
    let output = std::process::Command::new(keytool()?)
        .arg("-list")
        .arg("-keystore")
        .arg(path)
        .arg("-storepass")
        .arg(store_pass)
        .output()
        .map_err(|e| format!("Couldn't run keytool: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "keytool couldn't list {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    // One `alias, date, PrivateKeyEntry,` line per key, plus a header.
    let mut aliases = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let first = line.split(',').next().unwrap_or("").trim();
        if first.is_empty()
            || first.contains(' ')
            || first.eq_ignore_ascii_case("keystore")
            || first.eq_ignore_ascii_case("keystore type:")
        {
            continue;
        }
        aliases.push(first.to_string());
    }
    Ok(aliases)
}

// ─── Debug keystore ────────────────────────────────────────────────────
// One keystore for every dev install, clearly debug-only. Release signing
// above takes a keystore plus alias in Project settings and asks for
// passwords on each build; the debug key is what an empty release row
// falls back to.

/// The debug password Android tooling has used forever. Public knowledge,
/// which is exactly why nothing release ever signs with it.
pub const DEBUG_KEYSTORE_PASSWORD: &str = "android";

/// `blockloom-debug.keystore` in the data dir, beside `android.json`.
pub fn debug_keystore_path() -> PathBuf {
    crate::project::data_dir().join("blockloom-debug.keystore")
}

pub fn ensure_debug_keystore_at(keystore: &Path) -> Result<(), String> {
    if keystore.is_file() {
        return Ok(());
    }
    if let Some(dir) = keystore.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let status = std::process::Command::new(keytool()?)
        .arg("-genkeypair")
        .arg("-keystore")
        .arg(keystore)
        .arg("-alias")
        .arg("blockloom-debug")
        .arg("-keyalg")
        .arg("RSA")
        .arg("-keysize")
        .arg("2048")
        .arg("-validity")
        .arg("10950")
        .arg("-storepass")
        .arg(DEBUG_KEYSTORE_PASSWORD)
        .arg("-keypass")
        .arg(DEBUG_KEYSTORE_PASSWORD)
        .arg("-dname")
        .arg("CN=Blockloom Debug, OU=Blockloom, O=Blockloom, C=US")
        .output()
        .map_err(|e| format!("Couldn't run keytool: {e}"))?;
    if !status.status.success() {
        return Err(format!(
            "keytool couldn't create the debug keystore: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    Ok(())
}

pub fn ensure_debug_keystore() -> Result<PathBuf, String> {
    let path = debug_keystore_path();
    ensure_debug_keystore_at(&path)?;
    Ok(path)
}

// ─── Assembly ──────────────────────────────────────────────────────────

/// What goes into the APK: the rendered manifest, one PNG per density
/// folder, the staged game files (pack plus assets, atlas, bakes) and one
/// `.so` per ABI - the runtime plus every script and the native logic.
pub struct ApkContents {
    pub manifest: String,
    pub icons: Vec<(String, Vec<u8>)>,
    /// The game folder as `build.rs` staged it, mounted at `assets/`.
    pub assets_dir: PathBuf,
    /// (`arm64-v8a`, path to `libblockloom_runtime.so`) and friends.
    pub native_libs: Vec<(String, PathBuf)>,
}

/// A finished APK: where it landed and what it carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApkReport {
    pub apk: PathBuf,
    pub size: u64,
    pub application_id: String,
    pub version_name: String,
    pub version_code: u32,
    pub abis: Vec<String>,
    /// `debug` for the dev-loop key, `release` for the project's own key.
    pub signed: String,
}

fn run_tool(tool: &Path, args: &[String]) -> Result<String, String> {
    let output = std::process::Command::new(tool)
        .args(args)
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", tool.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            tool.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn aapt2_compile_args(res: &Path, out: &Path) -> Vec<String> {
    [
        "compile",
        "--dir",
        &res.to_string_lossy(),
        "-o",
        &out.to_string_lossy(),
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect()
}

pub fn aapt2_link_args(
    tools: &ApkTools,
    manifest: &Path,
    compiled_res: &Path,
    assets: &Path,
    out: &Path,
) -> Vec<String> {
    [
        "link",
        "-o",
        &out.to_string_lossy(),
        "-I",
        &tools.android_jar.to_string_lossy(),
        "--manifest",
        &manifest.to_string_lossy(),
        "-A",
        &assets.to_string_lossy(),
        &compiled_res.to_string_lossy(),
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect()
}

/// Packs every `.so` under `lib/<abi>/`, stored uncompressed. The file name
/// is kept as is: Android loads `lib<name>.so` for `android.app.lib_name`
/// `<name>`, so stripping the prefix would leave an unloadable entry. The
/// manifest sets `extractNativeLibs`, so the installer unpacks them itself
/// and no page-alignment dance is needed before zipalign.
pub fn inject_native_libs(apk: &Path, libs: &[(String, PathBuf)]) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(apk)
        .map_err(|e| format!("{}: {e}", apk.display()))?;
    let mut archive =
        zip::ZipWriter::new_append(file).map_err(|e| format!("{}: {e}", apk.display()))?;
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (abi, so) in libs {
        let file_name = so
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{} has no file name", so.display()))?;
        let name = format!("lib/{abi}/{file_name}");
        archive
            .start_file(&name, options)
            .map_err(|e| format!("{apk}: {e}", apk = apk.display()))?;
        let mut source = std::fs::File::open(so).map_err(|e| format!("{}: {e}", so.display()))?;
        std::io::copy(&mut source, &mut archive).map_err(|e| format!("{}: {e}", so.display()))?;
    }
    archive
        .finish()
        .map_err(|e| format!("{}: {e}", apk.display()))?;
    Ok(())
}

pub fn zipalign_args(from: &Path, to: &Path) -> Vec<String> {
    ["-f", "4", &from.to_string_lossy(), &to.to_string_lossy()]
        .iter()
        .map(|arg| arg.to_string())
        .collect()
}

pub fn apksigner_args(signing: &Signing, from: &Path, to: &Path) -> Vec<String> {
    let mut args = vec![
        "sign".to_string(),
        "--ks".to_string(),
        signing.keystore.to_string_lossy().into_owned(),
        "--ks-pass".to_string(),
        format!("pass:{}", signing.store_pass),
    ];
    if let Some(alias) = &signing.alias {
        args.push("--ks-key-alias".to_string());
        args.push(alias.clone());
    }
    // A key with its own password signs under `--key-pass`; one sharing
    // the store's needs no extra flag.
    if signing.key_pass != signing.store_pass {
        args.push("--key-pass".to_string());
        args.push(format!("pass:{}", signing.key_pass));
    }
    args.push("--out".to_string());
    args.push(to.to_string_lossy().into_owned());
    args.push(from.to_string_lossy().into_owned());
    args
}

/// Links, packs, aligns and signs: `work` is scratch space beside the
/// build, `dest` the finished APK. The unsigned middle steps stay in
/// `work` for debugging; only `dest` ships.
pub fn assemble_apk(
    contents: &ApkContents,
    work: &Path,
    tools: &ApkTools,
    signing: &Signing,
    dest: &Path,
) -> Result<ApkReport, String> {
    if contents.native_libs.is_empty() {
        return Err("Nothing to run: the build produced no runtime library.".to_string());
    }
    std::fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let manifest = work.join("AndroidManifest.xml");
    std::fs::write(&manifest, &contents.manifest)
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    let res = work.join("res");
    for (folder, png) in &contents.icons {
        let dir = res.join(folder);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(dir.join("ic_launcher.png"), png)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    let compiled_res = work.join("compiled_res.zip");
    run_tool(&tools.aapt2, &aapt2_compile_args(&res, &compiled_res))?;
    let unaligned = work.join("unaligned.apk");
    run_tool(
        &tools.aapt2,
        &aapt2_link_args(
            tools,
            &manifest,
            &compiled_res,
            &contents.assets_dir,
            &unaligned,
        ),
    )?;
    inject_native_libs(&unaligned, &contents.native_libs)?;
    let aligned = work.join("aligned.apk");
    run_tool(&tools.zipalign, &zipalign_args(&unaligned, &aligned))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    run_tool(&tools.apksigner, &apksigner_args(signing, &aligned, dest))?;

    let mut abis: Vec<String> = contents
        .native_libs
        .iter()
        .map(|(abi, _)| abi.clone())
        .collect();
    abis.sort();
    abis.dedup();
    Ok(ApkReport {
        size: std::fs::metadata(dest).map(|meta| meta.len()).unwrap_or(0),
        apk: dest.to_path_buf(),
        application_id: String::new(),
        version_name: String::new(),
        version_code: 0,
        abis,
        signed: signing.label().to_string(),
    })
}

// ─── The runtime `.so` ─────────────────────────────────────────────────
// Android has no staged player: the desktop NDK cross-builds
// `blockloom-runtime` into `lib/<abi>/libblockloom_runtime.so`. A build
// farm may hand one over instead through the env below.

/// Hand the build a prebuilt runtime library instead of compiling one.
/// Per-triple wins over the bare name when both are set.
pub fn runtime_so_override(triple: &str) -> Option<PathBuf> {
    let per_triple = format!(
        "BLOCKLOOM_ANDROID_RUNTIME_SO_{}",
        triple.to_uppercase().replace('-', "_")
    );
    for var in [per_triple, "BLOCKLOOM_ANDROID_RUNTIME_SO".to_string()] {
        if let Some(path) = std::env::var_os(&var) {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// The workspace root: the ancestor of the working dir holding
/// `blockloom-runtime/Cargo.toml`. A packaged install has no source tree,
/// so there the env override above is the only way.
pub fn workspace_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("blockloom-runtime/Cargo.toml").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Linker plus C toolchain for `triple`, from the installed NDK: rustc
/// links scripts and logic directly (no cargo), and cargo needs the same
/// rows to cross-build the runtime. Merges into the child env, never the
/// process's own.
pub fn ndk_cargo_env(config: &AppConfig, triple: &str) -> Option<Vec<(String, String)>> {
    let (_, clang) = cargo_linker_env(config, triple)?;
    let prebuilt = clang.parent()?;
    let lower = triple.replace('-', "_");
    let cc = prebuilt.join(exe(&format!("{triple}{MIN_SDK}-clang")));
    let cxx = prebuilt.join(exe(&format!("{triple}{MIN_SDK}-clang++")));
    let ar = prebuilt.join(exe("llvm-ar"));
    let upper = triple.to_uppercase().replace('-', "_");
    let mut env = vec![(
        format!("CARGO_TARGET_{upper}_LINKER"),
        clang.to_string_lossy().into_owned(),
    )];
    for (tool, path) in [("CC", cc), ("CXX", cxx), ("AR", ar)] {
        if path.is_file() {
            env.push((
                format!("{tool}_{lower}"),
                path.to_string_lossy().into_owned(),
            ));
        }
    }
    Some(env)
}

/// Just the linker rustc's `-C linker=` wants for `triple`, if the NDK
/// has one. Scripts and native logic compile through direct rustc calls,
/// so they take this path rather than the cargo env above.
pub fn ndk_linker_for(triple: &str) -> Option<PathBuf> {
    ndk_linker_for_config(&load(), triple)
}

pub fn ndk_linker_for_config(config: &AppConfig, triple: &str) -> Option<PathBuf> {
    cargo_linker_env(config, triple).map(|(_, path)| path)
}

/// Compiles `blockloom-runtime` for `triple` (release, SDR-only feature
/// set: no Solari, no KTX2/Basis - PNG/JPEG like web) and answers where
/// `libblockloom_runtime.so` landed. First run needs the network for the
/// target's crates, like the SDK install did.
pub fn build_runtime_so(config: &AppConfig, triple: &str) -> Result<PathBuf, String> {
    let root = workspace_root().ok_or_else(|| {
        "The Blockloom source tree wasn't found above the working dir, so the runtime can't be cross-built. Set BLOCKLOOM_ANDROID_RUNTIME_SO to a prebuilt libblockloom_runtime.so instead.".to_string()
    })?;
    let Some(env) = ndk_cargo_env(config, triple) else {
        return Err(format!(
            "No NDK linker for {triple}. Run android-install-sdk first."
        ));
    };
    let mut command = std::process::Command::new("cargo");
    command
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(triple)
        .arg("-p")
        .arg("blockloom-runtime")
        .arg("--lib")
        .arg("--no-default-features")
        .current_dir(&root);
    for (key, value) in &env {
        command.env(key, value);
    }
    let output = command
        .output()
        .map_err(|e| format!("Couldn't run cargo: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo couldn't cross-build the runtime for {triple}:\n{}",
            trim_cargo_output(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let so = target_dir
        .join(triple)
        .join("release")
        .join("libblockloom_runtime.so");
    if so.is_file() {
        Ok(so)
    } else {
        Err(format!(
            "cargo reported success but {} never landed.",
            so.display()
        ))
    }
}

/// Keeps the failure out of a cargo stderr: progress lines (`Compiling`,
/// `Finished`, downloads) never explain a failure, so they go, and what is
/// left is capped to the tail where the error and its summary sit. Without
/// this the Build dialog shows hundreds of red progress lines.
fn trim_cargo_output(stderr: &str) -> String {
    let kept: Vec<&str> = stderr
        .lines()
        .filter(|line| {
            let progress = line.trim_start();
            ![
                "Compiling ",
                "Finished ",
                "Running ",
                "Downloading ",
                "Downloaded ",
                "Locking ",
                "Updating ",
                "Adding ",
                "Removing ",
                "Checking ",
                "Fresh ",
                "Dirty ",
            ]
            .iter()
            .any(|prefix| progress.starts_with(prefix))
        })
        .collect();
    const TAIL: usize = 40;
    let tail = if kept.len() > TAIL {
        &kept[kept.len() - TAIL..]
    } else {
        &kept[..]
    };
    let trimmed = tail.join("\n").trim().to_string();
    if trimmed.is_empty() {
        "cargo failed with no output.".to_string()
    } else {
        trimmed
    }
}

/// A prebuilt `.so` when the env names one, else a fresh cross-build.
pub fn runtime_so_for(triple: &str) -> Result<PathBuf, String> {
    if let Some(so) = runtime_so_override(triple) {
        return Ok(so);
    }
    build_runtime_so(&load(), triple)
}

// ─── Install ───────────────────────────────────────────────────────────

/// What an `adb install` plus launch did: the started component and the
/// device it went to (empty when adb picked the only one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApkInstall {
    pub component: String,
    pub device: String,
}

/// `adb install` plus `am start` on `device` (or the only device when
/// unset). Answers the launched component.
pub fn install_apk(
    apk: &Path,
    application_id: &str,
    device: Option<&str>,
) -> Result<ApkInstall, String> {
    let config = load();
    let adb = sdk_dir(&config).join("platform-tools").join(exe("adb"));
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    install_apk_with_adb(&adb, apk, application_id, device)
}

fn install_apk_with_adb(
    adb: &Path,
    apk: &Path,
    application_id: &str,
    device: Option<&str>,
) -> Result<ApkInstall, String> {
    let mut install = std::process::Command::new(adb);
    if let Some(serial) = device {
        install.arg("-s").arg(serial);
    }
    let output = install
        .arg("install")
        .arg("-r")
        .arg(apk)
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "adb install failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let component = format!("{application_id}/android.app.NativeActivity");
    let mut start = std::process::Command::new(adb);
    if let Some(serial) = device {
        start.arg("-s").arg(serial);
    }
    let output = start
        .arg("shell")
        .arg("am")
        .arg("start")
        .arg("-n")
        .arg(&component)
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "The APK installed but wouldn't start: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    // A fresh buffer means the first logcat poll after this reads only the
    // new run. Failure just makes that poll noisier, never fails a launch.
    let _ = clear_logcat_with_adb(adb, device);
    Ok(ApkInstall {
        component,
        device: device.unwrap_or_default().to_string(),
    })
}

/// What one `adb logcat` dump kept: the matching lines plus whether any
/// reads as a native crash, which is what a smoke test fails on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Logcat {
    pub lines: Vec<String>,
    pub panics: Vec<String>,
}

/// Dumps the device log (`adb logcat -d`) and keeps the lines naming
/// `needle` - the runtime's `blockloom:` markers - plus any Rust panic or
/// fatal exception anywhere, which fail a smoke run. One shot, not a
/// stream: `logcat_tail` below polls this into the RunLog; this stays for
/// the dev loop and `just android-smoke`.
pub fn logcat(device: Option<&str>, needle: &str) -> Result<Logcat, String> {
    logcat_with_adb(&adb_path(), device, needle)
}

/// Clears the device log buffer (`adb logcat -c`), so a later dump reads
/// only what happened after. Best effort: the install step calls this and
/// ignores a failure, leaving a noisier first poll rather than no install.
pub fn clear_logcat(device: Option<&str>) -> Result<(), String> {
    clear_logcat_with_adb(&adb_path(), device)
}

/// Dumps like `logcat`, then clears the buffer so the next poll reads only
/// new lines. The Build dialog polls this after an install: each answer
/// lands in the RunLog, which is the streaming half of the signing bullet.
pub fn logcat_tail(device: Option<&str>, needle: &str) -> Result<Logcat, String> {
    let adb = adb_path();
    logcat_tail_with_adb(&adb, device, needle)
}

fn logcat_tail_with_adb(adb: &Path, device: Option<&str>, needle: &str) -> Result<Logcat, String> {
    let dumped = logcat_with_adb(adb, device, needle)?;
    // A failed clear only repeats lines next poll; the dump already won.
    let _ = clear_logcat_with_adb(adb, device);
    Ok(dumped)
}

fn adb_path() -> PathBuf {
    sdk_dir(&load()).join("platform-tools").join(exe("adb"))
}

fn clear_logcat_with_adb(adb: &Path, device: Option<&str>) -> Result<(), String> {
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let mut clear = std::process::Command::new(adb);
    if let Some(serial) = device {
        clear.arg("-s").arg(serial);
    }
    let output = clear
        .arg("logcat")
        .arg("-c")
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "adb logcat -c failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn logcat_with_adb(adb: &Path, device: Option<&str>, needle: &str) -> Result<Logcat, String> {
    if !adb.is_file() {
        return Err("No adb: install platform-tools in App Settings first.".to_string());
    }
    let mut dump = std::process::Command::new(adb);
    if let Some(serial) = device {
        dump.arg("-s").arg(serial);
    }
    let output = dump
        .arg("logcat")
        .arg("-d")
        .output()
        .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
    if !output.status.success() {
        return Err(format!(
            "adb logcat failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let needle = needle.to_lowercase();
    let mut lines = Vec::new();
    let mut panics = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let lower = line.to_lowercase();
        if lower.contains("panicked")
            || lower.contains("fatal exception")
            || lower.contains("fatal:")
        {
            panics.push(logcat_message(line).to_string());
        }
        if !needle.is_empty() && lower.contains(&needle) {
            lines.push(logcat_message(line).to_string());
        }
    }
    Ok(Logcat { lines, panics })
}

/// Strips the `adb logcat` threadtime prefix (`09-28 22:12:31.290 19629
/// 19629 D `), leaving `tag: message`. Anything not shaped like a stamp
/// passes through untouched, so other formats never lose text.
fn logcat_message(line: &str) -> &str {
    let mut rest = line;
    for _ in 0..5 {
        rest = match rest.find(char::is_whitespace) {
            Some(index) => rest[index..].trim_start(),
            None => return line,
        };
    }
    if rest.is_empty() {
        return line;
    }
    let head = &line[..line.len() - rest.len()];
    let mut fields = head.split_whitespace();
    let stamped = match (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) {
        (Some(date), Some(time), Some(pid), Some(tid), Some(prio)) => {
            date.len() == 5
                && date.as_bytes()[2] == b'-'
                && time.contains(':')
                && pid.bytes().all(|b| b.is_ascii_digit())
                && tid.bytes().all(|b| b.is_ascii_digit())
                && prio.len() == 1
        }
        _ => false,
    };
    if stamped { rest } else { line }
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
    #[cfg(unix)]
    fn each_android_triple_links_through_its_own_wrapper() {
        let root = temp_root("linkers");
        let bin = root
            .join("toolchains/llvm/prebuilt")
            .join(host_tag())
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        stub_tool(
            &bin,
            &format!("{ARM64_TRIPLE}{MIN_SDK}-clang"),
            "#!/bin/sh\n",
        );
        stub_tool(
            &bin,
            &format!("{EMULATOR_TRIPLE}{MIN_SDK}-clang"),
            "#!/bin/sh\n",
        );
        let config = AppConfig {
            ndk_path: Some(root.clone()),
            ..AppConfig::default()
        };
        // Each triple takes its own wrapper: an arm64 wrapper cannot link
        // x86_64 objects, which is what broke the emulator row's build.
        let (_, arm64) = cargo_linker_env(&config, ARM64_TRIPLE).unwrap();
        assert!(arm64.to_string_lossy().contains(ARM64_TRIPLE), "{arm64:?}");
        let (_, emulator) = cargo_linker_env(&config, EMULATOR_TRIPLE).unwrap();
        assert!(
            emulator.to_string_lossy().contains(EMULATOR_TRIPLE),
            "{emulator:?}"
        );
        // One wrapper left still links the other triple, as before.
        std::fs::remove_file(bin.join(format!("{EMULATOR_TRIPLE}{MIN_SDK}-clang"))).unwrap();
        let (_, fallback) = cargo_linker_env(&config, EMULATOR_TRIPLE).unwrap();
        assert!(
            fallback.to_string_lossy().contains(ARM64_TRIPLE),
            "{fallback:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
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
    fn android_settings_default_to_name_and_one() {
        let settings = AndroidSettings::default();
        assert_eq!(
            settings.application_id_for("Pond Game").unwrap(),
            "com.blockloom.game.pond_game"
        );
        assert_eq!(settings.version_code_or_default(), 1);
        assert_eq!(settings.version_name_or_default(), "1.0.0");

        let explicit = AndroidSettings {
            application_id: "com.example.pond".to_string(),
            version_code: 7,
            version_name: "2.1".to_string(),
            ..AndroidSettings::default()
        };
        assert_eq!(
            explicit.application_id_for("Pond Game").unwrap(),
            "com.example.pond"
        );
        assert_eq!(explicit.version_code_or_default(), 7);
        assert_eq!(explicit.version_name_or_default(), "2.1");

        let bad = AndroidSettings {
            application_id: "not an id".to_string(),
            ..AndroidSettings::default()
        };
        assert!(bad.application_id_for("Pond Game").is_err());
        let zero = AndroidSettings {
            version_code: 0,
            version_name: "  ".to_string(),
            ..AndroidSettings::default()
        };
        assert_eq!(zero.version_code_or_default(), 1);
        assert_eq!(zero.version_name_or_default(), "1.0.0");
    }

    #[test]
    fn the_manifest_names_the_app_and_pins_the_sdks() {
        let settings = AndroidSettings {
            application_id: String::new(),
            version_code: 3,
            version_name: "1.2.3".to_string(),
            ..AndroidSettings::default()
        };
        let manifest = render_manifest(&settings, "Pond & Pebbles").unwrap();
        assert!(
            manifest.contains("package=\"com.blockloom.game.pond___pebbles\""),
            "{manifest}"
        );
        assert!(manifest.contains("android:versionCode=\"3\""), "{manifest}");
        assert!(
            manifest.contains("android:versionName=\"1.2.3\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("android:label=\"Pond &amp; Pebbles\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("android:minSdkVersion=\"29\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("android:targetSdkVersion=\"35\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("android.app.NativeActivity"),
            "{manifest}"
        );
        assert!(manifest.contains("android:exported=\"true\""), "{manifest}");
        // No classes.dex ships in v1, so the manifest must say the APK has
        // no dex code of its own: without this the installer rejects it as
        // codeless (INSTALL_FAILED_INVALID_APK, "code is missing").
        assert!(manifest.contains("android:hasCode=\"false\""), "{manifest}");
        assert!(
            manifest.contains(&format!("android:value=\"{LIB_NAME}\"")),
            "{manifest}"
        );
        assert!(!manifest.contains("INTERNET"), "{manifest}");
        assert!(!manifest.contains("uses-permission"), "{manifest}");
        for token in ["__APPLICATION_ID__", "__APP_LABEL__", "__LIB_NAME__"] {
            assert!(!manifest.contains(token), "{manifest}");
        }
    }

    #[test]
    fn a_bad_application_id_stops_the_manifest() {
        let settings = AndroidSettings {
            application_id: "no good".to_string(),
            ..AndroidSettings::default()
        };
        assert!(render_manifest(&settings, "Pond").is_err());
    }

    #[test]
    fn launcher_icons_cover_every_density() {
        let root = temp_root("icons");
        let icons = launcher_icons(&root, "").unwrap();
        assert_eq!(icons.len(), LAUNCHER_DENSITIES.len());
        for ((folder, size), (got_folder, png)) in LAUNCHER_DENSITIES.iter().zip(&icons) {
            assert_eq!(got_folder, folder);
            let image = image::load_from_memory(png).unwrap();
            assert_eq!(image.width(), *size, "{folder}");
            assert_eq!(image.height(), *size, "{folder}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn build_tools_count_only_with_their_assembler() {
        let root = temp_root("tools-present");
        let sdk = root.join("sdk");
        // A stopped download leaves the version dir behind with nothing in it.
        std::fs::create_dir_all(sdk.join("build-tools").join(BUILD_TOOLS)).unwrap();
        assert!(!package_present(&sdk, "build-tools;35.0.0"));
        std::fs::write(
            sdk.join("build-tools").join(BUILD_TOOLS).join(exe("aapt2")),
            b"fake",
        )
        .unwrap();
        assert!(package_present(&sdk, "build-tools;35.0.0"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tool_paths_resolve_off_the_sdk_row() {
        let root = temp_root("tool-paths");
        let sdk = root.join("sdk");
        let bin = sdk.join("build-tools").join(BUILD_TOOLS);
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(sdk.join("platforms").join(PLATFORM)).unwrap();
        let config = AppConfig {
            sdk_path: Some(sdk.clone()),
            ndk_path: None,
            licenses_accepted: false,
        };
        assert!(apk_tools_for(&config).is_err());
        for tool in ["aapt2", "zipalign", "apksigner"] {
            std::fs::write(bin.join(exe(tool)), b"fake").unwrap();
        }
        // android.jar still missing.
        assert!(apk_tools_for(&config).is_err());
        std::fs::write(
            sdk.join("platforms").join(PLATFORM).join("android.jar"),
            b"fake",
        )
        .unwrap();
        let tools = apk_tools_for(&config).unwrap();
        assert_eq!(tools.aapt2, bin.join(exe("aapt2")));
        assert_eq!(tools.android_jar, android_jar(&config).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tool_args_spell_the_manual_assembly() {
        let root = temp_root("tool-args");
        let tools = ApkTools {
            aapt2: PathBuf::from("/sdk/aapt2"),
            zipalign: PathBuf::from("/sdk/zipalign"),
            apksigner: PathBuf::from("/sdk/apksigner"),
            android_jar: PathBuf::from("/sdk/android.jar"),
        };
        let res = root.join("res");
        let compiled = root.join("compiled.zip");
        assert_eq!(
            aapt2_compile_args(&res, &compiled),
            vec![
                "compile",
                "--dir",
                &res.to_string_lossy(),
                "-o",
                &compiled.to_string_lossy()
            ]
        );
        let link = aapt2_link_args(
            &tools,
            &root.join("AndroidManifest.xml"),
            &compiled,
            &root.join("assets"),
            &root.join("unaligned.apk"),
        );
        assert_eq!(link[0], "link");
        assert!(link.contains(&"-A".to_string()));
        assert!(link.contains(&"/sdk/android.jar".to_string()));
        let align = zipalign_args(&root.join("in.apk"), &root.join("out.apk"));
        assert_eq!(align[0..2], ["-f", "4"]);
        let signing = Signing {
            keystore: root.join("debug.keystore"),
            alias: Some("blockloom-debug".to_string()),
            store_pass: DEBUG_KEYSTORE_PASSWORD.to_string(),
            key_pass: DEBUG_KEYSTORE_PASSWORD.to_string(),
            release: false,
        };
        let sign = apksigner_args(&signing, &root.join("in.apk"), &root.join("out.apk"));
        assert_eq!(sign[0], "sign");
        assert!(sign.contains(&"pass:android".to_string()), "{sign:?}");
        assert!(sign.contains(&"blockloom-debug".to_string()), "{sign:?}");
        // A key with its own password signs under an extra flag.
        let separate = Signing {
            key_pass: "key-secret".to_string(),
            release: true,
            ..signing
        };
        let separate = apksigner_args(&separate, &root.join("in.apk"), &root.join("out.apk"));
        assert!(separate.contains(&"--key-pass".to_string()), "{separate:?}");
        assert!(
            separate.contains(&"pass:key-secret".to_string()),
            "{separate:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn native_libs_land_uncompressed_under_lib() {
        let root = temp_root("inject");
        let apk = root.join("base.apk");
        {
            let file = std::fs::File::create(&apk).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file(
                    "AndroidManifest.xml",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            use std::io::Write;
            writer.write_all(b"<manifest/>").unwrap();
            writer.finish().unwrap();
        }
        let runtime = root.join("libblockloom_runtime.so");
        std::fs::write(&runtime, vec![0x7fu8; 100]).unwrap();
        let script = root.join("libplayer.so");
        std::fs::write(&script, b"script").unwrap();
        inject_native_libs(
            &apk,
            &[
                ("arm64-v8a".to_string(), runtime),
                ("arm64-v8a".to_string(), script),
            ],
        )
        .unwrap();
        let file = std::fs::File::open(&apk).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        // File names kept whole: NativeActivity loads `lib<name>.so` for
        // `android.app.lib_name` `<name>`, so the prefix has to survive.
        for name in [
            "lib/arm64-v8a/libblockloom_runtime.so",
            "lib/arm64-v8a/libplayer.so",
        ] {
            let entry = archive.by_name(name).unwrap();
            assert_eq!(
                entry.compression(),
                zip::CompressionMethod::Stored,
                "{name}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn the_install_goes_through_adb_then_am_start() {
        let root = temp_root("install");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = root.join("calls.log");
        let adb = stub_tool(
            &bin,
            "adb",
            &format!(
                "#!/bin/sh\nlog=\"{}\"\nfor a in \"$@\"; do printf '%s ' \"$a\" >> \"$log\"; done\nprintf '\\n' >> \"$log\"\n",
                log.display()
            ),
        );
        let apk = root.join("pond.apk");
        std::fs::write(&apk, b"fake-apk").unwrap();
        let installed =
            install_apk_with_adb(&adb, &apk, "com.blockloom.game.pond", Some("emulator-5554"))
                .unwrap();
        assert_eq!(
            installed.component,
            "com.blockloom.game.pond/android.app.NativeActivity"
        );
        assert_eq!(installed.device, "emulator-5554");
        let calls = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert!(lines[0].contains("-s emulator-5554 install -r"), "{calls}");
        assert!(lines[0].contains("pond.apk"), "{calls}");
        assert!(
            lines[1]
                .contains("shell am start -n com.blockloom.game.pond/android.app.NativeActivity"),
            "{calls}"
        );
        // A fresh buffer after launch, so the first logcat poll reads only
        // the new run; a failed clear would only make it noisier.
        assert_eq!(lines.len(), 3, "{calls}");
        assert!(lines[2].contains("logcat -c"), "{calls}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn logcat_keeps_markers_and_calls_out_panics() {
        let root = temp_root("logcat");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let adb = stub_tool(
            &bin,
            "adb",
            "#!/bin/sh\necho '01-01 00:00:01.000  123  123 I blockloom: run started'\necho '01-01 00:00:02.000  123  123 I blockloom: actors {\"a\":[1,2]}'\necho '01-01 00:00:03.000  123  123 E AndroidRuntime: FATAL EXCEPTION: main'\necho '01-01 00:00:04.000  123  123 I SomeTag: unrelated line'\necho '01-01 00:00:05.000  123  123 D nativeloader: Load /data/app/com.blockloom.game.pond/lib.so ok'\n",
        );
        // The stub ignores its argv and prints a fixed dump: what matters is
        // the filtering, not the adb invocation (covered by the install test).
        // The needle needs its colon: the package path alone must not match.
        let dumped = logcat_with_adb(&adb, None, "blockloom:").unwrap();
        assert_eq!(
            dumped.lines,
            vec![
                "blockloom: run started".to_string(),
                "blockloom: actors {\"a\":[1,2]}".to_string(),
            ],
            "{dumped:?}"
        );
        assert_eq!(dumped.panics.len(), 1, "{dumped:?}");
        assert!(dumped.panics[0].contains("FATAL EXCEPTION"), "{dumped:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn logcat_strips_only_threadtime_stamps() {
        assert_eq!(
            logcat_message("09-28 22:12:31.290 19629 19629 D nativeloader: ok"),
            "nativeloader: ok"
        );
        assert_eq!(
            logcat_message("blockloom: run started"),
            "blockloom: run started"
        );
        assert_eq!(logcat_message(""), "");
        assert_eq!(logcat_message("   "), "   ");
    }

    #[test]
    #[cfg(unix)]
    fn avd_names_list_one_per_line() {
        let root = temp_root("avds");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let emulator = stub_tool(
            &bin,
            "emulator",
            "#!/bin/sh\necho 'blockloom'\necho ''\necho '  pond  '\n",
        );
        assert_eq!(
            list_avds_with(&emulator).unwrap(),
            vec!["blockloom".to_string(), "pond".to_string()]
        );
        // A missing binary names the install flow, not a crash.
        let missing = bin.join("no-emulator");
        assert!(list_avds_with(&missing).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn creating_an_avd_refuses_taken_and_bad_names() {
        let root = temp_root("avd-create");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = root.join("calls.log");
        let emulator = stub_tool(&bin, "emulator", "#!/bin/sh\necho 'blockloom'\n");
        let manager = stub_tool(
            &bin,
            "avdmanager",
            &format!(
                "#!/bin/sh\nlog=\"{}\"\nfor a in \"$@\"; do printf '%s ' \"$a\" >> \"$log\"; done\nprintf '\\n' >> \"$log\"\nread answer\nprintf 'answer=%s\\n' \"$answer\" >> \"$log\"\n",
                log.display()
            ),
        );
        // Taken names point at Start; bad names at the naming rule.
        assert!(create_avd_with(&emulator, &manager, true, Some("blockloom")).is_err());
        assert!(create_avd_with(&emulator, &manager, true, Some("pond game")).is_err());
        assert!(create_avd_with(&emulator, &manager, false, Some("pond")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn emulator_boots_take_the_avd_without_the_boot_animation() {
        assert_eq!(
            emulator_spawn_args("blockloom"),
            vec![
                "-avd".to_string(),
                "blockloom".to_string(),
                "-no-boot-anim".to_string()
            ]
        );
    }

    #[test]
    fn embedded_boots_hide_the_host_window() {
        assert_eq!(
            emulator_spawn_args_with("blockloom", true),
            vec![
                "-avd".to_string(),
                "blockloom".to_string(),
                "-no-window".to_string(),
                "-no-boot-anim".to_string()
            ]
        );
        assert_eq!(
            emulator_spawn_args_with("blockloom", false),
            emulator_spawn_args("blockloom")
        );
    }

    #[test]
    fn wm_size_reads_physical_and_override_lines() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\n"),
            Some((1080, 2400))
        );
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600\n"),
            Some((1080, 2400))
        );
        assert_eq!(parse_wm_size("nope\n"), None);
    }

    #[test]
    fn mirror_points_clamp_to_the_screen() {
        assert_eq!(mirror_point(0.5, 0.5, 1080, 2400), (540, 1200));
        assert_eq!(mirror_point(-1.0, 2.0, 1080, 2400), (0, 2400));
        assert_eq!(mirror_point(1.0, 1.0, 1080, 2400), (1080, 2400));
    }

    #[test]
    fn mirror_keys_refuse_unknown_names() {
        assert!(mirror_key(Some("emulator-5554"), "eject").is_err());
    }

    #[test]
    fn cargo_failures_keep_the_error_not_the_progress() {
        let stderr = "   Compiling cfg-if v1.0.5\n   Compiling log v0.4.34\nerror: linker `x86_64-linux-android29-clang` not found\n  |\n  = note: No such file\nerror: could not compile `blockloom-runtime` due to 1 previous error\n";
        let trimmed = trim_cargo_output(stderr);
        assert!(!trimmed.contains("Compiling"), "{trimmed}");
        assert!(trimmed.contains("linker"), "{trimmed}");
        assert!(trimmed.contains("could not compile"), "{trimmed}");
        // A wall of progress with no error still answers something.
        let progress = (0..100)
            .map(|n| format!("   Compiling crate{n} v0.1.0"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!trim_cargo_output(&progress).contains("Compiling"));
        // Long errors cap to the tail where the summary sits.
        let long = (0..100)
            .map(|n| format!("error: problem {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = trim_cargo_output(&long);
        assert!(tail.lines().count() <= 40, "{}", tail.lines().count());
        assert!(tail.contains("problem 99"), "{tail}");
    }

    #[test]
    #[cfg(unix)]
    fn boot_state_reads_sys_boot_completed() {
        let root = temp_root("boot");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let booted = stub_tool(&bin, "adb-on", "#!/bin/sh\necho '1'\n");
        assert!(booted_with_adb(&booted, "emulator-5554"));
        let booting = stub_tool(&bin, "adb-off", "#!/bin/sh\necho '0'\n");
        assert!(!booted_with_adb(&booting, "emulator-5554"));
        // `emu avd name` answers the name above the trailing OK.
        let named = stub_tool(&bin, "adb-name", "#!/bin/sh\necho 'blockloom'\necho 'OK'\n");
        assert_eq!(
            avd_name_with_adb(&named, "emulator-5554").as_deref(),
            Some("blockloom")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn stopping_a_physical_serial_is_refused_before_adb() {
        // The guard runs before any adb lookup, so no adb binary is needed.
        let error = stop_emulator(Some("ABCDEF1234")).unwrap_err();
        assert!(error.contains("not an emulator"), "{error}");
    }

    #[test]
    fn emulator_and_image_probe_off_directory_shape() {
        let root = temp_root("emu-present");
        assert!(!package_present(&root, "emulator"));
        assert!(!package_present(&root, EMULATOR_IMAGE));
        std::fs::create_dir_all(root.join("emulator")).unwrap();
        std::fs::write(root.join("emulator").join(exe("emulator")), b"fake").unwrap();
        let mut image = root.join("system-images");
        for part in ["android-35", "google_apis", "x86_64"] {
            image.push(part);
        }
        std::fs::create_dir_all(&image).unwrap();
        assert!(package_present(&root, "emulator"));
        assert!(package_present(&root, EMULATOR_IMAGE));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn emulator_status_names_what_is_missing() {
        let root = temp_root("emu-status");
        let config = AppConfig {
            sdk_path: Some(root.clone()),
            ndk_path: None,
            licenses_accepted: false,
        };
        // Neither binary nor image: the install flow is the fix.
        let status = emulator_status_for(&config);
        assert!(!status.available);
        assert!(status.avds.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn a_tail_dump_then_clears_for_the_next_poll() {
        let root = temp_root("logcat-tail");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = root.join("calls.log");
        let adb = stub_tool(
            &bin,
            "adb",
            &format!(
                "#!/bin/sh\nlog=\"{}\"\nfor a in \"$@\"; do printf '%s ' \"$a\" >> \"$log\"; done\nprintf '\\n' >> \"$log\"\necho '01-01 00:00:01.000  123  123 I blockloom: run started'\n",
                log.display()
            ),
        );
        let dumped = logcat_tail_with_adb(&adb, None, "blockloom").unwrap();
        assert!(dumped.lines[0].contains("run started"), "{dumped:?}");
        let calls = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert_eq!(lines.len(), 2, "{calls}");
        assert!(lines[0].contains("logcat -d"), "{calls}");
        assert!(lines[1].contains("logcat -c"), "{calls}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_runtime_override_wins_over_a_build() {
        let root = temp_root("override");
        let fake = root.join("libblockloom_runtime.so");
        std::fs::write(&fake, b"fake-so").unwrap();
        // SAFETY: no other test reads this var, so no thread observes it.
        unsafe {
            std::env::set_var("BLOCKLOOM_ANDROID_RUNTIME_SO", &fake);
        }
        assert_eq!(runtime_so_override(ARM64_TRIPLE), Some(fake.clone()));
        unsafe {
            std::env::remove_var("BLOCKLOOM_ANDROID_RUNTIME_SO");
        }
        assert_eq!(runtime_so_override(ARM64_TRIPLE), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_ndk_env_names_linker_and_c_toolchain() {
        let root = temp_root("ndk-env");
        let ndk = root.join("ndk").join("27.0.0");
        let bin = ndk
            .join("toolchains/llvm/prebuilt")
            .join(host_tag())
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in [
            format!("{ARM64_TRIPLE}{MIN_SDK}-clang"),
            format!("{ARM64_TRIPLE}{MIN_SDK}-clang++"),
            "llvm-ar".to_string(),
        ] {
            std::fs::write(bin.join(exe(&name)), b"fake").unwrap();
        }
        std::fs::write(ndk.join("source.properties"), b"Pkg.Revision = 27.0.0\n").unwrap();
        let config = AppConfig {
            sdk_path: None,
            ndk_path: Some(ndk),
            licenses_accepted: false,
        };
        let env = ndk_cargo_env(&config, ARM64_TRIPLE).unwrap();
        let linker = format!(
            "CARGO_TARGET_{}_LINKER",
            ARM64_TRIPLE.to_uppercase().replace('-', "_")
        );
        assert!(env.iter().any(|(key, _)| key == &linker), "{env:?}");
        assert!(
            env.iter().any(|(key, _)| key == "CC_aarch64_linux_android"),
            "{env:?}"
        );
        assert!(
            env.iter().any(|(key, _)| key == "AR_aarch64_linux_android"),
            "{env:?}"
        );
        assert!(ndk_linker_for_config(&config, ARM64_TRIPLE).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_workspace_root_holds_the_runtime_crate() {
        let root = workspace_root().expect("cargo test runs inside the workspace");
        assert!(root.join("blockloom-runtime/Cargo.toml").is_file());
    }

    #[test]
    fn keytool_makes_a_debug_keystore() {
        if keytool().is_err() {
            eprintln!("SKIP: no keytool on PATH, so no debug keystore test");
            return;
        }
        let root = temp_root("keystore");
        let store = root.join("debug.keystore");
        ensure_debug_keystore_at(&store).unwrap();
        assert!(store.is_file());
        // Second run reuses it rather than remaking it.
        ensure_debug_keystore_at(&store).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn release_rows_refuse_what_they_cannot_sign_with() {
        // Passwords come from the caller, never the project: clear the env
        // so this test reads its own args, not the machine's.
        // SAFETY: no other test reads these vars, so no thread observes it.
        unsafe {
            std::env::remove_var(STORE_PASS_ENV);
            std::env::remove_var(KEY_PASS_ENV);
        }
        if keytool().is_err() {
            eprintln!("SKIP: no keytool on PATH, so no signing test");
            return;
        }
        // Empty rows fall back to debug without needing passwords.
        let debug = resolve_signing(&AndroidSettings::default(), None, None).unwrap();
        assert!(!debug.release);
        assert_eq!(debug.label(), "debug");
        // A keystore that isn't there names the fix.
        let missing = AndroidSettings {
            keystore: "/no-such-dir/release.keystore".to_string(),
            key_alias: "upload".to_string(),
            ..AndroidSettings::default()
        };
        assert!(resolve_signing(&missing, Some("secret1"), None).is_err());
        // A keystore with no alias names the fix.
        let root = temp_root("signing-rows");
        let store = root.join("release.keystore");
        std::fs::write(&store, b"fake").unwrap();
        let no_alias = AndroidSettings {
            keystore: store.to_string_lossy().into_owned(),
            key_alias: String::new(),
            ..AndroidSettings::default()
        };
        let error = resolve_signing(&no_alias, Some("secret1"), None).unwrap_err();
        assert!(error.contains("alias"), "{error}");
        // A release row with no password says where one comes from.
        let no_pass = AndroidSettings {
            keystore: store.to_string_lossy().into_owned(),
            key_alias: "upload".to_string(),
            ..AndroidSettings::default()
        };
        let error = resolve_signing(&no_pass, None, None).unwrap_err();
        assert!(error.contains(STORE_PASS_ENV), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keytool_makes_a_release_key_and_lists_it() {
        if keytool().is_err() {
            eprintln!("SKIP: no keytool on PATH, so no release key test");
            return;
        }
        // SAFETY: no other test reads these vars, so no thread observes it.
        unsafe {
            std::env::remove_var(STORE_PASS_ENV);
            std::env::remove_var(KEY_PASS_ENV);
        }
        let root = temp_root("release-key");
        let store = root.join("release.keystore");
        let aliases = create_keystore(&store, "upload", Some("secret1"), None).unwrap();
        assert_eq!(aliases, vec!["upload".to_string()]);
        // The same alias twice is refused rather than overwritten.
        let again = create_keystore(&store, "upload", Some("secret1"), None).unwrap_err();
        assert!(again.contains("already holds"), "{again}");
        // A second alias lands beside the first.
        let aliases =
            create_keystore(&store, "upload2", Some("secret1"), Some("key2key2")).unwrap();
        assert_eq!(aliases.len(), 2, "{aliases:?}");
        // And the release row now resolves, with the key defaulting to the
        // store password when it has none of its own.
        let settings = AndroidSettings {
            keystore: store.to_string_lossy().into_owned(),
            key_alias: "upload".to_string(),
            ..AndroidSettings::default()
        };
        let signing = resolve_signing(&settings, Some("secret1"), None).unwrap();
        assert!(signing.release);
        assert_eq!(signing.label(), "release");
        assert_eq!(signing.key_pass, "secret1");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn the_keyring_remembers_forgets_and_feeds_the_next_build() {
        // SAFETY: no other test reads these vars, so no thread observes it.
        unsafe {
            std::env::remove_var(STORE_PASS_ENV);
            std::env::remove_var(KEY_PASS_ENV);
        }
        let root = temp_root("keyring");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // A fake `secret-tool` over a text file, argv-shaped like the real
        // one (see `android_keyring`): store appends, lookup prints, clear
        // drops. Point the probe at it through the env override.
        let db = root.join("entries.txt");
        let tool = stub_tool(
            &bin,
            "secret-tool",
            &format!(
                "#!/bin/sh\ndb=\"{}\"\ncmd=\"$1\"; shift\nacc=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"account\" ]; then acc=\"$a\"; fi\n  prev=\"$a\"\ndone\ncase \"$cmd\" in\n--help) exit 0;;\nstore) read secret; touch \"$db\"; grep -v \"^$acc \" \"$db\" 2>/dev/null > \"$db.tmp\" || true; mv \"$db.tmp\" \"$db\"; echo \"$acc $secret\" >> \"$db\";;\nlookup) touch \"$db\"; grep \"^$acc \" \"$db\" 2>/dev/null | tail -1 | cut -d' ' -f2-;;\nclear) touch \"$db\"; grep -v \"^$acc \" \"$db\" 2>/dev/null > \"$db.tmp\" || true; mv \"$db.tmp\" \"$db\";;\n*) exit 1;;\nesac\n",
                db.display()
            ),
        );
        // SAFETY: same vars discipline as above; accounts are pid-scoped.
        unsafe {
            std::env::set_var("BLOCKLOOM_ANDROID_SECRET_TOOL", &tool);
        }
        let store = root.join("release.keystore");
        std::fs::write(&store, b"fake").unwrap();
        let settings = AndroidSettings {
            keystore: store.to_string_lossy().into_owned(),
            key_alias: "upload".to_string(),
            ..AndroidSettings::default()
        };
        assert!(keyring_available());
        let empty = keyring_status_for(&settings);
        assert!(empty.available && !empty.store_saved && !empty.key_saved);
        // Nothing typed, nothing in env or keyring: the build refuses.
        assert!(resolve_signing(&settings, None, None).is_err());
        // Remember with a key of its own: both entries land.
        remember_signing(&settings, Some("secret1"), Some("key2key2")).unwrap();
        let saved = keyring_status_for(&settings);
        assert!(saved.store_saved && saved.key_saved, "{saved:?}");
        // The next build resolves with empty fields, straight from the ring.
        let signing = resolve_signing(&settings, None, None).unwrap();
        assert_eq!(signing.store_pass, "secret1");
        assert_eq!(signing.key_pass, "key2key2");
        // Forgetting clears both, and the build refuses again.
        assert!(forget_signing(&settings).unwrap());
        assert!(!forget_signing(&settings).unwrap());
        let cleared = keyring_status_for(&settings);
        assert!(!cleared.store_saved && !cleared.key_saved);
        assert!(resolve_signing(&settings, None, None).is_err());
        unsafe {
            std::env::remove_var("BLOCKLOOM_ANDROID_SECRET_TOOL");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A fake `aapt2 link` writes an empty zip (end-of-central-directory
    /// only) to its `-o`, so the real zip-append below has valid input.
    #[cfg(unix)]
    fn stub_tool(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    #[cfg(unix)]
    fn the_assembly_runs_tools_in_order() {
        let root = temp_root("assemble");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = root.join("calls.log");
        let logged = format!(
            "log=\"{}\"\nfor a in \"$@\"; do printf '%s ' \"$a\" >> \"$log\"; done\nprintf '\\n' >> \"$log\"\n",
            log.display()
        );
        // aapt2: `compile` touches its `-o`, `link` writes an empty zip
        // there (end-of-central-directory only), so the real zip-append
        // below has valid input.
        let aapt2 = stub_tool(
            &bin,
            "aapt2",
            &format!(
                "#!/bin/sh\n{logged}out=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"-o\" ]; then out=\"$a\"; fi\n  prev=\"$a\"\ndone\nif [ \"$1\" = \"link\" ]; then\n  printf '\\120\\113\\005\\006\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000' > \"$out\"\nelse\n  touch \"$out\"\nfi\n"
            ),
        );
        // zipalign `-f 4 in out`: copies its input to its output.
        let zipalign = stub_tool(
            &bin,
            "zipalign",
            &format!("#!/bin/sh\n{logged}cp \"$3\" \"$4\"\n"),
        );
        // apksigner: copies its last arg to its `--out`.
        let apksigner = stub_tool(
            &bin,
            "apksigner",
            &format!(
                "#!/bin/sh\n{logged}out=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"--out\" ]; then out=\"$a\"; fi\n  prev=\"$a\"\ndone\nlast=\"\"\nfor a in \"$@\"; do last=\"$a\"; done\ncp \"$last\" \"$out\"\n"
            ),
        );
        let tools = ApkTools {
            aapt2,
            zipalign,
            apksigner,
            android_jar: root.join("android.jar"),
        };
        std::fs::write(&tools.android_jar, b"fake").unwrap();

        let assets = root.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("game.pack"), b"{}").unwrap();
        let runtime = root.join("libblockloom_runtime.so");
        std::fs::write(&runtime, b"fake-so").unwrap();
        let contents = ApkContents {
            manifest: render_manifest(&AndroidSettings::default(), "Pond").unwrap(),
            icons: launcher_icons(&root, "").unwrap(),
            assets_dir: assets,
            native_libs: vec![("arm64-v8a".to_string(), runtime)],
        };
        let keystore = root.join("debug.keystore");
        std::fs::write(&keystore, b"fake").unwrap();
        let signing = Signing {
            keystore,
            alias: Some("blockloom-debug".to_string()),
            store_pass: DEBUG_KEYSTORE_PASSWORD.to_string(),
            key_pass: DEBUG_KEYSTORE_PASSWORD.to_string(),
            release: false,
        };
        let dest = root.join("pond.apk");
        let report = assemble_apk(&contents, &root.join("work"), &tools, &signing, &dest).unwrap();
        assert!(dest.is_file());
        assert_eq!(report.abis, vec!["arm64-v8a".to_string()]);
        assert_eq!(report.signed, "debug");
        let calls = std::fs::read_to_string(&log).unwrap();
        let compile = calls.find("compile").unwrap();
        let link = calls.find("link").unwrap();
        let align = calls.find(" 4 ").unwrap();
        let sign = calls.find("sign").unwrap();
        assert!(compile < link && link < align && align < sign, "{calls}");
        assert!(calls.contains("pass:android"), "{calls}");
        // The stubbed link wrote an empty zip; the real injection added the .so.
        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(
            archive
                .by_name("lib/arm64-v8a/libblockloom_runtime.so")
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&root);
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
        // An empty build-tools dir still fails the row: only the assembler
        // counts, since a stopped download leaves the dir behind.
        let row = sdk_status(&config);
        assert!(!row.ok, "{row:?}");
        assert!(row.detail.contains("aapt2"), "{row:?}");
        for tool in ["aapt2", "zipalign", "apksigner"] {
            std::fs::write(
                sdk.join("build-tools").join(BUILD_TOOLS).join(exe(tool)),
                b"fake",
            )
            .unwrap();
        }
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
