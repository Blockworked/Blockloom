use std::net::SocketAddr;
use std::time::{Duration, Instant};

use boring::ssl::{SslAlert, SslContextBuilder, SslMethod, SslVerifyError, SslVerifyMode};
use quiche::ConnectionId;
use ring::rand::{SecureRandom, SystemRandom};

use crate::identity::Fingerprint;
use crate::link::{Impairment, Link, LinkStats};
use crate::server::apply_transport;
use crate::session::{Event, Session, TransportStats};
use crate::{ALPN, NetError};

#[derive(Clone, Debug)]
pub struct ClientOptions {
    pub idle_timeout: Duration,
    pub impairment: Impairment,
    /// Application protocol to offer; only tests use anything but `ALPN`.
    pub alpn: Vec<u8>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        ClientOptions {
            idle_timeout: Duration::from_secs(10),
            impairment: Impairment::default(),
            alpn: ALPN.to_vec(),
        }
    }
}

/// One outgoing connection to a host whose certificate fingerprint is known.
pub struct Client {
    link: Link,
    server: SocketAddr,
    session: Session,
    started: Instant,
    established_after: Option<Duration>,
}

impl Client {
    /// The TLS handshake fails unless the server's certificate hashes to `pin`.
    /// A name or an IP address is never what is trusted.
    pub fn connect(
        server: SocketAddr,
        pin: Fingerprint,
        opts: ClientOptions,
    ) -> Result<Self, NetError> {
        let bind: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        }
        .parse()
        .expect("literal");
        let mut link = Link::bind(bind, opts.impairment)?;
        let local = link.local_addr()?;

        let mut tls = SslContextBuilder::new(SslMethod::tls())?;
        // The custom callback replaces chain validation: the pinned hash is the whole trust decision.
        tls.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
            let der = ssl.peer_certificate().and_then(|c| c.to_der().ok());
            match der {
                Some(der) if Fingerprint::of_der(&der) == pin => Ok(()),
                _ => Err(SslVerifyError::Invalid(SslAlert::BAD_CERTIFICATE)),
            }
        });
        let mut config =
            quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, tls)?;
        apply_transport(&mut config, opts.idle_timeout, &opts.alpn);

        let mut scid = [0u8; quiche::MAX_CONN_ID_LEN];
        SystemRandom::new()
            .fill(&mut scid)
            .map_err(|_| NetError::Tls("no randomness".into()))?;
        let conn = quiche::connect(
            None,
            &ConnectionId::from_ref(&scid),
            local,
            server,
            &mut config,
        )?;

        let mut session = Session::new(conn);
        // Send the first Initial now rather than on the first poll.
        session.service(&mut link, server, &mut Vec::new());
        Ok(Client {
            link,
            server,
            session,
            started: Instant::now(),
            established_after: None,
        })
    }

    pub fn poll(&mut self) -> Vec<Event> {
        self.link.flush();
        let mut buf = [0u8; 65535];
        let local = self.link.local_addr().expect("bound");
        while let Some((n, from)) = self.link.recv(&mut buf) {
            if from == self.server {
                self.session
                    .on_packet(&mut buf[..n], quiche::RecvInfo { from, to: local });
            }
        }
        let mut events = Vec::new();
        self.session
            .service(&mut self.link, self.server, &mut events);
        if self.established_after.is_none() && events.contains(&Event::Established) {
            self.established_after = Some(self.started.elapsed());
        }
        events
    }

    pub fn is_established(&self) -> bool {
        self.session.conn.is_established()
    }

    pub fn is_closed(&self) -> bool {
        self.session.is_closed()
    }

    /// Wall time from `connect` to an established connection (Retry round trip included).
    pub fn handshake_time(&self) -> Option<Duration> {
        self.established_after
    }

    pub fn application_proto(&self) -> Vec<u8> {
        self.session.conn.application_proto().to_vec()
    }

    pub fn send_stream(&mut self, stream: u64, data: &[u8], fin: bool) -> Result<(), NetError> {
        self.session.send_stream(stream, data, fin)
    }

    pub fn send_datagram(&mut self, data: &[u8]) -> Result<bool, NetError> {
        self.session.send_datagram(data)
    }

    pub fn close(&mut self, reason: &str) {
        self.session.close(0x100, reason.as_bytes());
    }

    pub fn stats(&self) -> TransportStats {
        self.session.stats()
    }

    pub fn link_stats(&self) -> LinkStats {
        self.link.stats
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.session
            .next_deadline()
            .into_iter()
            .chain(self.link.next_due())
            .min()
    }
}
