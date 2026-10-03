use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use boring::ssl::{SslContextBuilder, SslMethod};
use quiche::ConnectionId;
use ring::{hmac, rand::SecureRandom, rand::SystemRandom};

use crate::identity::Identity;
use crate::link::{Impairment, Link, LinkStats};
use crate::session::{Event, Session, TransportStats};
use crate::{ALPN, MAX_UDP_PAYLOAD, NetError};

/// A connected remote, stable for the life of its connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub u64);

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub max_peers: usize,
    pub idle_timeout: Duration,
    pub impairment: Impairment,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            max_peers: 16,
            idle_timeout: Duration::from_secs(10),
            impairment: Impairment::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ServerStats {
    pub retries_sent: u64,
    pub bad_tokens: u64,
    pub refused_full: u64,
    pub unparsable: u64,
    pub link: LinkStats,
}

struct Peer {
    session: Session,
    addr: SocketAddr,
}

/// Accepts connections on one UDP socket. Single-threaded: call `poll` often.
pub struct Server {
    link: Link,
    local: SocketAddr,
    config: quiche::Config,
    key: hmac::Key,
    peers: BTreeMap<PeerId, Peer>,
    by_cid: HashMap<Vec<u8>, PeerId>,
    next_id: u64,
    opts: ServerOptions,
    stats: ServerStats,
}

impl Server {
    pub fn bind(
        addr: SocketAddr,
        identity: &Identity,
        opts: ServerOptions,
    ) -> Result<Self, NetError> {
        let link = Link::bind(addr, opts.impairment)?;
        let local = link.local_addr()?;

        let mut tls = SslContextBuilder::new(SslMethod::tls())?;
        tls.set_certificate(identity.cert())?;
        tls.set_private_key(identity.key())?;
        tls.check_private_key()?;
        let mut config =
            quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, tls)?;
        apply_transport(&mut config, opts.idle_timeout, ALPN);

        let mut key_bytes = [0u8; 32];
        SystemRandom::new()
            .fill(&mut key_bytes)
            .map_err(|_| NetError::Tls("no randomness".into()))?;

        Ok(Server {
            link,
            local,
            config,
            key: hmac::Key::new(hmac::HMAC_SHA256, &key_bytes),
            peers: BTreeMap::new(),
            by_cid: HashMap::new(),
            next_id: 1,
            opts,
            stats: ServerStats::default(),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    pub fn stats(&self) -> ServerStats {
        ServerStats {
            link: self.link.stats,
            ..self.stats
        }
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn peer_stats(&self, peer: PeerId) -> Option<TransportStats> {
        self.peers.get(&peer).map(|p| p.session.stats())
    }

    pub fn send_stream(
        &mut self,
        peer: PeerId,
        stream: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<(), NetError> {
        self.peer_mut(peer)?.session.send_stream(stream, data, fin)
    }

    pub fn send_datagram(&mut self, peer: PeerId, data: &[u8]) -> Result<bool, NetError> {
        self.peer_mut(peer)?.session.send_datagram(data)
    }

    /// Disconnect a peer; its `Closed` event arrives from a later `poll`.
    pub fn kick(&mut self, peer: PeerId, reason: &str) {
        if let Some(p) = self.peers.get_mut(&peer) {
            p.session.close(0x100, reason.as_bytes());
        }
    }

    fn peer_mut(&mut self, peer: PeerId) -> Result<&mut Peer, NetError> {
        self.peers.get_mut(&peer).ok_or(NetError::NotEstablished)
    }

    /// The soonest moment `poll` has something to do without new input.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.peers
            .values()
            .filter_map(|p| p.session.next_deadline())
            .chain(self.link.next_due())
            .min()
    }

    pub fn poll(&mut self) -> Vec<(PeerId, Event)> {
        self.link.flush();
        let mut buf = [0u8; 65535];
        while let Some((n, from)) = self.link.recv(&mut buf) {
            self.on_datagram(&mut buf[..n], from);
        }

        let mut events = Vec::new();
        let mut gone = Vec::new();
        for (id, peer) in self.peers.iter_mut() {
            let mut local = Vec::new();
            peer.session.service(&mut self.link, peer.addr, &mut local);
            events.extend(local.into_iter().map(|e| (*id, e)));
            if peer.session.is_closed() {
                gone.push(*id);
            }
        }
        for id in gone {
            self.peers.remove(&id);
            self.by_cid.retain(|_, v| *v != id);
        }
        events
    }

    fn token(&self, from: SocketAddr, odcid: &[u8]) -> Vec<u8> {
        let tag = hmac::sign(&self.key, &[from.to_string().as_bytes(), odcid].concat());
        [tag.as_ref(), odcid].concat()
    }

    /// The connection ID a validated client must use after Retry. It is full length because a
    /// short header does not say how long its destination ID is.
    fn retry_scid(&self, from: SocketAddr, odcid: &[u8]) -> [u8; quiche::MAX_CONN_ID_LEN] {
        let tag = hmac::sign(
            &self.key,
            &[b"scid", from.to_string().as_bytes(), odcid].concat(),
        );
        let mut out = [0u8; quiche::MAX_CONN_ID_LEN];
        out.copy_from_slice(&tag.as_ref()[..quiche::MAX_CONN_ID_LEN]);
        out
    }

    fn on_datagram(&mut self, pkt: &mut [u8], from: SocketAddr) {
        let Ok(hdr) = quiche::Header::from_slice(pkt, quiche::MAX_CONN_ID_LEN) else {
            self.stats.unparsable += 1;
            return;
        };
        let info = quiche::RecvInfo {
            from,
            to: self.local,
        };

        if let Some(id) = self.by_cid.get(hdr.dcid.as_ref()).copied() {
            if let Some(p) = self.peers.get_mut(&id) {
                p.session.on_packet(pkt, info);
            }
            return;
        }
        if hdr.ty != quiche::Type::Initial {
            return;
        }

        let mut out = [0u8; MAX_UDP_PAYLOAD];
        if !quiche::version_is_supported(hdr.version) {
            if let Ok(n) = quiche::negotiate_version(&hdr.scid, &hdr.dcid, &mut out) {
                self.link.send(&out[..n], from, Instant::now());
            }
            return;
        }

        // Address validation first: no connection state exists until the client echoes a token.
        let token = hdr.token.as_deref().unwrap_or(&[]);
        if token.is_empty() {
            let new_scid = self.retry_scid(from, &hdr.dcid);
            let new_token = self.token(from, &hdr.dcid);
            let new_scid = ConnectionId::from_ref(&new_scid);
            if let Ok(n) = quiche::retry(
                &hdr.scid,
                &hdr.dcid,
                &new_scid,
                &new_token,
                hdr.version,
                &mut out,
            ) {
                self.stats.retries_sent += 1;
                self.link.send(&out[..n], from, Instant::now());
            }
            return;
        }
        if token.len() <= 32 {
            self.stats.bad_tokens += 1;
            return;
        }
        let (tag, odcid) = token.split_at(32);
        let expect = [from.to_string().as_bytes(), odcid].concat();
        if hmac::verify(&self.key, &expect, tag).is_err()
            || hdr.dcid.as_ref() != self.retry_scid(from, odcid)
        {
            self.stats.bad_tokens += 1;
            return;
        }
        if self.peers.len() >= self.opts.max_peers {
            self.stats.refused_full += 1;
            return;
        }

        let scid = hdr.dcid.clone();
        let odcid = ConnectionId::from_ref(odcid);
        let Ok(conn) = quiche::accept(&scid, Some(&odcid), self.local, from, &mut self.config)
        else {
            return;
        };
        let id = PeerId(self.next_id);
        self.next_id += 1;
        let mut session = Session::new(conn);
        session.on_packet(pkt, info);
        self.by_cid.insert(scid.to_vec(), id);
        self.peers.insert(
            id,
            Peer {
                session,
                addr: from,
            },
        );
    }
}

/// Limits both ends share: per-peer flow control, stream counts and idle timeout.
pub(crate) fn apply_transport(config: &mut quiche::Config, idle: Duration, alpn: &[u8]) {
    config.set_application_protos(&[alpn]).expect("alpn");
    config.set_max_idle_timeout(idle.as_millis() as u64);
    config.set_max_recv_udp_payload_size(MAX_UDP_PAYLOAD);
    config.set_max_send_udp_payload_size(MAX_UDP_PAYLOAD);
    config.set_initial_max_data(4 * 1024 * 1024);
    config.set_initial_max_stream_data_bidi_local(1024 * 1024);
    config.set_initial_max_stream_data_bidi_remote(1024 * 1024);
    config.set_initial_max_stream_data_uni(1024 * 1024);
    config.set_initial_max_streams_bidi(16);
    config.set_initial_max_streams_uni(16);
    config.set_disable_active_migration(true);
    config.enable_dgram(true, 256, 256);
    config.enable_pacing(true);
}
