//! Spike: the same pinned-identity game connection built on `tokio-quiche`'s
//! raw `ApplicationOverQuic` API, to compare with `blockloom-net`'s own driver.
//! Not used by the crate; see docs/multiplayer-phase0.md.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use blockloom_net::{ALPN, Fingerprint, Identity};
use boring::pkey::{PKey, Private};
use boring::ssl::{SslAlert, SslContextBuilder, SslMethod, SslVerifyError, SslVerifyMode};
use boring::x509::X509;
use futures::StreamExt;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio_quiche::metrics::DefaultMetrics;
use tokio_quiche::quic::{ConnectionHook, HandshakeInfo, QuicheConnection, connect_with_config};
use tokio_quiche::settings::{
    CertificateKind, ConnectionParams, Hooks, QuicSettings, TlsCertificatePaths,
};
use tokio_quiche::socket::Socket;
use tokio_quiche::{ApplicationOverQuic, QuicResult, listen};

#[derive(Debug)]
enum Cmd {
    Stream(u64, Vec<u8>, bool),
    Datagram(Vec<u8>),
}

#[derive(Debug, PartialEq)]
enum Ev {
    Established(Vec<u8>),
    Stream {
        stream: u64,
        data: Vec<u8>,
        fin: bool,
    },
    Datagram(Vec<u8>),
}

struct Pending {
    stream: u64,
    buf: Vec<u8>,
    sent: usize,
    fin: bool,
}

struct App {
    cmds: mpsc::UnboundedReceiver<Cmd>,
    events: mpsc::UnboundedSender<Ev>,
    buffered: VecDeque<Cmd>,
    pending: Vec<Pending>,
}

/// What a test holds for one connection.
struct Handle {
    cmds: mpsc::UnboundedSender<Cmd>,
    events: mpsc::UnboundedReceiver<Ev>,
}

fn app() -> (App, Handle) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    (
        App {
            cmds: cmd_rx,
            events: ev_tx,
            buffered: VecDeque::new(),
            pending: Vec::new(),
        },
        Handle {
            cmds: cmd_tx,
            events: ev_rx,
        },
    )
}

impl ApplicationOverQuic for App {
    fn on_conn_established(
        &mut self,
        qconn: &mut QuicheConnection,
        _: &HandshakeInfo,
    ) -> QuicResult<()> {
        let _ = self
            .events
            .send(Ev::Established(qconn.application_proto().to_vec()));
        Ok(())
    }

    fn should_act(&self) -> bool {
        true
    }

    async fn wait_for_data(&mut self, _: &mut QuicheConnection) -> QuicResult<()> {
        match self.cmds.recv().await {
            Some(c) => {
                self.buffered.push_back(c);
                Ok(())
            }
            None => std::future::pending().await,
        }
    }

    fn process_reads(&mut self, qconn: &mut QuicheConnection) -> QuicResult<()> {
        let mut buf = [0u8; 16 * 1024];
        let readable: Vec<u64> = qconn.readable().collect();
        for id in readable {
            while let Ok((n, fin)) = qconn.stream_recv(id, &mut buf) {
                let _ = self.events.send(Ev::Stream {
                    stream: id,
                    data: buf[..n].to_vec(),
                    fin,
                });
                if fin {
                    break;
                }
            }
        }
        let mut d = [0u8; 1350];
        while let Ok(n) = qconn.dgram_recv(&mut d) {
            let _ = self.events.send(Ev::Datagram(d[..n].to_vec()));
        }
        Ok(())
    }

    fn process_writes(&mut self, qconn: &mut QuicheConnection) -> QuicResult<()> {
        while let Some(cmd) = self.buffered.pop_front() {
            match cmd {
                Cmd::Datagram(d) => {
                    let _ = qconn.dgram_send(&d);
                }
                Cmd::Stream(stream, buf, fin) => self.pending.push(Pending {
                    stream,
                    buf,
                    sent: 0,
                    fin,
                }),
            }
        }
        self.pending.retain_mut(|p| {
            loop {
                let rest = &p.buf[p.sent..];
                if rest.is_empty() && !p.fin {
                    return false;
                }
                match qconn.stream_send(p.stream, rest, p.fin) {
                    Ok(n) => {
                        p.sent += n;
                        if p.sent == p.buf.len() {
                            return false;
                        }
                    }
                    Err(_) => return true,
                }
            }
        });
        Ok(())
    }
}

struct ServerHook {
    cert: X509,
    key: PKey<Private>,
}

impl ConnectionHook for ServerHook {
    fn create_custom_ssl_context_builder(
        &self,
        _: TlsCertificatePaths<'_>,
    ) -> Option<SslContextBuilder> {
        let mut b = SslContextBuilder::new(SslMethod::tls()).ok()?;
        b.set_certificate(&self.cert).ok()?;
        b.set_private_key(&self.key).ok()?;
        Some(b)
    }
}

struct PinHook(Fingerprint);

impl ConnectionHook for PinHook {
    fn create_custom_ssl_context_builder(
        &self,
        _: TlsCertificatePaths<'_>,
    ) -> Option<SslContextBuilder> {
        let pin = self.0;
        let mut b = SslContextBuilder::new(SslMethod::tls()).ok()?;
        b.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
            let der = ssl.peer_certificate().and_then(|c| c.to_der().ok());
            match der {
                Some(der) if Fingerprint::of_der(&der) == pin => Ok(()),
                _ => Err(SslVerifyError::Invalid(SslAlert::BAD_CERTIFICATE)),
            }
        });
        Some(b)
    }
}

// The hook only runs when certificate paths are given, even though it never reads them.
const NO_FILES: TlsCertificatePaths<'static> = TlsCertificatePaths {
    cert: "",
    private_key: "",
    kind: CertificateKind::X509,
};

fn settings(alpn: &[u8]) -> QuicSettings {
    let mut s = QuicSettings::default();
    s.alpn = vec![alpn.to_vec()];
    s.max_idle_timeout = Some(Duration::from_secs(10));
    s.max_recv_udp_payload_size = 1350;
    s.max_send_udp_payload_size = 1350;
    s
}

/// Starts a server and returns its address and a stream of accepted connections.
async fn serve(identity: &Identity) -> (SocketAddr, mpsc::UnboundedReceiver<Handle>) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let hooks = Hooks {
        connection_hook: Some(Arc::new(ServerHook {
            cert: identity.cert().clone(),
            key: identity.key().clone(),
        })),
    };
    let params = ConnectionParams::new_server(settings(ALPN), NO_FILES, hooks);
    let mut listeners = listen([socket], params, DefaultMetrics).unwrap();
    let mut accepted = listeners.remove(0);
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(conn) = accepted.next().await {
            let (a, handle) = app();
            conn.unwrap().start(a);
            let _ = tx.send(handle);
        }
    });
    (addr, rx)
}

async fn dial(addr: SocketAddr, pin: Fingerprint, alpn: &[u8]) -> QuicResult<Handle> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.connect(addr).await?;
    let socket: Socket<_, _> = socket.try_into()?;
    let hooks = Hooks {
        connection_hook: Some(Arc::new(PinHook(pin))),
    };
    let params = ConnectionParams::new_client(settings(alpn), Some(NO_FILES), hooks);
    let (a, handle) = app();
    connect_with_config(socket, None, &params, a).await?;
    Ok(handle)
}

async fn next(h: &mut Handle, within: Duration) -> Option<Ev> {
    tokio::time::timeout(within, h.events.recv())
        .await
        .ok()
        .flatten()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_handshake_stream_and_datagrams() {
    let id = Identity::generate(&["localhost"]).unwrap();
    let (addr, mut accepted) = serve(&id).await;

    let t0 = Instant::now();
    let mut client = dial(addr, id.fingerprint(), ALPN)
        .await
        .expect("pinned connect");
    assert_eq!(
        next(&mut client, Duration::from_secs(5)).await,
        Some(Ev::Established(ALPN.to_vec()))
    );
    let handshake = t0.elapsed();
    let mut server = tokio::time::timeout(Duration::from_secs(5), accepted.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        next(&mut server, Duration::from_secs(5)).await,
        Some(Ev::Established(ALPN.to_vec()))
    );

    let data: Vec<u8> = (0..768 * 1024)
        .map(|i: usize| (i.wrapping_mul(31) ^ (i >> 8)) as u8)
        .collect();
    let t1 = Instant::now();
    client
        .cmds
        .send(Cmd::Stream(0, data.clone(), true))
        .unwrap();
    server.cmds.send(Cmd::Datagram(b"pose".to_vec())).unwrap();
    client.cmds.send(Cmd::Datagram(b"input".to_vec())).unwrap();

    let mut got = Vec::new();
    let mut fin = false;
    let mut server_dgram = false;
    let mut client_dgram = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !(fin && server_dgram && client_dgram) {
        if let Some(ev) = next(&mut server, Duration::from_millis(20)).await {
            match ev {
                Ev::Stream { data, fin: f, .. } => {
                    got.extend(data);
                    fin |= f;
                }
                Ev::Datagram(d) if d == b"input" => server_dgram = true,
                _ => {}
            }
        }
        if let Some(Ev::Datagram(d)) = next(&mut client, Duration::from_millis(1)).await {
            client_dgram |= d == b"pose";
        }
    }
    assert!(
        fin && server_dgram && client_dgram,
        "got {} bytes",
        got.len()
    );
    assert_eq!(got, data);
    eprintln!(
        "tokio-quiche: handshake {handshake:?}, 768 KiB in {:?}",
        t1.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_fingerprint_never_connects() {
    let id = Identity::generate(&["localhost"]).unwrap();
    let (addr, mut accepted) = serve(&id).await;
    let wrong = Identity::generate(&["localhost"]).unwrap().fingerprint();
    let result = tokio::time::timeout(Duration::from_secs(5), dial(addr, wrong, ALPN)).await;
    if let Ok(Ok(mut h)) = result {
        assert!(!matches!(
            next(&mut h, Duration::from_secs(2)).await,
            Some(Ev::Established(_))
        ));
    }
    if let Ok(Some(mut s)) = tokio::time::timeout(Duration::from_millis(500), accepted.recv()).await
    {
        assert!(!matches!(
            next(&mut s, Duration::from_secs(1)).await,
            Some(Ev::Established(_))
        ));
    }
}
