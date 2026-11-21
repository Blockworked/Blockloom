//! Moving a folder to the OS trash, so deleting a project can be undone.
//!
//! Linux follows the XDG trash spec for the home trash, macOS moves into
//! `~/.Trash` and Windows asks the shell to use the Recycle Bin. A folder on
//! another drive can't be renamed into the home trash; that is an error the
//! caller can answer with a permanent delete.

use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unique(dir: &Path, name: &str) -> PathBuf {
    let mut candidate = dir.join(name);
    let mut n = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{name}.{n}"));
        n += 1;
    }
    candidate
}

#[cfg(any(target_os = "linux", test))]
fn percent_encode(path: &str) -> String {
    let mut out = String::new();
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(target_os = "linux")]
pub fn move_to_trash(dir: &Path) -> Result<(), String> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")))
        .ok_or("No home folder to find the trash in")?
        .join("Trash");
    let (files, info) = (base.join("files"), base.join("info"));
    for folder in [&files, &info] {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let absolute = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = absolute
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let target = unique(&files, &name);
    let stored = target.file_name().unwrap().to_string_lossy().into_owned();
    let when = {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Local-time formatting needs no dependency this way: UTC, which the
        // spec's reader treats as a plain timestamp.
        let (days, rem) = (secs / 86400, secs % 86400);
        let (y, m, d) = civil(days as i64);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        )
    };
    let record = format!(
        "[Trash Info]\nPath={}\nDeletionDate={when}\n",
        percent_encode(&absolute.to_string_lossy())
    );
    let record_path = info.join(format!("{stored}.trashinfo"));
    std::fs::write(&record_path, record).map_err(|e| format!("{}: {e}", record_path.display()))?;
    if let Err(e) = std::fs::rename(&absolute, &target) {
        let _ = std::fs::remove_file(&record_path);
        return Err(format!(
            "Couldn't move {} to the trash ({e}). It may be on another drive.",
            absolute.display()
        ));
    }
    Ok(())
}

/// Days since 1970-01-01 as a civil date.
#[cfg(any(target_os = "linux", test))]
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(target_os = "macos")]
pub fn move_to_trash(dir: &Path) -> Result<(), String> {
    let trash = dirs::home_dir().ok_or("No home folder")?.join(".Trash");
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let target = unique(&trash, &name);
    std::fs::rename(dir, &target)
        .map_err(|e| format!("Couldn't move {} to the trash ({e}).", dir.display()))
}

#[cfg(windows)]
pub fn move_to_trash(dir: &Path) -> Result<(), String> {
    let script = "Add-Type -AssemblyName Microsoft.VisualBasic; \
        [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($args[0], 'OnlyErrorDialogs', 'SendToRecycleBin')";
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .arg(dir)
        .status()
        .map_err(|e| format!("Couldn't reach the Recycle Bin: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Couldn't move {} to the Recycle Bin.",
            dir.display()
        ))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn move_to_trash(_dir: &Path) -> Result<(), String> {
    Err("This system has no trash to move the folder to.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_percent_encoded_and_dates_are_civil() {
        assert_eq!(percent_encode("/a b/é"), "/a%20b/%C3%A9");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19723), (2024, 1, 1));
    }
}
