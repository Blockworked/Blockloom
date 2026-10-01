//! Progress and cancellation for a build and its child tools.

use std::cell::RefCell;
use std::io::{self, BufRead, BufReader};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct BuildControl {
    cancelled: Arc<AtomicBool>,
    progress: Arc<Mutex<(String, String)>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<BuildControl>> = const { RefCell::new(None) };
}

impl BuildControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub fn progress(&self) -> (String, String) {
        self.progress.lock().unwrap().clone()
    }
    pub fn run<T>(&self, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        struct Scope(Option<BuildControl>);
        impl Drop for Scope {
            fn drop(&mut self) {
                ACTIVE.with(|active| active.replace(self.0.take()));
            }
        }
        let _scope = Scope(ACTIVE.with(|active| active.replace(Some(self.clone()))));
        check()?;
        let result = work();
        check()?;
        result
    }
}

pub(crate) fn current() -> Option<BuildControl> {
    ACTIVE.with(|active| active.borrow().clone())
}

pub fn check() -> Result<(), String> {
    if ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(BuildControl::cancelled)
    }) {
        Err("Build cancelled.".to_string())
    } else {
        Ok(())
    }
}

pub fn step(text: &str) -> Result<(), String> {
    check()?;
    ACTIVE.with(|active| {
        if let Some(control) = active.borrow().as_ref() {
            *control.progress.lock().unwrap() = (text.to_string(), String::new());
        }
    });
    Ok(())
}

/// Drains both pipes while waiting, so long compiler output cannot block Cancel.
pub fn output(command: &mut Command) -> io::Result<Output> {
    let control = ACTIVE.with(|active| active.borrow().clone());
    let Some(control) = control else {
        return command.output();
    };
    check().map_err(io::Error::other)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || read_pipe(stdout, None));
    let detail = control.clone();
    let err = std::thread::spawn(move || read_pipe(stderr, Some(detail)));
    let mut status = None;
    let mut killed = false;
    loop {
        if control.cancelled() && !killed {
            // Compilers spawn linkers and workers; cancel their whole process tree.
            #[cfg(unix)]
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status();
            #[cfg(windows)]
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &child.id().to_string()])
                .status();
            let _ = child.kill();
            killed = true;
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if status.is_some() && out.is_finished() && err.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(75));
    }
    let status = status.unwrap();
    let stdout = out
        .join()
        .map_err(|_| io::Error::other("Couldn't read tool output"))??;
    let stderr = err
        .join()
        .map_err(|_| io::Error::other("Couldn't read tool output"))??;
    if control.cancelled() {
        return Err(io::Error::other("Build cancelled."));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe(pipe: impl io::Read, control: Option<BuildControl>) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(pipe);
    let mut bytes = Vec::new();
    loop {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if let Some(control) = &control {
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            if !text.is_empty() {
                control.progress.lock().unwrap().1 = text.chars().take(300).collect();
            }
        }
        bytes.extend(line);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_and_cancellation_are_scoped() {
        let control = BuildControl::default();
        control
            .run(|| {
                step("Packing assets")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(control.progress().0, "Packing assets");
        control.cancel();
        assert!(control.run(|| Ok(())).is_err());
        assert!(check().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_stops_a_tool_and_its_children() {
        let control = BuildControl::default();
        let worker = control.clone();
        let thread = std::thread::spawn(move || {
            worker.run(|| {
                output(Command::new("sh").args(["-c", "echo running >&2; sleep 30 & wait"]))
                    .map_err(|e| e.to_string())
            })
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while control.progress().1 != "running" {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        control.cancel();
        assert!(thread.join().unwrap().unwrap_err().contains("cancelled"));
        assert!(std::time::Instant::now() < deadline);
    }
}
