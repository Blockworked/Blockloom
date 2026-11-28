//! Bounded LAN sessions over the pinned native transport: guests get the
//! host's state and a player seat, and send that seat's input back.

use crate::{Client, ClientOptions, Event, Fingerprint, Identity, PeerId, Server, ServerOptions};
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SCHEMA: u32 = 4;
/// Seats guests can take; seat 0 is the host's own player.
pub const MAX_SEAT: u8 = 16;
/// Actions one input frame may carry, and how long each name may be.
pub const MAX_INPUTS: usize = 64;
pub const MAX_INPUT_NAME: usize = 64;
/// Largest analog magnitude accepted (mouse deltas run past 1).
pub const MAX_INPUT_VALUE: f32 = 10_000.0;
pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_ACTORS: usize = 1024;
/// Plugin state records (a voxel section each) a snapshot may carry.
pub const MAX_REGIONS: usize = 16384;
pub const MAX_REGION: usize = 384 * 1024;
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

/// One plugin state record, such as a voxel section, keyed `plugin/key`.
/// A guest gets a record again only when its revision or bytes change.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub revision: u64,
    pub data: Arc<[u8]>,
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
    pub regions: BTreeMap<String, Region>,
}

/// One action as a guest's own devices read it, by its base name (`Jump`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionInput {
    pub held: bool,
    pub value: f32,
    pub vector: [f32; 2],
}

/// Where a guest's camera looks, in radians about +Y: motors that move with
/// the camera, or turn the body with it, follow the guest's view, not the host's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewInput {
    pub yaw: Option<f32>,
    pub facing: Option<f32>,
}

/// A guest taking or leaving a seat, in the order it happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatChange {
    Joined(u8),
    Left(u8),
}

fn input_frame(
    seq: u64,
    input: &BTreeMap<String, ActionInput>,
    view: ViewInput,
) -> Result<Vec<u8>, String> {
    if input.len() > MAX_INPUTS {
        return Err("Too many input actions".into());
    }
    let mut out = vec![5];
    out.extend(seq.to_le_bytes());
    out.extend((input.len() as u16).to_le_bytes());
    for (name, a) in input {
        if name.is_empty() || name.len() > MAX_INPUT_NAME {
            return Err("Invalid input action name".into());
        }
        string(&mut out, name)?;
        out.push(u8::from(a.held));
        for v in [a.value, a.vector[0], a.vector[1]] {
            if !v.is_finite() || v.abs() > MAX_INPUT_VALUE {
                return Err("Invalid input value".into());
            }
            out.extend(v.to_le_bytes());
        }
    }
    for v in [view.yaw, view.facing] {
        match v {
            Some(v) if v.is_finite() => {
                out.push(1);
                out.extend(v.to_le_bytes());
            }
            Some(_) => return Err("Invalid view angle".into()),
            None => out.push(0),
        }
    }
    framed(out)
}

fn read_angle(r: &mut Reader) -> Result<Option<f32>, String> {
    if !r.flag()? {
        return Ok(None);
    }
    let v = f32::from_le_bytes(r.bytes(4)?.try_into().unwrap());
    if !v.is_finite() {
        return Err("Invalid view angle".into());
    }
    Ok(Some(v.rem_euclid(std::f32::consts::TAU)))
}

type ReadInput = (u64, BTreeMap<String, ActionInput>, ViewInput);

fn read_input(r: &mut Reader) -> Result<ReadInput, String> {
    let seq = r.u64()?;
    let n = u16::from_le_bytes(r.bytes(2)?.try_into().unwrap()) as usize;
    if n > MAX_INPUTS {
        return Err("Too many input actions".into());
    }
    let mut input = BTreeMap::new();
    for _ in 0..n {
        let name = r.text()?;
        if name.is_empty() || name.len() > MAX_INPUT_NAME {
            return Err("Invalid input action name".into());
        }
        let held = r.flag()?;
        let mut v = [0f32; 3];
        for x in &mut v {
            *x = f32::from_le_bytes(r.bytes(4)?.try_into().unwrap());
            if !x.is_finite() || x.abs() > MAX_INPUT_VALUE {
                return Err("Invalid input value".into());
            }
        }
        let action = ActionInput {
            held,
            value: v[0],
            vector: [v[1], v[2]],
        };
        if input.insert(name, action).is_some() {
            return Err("Duplicate input action".into());
        }
    }
    let view = ViewInput {
        yaw: read_angle(r)?,
        facing: read_angle(r)?,
    };
    r.done()?;
    Ok((seq, input, view))
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
    let base_regions = base.map(|(_, s)| &s.regions);
    let changed: Vec<_> = next
        .regions
        .iter()
        .filter(|(k, r)| base_regions.is_none_or(|b| b.get(*k) != Some(r)))
        .collect();
    if next.regions.len() > MAX_REGIONS {
        return Err("Too many plugin regions".into());
    }
    out.extend((changed.len() as u32).to_le_bytes());
    for (key, region) in changed {
        if key.is_empty() || region.data.len() > MAX_REGION {
            return Err("Invalid plugin region".into());
        }
        string(&mut out, key)?;
        out.extend(region.revision.to_le_bytes());
        out.extend((region.data.len() as u32).to_le_bytes());
        out.extend(region.data.iter());
    }
    let removed: Vec<_> = base_regions
        .map(|b| {
            b.keys()
                .filter(|k| !next.regions.contains_key(*k))
                .collect()
        })
        .unwrap_or_default();
    out.extend((removed.len() as u32).to_le_bytes());
    for key in removed {
        string(&mut out, key)?;
    }
    framed(out)
}

/// What a guest gets next from `base`: `latest`, or as many of its changed
/// regions as fit `budget` bytes when they don't all fit one frame. Later
/// frames carry the rest.
fn stage(base: Option<&Snapshot>, latest: &Arc<Snapshot>, budget: usize) -> Arc<Snapshot> {
    let mut regions = base.map(|b| b.regions.clone()).unwrap_or_default();
    regions.retain(|k, _| latest.regions.contains_key(k));
    let mut used = 0;
    for (key, region) in &latest.regions {
        if regions.get(key) == Some(region) {
            continue;
        }
        let cost = key.len() + region.data.len() + 16;
        if used > 0 && used + cost > budget {
            return Arc::new(Snapshot {
                regions,
                ..(**latest).clone()
            });
        }
        used += cost;
        regions.insert(key.clone(), region.clone());
    }
    latest.clone()
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
    let mut regions = if base == 0 {
        BTreeMap::new()
    } else {
        old.regions.clone()
    };
    let n = r.u32()? as usize;
    if n > MAX_REGIONS {
        return Err("Too many plugin regions".into());
    }
    let mut keys = std::collections::BTreeSet::new();
    for _ in 0..n {
        let key = r.text()?;
        if key.is_empty() || !keys.insert(key.clone()) {
            return Err("Duplicate plugin region".into());
        }
        let revision = r.u64()?;
        let len = r.u32()? as usize;
        if len > MAX_REGION {
            return Err("Plugin region too large".into());
        }
        let data: Arc<[u8]> = r.bytes(len)?.into();
        regions.insert(key, Region { revision, data });
    }
    let n = r.u32()? as usize;
    if n > MAX_REGIONS {
        return Err("Too many region removals".into());
    }
    for _ in 0..n {
        let key = r.text()?;
        if keys.contains(&key) || regions.remove(&key).is_none() {
            return Err("Invalid region removal".into());
        }
    }
    r.done()?;
    if regions.len() > MAX_REGIONS {
        return Err("Too many plugin regions".into());
    }
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
            regions,
        },
    ))
}

struct Guest {
    frames: Framer,
    admitted: bool,
    /// Frames sent to this guest, which number its sequence.
    sent: u64,
    acked: Option<(u64, Arc<Snapshot>)>,
    flight: Option<(u64, Arc<Snapshot>)>,
    deadline: Instant,
    seat: Option<u8>,
    /// The newest input frame's sequence and actions.
    input_seq: u64,
    input: BTreeMap<String, ActionInput>,
    view: ViewInput,
}

/// Attaches to a running authority; publishing state never mutates the world.
pub struct LanHost {
    server: Server,
    invite: Invite,
    guests: BTreeMap<PeerId, Guest>,
    latest: Arc<Snapshot>,
    /// Bytes of regions a frame may carry beside the actors.
    region_budget: usize,
    closing: bool,
    changes: Vec<SeatChange>,
}

/// Checks a state the host is about to publish; answers what a frame may
/// spend on regions beside its actors.
fn check(state: &Snapshot) -> Result<usize, String> {
    let actors = state_frame(
        1,
        None,
        &Snapshot {
            regions: BTreeMap::new(),
            ..state.clone()
        },
    )?;
    if state.regions.len() > MAX_REGIONS {
        return Err("Too many plugin regions".into());
    }
    let largest = state.regions.iter().map(|(k, r)| {
        if k.is_empty() || k.len() > 256 || r.data.len() > MAX_REGION {
            None
        } else {
            Some(k.len() + r.data.len() + 16)
        }
    });
    let mut most = 0;
    for cost in largest {
        most = most.max(cost.ok_or("Invalid plugin region")?);
    }
    let budget = MAX_FRAME.saturating_sub(actors.len() + 64);
    if most > budget {
        return Err("Actors leave no room for a plugin region in one frame".into());
    }
    Ok(budget)
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
        let region_budget = check(&initial)?;
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
            region_budget,
            closing: false,
            changes: Vec::new(),
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
    /// Each seated guest's id and seat.
    pub fn seats(&self) -> Vec<(u64, u8)> {
        self.guests
            .iter()
            .filter_map(|(id, g)| Some((id.0, g.seat?)))
            .collect()
    }
    /// The latest actions the guest in `seat` sent, empty until it sends any.
    pub fn input(&self, seat: u8) -> Option<&BTreeMap<String, ActionInput>> {
        self.guests
            .values()
            .find(|g| g.seat == Some(seat))
            .map(|g| &g.input)
    }
    /// Where a seat's guest last said its camera looks.
    pub fn view(&self, seat: u8) -> Option<ViewInput> {
        self.guests
            .values()
            .find(|g| g.seat == Some(seat))
            .map(|g| g.view)
    }
    /// Sends every seated guest one tick's effects, opaque to this layer, in
    /// order on the reliable stream. A guest that can't keep up is dropped.
    pub fn publish_effects(&mut self, tick: u64, effects: &[u8]) -> Result<(), String> {
        let mut body = vec![7];
        body.extend(tick.to_le_bytes());
        body.extend(effects);
        let bytes = framed(body)?;
        let seated: Vec<PeerId> = self
            .guests
            .iter()
            .filter(|(_, g)| g.seat.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in seated {
            if self.server.send_stream(id, STREAM, &bytes, false).is_err() {
                self.server.kick(id, "Guest fell behind");
                self.drop_guest(id);
            }
        }
        Ok(())
    }
    /// Seats taken and left since the last call.
    pub fn take_seat_changes(&mut self) -> Vec<SeatChange> {
        std::mem::take(&mut self.changes)
    }
    fn drop_guest(&mut self, id: PeerId) {
        if let Some(seat) = self.guests.remove(&id).and_then(|g| g.seat) {
            self.changes.push(SeatChange::Left(seat));
        }
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
        let budget = check(&state)?;
        if *self.latest != state {
            self.region_budget = budget;
            self.latest = Arc::new(state);
        }
        Ok(())
    }
    pub fn kick(&mut self, id: u64) {
        self.server.kick(PeerId(id), "Guest removed");
        self.drop_guest(PeerId(id));
    }
    pub fn close(&mut self) {
        self.closing = true;
        let ids: Vec<_> = self.guests.keys().copied().collect();
        for id in ids {
            self.server.kick(id, "LAN closed");
            self.drop_guest(id);
        }
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
                            sent: 0,
                            acked: None,
                            flight: None,
                            deadline: Instant::now() + TIMEOUT,
                            seat: None,
                            input_seq: 0,
                            input: BTreeMap::new(),
                            view: ViewInput::default(),
                        },
                    );
                    Ok(())
                }
                Event::Closed { .. } => {
                    self.drop_guest(id);
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
                self.drop_guest(id);
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
                || g.acked
                    .as_ref()
                    .is_some_and(|(_, s)| Arc::ptr_eq(s, &self.latest))
            {
                continue;
            }
            let base = g
                .acked
                .as_ref()
                .filter(|(_, s)| s.epoch == self.latest.epoch)
                .map(|(seq, s)| (*seq, s.as_ref()));
            let target = stage(base.map(|b| b.1), &self.latest, self.region_budget);
            let seq = g.sent + 1;
            let bytes = state_frame(seq, base, &target).expect("validated state");
            if self.server.send_stream(*id, STREAM, &bytes, false).is_err() {
                failed.push(*id);
                continue;
            }
            g.sent = seq;
            g.flight = Some((seq, target));
            g.deadline = Instant::now() + TIMEOUT;
        }
        for id in failed {
            self.server.kick(id, "Guest stalled");
            self.drop_guest(id);
        }
    }
    fn receive(&mut self, id: PeerId, data: &[u8]) -> Result<(), String> {
        let taken: Vec<u8> = self.guests.values().filter_map(|g| g.seat).collect();
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
                    let seat = (1..=MAX_SEAT)
                        .find(|s| !taken.contains(s))
                        .ok_or("No free player seat")?;
                    g.admitted = true;
                    g.seat = Some(seat);
                    self.server
                        .send_stream(id, STREAM, &framed(vec![6, seat])?, false)
                        .map_err(|e| e.to_string())?;
                    self.changes.push(SeatChange::Joined(seat));
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
                5 if g.admitted => {
                    let (seq, input, view) = read_input(&mut r)?;
                    // Reordering can't happen on one stream; an old one is a replay.
                    if seq <= g.input_seq {
                        return Err("Stale input".into());
                    }
                    g.input_seq = seq;
                    g.input = input;
                    g.view = view;
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
    seat: Option<u8>,
    input_seq: u64,
    sent_input: Option<(BTreeMap<String, ActionInput>, ViewInput, Instant)>,
    effects: Vec<(u64, Vec<u8>)>,
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
            seat: None,
            input_seq: 0,
            sent_input: None,
            effects: Vec::new(),
        })
    }
    /// Every tick's effects that arrived since the last call, oldest first.
    pub fn take_effects(&mut self) -> Vec<(u64, Vec<u8>)> {
        std::mem::take(&mut self.effects)
    }
    /// The player seat the host gave this guest, once admitted.
    pub fn seat(&self) -> Option<u8> {
        self.seat
    }
    /// Sends this guest's actions when they change, and at least every
    /// quarter second so a lost-focus window can't leave a key held.
    pub fn send_input(
        &mut self,
        input: &BTreeMap<String, ActionInput>,
        view: ViewInput,
    ) -> Result<(), String> {
        if self.seat.is_none() || !self.client.is_established() {
            return Ok(());
        }
        if let Some((last, last_view, at)) = &self.sent_input
            && last == input
            && *last_view == view
            && at.elapsed() < Duration::from_millis(250)
        {
            return Ok(());
        }
        self.input_seq += 1;
        let bytes = input_frame(self.input_seq, input, view)?;
        self.client
            .send_stream(STREAM, &bytes, false)
            .map_err(|e| e.to_string())?;
        self.sent_input = Some((input.clone(), view, Instant::now()));
        Ok(())
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
                        if body.first() == Some(&6) {
                            if body.len() != 2
                                || self.seat.is_some()
                                || !(1..=MAX_SEAT).contains(&body[1])
                            {
                                return Err("Invalid seat".into());
                            }
                            self.seat = Some(body[1]);
                            continue;
                        }
                        if body.first() == Some(&7) {
                            if body.len() < 9 || self.seat.is_none() {
                                return Err("Invalid effects frame".into());
                            }
                            let tick = u64::from_le_bytes(body[1..9].try_into().unwrap());
                            if self.effects.len() >= 4096 {
                                return Err("Too many unread effects".into());
                            }
                            self.effects.push((tick, body[9..].to_vec()));
                            continue;
                        }
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
    fn region_deltas_carry_changes_and_removals_and_refuse_bad_ones() {
        let mut a = sample();
        let region = |n: u8| Region {
            revision: u64::from(n),
            data: vec![n; 4].into(),
        };
        a.regions.insert("p/one".into(), region(1));
        a.regions.insert("p/two".into(), region(2));
        let bytes = state_frame(1, None, &a).unwrap();
        let (_, got) = apply_state(&bytes[4..], 0, &Snapshot::default()).unwrap();
        assert_eq!(got, a);
        let mut b = a.clone();
        b.regions.remove("p/one");
        b.regions.insert("p/two".into(), region(3));
        let delta = state_frame(2, Some((1, &a)), &b).unwrap();
        assert!(delta.len() < bytes.len());
        assert_eq!(apply_state(&delta[4..], 1, &got).unwrap().1, b);
        // A removal of something the guest never had is refused.
        let delta = state_frame(2, Some((1, &a)), &b).unwrap();
        assert!(apply_state(&delta[4..], 1, &sample()).is_err());
    }
    #[test]
    fn staging_splits_regions_by_budget_and_finishes_with_the_latest() {
        let mut latest = sample();
        for i in 0..5u8 {
            latest.regions.insert(
                format!("p/{i}"),
                Region {
                    revision: 1,
                    data: vec![i; 100].into(),
                },
            );
        }
        let latest = Arc::new(latest);
        let first = stage(None, &latest, 250);
        assert_eq!(first.regions.len(), 2);
        let second = stage(Some(&first), &latest, 250);
        assert_eq!(second.regions.len(), 4);
        let third = stage(Some(&second), &latest, 250);
        assert!(Arc::ptr_eq(&third, &latest));
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
