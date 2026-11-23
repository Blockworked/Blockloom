//! The command line the Hub window and `scripts/hub.py` call: projects and
//! editors, and installing, downloading and building editors.
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
    "tool-cache",
    "prune-tools",
    "uninstall",
    "add-tools",
    "add-dev",
    "modules",
    "dev-options",
    "check-releases",
    "release-settings",
    "install",
    "download-release",
    "rebuild",
    "prepare-dev",
    "install-dev",
];

/// The `--java`, `--android-sdk` and `--android-ndk` flags given.
fn components(args: &Args) -> Vec<String> {
    crate::installs::OPTIONAL_TOOLS
        .iter()
        .filter(|name| args.has(name))
        .map(|name| name.to_string())
        .collect()
}

fn jobs(args: &Args) -> Result<Option<u32>> {
    match args.value("jobs") {
        None => Ok(None),
        Some(text) => match text.parse::<u32>() {
            Ok(n) if n > 0 => Ok(Some(n)),
            _ => Err(Error::new("--jobs must be positive")),
        },
    }
}

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
    let args = Args::parse(
        raw,
        &[
            "installation",
            "location",
            "mode",
            "sample",
            "source",
            "url",
            "repo",
            "sha256",
            "jobs",
            "build-id",
        ],
    )?;
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
        "tool-cache" => hub.tool_cache(),
        "prune-tools" => hub.prune_tools()?,
        "uninstall" => hub.uninstall(args.at(0, "installation")?)?,
        "add-tools" => hub.add_tools(
            args.at(0, "installation")?,
            &components(&args),
            args.has("android-rust-targets"),
        )?,
        "add-dev" => hub.add_dev(&PathBuf::from(args.at(0, "repository")?))?,
        "modules" => hub.modules(args.positional.first().map(String::as_str))?,
        "dev-options" => hub.dev_options(args.at(0, "installation")?)?,
        "check-releases" => hub.check_releases(args.positional.first().map(String::as_str))?,
        "release-settings" => {
            hub.release_settings(args.value("source"), args.value("url"), args.value("repo"))?
        }
        "install" => hub.install(
            &PathBuf::from(args.at(0, "bundle")?),
            &components(&args),
            args.has("android-rust-targets"),
        )?,
        "download-release" => hub.download_release(
            args.at(0, "version")?,
            &components(&args),
            args.has("android-rust-targets"),
            args.value("sha256"),
        )?,
        "rebuild" => hub.rebuild(args.at(0, "installation")?, jobs(&args)?)?,
        "prepare-dev" => hub.prepare_dev(args.at(0, "installation")?, jobs(&args)?)?,
        "install-dev" => hub.install_dev(
            args.at(0, "installation")?,
            &components(&args),
            args.has("android-rust-targets"),
            args.value("build-id"),
        )?,
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
