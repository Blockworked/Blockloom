//! Cancellable project snapshots and restoring them.

use crate::{Error, Hub, Result, checkpoint, key, oplock::OperationLock};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Left out of a snapshot, and left alone by a restore.
const EXCLUDED: [&str; 5] = [
    ".git",
    ".svn",
    ".blockloom/build",
    ".blockloom/lock.json",
    ".blockloom/backups",
];

#[derive(Debug, Clone, PartialEq)]
struct Item {
    dir: bool,
    len: u64,
    modified: u128,
    inode: u64,
    #[cfg(unix)]
    permissions: u32,
}

type Inventory = BTreeMap<String, Item>;

fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn inode(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

fn inventory(project: &Path) -> Result<Inventory> {
    fn walk(root: &Path, dir: &Path, out: &mut Inventory) -> Result<()> {
        checkpoint()?;
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("walk stays under its root");
            let relative = relative
                .to_str()
                .ok_or_else(|| Error::new("Project backup requires UTF-8 file names"))?;
            #[cfg(windows)]
            let relative = relative.replace('\\', "/");
            if excluded(&relative) {
                continue;
            }
            if !safe_name(&relative) {
                return Err(Error::new(format!(
                    "Project backup cannot include this path: {relative}"
                )));
            }
            let meta = std::fs::symlink_metadata(&path)?;
            if is_link(&meta) {
                return Err(Error::new(format!(
                    "Project backup does not follow links: {relative}"
                )));
            }
            if !meta.is_file() && !meta.is_dir() {
                return Err(Error::new(format!(
                    "Project backup cannot include special files: {relative}"
                )));
            }
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            out.insert(
                relative.to_string(),
                Item {
                    dir: meta.is_dir(),
                    len: if meta.is_dir() { 0 } else { meta.len() },
                    modified,
                    inode: inode(&meta),
                    #[cfg(unix)]
                    permissions: {
                        use std::os::unix::fs::PermissionsExt;
                        meta.permissions().mode() & 0o777
                    },
                },
            );
            if meta.is_dir() {
                walk(root, &path, out)?;
            }
        }
        Ok(())
    }
    let mut out = Inventory::new();
    walk(project, project, &mut out)?;
    Ok(out)
}

/// `20261010-134501` for a Unix time, UTC.
fn stamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rest = secs % 86400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct Progress {
    files: usize,
    size: u64,
    copied: u64,
    finished: usize,
    last: Instant,
}

impl Progress {
    fn new(files: usize, size: u64) -> Self {
        let mut p = Self {
            files,
            size,
            copied: 0,
            finished: 0,
            last: Instant::now(),
        };
        p.report();
        p
    }
    fn report(&mut self) {
        eprintln!(
            "Backing up project: {}/{} files, {}/{} MiB",
            self.finished,
            self.files,
            self.copied / (1024 * 1024),
            self.size / (1024 * 1024)
        );
        self.last = Instant::now();
    }
    fn step(&mut self, bytes: u64, finished: bool) {
        self.copied += bytes;
        self.finished += finished as usize;
        if self.last.elapsed().as_secs() >= 1 || self.finished == self.files {
            self.report();
        }
    }
}

fn create(project: &Path, destination: &Path, idle: &dyn Fn() -> Result<()>) -> Result<Value> {
    let resolved = std::fs::canonicalize(destination.parent().unwrap_or(destination))
        .unwrap_or_else(|_| destination.to_path_buf());
    if resolved
        .starts_with(std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf()))
    {
        return Err(Error::new(
            "Project backups must be stored outside the project folder",
        ));
    }
    idle()?;
    let entries = inventory(project)?;
    let files: Vec<&String> = entries
        .iter()
        .filter(|(_, i)| !i.dir)
        .map(|(n, _)| n)
        .collect();
    let size: u64 = entries.values().map(|i| i.len).sum();
    let mut progress = Progress::new(files.len(), size);
    let parent = destination
        .parent()
        .ok_or_else(|| Error::new("Backup destination must have a parent folder"))?;
    let mut pending = tempfile::NamedTempFile::new_in(parent)?;
    let mut archive = zip::ZipWriter::new(pending.as_file_mut());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(3))
        .large_file(true);
    let mut buffer = vec![0; 1024 * 1024];
    for (name, item) in &entries {
        checkpoint()?;
        let path = project.join(name);
        #[cfg(unix)]
        let options = options.unix_permissions(item.permissions);
        if item.dir {
            archive.add_directory(name.as_str(), options)?;
            continue;
        }
        archive.start_file(name.as_str(), options)?;
        let mut reader = std::fs::File::open(&path)?;
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            checkpoint()?;
            archive.write_all(&buffer[..n])?;
            progress.step(n as u64, false);
        }
        progress.step(0, true);
    }
    archive.finish()?.sync_all()?;
    checkpoint()?;
    idle()?;
    if inventory(project)? != entries {
        return Err(Error::new(
            "Project files changed during backup. Try again with the editor closed",
        ));
    }
    pending
        .persist_noclobber(destination)
        .map_err(|error| error.error)?;
    eprintln!("Project backup complete.");
    Ok(
        json!({"path": destination.to_string_lossy(), "files": files.len(), "created_at": unix_now()}),
    )
}

fn excluded(name: &str) -> bool {
    let eq = |a: &str, b: &str| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    EXCLUDED.iter().any(|entry| {
        eq(name, entry)
            || name
                .get(..entry.len())
                .is_some_and(|prefix| eq(prefix, entry))
                && name.as_bytes().get(entry.len()) == Some(&b'/')
    })
}

fn safe_name(name: &str) -> bool {
    !name.contains(['\\', '\0'])
        && name.trim_end_matches('/').split('/').all(|part| {
            !matches!(part, "" | "." | "..")
                && (!cfg!(windows) || !part.contains(':') && !part.ends_with(['.', ' ']))
        })
}

fn restore(project: &Path, archive: &Path, idle: &dyn Fn() -> Result<()>) -> Result<()> {
    let project = std::fs::canonicalize(project)?;
    let mut snapshot = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let parent = project
        .parent()
        .ok_or_else(|| Error::new("Cannot restore a filesystem root"))?;
    let work = parent.join(format!(".restore-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&work)?;
    let staging = work.join("incoming");
    let previous = work.join("previous");
    let result = (|| -> Result<()> {
        std::fs::create_dir(&staging)?;
        let mut buffer = vec![0; 1024 * 1024];
        for index in 0..snapshot.len() {
            checkpoint()?;
            let mut entry = snapshot.by_index(index)?;
            let name = entry.name().trim_end_matches('/');
            if !safe_name(name) || excluded(name) || entry.is_symlink() {
                return Err(Error::new(format!(
                    "Backup holds an unsupported path: {name}"
                )));
            }
            let target = staging.join(name);
            if entry.is_dir() {
                std::fs::create_dir_all(&target)?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            loop {
                checkpoint()?;
                let n = entry.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                out.write_all(&buffer[..n])?;
            }
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                out.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
        if blockloom_core::library::read_meta(&staging).is_none() {
            return Err(Error::new(
                "That archive is not a readable Blockloom project backup",
            ));
        }
        let mut protected = Vec::new();
        for name in EXCLUDED {
            match project.join(name).symlink_metadata() {
                Ok(_) => protected.push(name),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        for name in &protected {
            if let Some(parent) = staging.join(name).parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        checkpoint()?;
        idle()?;
        std::fs::rename(&project, &previous)?;
        // Finish the swap or roll it back before honoring cancellation again.
        let mut moved = Vec::new();
        let swap = (|| -> std::io::Result<()> {
            for name in protected {
                std::fs::rename(previous.join(name), staging.join(name))?;
                moved.push(name);
            }
            std::fs::rename(&staging, &project)
        })();
        if let Err(error) = swap {
            let mut rollback = Ok(());
            for name in moved.into_iter().rev() {
                if let Err(error) = std::fs::rename(staging.join(name), previous.join(name)) {
                    rollback = Err(error);
                }
            }
            if let Err(rollback) = rollback.and_then(|()| std::fs::rename(&previous, &project)) {
                return Err(Error::new(format!(
                    "Restore failed: {error}. Rollback failed: {rollback}. Project files are in {}",
                    work.display()
                )));
            }
            return Err(error.into());
        }
        Ok(())
    })();
    if result.is_ok() || !previous.exists() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result?;
    eprintln!("Project restored.");
    Ok(())
}

impl Hub {
    fn backup_directory(&self, project: &Path) -> PathBuf {
        let digest = Sha256::digest(key(project).as_bytes());
        let name: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        self.root.join("backups").join(name)
    }

    /// Snapshots a project. The caller holds the operation lock.
    pub(crate) fn backup_locked(&self, project: &Value) -> Result<Value> {
        let source = PathBuf::from(project["path"].as_str().unwrap_or(""));
        let directory = self.backup_directory(&source);
        let inside = directory.starts_with(&source);
        if inside {
            return Err(Error::new(
                "Project backups must be stored outside the project folder",
            ));
        }
        std::fs::create_dir_all(&directory)?;
        let name = format!(
            "{}-{}.zip",
            stamp(unix_now()),
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let idle = || self.require_idle(&source);
        let mut snapshot = create(&source, &directory.join(name), &idle)?;
        snapshot["project"] = json!(source.to_string_lossy());
        snapshot["installation"] = project["installation"].clone();
        Ok(snapshot)
    }

    pub fn backup_project(&self, project: &Path) -> Result<Value> {
        let _lock = OperationLock::acquire(&self.root)?;
        let info = self.project(project)?;
        self.backup_locked(&info)
    }

    /// Snapshots of a project, newest first.
    pub fn backups(&self, project: &Path) -> Result<Vec<Value>> {
        let directory =
            self.backup_directory(&std::fs::canonicalize(project).unwrap_or(project.to_path_buf()));
        let mut rows = vec![];
        if let Ok(read) = std::fs::read_dir(&directory) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "zip") {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                let created = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                rows.push(json!({
                    "name": entry.file_name().to_string_lossy(),
                    "path": path.to_string_lossy(),
                    "size": meta.len(),
                    "created_at": created,
                }));
            }
        }
        rows.sort_by(|a, b| {
            b["created_at"]
                .as_u64()
                .cmp(&a["created_at"].as_u64())
                .then(a["name"].as_str().cmp(&b["name"].as_str()))
        });
        Ok(rows)
    }

    /// Puts a snapshot back, keeping what is there now as a snapshot first.
    pub fn restore_backup(&self, project: &Path, name: &str) -> Result<Value> {
        if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name)
            || !name.ends_with(".zip")
        {
            return Err(Error::new("Invalid backup name"));
        }
        let _lock = OperationLock::acquire(&self.root)?;
        let info = self.project(project)?;
        let path = PathBuf::from(info["path"].as_str().unwrap_or(""));
        let archive = self.backup_directory(&path).join(name);
        if !archive.is_file() {
            return Err(Error::new("That backup no longer exists"));
        }
        self.require_idle(&path)?;
        eprintln!("Stage: Keeping the current project as a backup");
        let before = self.backup_locked(&info)?;
        eprintln!("Stage: Restoring the project");
        restore(&path, &archive, &|| self.require_idle(&path))?;
        let mut result = self.project(&path)?;
        result["restored"] = json!(name);
        result["previous"] = before;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stamp_is_a_utc_date() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_791_639_901), "20261010-134501");
    }

    #[test]
    fn a_backup_name_collision_leaves_the_existing_file_intact() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let destination = temp.path().join("snapshot.zip");
        std::fs::write(&destination, "already backed up").unwrap();
        assert!(create(&project, &destination, &|| Ok(())).is_err());
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "already backed up"
        );
    }

    #[test]
    fn a_failed_restore_leaves_the_project_intact() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let manifest = project.join("project.blockloom");
        let original = r#"{"name":"Original"}"#;
        std::fs::write(&manifest, original).unwrap();
        let archive = temp.path().join("snapshot.zip");
        for (name, contents) in [
            ("project.blockloom", "invalid JSON"),
            (".blockloom/lock.json", "excluded metadata"),
        ] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
            zip.finish().unwrap();
            assert!(restore(&project, &archive, &|| Ok(())).is_err());
            assert_eq!(std::fs::read_to_string(&manifest).unwrap(), original);
        }
    }

    #[test]
    fn unsafe_names_are_refused() {
        assert!(safe_name("assets/a.png"));
        assert!(!safe_name("../x"));
        assert!(!safe_name("/etc/passwd"));
        assert!(!safe_name("a/../../b"));
    }
}
