//! The agent shell: hosts a real backend and reads one shell command per line
//! from stdin, writing one JSON response per line to stdout.
//!
//! Every response is `{"ok", "result", "error", "state"}`, where `state` is
//! the same full snapshot the editor window hands the frontend after each
//! edit - so an agent can create and edit a project exactly as a user can,
//! and read the world straight back after every step. `help` in the shell
//! documents every command.
//!
//! ```text
//! blockloom-shell                   # an interactive REPL
//! blockloom-shell --eval <command>  # run one line and exit
//! blockloom-shell < script.txt      # run a file of commands
//! blockloom-shell --no-state        # skip the state snapshot in responses
//! ```
//!
//! Each line is one command; blank lines and `#` comments are skipped, and
//! `exit`/`quit` end the shell. By default the shell owns its own copy of
//! whatever project it opens: against an editor holding the same folder it
//! attaches to the live owner's files (and says so), takes the lock over
//! explicitly with `take-over-lock`, or - with `--attach` - drives the
//! editor's own backend over the attach socket, so there is ever one copy.

use blockloom_app::attach::AttachClient;
use blockloom_app::shell::{Action, parse, run, run_forwarded};
use blockloom_app::{AppHandle, Backend};
use serde_json::Value;
use std::io::{BufRead, IsTerminal, Write};
use std::time::Duration;

fn usage() -> &'static str {
    concat!(
        "blockloom-shell - drive a Blockloom backend from a shell\n\n",
        "Usage: blockloom-shell [--eval <line>] [--no-state] [--attach] [--watch <dir>] [--specs]\n\n",
        "  --eval <line>   Run one command line and exit.\n",
        "  --no-state      Leave the state snapshot out of responses.\n",
        "  --attach        Drive the editor's own backend over its attach\n",
        "                  socket instead of booting a second copy. Needs the\n",
        "                  editor running.\n",
        "  --watch <dir>   Print {\"ok\",\"path\",\"revision\"} whenever the project\n",
        "                  folder's revision changes, until killed. Polls the\n",
        "                  revision file; combine with --poll-ms.\n",
        "  --poll-ms <n>   Watch poll interval in milliseconds (default 1000).\n",
        "  --specs         Print the whole command registry as JSON and exit.\n",
        "  --help          Show this help.\n"
    )
}

fn main() {
    // The shell's own chatter is noise on the agent's channel, which is stdout.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let mut eval: Option<String> = None;
    let mut with_state = true;
    let mut attach = false;
    let mut watch: Option<String> = None;
    let mut poll_ms: u64 = 1000;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--eval" => match args.next() {
                Some(line) => eval = Some(line),
                None => {
                    eprintln!("--eval needs a command line");
                    std::process::exit(2);
                }
            },
            "--no-state" => with_state = false,
            "--attach" => attach = true,
            "--watch" => match args.next() {
                Some(dir) => watch = Some(dir),
                None => {
                    eprintln!("--watch needs a project folder");
                    std::process::exit(2);
                }
            },
            "--poll-ms" => match args.next().and_then(|n| n.parse().ok()) {
                Some(ms) => poll_ms = ms,
                None => {
                    eprintln!("--poll-ms needs a number of milliseconds");
                    std::process::exit(2);
                }
            },
            "--specs" => {
                println!("{}", blockloom_app::shell::specs_json());
                return;
            }
            "--help" | "-h" => {
                println!("{}", usage());
                return;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!("{}", usage());
                std::process::exit(2);
            }
        }
    }

    if let Some(dir) = watch {
        if eval.is_some() || attach {
            eprintln!("--watch runs on its own; drop --eval/--attach with it");
            std::process::exit(2);
        }
        std::process::exit(watch_revisions(&dir, poll_ms));
    }

    if attach {
        let mut client = match AttachClient::connect() {
            Ok(client) => client,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        if let Some(line) = eval {
            let response = run_forwarded(
                &mut |cmd, args| client.roundtrip(cmd, args),
                &line,
                with_state,
            );
            let ok = response["ok"] == Value::Bool(true);
            print_response(&response, false);
            std::process::exit(if ok { 0 } else { 1 });
        }

        let had_error = repl_loop(&mut |line| {
            run_forwarded(
                &mut |cmd, args| client.roundtrip(cmd, args),
                line,
                with_state,
            )
        });
        std::process::exit(if had_error { 1 } else { 0 });
    }

    let backend = Backend::start(AppHandle::new(|_| {}));

    if let Some(line) = eval {
        let response = run(&backend, &line, with_state);
        let ok = response["ok"] == Value::Bool(true);
        print_response(&response, false);
        backend.shutdown();
        std::process::exit(if ok { 0 } else { 1 });
    }

    let had_error = repl_loop(&mut |line| run(&backend, line, with_state));
    backend.shutdown();
    // A piped script whose last command failed leaves a nonzero status behind.
    std::process::exit(if had_error { 1 } else { 0 });
}

/// The interactive loop both transports share: one JSON response per line,
/// `exit`/`quit` to leave. Answers whether any command failed.
fn repl_loop(run_one: &mut dyn FnMut(&str) -> Value) -> bool {
    let interactive = std::io::stdin().is_terminal();
    let stdin = std::io::stdin();

    if interactive {
        let banner = concat!(
            "Blockloom shell. Type \"help\" for every command, or \"exit\" to leave.\n",
            "Each response includes the full state snapshot afterwards.\n"
        );
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(banner.as_bytes());
        let _ = stdout.flush();
    }

    let mut had_error = false;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if matches!(parse(&line), Ok(Some(Action::Exit))) {
            break;
        }
        let response = run_one(&line);
        if response["ok"] != Value::Bool(true) {
            had_error = true;
        }
        print_response(&response, interactive);
    }
    had_error
}

/// Streams a project folder's revision counter until killed: one JSON object
/// per change, so an agent polls cheaply instead of re-reading the document.
/// Returns the process exit code (never, unless the folder is unusable).
fn watch_revisions(dir: &str, poll_ms: u64) -> i32 {
    use std::path::PathBuf;
    let dir = PathBuf::from(dir);
    if !blockloom_core::project::is_project_dir(&dir) {
        eprintln!("{} isn't a Blockloom project folder", dir.display());
        return 2;
    }
    let mut last: Option<u64> = None;
    loop {
        let revision = blockloom_core::sync::read_revision(&dir);
        if last != Some(revision) {
            last = Some(revision);
            let line = serde_json::json!({
                "ok": true,
                "path": dir.to_string_lossy(),
                "revision": revision,
            });
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(line.to_string().as_bytes());
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
        std::thread::sleep(Duration::from_millis(poll_ms.max(50)));
    }
}

/// One JSON response, indented for a human terminal and one compact line for
/// a script - an agent slurping stdout wants no prose between records.
fn print_response(response: &Value, pretty: bool) {
    let json = if pretty {
        serde_json::to_string_pretty(response)
    } else {
        serde_json::to_string(response)
    };
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(json.unwrap_or_default().as_bytes());
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
}
