//! The Problems panel: everything that stops Play or Build, or will surprise
//! somebody later, gathered from the checks that already exist.

use super::{lock, plugins};
use crate::state::SharedState;
use blockloom_core::pipeline;
use blockloom_plugin_host::imports::{self, ImportState};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct Problem {
    /// `plugins`, `scripts`, `imports` or `assets`.
    pub source: &'static str,
    /// `error` stops Play or Build, `warning` doesn't.
    pub severity: &'static str,
    pub message: String,
    /// A project file the problem is about, for "open" or "reimport".
    pub path: Option<String>,
    pub line: Option<u32>,
}

impl Problem {
    fn new(source: &'static str, severity: &'static str, message: String) -> Self {
        Self {
            source,
            severity,
            message,
            path: None,
            line: None,
        }
    }
    fn at(mut self, path: &str, line: Option<u32>) -> Self {
        self.path = Some(path.to_string());
        self.line = line;
        self
    }
}

/// `problems`: errors first, then warnings, each group in source order.
pub(crate) fn problems(state: &SharedState) -> Result<Vec<Problem>, String> {
    let s = lock(state)?;
    let (Some(open), Some(project)) = (s.open.as_ref(), s.project()) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<Problem> = plugins::preflight_lines(&open.plugins, project)
        .into_iter()
        .map(|line| Problem::new("plugins", "error", line))
        .collect();
    out.extend(
        open.plugins
            .audit(project.plugin_records())
            .iter()
            .filter(|i| !i.blocks_run)
            .map(|i| Problem::new("plugins", "warning", plugins::describe(i))),
    );
    out.extend(
        open.plugins
            .conflicts()
            .into_iter()
            .filter(|c| !c.blocks_run)
            .map(|c| Problem::new("plugins", "warning", c.message)),
    );
    for (path, status) in &open.script_statuses {
        if let Some(error) = &status.error {
            let what = if status.stage == "load_failed" {
                "didn't load"
            } else {
                "didn't build"
            };
            out.push(
                Problem::new(
                    "scripts",
                    "error",
                    format!("{path} {what}: {}", first_line(error)),
                )
                .at(path, line_in(error, path)),
            );
        }
    }
    for import in imports::status(&open.dir) {
        let why = match &import.state {
            ImportState::Fresh => continue,
            ImportState::SourceChanged => "changed since it was imported; reimport it".to_string(),
            ImportState::DependencyChanged { path } => {
                format!("reads {path}, which changed; reimport it")
            }
            ImportState::OutputMissing { path } => {
                format!("made {path}, which is gone; reimport it")
            }
            ImportState::OutputEdited { path } => format!("made {path}, which was edited by hand"),
            ImportState::SourceMissing => "is gone, but its import is still remembered".to_string(),
        };
        out.push(
            Problem::new("imports", "warning", format!("{} {why}", import.source))
                .at(&import.source, None),
        );
        for warning in import.warnings {
            out.push(
                Problem::new(
                    "imports",
                    "warning",
                    format!("{}: {warning}", import.source),
                )
                .at(&import.source, None),
            );
        }
    }
    for report in pipeline::scan_project(&open.dir) {
        for warning in &report.warnings {
            out.push(
                Problem::new("assets", "warning", format!("{}: {warning}", report.path))
                    .at(&report.path, None),
            );
        }
    }
    out.sort_by_key(|p| p.severity != "error");
    Ok(out)
}

fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(text)
}

/// The first `path:LINE` rustc points at in its output.
fn line_in(error: &str, path: &str) -> Option<u32> {
    let name = path.rsplit('/').next().unwrap_or(path);
    error.match_indices(name).find_map(|(at, _)| {
        let rest = error[at + name.len()..].strip_prefix(':')?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rustc_error_points_at_its_line() {
        let error = "error[E0425]: cannot find value `x`\n --> assets/scripts/ball.rs:12:5\n  |";
        assert_eq!(line_in(error, "assets/scripts/ball.rs"), Some(12));
        assert_eq!(line_in("no toolchain", "assets/scripts/ball.rs"), None);
        assert_eq!(first_line("\n  error: oops\nmore"), "error: oops");
    }
}
