//! Reads the runtime's MJPEG preview sidecar and hands each frame over as a
//! `data:` URL, which a QML `Image` shows without any image provider.

use base64::Engine as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A running stream reader. Dropping it stops the thread at its next frame.
pub struct Watch {
    stop: Arc<AtomicBool>,
}

impl Watch {
    pub fn start(port: u16, on_frame: impl Fn(String) + Send + 'static) -> Watch {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = std::thread::Builder::new()
            .name("blockloom-preview".into())
            .spawn(move || {
                // The sidecar can take a moment to start listening after the
                // port is announced, so keep trying until told to stop.
                while !flag.load(Ordering::Relaxed) {
                    if let Err(error) = follow(port, &flag, &on_frame) {
                        tracing::debug!("preview stream on {port}: {error}");
                    }
                    std::thread::sleep(Duration::from_millis(400));
                }
            });
        Watch { stop }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn follow(port: u16, stop: &AtomicBool, on_frame: &impl Fn(String)) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"GET /preview.mjpg HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")?;
    let mut reader = BufReader::new(stream);
    skip_headers(&mut reader)?;
    let mut line = String::new();
    while !stop.load(Ordering::Relaxed) {
        // A read timeout only means a still world; keep waiting.
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock || error.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(error) => return Err(error),
        }
        if !line.trim_end().starts_with("--") {
            continue;
        }
        let length = part_length(&mut reader)?;
        let mut jpeg = vec![0; length];
        reader.read_exact(&mut jpeg)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        on_frame(format!("data:image/jpeg;base64,{encoded}"));
    }
    Ok(())
}

fn skip_headers(reader: &mut impl BufRead) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim_end().is_empty() {
            return Ok(());
        }
    }
}

/// Reads one part's headers and returns its Content-Length.
fn part_length(reader: &mut impl BufRead) -> std::io::Result<usize> {
    let mut length = 0;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return Ok(length);
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
}
