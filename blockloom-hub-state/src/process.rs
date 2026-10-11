//! Whether a process is the one we started, across platforms.

use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

/// A process's birth identifier, or `None` when it has exited. A pid number
/// alone is reused, so a record also keeps this to tell its process apart.
pub fn token(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let target = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let process = system.process(target)?;
    if matches!(process.status(), ProcessStatus::Zombie | ProcessStatus::Dead) {
        return None;
    }
    Some(process.start_time().to_string())
}

pub fn alive(pid: u32) -> bool {
    token(pid).is_some()
}

/// Whether `pid` is still the process a record named.
pub fn is_same(pid: u32, recorded: Option<&str>) -> bool {
    match token(pid) {
        None => false,
        Some(current) => match recorded {
            Some(old) => old == current || old == "unknown",
            None => true,
        },
    }
}
