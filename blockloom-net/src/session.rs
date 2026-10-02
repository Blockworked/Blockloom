use std::collections::BTreeMap;
use std::time::Instant;

use crate::link::Link;
use crate::{MAX_UDP_PAYLOAD, NetError};

/// Bytes a peer may have queued for reliable delivery before sends are refused.
pub const MAX_QUEUED_STREAM_BYTES: usize = 4 * 1024 * 1024;

const COMPACT_AT: usize = 1024 * 1024;

/// What one connection reports to its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Established,
    Stream {
        stream: u64,
        data: Vec<u8>,
        fin: bool,
    },
    Datagram(Vec<u8>),
    Closed {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TransportStats {
    pub rtt_ms: f64,
    pub lost: usize,
    pub retransmitted: usize,
    pub sent_bytes: u64,
    pub recv_bytes: u64,
    pub dgram_max: Option<usize>,
    pub queued_stream_bytes: usize,
}

#[derive(Default)]
struct Pending {
    buf: Vec<u8>,
    sent: usize,
    fin: bool,
}

/// One QUIC connection plus the application-side queues around it.
pub(crate) struct Session {
    pub conn: quiche::Connection,
    pending: BTreeMap<u64, Pending>,
    queued: usize,
    deadline: Option<Instant>,
    established: bool,
    closed: bool,
}

impl Session {
    pub fn new(conn: quiche::Connection) -> Self {
        Session {
            conn,
            pending: BTreeMap::new(),
            queued: 0,
            deadline: None,
            established: false,
            closed: false,
        }
    }

    pub fn on_packet(&mut self, buf: &mut [u8], info: quiche::RecvInfo) {
        // A bad packet closes nothing by itself; quiche reports real failures through the connection.
        let _ = self.conn.recv(buf, info);
    }

    /// Queue bytes on a stream. Refused rather than buffered without limit.
    pub fn send_stream(&mut self, stream: u64, data: &[u8], fin: bool) -> Result<(), NetError> {
        if !self.conn.is_established() {
            return Err(NetError::NotEstablished);
        }
        if self.queued + data.len() > MAX_QUEUED_STREAM_BYTES {
            return Err(NetError::QueueFull);
        }
        let p = self.pending.entry(stream).or_default();
        p.buf.extend_from_slice(data);
        p.fin |= fin;
        self.queued += data.len();
        self.push_streams();
        Ok(())
    }

    /// Ok(true) if queued, Ok(false) if the datagram queue was full (state is replaceable, so drop it).
    pub fn send_datagram(&mut self, data: &[u8]) -> Result<bool, NetError> {
        let max = self.conn.dgram_max_writable_len();
        match max {
            Some(m) if data.len() <= m => {}
            _ => return Err(NetError::DatagramTooLarge { max }),
        }
        match self.conn.dgram_send(data) {
            Ok(()) => Ok(true),
            Err(quiche::Error::Done) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub fn close(&mut self, code: u64, reason: &[u8]) {
        let _ = self.conn.close(true, code, reason);
    }

    fn push_streams(&mut self) {
        let ids: Vec<u64> = self.pending.keys().copied().collect();
        for id in ids {
            let Some(p) = self.pending.get_mut(&id) else {
                continue;
            };
            let done = loop {
                let rest = &p.buf[p.sent..];
                if rest.is_empty() && !p.fin {
                    break true;
                }
                match self.conn.stream_send(id, rest, p.fin) {
                    Ok(n) => {
                        p.sent += n;
                        self.queued -= n;
                        if p.sent == p.buf.len() {
                            break true;
                        }
                    }
                    Err(quiche::Error::Done) => break false,
                    Err(_) => {
                        // The stream is gone (reset or stopped): forget what was waiting for it.
                        self.queued -= rest.len();
                        break true;
                    }
                }
            };
            if done {
                self.pending.remove(&id);
            } else if p.sent >= COMPACT_AT {
                p.buf.drain(..p.sent);
                p.sent = 0;
            }
        }
    }

    /// Run timers, collect what arrived, move queued bytes onto the wire.
    pub fn service(
        &mut self,
        link: &mut Link,
        peer: std::net::SocketAddr,
        events: &mut Vec<Event>,
    ) {
        let now = Instant::now();
        if self.deadline.is_some_and(|d| now >= d) {
            self.conn.on_timeout();
        }

        if self.conn.is_established() && !self.established {
            self.established = true;
            events.push(Event::Established);
        }

        if self.established {
            let mut buf = [0u8; 16 * 1024];
            let readable: Vec<u64> = self.conn.readable().collect();
            for id in readable {
                while let Ok((n, fin)) = self.conn.stream_recv(id, &mut buf) {
                    events.push(Event::Stream {
                        stream: id,
                        data: buf[..n].to_vec(),
                        fin,
                    });
                    if fin {
                        break;
                    }
                }
            }
            let mut dgram = [0u8; MAX_UDP_PAYLOAD];
            while let Ok(n) = self.conn.dgram_recv(&mut dgram) {
                events.push(Event::Datagram(dgram[..n].to_vec()));
            }
            self.push_streams();
        }

        let mut out = [0u8; MAX_UDP_PAYLOAD];
        loop {
            match self.conn.send(&mut out) {
                Ok((n, info)) => link.send(&out[..n], peer, info.at),
                Err(quiche::Error::Done) => break,
                Err(_) => {
                    let _ = self.conn.close(false, 0x1, b"send failed");
                    break;
                }
            }
        }

        self.deadline = self.conn.timeout().map(|t| Instant::now() + t);

        if self.conn.is_closed() && !self.closed {
            self.closed = true;
            events.push(Event::Closed {
                reason: self.close_reason(),
            });
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn close_reason(&self) -> String {
        if let Some(e) = self.conn.peer_error() {
            format!(
                "peer closed: code {} {}",
                e.error_code,
                String::from_utf8_lossy(&e.reason)
            )
        } else if let Some(e) = self.conn.local_error() {
            format!(
                "closed locally: code {} {}",
                e.error_code,
                String::from_utf8_lossy(&e.reason)
            )
        } else if self.conn.is_timed_out() {
            "timed out".into()
        } else {
            "closed".into()
        }
    }

    pub fn stats(&self) -> TransportStats {
        let s = self.conn.stats();
        let rtt = self
            .conn
            .path_stats()
            .next()
            .map(|p| p.rtt.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        TransportStats {
            rtt_ms: rtt,
            lost: s.lost,
            retransmitted: s.retrans,
            sent_bytes: s.sent_bytes,
            recv_bytes: s.recv_bytes,
            dgram_max: self.conn.dgram_max_writable_len(),
            queued_stream_bytes: self.queued,
        }
    }
}
