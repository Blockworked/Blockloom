//! How package files are read.
//!
//! Disk by default. A browser player has no disk: it mounts its game's files
//! in memory and installs a reader over them, so a shipped plugin loads the
//! same way there.

use std::io;
use std::path::Path;
use std::sync::OnceLock;

type Reader = fn(&Path) -> io::Result<Vec<u8>>;

static READER: OnceLock<Reader> = OnceLock::new();

/// Replaces how files are read, once, before any is. Later calls are ignored.
pub fn set_reader(reader: Reader) {
    let _ = READER.set(reader);
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match READER.get() {
        Some(reader) => reader(path),
        None => std::fs::read(path),
    }
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
