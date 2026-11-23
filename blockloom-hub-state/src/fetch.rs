//! Bounded HTTPS downloads, release catalogs and safe archive extraction.

use crate::run::copy_file;
use crate::{Error, Result, checkpoint};
use serde_json::Value;
use sha2::Digest;
use std::collections::HashSet;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const MAX_ARCHIVE: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_EXTRACTED: u64 = 24 * 1024 * 1024 * 1024;
const MAX_MEMBERS: u64 = 100_000;
const ATTEMPTS: u32 = 8;
const MAX_RETRY_DELAY: u64 = 30;
const MIB: f64 = 1024.0 * 1024.0;

/// An answer from a server, before its body is read.
pub struct Remote {
    pub status: u16,
    pub content_range: Option<String>,
    pub body: Box<dyn Read + Send>,
}

/// What went wrong with one attempt: worth another try, or not.
enum Failure {
    Transient(String),
    Fatal(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Fatal(e)
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Fatal(e.into())
    }
}

type Opener = dyn Fn(&str, u64) -> std::result::Result<Remote, String>;

thread_local! {
    static FAKE: std::cell::RefCell<Option<Box<Opener>>> = const { std::cell::RefCell::new(None) };
}

/// Answers requests from `opener` instead of the network on this thread.
#[cfg(test)]
pub fn fake_network(opener: impl Fn(&str, u64) -> std::result::Result<Remote, String> + 'static) {
    FAKE.with(|f| *f.borrow_mut() = Some(Box::new(opener)));
}

/// An HTTPS URL with no credentials or fragment.
pub fn secure_url(url: &str) -> Result<url::Url> {
    let parsed = url::Url::parse(url).map_err(|_| Error::new(SECURE))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none_or(str::is_empty)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Error::new(SECURE));
    }
    Ok(parsed)
}

const SECURE: &str = "Downloads require an HTTPS URL without credentials or fragments";

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent("Blockloom-Hub")
        .build()
        .into()
}

fn open(url: &str, start: u64) -> std::result::Result<Remote, Failure> {
    checkpoint()?;
    secure_url(url)?;
    if let Some(answer) = FAKE.with(|f| f.borrow().as_ref().map(|open| open(url, start))) {
        return answer.map_err(Failure::Transient);
    }
    let mut request = agent().get(url).header("Accept-Encoding", "identity");
    if start > 0 {
        request = request.header("Range", format!("bytes={start}-"));
    }
    let response = request
        .call()
        .map_err(|e| Failure::Transient(e.to_string()))?;
    let status = response.status().as_u16();
    let content_range = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = response.into_body().into_reader();
    Ok(Remote {
        status,
        content_range,
        body: Box::new(body),
    })
}

fn check_status(remote: &Remote, url: &str) -> std::result::Result<(), Failure> {
    match remote.status {
        200..=299 => Ok(()),
        429 | 500..=599 => Err(Failure::Transient(format!("HTTP {}", remote.status))),
        status => Err(Failure::Fatal(Error::new(format!(
            "HTTP {status} from {url}"
        )))),
    }
}

fn sleep_interruptible(delay: Duration) -> Result<()> {
    let steps = (delay.as_secs_f64() / 0.2).max(1.0) as u32;
    for _ in 0..steps {
        std::thread::sleep(delay / steps);
        checkpoint()?;
    }
    Ok(())
}

/// Retries `action` with backoff while it fails in a way worth retrying.
fn retrying<T>(
    url: &str,
    mut action: impl FnMut() -> std::result::Result<T, Failure>,
) -> Result<T> {
    let mut attempt = 0;
    loop {
        match action() {
            Ok(value) => return Ok(value),
            Err(Failure::Fatal(e)) => return Err(e),
            Err(Failure::Transient(message)) => {
                checkpoint()?;
                attempt += 1;
                if attempt >= ATTEMPTS {
                    return Err(Error::new(format!(
                        "Download failed after {ATTEMPTS} attempts: {url} ({message}). Check your connection and retry."
                    )));
                }
                let delay = (1u64 << attempt).min(MAX_RETRY_DELAY);
                eprintln!(
                    "Download stalled ({message}); retrying {}/{ATTEMPTS} in {delay}s...",
                    attempt + 1
                );
                sleep_interruptible(Duration::from_secs(delay))?;
            }
        }
    }
}

fn read_error(e: std::io::Error) -> Failure {
    Failure::Transient(e.to_string())
}

/// A small remote file, whole.
pub fn read_remote(url: &str, limit: u64) -> Result<Vec<u8>> {
    retrying(url, || {
        let remote = open(url, 0)?;
        check_status(&remote, url)?;
        let mut data = vec![];
        let mut body = remote.body.take(limit + 1);
        let mut buffer = [0u8; 65536];
        loop {
            checkpoint()?;
            let n = body.read(&mut buffer).map_err(read_error)?;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buffer[..n]);
        }
        if data.len() as u64 > limit {
            return Err(Error::new("Remote manifest exceeds the size limit").into());
        }
        Ok(data)
    })
}

/// Canonical SemVer without build metadata.
pub fn version(value: &str) -> Result<&str> {
    let pattern = regex::Regex::new(
        r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$",
    )
    .expect("pattern");
    if !pattern.is_match(value) {
        return Err(Error::new(
            "Release version must be canonical SemVer without build metadata",
        ));
    }
    if let Some((_, pre)) = value.split_once('-') {
        for part in pre.split('.') {
            if part.len() > 1 && part.starts_with('0') && part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::new(
                    "Numeric SemVer prerelease identifiers cannot start with zero",
                ));
            }
        }
    }
    Ok(value)
}

/// A SHA-256 in lower case.
pub fn digest(value: &str) -> Result<String> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::new("Archive must declare its SHA-256"));
    }
    Ok(value.to_lowercase())
}

/// The releases for `target` in a catalog at `url`.
pub fn catalog(url: &str, target: &str) -> Result<Vec<Value>> {
    let raw = read_remote(url, 2 * 1024 * 1024)?;
    let Ok(data) = serde_json::from_slice::<Value>(&raw) else {
        // Pasting a repository page is the common mistake.
        if matches!(host_of(url).as_str(), "github.com" | "www.github.com") {
            return Err(Error::new(
                "This looks like a GitHub repository page, not a release catalog. \
                 In Hub settings, enable 'Use GitHub CLI for private releases' instead.",
            ));
        }
        return Err(Error::new(
            "The catalog URL did not return a release catalog (JSON with schema 1). Check it in Hub settings.",
        ));
    };
    catalog_data(&data, target)
}

/// Checks every entry of a schema 1 catalog, then keeps `target`'s.
pub fn catalog_data(data: &Value, target: &str) -> Result<Vec<Value>> {
    let releases = match (data["schema"].as_i64(), data["releases"].as_array()) {
        (Some(1), Some(releases)) => releases,
        _ => return Err(Error::new("Expected a schema 1 Blockloom release catalog")),
    };
    if releases.len() > 1000 {
        return Err(Error::new("Release catalog has too many entries"));
    }
    let mut entries = vec![];
    let mut seen = HashSet::new();
    for entry in releases {
        if !entry.is_object() {
            return Err(Error::new("Invalid release catalog entry"));
        }
        version(entry["version"].as_str().unwrap_or(""))?;
        secure_url(entry["url"].as_str().unwrap_or(""))?;
        digest(entry["sha256"].as_str().unwrap_or(""))?;
        let bounded =
            |name: &str, max: u64| entry[name].as_u64().is_some_and(|n| n > 0 && n <= max);
        if !bounded("size", MAX_ARCHIVE) {
            return Err(Error::new(
                "Release must declare a bounded archive byte count",
            ));
        }
        if !bounded("unpacked_size", MAX_EXTRACTED) {
            return Err(Error::new(
                "Release must declare a bounded extraction byte count",
            ));
        }
        let format_ok = matches!(entry["format"].as_str(), Some("zip" | "tar.xz" | "tar.gz"));
        let Some(entry_target) = entry["target"].as_str().filter(|_| format_ok) else {
            return Err(Error::new("Release archive format or target is invalid"));
        };
        if !seen.insert((
            entry["version"].as_str().unwrap_or("").to_string(),
            entry_target.to_string(),
        )) {
            return Err(Error::new(
                "Catalog contains duplicate releases for the same version and target",
            ));
        }
        if entry_target == target {
            entries.push(entry.clone());
        }
    }
    Ok(entries)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Checksum {
    Sha256,
    Sha1,
}

enum Hasher {
    Sha256(sha2::Sha256),
    Sha1(sha1_smol::Sha1),
}

impl Hasher {
    fn new(kind: Checksum) -> Self {
        match kind {
            Checksum::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            Checksum::Sha1 => Hasher::Sha1(sha1_smol::Sha1::new()),
        }
    }
    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
        }
    }
    fn hex(&self) -> String {
        match self {
            Hasher::Sha256(h) => h
                .clone()
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            Hasher::Sha1(h) => h.digest().to_string(),
        }
    }
}

/// Downloads `url` to `destination`, resuming a partial file, and checks
/// its size and checksum. A failed check is retried like a dropped
/// connection, since it is most likely a truncated transfer.
pub fn download(
    url: &str,
    destination: &Path,
    checksum: &str,
    size: Option<u64>,
    limit: u64,
    kind: Checksum,
) -> Result<()> {
    let length = if kind == Checksum::Sha256 { 64 } else { 40 };
    if checksum.len() != length || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::new("Archive must declare its checksum"));
    }
    let expected = checksum.to_lowercase();
    let result = retrying(url, || {
        fetch_once(url, destination, &expected, size, limit, kind)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

fn fetch_once(
    url: &str,
    destination: &Path,
    expected: &str,
    size: Option<u64>,
    limit: u64,
    kind: Checksum,
) -> std::result::Result<(), Failure> {
    let mut start = std::fs::metadata(destination).map(|m| m.len()).unwrap_or(0);
    if size.is_some_and(|s| start > s) {
        let _ = std::fs::remove_file(destination);
        start = 0;
    }
    let mut hash = Hasher::new(kind);
    if start > 0 {
        let mut existing = std::fs::File::open(destination)?;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            checkpoint()?;
            let n = existing.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        eprintln!("Resuming download from {:.1} MiB...", start as f64 / MIB);
    }
    let mut remote = open(url, start)?;
    if start > 0 {
        let resumed = remote.status == 206
            && remote
                .content_range
                .as_deref()
                .is_none_or(|r| r.starts_with(&format!("bytes {start}-")));
        if !resumed {
            // The server can't resume (416, or a whole body): start over.
            let _ = std::fs::remove_file(destination);
            hash = Hasher::new(kind);
            start = 0;
            if remote.status != 200 {
                remote = open(url, 0)?;
            }
        }
    }
    check_status(&remote, url)?;
    let mut output = if start > 0 {
        std::fs::OpenOptions::new().append(true).open(destination)?
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?
    };
    let mut amount = start;
    let mut last = Instant::now() - Duration::from_secs(2);
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        checkpoint()?;
        let n = remote.body.read(&mut buffer).map_err(read_error)?;
        if n == 0 {
            break;
        }
        amount += n as u64;
        if amount > limit || size.is_some_and(|s| amount > s) {
            return Err(Error::new("Download exceeds the declared size").into());
        }
        output.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
        if last.elapsed() >= Duration::from_secs(1) {
            match size {
                Some(s) => eprintln!(
                    "Downloading: {:.1} MiB / {:.1} MiB",
                    amount as f64 / MIB,
                    s as f64 / MIB
                ),
                None => eprintln!("Downloading: {:.1} MiB", amount as f64 / MIB),
            }
            last = Instant::now();
        }
    }
    output.flush()?;
    let got = hash.hex();
    if size.is_some_and(|s| amount != s) || got != expected {
        let algorithm = if kind == Checksum::Sha256 {
            "sha256"
        } else {
            "sha1"
        };
        let _ = std::fs::remove_file(destination);
        return Err(Failure::Transient(format!(
            "Download size or {algorithm} does not match the manifest: {url} (got {amount} bytes, \
             expected {} bytes; got {algorithm} {got}, expected {expected})",
            size.map(|s| s.to_string()).unwrap_or_else(|| "any".into())
        )));
    }
    checkpoint()?;
    Ok(())
}

/// Checks a file against a catalog's size and SHA-256.
pub fn verify_file(path: &Path, sha256: &str, size: u64) -> Result<()> {
    if std::fs::metadata(path)?.len() != size || size > MAX_ARCHIVE {
        return Err(Error::new(
            "Downloaded asset size does not match the catalog",
        ));
    }
    let mut hash = sha2::Sha256::new();
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        checkpoint()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != digest(sha256)? {
        return Err(Error::new(
            "Downloaded asset SHA-256 does not match the catalog",
        ));
    }
    Ok(())
}

fn case_sensitive(directory: &Path) -> bool {
    let Ok(probe) = tempfile::Builder::new()
        .prefix(".hub-case-")
        .tempdir_in(directory)
    else {
        return false;
    };
    if std::fs::write(probe.path().join("case"), "").is_err() {
        return false;
    }
    !probe.path().join("CASE").exists()
}

/// How an archive may be laid out on disk.
#[derive(Clone, Copy)]
pub struct Extract {
    pub limit: u64,
    /// Tool archives hold compiler aliases: copy each target in as a file.
    pub allow_links: bool,
    /// Linux tool archives hold headers whose names differ only by case.
    pub allow_case_sensitive: bool,
}

impl Extract {
    pub fn bundle(limit: u64) -> Self {
        Self {
            limit,
            allow_links: false,
            allow_case_sensitive: false,
        }
    }
    pub fn tool() -> Self {
        Self {
            limit: MAX_EXTRACTED,
            allow_links: true,
            allow_case_sensitive: true,
        }
    }
}

struct Extractor {
    destination: PathBuf,
    options: Extract,
    case_sensitive: bool,
    names: HashSet<String>,
    total: u64,
    count: u64,
    last: Instant,
    links: Vec<(PathBuf, PathBuf)>,
}

const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

impl Extractor {
    fn path_for(&mut self, name: &str, size: u64) -> Result<PathBuf> {
        checkpoint()?;
        let parts: Vec<&str> = name
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        if parts.is_empty()
            || name.starts_with('/')
            || name.contains('\\')
            || name.contains(':')
            || parts.contains(&"..")
        {
            return Err(Error::new("Archive contains an unsafe path"));
        }
        let nonportable = parts.iter().any(|p| {
            let stem = p.split('.').next().unwrap_or("").to_uppercase();
            p.ends_with('.')
                || p.ends_with(' ')
                || RESERVED.contains(&stem.as_str())
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        });
        if nonportable {
            return Err(Error::new("Archive contains a nonportable path"));
        }
        let mut key = parts.join("/");
        if !self.case_sensitive {
            key = key.to_lowercase();
        }
        if !self.names.insert(key) {
            return Err(Error::new(format!(
                "Archive contains duplicate paths: {name}"
            )));
        }
        self.count += 1;
        self.total += size;
        if self.count > MAX_MEMBERS || self.total > self.options.limit {
            return Err(Error::new("Archive exceeds extraction limits"));
        }
        if self.last.elapsed() >= Duration::from_secs(1) {
            eprintln!(
                "Extracting: {} entries, {:.0} MiB",
                self.count,
                self.total as f64 / MIB
            );
            self.last = Instant::now();
        }
        Ok(parts
            .iter()
            .fold(self.destination.clone(), |p, part| p.join(part)))
    }

    fn link(&mut self, path: PathBuf, target: &str, relative: bool) -> Result<()> {
        if target.starts_with('/') || target.contains('\\') || target.contains(':') {
            return Err(Error::new("Archive contains an unsafe link"));
        }
        let base = if relative {
            let parent = path.parent().unwrap_or(&self.destination);
            parent
                .strip_prefix(&self.destination)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let joined = if base.is_empty() {
            target.to_string()
        } else {
            format!("{base}/{target}")
        };
        let mut parts: Vec<&str> = vec![];
        for part in joined.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(Error::new("Archive link escapes its destination"));
                    }
                }
                other => parts.push(other),
            }
        }
        let resolved = parts
            .iter()
            .fold(self.destination.clone(), |p, part| p.join(part));
        self.links.push((path, resolved));
        Ok(())
    }

    fn write(&self, stream: &mut dyn Read, path: &Path, size: u64, mode: u32) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut remaining = size;
        let mut buffer = vec![0u8; 1024 * 1024];
        while remaining > 0 {
            checkpoint()?;
            let want = remaining.min(buffer.len() as u64) as usize;
            let n = stream.read(&mut buffer[..want])?;
            if n == 0 {
                return Err(Error::new("Archive file is truncated"));
            }
            output.write_all(&buffer[..n])?;
            remaining -= n as u64;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bits = mode & 0o777;
            std::fs::set_permissions(
                path,
                std::fs::Permissions::from_mode(if bits == 0 { 0o644 } else { bits }),
            )?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        Ok(())
    }

    fn zip(&mut self, archive: &Path) -> Result<()> {
        let mut bundle = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        for index in 0..bundle.len() {
            let mut entry = bundle.by_index(index)?;
            let mode = entry.unix_mode().unwrap_or(0);
            let kind = mode & 0o170000;
            let name = entry.name().to_string();
            if kind == 0o120000 {
                if !self.options.allow_links {
                    return Err(Error::new(
                        "Archive links and special files are not supported",
                    ));
                }
                let path = self.path_for(&name, entry.size())?;
                if entry.size() > 4096 {
                    return Err(Error::new("Archive link target is too long"));
                }
                let mut target = String::new();
                entry.read_to_string(&mut target)?;
                self.link(path, &target, true)?;
                continue;
            }
            if !matches!(kind, 0 | 0o100000 | 0o040000) {
                return Err(Error::new(
                    "Archive links and special files are not supported",
                ));
            }
            let path = self.path_for(&name, entry.size())?;
            if entry.is_dir() {
                std::fs::create_dir_all(&path)?;
            } else {
                let size = entry.size();
                self.write(&mut entry, &path, size, mode)?;
            }
        }
        Ok(())
    }

    fn tar(&mut self, stream: impl Read) -> Result<()> {
        let mut bundle = tar::Archive::new(stream);
        for entry in bundle.entries()? {
            let mut entry = entry?;
            let header = entry.header().clone();
            let name = entry.path()?.to_string_lossy().replace('\\', "/");
            let kind = header.entry_type();
            if kind.is_symlink() || kind.is_hard_link() {
                if !self.options.allow_links {
                    return Err(Error::new(
                        "Archive links and special files are not supported",
                    ));
                }
                let path = self.path_for(&name, 0)?;
                let target = entry
                    .link_name()?
                    .map(|t| t.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                self.link(path, &target, kind.is_symlink())?;
                continue;
            }
            if !(kind.is_file() || kind.is_dir()) {
                // Pax and GNU long-name headers are folded in by the reader.
                if matches!(
                    kind,
                    tar::EntryType::XGlobalHeader | tar::EntryType::XHeader
                ) {
                    continue;
                }
                return Err(Error::new(
                    "Archive links and special files are not supported",
                ));
            }
            let size = entry.size();
            let path = self.path_for(&name, size)?;
            if kind.is_dir() {
                std::fs::create_dir_all(&path)?;
            } else {
                let mode = header.mode().unwrap_or(0o644);
                self.write(&mut entry, &path, size, mode)?;
            }
        }
        Ok(())
    }

    fn resolve_links(&mut self) -> Result<()> {
        let mut links = std::mem::take(&mut self.links);
        while !links.is_empty() {
            let mut pending = vec![];
            let before = links.len();
            for (path, target) in links {
                checkpoint()?;
                if !target.is_file() {
                    pending.push((path, target));
                    continue;
                }
                self.total += std::fs::metadata(&target)?.len();
                if self.total > self.options.limit {
                    return Err(Error::new("Archive exceeds extraction limits"));
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                copy_file(&target, &path)?;
            }
            if pending.len() == before {
                return Err(Error::new("Archive contains unresolved or non-file links"));
            }
            links = pending;
        }
        Ok(())
    }
}

/// Unpacks a `zip`, `tar.gz` or `tar.xz` into a new `destination`, refusing
/// paths that escape it or don't work on every platform.
pub fn extract(archive: &Path, destination: &Path, format: &str, options: Extract) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let mut extractor = Extractor {
        destination: destination.to_path_buf(),
        options,
        case_sensitive: options.allow_case_sensitive && case_sensitive(destination),
        names: HashSet::new(),
        total: 0,
        count: 0,
        last: Instant::now(),
        links: vec![],
    };
    match format {
        "zip" => extractor.zip(archive)?,
        "tar.gz" => extractor.tar(flate2::read::GzDecoder::new(BufReader::new(
            std::fs::File::open(archive)?,
        )))?,
        "tar.xz" => {
            // lzma-rs decodes into a writer, so the tar goes through a file.
            let parent = destination.parent().unwrap_or(Path::new("."));
            let mut tarball = tempfile::Builder::new()
                .prefix(".hub-xz-")
                .tempfile_in(parent)?;
            let mut input = BufReader::new(std::fs::File::open(archive)?);
            let mut limited = Limited {
                inner: tarball.as_file_mut(),
                left: options.limit.saturating_add(64 * 1024 * 1024),
            };
            lzma_rs::xz_decompress(&mut input, &mut limited)
                .map_err(|e| Error::new(format!("Archive is not valid xz: {e}")))?;
            extractor.tar(BufReader::new(std::fs::File::open(tarball.path())?))?;
        }
        other => return Err(Error::new(format!("Unknown archive format: {other}"))),
    }
    extractor.resolve_links()?;
    checkpoint()
}

/// A writer that refuses to grow past a bound, so a hostile xz can't fill
/// the disk before the tar limits are checked.
struct Limited<'a> {
    inner: &'a mut std::fs::File,
    left: u64,
}

impl Write for Limited<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() as u64 > self.left {
            return Err(std::io::Error::other("Archive exceeds extraction limits"));
        }
        if checkpoint().is_err() {
            return Err(std::io::Error::other("Operation cancelled"));
        }
        self.left -= data.len() as u64;
        self.inner.write(data)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Downloads and unpacks a tool archive, answering its one top folder.
pub fn tool_archive(
    url: &str,
    checksum: &str,
    size: u64,
    temporary: &Path,
    name: &str,
    kind: Checksum,
) -> Result<PathBuf> {
    let path = secure_url(url)?.path().to_string();
    let format = if path.ends_with(".zip") {
        "zip"
    } else {
        "tar.gz"
    };
    let archive = temporary.join(format!("{name}.{format}"));
    download(url, &archive, checksum, Some(size), MAX_ARCHIVE, kind)?;
    let output = temporary.join(name);
    extract(&archive, &output, format, Extract::tool())?;
    let roots: Vec<PathBuf> = std::fs::read_dir(&output)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "__MACOSX"))
        .collect();
    if roots.len() != 1 {
        return Err(Error::new("Tool archive has an unexpected layout"));
    }
    Ok(roots.into_iter().next().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serve(body: Vec<u8>) {
        fake_network(move |_, start| {
            Ok(Remote {
                status: if start > 0 { 206 } else { 200 },
                content_range: (start > 0).then(|| format!("bytes {start}-")),
                body: Box::new(std::io::Cursor::new(body[start as usize..].to_vec())),
            })
        });
    }

    fn sha(data: &[u8]) -> String {
        sha2::Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn urls_must_be_plain_https() {
        assert!(secure_url("https://example.com/a.zip").is_ok());
        for bad in [
            "http://example.com/a",
            "https://u:p@example.com/a",
            "https://example.com/a#x",
            "file:///etc",
        ] {
            assert!(secure_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_download_checks_size_and_hash_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let body = b"hello world".to_vec();
        serve(body.clone());
        let path = dir.path().join("file");
        download(
            "https://example.com/f",
            &path,
            &sha(&body),
            Some(11),
            MAX_ARCHIVE,
            Checksum::Sha256,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        // A partial file resumes from where it stopped.
        std::fs::write(&path, b"hello").unwrap();
        download(
            "https://example.com/f",
            &path,
            &sha(&body),
            Some(11),
            MAX_ARCHIVE,
            Checksum::Sha256,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        // A size larger than declared is refused and leaves nothing.
        std::fs::remove_file(&path).unwrap();
        let error = download(
            "https://example.com/f",
            &path,
            &sha(&body),
            Some(5),
            MAX_ARCHIVE,
            Checksum::Sha256,
        )
        .unwrap_err();
        assert!(
            error.message.contains("exceeds the declared size"),
            "{error}"
        );
        assert!(!path.exists());
    }

    #[test]
    fn sha1_checksums_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        serve(b"abc".to_vec());
        let path = dir.path().join("file");
        let sha1 = "a9993e364706816aba3e25717850c26c9cd0d89d";
        download(
            "https://example.com/f",
            &path,
            sha1,
            Some(3),
            MAX_ARCHIVE,
            Checksum::Sha1,
        )
        .unwrap();
        assert!(
            download(
                "https://example.com/f",
                &dir.path().join("x"),
                &sha(b"abc"),
                Some(3),
                MAX_ARCHIVE,
                Checksum::Sha1
            )
            .is_err()
        );
    }

    #[test]
    fn catalogs_keep_this_host_and_refuse_duplicates() {
        let entry = |target: &str| {
            serde_json::json!({"version": "0.1.0", "url": "https://example.com/a.zip",
            "sha256": "a".repeat(64), "size": 10, "unpacked_size": 20, "format": "zip", "target": target})
        };
        let data = serde_json::json!({"schema": 1, "releases": [entry("x"), entry("y")]});
        assert_eq!(catalog_data(&data, "x").unwrap().len(), 1);
        let twice = serde_json::json!({"schema": 1, "releases": [entry("x"), entry("x")]});
        assert!(catalog_data(&twice, "x").is_err());
        assert!(version("1.02.0").is_err());
        assert!(version("1.2.0-rc.01").is_err());
        assert!(version("1.2.0-rc.1").is_ok());
    }

    #[test]
    fn a_page_instead_of_a_catalog_says_what_to_change() {
        serve(b"<html>".to_vec());
        let error = catalog("https://github.com/owner/repo", "x").unwrap_err();
        assert!(error.message.contains("GitHub CLI"), "{error}");
    }

    fn tar_gz(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
        let mut builder = tar::Builder::new(vec![]);
        build(&mut builder);
        let data = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
        gz.write_all(&data).unwrap();
        gz.finish().unwrap()
    }

    fn file(builder: &mut tar::Builder<Vec<u8>>, name: &str, data: &[u8], mode: u32) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, name, data).unwrap();
    }

    fn link(builder: &mut tar::Builder<Vec<u8>>, name: &str, target: &str, kind: tar::EntryType) {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(kind);
        builder.append_link(&mut header, name, target).unwrap();
    }

    #[test]
    fn tool_links_become_regular_files_and_escapes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        std::fs::write(
            &archive,
            tar_gz(|b| {
                file(b, "tool/bin/clang", b"compiler", 0o755);
                link(b, "tool/bin/clang++", "clang", tar::EntryType::Symlink);
                link(b, "tool/bin/cc", "tool/bin/clang", tar::EntryType::Link);
            }),
        )
        .unwrap();
        let out = dir.path().join("out");
        extract(&archive, &out, "tar.gz", Extract::tool()).unwrap();
        for name in ["clang++", "cc"] {
            let path = out.join("tool/bin").join(name);
            assert!(
                !std::fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"compiler");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(out.join("tool/bin/clang"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
        // Bundles refuse links, and a link may not leave the archive.
        assert!(
            extract(
                &archive,
                &dir.path().join("bundle"),
                "tar.gz",
                Extract::bundle(MAX_EXTRACTED)
            )
            .is_err()
        );
        std::fs::write(
            &archive,
            tar_gz(|b| link(b, "tool/x", "../../etc/passwd", tar::EntryType::Symlink)),
        )
        .unwrap();
        assert!(
            extract(
                &archive,
                &dir.path().join("escape"),
                "tar.gz",
                Extract::tool()
            )
            .is_err()
        );
    }

    #[test]
    fn unsafe_and_oversized_archives_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        let write_zip = |names: &[(&str, &[u8])]| {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            for (name, data) in names {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        };
        write_zip(&[("a/CON.txt", b"x")]);
        assert!(
            extract(
                &archive,
                &dir.path().join("1"),
                "zip",
                Extract::bundle(MAX_EXTRACTED)
            )
            .is_err()
        );
        write_zip(&[("a/b", b"x"), ("a/B", b"y")]);
        assert!(
            extract(
                &archive,
                &dir.path().join("2"),
                "zip",
                Extract::bundle(MAX_EXTRACTED)
            )
            .is_err()
        );
        write_zip(&[("a/big", &[0u8; 100])]);
        assert!(extract(&archive, &dir.path().join("3"), "zip", Extract::bundle(10)).is_err());
        write_zip(&[("a/ok", b"fine")]);
        extract(
            &archive,
            &dir.path().join("4"),
            "zip",
            Extract::bundle(MAX_EXTRACTED),
        )
        .unwrap();
        assert_eq!(std::fs::read(dir.path().join("4/a/ok")).unwrap(), b"fine");
    }
}
