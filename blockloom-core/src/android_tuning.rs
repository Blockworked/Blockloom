//! What makes an emulator boot fast: the GPU it draws with, how many cores
//! and how much memory the guest gets, whether the host has a hypervisor at
//! all, and a record of each spawned emulator so a start that died is
//! reported instead of waited on.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;

/// The GPU modes the emulator's `-gpu` flag takes that Blockloom allows.
const GPU_MODES: [&str; 5] = [
    "host",
    "auto",
    "angle_indirect",
    "swangle_indirect",
    "swiftshader_indirect",
];

/// Guest cores the emulator is raised to at most, and the memory it is
/// raised to. Raise-only: an AVD the user sized bigger is left alone.
const MAX_CORES: u32 = 6;
const WANT_MEMORY_MB: u32 = 4096;
/// Hosts with less RAM than this give the guest a smaller bump.
const ROOMY_HOST_MB: u64 = 12 * 1024;
const TIGHT_MEMORY_MB: u32 = 3072;

/// The flags an emulator boot adds on top of the AVD and window choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuning {
    /// The `-gpu` mode. Empty leaves the flag off.
    pub gpu: String,
    /// `-cores`, when the AVD asks for fewer than the host can spare.
    pub cores: Option<u32>,
    /// `-memory` in MiB, likewise.
    pub memory_mb: Option<u32>,
}

impl Tuning {
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if !self.gpu.is_empty() {
            args.extend(["-gpu".to_string(), self.gpu.clone()]);
        }
        if let Some(cores) = self.cores {
            args.extend(["-cores".to_string(), cores.to_string()]);
        }
        if let Some(memory) = self.memory_mb {
            args.extend(["-memory".to_string(), memory.to_string()]);
        }
        // Never stop at the metrics consent prompt of a headless boot.
        args.push("-no-metrics".to_string());
        args
    }
}

/// The GPU mode to boot with: `BLOCKLOOM_EMULATOR_GPU` when it names a
/// known mode, `host` otherwise. `auto` falls back to software drawing as
/// soon as the window is hidden, which is what an embedded boot does, so
/// the host GPU is asked for outright.
pub fn gpu_mode(wanted: Option<&str>) -> String {
    wanted
        .map(|mode| mode.trim().to_lowercase())
        .filter(|mode| GPU_MODES.contains(&mode.as_str()))
        .unwrap_or_else(|| "host".to_string())
}

/// The tuning for an AVD whose config says `avd_cores` and `avd_memory_mb`
/// (None when unreadable), on a host with `host_cores` and `host_memory_mb`.
pub fn tune(
    gpu: String,
    avd_cores: Option<u32>,
    avd_memory_mb: Option<u32>,
    host_cores: u32,
    host_memory_mb: Option<u64>,
) -> Tuning {
    let want_cores = (host_cores / 2).clamp(2, MAX_CORES);
    let want_memory = match host_memory_mb {
        Some(total) if total < ROOMY_HOST_MB => TIGHT_MEMORY_MB,
        _ => WANT_MEMORY_MB,
    };
    Tuning {
        gpu,
        cores: Some(want_cores).filter(|want| avd_cores.is_none_or(|have| have < *want)),
        memory_mb: Some(want_memory).filter(|want| avd_memory_mb.is_none_or(|have| have < *want)),
    }
}

/// Reads `key=value` out of an AVD `config.ini`.
pub fn ini_number(config: &str, key: &str) -> Option<u32> {
    config
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .and_then(|(_, value)| value.trim().parse().ok())
}

/// Where AVDs live: `ANDROID_AVD_HOME`, else `$ANDROID_SDK_HOME/.android/avd`
/// or `~/.android/avd`.
pub fn avd_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ANDROID_AVD_HOME").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::var_os("ANDROID_SDK_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".android")
        .join("avd")
}

/// The tuning this machine gives `avd`.
pub fn tuning_for(avd: &str) -> Tuning {
    let config = std::fs::read_to_string(avd_dir().join(format!("{avd}.avd")).join("config.ini"))
        .unwrap_or_default();
    tune(
        gpu_mode(std::env::var("BLOCKLOOM_EMULATOR_GPU").ok().as_deref()),
        ini_number(&config, "hw.cpu.ncore"),
        ini_number(&config, "hw.ramSize"),
        std::thread::available_parallelism().map_or(4, |n| n.get() as u32),
        host_memory_mb(),
    )
}

#[cfg(target_os = "linux")]
fn host_memory_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb / 1024)
}

#[cfg(not(target_os = "linux"))]
fn host_memory_mb() -> Option<u64> {
    None
}

/// Whether the host can run the x86_64 guest with a hypervisor: without
/// one the emulator either refuses to start or translates every
/// instruction in software, which is the slowest an emulator gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceleration {
    pub ok: bool,
    pub detail: String,
}

/// Reads what `emulator -accel-check` said: exit 0 is usable, and its
/// lines (minus the bare `accel` markers and the status number) are the
/// reason either way.
pub fn accel_from_check(success: bool, output: &str) -> Acceleration {
    let detail: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "accel" && line.parse::<i32>().is_err())
        .collect();
    let detail = detail.join(" ");
    Acceleration {
        ok: success,
        detail: if detail.is_empty() {
            if success {
                "Hardware acceleration is available.".to_string()
            } else {
                "Hardware acceleration is not available.".to_string()
            }
        } else {
            detail
        },
    }
}

/// Runs the emulator's own accelerator probe. A host that can't answer
/// (no binary yet) reports unknown as not ok, with the reason.
pub fn acceleration(emulator: &Path) -> Acceleration {
    if std::env::consts::ARCH == "aarch64" {
        return Acceleration {
            ok: false,
            detail: "This machine is arm64, so the x86_64 emulator image runs unaccelerated \
                     (or not at all)."
                .to_string(),
        };
    }
    match crate::process::background_command(emulator)
        .arg("-accel-check")
        .output()
    {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            let mut accel = accel_from_check(output.status.success(), &text);
            if !accel.ok && cfg!(target_os = "linux") {
                accel.detail.push_str(&linux_kvm_hint());
            }
            accel
        }
        Err(error) => Acceleration {
            ok: false,
            detail: format!("Couldn't check hardware acceleration: {error}"),
        },
    }
}

#[cfg(target_os = "linux")]
fn linux_kvm_hint() -> String {
    use std::fs::OpenOptions;
    if !Path::new("/dev/kvm").exists() {
        return " /dev/kvm is missing: turn on virtualization (VT-x/AMD-V) in the BIOS."
            .to_string();
    }
    if OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_err()
    {
        return " /dev/kvm isn't accessible: add your user to the kvm group and log in again."
            .to_string();
    }
    String::new()
}

#[cfg(not(target_os = "linux"))]
fn linux_kvm_hint() -> String {
    String::new()
}

// ─── Spawned emulators ───────────────────────────────────────────────────
// A boot that dies (no hypervisor, a bad GPU mode) leaves nothing for adb to
// see, so the panel would wait out its whole deadline. The child and its log
// are kept so the next status read can say why.

struct Spawned {
    avd: String,
    child: Child,
    log: PathBuf,
}

static SPAWNED: Mutex<Vec<Spawned>> = Mutex::new(Vec::new());

/// Where an AVD's emulator output goes.
pub fn log_path(data_dir: &Path, avd: &str) -> PathBuf {
    data_dir.join(format!("emulator-{avd}.log"))
}

/// Where the gRPC port an AVD's emulator was started with is kept.
pub fn grpc_path(data_dir: &Path, avd: &str) -> PathBuf {
    data_dir.join(format!("emulator-{avd}.grpc"))
}

/// A loopback port nothing is listening on right now, for the emulator's
/// gRPC service. The emulator binds it a moment later, so another process
/// could in principle take it first; the screen then falls back to adb.
pub fn free_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .ok()
        .map(|addr| addr.port())
}

/// Remembers a freshly spawned emulator.
pub fn track(avd: &str, child: Child, log: PathBuf) {
    if let Ok(mut spawned) = SPAWNED.lock() {
        spawned.retain(|s| s.avd != avd);
        spawned.push(Spawned {
            avd: avd.to_string(),
            child,
            log,
        });
    }
}

/// Reaps every tracked emulator that exited and answers why, as AVD name
/// and the tail of its log. An emulator that exited cleanly (the user closed
/// its window) isn't a failure and answers nothing.
pub fn take_failures() -> Vec<(String, String)> {
    let mut failures = Vec::new();
    let Ok(mut spawned) = SPAWNED.lock() else {
        return failures;
    };
    spawned.retain_mut(|s| match s.child.try_wait() {
        Ok(Some(status)) => {
            if !status.success() {
                failures.push((s.avd.clone(), failure_text(&s.log, status.code())));
            }
            false
        }
        _ => true,
    });
    failures
}

fn failure_text(log: &Path, code: Option<i32>) -> String {
    let tail = log_tail(&std::fs::read_to_string(log).unwrap_or_default());
    let code = code.map_or_else(
        || "a signal".to_string(),
        |code| format!("exit code {code}"),
    );
    if tail.is_empty() {
        format!("The emulator stopped ({code}).")
    } else {
        format!("The emulator stopped ({code}): {tail}")
    }
}

/// The last few meaningful lines of an emulator log: the ERROR/PANIC/fatal
/// ones when there are any, else just the tail.
pub fn log_tail(log: &str) -> String {
    let lines: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let loud: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains("error") || lower.contains("panic") || lower.contains("fatal")
        })
        .collect();
    let pick = if loud.is_empty() { &lines } else { &loud };
    let from = pick.len().saturating_sub(4);
    pick[from..].join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gpu_defaults_to_host_and_refuses_unknown_modes() {
        assert_eq!(gpu_mode(None), "host");
        assert_eq!(gpu_mode(Some(" Auto ")), "auto");
        assert_eq!(
            gpu_mode(Some("swiftshader_indirect")),
            "swiftshader_indirect"
        );
        assert_eq!(gpu_mode(Some("rm -rf")), "host");
    }

    #[test]
    fn cores_and_memory_are_only_ever_raised() {
        let small = tune("host".into(), Some(2), Some(2048), 16, Some(32 * 1024));
        assert_eq!(small.cores, Some(6));
        assert_eq!(small.memory_mb, Some(4096));
        // An AVD already past the target is left as the user sized it.
        let big = tune("host".into(), Some(8), Some(8192), 16, Some(32 * 1024));
        assert_eq!((big.cores, big.memory_mb), (None, None));
        // A small host gets fewer cores and a smaller memory bump.
        let tight = tune("host".into(), Some(1), Some(2048), 4, Some(8 * 1024));
        assert_eq!((tight.cores, tight.memory_mb), (Some(2), Some(3072)));
        // An unreadable config is raised rather than trusted.
        assert_eq!(tune("host".into(), None, None, 8, None).cores, Some(4));
    }

    #[test]
    fn tuning_spells_the_flags() {
        let tuning = Tuning {
            gpu: "host".into(),
            cores: Some(4),
            memory_mb: None,
        };
        assert_eq!(
            tuning.args(),
            ["-gpu", "host", "-cores", "4", "-no-metrics"].map(String::from)
        );
    }

    #[test]
    fn avd_config_numbers_read_by_exact_key() {
        let config = "hw.ramSize=2048\nhw.ramSize.extra=1\nhw.cpu.ncore = 4\n";
        assert_eq!(ini_number(config, "hw.ramSize"), Some(2048));
        assert_eq!(ini_number(config, "hw.cpu.ncore"), Some(4));
        assert_eq!(ini_number(config, "hw.lcd.width"), None);
    }

    #[test]
    fn the_accel_check_output_becomes_a_sentence() {
        let ok = accel_from_check(
            true,
            "accel:\n0\nKVM (version 12) is installed and usable.\naccel\n",
        );
        assert!(ok.ok);
        assert_eq!(
            ok.detail,
            "accel: KVM (version 12) is installed and usable."
        );
        let bad = accel_from_check(false, "accel:\n1\n/dev/kvm is not found\naccel\n");
        assert!(!bad.ok);
        assert!(bad.detail.contains("/dev/kvm is not found"));
        assert!(accel_from_check(false, "").detail.contains("not available"));
    }

    #[test]
    fn a_log_tail_prefers_the_errors() {
        let log = "boot\nERROR: x86_64 emulation requires hardware acceleration\nmore\nlater\n";
        assert_eq!(
            log_tail(log),
            "ERROR: x86_64 emulation requires hardware acceleration"
        );
        assert_eq!(log_tail("a\nb\n"), "a | b");
        assert_eq!(log_tail(""), "");
    }

    #[cfg(unix)]
    #[test]
    fn a_dead_emulator_is_reported_once_and_a_clean_exit_isnt() {
        let dir = std::env::temp_dir().join(format!("blockloom-tuning-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = log_path(&dir, "crashy");
        std::fs::write(&log, "PANIC: no KVM\n").unwrap();
        let child = std::process::Command::new("sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        track("crashy", child, log);
        let clean = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        track("closed", clean, log_path(&dir, "closed"));
        // Both children are short-lived; give them a moment to exit.
        let mut failures = Vec::new();
        for _ in 0..100 {
            failures.extend(take_failures());
            if !failures.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].0, "crashy");
        assert!(failures[0].1.contains("PANIC: no KVM"));
        assert!(failures[0].1.contains("exit code 3"));
        assert!(take_failures().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
