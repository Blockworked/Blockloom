//! What an importer or a build hook is asked, and what it answers.
//!
//! Both are module ops over raw bytes, so a source file or a produced one
//! never goes through base64. A request is one line of JSON ([`Request`]),
//! a newline, then the source file's bytes. An answer is one line of JSON
//! (the report), a newline, then each produced file's bytes back to back in
//! the order the report lists them. The host does every read and write: a
//! module sees only the bytes it is handed and names only paths relative to
//! the folder the host gives its output.

use serde::{Deserialize, Serialize};

/// The op an importer named `name` answers, `importer.<name>`.
pub fn importer_op(name: &str) -> String {
    format!("importer.{name}")
}

/// The op a build hook named `name` answers, `build.<name>`.
pub fn build_op(name: &str) -> String {
    format!("build.{name}")
}

/// Most files one call may produce.
pub const MAX_FILES: usize = 1024;
/// Most bytes one call may produce in all.
pub const MAX_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
/// The longest an output path may be.
pub const MAX_PATH_BYTES: usize = 200;
/// The budget a call gets when its schema names none, in milliseconds.
pub const DEFAULT_LIMIT_MS: u32 = 5_000;
/// The most a schema may ask for.
pub const MAX_LIMIT_MS: u32 = 120_000;

/// What the host asks of a module. Only the fields its kind uses are set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// `importer` or `build`.
    pub what: String,
    /// The importer's or the hook's name in its plugin.
    pub name: String,
    /// Importer: the source file, relative to the project (`assets/...`).
    #[serde(default)]
    pub path: String,
    /// Importer: the source's extension, lowercase and without the dot.
    #[serde(default)]
    pub extension: String,
    /// Build: the target triple.
    #[serde(default)]
    pub target: String,
    /// The project's name.
    #[serde(default)]
    pub project: String,
    /// Build: every file under `assets/`, relative to the project.
    #[serde(default)]
    pub assets: Vec<String>,
}

impl Request {
    /// The wire form: this request's line, a newline, then `data`.
    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        let mut out = serde_json::to_vec(self).unwrap_or_default();
        out.push(b'\n');
        out.extend_from_slice(data);
        out
    }

    /// Splits a request back into itself and its bytes.
    pub fn decode(bytes: &[u8]) -> Result<(Request, &[u8]), String> {
        let (line, rest) = split_line(bytes)?;
        let request = serde_json::from_slice(line).map_err(|e| format!("the request: {e}"))?;
        Ok((request, rest))
    }
}

/// A file a call produced: where, relative to the output folder, and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedFile {
    pub path: String,
    pub data: Vec<u8>,
}

/// What a module answers: the files it made and what it has to say about
/// the work. A non-empty `errors` fails the call, and with it the build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Produced {
    pub files: Vec<ProducedFile>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    /// Project files besides the source that the result was made from, so
    /// the host knows when it is stale.
    pub dependencies: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Report {
    #[serde(default)]
    files: Vec<Listed>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Listed {
    path: String,
    size: u64,
}

impl Produced {
    pub fn file(mut self, path: impl Into<String>, data: Vec<u8>) -> Self {
        self.files.push(ProducedFile {
            path: path.into(),
            data,
        });
        self
    }

    pub fn warn(mut self, message: impl Into<String>) -> Self {
        self.warnings.push(message.into());
        self
    }

    pub fn error(mut self, message: impl Into<String>) -> Self {
        self.errors.push(message.into());
        self
    }

    pub fn depends_on(mut self, path: impl Into<String>) -> Self {
        self.dependencies.push(path.into());
        self
    }

    /// The wire form an op answers with.
    pub fn encode(&self) -> Vec<u8> {
        let report = Report {
            files: self
                .files
                .iter()
                .map(|f| Listed {
                    path: f.path.clone(),
                    size: f.data.len() as u64,
                })
                .collect(),
            warnings: self.warnings.clone(),
            errors: self.errors.clone(),
            dependencies: self.dependencies.clone(),
        };
        let mut out = serde_json::to_vec(&report).unwrap_or_default();
        out.push(b'\n');
        for file in &self.files {
            out.extend_from_slice(&file.data);
        }
        out
    }

    /// Reads an answer, refusing one that lies about its sizes, names a path
    /// outside the output folder, repeats a path or is larger than the
    /// limits allow.
    pub fn decode(bytes: &[u8]) -> Result<Produced, String> {
        let (line, mut rest) = split_line(bytes)?;
        let report: Report =
            serde_json::from_slice(line).map_err(|e| format!("the answer: {e}"))?;
        if report.files.len() > MAX_FILES {
            return Err(format!(
                "the answer lists {} files, more than the {MAX_FILES} allowed",
                report.files.len()
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut files = Vec::new();
        let mut total = 0u64;
        for listed in report.files {
            check_output_path(&listed.path)?;
            if !seen.insert(listed.path.clone()) {
                return Err(format!("the answer names {} twice", listed.path));
            }
            total = total.saturating_add(listed.size);
            if total > MAX_OUTPUT_BYTES {
                return Err(format!(
                    "the answer is larger than the {} MiB allowed",
                    MAX_OUTPUT_BYTES / (1024 * 1024)
                ));
            }
            let size = usize::try_from(listed.size)
                .ok()
                .filter(|size| *size <= rest.len())
                .ok_or_else(|| format!("{} is listed larger than the answer", listed.path))?;
            let (data, tail) = rest.split_at(size);
            files.push(ProducedFile {
                path: listed.path,
                data: data.to_vec(),
            });
            rest = tail;
        }
        if !rest.is_empty() {
            return Err(format!("the answer carries {} unlisted bytes", rest.len()));
        }
        Ok(Produced {
            files,
            warnings: report.warnings,
            errors: report.errors,
            dependencies: report.dependencies,
        })
    }
}

fn split_line(bytes: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let end = bytes
        .iter()
        .position(|b| *b == b'\n')
        .ok_or("no header line")?;
    Ok((&bytes[..end], &bytes[end + 1..]))
}

/// A path a module may write under its output folder: relative, forward
/// slashes, plain names only.
pub fn check_output_path(path: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("\"{path}\" is not an output path: {why}"));
    if path.is_empty() {
        return bad("it is empty");
    }
    if path.len() > MAX_PATH_BYTES {
        return bad("it is too long");
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return bad("it must be relative with forward slashes");
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment.starts_with('.') {
            return bad("a name is empty or starts with a dot");
        }
        if segment.chars().any(char::is_control) {
            return bad("a name has control characters");
        }
        if segment.ends_with(' ') {
            return bad("a name ends with a space");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_with_its_bytes() {
        let request = Request {
            what: "importer".to_string(),
            name: "gpl".to_string(),
            path: "assets/a.gpl".to_string(),
            extension: "gpl".to_string(),
            ..Default::default()
        };
        let wire = request.encode(b"line one\nline two");
        let (back, data) = Request::decode(&wire).unwrap();
        assert_eq!(back, request);
        assert_eq!(data, b"line one\nline two");
    }

    #[test]
    fn an_answer_round_trips_with_its_files_in_order() {
        let made = Produced::default()
            .file("a.png", vec![1, 2, 3])
            .file("sub/b.txt", b"b\n".to_vec())
            .warn("one colour was clamped")
            .depends_on("assets/shared.gpl");
        let back = Produced::decode(&made.encode()).unwrap();
        assert_eq!(back, made);
    }

    #[test]
    fn an_answer_that_lies_about_its_sizes_is_refused() {
        let mut wire = Produced::default().file("a.bin", vec![9; 4]).encode();
        wire.pop();
        assert!(Produced::decode(&wire).unwrap_err().contains("larger"));
        let mut wire = Produced::default().file("a.bin", vec![9; 4]).encode();
        wire.push(0);
        assert!(Produced::decode(&wire).unwrap_err().contains("unlisted"));
        assert!(Produced::decode(b"no newline").is_err());
    }

    #[test]
    fn a_path_must_stay_inside_the_output_folder() {
        for bad in [
            "",
            "/etc/passwd",
            "..\\x",
            "../x",
            "a/../x",
            "a//x",
            ".hidden",
            "a/.hidden",
            "C:/x",
            "a\\b",
            "tab\there",
        ] {
            assert!(check_output_path(bad).is_err(), "{bad:?}");
        }
        for good in ["a.png", "dir/a.png", "a b/c-d_e.txt"] {
            assert!(check_output_path(good).is_ok(), "{good:?}");
        }
        let wire = Produced::default().file("../x", vec![1]).encode();
        assert!(Produced::decode(&wire).is_err());
        let twice = Produced::default().file("a", vec![1]).file("a", vec![2]);
        assert!(
            Produced::decode(&twice.encode())
                .unwrap_err()
                .contains("twice")
        );
    }

    #[test]
    fn too_many_files_are_refused_before_any_are_read() {
        let mut made = Produced::default();
        for i in 0..=MAX_FILES {
            made = made.file(format!("f{i}"), Vec::new());
        }
        assert!(
            Produced::decode(&made.encode())
                .unwrap_err()
                .contains("more than")
        );
    }
}
