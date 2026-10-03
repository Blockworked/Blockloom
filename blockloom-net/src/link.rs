use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Reproducible faults applied to outgoing datagrams.
#[derive(Clone, Copy, Debug, Default)]
pub struct Impairment {
    /// Chance in 0..=1 that a datagram is dropped.
    pub loss: f64,
    /// Chance in 0..=1 that a datagram is sent twice.
    pub duplicate: f64,
    /// Fixed one-way delay.
    pub delay: Duration,
    /// Extra random delay up to this much; it reorders datagrams.
    pub jitter: Duration,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LinkStats {
    pub sent: u64,
    pub dropped: u64,
    pub duplicated: u64,
    pub received: u64,
}

struct Queued {
    at: Instant,
    seq: u64,
    to: SocketAddr,
    data: Vec<u8>,
}

impl PartialEq for Queued {
    fn eq(&self, o: &Self) -> bool {
        (self.at, self.seq) == (o.at, o.seq)
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Queued {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.at, self.seq).cmp(&(o.at, o.seq))
    }
}

/// A nonblocking UDP socket that holds each datagram until Quiche's pacing
/// time (plus any injected delay) before sending it.
pub(crate) struct Link {
    socket: UdpSocket,
    imp: Impairment,
    rng: u64,
    queue: BinaryHeap<Reverse<Queued>>,
    seq: u64,
    pub stats: LinkStats,
}

impl Link {
    pub fn bind(addr: SocketAddr, imp: Impairment) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Link {
            socket,
            imp,
            rng: imp.seed ^ 0x9e37_79b9_7f4a_7c15,
            queue: BinaryHeap::new(),
            seq: 0,
            stats: LinkStats::default(),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn next_f64(&mut self) -> f64 {
        // splitmix64
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Queue a datagram to leave no earlier than `at`.
    pub fn send(&mut self, data: &[u8], to: SocketAddr, at: Instant) {
        self.stats.sent += 1;
        if self.next_f64() < self.imp.loss {
            self.stats.dropped += 1;
            return;
        }
        let copies = if self.next_f64() < self.imp.duplicate {
            self.stats.duplicated += 1;
            2
        } else {
            1
        };
        for _ in 0..copies {
            let jitter = self.imp.jitter.mul_f64(self.next_f64());
            self.seq += 1;
            self.queue.push(Reverse(Queued {
                at: at.max(Instant::now()) + self.imp.delay + jitter,
                seq: self.seq,
                to,
                data: data.to_vec(),
            }));
        }
        self.flush();
    }

    /// Put every datagram that is due on the wire.
    pub fn flush(&mut self) {
        let now = Instant::now();
        while let Some(Reverse(head)) = self.queue.peek() {
            if head.at > now {
                break;
            }
            let Reverse(q) = self.queue.pop().expect("peeked");
            // A full socket buffer or an unreachable peer drops it, like the network would.
            let _ = self.socket.send_to(&q.data, q.to);
        }
    }

    pub fn recv(&mut self, buf: &mut [u8]) -> Option<(usize, SocketAddr)> {
        loop {
            match self.socket.recv_from(buf) {
                Ok((n, from)) => {
                    self.stats.received += 1;
                    return Some((n, from));
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return None,
                // Windows reports a prior ICMP unreachable as a recv error.
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(_) => return None,
            }
        }
    }

    /// When the next held datagram is due, so a driver can sleep until then.
    pub fn next_due(&self) -> Option<Instant> {
        self.queue.peek().map(|Reverse(q)| q.at)
    }
}
