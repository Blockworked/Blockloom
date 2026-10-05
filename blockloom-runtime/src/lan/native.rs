use super::*;
use crate::engine::{ActorId, Dimension, PhysicsPose};
use bevy::prelude::*;
use blockloom_net::ServerOptions;
use blockloom_net::game::{ActorState, LanHost, Snapshot, build_hash};
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

pub fn begin(e: &mut Engine) {
    e.lan.settings = e.project.multiplayer.clone();
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
        let mut content =
            format!("blockloom-{}-lan-spectator-1\n", env!("CARGO_PKG_VERSION")).into_bytes();
        content.extend(serde_json::to_vec(&e.project).map_err(|err| err.to_string())?);
        let build = build_hash(&content);
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
        let guests = h.guests();
        if guests != e.lan.last_guests {
            e.lan.last_guests = guests;
            super::report(&e, None);
        }
    }
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
    }
    e.lan.tick += 1;
    let mut state = Snapshot {
        epoch: e.lan.epoch,
        tick: e.lan.tick,
        game_ns: (e.run_time(time.elapsed_secs_f64()) * 1e9).max(0.) as u64,
        paused: e.paused,
        dimension: u8::from(dimension.0 == blockloom_core::scene::Mode::ThreeD),
        scene,
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
    } else if let Some(h) = &mut e.lan.host
        && let Err(error) = h.publish(state)
    {
        close(&mut e);
        super::report(&e, Some(error));
    }
}
