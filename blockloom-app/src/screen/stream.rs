//! The transport every device has: one `screencap` loop running on the
//! device for the whole session, its PNGs read off one adb pipe. No process
//! is spawned per frame and nothing is polled; a frame is shown as soon as
//! the device finishes it, and one the editor couldn't keep up with is
//! dropped for the newest.

use super::{Context, Event, Frame, frames};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The newest unshown PNG, handed from the pipe's reader to the decoder.
#[derive(Default)]
struct Slot {
    png: Option<Vec<u8>>,
    closed: Option<String>,
}

/// Streams until the session stops (Ok) or the device stops answering.
pub(super) fn run(ctx: &Context) -> Result<(), String> {
    let mut child = Command::new(&ctx.adb)
        .args(["-s", &ctx.serial, "exec-out"])
        // The loop sleeps on failure so a locked or asleep display doesn't
        // spin the device.
        .arg("while :; do screencap -p || sleep 1; done")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't run {}: {e}", ctx.adb.display()))?;
    let stdout = child.stdout.take().ok_or("Couldn't open adb's output")?;
    let slot = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
    let reader = slot.clone();
    std::thread::Builder::new()
        .name("blockloom-screen-read".into())
        .spawn(move || {
            let mut pngs = frames::PngStream::new(stdout);
            loop {
                let next = pngs.next_png();
                let (lock, wake) = &*reader;
                let Ok(mut held) = lock.lock() else { return };
                match next {
                    Ok(Some(png)) => held.png = Some(png),
                    Ok(None) => {
                        held.closed = Some("The device stopped sending its screen.".to_string());
                    }
                    Err(e) => held.closed = Some(format!("Lost the device's screen: {e}")),
                }
                let done = held.closed.is_some();
                wake.notify_one();
                if done {
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let result = pump(ctx, &slot);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn pump(ctx: &Context, slot: &(Mutex<Slot>, Condvar)) -> Result<(), String> {
    let (lock, wake) = slot;
    let mut seq = 0u64;
    let mut last_hash = 0u64;
    while !ctx.stopped() {
        let png = {
            let mut held = lock.lock().map_err(|e| e.to_string())?;
            if held.png.is_none() && held.closed.is_none() {
                held = wake
                    .wait_timeout(held, Duration::from_millis(100))
                    .map_err(|e| e.to_string())?
                    .0;
            }
            match held.png.take() {
                Some(png) => png,
                None => match &held.closed {
                    Some(why) => return Err(why.clone()),
                    None => continue,
                },
            }
        };
        let Some((device_width, device_height)) = frames::png_size(&png) else {
            continue;
        };
        ctx.size.set(device_width, device_height);
        // A still screen sends the same bytes again; there is nothing to show.
        let mut hasher = DefaultHasher::new();
        png.hash(&mut hasher);
        let hash = hasher.finish();
        if hash == last_hash {
            continue;
        }
        last_hash = hash;
        let shot = match image::load_from_memory_with_format(&png, image::ImageFormat::Png) {
            Ok(shot) => shot,
            Err(error) => {
                tracing::debug!("{}: undecodable frame: {error}", ctx.serial);
                continue;
            }
        };
        let packed = frames::pack(&shot, ctx.max_width)?;
        seq += 1;
        (ctx.sink)(Event::Frame(Frame {
            serial: ctx.serial.clone(),
            image: packed.url,
            width: packed.width,
            height: packed.height,
            device_width,
            device_height,
            seq,
            transport: "adb",
        }));
    }
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::super::testing::{context, stub, temp};
    use super::*;
    use std::sync::atomic::Ordering;

    fn png(shade: u8) -> Vec<u8> {
        let shot = image::RgbaImage::from_pixel(32, 64, image::Rgba([shade, 30, 60, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(shot)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn a_device_screen_streams_as_frames_without_repeating_a_still_one() {
        let dir = temp("stream");
        let (first, second) = (dir.join("a.png"), dir.join("b.png"));
        std::fs::write(&first, png(10)).unwrap();
        std::fs::write(&second, png(220)).unwrap();
        // What a real device sends: the same screen twice, then a change, each
        // a moment apart (a frame the editor hasn't shown yet is dropped).
        let adb = stub(
            &dir,
            "adb",
            &format!(
                "#!/bin/sh\ncat '{a}'\nsleep 0.3\ncat '{a}'\nsleep 0.3\ncat '{b}'\nsleep 5\n",
                a = first.display(),
                b = second.display()
            ),
        );
        let (ctx, frames) = context(adb);
        let stop = ctx.stop.clone();
        let seen = frames.clone();
        std::thread::spawn(move || {
            for _ in 0..500 {
                if seen.lock().unwrap().len() >= 2 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            stop.store(true, Ordering::Relaxed);
        });
        run(&ctx).unwrap();
        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!((frames[0].width, frames[0].height), (8, 16));
        assert_eq!((frames[0].device_width, frames[0].device_height), (32, 64));
        assert_eq!((frames[0].seq, frames[1].seq), (1, 2));
        assert_eq!(frames[0].transport, "adb");
        assert_ne!(frames[0].image, frames[1].image);
        assert_eq!(ctx.size.get(), (32, 64));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_device_that_goes_quiet_ends_the_stream_with_a_reason() {
        let dir = temp("quiet");
        let adb = stub(&dir, "adb", "#!/bin/sh\nexit 0\n");
        let (ctx, frames) = context(adb);
        let error = run(&ctx).unwrap_err();
        assert!(error.contains("stopped sending"), "{error}");
        assert!(frames.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
