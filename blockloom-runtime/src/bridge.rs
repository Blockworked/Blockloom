//! Talking to the editor over this process's own pipes.
//!
//! stdin carries [`EditorMessage`]s, one JSON object per line, read on a
//! background thread so a slow editor never stalls a frame. stdout carries
//! [`RuntimeMessage`]s the same way. Nothing else may print to stdout - a
//! stray `println!` would corrupt the stream, so logs go to stderr.
//!
//! A built game has no editor on the other end: [`listen`] is never called,
//! [`attached`] is false, and the reports the editor would have shown are
//! dropped apart from the ones worth a line on stderr.

use blockloom_protocol::{EditorMessage, RuntimeMessage, decode, encode};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

static ATTACHED: AtomicBool = AtomicBool::new(false);

/// Whether an editor is on the other end of the pipes. False in a built game,
/// which is how the runtime knows not to report, and that nothing can press
/// Play for it.
pub fn attached() -> bool {
    ATTACHED.load(Ordering::Relaxed)
}

/// [`attached`] as a run condition, for the systems that only exist to tell
/// the editor things.
pub fn editor_attached() -> bool {
    attached()
}

/// Starts the reader thread and hands back the channel its lines arrive on.
/// The channel closes when the editor closes the pipe, which is the signal to
/// shut down.
pub fn listen() -> Receiver<EditorMessage> {
    ATTACHED.store(true, Ordering::Relaxed);
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("editor-stdin".to_string())
        .spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                match decode::<EditorMessage>(&line) {
                    Some(Ok(message)) => {
                        if tx.send(message).is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => eprintln!("blockloom-runtime: bad message: {e}"),
                    None => {}
                }
            }
        })
        .expect("failed to start the editor reader thread");
    rx
}

/// Sends one message to the editor. A closed pipe is ignored: the editor is
/// gone and the window is about to follow. With no editor at all, only the
/// messages a player might need to see reach stderr.
pub fn send(message: &RuntimeMessage) {
    if !attached() {
        match message {
            RuntimeMessage::Error { actor, message } => eprintln!("blockloom: {actor}: {message}"),
            RuntimeMessage::Fatal { message } => eprintln!("blockloom: {message}"),
            _ => {}
        }
        return;
    }
    let line = encode(message);
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = stdout.write_all(line.as_bytes());
    let _ = stdout.flush();
}
