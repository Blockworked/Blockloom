use blockloom_net::game::{
    ActionInput, ActorState, Invite, LanClient, LanHost, SeatChange, Snapshot, build_hash,
};
use blockloom_net::{ClientOptions, Impairment, ServerOptions};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

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
        tick: 60,
        scene: "First".into(),
        actors: BTreeMap::from([(a.id.clone(), a)]),
        ..Default::default()
    }
}
fn until(host: &mut LanHost, client: &mut LanClient, done: impl Fn(&LanClient) -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        host.poll();
        client.poll().unwrap();
        if done(client) {
            return;
        }
        assert!(Instant::now() < end, "LAN client stalled");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn late_join_move_spawn_delete_scene_and_close_under_loss() {
    let initial = sample();
    let build = build_hash(b"trusted fixture");
    let impairment = Impairment {
        loss: 0.05,
        delay: Duration::from_millis(10),
        jitter: Duration::from_millis(5),
        ..Default::default()
    };
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        initial.clone(),
        ServerOptions {
            impairment,
            ..Default::default()
        },
    )
    .unwrap();
    let invite = Invite::decode(&host.invite().encode()).unwrap();
    let mut c = LanClient::connect(
        invite,
        build,
        ClientOptions {
            impairment,
            ..Default::default()
        },
    )
    .unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&initial));
    let mut next = initial.clone();
    next.tick += 1;
    next.actors.get_mut("player").unwrap().pose[0] = 12.;
    let mut clone = next.actors["player"].clone();
    clone.id = "clone".into();
    next.actors.insert(clone.id.clone(), clone);
    host.publish(next.clone()).unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&next));
    next.tick += 1;
    next.actors.remove("player");
    host.publish(next.clone()).unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&next));
    next.epoch = 2;
    next.tick = 0;
    next.dimension = 1;
    next.scene = "Next".into();
    next.actors.clear();
    host.publish(next.clone()).unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&next));
    assert_eq!(host.guests().len(), 1);
    host.close();
    assert!(host.guests().is_empty());
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        host.poll();
        if c.poll().is_err() {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn wrong_admission_and_build_cannot_receive_state() {
    let build = build_hash(b"fixture");
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        sample(),
        ServerOptions::default(),
    )
    .unwrap();
    assert!(LanClient::connect(host.invite().clone(), [0; 32], ClientOptions::default()).is_err());
    let mut invite = host.invite().clone();
    invite.token = [0; 32];
    let mut c = LanClient::connect(invite, build, ClientOptions::default()).unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        host.poll();
        if c.poll().is_err() {
            break;
        }
        assert!(c.state().is_none());
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(host.guests().is_empty());
}
#[test]
fn bind_failure_and_interface_validation_are_explicit() {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    assert!(
        LanHost::open(
            socket.local_addr().unwrap(),
            [0; 32],
            sample(),
            ServerOptions::default()
        )
        .is_err()
    );
    assert!(
        LanHost::open(
            "0.0.0.0:0".parse().unwrap(),
            [0; 32],
            sample(),
            ServerOptions::default()
        )
        .is_err()
    );
}
#[test]
fn slow_guest_coalesces_state_without_unbounded_tail() {
    let build = build_hash(b"fixture");
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        sample(),
        ServerOptions::default(),
    )
    .unwrap();
    let mut c = LanClient::connect(host.invite().clone(), build, ClientOptions::default()).unwrap();
    until(&mut host, &mut c, |c| c.state().is_some());
    let mut next = sample();
    for i in 61..500 {
        next.tick = i;
        next.actors.get_mut("player").unwrap().pose[0] = i as f32;
        host.publish(next.clone()).unwrap();
        host.poll();
    }
    until(&mut host, &mut c, |c| c.state() == Some(&next));
}

#[test]
fn plugin_regions_larger_than_a_frame_stream_in_pieces_under_loss() {
    use blockloom_net::game::{MAX_FRAME, Region};
    let build = build_hash(b"trusted fixture");
    let impairment = Impairment {
        loss: 0.05,
        delay: Duration::from_millis(5),
        ..Default::default()
    };
    let mut initial = sample();
    // Eight 300 KiB sections: about two and a half frames' worth.
    for i in 0..8u8 {
        initial.regions.insert(
            format!("com.example.voxel/section/{i}"),
            Region {
                revision: 1,
                data: vec![i; 300 * 1024].into(),
            },
        );
    }
    let total: usize = initial.regions.values().map(|r| r.data.len()).sum();
    assert!(total > 2 * MAX_FRAME);
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        initial.clone(),
        ServerOptions {
            impairment,
            ..Default::default()
        },
    )
    .unwrap();
    let invite = Invite::decode(&host.invite().encode()).unwrap();
    let mut c = LanClient::connect(
        invite,
        build,
        ClientOptions {
            impairment,
            ..Default::default()
        },
    )
    .unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&initial));
    assert!(c.sequence() >= 3, "sent in {} frames", c.sequence());

    // One section changes and one goes: only those travel.
    let mut next = initial.clone();
    next.tick += 1;
    next.regions
        .get_mut("com.example.voxel/section/2")
        .unwrap()
        .revision = 2;
    next.regions
        .get_mut("com.example.voxel/section/2")
        .unwrap()
        .data = vec![9; 10].into();
    next.regions.remove("com.example.voxel/section/5");
    let before = c.sequence();
    host.publish(next.clone()).unwrap();
    until(&mut host, &mut c, |c| c.state() == Some(&next));
    assert_eq!(c.sequence(), before + 1);
}

#[test]
fn a_region_too_big_for_any_frame_is_refused() {
    use blockloom_net::game::{MAX_REGION, Region};
    let mut state = sample();
    state.regions.insert(
        "p/k".into(),
        Region {
            revision: 1,
            data: vec![0; MAX_REGION + 1].into(),
        },
    );
    assert!(
        LanHost::open(
            "127.0.0.1:0".parse().unwrap(),
            build_hash(b"x"),
            state,
            ServerOptions::default()
        )
        .is_err()
    );
}

#[test]
fn two_guests_take_their_own_seats_and_send_their_own_input() {
    let build = build_hash(b"trusted fixture");
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        sample(),
        Default::default(),
    )
    .unwrap();
    let code = host_invite(&host);
    let invite = || Invite::decode(&code).unwrap();
    let mut a = LanClient::connect(invite(), build, Default::default()).unwrap();
    let mut b = LanClient::connect(invite(), build, Default::default()).unwrap();
    let end = Instant::now() + Duration::from_secs(8);
    while a.seat().is_none() || b.seat().is_none() || a.state().is_none() || b.state().is_none() {
        host.poll();
        a.poll().unwrap();
        b.poll().unwrap();
        assert!(Instant::now() < end, "guests never seated");
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut seats = vec![a.seat().unwrap(), b.seat().unwrap()];
    seats.sort();
    assert_eq!(seats, [1, 2]);
    let mut joined = host.take_seat_changes();
    joined.sort_by_key(|c| format!("{c:?}"));
    assert_eq!(joined, [SeatChange::Joined(1), SeatChange::Joined(2)]);

    let jump = BTreeMap::from([(
        "Jump".to_string(),
        ActionInput {
            held: true,
            value: 1.0,
            vector: [1.0, 0.0],
        },
    )]);
    let walk = BTreeMap::from([(
        "Move".to_string(),
        ActionInput {
            held: true,
            value: 1.0,
            vector: [0.0, -1.0],
        },
    )]);
    a.send_input(&jump).unwrap();
    b.send_input(&walk).unwrap();
    let (sa, sb) = (a.seat().unwrap(), b.seat().unwrap());
    let end = Instant::now() + Duration::from_secs(8);
    while host.input(sa) != Some(&jump) || host.input(sb) != Some(&walk) {
        host.poll();
        a.poll().unwrap();
        b.poll().unwrap();
        assert!(Instant::now() < end, "input never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }

    // Every seated guest gets each tick's effects, in order.
    host.publish_effects(7, b"[1]").unwrap();
    host.publish_effects(8, b"[2]").unwrap();
    let end = Instant::now() + Duration::from_secs(8);
    let (mut ga, mut gb) = (Vec::new(), Vec::new());
    while ga.len() < 2 || gb.len() < 2 {
        host.poll();
        a.poll().unwrap();
        b.poll().unwrap();
        ga.extend(a.take_effects());
        gb.extend(b.take_effects());
        assert!(Instant::now() < end, "effects never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    let want = vec![(7, b"[1]".to_vec()), (8, b"[2]".to_vec())];
    assert_eq!((ga, gb), (want.clone(), want));

    // A leaving guest frees its seat, and the next one takes the lowest free.
    let gone = a.seat().unwrap();
    drop(a);
    let id = host
        .seats()
        .into_iter()
        .find(|(_, s)| *s == gone)
        .unwrap()
        .0;
    host.kick(id);
    assert_eq!(host.take_seat_changes(), [SeatChange::Left(gone)]);
    assert!(host.input(gone).is_none());
    let mut c = LanClient::connect(invite(), build, Default::default()).unwrap();
    until(&mut host, &mut c, |c| {
        c.seat().is_some() && c.state().is_some()
    });
    assert_eq!(c.seat(), Some(gone));
}

#[test]
fn malformed_input_is_refused_before_it_is_sent() {
    let build = build_hash(b"trusted fixture");
    let mut host = LanHost::open(
        "127.0.0.1:0".parse().unwrap(),
        build,
        sample(),
        Default::default(),
    )
    .unwrap();
    let mut c = LanClient::connect(
        Invite::decode(&host_invite(&host)).unwrap(),
        build,
        Default::default(),
    )
    .unwrap();
    until(&mut host, &mut c, |c| c.seat().is_some());
    let bad = |a: ActionInput| BTreeMap::from([("Jump".to_string(), a)]);
    assert!(
        c.send_input(&bad(ActionInput {
            value: f32::NAN,
            ..Default::default()
        }))
        .is_err()
    );
    assert!(
        c.send_input(&bad(ActionInput {
            value: 1e9,
            ..Default::default()
        }))
        .is_err()
    );
    let long = BTreeMap::from([("x".repeat(65), ActionInput::default())]);
    assert!(c.send_input(&long).is_err());
    let many: BTreeMap<_, _> = (0..65)
        .map(|i| (format!("a{i}"), ActionInput::default()))
        .collect();
    assert!(c.send_input(&many).is_err());
}

fn host_invite(host: &LanHost) -> String {
    host.invite().encode()
}
