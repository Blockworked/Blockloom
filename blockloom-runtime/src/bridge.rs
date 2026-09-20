//! Talking to the editor over this process's own pipes.
//!
//! stdin carries [`EditorMessage`]s, one JSON object per line, read on a
//! background thread so a slow editor never stalls a frame. stdout carries
//! [`RuntimeMessage`]s the same way. Nothing else may print to stdout - a
//! stray `println!` would corrupt the stream, so logs go to stderr.

use blockloom_protocol::{EditorMessage, RuntimeMessage, decode, encode};
use std::io::{BufRead, Write};
use std::sync::mpsc::{Receiver, channel};

/// Starts the reader thread and hands back the channel its lines arrive on.
/// The channel closes when the editor closes the pipe, which is the signal to
/// shut down.
pub fn listen() -> Receiver<EditorMessage> {
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
/// gone and the window is about to follow.
pub fn send(message: &RuntimeMessage) {
    let line = encode(message);
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = stdout.write_all(line.as_bytes());
    let _ = stdout.flush();
}
