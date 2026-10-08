// In-app completion and hover for the script editor: the same names
// rust-analyzer would offer, without leaving Blockloom.
//
// Entries come from two places: the script API (`super::PRELUDE_SOURCE`,
// scanned for functions, types and enum variants with their doc lines) and
// this project's `symbols` module (parsed back from the generated source, so
// the labels match what actually compiles). Hover answers the doc line for
// either, plus a short note for keywords.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// One row of the completion popup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Completion {
    /// What the list shows, e.g. `go_to` or `symbols::actors::BALL`.
    pub label: String,
    /// Where it comes from, e.g. `Actor method` or `Project symbol "Ball"`.
    pub detail: String,
    /// The text inserted on accept.
    pub insert: String,
}

struct ApiEntry {
    name: String,
    scope: String,
    kind: String,
    doc: String,
    signature: String,
}

/// Up to 60 completions whose label or last path segment starts with
/// `prefix` (case-insensitive). Empty prefix lists API entry points plus the
/// project's symbols: enough to discover, short enough to read.
pub fn completions_for(project_dir: &Path, prefix: &str) -> Vec<Completion> {
    let needle = prefix.trim().to_lowercase();
    let mut out: Vec<Completion> = Vec::new();
    for entry in api_entries() {
        if matches_prefix(&entry.name, &needle) {
            out.push(Completion {
                label: entry.name.clone(),
                detail: api_detail(&entry),
                insert: entry.name.clone(),
            });
        }
        // Enum variants complete as `Type::Variant`, the way they are typed.
        if !entry.scope.is_empty() && entry.kind == "variant" {
            let qualified = format!("{}::{}", entry.scope, entry.name);
            if matches_prefix(&qualified, &needle) || matches_prefix(&entry.name, &needle) {
                out.push(Completion {
                    label: qualified.clone(),
                    detail: format!("{} of {}", entry.scope, entry.doc),
                    insert: qualified,
                });
            }
        }
    }
    let (symbols_source, _) = super::symbols::project_symbols_for_dir(project_dir);
    for (path, original) in symbol_paths(&symbols_source) {
        let short = path.rsplit("::").next().unwrap_or(&path);
        if matches_prefix(short, &needle) || matches_prefix(&path, &needle) {
            out.push(Completion {
                label: path.clone(),
                detail: format!("Project symbol \"{original}\""),
                insert: path,
            });
        }
    }
    // Keywords only compete when something was typed: an empty prefix is for
    // discovering API and project names, not the whole language.
    if !needle.is_empty() {
        // Keywords last, so API and project names win the top rows.
        for keyword in KEYWORDS {
            if matches_prefix(keyword, &needle) {
                out.push(Completion {
                    label: keyword.to_string(),
                    detail: "Rust keyword".to_string(),
                    insert: keyword.to_string(),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        let rank = |c: &Completion| {
            (
                c.detail == "Rust keyword",
                c.label.starts_with("symbols::"),
                c.label.clone(),
            )
        };
        rank(a).cmp(&rank(b))
    });
    out.dedup_by(|a, b| a.insert == b.insert);
    out.truncate(60);
    out
}

/// One or two lines about `symbol`: an API method's doc and signature, a
/// project symbol's original name, or a keyword note. `None` means unknown.
pub fn hover_for(project_dir: &Path, symbol: &str) -> Option<String> {
    let symbol = symbol.trim().trim_end_matches("()").trim();
    if symbol.is_empty() {
        return None;
    }
    if let Some(path) = symbol.strip_prefix("symbols::") {
        let (source, _) = super::symbols::project_symbols_for_dir(project_dir);
        let short = path.rsplit("::").next().unwrap_or(path);
        for (full, original) in symbol_paths(&source) {
            let tail = full.strip_prefix("symbols::").unwrap_or(&full);
            if tail == path
                || full.rsplit("::").next().unwrap_or("") == short && tail.ends_with(path)
            {
                return Some(format!("Project symbol \"{original}\": `{full}`"));
            }
        }
        return None;
    }
    let short = symbol.rsplit("::").next().unwrap_or(symbol);
    let mut best: Option<&ApiEntry> = None;
    // Cached per call; the prelude scan is a few hundred lines.
    let entries = api_entries();
    for entry in &entries {
        if entry.name == short {
            // A bare `go_to` prefers the `Actor` method over `ActorRef`'s.
            if best.is_none_or(|b| b.scope == "ActorRef" && entry.scope == "Actor") {
                best = Some(entry);
            }
        }
    }
    best.map(|entry| {
        let mut text = api_detail(entry);
        if !entry.signature.is_empty() {
            text.push_str(&format!("\n`{}`", entry.signature));
        }
        text
    })
    .or_else(|| {
        KEYWORDS
            .contains(&short)
            .then(|| format!("`{short}` is a Rust keyword"))
    })
}

fn matches_prefix(label: &str, needle: &str) -> bool {
    needle.is_empty() || label.to_lowercase().starts_with(needle)
}

fn api_detail(entry: &ApiEntry) -> String {
    if entry.doc.is_empty() {
        format!("{} {}", entry.scope, entry.kind)
    } else {
        format!("{} {} - {}", entry.scope, entry.kind, entry.doc)
    }
}

/// Every function, const, type and enum variant in the script crate, with the
/// first doc line above it. A small line scanner, not a parser: the prelude
/// is generated source with one item per line, which is all this needs.
fn api_entries() -> Vec<ApiEntry> {
    let mut out = Vec::new();
    let mut scope = String::new();
    let mut doc: Vec<String> = Vec::new();
    for line in super::PRELUDE_SOURCE.lines() {
        let trimmed = line.trim();
        if let Some(comment) = trimmed.strip_prefix("///") {
            doc.push(comment.trim().to_string());
            continue;
        }
        if trimmed.is_empty() {
            doc.clear();
            continue;
        }
        if let Some(name) = header_scope(trimmed) {
            scope = name;
            doc.clear();
            continue;
        }
        if trimmed == "}" {
            doc.clear();
            continue;
        }
        if let Some(name) = fn_name(trimmed) {
            out.push(ApiEntry {
                name,
                scope: scope.clone(),
                kind: "method".to_string(),
                doc: doc.first().cloned().unwrap_or_default(),
                signature: trimmed.to_string(),
            });
            doc.clear();
            continue;
        }
        if let Some(name) = const_name(trimmed) {
            out.push(ApiEntry {
                name,
                scope: scope.clone(),
                kind: "const".to_string(),
                doc: doc.first().cloned().unwrap_or_default(),
                signature: trimmed.to_string(),
            });
            doc.clear();
            continue;
        }
        if !scope.is_empty()
            && let Some(name) = variant_name(trimmed)
        {
            out.push(ApiEntry {
                name,
                scope: scope.clone(),
                kind: "variant".to_string(),
                doc: doc.first().cloned().unwrap_or_default(),
                signature: String::new(),
            });
            doc.clear();
            continue;
        }
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        doc.clear();
    }
    out
}

/// `impl Actor`, `pub struct Foo`, `pub enum Bar`, `pub mod baz`: what the
/// following items belong to.
fn header_scope(trimmed: &str) -> Option<String> {
    for prefix in ["pub mod ", "pub enum ", "pub struct "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    if trimmed.starts_with("impl ") {
        let name: String = trimmed
            .strip_prefix("impl ")
            .unwrap_or("")
            .split_whitespace()
            .last()
            .unwrap_or("")
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn fn_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("pub fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn const_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("pub const ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// An enum variant line: an uppercase word with nothing but a trailing comma,
/// brace or paren on it.
fn variant_name(trimmed: &str) -> Option<String> {
    let word: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if word.is_empty() || !word.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }
    let rest = trimmed[word.len()..].trim_start();
    if rest.starts_with(',') || rest.starts_with('{') || rest.starts_with('(') || rest.is_empty() {
        Some(word)
    } else {
        None
    }
}

/// Full `symbols::...` paths from generated symbols source, with the original
/// project name each doc comment holds. Tracks `pub mod` nesting by indent:
// the generator writes one level per four spaces.
fn symbol_paths(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<String> = vec!["symbols".to_string()];
    let mut doc_name: Option<String> = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("///") {
            // Doc lines read `Actor `"Ball".`: the quoted name is original.
            if let Some(quoted) = trimmed.split('"').nth(1) {
                doc_name = Some(quoted.to_string());
            }
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if trimmed.starts_with("pub mod ") {
            let depth = indent / 4;
            stack.truncate(depth);
            let name: String = trimmed
                .strip_prefix("pub mod ")
                .unwrap_or("")
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                stack.push(name);
            }
            doc_name = None;
            continue;
        }
        if trimmed.starts_with("pub const ") {
            let name = const_name(trimmed).unwrap_or_default();
            if !name.is_empty() {
                let mut full = stack.join("::");
                full.push_str("::");
                full.push_str(&name);
                out.push((full, doc_name.clone().unwrap_or_default()));
            }
            doc_name = None;
            continue;
        }
        if trimmed == "}" {
            doc_name = None;
        }
    }
    out
}

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "blockloom-complete-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn api_methods_complete_by_prefix() {
        let dir = temp_dir("prefix");
        let found = completions_for(&dir, "go_t");
        assert!(
            found.iter().any(|c| c.insert == "go_to"),
            "go_to missing in {:?}",
            found.iter().map(|c| &c.label).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_symbols_complete_and_hover() {
        let dir = temp_dir("symbols");
        let mut project = crate::project::Project::starter("Complete", crate::scene::Mode::TwoD);
        // Name the starter actor so the symbols module has something to say.
        project.scenes[0].actors[0].name = "Ball".to_string();
        crate::project::save_project(&project, &dir).unwrap();
        let found = completions_for(&dir, "symbols::actors::b");
        assert!(
            found.iter().any(|c| c.insert == "symbols::actors::BALL"),
            "BALL missing in {:?}",
            found.iter().map(|c| &c.label).collect::<Vec<_>>()
        );
        let hovered = hover_for(&dir, "symbols::actors::BALL").expect("hover");
        assert!(hovered.contains("Ball"), "{hovered}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn method_hover_names_the_owner() {
        let dir = temp_dir("hover");
        let hovered = hover_for(&dir, "say").expect("hover");
        assert!(hovered.contains("Actor"), "{hovered}");
        assert!(hover_for(&dir, "no_such_thing_here").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_prefix_lists_both_worlds() {
        let dir = temp_dir("empty");
        let found = completions_for(&dir, "");
        assert!(!found.is_empty());
        assert!(found.len() <= 60);
        // Discovery still works for late-alphabet methods via a prefix, and
        // keywords join in once something is typed.
        let said = completions_for(&dir, "sa");
        assert!(said.iter().any(|c| c.insert == "say"));
        let keyed = completions_for(&dir, "fn");
        assert!(keyed.iter().any(|c| c.label == "fn"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
