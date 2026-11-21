//! The command line the Hub window and `scripts/hub.py` call.
//!
//! JSON on stdout, progress and errors on stderr (`hub: <message>`), exit 1
//! for an error and 130 for a cancellation, as the Python service did.

use crate::{Error, Hub, Result};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The commands this tool owns.
pub const COMMANDS: &[&str] = &[
    "projects",
    "installations",
    "remember",
    "forget",
    "relocate",
    "bind",
    "backups",
    "backup-project",
    "restore-backup",
    "focus",
    "open",
    "new-project",
    "default-installation",
    "installation-usage",
];

struct Args {
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(raw: &[String], valued: &[&str]) -> Result<Self> {
        let mut positional = vec![];
        let mut flags = vec![];
        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            if let Some(name) = arg.strip_prefix("--") {
                if valued.contains(&name) {
                    let value = iter
                        .next()
                        .ok_or_else(|| Error::new(format!("--{name} needs a value")))?;
                    flags.push((name.to_string(), Some(value.clone())));
                } else {
                    flags.push((name.to_string(), None));
                }
            } else {
                positional.push(arg.clone());
            }
        }
        Ok(Self { positional, flags })
    }

    fn at(&self, index: usize, what: &str) -> Result<&str> {
        self.positional
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| Error::new(format!("Missing argument: {what}")))
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }
}

/// Runs one command against `hub`.
pub fn dispatch(hub: &Hub, command: &str, raw: &[String]) -> Result<Value> {
    let args = Args::parse(raw, &["installation", "location", "mode", "sample"])?;
    let project = |i| -> Result<PathBuf> { Ok(PathBuf::from(args.at(i, "project")?)) };
    Ok(match command {
        "projects" => json!(hub.projects()?),
        "installations" => json!(hub.installations()?),
        "remember" => hub.remember(&project(0)?)?,
        "forget" => hub.forget(&project(0)?)?,
        "relocate" => hub.relocate(&project(0)?, &PathBuf::from(args.at(1, "folder")?))?,
        "bind" => hub.bind(
            &project(0)?,
            args.at(1, "installation")?,
            args.has("backup"),
        )?,
        "backups" => json!(hub.backups(&project(0)?)?),
        "backup-project" => hub.backup_project(&project(0)?)?,
        "restore-backup" => hub.restore_backup(&project(0)?, args.at(1, "backup name")?)?,
        "focus" => hub.focus(&project(0)?)?,
        "open" => hub.launch(&project(0)?)?,
        "new-project" => hub.new_project(
            args.at(0, "name")?,
            args.value("installation"),
            args.value("location"),
            args.value("mode").unwrap_or("TwoD"),
            args.value("sample"),
        )?,
        "default-installation" => json!({"installation": hub.default_installation()?}),
        "installation-usage" => hub.installation_usage(args.at(0, "installation")?)?,
        other => return Err(Error::new(format!("Unknown command: {other}"))),
    })
}

/// Parses `[--root DIR] <command> args...`, runs it and returns the exit
/// code.
pub fn run(argv: &[String]) -> i32 {
    let mut rest = argv;
    // The Hub executable is told apart from the window by this flag.
    if rest.first().map(String::as_str) == Some("--hubctl") {
        rest = &rest[1..];
    }
    let mut root = None;
    if rest.first().map(String::as_str) == Some("--root") && rest.len() >= 2 {
        root = Some(PathBuf::from(&rest[1]));
        rest = &rest[2..];
    }
    let Some((command, tail)) = rest.split_first() else {
        eprintln!("hub: Missing command");
        return 2;
    };
    let hub = Hub::new(root, None);
    match dispatch(&hub, command, tail) {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
            0
        }
        Err(error) => {
            eprintln!("hub: {error}");
            if error.cancelled { 130 } else { 1 }
        }
    }
}
