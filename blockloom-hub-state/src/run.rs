//! Running builds and copying trees, both stoppable through [`checkpoint`].

use crate::{Error, Result, checkpoint, process};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System};

/// Runs `command` to completion, checking for cancellation while it runs.
/// With `log`, its output goes to that file as well as to stderr. A
/// cancellation or failure stops it and everything it started.
pub fn run(command: &mut Command, log: Option<&Path>) -> Result<()> {
    checkpoint()?;
    let sink = match log {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
            Some(Arc::new(Mutex::new(std::fs::File::create(path)?)))
        }
        None => None,
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let shown = format!("{command:?}");
    let mut child = command
        .spawn()
        .map_err(|e| Error::new(format!("Couldn't run {shown}: {e}")))?;
    let mut pumps = vec![];
    if let Some(sink) = &sink {
        let streams: [Option<Box<dyn Read + Send>>; 2] = [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ];
        for mut stream in streams.into_iter().flatten() {
            let sink = sink.clone();
            pumps.push(std::thread::spawn(move || {
                let mut buffer = [0u8; 65536];
                while let Ok(n) = stream.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut file) = sink.lock() {
                        let _ = file.write_all(&buffer[..n]);
                        let _ = file.flush();
                    }
                    let mut err = std::io::stderr().lock();
                    let _ = err.write_all(&buffer[..n]);
                    let _ = err.flush();
                }
            }));
        }
    }
    let outcome = loop {
        if let Err(e) = checkpoint() {
            break Err(e);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break checkpoint().and_then(|()| {
                    if status.success() {
                        Ok(())
                    } else {
                        Err(Error::new(format!("{shown} failed ({status})")))
                    }
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => break Err(e.into()),
        }
    };
    if outcome.is_err() {
        stop_tree(child.id());
        let _ = child.wait();
    }
    for pump in pumps {
        let _ = pump.join();
    }
    outcome
}

/// Stops `root` and every process under it, deepest first. A build can start
/// its own sessions, so the tree is walked by parent rather than by group.
pub fn stop_tree(root: u32) {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut tree = vec![Pid::from_u32(root)];
    let mut index = 0;
    while index < tree.len() {
        let parent = tree[index];
        for (pid, proc) in system.processes() {
            if proc.parent() == Some(parent) && !tree.contains(pid) {
                tree.push(*pid);
            }
        }
        index += 1;
    }
    let tokens: Vec<_> = tree.iter().map(|p| process::token(p.as_u32())).collect();
    for signal in [Signal::Term, Signal::Kill] {
        for (pid, token) in tree.iter().zip(&tokens).rev() {
            if token.is_some()
                && process::token(pid.as_u32()) == *token
                && let Some(proc) = system.process(*pid)
                && proc.kill_with(signal).is_none()
            {
                proc.kill();
            }
        }
        if signal == Signal::Term {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Copies one file in bounded chunks, checking for cancellation between
/// them, and keeps its permissions.
pub fn copy_file(source: &Path, destination: &Path) -> Result<u64> {
    let mut reader = std::fs::File::open(source)?;
    let mut writer = std::fs::File::create(destination)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0;
    loop {
        checkpoint()?;
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buffer[..n])?;
        total += n as u64;
    }
    drop(writer);
    std::fs::set_permissions(destination, std::fs::metadata(source)?.permissions())?;
    if let Ok(modified) = std::fs::metadata(source).and_then(|m| m.modified()) {
        let _ = std::fs::File::options()
            .write(true)
            .open(destination)
            .and_then(|f| f.set_modified(modified));
    }
    Ok(total)
}

/// Copies a directory tree. `skip(relative)` leaves out a path (relative to
/// `source`, forward slashes); links are refused, since bundles hold none.
pub fn copy_tree(source: &Path, destination: &Path, skip: &dyn Fn(&str) -> bool) -> Result<()> {
    copy_tree_at(source, destination, "", skip, &mut |_| {})
}

pub(crate) fn copy_tree_at(
    source: &Path,
    destination: &Path,
    relative: &str,
    skip: &dyn Fn(&str) -> bool,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    checkpoint()?;
    std::fs::create_dir_all(destination)?;
    let mut entries: Vec<_> = std::fs::read_dir(source)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        if skip(&rel) {
            continue;
        }
        let kind = entry.file_type()?;
        let to = destination.join(&name);
        if kind.is_symlink() {
            return Err(Error::new(format!(
                "Bundle links are not supported: {}",
                entry.path().display()
            )));
        } else if kind.is_dir() {
            copy_tree_at(&entry.path(), &to, &rel, skip, progress)?;
        } else {
            progress(copy_file(&entry.path(), &to)?);
        }
    }
    Ok(())
}

/// Copies a rustup toolchain's `bin`, `lib`, `libexec` and `etc` into
/// `destination`, without sources or the `excluded` targets' libraries.
pub fn bundle_rust(
    source: &Path,
    destination: &Path,
    channel: &str,
    excluded: &[&str],
) -> Result<()> {
    let skip = |rel: &str| {
        rel.strip_prefix("lib/rustlib/")
            .is_some_and(|rest| !rest.contains('/') && (rest == "src" || excluded.contains(&rest)))
    };
    let roots: Vec<&str> = ["bin", "lib", "libexec", "etc"]
        .into_iter()
        .filter(|n| source.join(n).is_dir())
        .collect();
    let mut total = 0u64;
    for root in &roots {
        total += tree_size(&source.join(root), root, &skip);
    }
    std::fs::create_dir_all(destination)?;
    let mut copied = 0u64;
    let mut last = Instant::now() - Duration::from_secs(2);
    let mut report = |n: u64| {
        copied += n;
        if last.elapsed() >= Duration::from_secs(1) {
            eprintln!(
                "Bundling Rust {channel}: {:.0}/{:.0} MiB",
                copied as f64 / 1048576.0,
                total as f64 / 1048576.0
            );
            last = Instant::now();
        }
    };
    for root in roots {
        copy_tree_at(
            &source.join(root),
            &destination.join(root),
            root,
            &skip,
            &mut report,
        )?;
    }
    Ok(())
}

fn tree_size(path: &Path, relative: &str, skip: &dyn Fn(&str) -> bool) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let rel = format!("{relative}/{}", e.file_name().to_string_lossy());
            if skip(&rel) {
                0
            } else if e.file_type().is_ok_and(|t| t.is_dir()) {
                tree_size(&e.path(), &rel, skip)
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// Total size of the regular files under `path`.
pub fn size_of(path: &Path) -> u64 {
    tree_size(path, "", &|_| false)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_build_logs_its_output_and_a_failure_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("logs/build.log");
        run(
            Command::new("sh").args(["-c", "echo out; echo err >&2"]),
            Some(&log),
        )
        .unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("out") && text.contains("err"), "{text}");
        assert!(run(Command::new("sh").args(["-c", "exit 3"]), None).is_err());
    }

    #[test]
    fn stopping_a_tree_ends_its_children() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 60 & echo $!; wait"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().unwrap()),
            &mut line,
        )
        .unwrap();
        let grandchild: u32 = line.trim().parse().unwrap();
        stop_tree(child.id());
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!process::alive(grandchild));
    }
}
