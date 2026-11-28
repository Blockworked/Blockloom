//! Opt-in LAN announcement: an open game broadcasts its invite once a
//! second, and the same build elsewhere lists the games it hears. Anyone on
//! the network can read an announced invite, so it is the host's choice.

use crate::game::Invite;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// The port announcements go to.
pub const PORT: u16 = 47777;
const MAGIC: &str = "blockloom-announce/1";
const EVERY: Duration = Duration::from_secs(1);
/// A game unheard for this long has gone.
const FORGET: Duration = Duration::from_secs(4);
const MAX_NAME: usize = 64;
const MAX_PACKET: usize = 1024;

/// This machine's address on the local network: the source address the
/// system would use to reach a private range. Nothing is sent.
pub fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    for probe in ["192.168.255.255:9", "10.255.255.255:9", "172.31.255.255:9"] {
        if socket.connect(probe).is_ok()
            && let Ok(local) = socket.local_addr()
            && !local.ip().is_unspecified()
        {
            return Some(local.ip());
        }
    }
    None
}

fn packet(name: &str, invite: &Invite) -> Vec<u8> {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .scan(0, |len, c| {
            *len += c.len_utf8();
            (*len <= MAX_NAME).then_some(c)
        })
        .collect();
    format!("{MAGIC}\n{name}\n{}", invite.encode()).into_bytes()
}

fn parse(bytes: &[u8]) -> Option<(String, Invite)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.splitn(3, '\n');
    if lines.next()? != MAGIC {
        return None;
    }
    let name = lines.next()?;
    if name.len() > MAX_NAME {
        return None;
    }
    let invite = Invite::decode(lines.next()?).ok()?;
    Some((name.to_string(), invite))
}

/// Sends one open game's announcement.
pub struct Announcer {
    socket: UdpSocket,
    packet: Vec<u8>,
    targets: Vec<SocketAddr>,
    last: Option<Instant>,
}

impl Announcer {
    /// Announces to the whole local network.
    pub fn new(name: &str, invite: &Invite) -> Result<Self, String> {
        Self::to(
            name,
            invite,
            vec![SocketAddr::from((Ipv4Addr::BROADCAST, PORT))],
        )
    }

    /// Announces to `targets` only (tests use loopback).
    pub fn to(name: &str, invite: &Invite, targets: Vec<SocketAddr>) -> Result<Self, String> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
        socket.set_broadcast(true).map_err(|e| e.to_string())?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self {
            socket,
            packet: packet(name, invite),
            targets,
            last: None,
        })
    }

    /// Sends when a second has passed. A failed send (no network) is quiet.
    pub fn tick(&mut self) {
        if self.last.is_some_and(|at| at.elapsed() < EVERY) {
            return;
        }
        self.last = Some(Instant::now());
        for target in &self.targets {
            let _ = self.socket.send_to(&self.packet, target);
        }
    }
}

/// One game heard on the network.
#[derive(Clone)]
pub struct Heard {
    pub name: String,
    pub invite: Invite,
}

/// Lists the games of one build that announce themselves.
pub struct Listener {
    socket: UdpSocket,
    build: [u8; 32],
    heard: BTreeMap<SocketAddr, (Heard, Instant)>,
}

impl Listener {
    pub fn new(build: [u8; 32]) -> Result<Self, String> {
        Self::on(build, PORT)
    }

    /// Listens on `port`, shared with every other listener on the machine.
    pub fn on(build: [u8; 32], port: u16) -> Result<Self, String> {
        use socket2::{Domain, Protocol, Socket, Type};
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| e.to_string())?;
        socket.set_reuse_address(true).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        socket.set_reuse_port(true).map_err(|e| e.to_string())?;
        socket.set_broadcast(true).map_err(|e| e.to_string())?;
        socket
            .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())
            .map_err(|e| format!("Couldn't listen for LAN games on port {port}: {e}"))?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self {
            socket: socket.into(),
            build,
            heard: BTreeMap::new(),
        })
    }

    /// Reads what arrived and forgets games gone quiet.
    pub fn poll(&mut self) {
        let mut buf = [0u8; MAX_PACKET];
        // Bounded, so a flood can't hold the frame.
        for _ in 0..64 {
            let Ok((n, _)) = self.socket.recv_from(&mut buf) else {
                break;
            };
            if let Some((name, invite)) = parse(&buf[..n])
                && invite.build == self.build
            {
                self.heard
                    .insert(invite.address, (Heard { name, invite }, Instant::now()));
            }
        }
        self.heard.retain(|_, (_, at)| at.elapsed() < FORGET);
    }

    /// The games heard lately, by name and then address.
    pub fn games(&self) -> Vec<Heard> {
        let mut games: Vec<Heard> = self.heard.values().map(|(h, _)| h.clone()).collect();
        games.sort_by(|a, b| {
            (a.name.as_str(), a.invite.address).cmp(&(b.name.as_str(), b.invite.address))
        });
        games
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(port: u16, build: u8) -> Invite {
        Invite::decode(&format!(
            "blockloom-lan/1|127.0.0.1:{port}|{}|{}|{}",
            "ab".repeat(32),
            "cd".repeat(32),
            format!("{build:02x}").repeat(32)
        ))
        .unwrap()
    }

    #[test]
    fn a_listener_hears_its_own_build_only() {
        let port = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut listener = Listener::on([1; 32], port).unwrap();
        let target = vec![SocketAddr::from((Ipv4Addr::LOCALHOST, port))];
        let mut ours = Announcer::to("Pond\nGame", &invite(7000, 1), target.clone()).unwrap();
        let mut other = Announcer::to("Other", &invite(7001, 2), target).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while listener.games().is_empty() {
            ours.tick();
            other.tick();
            listener.poll();
            assert!(Instant::now() < end, "never heard the announcement");
            std::thread::sleep(Duration::from_millis(5));
        }
        let games = listener.games();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "PondGame");
        assert_eq!(games[0].invite.address.port(), 7000);
        assert!(parse(b"blockloom-announce/1\nx\nnot an invite").is_none());
        assert!(parse(&packet(&"n".repeat(200), &invite(1, 1))).is_some());
    }
}
