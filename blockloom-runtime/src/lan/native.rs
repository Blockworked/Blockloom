use super::*;
use crate::engine::{ActorId, Dimension, PhysicsPose};
use bevy::prelude::*;
use blockloom_net::ServerOptions;
use blockloom_net::announce::{Announcer, Heard, Listener};
use blockloom_net::game::{ActorState, LanHost, Region, SeatChange, Snapshot, build_hash};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Session {
    settings: blockloom_core::multiplayer::MultiplayerSettings,
    host: Option<LanHost>,
    draining: Vec<(LanHost, Instant)>,
    requested: Option<(std::net::SocketAddr, [u8; 32], usize)>,
    scene: String,
    epoch: u64,
    tick: u64,
    last_guests: Vec<u64>,
    /// Plugin state records (voxel sections) guests hold, keyed `plugin/key`.
    regions: BTreeMap<String, Region>,
    /// Each plugin's last `sync.sections` generation and revision.
    synced: BTreeMap<String, (u64, u64)>,
    unsynced: BTreeSet<String>,
    /// This tick's effects, for guests that follow the host as clients.
    effects: Vec<blockloom_core::vm::Effect>,
    /// Seat actions held last tick, for their pressed and released edges.
    seat_held: BTreeMap<String, bool>,
    /// This run's content hash: what a host's invite carries and a guest
    /// must match.
    build: [u8; 32],
    announcer: Option<Announcer>,
    listener: Option<Listener>,
    heard: Vec<Heard>,
}

/// How often, in fixed ticks, plugins are asked what changed.
const SYNC_EVERY: u64 = 6;

/// Folds plugins' `sync.sections` answers into the records guests get.
fn gather_regions(e: &mut Engine) {
    let mut unsynced = std::mem::take(&mut e.lan.unsynced);
    let answers = crate::plugins::sync_sections(e, &e.lan.synced, &mut unsynced);
    let lan = &mut e.lan;
    lan.unsynced = unsynced;
    for (plugin, answer) in answers {
        let generation = answer["generation"].as_u64().unwrap_or(0);
        let revision = answer["revision"].as_u64().unwrap_or(0);
        if lan.synced.get(&plugin).map(|s| s.0) != Some(generation) {
            let prefix = format!("{plugin}/");
            lan.regions.retain(|k, _| !k.starts_with(&prefix));
        }
        let identity = serde_json::to_vec(&answer["identity"]).unwrap_or_default();
        lan.regions.insert(
            format!("{plugin}/identity"),
            Region {
                revision: generation,
                data: identity.into(),
            },
        );
        for section in answer["sections"].as_array().into_iter().flatten() {
            let Some(key) = section["key"].as_str() else {
                continue;
            };
            let data = serde_json::to_vec(&section["data"]).unwrap_or_default();
            lan.regions.insert(
                format!("{plugin}/{key}"),
                Region {
                    revision: section["revision"].as_u64().unwrap_or(0),
                    data: data.into(),
                },
            );
        }
        lan.synced.insert(plugin, (generation, revision));
    }
}

pub fn status(e: &Engine) -> LanSessionStatus {
    match &e.lan.host {
        Some(h) => LanSessionStatus {
            enabled: e.lan.settings.enabled,
            max_guests: e.lan.settings.max_guests,
            open: true,
            address: Some(h.invite().address.to_string()),
            invite: Some(h.invite().encode()),
            guests: h.guests(),
            error: None,
        },
        None => LanSessionStatus {
            enabled: e.lan.settings.enabled,
            max_guests: e.lan.settings.max_guests,
            ..Default::default()
        },
    }
}

/// The player numbers seated guests hold (2 and up).
pub fn guest_players(e: &Engine) -> Vec<u8> {
    let mut players: Vec<u8> = e
        .lan
        .host
        .as_ref()
        .map(|h| h.seats().iter().map(|(_, seat)| seat + 1).collect())
        .unwrap_or_default();
    players.sort_unstable();
    players
}

/// What the guest in seat `player` holds on the action `name` (`Jump P2`
/// reads its `Jump`), or None when no guest sits there so local pads drive it.
pub fn remote_action(e: &Engine, player: u8, name: &str) -> Option<super::RemoteAction> {
    let input = e.lan.host.as_ref()?.input(player)?;
    let base = super::base_action(name, player);
    let got = input
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(base))
        .map(|(_, a)| *a)
        .unwrap_or_default();
    Some(super::RemoteAction {
        held: got.held,
        value: got.value,
        vector: got.vector,
    })
}

/// What a host and its guests must agree on: the engine and the document.
pub fn content_hash(project: &blockloom_core::project::Project) -> [u8; 32] {
    let mut content =
        format!("blockloom-{}-lan-spectator-1\n", env!("CARGO_PKG_VERSION")).into_bytes();
    content.extend(serde_json::to_vec(project).expect("a project serializes"));
    build_hash(&content)
}

/// This run's content hash as hex, the trusted build a guest joins with.
pub fn own_build(e: &Engine) -> String {
    e.lan.build.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn begin(e: &mut Engine) {
    e.lan.settings = e.project.multiplayer.clone();
    e.lan.build = content_hash(&e.project);
    e.lan.heard.clear();
    // Listening is quiet when the port can't be shared: no games are found.
    e.lan.listener = e
        .lan
        .settings
        .enabled
        .then(|| Listener::new(e.lan.build).ok())
        .flatten();
    e.lan.regions.clear();
    e.lan.synced.clear();
    e.lan.unsynced.clear();
    super::report(e, None);
}

pub fn open(e: &mut Engine, bind: &str, max_guests: usize) {
    let result = (|| {
        if !e.running {
            return Err("Start the game before opening LAN".into());
        }
        if !e.lan.settings.enabled {
            return Err("Enable multiplayer in Project settings, then start a new run".into());
        }
        if max_guests > e.lan.settings.max_guests {
            return Err(format!(
                "This run allows at most {} guests",
                e.lan.settings.max_guests
            ));
        }
        if e.lan.host.is_some() || e.lan.requested.is_some() {
            return Err("LAN is already open".into());
        }
        let build = e.lan.build;
        let bind: std::net::SocketAddr = bind
            .parse()
            .map_err(|_| "Use a numeric interface IP and port, such as 192.168.1.5:7777")?;
        if bind.ip().is_unspecified() || bind.ip().is_multicast() || !(1..=16).contains(&max_guests)
        {
            return Err("Select a specific interface and 1 to 16 guests".into());
        }
        e.lan.requested = Some((bind, build, max_guests));
        Ok(())
    })();
    super::report(e, result.err());
}

pub fn close(e: &mut Engine) {
    e.lan.requested = None;
    e.lan.announcer = None;
    if let Some(mut host) = e.lan.host.take() {
        host.close();
        // Keep close retransmission bounded even under repeated toggles.
        if e.lan.draining.len() == 4 {
            e.lan.draining.remove(0);
        }
        e.lan.draining.push((host, Instant::now()));
        e.lan.last_guests.clear();
        super::report(e, None);
    }
}
pub fn kick(e: &mut Engine, id: u64) {
    if let Some(host) = &mut e.lan.host {
        host.kick(id);
    }
    super::report(e, None);
}

pub fn register(app: &mut App) {
    app.add_systems(
        FixedPostUpdate,
        capture
            .after(crate::dim2::record_poses)
            .after(crate::dim3::record_poses)
            .after(crate::dim2::track_contacts)
            .after(crate::dim3::track_contacts),
    );
    app.add_systems(Update, poll.after(crate::world::pump_editor));
    app.add_systems(
        FixedUpdate,
        apply_seat_input
            .after(crate::simulation::SimStep::Sample)
            .before(crate::simulation::SimStep::PluginInput)
            .in_set(crate::world::SimulationSet),
    );
    app.add_systems(
        FixedUpdate,
        (
            publish_lan
                .after(crate::simulation::SimStep::Sample)
                .before(crate::simulation::SimStep::PluginInput),
            lan_verbs.in_set(crate::simulation::SimStep::Verbs),
        )
            .in_set(crate::world::SimulationSet),
    );
    app.add_systems(
        FixedUpdate,
        keep_effects
            .after(crate::simulation::SimStep::Animation)
            .before(crate::simulation::SimStep::ClearEffects)
            .in_set(crate::world::SimulationSet),
    );
}

/// Seated guests' actions as their players' copies, with edges, once a
/// tick on the server, so blocks and motors read them like a local pad.
fn apply_seat_input(mut e: NonSendMut<Engine>, mut views: ResMut<crate::server::SeatViews>) {
    if !e.running || e.lan.host.is_none() {
        if !e.lan.seat_held.is_empty() {
            e.lan.seat_held.clear();
        }
        views.0.retain(|seat, _| *seat == 0);
        return;
    }
    if let Some(host) = &e.lan.host {
        views.0.retain(|seat, _| *seat == 0);
        for (_, seat) in host.seats() {
            if let Some(view) = host.view(seat) {
                views.0.insert(
                    seat,
                    crate::server::SeatView {
                        yaw: view.yaw,
                        facing: view.facing.map(|yaw| (None, yaw)),
                    },
                );
            }
        }
    }
    let mut actions = Vec::new();
    let mut fired = Vec::new();
    let defs: Vec<_> = e
        .project
        .settings
        .input
        .actions
        .iter()
        .filter(|a| a.player > 0)
        .map(|a| (a.name.clone(), a.player))
        .collect();
    for (name, player) in defs {
        let Some(r) = remote_action(&e, player, &name) else {
            e.lan.seat_held.remove(&name);
            continue;
        };
        let was = e
            .lan
            .seat_held
            .insert(name.clone(), r.held)
            .unwrap_or(false);
        if r.held && !was {
            fired.push(name.to_lowercase());
        }
        actions.push((
            name,
            blockloom_core::input::ActionSense {
                held: r.held,
                pressed: r.held && !was,
                released: !r.held && was,
                value: r.value,
                vector: r.vector,
            },
        ));
    }
    blockloom_core::sense::publish_seats(guest_players(&e), actions);
    if !e.paused {
        for action in fired {
            e.fire(blockloom_core::vm::Event::Action(action));
        }
    }
}

fn keep_effects(mut e: NonSendMut<Engine>, effects: Res<crate::engine::PendingEffects>) {
    if e.lan.host.is_some() {
        // Hosting and joining are the host's own business.
        let shared = effects.0.iter().filter(|effect| {
            !matches!(effect, blockloom_core::vm::Effect::Verb { verb, .. } if crate::server::LAN_VERBS.contains(&verb.as_str()))
        });
        e.lan.effects.extend(shared.cloned());
    }
}

fn poll(mut e: NonSendMut<Engine>) {
    for (h, _) in &mut e.lan.draining {
        h.poll();
    }
    e.lan
        .draining
        .retain(|(h, start)| !h.is_drained() && start.elapsed() < Duration::from_secs(1));
    if let Some(h) = &mut e.lan.host {
        h.poll();
        let changes = h.take_seat_changes();
        for change in changes {
            e.fire(match change {
                SeatChange::Joined(seat) => blockloom_core::vm::Event::PlayerJoined(seat + 1),
                SeatChange::Left(seat) => blockloom_core::vm::Event::PlayerLeft(seat + 1),
            });
        }
        let Some(h) = &mut e.lan.host else { return };
        let guests = h.guests();
        if guests != e.lan.last_guests {
            e.lan.last_guests = guests;
            super::report(&e, None);
        }
        if e.lan.settings.announce && e.lan.announcer.is_none() {
            let invite = e.lan.host.as_ref().expect("checked above").invite().clone();
            let name = format!("{} on {}", e.project.name, invite.address.ip());
            e.lan.announcer = Announcer::new(&name, &invite).ok();
        }
        if let Some(a) = &mut e.lan.announcer {
            a.tick();
        }
    }
    if let Some(listener) = &mut e.lan.listener {
        listener.poll();
        let games = listener.games();
        e.lan.heard = games;
    }
}

/// The address `open game to LAN` binds: this machine's LAN address, or
/// loopback with no network, on the project's port.
fn host_address(e: &Engine) -> String {
    let ip = blockloom_net::announce::lan_address()
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    std::net::SocketAddr::new(ip, e.lan.settings.port).to_string()
}

/// A `join LAN game` target as an invite: a number from the games found,
/// a game's name, or an invite already.
fn resolve_join(e: &Engine, target: &str) -> Result<String, String> {
    let target = target.trim();
    if target.starts_with("blockloom-lan/") {
        return Ok(target.to_string());
    }
    let heard = &e.lan.heard;
    let found = match target.parse::<f64>() {
        Ok(n) if n >= 1.0 => heard.get(n as usize - 1),
        _ => heard.iter().find(|h| h.name.eq_ignore_ascii_case(target)),
    };
    found.map(|h| h.invite.encode()).ok_or_else(|| {
        format!(
            "No LAN game \"{target}\" was found: pick one from LAN games found, and turn on Announce on the host"
        )
    })
}

/// `open game to LAN`, `close LAN game`, and `join LAN game` turned into an
/// invite, where the blocks run. Joining itself is the playing world's.
fn lan_verbs(mut e: NonSendMut<Engine>, mut effects: ResMut<crate::engine::PendingEffects>) {
    if !e.authority {
        return;
    }
    let mut errors = Vec::new();
    for effect in effects.0.iter_mut() {
        let blockloom_core::vm::Effect::Verb { actor, verb, args } = effect else {
            continue;
        };
        match verb.as_str() {
            "host_lan" => {
                let bind = host_address(&e);
                let max = e.lan.settings.max_guests;
                open(&mut e, &bind, max);
                if let Some(error) = status(&e).error {
                    errors.push((actor.clone(), error));
                }
            }
            "close_lan" => close(&mut e),
            "join_lan" => {
                let target = args.first().and_then(|a| a.as_str()).unwrap_or("");
                match resolve_join(&e, target) {
                    Ok(invite) => *args = vec![serde_json::Value::String(invite)],
                    Err(error) => {
                        errors.push((actor.clone(), error));
                        *verb = String::new();
                    }
                }
            }
            _ => {}
        }
    }
    // Reported where the run is shown, as a block's error.
    for (actor, message) in errors {
        effects
            .0
            .push(blockloom_core::vm::Effect::Error { actor, message });
    }
}

/// What the LAN reporters read, once a tick where the blocks run.
fn publish_lan(e: NonSend<Engine>) {
    if !e.running {
        return;
    }
    let games = e.lan.heard.iter().map(|h| h.name.clone()).collect();
    let hosting = e.lan.host.is_some() || e.lan.requested.is_some();
    blockloom_core::sense::publish_lan(games, hosting);
}

fn capture(
    mut e: NonSendMut<Engine>,
    dimension: Res<Dimension>,
    time: Res<Time<Fixed>>,
    actors: Query<(
        &ActorId,
        &Transform,
        Option<&PhysicsPose>,
        &Visibility,
        Option<&bevy_rapier2d::prelude::Velocity>,
        Option<&bevy_rapier3d::prelude::Velocity>,
    )>,
) {
    if !e.running {
        close(&mut e);
        e.lan.requested = None;
        return;
    }
    if e.rebuild || e.starting {
        return;
    }
    if e.lan.host.is_none() && e.lan.requested.is_none() {
        return;
    }
    let scene = e.project.active_scene().name.clone();
    if e.lan.scene != scene {
        e.lan.epoch += 1;
        e.lan.scene = scene.clone();
        // A new scene starts its plugins' worlds over.
        e.lan.regions.clear();
        e.lan.synced.clear();
    }
    e.lan.tick += 1;
    if e.lan.requested.is_some() || e.lan.tick.is_multiple_of(SYNC_EVERY) {
        gather_regions(&mut e);
    }
    let mut state = Snapshot {
        epoch: e.lan.epoch,
        tick: e.lan.tick,
        game_ns: (e.run_time(time.elapsed_secs_f64()) * 1e9).max(0.) as u64,
        paused: e.paused,
        dimension: u8::from(dimension.0 == blockloom_core::scene::Mode::ThreeD),
        scene,
        regions: e.lan.regions.clone(),
        ..Default::default()
    };
    for (id, t, pose, visible, v2, v3) in &actors {
        let Some(actor) = e.actor(&id.0) else {
            continue;
        };
        let t = pose.map_or(t, |p| &p.0);
        let mut data = [0.; 16];
        data[..3].copy_from_slice(&t.translation.to_array());
        data[3..7].copy_from_slice(&t.rotation.to_array());
        data[7..10].copy_from_slice(&t.scale.to_array());
        if let Some(v) = v3 {
            data[10..13].copy_from_slice(&v.linear.to_array());
            data[13..16].copy_from_slice(&v.angular.to_array());
        }
        if let Some(v) = v2 {
            data[10..12].copy_from_slice(&v.linear.to_array());
            data[15] = v.angular;
        }
        let appearance=serde_json::to_vec(&serde_json::json!({"name":actor.name,"visual":actor.visual(),"parent":e.parents.get(&id.0)})).expect("actor presentation serializes");
        state.actors.insert(
            id.0.clone(),
            ActorState {
                id: id.0.clone(),
                template: e.clones.get(&id.0).cloned().unwrap_or_else(|| id.0.clone()),
                pose: data,
                visible: *visible != Visibility::Hidden,
                appearance,
            },
        );
    }
    if let Some((bind, build, max_peers)) = e.lan.requested.take() {
        match LanHost::open(
            bind,
            build,
            state,
            ServerOptions {
                max_peers,
                ..Default::default()
            },
        ) {
            Ok(host) => {
                e.lan.host = Some(host);
                super::report(&e, None);
            }
            Err(error) => super::report(&e, Some(error)),
        }
    } else if e.lan.host.is_some() {
        let effects = std::mem::take(&mut e.lan.effects);
        let tick = e.lan.tick;
        let h = e.lan.host.as_mut().expect("checked above");
        let mut result = h.publish(state);
        if result.is_ok() && !effects.is_empty() {
            result = serde_json::to_vec(&effects)
                .map_err(|err| err.to_string())
                .and_then(|bytes| h.publish_effects(tick, &bytes));
        }
        if let Err(error) = result {
            close(&mut e);
            super::report(&e, Some(error));
        }
    }
}

/// What a guest's own copy of each plugin applies, as `sync.apply` args per
/// plugin: the records that differ from `old` (all of them without one),
/// with the plugin's world identity always first.
pub fn sync_apply_calls(
    old: Option<&BTreeMap<String, Region>>,
    regions: &BTreeMap<String, Region>,
) -> Vec<(String, serde_json::Value)> {
    let mut calls = BTreeMap::<String, serde_json::Value>::new();
    for (key, region) in regions {
        let Some((plugin, rest)) = key.split_once('/') else {
            continue;
        };
        let Ok(data) = serde_json::from_slice::<serde_json::Value>(&region.data) else {
            continue;
        };
        let call = calls
            .entry(plugin.to_string())
            .or_insert_with(|| serde_json::json!({"identity":null,"sections":[]}));
        if rest == "identity" {
            call["identity"] = data;
        } else if old.is_none_or(|o| o.get(key) != Some(region)) {
            call["sections"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"key":rest,"data":data}));
        }
    }
    calls.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_net::game::Invite;

    #[test]
    fn a_join_target_is_a_number_a_name_or_an_invite() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut e = Engine::new(rx, blockloom_core::scene::Mode::TwoD);
        let invite = |port: u16| {
            Invite::decode(&format!(
                "blockloom-lan/1|127.0.0.1:{port}|{}|{}|{}",
                "ab".repeat(32),
                "cd".repeat(32),
                "ef".repeat(32)
            ))
            .unwrap()
        };
        e.lan.heard = vec![
            Heard {
                name: "Pond on 10.0.0.2".into(),
                invite: invite(7001),
            },
            Heard {
                name: "Pond on 10.0.0.3".into(),
                invite: invite(7002),
            },
        ];
        assert!(resolve_join(&e, "2").unwrap().contains(":7002|"));
        assert!(
            resolve_join(&e, "pond on 10.0.0.2")
                .unwrap()
                .contains(":7001|")
        );
        let typed = invite(9).encode();
        assert_eq!(resolve_join(&e, &format!(" {typed} ")).unwrap(), typed);
        let missing = resolve_join(&e, "3").unwrap_err();
        assert!(missing.contains(": "), "names the fix: {missing}");
    }
}
