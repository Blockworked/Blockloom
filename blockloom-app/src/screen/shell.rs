//! A long-lived `adb shell` that input is typed into. Spawning `adb` per
//! tap costs a process on the host and another on the device; this keeps
//! one of each and learns when each command finished.

use super::{DeviceSize, Phase, TouchSink};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Printed by the device after each command, so the host can count them.
const ACK: &str = "::blockloom-ack";
/// A press that moves less than this many pixels is a tap.
const TAP_SLOP: u32 = 12;

pub(crate) struct Shell {
    child: Child,
    stdin: ChildStdin,
    /// Commands written whose acknowledgement hasn't come back.
    inflight: Arc<AtomicUsize>,
    /// Whether `input motionevent` exists, so a drag can follow the pointer.
    pub motion: bool,
}

impl Shell {
    pub fn start(adb: &Path, serial: &str) -> Result<Shell, String> {
        let motion = Command::new(adb)
            .args(["-s", serial, "shell", "input"])
            .stdin(Stdio::null())
            .output()
            .map(|out| {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                text.contains("motionevent")
            })
            .unwrap_or(false);
        let mut child = Command::new(adb)
            .args(["-s", serial, "shell"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Couldn't run {}: {e}", adb.display()))?;
        let stdin = child.stdin.take().ok_or("Couldn't open adb's input")?;
        let stdout = child.stdout.take().ok_or("Couldn't open adb's output")?;
        let inflight = Arc::new(AtomicUsize::new(0));
        let counted = inflight.clone();
        std::thread::Builder::new()
            .name("blockloom-screen-acks".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if line.trim() == ACK {
                        let _ = counted.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                            Some(n.saturating_sub(1))
                        });
                    }
                }
                // The shell ended: nothing in flight will ever be answered.
                counted.store(0, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        Ok(Shell {
            child,
            stdin,
            inflight,
            motion,
        })
    }

    /// Whether every command written so far has finished on the device.
    pub fn idle(&self) -> bool {
        self.inflight.load(Ordering::Acquire) == 0
    }

    fn run(&mut self, command: &str) -> Result<(), String> {
        self.inflight.fetch_add(1, Ordering::AcqRel);
        writeln!(self.stdin, "{command}; echo {ACK}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| {
                self.inflight.store(0, Ordering::Release);
                format!("Lost the device's input: {e}")
            })
    }

    pub fn key(&mut self, code: &str) -> Result<(), String> {
        let keycode = blockloom_core::android::mirror_keycode(code)?;
        self.run(&format!("input keyevent {keycode}"))
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Touch as `input` commands: a drag follows the pointer where the device
/// has `input motionevent`, and is replayed as a swipe where it doesn't.
pub(crate) struct ShellTouch {
    shell: Arc<Mutex<Shell>>,
    size: Arc<DeviceSize>,
    pressed: Option<(Instant, (u32, u32))>,
    last: (u32, u32),
}

impl ShellTouch {
    pub fn new(shell: Arc<Mutex<Shell>>, size: Arc<DeviceSize>) -> Self {
        Self {
            shell,
            size,
            pressed: None,
            last: (0, 0),
        }
    }
}

impl TouchSink for ShellTouch {
    fn touch(&mut self, phase: Phase, x: f32, y: f32) {
        let Some((px, py)) = self.size.point(x, y) else {
            return;
        };
        let Ok(mut shell) = self.shell.lock() else {
            return;
        };
        let motion = shell.motion;
        match phase {
            Phase::Down => {
                self.pressed = Some((Instant::now(), (px, py)));
                self.last = (px, py);
                if motion {
                    let _ = shell.run(&format!("input motionevent DOWN {px} {py}"));
                }
            }
            Phase::Move => {
                self.last = (px, py);
                // A move while the last command is still running would only
                // queue behind it; the next one (or the release) carries on.
                if motion && self.pressed.is_some() && shell.idle() {
                    let _ = shell.run(&format!("input motionevent MOVE {px} {py}"));
                }
            }
            Phase::Up => {
                let Some((since, (sx, sy))) = self.pressed.take() else {
                    return;
                };
                if motion {
                    let _ = shell.run(&format!("input motionevent UP {px} {py}"));
                } else if px.abs_diff(sx).max(py.abs_diff(sy)) < TAP_SLOP {
                    let _ = shell.run(&format!("input tap {px} {py}"));
                } else {
                    let ms = since.elapsed().as_millis().clamp(60, 1500);
                    let _ = shell.run(&format!("input swipe {sx} {sy} {px} {py} {ms}"));
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::super::testing::{stub, temp};
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    /// An adb whose shell logs each line it is typed and answers the ack, and
    /// whose `input` usage mentions `motionevent` only when asked to.
    fn adb(dir: &Path, motion: bool) -> (PathBuf, PathBuf) {
        let log = dir.join("typed.log");
        let usage = if motion {
            "usage: input motionevent"
        } else {
            "usage: input tap"
        };
        let body = format!(
            "#!/bin/sh\nif [ \"$4\" = input ]; then echo '{usage}'; exit 1; fi\n\
             while IFS= read -r line; do echo \"$line\" >> '{log}'; echo '{ACK}'; done\n",
            log = log.display()
        );
        (stub(dir, "adb", &body), log)
    }

    fn typed(log: &Path, count: usize) -> Vec<String> {
        for _ in 0..500 {
            let text = std::fs::read_to_string(log).unwrap_or_default();
            let lines: Vec<String> = text
                .lines()
                .map(|line| line.trim_end_matches(&format!("; echo {ACK}")).to_string())
                .collect();
            if lines.len() >= count {
                return lines;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("expected {count} commands");
    }

    fn touch(dir: &Path, motion: bool) -> (ShellTouch, PathBuf, Arc<Mutex<Shell>>) {
        let (adb, log) = adb(dir, motion);
        let shell = Arc::new(Mutex::new(Shell::start(&adb, "phone-1").unwrap()));
        let size = Arc::new(DeviceSize::default());
        size.set(1000, 2000);
        (ShellTouch::new(shell.clone(), size), log, shell)
    }

    fn settle(shell: &Arc<Mutex<Shell>>) {
        for _ in 0..500 {
            if shell.lock().unwrap().idle() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the shell never answered");
    }

    #[test]
    fn a_drag_follows_the_pointer_where_the_device_can() {
        let dir = temp("motion");
        let (mut pointer, log, shell) = touch(&dir, true);
        assert!(shell.lock().unwrap().motion);
        pointer.touch(Phase::Down, 0.0, 0.0);
        settle(&shell);
        pointer.touch(Phase::Move, 0.5, 0.5);
        settle(&shell);
        pointer.touch(Phase::Up, 1.0, 1.0);
        let lines = typed(&log, 3);
        assert_eq!(lines[0], "input motionevent DOWN 0 0");
        assert_eq!(lines[1], "input motionevent MOVE 500 1000");
        assert_eq!(lines[2], "input motionevent UP 999 1999");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_motion_events_a_press_replays_as_a_tap_or_a_swipe() {
        let dir = temp("legacy");
        let (mut pointer, log, shell) = touch(&dir, false);
        assert!(!shell.lock().unwrap().motion);
        pointer.touch(Phase::Down, 0.5, 0.5);
        pointer.touch(Phase::Move, 0.5, 0.5);
        pointer.touch(Phase::Up, 0.5, 0.5);
        pointer.touch(Phase::Down, 0.0, 0.0);
        pointer.touch(Phase::Up, 1.0, 1.0);
        let lines = typed(&log, 2);
        assert_eq!(lines[0], "input tap 500 1000");
        assert!(
            lines[1].starts_with("input swipe 0 0 999 1999 "),
            "{}",
            lines[1]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_type_their_keycode_and_unknown_ones_are_refused() {
        let dir = temp("keys");
        let (_pointer, log, shell) = touch(&dir, true);
        shell.lock().unwrap().key("home").unwrap();
        assert!(shell.lock().unwrap().key("eject").is_err());
        assert_eq!(typed(&log, 1), ["input keyevent KEYCODE_HOME"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
