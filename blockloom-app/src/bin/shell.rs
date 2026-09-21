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
//! `exit`/`quit` end the shell. The editor keeps one copy of whatever project
//! it has open in memory, so don't edit the same project from this shell and
//! the window at the same time.

use blockloom_app::shell::{Action, parse, run};
use blockloom_app::{AppHandle, Backend};
use serde_json::Value;
use std::io::{BufRead, IsTerminal, Write};

fn usage() -> &'static str {
    concat!(
        "blockloom-shell - drive a Blockloom backend from a shell\n\n",
        "Usage: blockloom-shell [--eval <line>] [--no-state] [--specs]\n\n",
        "  --eval <line>   Run one command line and exit.\n",
        "  --no-state      Leave the state snapshot out of responses.\n",
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

    let backend = Backend::start(AppHandle::new(|_| {}));

    if let Some(line) = eval {
        let response = run(&backend, &line, with_state);
        print_response(&response, false);
        std::process::exit(if response["ok"] == Value::Bool(true) {
            0
        } else {
            1
        });
    }

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
        let response = run(&backend, &line, with_state);
        if response["ok"] != Value::Bool(true) {
            had_error = true;
        }
        print_response(&response, interactive);
    }
    // A piped script whose last command failed leaves a nonzero status behind.
    std::process::exit(if had_error { 1 } else { 0 });
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
