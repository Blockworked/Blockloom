//! Server and client in one thread over real loopback UDP.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use blockloom_net::{
    Client, ClientOptions, Event, Fingerprint, Identity, Impairment, NetError, PeerId, Server,
    ServerOptions,
};

struct Rig {
    server: Server,
    client: Client,
    peer: Option<PeerId>,
    server_events: Vec<Event>,
    client_events: Vec<Event>,
}

fn identity() -> Identity {
    Identity::generate(&["localhost"]).unwrap()
}

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn rig(server_opts: ServerOptions, client_opts: ClientOptions) -> Rig {
    let id = identity();
    let server = Server::bind(loopback(), &id, server_opts).unwrap();
    let client = Client::connect(server.local_addr(), id.fingerprint(), client_opts).unwrap();
    Rig {
        server,
        client,
        peer: None,
        server_events: vec![],
        client_events: vec![],
    }
}

impl Rig {
    fn step(&mut self) {
        for (id, e) in self.server.poll() {
            self.peer = Some(id);
            self.server_events.push(e);
        }
        let events = self.client.poll();
        self.client_events.extend(events);
        std::thread::sleep(Duration::from_micros(300));
    }

    fn until(&mut self, limit: Duration, mut done: impl FnMut(&mut Rig) -> bool) -> bool {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn connect(&mut self) {
        assert!(
            self.until(Duration::from_secs(10), |r| r.client.is_established()
                && r.server_events.contains(&Event::Established))
        );
    }

    fn server_stream_bytes(&self, stream: u64) -> Vec<u8> {
        self.server_events
            .iter()
            .filter_map(|e| match e {
                Event::Stream {
                    stream: s, data, ..
                } if *s == stream => Some(data.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn server_stream_fin(&self, stream: u64) -> bool {
        self.server_events
            .iter()
            .any(|e| matches!(e, Event::Stream { stream: s, fin: true, .. } if *s == stream))
    }
}

fn payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 8)) as u8)
        .collect()
}

#[test]
fn pinned_handshake_streams_and_datagrams() {
    let mut r = rig(ServerOptions::default(), ClientOptions::default());
    r.connect();
    assert_eq!(r.client.application_proto(), blockloom_net::ALPN);
    assert!(
        r.server.stats().retries_sent >= 1,
        "address validated by Retry before any state"
    );
    assert_eq!(r.server.peer_count(), 1);

    // Client to server, reliable, larger than one packet and one flow-control window.
    let data = payload(768 * 1024);
    r.client.send_stream(0, &data, true).unwrap();
    assert!(r.until(Duration::from_secs(10), |r| r.server_stream_fin(0)));
    assert_eq!(r.server_stream_bytes(0), data);

    // Server to client.
    let peer = r.peer.unwrap();
    r.server.send_stream(peer, 0, b"welcome", true).unwrap();
    r.server.send_datagram(peer, b"pose").unwrap();
    r.client.send_datagram(b"input").unwrap();
    assert!(r.until(Duration::from_secs(5), |r| {
        r.client_events.contains(&Event::Datagram(b"pose".to_vec()))
            && r.server_events
                .contains(&Event::Datagram(b"input".to_vec()))
            && r.client_events
                .iter()
                .any(|e| matches!(e, Event::Stream { data, .. } if data == b"welcome"))
    }));

    let max = r.client.stats().dgram_max.expect("datagrams negotiated");
    assert!(
        max > 1000 && max < blockloom_net::MAX_UDP_PAYLOAD,
        "max datagram {max}"
    );
    assert!(matches!(
        r.client.send_datagram(&vec![0u8; max + 1]),
        Err(NetError::DatagramTooLarge { .. })
    ));
}

#[test]
fn wrong_fingerprint_never_connects() {
    let id = identity();
    let server = Server::bind(loopback(), &id, ServerOptions::default()).unwrap();
    let wrong = identity().fingerprint();
    let client = Client::connect(server.local_addr(), wrong, ClientOptions::default()).unwrap();
    let mut r = Rig {
        server,
        client,
        peer: None,
        server_events: vec![],
        client_events: vec![],
    };

    assert!(r.until(Duration::from_secs(5), |r| r.client.is_closed()));
    assert!(!r.client.is_established());
    assert!(!r.server_events.contains(&Event::Established));
    assert!(
        r.client_events
            .iter()
            .any(|e| matches!(e, Event::Closed { .. }))
    );
}

#[test]
fn wrong_application_protocol_is_refused() {
    let opts = ClientOptions {
        alpn: b"editor/1".to_vec(),
        ..ClientOptions::default()
    };
    let mut r = rig(ServerOptions::default(), opts);
    assert!(r.until(Duration::from_secs(5), |r| r.client.is_closed()));
    assert!(!r.server_events.contains(&Event::Established));
}

#[test]
fn garbage_and_forged_packets_change_nothing() {
    let id = identity();
    let mut server = Server::bind(loopback(), &id, ServerOptions::default()).unwrap();
    let addr = server.local_addr();

    let raw = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut seed = 7u64;
    let mut noise = |len: usize| -> Vec<u8> {
        (0..len)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (seed >> 33) as u8
            })
            .collect()
    };
    for len in [0, 1, 7, 40, 200, 1200, 1350, 5000] {
        raw.send_to(&noise(len), addr).unwrap();
    }
    // A long-header Initial shaped packet with a forged token.
    let mut forged = vec![0xc0, 0, 0, 0, 1, 8];
    forged.extend([1u8; 8]);
    forged.extend([8u8]);
    forged.extend([2u8; 8]);
    forged.extend([0x40]);
    forged.extend([9u8; 64]);
    forged.resize(1200, 0);
    raw.send_to(&forged, addr).unwrap();

    for _ in 0..50 {
        assert!(server.poll().is_empty());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(server.peer_count(), 0);

    // A real client still gets in afterwards.
    let client = Client::connect(addr, id.fingerprint(), ClientOptions::default()).unwrap();
    let mut r = Rig {
        server,
        client,
        peer: None,
        server_events: vec![],
        client_events: vec![],
    };
    r.connect();
}

#[test]
fn peer_cap_refuses_extra_clients() {
    let id = identity();
    let mut server = Server::bind(
        loopback(),
        &id,
        ServerOptions {
            max_peers: 1,
            ..ServerOptions::default()
        },
    )
    .unwrap();
    let addr = server.local_addr();
    let mut a = Client::connect(addr, id.fingerprint(), ClientOptions::default()).unwrap();
    let mut b = Client::connect(addr, id.fingerprint(), ClientOptions::default()).unwrap();

    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        server.poll();
        a.poll();
        b.poll();
        std::thread::sleep(Duration::from_micros(300));
    }
    assert_eq!(server.peer_count(), 1);
    assert!(a.is_established() ^ b.is_established());
    assert!(server.stats().refused_full > 0);
}

#[test]
fn reliable_queue_is_bounded() {
    let mut r = rig(ServerOptions::default(), ClientOptions::default());
    r.connect();
    let chunk = vec![0u8; 1024 * 1024];
    let mut accepted = 0usize;
    // Nothing is polled, so nothing drains: the queue must say no instead of growing.
    let refused = loop {
        match r.client.send_stream(0, &chunk, false) {
            Ok(()) => accepted += chunk.len(),
            Err(e) => break e,
        }
        assert!(accepted <= 64 * 1024 * 1024, "queue never refused");
    };
    assert!(matches!(refused, NetError::QueueFull));
    assert!(r.client.stats().queued_stream_bytes <= 4 * 1024 * 1024);
}

#[test]
fn kick_closes_the_peer() {
    let mut r = rig(ServerOptions::default(), ClientOptions::default());
    r.connect();
    let peer = r.peer.unwrap();
    r.server.kick(peer, "bye");
    assert!(
        r.until(Duration::from_secs(5), |r| r.client_events.iter().any(
            |e| matches!(e, Event::Closed { reason } if reason.contains("bye"))
        ))
    );
    assert!(r.until(Duration::from_secs(5), |r| r.server.peer_count() == 0));
}

/// Each side delays what it sends by half the round trip, so the two together make `rtt_ms`.
fn impaired(rtt_ms: u64, loss: f64, seed: u64) -> Impairment {
    Impairment {
        loss,
        duplicate: loss / 2.0,
        delay: Duration::from_millis(rtt_ms / 2),
        jitter: Duration::from_millis(if rtt_ms == 0 { 0 } else { 20 }),
        seed,
    }
}

/// Run one impaired session: a reliable transfer plus a datagram stream.
/// Returns (time until the stream finished, datagrams received of 200, handshake time).
fn run_impaired(rtt_ms: u64, loss: f64, bytes: usize) -> (Duration, usize, Duration) {
    let so = ServerOptions {
        impairment: impaired(rtt_ms, loss, 1),
        ..ServerOptions::default()
    };
    let co = ClientOptions {
        impairment: impaired(rtt_ms, loss, 2),
        ..ClientOptions::default()
    };
    let mut r = rig(so, co);
    assert!(
        r.until(Duration::from_secs(30), |r| r.client.is_established()
            && r.server_events.contains(&Event::Established))
    );
    let handshake = r.client.handshake_time().unwrap();

    let data = payload(bytes);
    let t0 = Instant::now();
    r.client.send_stream(0, &data, true).unwrap();
    let mut sent_dgrams = 0u32;
    let mut next = Instant::now();
    let mut fin_at = None;
    assert!(r.until(Duration::from_secs(60), |r| {
        if fin_at.is_none() && r.server_stream_fin(0) {
            fin_at = Some(t0.elapsed());
        }
        if sent_dgrams < 200 && Instant::now() >= next {
            let mut d = sent_dgrams.to_be_bytes().to_vec();
            d.extend([7u8; 60]);
            let _ = r.client.send_datagram(&d);
            sent_dgrams += 1;
            next += Duration::from_millis(5);
        }
        r.server_stream_fin(0) && sent_dgrams == 200
    }));
    let took = fin_at.expect("finished");
    assert_eq!(
        r.server_stream_bytes(0),
        data,
        "reliable stream must arrive intact at {loss} loss"
    );

    // Let the last datagrams land, then check each one is whole, in range, and not repeated more than sent.
    r.until(Duration::from_millis(300 + rtt_ms), |_| false);
    let mut seen = std::collections::BTreeSet::new();
    for e in &r.server_events {
        if let Event::Datagram(d) = e {
            assert_eq!(d.len(), 64);
            assert!(d[4..].iter().all(|b| *b == 7));
            seen.insert(u32::from_be_bytes(d[..4].try_into().unwrap()));
        }
    }
    assert!(seen.iter().all(|n| *n < 200));
    (took, seen.len(), handshake)
}

#[test]
fn survives_loss_duplication_and_reordering() {
    let (took, got, hs) = run_impaired(50, 0.05, 256 * 1024);
    eprintln!("5% loss, 50 ms RTT: 256 KiB in {took:?}, {got}/200 datagrams, handshake {hs:?}");
    assert!(got > 120, "datagrams are lossy but not that lossy: {got}");
}

/// The plan's network matrix (0/50/100 ms RTT, 0/1/5 % loss). Slow and noisy, so on demand:
/// `cargo test -p blockloom-net --release -- --ignored --nocapture matrix`
#[test]
#[ignore]
fn matrix() {
    eprintln!("rtt_ms loss  handshake_ms  256KiB_ms  datagrams/200");
    for rtt in [0u64, 50, 100] {
        for loss in [0.0, 0.01, 0.05] {
            let (took, got, hs) = run_impaired(rtt, loss, 256 * 1024);
            eprintln!(
                "{rtt:>6} {loss:>4.2}  {:>12.0}  {:>9.0}  {got:>6}",
                hs.as_secs_f64() * 1000.0,
                took.as_secs_f64() * 1000.0
            );
        }
    }
}

#[test]
fn fingerprint_round_trips() {
    let f = identity().fingerprint();
    let text = f.to_string();
    assert_eq!(text.len(), 64);
    assert_eq!(text.parse::<Fingerprint>().unwrap(), f);
    let colons: String = text
        .as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(":");
    assert_eq!(colons.parse::<Fingerprint>().unwrap(), f);
    assert!("zz".parse::<Fingerprint>().is_err());
}
