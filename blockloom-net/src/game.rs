//! Bounded LAN spectator sessions over the pinned native transport.

use crate::{Client, ClientOptions, Event, Fingerprint, Identity, PeerId, Server, ServerOptions};
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SCHEMA: u32 = 1;
pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_ACTORS: usize = 1024;
const STREAM: u64 = 0;
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub struct ActorState {
    pub id: String,
    pub template: String,
    /// Translation, quaternion (xyzw), scale, linear and angular velocity.
    pub pose: [f32; 16],
    pub visible: bool,
    /// Host-provided presentation metadata; never executable guest code.
    pub appearance: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub epoch: u64,
    pub tick: u64,
    pub game_ns: u64,
    pub paused: bool,
    pub dimension: u8,
    pub scene: String,
    pub actors: BTreeMap<String, ActorState>,
}

#[derive(Clone)]
pub struct Invite {
    pub address: SocketAddr,
    pub fingerprint: Fingerprint,
    pub token: [u8; 32],
    pub build: [u8; 32],
}

impl Invite {
    pub fn encode(&self) -> String {
        format!(
            "blockloom-lan/1|{}|{}|{}|{}",
            self.address,
            self.fingerprint,
            hex(&self.token),
            hex(&self.build)
        )
    }
    pub fn decode(s: &str) -> Result<Self, String> {
        let p: Vec<_> = s.split('|').collect();
        if p.len() != 5 || p[0] != "blockloom-lan/1" {
            return Err("Invalid LAN invite".into());
        }
        Ok(Self {
            address: p[1].parse().map_err(|_| "Invalid LAN address")?,
            fingerprint: p[2]
                .parse()
                .map_err(|_| "Invalid certificate fingerprint")?,
            token: unhex(p[3])?,
            build: unhex(p[4])?,
        })
    }
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Result<[u8; 32], String> {
    if s.len() != 64 || !s.is_ascii() {
        return Err("Expected 64 hex digits".into());
    }
    let mut b = [0; 32];
    for (i, v) in b.iter_mut().enumerate() {
        *v = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "Invalid hex")?;
    }
    Ok(b)
}

/// Includes content bytes and the caller's engine compatibility identifier.
pub fn build_hash(content: &[u8]) -> [u8; 32] {
    ring::digest::digest(&ring::digest::SHA256, content)
        .as_ref()
        .try_into()
        .unwrap()
}

#[derive(Default)]
struct Framer {
    pending: Vec<u8>,
}
impl Framer {
    fn feed(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        if self.pending.len() + data.len() > MAX_FRAME + 4 {
            return Err("Receive buffer exceeded".into());
        }
        self.pending.extend_from_slice(data);
        let mut frames = vec![];
        let mut at = 0;
        while self.pending.len() - at >= 4 {
            let len = u32::from_le_bytes(self.pending[at..at + 4].try_into().unwrap()) as usize;
            if len == 0 || len > MAX_FRAME {
                return Err("Invalid frame length".into());
            }
            if self.pending.len() - at < len + 4 {
                break;
            }
            frames.push(self.pending[at + 4..at + 4 + len].to_vec());
            at += 4 + len;
        }
        self.pending.drain(..at);
        Ok(frames)
    }
}
fn framed(body: Vec<u8>) -> Result<Vec<u8>, String> {
    if body.len() > MAX_FRAME {
        return Err("Snapshot exceeds 1 MiB limit".into());
    }
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend(body);
    Ok(out)
}
fn string(out: &mut Vec<u8>, s: &str) -> Result<(), String> {
    if s.len() > 256 {
        return Err("Identifier exceeds 256 bytes".into());
    }
    out.extend((s.len() as u16).to_le_bytes());
    out.extend(s.as_bytes());
    Ok(())
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("Truncated frame".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }
    fn flag(&mut self) -> Result<bool, String> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("Invalid flag".into()),
        }
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<String, String> {
        let n = u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()) as usize;
        if n > 256 {
            return Err("Identifier too long".into());
        }
        String::from_utf8(self.bytes(n)?.to_vec()).map_err(|_| "Invalid UTF-8".into())
    }
    fn done(&self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err("Trailing bytes".into())
        }
    }
}

fn state_frame(
    seq: u64,
    base: Option<(u64, &Snapshot)>,
    next: &Snapshot,
) -> Result<Vec<u8>, String> {
    if next.actors.len() > MAX_ACTORS || next.dimension > 1 {
        return Err("Unsupported snapshot size or dimension".into());
    }
    let mut out = vec![2];
    out.extend(seq.to_le_bytes());
    out.extend(base.map_or(0, |b| b.0).to_le_bytes());
    for n in [next.epoch, next.tick, next.game_ns] {
        out.extend(n.to_le_bytes());
    }
    out.push(u8::from(next.paused));
    out.push(next.dimension);
    string(&mut out, &next.scene)?;
    let changed: Vec<_> = next
        .actors
        .values()
        .filter(|a| base.is_none_or(|(_, s)| s.actors.get(&a.id) != Some(a)))
        .collect();
    out.extend((changed.len() as u32).to_le_bytes());
    for a in changed {
        if next.actors.get(&a.id) != Some(a) || a.id.is_empty() {
            return Err("Invalid actor identity".into());
        }
        string(&mut out, &a.id)?;
        string(&mut out, &a.template)?;
        out.push(u8::from(a.visible));
        for n in a.pose {
            if !n.is_finite() {
                return Err("Non-finite actor pose".into());
            }
            out.extend(n.to_le_bytes());
        }
        if a.appearance.len() > 32768 {
            return Err("Appearance too large".into());
        }
        out.extend((a.appearance.len() as u32).to_le_bytes());
        out.extend(&a.appearance);
    }
    let removed: Vec<_> = base
        .map(|(_, s)| {
            s.actors
                .keys()
                .filter(|id| !next.actors.contains_key(*id))
                .collect()
        })
        .unwrap_or_default();
    out.extend((removed.len() as u32).to_le_bytes());
    for id in removed {
        string(&mut out, id)?;
    }
    framed(out)
}

fn apply_state(body: &[u8], old_seq: u64, old: &Snapshot) -> Result<(u64, Snapshot), String> {
    let mut r = Reader(body);
    if r.byte()? != 2 {
        return Err("Expected state frame".into());
    }
    let seq = r.u64()?;
    let base = r.u64()?;
    if seq <= old_seq || (base != 0 && base != old_seq) {
        return Err("Invalid state sequence".into());
    }
    let epoch = r.u64()?;
    let tick = r.u64()?;
    let game_ns = r.u64()?;
    let paused = r.flag()?;
    let dimension = r.byte()?;
    if dimension > 1 || (base != 0 && (epoch != old.epoch || tick < old.tick)) || epoch < old.epoch
    {
        return Err("Invalid scene epoch".into());
    }
    let scene = r.text()?;
    if old_seq != 0 && epoch == old.epoch && (scene != old.scene || dimension != old.dimension) {
        return Err("Scene changes require a new epoch".into());
    }
    let mut actors = if base == 0 {
        BTreeMap::new()
    } else {
        old.actors.clone()
    };
    let n = r.u32()? as usize;
    if n > MAX_ACTORS {
        return Err("Too many actors".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..n {
        let id = r.text()?;
        if id.is_empty() || !ids.insert(id.clone()) {
            return Err("Duplicate actor".into());
        }
        let template = r.text()?;
        let visible = r.flag()?;
        let mut pose = [0.; 16];
        for v in &mut pose {
            *v = f32::from_le_bytes(r.bytes(4)?.try_into().unwrap());
            if !v.is_finite() {
                return Err("Invalid pose".into());
            }
        }
        let len = r.u32()? as usize;
        if len > 32768 {
            return Err("Appearance too large".into());
        }
        let appearance = r.bytes(len)?.to_vec();
        actors.insert(
            id.clone(),
            ActorState {
                id,
                template,
                pose,
                visible,
                appearance,
            },
        );
    }
    let n = r.u32()? as usize;
    if n > MAX_ACTORS {
        return Err("Too many removals".into());
    }
    for _ in 0..n {
        let id = r.text()?;
        if ids.contains(&id) || actors.remove(&id).is_none() {
            return Err("Invalid removal".into());
        }
    }
    r.done()?;
    if actors.len() > MAX_ACTORS {
        return Err("Too many actors".into());
    }
    Ok((
        seq,
        Snapshot {
            epoch,
            tick,
            game_ns,
            paused,
            dimension,
            scene,
            actors,
        },
    ))
}

struct Guest {
    frames: Framer,
    admitted: bool,
    acked: Option<(u64, Arc<Snapshot>)>,
    flight: Option<(u64, Arc<Snapshot>)>,
    deadline: Instant,
}

/// Attaches to a running authority; publishing state never mutates the world.
pub struct LanHost {
    server: Server,
    invite: Invite,
    guests: BTreeMap<PeerId, Guest>,
    latest: Arc<Snapshot>,
    seq: u64,
    closing: bool,
}
impl LanHost {
    pub fn open(
        bind: SocketAddr,
        build: [u8; 32],
        initial: Snapshot,
        opts: ServerOptions,
    ) -> Result<Self, String> {
        if bind.ip().is_unspecified() || bind.ip().is_multicast() {
            return Err("Select a specific LAN interface address".into());
        }
        if !(1..=16).contains(&opts.max_peers) {
            return Err("Guest limit must be 1 to 16".into());
        }
        state_frame(1, None, &initial)?;
        let id = Identity::generate(&["blockloom-lan"]).map_err(|e| e.to_string())?;
        let server = Server::bind(bind, &id, opts).map_err(|e| e.to_string())?;
        let mut token = [0; 32];
        SystemRandom::new()
            .fill(&mut token)
            .map_err(|_| "Randomness unavailable")?;
        let invite = Invite {
            address: server.local_addr(),
            fingerprint: id.fingerprint(),
            token,
            build,
        };
        Ok(Self {
            server,
            invite,
            guests: BTreeMap::new(),
            latest: Arc::new(initial),
            seq: 1,
            closing: false,
        })
    }
    pub fn invite(&self) -> &Invite {
        &self.invite
    }
    pub fn guests(&self) -> Vec<u64> {
        self.guests
            .iter()
            .filter(|(_, g)| g.admitted)
            .map(|(id, _)| id.0)
            .collect()
    }
    pub fn publish(&mut self, state: Snapshot) -> Result<(), String> {
        if state.epoch < self.latest.epoch
            || (state.epoch == self.latest.epoch && state.tick < self.latest.tick)
        {
            return Err("Authority moved backwards".into());
        }
        if state.epoch == self.latest.epoch
            && (state.scene != self.latest.scene || state.dimension != self.latest.dimension)
        {
            return Err("Scene changes require a new epoch".into());
        }
        state_frame(1, None, &state)?;
        if *self.latest != state {
            self.seq = self.seq.checked_add(1).ok_or("Sequence exhausted")?;
            self.latest = Arc::new(state);
        }
        Ok(())
    }
    pub fn kick(&mut self, id: u64) {
        self.server.kick(PeerId(id), "Guest removed");
        self.guests.remove(&PeerId(id));
    }
    pub fn close(&mut self) {
        self.closing = true;
        for id in self.guests.keys() {
            self.server.kick(*id, "LAN closed");
        }
        self.guests.clear();
        self.server.poll();
    }
    pub fn is_drained(&self) -> bool {
        self.server.peer_count() == 0
    }
    pub fn poll(&mut self) {
        for (id, event) in self.server.poll() {
            let result = match event {
                Event::Established if !self.closing => {
                    self.guests.insert(
                        id,
                        Guest {
                            frames: Framer::default(),
                            admitted: false,
                            acked: None,
                            flight: None,
                            deadline: Instant::now() + TIMEOUT,
                        },
                    );
                    Ok(())
                }
                Event::Closed { .. } => {
                    self.guests.remove(&id);
                    Ok(())
                }
                Event::Stream {
                    stream: STREAM,
                    data,
                    fin: false,
                } if !self.closing => self.receive(id, &data),
                _ => Err("Unexpected guest traffic".into()),
            };
            if let Err(e) = result {
                self.server.kick(id, &e);
                self.guests.remove(&id);
            }
        }
        let mut failed = vec![];
        for (id, g) in &mut self.guests {
            if Instant::now() > g.deadline {
                failed.push(*id);
                continue;
            }
            if !g.admitted
                || g.flight.is_some()
                || g.acked.as_ref().is_some_and(|(seq, _)| *seq == self.seq)
            {
                continue;
            }
            let base = g
                .acked
                .as_ref()
                .filter(|(_, s)| s.epoch == self.latest.epoch)
                .map(|(seq, s)| (*seq, s.as_ref()));
            let bytes = state_frame(self.seq, base, &self.latest).expect("validated state");
            if self.server.send_stream(*id, STREAM, &bytes, false).is_err() {
                failed.push(*id);
                continue;
            }
            g.flight = Some((self.seq, self.latest.clone()));
            g.deadline = Instant::now() + TIMEOUT;
        }
        for id in failed {
            self.server.kick(id, "Guest stalled");
            self.guests.remove(&id);
        }
    }
    fn receive(&mut self, id: PeerId, data: &[u8]) -> Result<(), String> {
        let g = self.guests.get_mut(&id).ok_or("Unknown guest")?;
        for frame in g.frames.feed(data)? {
            let mut r = Reader(&frame);
            match r.byte()? {
                1 if !g.admitted => {
                    if r.u32()? != SCHEMA || r.bytes(32)? != self.invite.build {
                        return Err("Incompatible game build".into());
                    }
                    let supplied = r.bytes(32)?;
                    let key = hmac::Key::new(hmac::HMAC_SHA256, &self.invite.token);
                    let tag = hmac::sign(&key, supplied);
                    hmac::verify(&key, &self.invite.token, tag.as_ref())
                        .map_err(|_| "Admission refused")?;
                    r.done()?;
                    g.admitted = true;
                }
                3 if g.admitted => {
                    let seq = r.u64()?;
                    r.done()?;
                    let flight = g.flight.take().ok_or("Unexpected acknowledgement")?;
                    if seq != flight.0 {
                        return Err("Invalid acknowledgement".into());
                    }
                    g.acked = Some(flight);
                }
                4 if g.admitted => {
                    r.done()?;
                }
                _ => return Err("Unexpected guest message".into()),
            }
            if g.flight.is_none() {
                g.deadline = Instant::now() + TIMEOUT;
            }
        }
        Ok(())
    }
}

/// A read-only replica. It applies each update atomically before acknowledging.
pub struct LanClient {
    client: Client,
    invite: Invite,
    frames: Framer,
    state: Snapshot,
    seq: u64,
    heartbeat: Instant,
}
impl LanClient {
    pub fn connect(
        invite: Invite,
        trusted_build: [u8; 32],
        opts: ClientOptions,
    ) -> Result<Self, String> {
        if invite.build != trusted_build {
            return Err("Invite does not match the installed game build".into());
        }
        let client =
            Client::connect(invite.address, invite.fingerprint, opts).map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            invite,
            frames: Framer::default(),
            state: Snapshot::default(),
            seq: 0,
            heartbeat: Instant::now(),
        })
    }
    pub fn state(&self) -> Option<&Snapshot> {
        (self.seq != 0).then_some(&self.state)
    }
    pub fn sequence(&self) -> u64 {
        self.seq
    }
    pub fn poll(&mut self) -> Result<bool, String> {
        let mut changed = false;
        for event in self.client.poll() {
            match event {
                Event::Established => {
                    let mut b = vec![1];
                    b.extend(SCHEMA.to_le_bytes());
                    b.extend(self.invite.build);
                    b.extend(self.invite.token);
                    self.client
                        .send_stream(STREAM, &framed(b)?, false)
                        .map_err(|e| e.to_string())?;
                }
                Event::Stream {
                    stream: STREAM,
                    data,
                    fin: false,
                } => {
                    for body in self.frames.feed(&data)? {
                        let (seq, state) = apply_state(&body, self.seq, &self.state)?;
                        self.seq = seq;
                        self.state = state;
                        changed = true;
                        let mut b = vec![3];
                        b.extend(seq.to_le_bytes());
                        self.client
                            .send_stream(STREAM, &framed(b)?, false)
                            .map_err(|e| e.to_string())?;
                    }
                }
                Event::Closed { reason } => return Err(reason),
                _ => return Err("Unexpected host traffic".into()),
            }
        }
        if self.client.is_established() && self.heartbeat.elapsed() > Duration::from_secs(2) {
            self.client
                .send_stream(STREAM, &framed(vec![4])?, false)
                .map_err(|e| e.to_string())?;
            self.heartbeat = Instant::now();
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Snapshot {
        let a = ActorState {
            id: "player".into(),
            template: "player".into(),
            pose: [0.; 16],
            visible: true,
            appearance: b"{}".to_vec(),
        };
        Snapshot {
            epoch: 1,
            tick: 1,
            scene: "First".into(),
            actors: BTreeMap::from([(a.id.clone(), a)]),
            ..Default::default()
        }
    }
    #[test]
    fn fragmented_frames_and_acknowledged_deltas_apply_atomically() {
        let a = sample();
        let bytes = state_frame(1, None, &a).unwrap();
        let mut f = Framer::default();
        let mut frames = vec![];
        for chunk in bytes.chunks(3) {
            frames.extend(f.feed(chunk).unwrap());
        }
        let (_, baseline) = apply_state(&frames[0], 0, &Snapshot::default()).unwrap();
        assert_eq!(baseline, a);
        let mut b = a.clone();
        b.tick = 2;
        b.actors.clear();
        let bytes = state_frame(2, Some((1, &a)), &b).unwrap();
        let (_, next) = apply_state(&bytes[4..], 1, &baseline).unwrap();
        assert_eq!(next, b);
        assert!(apply_state(&bytes[4..], 0, &baseline).is_err());
        assert_eq!(baseline, a);
    }
    #[test]
    fn malformed_and_oversized_state_is_refused() {
        assert!(
            Framer::default()
                .feed(&((MAX_FRAME + 1) as u32).to_le_bytes())
                .is_err()
        );
        let mut s = sample();
        s.actors.get_mut("player").unwrap().pose[0] = f32::NAN;
        assert!(state_frame(1, None, &s).is_err());
        let s = sample();
        let bytes = state_frame(1, None, &s).unwrap();
        assert!(apply_state(&bytes[4..bytes.len() - 1], 0, &Snapshot::default()).is_err());
        let mut extra = bytes[4..].to_vec();
        extra.push(0);
        assert!(apply_state(&extra, 0, &Snapshot::default()).is_err());
    }
    #[test]
    fn new_epoch_requires_a_full_baseline() {
        let a = sample();
        let mut b = a.clone();
        b.epoch = 2;
        b.tick = 0;
        b.scene = "Next".into();
        let delta = state_frame(2, Some((1, &a)), &b).unwrap();
        assert!(apply_state(&delta[4..], 1, &a).is_err());
        let full = state_frame(2, None, &b).unwrap();
        assert_eq!(apply_state(&full[4..], 1, &a).unwrap().1, b);
    }
}
