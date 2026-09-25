//! The Game view's Rust half: runs the game world on a thread of this
//! process, and answers the C++ item (`game_view.cpp`) about which frame to
//! show. On platforms without GPU frame sharing yet, the world stays a child
//! process and the view never has a frame.

#[cxx::bridge]
mod ffi {
    /// The ring's images, with fds the caller now owns.
    struct GameFrames {
        generation: u64,
        width: u32,
        height: u32,
        fourcc: u32,
        modifier: u64,
        fds: Vec<i32>,
        offsets: Vec<u32>,
        strides: Vec<u32>,
    }

    struct GameFrame {
        valid: bool,
        generation: u64,
        index: usize,
    }

    extern "Rust" {
        /// Fills `out` and answers true when the ring isn't `known` any more.
        /// Generation 0 means there is no world to show.
        fn game_view_slots(known: u64, out: &mut GameFrames) -> bool;
        fn game_view_latest() -> GameFrame;
        fn game_view_hold(generation: u64, index: usize);
        /// The window put a frame on screen. Any thread.
        fn game_view_presented();
        /// The tiled `XBGR8888` modifiers the viewer samples as a plain texture.
        fn game_view_accept(modifiers: &[u64]);
        /// Ring `generation` couldn't be imported after all.
        fn game_view_refuse(generation: u64);
    }

    unsafe extern "C++" {
        include!("game_view.h");

        fn game_view_wake();
        fn game_view_prefer_opengl();
    }
}

use ffi::{GameFrame, GameFrames};

/// Puts Qt Quick on OpenGL, through EGL, before any window exists.
pub fn prefer_opengl() {
    #[cfg(target_os = "linux")]
    {
        // X11 defaults to GLX, which can't import a dma-buf.
        if std::env::var_os("QT_XCB_GL_INTEGRATION").is_none() {
            // SAFETY: still single-threaded; nothing else reads the environment yet.
            unsafe { std::env::set_var("QT_XCB_GL_INTEGRATION", "xcb_egl") };
        }
        ffi::game_view_prefer_opengl();
    }
}

#[cfg(target_os = "linux")]
mod embedded {
    use super::ffi;
    use blockloom_app::EmbeddedRuntime;
    use blockloom_core::scene::Mode;
    use blockloom_protocol::{EditorMessage, RuntimeMessage};
    use blockloom_runtime::embed::{self, Embedded, FrameExchange};
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{Arc, LazyLock};
    use std::thread::JoinHandle;

    /// Outlives every world, so the view keeps one source across restarts.
    pub static FRAMES: LazyLock<Arc<FrameExchange>> =
        LazyLock::new(|| FrameExchange::new(ffi::game_view_wake));

    /// Runs each world on its own thread of this process.
    pub struct Host;

    impl EmbeddedRuntime for Host {
        fn start(
            &self,
            mode: Mode,
            incoming: Receiver<EditorMessage>,
            outgoing: Sender<RuntimeMessage>,
        ) -> Result<Box<dyn Send>, String> {
            let frames = FRAMES.clone();
            let crashed = outgoing.clone();
            let thread = std::thread::Builder::new()
                .name("blockloom-world".to_string())
                .spawn(move || {
                    let run = std::panic::AssertUnwindSafe(|| {
                        embed::run(Embedded {
                            mode,
                            incoming,
                            outgoing,
                            frames,
                        })
                    });
                    // A panicking world ends the run, not the editor.
                    if let Err(panic) = std::panic::catch_unwind(run) {
                        let reason = panic
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| panic.downcast_ref::<&str>().copied())
                            .unwrap_or("unknown panic");
                        let _ = crashed.send(RuntimeMessage::Fatal {
                            message: format!("The game world crashed: {reason}"),
                        });
                    }
                })
                .map_err(|e| format!("Couldn't start the game world: {e}"))?;
            Ok(Box::new(World(Some(thread))))
        }
    }

    /// Waits for the world's thread when dropped.
    struct World(Option<JoinHandle<()>>);

    impl Drop for World {
        fn drop(&mut self) {
            if let Some(thread) = self.0.take() {
                let _ = thread.join();
            }
        }
    }
}

/// The in-process host, unless `BLOCKLOOM_RUNTIME=process` asks for the
/// child process (and its MJPEG preview) instead.
pub fn host() -> Option<std::sync::Arc<dyn blockloom_app::EmbeddedRuntime>> {
    #[cfg(target_os = "linux")]
    if std::env::var("BLOCKLOOM_RUNTIME").as_deref() != Ok("process") {
        return Some(std::sync::Arc::new(embedded::Host));
    }
    None
}

fn game_view_slots(known: u64, out: &mut GameFrames) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsFd, IntoRawFd};
        let set = embedded::FRAMES.slots();
        let generation = set.as_ref().map_or(0, |set| set.generation);
        if generation == known {
            return false;
        }
        out.generation = generation;
        if let Some(set) = set {
            out.width = set.width;
            out.height = set.height;
            out.fourcc = set.fourcc;
            out.modifier = set.modifier;
            for image in &set.images {
                // A copy per import: the ring keeps its own until replaced.
                let Ok(fd) = image.fd.as_fd().try_clone_to_owned() else {
                    for fd in out.fds.drain(..) {
                        // SAFETY: each was just duplicated for this call.
                        drop(unsafe {
                            <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd)
                        });
                    }
                    out.generation = known;
                    return false;
                };
                out.fds.push(fd.into_raw_fd());
                out.offsets.push(image.offset);
                out.strides.push(image.stride);
            }
        }
        true
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (known, out);
        false
    }
}

fn game_view_latest() -> GameFrame {
    #[cfg(target_os = "linux")]
    if let Some((generation, index)) = embedded::FRAMES.latest() {
        return GameFrame {
            valid: true,
            generation,
            index,
        };
    }
    GameFrame {
        valid: false,
        generation: 0,
        index: 0,
    }
}

fn game_view_hold(generation: u64, index: usize) {
    #[cfg(target_os = "linux")]
    embedded::FRAMES.hold(generation, index);
    #[cfg(not(target_os = "linux"))]
    let _ = (generation, index);
}

fn game_view_presented() {
    #[cfg(target_os = "linux")]
    embedded::FRAMES.presented();
}

fn game_view_accept(modifiers: &[u64]) {
    #[cfg(target_os = "linux")]
    embedded::FRAMES.accept(modifiers.iter().copied());
    #[cfg(not(target_os = "linux"))]
    let _ = modifiers;
}

fn game_view_refuse(generation: u64) {
    #[cfg(target_os = "linux")]
    embedded::FRAMES.refuse(generation);
    #[cfg(not(target_os = "linux"))]
    let _ = generation;
}
