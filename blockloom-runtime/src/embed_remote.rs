//! A world in a child process drawing into the editor's Game view. The child
//! keeps its own `FrameExchange` whose ring calls go over a Unix socket to the
//! editor's, which holds the real ring: the dma-bufs travel as file
//! descriptors (`SCM_RIGHTS`), so the view imports the child's images exactly
//! as it imports an in-process world's. What the view asks for (size,
//! modifiers, each present) comes back the other way.

use super::{FrameExchange, Offer, SharedImage, Viewport};
use blockloom_protocol::InterfaceLayout;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// What the view asked for, mirrored to a child world.
pub enum ViewEvent {
    Viewport(Viewport),
    Offer(u64, Vec<u64>),
    Presented,
}

pub type Watch = Box<dyn Fn(ViewEvent) + Send + Sync>;

#[derive(Serialize, Deserialize, Debug)]
enum FromWorld {
    /// A new ring; its images' descriptors ride along, one per stride.
    Install {
        width: u32,
        height: u32,
        modifier: u64,
        offsets: Vec<u32>,
        strides: Vec<u32>,
    },
    Claim {
        generation: u64,
    },
    Finished {
        generation: u64,
        index: usize,
        layout: Option<InterfaceLayout>,
    },
    Clear,
    /// A read-back frame; the pixels are the message's bytes.
    Shm {
        width: u32,
        height: u32,
    },
}

#[derive(Serialize, Deserialize, Debug)]
enum FromView {
    Installed { generation: u64 },
    Claimed { index: Option<usize> },
    Viewport { width: u32, height: u32, scale: f32 },
    Offer { version: u64, modifiers: Vec<u64> },
    Presented,
}

/// The child's end: forwards ring calls and waits for the two that answer.
pub struct Link {
    writer: Mutex<UnixStream>,
    /// One request answered at a time, in order.
    replies: Mutex<Receiver<FromView>>,
}

impl Link {
    fn ask(&self, message: &FromWorld, fds: &[RawFd]) -> Option<FromView> {
        let replies = self.replies.lock().ok()?;
        self.send(message, fds, &[]);
        replies.recv().ok()
    }

    fn send(&self, message: &FromWorld, fds: &[RawFd], bytes: &[u8]) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = write_frame(&mut writer, message, fds, bytes);
        }
    }

    pub(super) fn install(
        &self,
        width: u32,
        height: u32,
        modifier: u64,
        images: Vec<SharedImage>,
    ) -> u64 {
        let message = FromWorld::Install {
            width,
            height,
            modifier,
            offsets: images.iter().map(|i| i.offset).collect(),
            strides: images.iter().map(|i| i.stride).collect(),
        };
        let fds: Vec<RawFd> = images.iter().map(|i| i.fd.as_raw_fd()).collect();
        // The editor has its own copies once this returns.
        match self.ask(&message, &fds) {
            Some(FromView::Installed { generation }) => generation,
            _ => 0,
        }
    }

    pub(super) fn claim(&self, generation: u64) -> Option<usize> {
        match self.ask(&FromWorld::Claim { generation }, &[])? {
            FromView::Claimed { index } => index,
            _ => None,
        }
    }

    pub(super) fn finished(&self, generation: u64, index: usize, layout: Option<InterfaceLayout>) {
        self.send(
            &FromWorld::Finished {
                generation,
                index,
                layout,
            },
            &[],
            &[],
        );
    }

    pub(super) fn clear(&self) {
        self.send(&FromWorld::Clear, &[], &[]);
    }

    pub(super) fn shm(&self, width: u32, height: u32, pixels: Vec<u8>) {
        self.send(&FromWorld::Shm { width, height }, &[], &pixels);
    }
}

/// A child world's exchange over `socket`, the editor's end of which is
/// served by [`serve`]. The world paces on the presents it hears.
pub fn connect(socket: UnixStream) -> std::io::Result<Arc<FrameExchange>> {
    let frames = FrameExchange::new(|| {});
    let (answer, replies) = channel();
    let link = Arc::new(Link {
        writer: Mutex::new(socket.try_clone()?),
        replies: Mutex::new(replies),
    });
    let _ = frames.remote.set(link);
    let mirror = frames.clone();
    std::thread::Builder::new()
        .name("frames-from-view".into())
        .spawn(move || read_view(socket, &mirror, &answer))?;
    Ok(frames)
}

fn read_view(mut socket: UnixStream, frames: &FrameExchange, answer: &Sender<FromView>) {
    while let Ok((message, _, _)) = read_frame::<FromView>(&mut socket) {
        match message {
            FromView::Viewport {
                width,
                height,
                scale,
            } => frames.resize(width, height, scale),
            FromView::Offer { version, modifiers } => {
                if let Ok(mut offer) = frames.offer.lock() {
                    *offer = Offer { version, modifiers };
                }
            }
            FromView::Presented => frames.presented(),
            reply => {
                let _ = answer.send(reply);
            }
        }
    }
    // The editor went away: stop waiting on presents that won't come.
    frames.presented();
}

static NEXT_WATCH: AtomicU64 = AtomicU64::new(1);

/// The editor's end: carries out a child world's ring calls on `frames`
/// and mirrors the view's asks to it, until the child hangs up.
pub fn serve(frames: Arc<FrameExchange>, socket: UnixStream) -> std::io::Result<()> {
    let writer = Arc::new(Mutex::new(socket.try_clone()?));
    let id = NEXT_WATCH.fetch_add(1, Ordering::Relaxed);
    let tell = {
        let writer = writer.clone();
        move |message: &FromView| {
            if let Ok(mut writer) = writer.lock() {
                let _ = write_frame(&mut writer, message, &[], &[]);
            }
        }
    };
    let wanted = frames.wanted();
    tell(&FromView::Viewport {
        width: wanted.size.x,
        height: wanted.size.y,
        scale: wanted.scale,
    });
    let offer = frames.offer();
    tell(&FromView::Offer {
        version: offer.version,
        modifiers: offer.modifiers,
    });
    if let Ok(mut watch) = frames.watch.lock() {
        let tell = tell.clone();
        *watch = Some((
            id,
            Box::new(move |event| {
                tell(&match event {
                    ViewEvent::Viewport(viewport) => FromView::Viewport {
                        width: viewport.size.x,
                        height: viewport.size.y,
                        scale: viewport.scale,
                    },
                    ViewEvent::Offer(version, modifiers) => FromView::Offer { version, modifiers },
                    ViewEvent::Presented => FromView::Presented,
                })
            }),
        ));
    }
    std::thread::Builder::new()
        .name("frames-from-world".into())
        .spawn(move || {
            let mut socket = socket;
            while let Ok((message, fds, bytes)) = read_frame::<FromWorld>(&mut socket) {
                match message {
                    FromWorld::Install {
                        width,
                        height,
                        modifier,
                        offsets,
                        strides,
                    } => {
                        let images = fds
                            .into_iter()
                            .zip(offsets.into_iter().zip(strides))
                            .map(|(fd, (offset, stride))| SharedImage { fd, offset, stride })
                            .collect();
                        let generation = frames.install(width, height, modifier, images);
                        tell(&FromView::Installed { generation });
                    }
                    FromWorld::Claim { generation } => {
                        tell(&FromView::Claimed {
                            index: frames.claim(generation),
                        });
                    }
                    FromWorld::Finished {
                        generation,
                        index,
                        layout,
                    } => frames.finished_with_layout(generation, index, layout),
                    FromWorld::Clear => frames.clear(),
                    FromWorld::Shm { width, height } => frames.publish_shm(width, height, bytes),
                }
            }
            if let Ok(mut watch) = frames.watch.lock()
                && watch.as_ref().is_some_and(|(owner, _)| *owner == id)
            {
                *watch = None;
            }
            frames.clear();
        })?;
    Ok(())
}

// ─── Framing ────────────────────────────────────────────────────────────────
// A frame is two little-endian lengths, the JSON message and its raw bytes.
// Descriptors are attached to the header's bytes.

const MAX_FDS: usize = 8;

fn write_frame(
    socket: &mut UnixStream,
    message: &impl Serialize,
    fds: &[RawFd],
    bytes: &[u8],
) -> std::io::Result<()> {
    let json = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&(json.len() as u32).to_le_bytes());
    header[4..].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    send_with_fds(socket, &header, fds)?;
    socket.write_all(&json)?;
    socket.write_all(bytes)?;
    socket.flush()
}

fn read_frame<T: for<'de> Deserialize<'de>>(
    socket: &mut UnixStream,
) -> std::io::Result<(T, Vec<OwnedFd>, Vec<u8>)> {
    let mut header = [0u8; 8];
    let fds = recv_with_fds(socket, &mut header)?;
    let json_len = u32::from_le_bytes(header[..4].try_into().unwrap_or_default()) as usize;
    let bytes_len = u32::from_le_bytes(header[4..].try_into().unwrap_or_default()) as usize;
    let mut json = vec![0; json_len];
    socket.read_exact(&mut json)?;
    let mut bytes = vec![0; bytes_len];
    socket.read_exact(&mut bytes)?;
    let message = serde_json::from_slice(&json).map_err(std::io::Error::other)?;
    Ok((message, fds, bytes))
}

fn cmsg_space() -> usize {
    // SAFETY: CMSG_SPACE is pure arithmetic.
    unsafe { libc::CMSG_SPACE((MAX_FDS * size_of::<RawFd>()) as u32) as usize }
}

fn send_with_fds(socket: &UnixStream, header: &[u8; 8], fds: &[RawFd]) -> std::io::Result<()> {
    if fds.len() > MAX_FDS {
        return Err(std::io::Error::other("too many descriptors"));
    }
    let mut iov = libc::iovec {
        iov_base: header.as_ptr() as *mut libc::c_void,
        iov_len: header.len(),
    };
    let mut control = vec![0u8; cmsg_space()];
    // SAFETY: an all-zero msghdr is valid; every pointer set below outlives
    // the sendmsg call, and the control buffer is sized by CMSG_SPACE.
    let sent = unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        if !fds.is_empty() {
            let size = std::mem::size_of_val(fds);
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = libc::CMSG_SPACE(size as u32) as _;
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(size as u32) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(cmsg).cast(), fds.len());
        }
        libc::sendmsg(socket.as_raw_fd(), &msg, libc::MSG_NOSIGNAL)
    };
    match sent {
        n if n < 0 => Err(std::io::Error::last_os_error()),
        n if n as usize == header.len() => Ok(()),
        // The descriptors went with the first byte; the rest is plain.
        n => {
            let mut rest = socket;
            rest.write_all(&header[n as usize..])
        }
    }
}

fn recv_with_fds(socket: &UnixStream, header: &mut [u8; 8]) -> std::io::Result<Vec<OwnedFd>> {
    let mut iov = libc::iovec {
        iov_base: header.as_mut_ptr().cast(),
        iov_len: header.len(),
    };
    let mut control = vec![0u8; cmsg_space()];
    let mut fds = Vec::new();
    // SAFETY: as in `send_with_fds`; each descriptor the kernel hands over is
    // new to this process, so owning it is sound.
    let received = unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = control.len() as _;
        let n = libc::recvmsg(socket.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
        if n > 0 {
            let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
            while !cmsg.is_null() {
                if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
                    let count = ((*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize)
                        / size_of::<RawFd>();
                    for i in 0..count {
                        fds.push(OwnedFd::from_raw_fd(data.add(i).read_unaligned()));
                    }
                }
                cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
            }
        }
        n
    };
    match received {
        n if n < 0 => Err(std::io::Error::last_os_error()),
        0 => Err(std::io::ErrorKind::UnexpectedEof.into()),
        n => {
            let mut rest = socket;
            rest.read_exact(&mut header[n as usize..])?;
            Ok(fds)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_cross_with_their_frame() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let file = std::fs::File::open("/dev/null").unwrap();
        write_frame(
            &mut a,
            &FromWorld::Shm {
                width: 1,
                height: 1,
            },
            &[file.as_raw_fd()],
            &[1, 2, 3, 4],
        )
        .unwrap();
        let (message, fds, bytes) = read_frame::<FromWorld>(&mut b).unwrap();
        assert!(matches!(
            message,
            FromWorld::Shm {
                width: 1,
                height: 1
            }
        ));
        assert_eq!(fds.len(), 1);
        assert_ne!(fds[0].as_raw_fd(), file.as_raw_fd());
        assert_eq!(bytes, [1, 2, 3, 4]);
    }

    #[test]
    fn a_child_ring_lives_in_the_editors_exchange() {
        let (editor_end, child_end) = UnixStream::pair().unwrap();
        let editor = FrameExchange::new(|| {});
        editor.resize(640, 480, 2.0);
        serve(editor.clone(), editor_end).unwrap();
        let child = connect(child_end).unwrap();
        let images = (0..3)
            .map(|_| SharedImage {
                fd: std::fs::File::open("/dev/null").unwrap().into(),
                offset: 0,
                stride: 2560,
            })
            .collect();
        let generation = child.install(640, 480, 7, images);
        assert_eq!(editor.slots().unwrap().generation, generation);
        assert_eq!(editor.slots().unwrap().images.len(), 3);
        let index = child.claim(generation).unwrap();
        child.finished_with_layout(generation, index, None);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while editor.latest() != Some((generation, index)) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(child.wanted().size.x, 640);
        let seen = child.presented_count();
        editor.presented();
        assert!(child.wait_presented(seen, std::time::Duration::from_secs(5)) > seen);
        drop(child);
    }
}
